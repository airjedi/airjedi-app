use crate::sensor::SensorObservation;
use crate::types::{Timestamp, TrackId};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct InitiationConfig {
    pub required_detections: u32,
    pub scan_window: Duration,
    pub max_candidates: usize,
}

impl Default for InitiationConfig {
    fn default() -> Self {
        Self {
            required_detections: 3,
            scan_window: Duration::from_secs(10),
            max_candidates: 1000,
        }
    }
}

#[derive(Debug, Clone)]
struct CandidateTrack {
    detections: Vec<Timestamp>,
    last_observation: SensorObservation,
    created_at: Timestamp,
}

#[derive(Debug, Clone)]
pub struct MofNInitiator {
    candidates: HashMap<String, CandidateTrack>,
    config: InitiationConfig,
}

impl MofNInitiator {
    #[must_use]
    pub fn new(config: InitiationConfig) -> Self {
        Self {
            candidates: HashMap::new(),
            config,
        }
    }

    pub fn process_observation(
        &mut self,
        obs: &SensorObservation,
        now: Timestamp,
    ) -> InitiationDecision {
        let key = match &obs.target_id {
            Some(tid) => tid.id.clone(),
            None => return InitiationDecision::SinglePoint,
        };

        let entry = self.candidates.entry(key.clone()).or_insert_with(|| {
            CandidateTrack {
                detections: Vec::new(),
                last_observation: obs.clone(),
                created_at: now,
            }
        });

        entry.last_observation = obs.clone();
        entry.detections.push(now);

        let window_start = now - chrono::Duration::from_std(self.config.scan_window).unwrap_or(chrono::Duration::seconds(10));
        entry.detections.retain(|t| *t >= window_start);

        if entry.detections.len() >= self.config.required_detections as usize {
            let obs = entry.last_observation.clone();
            self.candidates.remove(&key);
            return InitiationDecision::Promote(obs);
        }

        InitiationDecision::Pending
    }

    pub fn evict_stale(&mut self, now: Timestamp) {
        let window = chrono::Duration::from_std(self.config.scan_window)
            .unwrap_or(chrono::Duration::seconds(10));
        self.candidates.retain(|_, c| {
            now.signed_duration_since(c.created_at) <= window
        });

        while self.candidates.len() > self.config.max_candidates {
            if let Some(oldest_key) = self
                .candidates
                .iter()
                .min_by_key(|(_, c)| c.created_at)
                .map(|(k, _)| k.clone())
            {
                self.candidates.remove(&oldest_key);
            } else {
                break;
            }
        }
    }

    #[must_use]
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }
}

#[derive(Debug, Clone)]
pub enum InitiationDecision {
    Promote(SensorObservation),
    Pending,
    SinglePoint,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::CoordinateFrame;
    use crate::sensor::*;
    use crate::types::*;
    use chrono::Utc;
    use nalgebra::DMatrix;

    fn make_obs(icao: &str) -> SensorObservation {
        SensorObservation {
            sensor_id: SensorId {
                id: "test".to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: Some(TargetId {
                domain: TargetDomain::Air,
                id: icao.to_string(),
                id_type: IdentifierType::Icao,
            }),
            measurement: Measurement::PositionVelocity3D {
                lat_deg: 37.6872,
                lon_deg: -97.3301,
                alt_m: Some(10000.0),
                vel_north_mps: Some(100.0),
                vel_east_mps: Some(0.0),
                vel_down_mps: Some(0.0),
                heading_deg: None,
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(6, 6) * 100.0,
            },
            classification_hint: Some(TargetCategory::FixedWing),
            metadata: ObservationMetadata::default(),
        }
    }

    #[test]
    fn first_detection_is_pending() {
        let mut initiator = MofNInitiator::new(InitiationConfig::default());
        let obs = make_obs("ABC123");
        let now = Utc::now();
        let result = initiator.process_observation(&obs, now);
        assert!(matches!(result, InitiationDecision::Pending));
        assert_eq!(initiator.candidate_count(), 1);
    }

    #[test]
    fn promotes_after_m_detections() {
        let config = InitiationConfig {
            required_detections: 3,
            scan_window: Duration::from_secs(10),
            ..Default::default()
        };
        let mut initiator = MofNInitiator::new(config);
        let obs = make_obs("ABC123");
        let now = Utc::now();

        assert!(matches!(
            initiator.process_observation(&obs, now),
            InitiationDecision::Pending
        ));
        assert!(matches!(
            initiator.process_observation(&obs, now + chrono::Duration::seconds(1)),
            InitiationDecision::Pending
        ));
        let result = initiator.process_observation(&obs, now + chrono::Duration::seconds(2));
        assert!(matches!(result, InitiationDecision::Promote(_)));
        assert_eq!(initiator.candidate_count(), 0);
    }

    #[test]
    fn different_targets_tracked_separately() {
        let config = InitiationConfig {
            required_detections: 2,
            ..Default::default()
        };
        let mut initiator = MofNInitiator::new(config);
        let now = Utc::now();

        initiator.process_observation(&make_obs("ABC"), now);
        initiator.process_observation(&make_obs("DEF"), now);
        assert_eq!(initiator.candidate_count(), 2);

        let result = initiator.process_observation(&make_obs("ABC"), now + chrono::Duration::seconds(1));
        assert!(matches!(result, InitiationDecision::Promote(_)));
        assert_eq!(initiator.candidate_count(), 1);
    }

    #[test]
    fn evicts_stale_candidates() {
        let config = InitiationConfig {
            required_detections: 3,
            scan_window: Duration::from_secs(5),
            ..Default::default()
        };
        let mut initiator = MofNInitiator::new(config);
        let now = Utc::now();

        initiator.process_observation(&make_obs("OLD"), now);
        assert_eq!(initiator.candidate_count(), 1);

        initiator.evict_stale(now + chrono::Duration::seconds(10));
        assert_eq!(initiator.candidate_count(), 0);
    }

    #[test]
    fn no_target_id_returns_single_point() {
        let mut initiator = MofNInitiator::new(InitiationConfig::default());
        let mut obs = make_obs("X");
        obs.target_id = None;
        let now = Utc::now();
        let result = initiator.process_observation(&obs, now);
        assert!(matches!(result, InitiationDecision::SinglePoint));
    }
}
