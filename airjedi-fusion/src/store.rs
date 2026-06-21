use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use crate::prelude_imports::*;
use crate::sensor::SensorObservation;
use crate::types::{TrackId, Timestamp};

#[derive(Debug, Clone)]
pub struct StoredObservation {
    pub observation: SensorObservation,
    pub associated_track: Option<TrackId>,
    pub store_index: usize,
}

#[derive(Clone, Debug)]
pub struct StoreConfig {
    pub hot_retention: Duration,
    pub max_observations_per_track: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            hot_retention: Duration::from_secs(60),
            max_observations_per_track: 1000,
        }
    }
}

#[derive(Resource)]
pub struct TimelineStore {
    by_track: HashMap<TrackId, VecDeque<StoredObservation>>,
    unassociated_obs: Vec<StoredObservation>,
    next_index: usize,
    config: StoreConfig,
}

impl TimelineStore {
    #[must_use]
    pub fn new(config: StoreConfig) -> Self {
        Self {
            by_track: HashMap::new(),
            unassociated_obs: Vec::new(),
            next_index: 0,
            config,
        }
    }

    pub fn insert(&mut self, observation: SensorObservation) {
        let stored = StoredObservation {
            observation,
            associated_track: None,
            store_index: self.next_index,
        };
        self.next_index += 1;
        self.unassociated_obs.push(stored);
    }

    pub fn associate(&mut self, unassociated_idx: usize, track_id: &TrackId) {
        if unassociated_idx >= self.unassociated_obs.len() {
            return;
        }
        let mut obs = self.unassociated_obs.remove(unassociated_idx);
        obs.associated_track = Some(track_id.clone());

        let buffer = self
            .by_track
            .entry(track_id.clone())
            .or_default();

        if buffer.len() >= self.config.max_observations_per_track {
            buffer.pop_front();
        }
        buffer.push_back(obs);
    }

    #[must_use]
    pub fn query_range(
        &self,
        track_id: &TrackId,
        from: Timestamp,
        to: Timestamp,
    ) -> Vec<&StoredObservation> {
        self.by_track
            .get(track_id)
            .map(|buf| {
                buf.iter()
                    .filter(|o| o.observation.timestamp >= from && o.observation.timestamp <= to)
                    .collect()
            })
            .unwrap_or_default()
    }

    #[must_use]
    pub fn latest_per_sensor(
        &self,
        track_id: &TrackId,
    ) -> HashMap<String, &StoredObservation> {
        let mut latest: HashMap<String, &StoredObservation> = HashMap::new();
        if let Some(buf) = self.by_track.get(track_id) {
            for obs in buf.iter().rev() {
                let key = obs.observation.sensor_id.id.clone();
                latest.entry(key).or_insert(obs);
            }
        }
        latest
    }

    #[must_use]
    pub fn unassociated(&self) -> &[StoredObservation] {
        &self.unassociated_obs
    }

    pub fn evict_old(&mut self, now: Timestamp) {
        let cutoff = now
            - chrono::Duration::from_std(self.config.hot_retention)
                .unwrap_or(chrono::Duration::seconds(60));

        for buffer in self.by_track.values_mut() {
            while let Some(front) = buffer.front() {
                if front.observation.timestamp < cutoff {
                    buffer.pop_front();
                } else {
                    break;
                }
            }
        }

        self.unassociated_obs
            .retain(|o| o.observation.timestamp >= cutoff);
    }

    pub fn evict_and_collect(&mut self, now: Timestamp) -> Vec<StoredObservation> {
        let cutoff = now
            - chrono::Duration::from_std(self.config.hot_retention)
                .unwrap_or(chrono::Duration::seconds(60));

        let mut evicted = Vec::new();

        for buffer in self.by_track.values_mut() {
            while let Some(front) = buffer.front() {
                if front.observation.timestamp < cutoff {
                    if let Some(obs) = buffer.pop_front() {
                        evicted.push(obs);
                    }
                } else {
                    break;
                }
            }
        }

        let split_idx = self
            .unassociated_obs
            .iter()
            .position(|o| o.observation.timestamp >= cutoff)
            .unwrap_or(self.unassociated_obs.len());
        let old_unassociated: Vec<_> = self.unassociated_obs.drain(..split_idx).collect();
        evicted.extend(old_unassociated);

        evicted
    }

    #[must_use]
    pub fn track_observation_count(&self, track_id: &TrackId) -> usize {
        self.by_track.get(track_id).map_or(0, VecDeque::len)
    }

    #[must_use]
    pub fn total_observation_count(&self) -> usize {
        let associated: usize = self.by_track.values().map(VecDeque::len).sum();
        associated + self.unassociated_obs.len()
    }

    pub fn clear_unassociated(&mut self) {
        self.unassociated_obs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::CoordinateFrame;
    use crate::sensor::*;
    use crate::types::*;
    use chrono::Utc;
    use nalgebra::DMatrix;

    fn make_test_obs(sensor: &str) -> SensorObservation {
        SensorObservation {
            sensor_id: SensorId {
                id: sensor.to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: None,
            measurement: Measurement::PositionVelocity3D {
                lat_deg: 37.0,
                lon_deg: -97.0,
                alt_m: Some(10000.0),
                vel_north_mps: None,
                vel_east_mps: None,
                vel_down_mps: None,
                heading_deg: None,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(3, 3),
            },
            classification_hint: None,
            metadata: ObservationMetadata::default(),
        }
    }

    #[test]
    fn insert_and_retrieve_unassociated() {
        let mut store = TimelineStore::new(StoreConfig::default());
        store.insert(make_test_obs("s1"));
        store.insert(make_test_obs("s2"));
        assert_eq!(store.unassociated().len(), 2);
        assert_eq!(store.total_observation_count(), 2);
    }

    #[test]
    fn associate_moves_to_track() {
        let mut store = TimelineStore::new(StoreConfig::default());
        store.insert(make_test_obs("s1"));
        assert_eq!(store.unassociated().len(), 1);

        let track_id = TrackId::new();
        store.associate(0, &track_id);
        assert_eq!(store.unassociated().len(), 0);
        assert_eq!(store.track_observation_count(&track_id), 1);

        let range = store.query_range(
            &track_id,
            Utc::now() - chrono::Duration::seconds(10),
            Utc::now() + chrono::Duration::seconds(10),
        );
        assert_eq!(range.len(), 1);
    }

    #[test]
    fn latest_per_sensor_returns_most_recent() {
        let mut store = TimelineStore::new(StoreConfig::default());
        let track_id = TrackId::new();

        let mut obs1 = make_test_obs("s1");
        obs1.timestamp = Utc::now() - chrono::Duration::seconds(5);
        store.insert(obs1);
        store.associate(0, &track_id);

        let mut obs2 = make_test_obs("s1");
        obs2.timestamp = Utc::now();
        store.insert(obs2);
        store.associate(0, &track_id);

        let latest = store.latest_per_sensor(&track_id);
        assert_eq!(latest.len(), 1);
        assert!(latest.contains_key("s1"));
    }

    #[test]
    fn multiple_sensors_per_track() {
        let mut store = TimelineStore::new(StoreConfig::default());
        let track_id = TrackId::new();

        store.insert(make_test_obs("adsb"));
        store.associate(0, &track_id);
        store.insert(make_test_obs("radar"));
        store.associate(0, &track_id);

        let latest = store.latest_per_sensor(&track_id);
        assert_eq!(latest.len(), 2);
        assert!(latest.contains_key("adsb"));
        assert!(latest.contains_key("radar"));
    }

    #[test]
    fn respects_max_observations_per_track() {
        let config = StoreConfig {
            max_observations_per_track: 3,
            ..Default::default()
        };
        let mut store = TimelineStore::new(config);
        let track_id = TrackId::new();

        for _ in 0..5 {
            store.insert(make_test_obs("s1"));
            store.associate(0, &track_id);
        }

        assert_eq!(store.track_observation_count(&track_id), 3);
    }

    #[test]
    fn associate_out_of_bounds_is_noop() {
        let mut store = TimelineStore::new(StoreConfig::default());
        let track_id = TrackId::new();
        store.associate(99, &track_id);
        assert_eq!(store.total_observation_count(), 0);
    }
}
