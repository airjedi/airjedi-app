use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::time::Instant;

/// Which renderer to use for aircraft trails.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrailRenderer {
    #[default]
    Gizmo,
    MeshStrip,
}

/// Resource providing a session-relative clock for serializable timestamps.
/// Trail points store seconds since this clock's epoch (session start).
#[derive(Resource)]
pub struct SessionClock {
    epoch: Instant,
}

impl Default for SessionClock {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }
}

impl SessionClock {
    /// Current time in seconds since session start.
    pub fn now_secs(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// Age of a timestamp (seconds elapsed since that timestamp).
    pub fn age_secs(&self, timestamp_secs: f64) -> f64 {
        self.now_secs() - timestamp_secs
    }
}

/// A single point in the trail history
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrailPoint {
    pub lat: f64,
    pub lon: f64,
    pub altitude: Option<i32>,
    /// Seconds since session start (serializable replacement for Instant)
    pub timestamp: f64,
    /// True when this point was generated from EKF prediction rather than a real observation
    #[serde(default)]
    pub estimated: bool,
}

/// Component storing trail history for an aircraft
#[derive(Component, Default)]
pub struct TrailHistory {
    pub points: VecDeque<TrailPoint>,
}

impl fmt::Display for TrailRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrailRenderer::Gizmo => write!(f, "Gizmo"),
            TrailRenderer::MeshStrip => write!(f, "Mesh Strip"),
        }
    }
}

impl TrailRenderer {
    pub const ALL: &'static [TrailRenderer] = &[
        TrailRenderer::Gizmo,
        TrailRenderer::MeshStrip,
    ];
}

/// Resource for trail configuration
#[derive(Resource)]
pub struct TrailConfig {
    pub enabled: bool,
    pub max_age_seconds: u64,
    pub solid_duration_seconds: u64,
    pub fade_duration_seconds: u64,
    pub renderer_2d: TrailRenderer,
    pub renderer_3d: TrailRenderer,
}

impl Default for TrailConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_age_seconds: 300,
            solid_duration_seconds: 225,
            fade_duration_seconds: 75,
            renderer_2d: TrailRenderer::Gizmo,
            renderer_3d: TrailRenderer::MeshStrip,
        }
    }
}

impl TrailHistory {
    /// Add a new point to the trail
    pub fn add_point(&mut self, lat: f64, lon: f64, altitude: Option<i32>, estimated: bool, clock: &SessionClock) {
        self.points.push_back(TrailPoint {
            lat,
            lon,
            altitude,
            timestamp: clock.now_secs(),
            estimated,
        });
    }

    /// Remove points older than max_age
    pub fn prune(&mut self, max_age_seconds: u64, clock: &SessionClock) {
        let cutoff = clock.now_secs() - max_age_seconds as f64;
        while let Some(front) = self.points.front() {
            if front.timestamp < cutoff {
                self.points.pop_front();
            } else {
                break;
            }
        }
    }
}

/// Get color for altitude (cyan at low, purple at high)
pub fn altitude_color(altitude: Option<i32>) -> Color {
    let alt = altitude.unwrap_or(0).max(0) as f32;

    // Altitude ranges: 0-10k cyan, 10k-20k green, 20k-30k yellow, 30k-40k orange, 40k+ purple
    let t = (alt / 40000.0).clamp(0.0, 1.0);

    if t < 0.25 {
        // Cyan to green
        let s = t / 0.25;
        Color::srgb(0.0, 1.0 - s * 0.5, 1.0 - s)
    } else if t < 0.5 {
        // Green to yellow
        let s = (t - 0.25) / 0.25;
        Color::srgb(s, 0.5 + s * 0.5, 0.0)
    } else if t < 0.75 {
        // Yellow to orange
        let s = (t - 0.5) / 0.25;
        Color::srgb(1.0, 1.0 - s * 0.4, 0.0)
    } else {
        // Orange to purple
        let s = (t - 0.75) / 0.25;
        Color::srgb(1.0 - s * 0.2, 0.6 - s * 0.6, s)
    }
}

/// Calculate opacity based on age (seconds since the point was recorded).
pub fn age_opacity(age_secs: f64, solid_secs: u64, fade_secs: u64) -> f32 {
    let age = age_secs as f32;
    let solid = solid_secs as f32;
    let fade = fade_secs as f32;

    if age < solid {
        1.0
    } else if age < solid + fade {
        1.0 - (age - solid) / fade
    } else {
        0.0
    }
}

/// Resource to track when we last recorded trail points
#[derive(Resource)]
pub struct TrailRecordTimer {
    pub last_record: Instant,
    pub interval_secs: f32,
}

impl Default for TrailRecordTimer {
    fn default() -> Self {
        Self {
            last_record: Instant::now(),
            interval_secs: 2.0, // Record position every 2 seconds
        }
    }
}

/// System to record aircraft positions into trail history
pub fn record_trail_points(
    mut timer: ResMut<TrailRecordTimer>,
    config: Res<TrailConfig>,
    clock: Res<SessionClock>,
    mut query: Query<(&crate::Aircraft, &mut TrailHistory)>,
) {
    if !config.enabled {
        return;
    }

    let now = Instant::now();
    if now.duration_since(timer.last_record).as_secs_f32() < timer.interval_secs {
        return;
    }
    timer.last_record = now;

    let now_utc = chrono::Utc::now();
    for (aircraft, mut trail) in query.iter_mut() {
        let age_secs = (now_utc - aircraft.last_seen).num_seconds();
        let estimated = age_secs > timer.interval_secs as i64;
        trail.add_point(aircraft.latitude, aircraft.longitude, aircraft.altitude, estimated, &clock);
    }
}
