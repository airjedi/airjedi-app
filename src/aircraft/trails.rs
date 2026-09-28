use airjedi_core::{
    AltitudeReference, DisplayHistorySample, DisplayProvenance, DisplayTrail, HistoryBreakReason,
};
use bevy::prelude::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::time::Instant;

const TRAIL_DISCONTINUITY_SECS: i64 = 10;

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
    /// Reference for the altitude when one is present.
    #[serde(default = "default_altitude_reference")]
    pub altitude_reference: AltitudeReference,
    /// Seconds since session start (serializable replacement for Instant)
    pub timestamp: f64,
    /// True when this point was generated from EKF prediction rather than a real observation
    #[serde(default)]
    pub estimated: bool,
    /// Original agent timestamp, retained even though the current renderer
    /// adapts it to the local session clock for opacity calculations.
    #[serde(default)]
    pub timestamp_utc: Option<DateTime<Utc>>,
    /// Stable authoritative sample identity when this point came from the
    /// agent history preview. Playback points have no sequence.
    #[serde(default)]
    pub sample_sequence: Option<u64>,
    #[serde(default)]
    pub segment_id: u32,
    /// Why this point starts a new authoritative continuity segment.
    #[serde(default)]
    pub break_reason: Option<HistoryBreakReason>,
    /// Field-level source metadata retained for render adapters and diagnostics.
    #[serde(default)]
    pub provenance: DisplayProvenance,
}

fn default_altitude_reference() -> AltitudeReference {
    AltitudeReference::Unknown
}

impl TrailPoint {
    fn from_history_sample(
        sample: &DisplayHistorySample,
        server_time: DateTime<Utc>,
        local_now_secs: f64,
    ) -> Self {
        Self {
            lat: sample.latitude,
            lon: sample.longitude,
            altitude: sample.altitude_ft,
            altitude_reference: sample.altitude_reference,
            timestamp: local_timestamp_for_server_time(
                local_now_secs,
                server_time,
                sample.state_time,
            ),
            estimated: sample.estimated,
            timestamp_utc: Some(sample.state_time),
            sample_sequence: Some(sample.sample_sequence),
            segment_id: sample.segment_id,
            break_reason: sample.break_reason,
            provenance: sample.provenance.clone(),
        }
    }

    /// True when the authoritative record says there is no drawable segment
    /// between these points. The timestamp fallback protects older playback
    /// data that predates explicit segment metadata.
    #[must_use]
    pub fn starts_new_segment(&self, previous: &Self) -> bool {
        self.segment_id != previous.segment_id
            || self.break_reason.is_some()
            || self
                .timestamp_utc
                .zip(previous.timestamp_utc)
                .is_some_and(|(current, previous)| {
                    (current - previous).num_seconds() > TRAIL_DISCONTINUITY_SECS
                })
    }
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
    pub const ALL: &'static [TrailRenderer] = &[TrailRenderer::Gizmo, TrailRenderer::MeshStrip];
}

/// Resource for trail configuration
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct TrailConfig {
    pub enabled: bool,
    pub max_age_seconds: u64,
    pub solid_duration_seconds: u64,
    pub fade_duration_seconds: u64,
    #[reflect(ignore)]
    pub renderer_2d: TrailRenderer,
    #[reflect(ignore)]
    pub renderer_3d: TrailRenderer,
    pub trail_width_2d: f32,
    pub trail_width_3d: f32,
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
            trail_width_2d: 2.5,
            trail_width_3d: 6.0,
        }
    }
}

impl TrailHistory {
    /// Add a new point to the trail
    pub fn add_point(
        &mut self,
        lat: f64,
        lon: f64,
        altitude: Option<i32>,
        estimated: bool,
        clock: &SessionClock,
    ) {
        self.points.push_back(TrailPoint {
            lat,
            lon,
            altitude,
            altitude_reference: AltitudeReference::Unknown,
            timestamp: clock.now_secs(),
            estimated,
            timestamp_utc: None,
            sample_sequence: None,
            segment_id: 0,
            break_reason: None,
            provenance: DisplayProvenance::default(),
        });
    }

    /// Materialize a bounded agent preview for rendering. The exact timestamp
    /// remains on each point; `timestamp` is only a local opacity-coordinate
    /// derived from the server time carried by the preview.
    pub fn replace_from_display(&mut self, preview: &DisplayTrail, clock: &SessionClock) {
        let now = clock.now_secs();
        self.points.clear();
        for sample in &preview.samples {
            self.points.push_back(TrailPoint::from_history_sample(
                sample,
                preview.server_time,
                now,
            ));
        }
    }

    /// Materialize the stable client read model without requiring a display
    /// component. This is used when selected-track chunks finish after the
    /// aircraft visual has already been created.
    pub fn replace_from_samples(
        &mut self,
        samples: &[airjedi_core::DisplayHistorySample],
        server_time: DateTime<Utc>,
        clock: &SessionClock,
    ) {
        let now = clock.now_secs();
        self.points.clear();
        for sample in samples {
            self.points
                .push_back(TrailPoint::from_history_sample(sample, server_time, now));
        }
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

/// Convert an authoritative server-time age to the local monotonic trail clock.
/// Keeping this calculation independent from message arrival time prevents a
/// delayed snapshot or correction from making old points appear newly observed.
#[must_use]
pub fn local_timestamp_for_server_time(
    local_now_secs: f64,
    server_time: DateTime<Utc>,
    state_time: DateTime<Utc>,
) -> f64 {
    local_now_secs - (server_time - state_time).num_milliseconds() as f64 / 1000.0
}

/// Return the age-based opacity for one point using the local monotonic clock.
#[must_use]
pub fn point_opacity(
    point: &TrailPoint,
    clock: &SessionClock,
    config: &TrailConfig,
    selected: bool,
) -> f32 {
    let age = clock.age_secs(point.timestamp);
    if selected {
        age_opacity(
            age,
            config.solid_duration_seconds,
            config.fade_duration_seconds,
        )
        .max(0.3)
    } else if age > config.max_age_seconds as f64 {
        0.0
    } else {
        age_opacity(
            age,
            config.solid_duration_seconds,
            config.fade_duration_seconds,
        )
    }
}

/// Iterate only the drawable, contiguous pairs shared by both trail renderers.
/// A missing 3D altitude is a hard break because drawing it at zero would imply
/// an observed sea-level measurement.
pub fn contiguous_trail_pairs<'a>(
    points: &'a VecDeque<TrailPoint>,
    clock: &SessionClock,
    config: &TrailConfig,
    selected: bool,
    is_3d: bool,
) -> Vec<(&'a TrailPoint, &'a TrailPoint)> {
    let mut pairs = Vec::new();
    let mut previous = None;

    for point in points {
        if point_opacity(point, clock, config, selected) <= 0.0
            || (is_3d && point.altitude.is_none())
        {
            previous = None;
            continue;
        }

        if let Some(previous_point) = previous {
            if !point.starts_new_segment(previous_point) {
                pairs.push((previous_point, point));
            }
        }
        previous = Some(point);
    }

    pairs
}

/// Get color for altitude (cyan at low, purple at high)
pub fn altitude_color(altitude: Option<i32>) -> Color {
    let Some(altitude) = altitude else {
        // Unknown altitude is intentionally neutral rather than the low-altitude
        // cyan used for an observed value of exactly zero feet.
        return Color::srgb(0.45, 0.48, 0.55);
    };
    let alt = altitude.max(0) as f32;

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
    mut query: Query<
        (&crate::Aircraft, &mut TrailHistory),
        Without<super::components::AuthoritativeHistory>,
    >,
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
        trail.add_point(
            aircraft.latitude,
            aircraft.longitude,
            aircraft.altitude,
            estimated,
            &clock,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use airjedi_core::{
        DisplayHistorySample, DisplayProvenance, HeadingReference, TrackStatus,
        VerticalRateReference,
    };
    use chrono::{TimeZone, Utc};

    fn sample(
        sequence: u64,
        state_time: DateTime<Utc>,
        segment_id: u32,
        break_reason: Option<HistoryBreakReason>,
        altitude_ft: Option<i32>,
    ) -> DisplayHistorySample {
        DisplayHistorySample {
            sample_sequence: sequence,
            state_time,
            latitude: 37.0 + sequence as f64 * 0.01,
            longitude: -97.0,
            altitude_ft,
            altitude_reference: AltitudeReference::Barometric,
            ground_speed_kts: Some(120.0),
            heading: Some(90.0),
            heading_reference: HeadingReference::GroundTrack,
            vertical_rate: None,
            vertical_rate_reference: VerticalRateReference::FeetPerMinute,
            position_source: None,
            status: TrackStatus::Confirmed,
            estimated: false,
            provenance: DisplayProvenance::default(),
            segment_id,
            break_reason,
        }
    }

    #[test]
    fn server_time_age_is_independent_of_delivery_delay() {
        let server_time = Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap();
        let state_time = server_time - chrono::Duration::seconds(45);
        let local_timestamp = local_timestamp_for_server_time(100.0, server_time, state_time);

        assert_eq!(local_timestamp, 55.0);
    }

    #[test]
    fn correction_replaces_a_sample_without_duplicate_points() {
        let server_time = Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap();
        let original = sample(
            1,
            server_time - chrono::Duration::seconds(20),
            0,
            None,
            Some(10_000),
        );
        let corrected = sample(1, original.state_time, 0, None, Some(12_000));
        let mut history = TrailHistory::default();
        let clock = SessionClock::default();

        history.replace_from_samples(&[original], server_time, &clock);
        history.replace_from_samples(&[corrected], server_time, &clock);

        assert_eq!(history.points.len(), 1);
        assert_eq!(history.points[0].altitude, Some(12_000));
        assert_eq!(history.points[0].sample_sequence, Some(1));
    }

    #[test]
    fn contiguous_pairs_break_on_gap_segment_and_missing_3d_altitude() {
        let server_time = Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap();
        let points = [
            sample(
                1,
                server_time - chrono::Duration::seconds(6),
                0,
                None,
                Some(10_000),
            ),
            sample(
                2,
                server_time - chrono::Duration::seconds(4),
                0,
                None,
                Some(10_000),
            ),
            sample(
                3,
                server_time - chrono::Duration::seconds(2),
                1,
                Some(HistoryBreakReason::Reacquired),
                Some(11_000),
            ),
            sample(4, server_time, 1, None, None),
            sample(
                5,
                server_time + chrono::Duration::seconds(2),
                1,
                None,
                Some(11_000),
            ),
        ];
        let mut history = TrailHistory::default();
        let clock = SessionClock::default();
        history.replace_from_samples(&points, server_time, &clock);
        let config = TrailConfig::default();

        assert_eq!(
            contiguous_trail_pairs(&history.points, &clock, &config, true, false).len(),
            3
        );
        assert_eq!(
            contiguous_trail_pairs(&history.points, &clock, &config, true, true).len(),
            1
        );
    }

    #[test]
    fn missing_altitude_has_distinct_color_from_observed_zero() {
        assert_ne!(altitude_color(None), altitude_color(Some(0)));
    }
}
