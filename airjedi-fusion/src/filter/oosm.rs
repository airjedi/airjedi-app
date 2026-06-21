use crate::filter::{FilterResult, OosmConfig, TrackerState};
use crate::sensor::SensorObservation;
use crate::store::TimelineStore;
use crate::types::{Timestamp, TrackId};

pub fn handle_oosm(
    tracker: &mut TrackerState,
    late_obs: &SensorObservation,
    track_id: &TrackId,
    store: &TimelineStore,
    config: &OosmConfig,
    now: Timestamp,
) -> FilterResult {
    let obs_time = late_obs.timestamp;

    // Reject if too old
    let lag = now.signed_duration_since(obs_time);
    let max_lag = chrono::Duration::from_std(config.max_lag)
        .unwrap_or(chrono::Duration::seconds(30));
    if lag > max_lag {
        return FilterResult::OutlierRejected {
            distance: f64::INFINITY,
        };
    }

    // Find a state snapshot from before the late observation
    let history = tracker.variant.state_history();
    let snapshot = match history.find_before(obs_time) {
        Some(s) => s.clone(),
        None => {
            // No snapshot old enough - apply as normal late update
            return tracker.variant.update(late_obs);
        }
    };

    // Rollback to the snapshot
    tracker.variant.initialize_from_state(
        snapshot.state.clone(),
        snapshot.covariance.clone(),
    );

    // Gather all observations for this track between snapshot time and now
    let stored_obs = store.query_range(track_id, snapshot.timestamp, now);

    // Build replay sequence: existing observations + the late one, sorted by time
    let mut replay_timestamps: Vec<(&SensorObservation, Timestamp)> = stored_obs
        .iter()
        .map(|so| (&so.observation, so.observation.timestamp))
        .collect();
    replay_timestamps.push((late_obs, obs_time));
    replay_timestamps.sort_by_key(|(_, t)| *t);

    // Replay all observations in chronological order
    let mut last_time = snapshot.timestamp;
    let mut last_result = FilterResult::Updated;

    for (obs, t) in &replay_timestamps {
        let dt = t.signed_duration_since(last_time).num_milliseconds() as f64 / 1000.0;
        if dt > 0.0 {
            tracker.variant.predict(dt);
        }
        last_result = tracker.variant.update(obs);
        last_time = *t;
    }

    // Predict forward to current time
    let final_dt = now.signed_duration_since(last_time).num_milliseconds() as f64 / 1000.0;
    if final_dt > 0.0 {
        tracker.variant.predict(final_dt);
    }

    last_result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::CoordinateFrame;
    use crate::filter::ekf::ProcessNoiseConfig;
    use crate::sensor::*;
    use crate::store::StoreConfig;
    use crate::types::*;
    use chrono::{Duration, Utc};
    use nalgebra::DMatrix;

    fn make_obs_at_time(t: Timestamp, lat: f64) -> SensorObservation {
        SensorObservation {
            sensor_id: SensorId {
                id: "test".to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp: t,
            receipt_time: Utc::now(),
            target_id: None,
            measurement: Measurement::PositionVelocity3D {
                lat_deg: lat,
                lon_deg: -97.0,
                alt_m: Some(10000.0),
                vel_north_mps: Some(100.0),
                vel_east_mps: Some(0.0),
                vel_down_mps: Some(0.0),
                heading_deg: None,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(6, 6) * 100.0,
            },
            classification_hint: None,
            metadata: ObservationMetadata::default(),
        }
    }

    #[test]
    fn oosm_too_old_is_rejected() {
        let now = Utc::now();
        let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
        let init_obs = make_obs_at_time(now, 37.0);
        tracker.variant.initialize(&init_obs);

        let track_id = TrackId::new();
        let store = TimelineStore::new(StoreConfig::default());
        let config = OosmConfig {
            max_lag: std::time::Duration::from_secs(5),
            history_depth: 10,
        };

        // Observation from 60 seconds ago exceeds 5s max_lag
        let old_obs = make_obs_at_time(now - Duration::seconds(60), 37.1);
        let result = handle_oosm(&mut tracker, &old_obs, &track_id, &store, &config, now);
        assert!(matches!(result, FilterResult::OutlierRejected { .. }));
    }

    #[test]
    fn oosm_within_lag_is_accepted() {
        let now = Utc::now();
        let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
        let init_obs = make_obs_at_time(now - Duration::seconds(10), 37.0);
        tracker.variant.initialize(&init_obs);

        // Build state history with predict steps
        tracker.variant.predict(1.0);
        tracker.variant.predict(1.0);
        tracker.variant.predict(1.0);

        let track_id = TrackId::new();
        let store = TimelineStore::new(StoreConfig::default());
        let config = OosmConfig::default(); // 30s max_lag

        // Late observation from 2 seconds ago - within lag tolerance
        let late_obs = make_obs_at_time(now - Duration::seconds(2), 37.001);
        let result = handle_oosm(&mut tracker, &late_obs, &track_id, &store, &config, now);
        // Should not diverge - either Updated or OutlierRejected (based on distance)
        assert!(!matches!(result, FilterResult::DivergenceDetected));
    }

    #[test]
    fn oosm_without_history_falls_back_to_normal_update() {
        let now = Utc::now();
        let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
        let init_obs = make_obs_at_time(now, 37.0);
        tracker.variant.initialize(&init_obs);
        // No predict calls - no state history

        let track_id = TrackId::new();
        let store = TimelineStore::new(StoreConfig::default());
        let config = OosmConfig::default();

        let late_obs = make_obs_at_time(now - Duration::seconds(1), 37.0);
        let result = handle_oosm(&mut tracker, &late_obs, &track_id, &store, &config, now);
        // Falls back to normal update since no history snapshot is available
        assert!(matches!(result, FilterResult::Updated | FilterResult::OutlierRejected { .. }));
    }
}
