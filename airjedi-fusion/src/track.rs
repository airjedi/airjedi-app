use crate::prelude_imports::*;
use std::collections::HashMap;
use std::time::Duration;
use crate::types::{TargetCategory, TargetId, Timestamp, TrackId};

#[derive(Component, Debug, Clone)]
pub struct Track {
    pub id: TrackId,
    pub cooperative_ids: Vec<TargetId>,
    pub created_at: Timestamp,
    pub last_update: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub enum TrackStatus {
    Tentative,
    Confirmed,
    Coasting,
    Lost,
}

#[derive(Component, Debug, Clone, Reflect)]
pub struct TrackQuality {
    pub status: TrackStatus,
    pub sensor_count: u8,
    pub update_rate: f32,
    #[reflect(ignore)]
    pub staleness: Duration,
    pub confidence: f32,
    pub observation_count: u32,
}

impl Default for TrackQuality {
    fn default() -> Self {
        Self {
            status: TrackStatus::Tentative,
            sensor_count: 0,
            update_rate: 0.0,
            staleness: Duration::ZERO,
            confidence: 0.0,
            observation_count: 0,
        }
    }
}

impl TrackQuality {
    pub fn transition(&mut self, staleness: Duration, config: &TrackLifecycleConfig) {
        self.staleness = staleness;
        match self.status {
            TrackStatus::Tentative => {
                if self.observation_count >= config.confirm_threshold {
                    self.status = TrackStatus::Confirmed;
                }
            }
            TrackStatus::Confirmed => {
                if staleness > config.coast_timeout {
                    self.status = TrackStatus::Coasting;
                }
            }
            TrackStatus::Coasting => {
                if staleness > config.coast_timeout + config.lost_timeout {
                    self.status = TrackStatus::Lost;
                }
            }
            TrackStatus::Lost => {}
        }
    }

    pub fn reacquire(&mut self) {
        if self.status == TrackStatus::Coasting {
            self.status = TrackStatus::Confirmed;
        }
        self.staleness = Duration::ZERO;
    }
}

#[derive(Debug, Clone)]
pub struct TrackLifecycleConfig {
    pub confirm_threshold: u32,
    pub confirm_window: Duration,
    pub coast_timeout: Duration,
    pub lost_timeout: Duration,
    pub cleanup_delay: Duration,
}

impl Default for TrackLifecycleConfig {
    fn default() -> Self {
        Self {
            confirm_threshold: 3,
            confirm_window: Duration::from_secs(10),
            coast_timeout: Duration::from_secs(15),
            lost_timeout: Duration::from_secs(60),
            cleanup_delay: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, Resource)]
pub struct LifecycleProfiles {
    pub profiles: HashMap<TargetCategory, TrackLifecycleConfig>,
    pub default_profile: TrackLifecycleConfig,
}

impl Default for LifecycleProfiles {
    fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert(
            TargetCategory::FixedWing,
            TrackLifecycleConfig {
                confirm_threshold: 3,
                confirm_window: Duration::from_secs(10),
                coast_timeout: Duration::from_secs(15),
                lost_timeout: Duration::from_secs(60),
                cleanup_delay: Duration::from_secs(5),
            },
        );
        profiles.insert(
            TargetCategory::Drone,
            TrackLifecycleConfig {
                confirm_threshold: 3,
                confirm_window: Duration::from_secs(5),
                coast_timeout: Duration::from_secs(10),
                lost_timeout: Duration::from_secs(30),
                cleanup_delay: Duration::from_secs(3),
            },
        );
        profiles.insert(
            TargetCategory::Missile,
            TrackLifecycleConfig {
                confirm_threshold: 2,
                confirm_window: Duration::from_secs(3),
                coast_timeout: Duration::from_secs(5),
                lost_timeout: Duration::from_secs(15),
                cleanup_delay: Duration::from_secs(2),
            },
        );
        profiles.insert(
            TargetCategory::SurfaceVessel,
            TrackLifecycleConfig {
                confirm_threshold: 3,
                confirm_window: Duration::from_secs(60),
                coast_timeout: Duration::from_secs(600),
                lost_timeout: Duration::from_secs(7200),
                cleanup_delay: Duration::from_secs(60),
            },
        );
        profiles.insert(
            TargetCategory::GroundVehicle,
            TrackLifecycleConfig {
                confirm_threshold: 3,
                confirm_window: Duration::from_secs(30),
                coast_timeout: Duration::from_secs(300),
                lost_timeout: Duration::from_secs(3600),
                cleanup_delay: Duration::from_secs(30),
            },
        );
        profiles.insert(
            TargetCategory::Person,
            TrackLifecycleConfig {
                confirm_threshold: 3,
                confirm_window: Duration::from_secs(10),
                coast_timeout: Duration::from_secs(30),
                lost_timeout: Duration::from_secs(120),
                cleanup_delay: Duration::from_secs(10),
            },
        );

        Self {
            profiles,
            default_profile: TrackLifecycleConfig::default(),
        }
    }
}

impl LifecycleProfiles {
    #[must_use]
    pub fn get(&self, category: &TargetCategory) -> &TrackLifecycleConfig {
        self.profiles.get(category).unwrap_or(&self.default_profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tentative_to_confirmed() {
        let config = TrackLifecycleConfig::default();
        let mut quality = TrackQuality::default();
        quality.observation_count = 3;
        quality.transition(Duration::from_secs(0), &config);
        assert_eq!(quality.status, TrackStatus::Confirmed);
    }

    #[test]
    fn stays_tentative_below_threshold() {
        let config = TrackLifecycleConfig::default();
        let mut quality = TrackQuality::default();
        quality.observation_count = 2;
        quality.transition(Duration::from_secs(0), &config);
        assert_eq!(quality.status, TrackStatus::Tentative);
    }

    #[test]
    fn confirmed_to_coasting() {
        let config = TrackLifecycleConfig::default();
        let mut quality = TrackQuality {
            status: TrackStatus::Confirmed,
            observation_count: 5,
            ..Default::default()
        };
        quality.transition(Duration::from_secs(20), &config);
        assert_eq!(quality.status, TrackStatus::Coasting);
    }

    #[test]
    fn coasting_to_lost() {
        let config = TrackLifecycleConfig::default();
        let mut quality = TrackQuality {
            status: TrackStatus::Coasting,
            ..Default::default()
        };
        // coast_timeout (15) + lost_timeout (60) = 75s
        quality.transition(Duration::from_secs(80), &config);
        assert_eq!(quality.status, TrackStatus::Lost);
    }

    #[test]
    fn coasting_reacquire() {
        let mut quality = TrackQuality {
            status: TrackStatus::Coasting,
            staleness: Duration::from_secs(30),
            ..Default::default()
        };
        quality.reacquire();
        assert_eq!(quality.status, TrackStatus::Confirmed);
        assert_eq!(quality.staleness, Duration::ZERO);
    }

    #[test]
    fn reacquire_only_from_coasting() {
        let mut quality = TrackQuality {
            status: TrackStatus::Tentative,
            staleness: Duration::from_secs(5),
            ..Default::default()
        };
        quality.reacquire();
        assert_eq!(quality.status, TrackStatus::Tentative);
    }

    #[test]
    fn lifecycle_profiles_per_category() {
        let profiles = LifecycleProfiles::default();
        let missile = profiles.get(&TargetCategory::Missile);
        let vessel = profiles.get(&TargetCategory::SurfaceVessel);
        assert!(missile.lost_timeout < vessel.lost_timeout);
        assert!(missile.confirm_threshold < vessel.confirm_threshold);
    }

    #[test]
    fn lifecycle_profiles_unknown_returns_default() {
        let profiles = LifecycleProfiles::default();
        let unknown = profiles.get(&TargetCategory::Unknown);
        assert_eq!(unknown.confirm_threshold, 3);
        assert_eq!(unknown.coast_timeout, Duration::from_secs(15));
    }
}
