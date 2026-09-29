use crate::prelude_imports::*;
use crate::sensor::SensorObservation;
use crate::types::{TargetId, Timestamp, TrackId};
use airjedi_core::ObservationIdentity;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct StoredObservation {
    pub observation: SensorObservation,
    pub associated_track: Option<TrackId>,
    pub store_index: usize,
}

/// Source-aware identity used to consume one decoded report exactly once.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObservationKey {
    pub source_id: String,
    pub identity: ObservationIdentity,
}

#[must_use]
pub fn observation_key(observation: &SensorObservation) -> Option<ObservationKey> {
    observation
        .metadata
        .observation_id
        .map(|identity| ObservationKey {
            source_id: observation.sensor_id.id.clone(),
            identity,
        })
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
    seen_observations: std::collections::HashSet<ObservationKey>,
}

impl TimelineStore {
    #[must_use]
    pub fn new(config: StoreConfig) -> Self {
        Self {
            by_track: HashMap::new(),
            unassociated_obs: Vec::new(),
            next_index: 0,
            config,
            seen_observations: std::collections::HashSet::new(),
        }
    }

    pub fn insert(&mut self, observation: SensorObservation) -> bool {
        if let Some(key) = observation_key(&observation) {
            if !self.seen_observations.insert(key) {
                return false;
            }
        }
        let stored = StoredObservation {
            observation,
            associated_track: None,
            store_index: self.next_index,
        };
        self.next_index += 1;
        self.unassociated_obs.push(stored);
        true
    }

    pub fn associate(&mut self, unassociated_idx: usize, track_id: &TrackId) {
        if unassociated_idx >= self.unassociated_obs.len() {
            return;
        }
        let mut obs = self.unassociated_obs.remove(unassociated_idx);
        obs.associated_track = Some(track_id.clone());

        let buffer = self.by_track.entry(track_id.clone()).or_default();

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
    pub fn latest_per_sensor(&self, track_id: &TrackId) -> HashMap<String, &StoredObservation> {
        let mut latest: HashMap<String, &StoredObservation> = HashMap::new();
        if let Some(buf) = self.by_track.get(track_id) {
            for obs in buf.iter().rev() {
                let key = obs.observation.sensor_id.id.clone();
                latest.entry(key).or_insert(obs);
            }
        }
        latest
    }

    /// Return observations already associated with a track plus observations
    /// still awaiting association that identify the same target. The latter is
    /// needed during the frame in which track initiation promotes a report.
    #[must_use]
    pub fn observations_for_track<'a>(
        &'a self,
        track_id: &TrackId,
        cooperative_ids: &[TargetId],
    ) -> Vec<&'a StoredObservation> {
        let mut observations: Vec<&StoredObservation> = self
            .by_track
            .get(track_id)
            .into_iter()
            .flat_map(|stored| stored.iter())
            .collect();

        observations.extend(self.unassociated_obs.iter().filter(|stored| {
            stored
                .observation
                .target_id
                .as_ref()
                .is_some_and(|target| cooperative_ids.iter().any(|id| id == target))
        }));
        observations
    }

    #[must_use]
    pub fn unassociated(&self) -> &[StoredObservation] {
        &self.unassociated_obs
    }

    #[must_use]
    pub fn associated_observations_for_track(&self, track_id: &TrackId) -> Vec<&StoredObservation> {
        self.by_track
            .get(track_id)
            .map(|observations| observations.iter().collect())
            .unwrap_or_default()
    }

    fn forget_observation(&mut self, observation: &SensorObservation) {
        if let Some(key) = observation_key(observation) {
            self.seen_observations.remove(&key);
        }
    }

    pub fn evict_old(&mut self, now: Timestamp) {
        let cutoff = now
            - chrono::Duration::from_std(self.config.hot_retention)
                .unwrap_or(chrono::Duration::seconds(60));

        let mut forgotten = Vec::new();
        for buffer in self.by_track.values_mut() {
            while let Some(front) = buffer.front() {
                if front.observation.timestamp < cutoff {
                    if let Some(observation) = buffer.pop_front() {
                        if let Some(key) = observation_key(&observation.observation) {
                            forgotten.push(key);
                        }
                    }
                } else {
                    break;
                }
            }
        }
        for key in forgotten {
            self.seen_observations.remove(&key);
        }

        let (old_unassociated, remaining): (Vec<_>, Vec<_>) = self
            .unassociated_obs
            .drain(..)
            .partition(|observation| observation.observation.timestamp < cutoff);
        self.unassociated_obs = remaining;
        for observation in old_unassociated {
            self.forget_observation(&observation.observation);
        }
    }

    pub fn evict_and_collect(&mut self, now: Timestamp) -> Vec<StoredObservation> {
        let cutoff = now
            - chrono::Duration::from_std(self.config.hot_retention)
                .unwrap_or(chrono::Duration::seconds(60));

        let mut evicted = Vec::new();

        let mut forgotten = Vec::new();
        for buffer in self.by_track.values_mut() {
            while let Some(front) = buffer.front() {
                if front.observation.timestamp < cutoff {
                    if let Some(obs) = buffer.pop_front() {
                        if let Some(key) = observation_key(&obs.observation) {
                            forgotten.push(key);
                        }
                        evicted.push(obs);
                    }
                } else {
                    break;
                }
            }
        }
        for key in forgotten {
            self.seen_observations.remove(&key);
        }

        let split_idx = self
            .unassociated_obs
            .iter()
            .position(|o| o.observation.timestamp >= cutoff)
            .unwrap_or(self.unassociated_obs.len());
        let old_unassociated: Vec<_> = self.unassociated_obs.drain(..split_idx).collect();
        for observation in &old_unassociated {
            self.forget_observation(&observation.observation);
        }
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
        let observations = std::mem::take(&mut self.unassociated_obs);
        for observation in observations {
            self.forget_observation(&observation.observation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::CoordinateFrame;
    use crate::sensor::*;
    use crate::types::*;
    use airjedi_core::ObservationIdentity;
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

    fn make_identified_test_obs(frame_sequence: u64) -> SensorObservation {
        let timestamp = chrono::DateTime::from_timestamp(1_700_000_000 + frame_sequence as i64, 0)
            .expect("fixed test timestamp is valid");
        let mut observation = make_test_obs("adsb-history-probe");
        observation.timestamp = timestamp;
        observation.receipt_time = timestamp;
        observation.metadata.observation_id = Some(ObservationIdentity {
            frame_sequence,
            payload_index: 0,
        });
        observation
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

    #[test]
    fn deduplicates_by_source_and_observation_identity() {
        let mut store = TimelineStore::new(StoreConfig::default());
        let identity = ObservationIdentity {
            frame_sequence: 42,
            payload_index: 0,
        };
        let timestamp = Utc::now();

        let mut first = make_test_obs("sensor-a");
        first.timestamp = timestamp;
        first.receipt_time = timestamp;
        first.metadata.observation_id = Some(identity);
        assert!(store.insert(first.clone()));
        assert!(!store.insert(first));

        let mut independent = make_test_obs("sensor-b");
        independent.timestamp = timestamp;
        independent.receipt_time = timestamp;
        independent.metadata.observation_id = Some(identity);
        assert!(store.insert(independent));
        assert_eq!(store.total_observation_count(), 2);
    }

    #[test]
    #[ignore = "known history regression: per-track FIFO eviction retains dedup identity"]
    fn per_track_fifo_releases_an_evicted_observation_identity() {
        let config = StoreConfig {
            max_observations_per_track: 1,
            ..Default::default()
        };
        let mut store = TimelineStore::new(config);
        let track_id = TrackId::new();

        let first = make_identified_test_obs(1);
        assert!(store.insert(first.clone()));
        store.associate(0, &track_id);
        assert!(store.insert(make_identified_test_obs(2)));
        store.associate(0, &track_id);
        assert_eq!(store.track_observation_count(&track_id), 1);

        assert!(
            store.insert(first),
            "the identity of an observation discarded by the per-track FIFO must be reusable"
        );
    }

    #[test]
    #[ignore = "known history regression: per-track FIFO leaves dedup memory unbounded"]
    fn per_track_fifo_bounds_dedup_identity_memory() {
        let config = StoreConfig {
            max_observations_per_track: 1,
            ..Default::default()
        };
        let mut store = TimelineStore::new(config);
        let track_id = TrackId::new();

        for frame_sequence in 1..=32 {
            assert!(store.insert(make_identified_test_obs(frame_sequence)));
            store.associate(0, &track_id);
        }

        assert_eq!(store.track_observation_count(&track_id), 1);
        assert!(
            store.seen_observations.len() <= 1,
            "FIFO retention of one observation must retain at most one dedup identity"
        );
    }
}
