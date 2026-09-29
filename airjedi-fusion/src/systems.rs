use crate::associator::gnn::GnnAssociator;
use crate::associator::spatial_index::SpatialIndex;
use crate::associator::AssociatorConfig;
use crate::classification::TargetClassification;
use crate::clock::FusionClock;
use crate::config::FusionConfig;
use crate::filter::{FilterResult, TrackerState};
use crate::prelude_imports::*;
use crate::sensor::SensorObservation;
use crate::store::{observation_key, ObservationKey, TimelineStore};
use crate::track::initiation::MofNInitiator;
use crate::track::{LifecycleProfiles, Track, TrackQuality, TrackStatus};
use crate::types::TrackId;

#[derive(Resource)]
pub struct TrackInitiator {
    pub initiator: MofNInitiator,
    pub processed_observations: std::collections::HashSet<ObservationKey>,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum FusionSet {
    Drain,
    Associate,
    Fuse,
    Lifecycle,
}

#[derive(Resource, Default)]
pub struct ObservationBuffer {
    pub observations: Vec<SensorObservation>,
}

pub fn drain_observations(mut buffer: ResMut<ObservationBuffer>, mut store: ResMut<TimelineStore>) {
    for obs in buffer.observations.drain(..) {
        store.insert(obs);
    }
}

pub fn association_system(
    mut store: ResMut<TimelineStore>,
    tracks: Query<(&Track, &TrackerState, &TargetClassification)>,
    spatial_index: Res<SpatialIndex>,
    config: Res<AssociatorConfig>,
) {
    if store.unassociated().is_empty() {
        return;
    }

    let track_list: Vec<_> = tracks.iter().collect();
    if track_list.is_empty() {
        return;
    }

    let unassociated_refs: Vec<_> = store.unassociated().iter().collect();

    let result = GnnAssociator::associate(&unassociated_refs, &track_list, &spatial_index, &config);

    // Associate in reverse index order to keep indices valid during removal
    let mut sorted_assignments = result.assignments;
    sorted_assignments.sort_by(|a, b| b.observation_idx.cmp(&a.observation_idx));
    for assignment in &sorted_assignments {
        let track_id = &track_list[assignment.track_idx].0.id;
        store.associate(assignment.observation_idx, track_id);
    }
}

/// How long a track can go without a fused observation before a gate-rejected
/// observation is treated as a post-gap reacquisition rather than an outlier.
/// A constant-velocity filter's predicted velocity still points the pre-gap
/// direction, so a target that maneuvered during the gap returns a velocity
/// (and position) innovation that blows the Mahalanobis gate. For cooperative
/// ID-matched targets there is no association ambiguity, so once a gap has
/// opened we trust the fresh report and re-seed the filter instead of
/// discarding it forever (which strands the track until it is cleaned up and
/// re-initiated far away).
const REACQUIRE_GAP: std::time::Duration = std::time::Duration::from_secs(3);

pub fn fusion_update_system(
    store: Res<TimelineStore>,
    mut tracks: Query<(&mut Track, &mut TrackerState, &mut TrackQuality)>,
    clock: Res<FusionClock>,
) {
    let dt = clock.delta_secs_f64();
    if dt <= 0.0 {
        return;
    }
    let now = clock.now_utc();

    for (mut track, mut tracker, mut quality) in &mut tracks {
        // Always predict, even when coasting or lost. Skipping predict() during coasting
        // freezes the filter covariance, causing returning observations to exceed the
        // Mahalanobis gate and be rejected as outliers, preventing reacquisition.
        if !track.is_on_ground {
            tracker.variant.predict(dt);
        }

        let obs = store.associated_observations_for_track(&track.id);

        for stored_obs in obs {
            if tracker.is_processed(stored_obs) {
                continue;
            }

            match tracker.variant.update(&stored_obs.observation) {
                FilterResult::Updated => {
                    quality.observation_count += 1;
                    quality.reacquire();
                    track.last_update = now;
                }
                FilterResult::TelemetryOnly => {}
                FilterResult::OutlierRejected { .. } => {
                    // A gate rejection after a signal gap is almost always a coasted
                    // constant-velocity prediction that diverged from a maneuvering
                    // target, not a genuinely spurious report. When the track has gone
                    // stale (gap opened) or already lost lock, re-seed the filter from
                    // the fresh observation and reacquire rather than stranding the
                    // track. Healthy, continuously-tracked targets have staleness ~0 and
                    // keep full outlier protection.
                    let gap_reacquire = quality.staleness >= REACQUIRE_GAP
                        || matches!(quality.status, TrackStatus::Coasting | TrackStatus::Lost);
                    if gap_reacquire {
                        tracker.variant.initialize(&stored_obs.observation);
                        quality.observation_count += 1;
                        quality.reacquire();
                        track.last_update = now;
                    }
                }
                FilterResult::DivergenceDetected => {
                    tracker.variant.initialize(&stored_obs.observation);
                    quality.reacquire();
                    track.last_update = now;
                }
            }

            if let Some(on_ground) = stored_obs.observation.metadata.is_on_ground {
                track.is_on_ground = on_ground;
                if on_ground {
                    tracker.zero_velocity();
                }
            }

            tracker.mark_processed(stored_obs);
        }

        tracker.last_update = Some(now);
    }
}

pub fn update_spatial_index(
    mut spatial_index: ResMut<SpatialIndex>,
    tracks: Query<(&Track, &TrackerState), Changed<TrackerState>>,
) {
    for (track, tracker) in &tracks {
        if tracker.last_update.is_none() {
            continue;
        }
        let (lat, lon, _) = tracker.position_geodetic();
        spatial_index.update_track(&track.id, lat, lon);
    }
    if spatial_index.needs_compaction() {
        spatial_index.rebuild();
    }
}

pub fn track_status_system(
    clock: Res<FusionClock>,
    lifecycle: Res<LifecycleProfiles>,
    mut tracks: Query<(&mut TrackQuality, &TargetClassification)>,
) {
    for (mut quality, classification) in &mut tracks {
        let config = lifecycle.get(&classification.category);
        let staleness =
            quality.staleness + std::time::Duration::from_secs_f64(clock.delta_secs_f64().max(0.0));
        quality.transition(staleness, config);
    }
}

pub fn track_initiation_system(
    mut commands: Commands,
    store: Res<TimelineStore>,
    existing_tracks: Query<&Track>,
    fusion_config: Res<FusionConfig>,
    mut initiator: ResMut<TrackInitiator>,
    clock: Res<FusionClock>,
) {
    use std::collections::HashSet;

    if store.unassociated().is_empty() {
        return;
    }

    let now = clock.now_utc();

    let existing_ids: HashSet<String> = existing_tracks
        .iter()
        .flat_map(|t| t.cooperative_ids.iter().map(|cid| cid.id.clone()))
        .collect();

    let mut initiated_ids: HashSet<String> = HashSet::new();

    for obs in store.unassociated() {
        if let Some(key) = observation_key(&obs.observation) {
            if !initiator.processed_observations.insert(key) {
                continue;
            }
        }

        if obs.observation.is_telemetry_only() {
            continue;
        }

        if let Some(ref target_id) = obs.observation.target_id {
            if existing_ids.contains(&target_id.id) {
                continue;
            }
            if initiated_ids.contains(&target_id.id) {
                continue;
            }
        }

        let decision = initiator
            .initiator
            .process_observation(&obs.observation, now);

        let promote_obs = match decision {
            crate::track::initiation::InitiationDecision::Promote(promoted) => promoted,
            crate::track::initiation::InitiationDecision::SinglePoint => obs.observation.clone(),
            crate::track::initiation::InitiationDecision::Pending => continue,
        };

        if let Some(ref target_id) = promote_obs.target_id {
            if initiated_ids.contains(&target_id.id) {
                continue;
            }
            initiated_ids.insert(target_id.id.clone());
        }

        let category = promote_obs
            .classification_hint
            .unwrap_or(crate::types::TargetCategory::Unknown);

        let mut tracker = fusion_config.create_tracker(&category);
        tracker.variant.initialize(&promote_obs);
        tracker.last_update = Some(now);
        tracker.mark_processed(obs);

        let mut cooperative_ids = Vec::new();
        if let Some(ref target_id) = promote_obs.target_id {
            cooperative_ids.push(target_id.clone());
        }

        let classification = TargetClassification {
            category,
            ..Default::default()
        };

        commands.spawn((
            Track {
                id: TrackId::new(),
                cooperative_ids,
                created_at: now,
                last_update: now,
                is_on_ground: false,
            },
            tracker,
            TrackQuality {
                observation_count: 1,
                ..Default::default()
            },
            classification,
        ));
    }

    initiator.initiator.evict_stale(now);
}

pub fn track_cleanup_system(
    mut commands: Commands,
    mut spatial_index: ResMut<SpatialIndex>,
    lifecycle: Res<LifecycleProfiles>,
    tracks: Query<(Entity, &Track, &TrackQuality, &TargetClassification)>,
) {
    for (entity, track, quality, classification) in &tracks {
        if quality.status == TrackStatus::Lost {
            let config = lifecycle.get(&classification.category);
            let cleanup_after = config.coast_timeout + config.lost_timeout + config.cleanup_delay;
            if quality.staleness > cleanup_after {
                spatial_index.remove_track(&track.id);
                commands.entity(entity).despawn();
            }
        }
    }
}

pub fn store_eviction_system(mut store: ResMut<TimelineStore>, clock: Res<FusionClock>) {
    store.evict_old(clock.now_utc());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FusionClock;
    use crate::config::FusionConfig;
    use crate::coord::CoordinateFrame;
    use crate::sensor::{
        FusionTier, Measurement, ObservationCovariance, ObservationMetadata, SensorId, SensorKind,
    };
    use crate::store::StoreConfig;
    use crate::track::initiation::{InitiationConfig, MofNInitiator};
    use crate::types::Timestamp;
    use airjedi_core::{ObservationFreshness, ObservationIdentity, TimeSourceQuality};
    use chrono::Duration as ChronoDuration;
    use nalgebra::DMatrix;
    use std::collections::HashSet;
    use std::time::Duration;

    fn telemetry_observation(timestamp: Timestamp, frame_sequence: u64) -> SensorObservation {
        let identity = ObservationIdentity {
            frame_sequence,
            payload_index: 0,
        };
        let freshness = ObservationFreshness {
            observation_time: timestamp,
            receipt_time: timestamp,
            time_source: TimeSourceQuality::ProtocolTimestamp,
            identity,
        };
        SensorObservation {
            sensor_id: SensorId {
                id: "initiator-history-probe".to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp,
            receipt_time: timestamp,
            target_id: None,
            measurement: Measurement::PositionVelocity3D {
                lat_deg: 37.0,
                lon_deg: -97.0,
                alt_m: Some(10_000.0),
                vel_north_mps: None,
                vel_east_mps: None,
                vel_down_mps: None,
                heading_deg: None,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(3, 3),
            },
            classification_hint: None,
            metadata: ObservationMetadata {
                observation_id: Some(identity),
                altitude_freshness: Some(freshness),
                ..Default::default()
            },
        }
    }

    #[test]
    #[ignore = "known history regression: track initiator retains dedup identities after store eviction"]
    fn initiator_releases_dedup_identities_after_the_store_evicts_their_source_observations() {
        let timestamp = chrono::DateTime::from_timestamp(1_700_000_000, 0)
            .expect("fixed test timestamp is valid");
        let mut app = App::new();
        app.insert_resource(TimelineStore::new(StoreConfig {
            hot_retention: Duration::ZERO,
            ..Default::default()
        }))
        .insert_resource(FusionConfig::default())
        .insert_resource(FusionClock::fixed(timestamp))
        .insert_resource(TrackInitiator {
            initiator: MofNInitiator::new(InitiationConfig::default()),
            processed_observations: HashSet::new(),
        })
        .add_systems(Update, track_initiation_system);

        {
            let mut store = app.world_mut().resource_mut::<TimelineStore>();
            for frame_sequence in 1..=32 {
                assert!(store.insert(telemetry_observation(timestamp, frame_sequence)));
            }
        }
        app.update();
        assert_eq!(
            app.world()
                .resource::<TrackInitiator>()
                .processed_observations
                .len(),
            32
        );

        app.world_mut()
            .resource_mut::<TimelineStore>()
            .evict_old(timestamp + ChronoDuration::seconds(1));
        assert_eq!(
            app.world()
                .resource::<TimelineStore>()
                .total_observation_count(),
            0
        );
        app.update();

        assert!(
            app.world()
                .resource::<TrackInitiator>()
                .processed_observations
                .is_empty(),
            "initiator dedup identities must be released once their source observations leave the store"
        );
    }
}
