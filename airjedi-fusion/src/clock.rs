use crate::types::Timestamp;
use bevy_ecs::prelude::Resource;
use bevy_time::prelude::Time;
use chrono::Utc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSource {
    Live,
    Fixed,
}

/// Shared UTC and monotonic timing input for fusion lifecycle systems.
#[derive(Resource, Debug, Clone)]
pub struct FusionClock {
    now_utc: Timestamp,
    delta_secs: f64,
    source: ClockSource,
}

impl Default for FusionClock {
    fn default() -> Self {
        Self {
            now_utc: Utc::now(),
            delta_secs: 0.0,
            source: ClockSource::Live,
        }
    }
}

impl FusionClock {
    #[must_use]
    pub fn now_utc(&self) -> Timestamp {
        self.now_utc
    }

    #[must_use]
    pub fn delta_secs_f64(&self) -> f64 {
        self.delta_secs
    }

    #[must_use]
    pub fn source(&self) -> ClockSource {
        self.source
    }

    #[must_use]
    pub fn fixed(now_utc: Timestamp) -> Self {
        Self {
            now_utc,
            delta_secs: 0.0,
            source: ClockSource::Fixed,
        }
    }

    pub fn advance_to(&mut self, now_utc: Timestamp, delta_secs: f64) {
        self.now_utc = now_utc;
        self.delta_secs = delta_secs;
    }
}

/// Refresh live fusion time once per frame. Fixed clocks are test/replay owned.
pub fn advance_fusion_clock(
    mut clock: bevy_ecs::prelude::ResMut<FusionClock>,
    time: bevy_ecs::prelude::Res<Time>,
) {
    if clock.source == ClockSource::Fixed {
        return;
    }
    clock.now_utc = Utc::now();
    clock.delta_secs = time.delta_secs_f64();
}
