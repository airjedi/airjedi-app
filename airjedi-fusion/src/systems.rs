use crate::prelude_imports::*;
use chrono::Utc;
use crate::associator::gnn::GnnAssociator;
use crate::associator::spatial_index::SpatialIndex;
use crate::associator::AssociatorConfig;
use crate::classification::TargetClassification;
use crate::config::FusionConfig;
use crate::filter::{FilterResult, TrackerState};
use crate::sensor::SensorObservation;
use crate::store::TimelineStore;
use crate::track::{LifecycleProfiles, Track, TrackQuality, TrackStatus};
use crate::types::TrackId;

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

pub fn drain_observations(
    mut buffer: ResMut<ObservationBuffer>,
    mut store: ResMut<TimelineStore>,
) {
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

    let result = GnnAssociator::associate(
        &unassociated_refs,
        &track_list,
        &spatial_index,
        &config,
    );

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
    mut tracks: Query<(&Track, &mut TrackerState, &mut TrackQuality)>,
    time: Res<Time>,
) {
    let dt = time.delta_secs_f64();
    if dt <= 0.0 {
        return;
    }
    let now = Utc::now();

    for (track, mut tracker, mut quality) in &mut tracks {
        tracker.variant.predict(dt);

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
                }
                FilterResult::OutlierRejected { .. } => {}
                FilterResult::DivergenceDetected => {
                    tracker.variant.initialize(&stored_obs.observation);
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
        let (lat, lon, _) = tracker.position_geodetic();
        spatial_index.update_track(&track.id, lat, lon);
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
    fusion_config: Res<FusionConfig>,
) {
    for obs in store.unassociated() {
        let mut tracker =
            TrackerState::new_6dof(fusion_config.filter_defaults.clone());
        tracker.variant.initialize(&obs.observation);
        tracker.last_update = Some(Utc::now());

        let track_id = TrackId::new();

        let mut cooperative_ids = Vec::new();
        if let Some(ref target_id) = obs.observation.target_id {
            cooperative_ids.push(target_id.clone());
        }

        let classification = TargetClassification {
            category: obs
                .observation
                .classification_hint
                .unwrap_or(crate::types::TargetCategory::Unknown),
            ..Default::default()
        };

        commands.spawn((
            Track {
                id: track_id,
                cooperative_ids,
                created_at: Utc::now(),
                last_update: Utc::now(),
            },
            tracker,
            TrackQuality::default(),
            classification,
        ));
    }
}

pub fn track_cleanup_system(
    mut commands: Commands,
    mut spatial_index: ResMut<SpatialIndex>,
    tracks: Query<(Entity, &Track, &TrackQuality)>,
) {
    for (entity, track, quality) in &tracks {
        if quality.status == TrackStatus::Lost {
            spatial_index.remove_track(&track.id);
            commands.entity(entity).despawn();
        }
    }
}

pub fn store_eviction_system(
    mut store: ResMut<TimelineStore>,
) {
    store.evict_old(Utc::now());
}
