//! Injectable clock resource for the projection/derivation systems.
//!
//! Projection systems must never call `chrono::Utc::now()` or read `Time<Real>`
//! directly. A hidden wall-clock dependency is exactly what makes an agent
//! non-replayable: the ingest-simulation suite caught `Rs1090Decoder`
//! timestamping frames with `Utc::now()`, which collapsed every replayed frame
//! to one instant and broke CPR (90 of ~17k positions until the fix). Routing
//! time through a resource means the same systems run identically live and under
//! replay, where a harness drives the clock from fixture timestamps.
//!
//! Live mode: [`advance_sim_clock`] refreshes the resource from the real clock
//! once per frame, before any projection system reads it - so live behavior is
//! byte-identical to reading `Utc::now()` / `Time<Real>` directly.
//!
//! Fixed mode: a replay/snapshot harness inserts a [`SimClock::fixed`] and
//! advances it with [`SimClock::advance_to`]; the live refresh becomes a no-op.

use bevy::prelude::*;
use chrono::{DateTime, Utc};

#[derive(Resource, Debug, Clone)]
pub struct SimClock {
    now_utc: DateTime<Utc>,
    elapsed_secs: f64,
    delta_secs: f64,
    source: ClockSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSource {
    /// Refreshed from wall-clock + `Time<Real>` each frame.
    Live,
    /// Advanced explicitly by a harness; the live refresh leaves it untouched.
    Fixed,
}

impl Default for SimClock {
    fn default() -> Self {
        Self {
            now_utc: Utc::now(),
            elapsed_secs: 0.0,
            delta_secs: 0.0,
            source: ClockSource::Live,
        }
    }
}

impl SimClock {
    /// Wall-clock "now" for this frame (replaces `chrono::Utc::now()`).
    #[must_use]
    pub fn now_utc(&self) -> DateTime<Utc> {
        self.now_utc
    }

    /// Monotonic seconds since startup (replaces `Time<Real>::elapsed_secs_f64`).
    #[must_use]
    pub fn elapsed_secs_f64(&self) -> f64 {
        self.elapsed_secs
    }

    /// Seconds elapsed since the previous frame.
    #[must_use]
    pub fn delta_secs_f64(&self) -> f64 {
        self.delta_secs
    }

    #[must_use]
    pub fn source(&self) -> ClockSource {
        self.source
    }

    /// Create a fixed clock seeded at a known instant, for replay/tests.
    #[must_use]
    pub fn fixed(now_utc: DateTime<Utc>) -> Self {
        Self {
            now_utc,
            elapsed_secs: 0.0,
            delta_secs: 0.0,
            source: ClockSource::Fixed,
        }
    }

    /// Advance a fixed clock to a new wall-clock instant, `dt` seconds later.
    pub fn advance_to(&mut self, now_utc: DateTime<Utc>, dt: f64) {
        self.now_utc = now_utc;
        self.delta_secs = dt;
        self.elapsed_secs += dt;
    }
}

/// Live-mode refresh: pull the real clock into [`SimClock`] once per frame,
/// before any projection system reads it. No-op when the clock is `Fixed`.
pub fn advance_sim_clock(mut clock: ResMut<SimClock>, time: Res<Time<Real>>) {
    if clock.source == ClockSource::Fixed {
        return;
    }
    clock.now_utc = Utc::now();
    clock.delta_secs = time.delta_secs_f64();
    clock.elapsed_secs = time.elapsed_secs_f64();
}
