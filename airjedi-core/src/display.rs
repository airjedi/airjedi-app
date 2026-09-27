//! The serializable display-component set: the projection boundary from fused
//! state to render-ready state.
//!
//! Design rules (see the Phase 1 plan):
//! - All four are ECS `Component`s and all are `Serialize`/`Deserialize`, so the
//!   same components work in fat mode (written into the app `World`) and thin
//!   mode (replicated agent -> client).
//! - Cross-references use [`TrackId`], never a raw `Entity` - an `Entity` id is
//!   meaningless across the replication boundary.
//! - Nothing that cannot cross the wire appears here: no `nalgebra` matrices, no
//!   `Entity`, no `bevy_asset::Handle`. That is why the covariance and the
//!   forward prediction are pre-reduced to scalars/samples agent-side before
//!   they land in these components.
//!
//! These components are defined now (Phase 1 Task 1) and populated by later
//! tasks; until then they simply exist and compile.

use bevy_ecs::prelude::Component;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::TrackId;
use crate::observation::ObservationFreshness;
use crate::sensor_kind::SensorKind;
use crate::source::PositionSource;
use crate::status::TrackStatus;

/// Complete render-ready track state (supersedes the app's `Aircraft` +
/// `FusionDiagnostics` for display purposes). One per visible target.
#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct DisplayTrack {
    /// Stable cross-boundary identity. The UI keys everything off this, never
    /// off the local `Entity`.
    pub track_id: TrackId,

    pub icao: String,
    pub callsign: Option<String>,

    pub latitude: f64,
    pub longitude: f64,
    pub altitude_ft: Option<i32>,

    /// Latest independent measurement timing for each display field.
    pub position_freshness: Option<ObservationFreshness>,
    pub altitude_freshness: Option<ObservationFreshness>,
    pub velocity_freshness: Option<ObservationFreshness>,

    pub heading: Option<f32>,
    pub velocity_kts: Option<f64>,
    pub vertical_rate: Option<i32>,
    pub roll_angle: Option<f32>,
    pub track_angle_rate: Option<f32>,

    pub squawk: Option<String>,
    pub is_on_ground: Option<bool>,
    pub alert: Option<bool>,
    pub emergency: Option<bool>,
    pub spi: Option<bool>,

    pub last_seen: DateTime<Utc>,
    pub status: TrackStatus,

    /// ADS-B / MLAT / TIS-B, from the enrichment join.
    pub position_source: Option<PositionSource>,

    /// 1-sigma horizontal position uncertainty in meters. Pre-reduced from the
    /// filter covariance agent-side (absorbs `uncertainty_viz`'s scalar output).
    pub h_uncertainty_m: Option<f64>,

    /// Whether the client is currently dead-reckoning this track between updates
    /// (absorbs `interpolation`'s flag).
    pub predicting: bool,

    /// Filter identity + IMM diagnostics, pre-reduced from `TrackerState`.
    /// `String` (not `&'static str`) so it round-trips through serialization.
    pub filter_type: String,
    pub mode_probabilities: Option<Vec<f64>>,
    pub dominant_mode: Option<usize>,
    pub observation_count: u32,
}

/// Forward-predicted track samples for the estimated-track cone (absorbs
/// `estimated_track`). The heavy `predict()` sampling runs agent-side; the
/// client only draws these points.
#[derive(Component, Debug, Clone, Default, Serialize, Deserialize)]
pub struct DisplayEstimate {
    pub samples: Vec<PredictedSample>,
    /// Maneuver probability, drives cone coloring client-side.
    pub maneuver_prob: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictedSample {
    pub lat: f64,
    pub lon: f64,
    pub h_uncertainty_m: f64,
    pub heading_deg: f32,
    /// Seconds ahead of "now" this sample predicts.
    pub time_ahead: f32,
}

/// Per-sensor contributions to a fused track (absorbs `multi_sensor_debug`).
/// The client draws a marker/line per source; the agent decides membership.
#[derive(Component, Debug, Clone, Default, Serialize, Deserialize)]
pub struct SensorContributions {
    pub sources: Vec<SensorReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorReport {
    pub sensor_id: String,
    pub lat: f64,
    pub lon: f64,
    pub kind: SensorKind,
}

/// Bounded position history for connecting a track's recent path. Kept bounded
/// (e.g. ~200 points) so it stays cheap to replicate.
#[derive(Component, Debug, Clone, Default, Serialize, Deserialize)]
pub struct DisplayTrail {
    pub points: Vec<TrailPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrailPoint {
    pub lat: f64,
    pub lon: f64,
    pub alt_ft: Option<i32>,
    pub ts: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_track_round_trips_through_json() {
        let track = DisplayTrack {
            track_id: TrackId::new(),
            icao: "ae5e13".to_string(),
            callsign: Some("N123AB".to_string()),
            latitude: 37.8233,
            longitude: -97.1529,
            altitude_ft: Some(30_000),
            position_freshness: None,
            altitude_freshness: None,
            velocity_freshness: None,
            heading: Some(270.0),
            velocity_kts: Some(420.0),
            vertical_rate: Some(-64),
            roll_angle: None,
            track_angle_rate: None,
            squawk: Some("1200".to_string()),
            is_on_ground: Some(false),
            alert: Some(false),
            emergency: Some(false),
            spi: Some(false),
            last_seen: Utc::now(),
            status: TrackStatus::Confirmed,
            position_source: Some(PositionSource::Mlat),
            h_uncertainty_m: Some(250.0),
            predicting: false,
            filter_type: "IMM".to_string(),
            mode_probabilities: Some(vec![0.7, 0.3]),
            dominant_mode: Some(0),
            observation_count: 42,
        };

        let json = serde_json::to_string(&track).expect("serialize");
        let back: DisplayTrack = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.track_id, track.track_id);
        assert_eq!(back.icao, track.icao);
        assert_eq!(back.position_source, track.position_source);
        assert_eq!(back.status, track.status);
        assert_eq!(back.mode_probabilities, track.mode_probabilities);
    }

    #[test]
    fn estimate_and_contributions_default_empty() {
        assert!(DisplayEstimate::default().samples.is_empty());
        assert!(SensorContributions::default().sources.is_empty());
        assert!(DisplayTrail::default().points.is_empty());
    }
}
