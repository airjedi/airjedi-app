use crate::associator::gnn::GnnAssociator;
use crate::associator::spatial_index::SpatialIndex;
use crate::associator::AssociatorConfig;
use crate::classification::TargetClassification;
use crate::config::FusionConfig;
use crate::filter::{FilterResult, TrackerState};
use crate::prelude_imports::*;
use crate::sensor::SensorObservation;
use crate::store::TimelineStore;
use crate::track::initiation::MofNInitiator;
use crate::track::{LifecycleProfiles, Track, TrackQuality, TrackStatus};
use crate::types::TrackId;
use chrono::Utc;

#[derive(Resource)]
pub struct TrackInitiator(pub MofNInitiator);

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

pub fn fusion_update_system(
    store: Res<TimelineStore>,
    mut tracks: Query<(&mut Track, &mut TrackerState, &mut TrackQuality)>,
    time: Res<Time>,
) {
    let dt = time.delta_secs_f64();
    if dt <= 0.0 {
        return;
    }
    let now = Utc::now();

    for (mut track, mut tracker, mut quality) in &mut tracks {
        // Always predict, even when coasting or lost. Skipping predict() during coasting
        // freezes the filter covariance, causing returning observations to exceed the
        // Mahalanobis gate and be rejected as outliers, preventing reacquisition.
        if !track.is_on_ground {
            tracker.variant.predict(dt);
        }

        let obs = store.query_range(
            &track.id,
            tracker.last_update.unwrap_or(track.created_at),
            now,
        );

        for stored_obs in &obs {
            match tracker.variant.update(&stored_obs.observation) {
                FilterResult::Updated => {
                    quality.observation_count += 1;
                    quality.reacquire();
                    track.last_update = now;
                }
                FilterResult::OutlierRejected { .. } => {}
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
    time: Res<Time>,
    lifecycle: Res<LifecycleProfiles>,
    mut tracks: Query<(&mut TrackQuality, &TargetClassification)>,
) {
    for (mut quality, classification) in &mut tracks {
        let config = lifecycle.get(&classification.category);
        let staleness = quality.staleness + time.delta();
        quality.transition(staleness, config);
    }
}

pub fn track_initiation_system(
    mut commands: Commands,
    store: Res<TimelineStore>,
    existing_tracks: Query<&Track>,
    fusion_config: Res<FusionConfig>,
    mut initiator: ResMut<TrackInitiator>,
) {
    use std::collections::HashSet;

    if store.unassociated().is_empty() {
        return;
    }

    let now = Utc::now();

    let existing_ids: HashSet<String> = existing_tracks
        .iter()
        .flat_map(|t| t.cooperative_ids.iter().map(|cid| cid.id.clone()))
        .collect();

    let mut initiated_ids: HashSet<String> = HashSet::new();

    for obs in store.unassociated() {
        if let Some(ref target_id) = obs.observation.target_id {
            if existing_ids.contains(&target_id.id) {
                continue;
            }
            if initiated_ids.contains(&target_id.id) {
                continue;
            }
        }

        let decision = initiator.0.process_observation(&obs.observation, now);

        let promote_obs = match decision {
            crate::track::initiation::InitiationDecision::Promote(promoted) => promoted,
            crate::track::initiation::InitiationDecision::SinglePoint => {
                obs.observation.clone()
            }
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
            TrackQuality::default(),
            classification,
        ));
    }

    initiator.0.evict_stale(now);
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

pub fn store_eviction_system(mut store: ResMut<TimelineStore>) {
    store.evict_old(Utc::now());
}
