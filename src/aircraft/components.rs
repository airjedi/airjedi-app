use bevy::prelude::*;
use chrono::{DateTime, Utc};

/// Component for aircraft entities
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct Aircraft {
    /// ICAO 24-bit address (hex string)
    pub icao: String,
    /// Callsign (optional)
    pub callsign: Option<String>,
    /// Current latitude in degrees
    pub latitude: f64,
    /// Current longitude in degrees
    pub longitude: f64,
    /// Altitude in feet
    pub altitude: Option<i32>,
    /// Track/heading in degrees (0-360)
    pub heading: Option<f32>,
    /// Ground speed in knots
    pub velocity: Option<f64>,
    /// Vertical rate in feet per minute
    pub vertical_rate: Option<i32>,
    /// Roll angle in degrees (from BEAST BDS 5,0). Positive = right wing down.
    pub roll_angle: Option<f32>,
    /// Track angle rate in degrees/second (from BEAST BDS 5,0). Positive = turning right.
    pub track_angle_rate: Option<f32>,
    /// Timestamp of the last BDS 5,0 roll/turn update
    #[reflect(ignore)]
    pub roll_last_seen: Option<DateTime<Utc>>,
    /// Squawk code (transponder code)
    pub squawk: Option<String>,
    /// Whether the aircraft is on the ground
    pub is_on_ground: Option<bool>,
    /// Alert flag (squawk change)
    pub alert: Option<bool>,
    /// Emergency flag
    pub emergency: Option<bool>,
    /// SPI (Special Position Identification) flag
    pub spi: Option<bool>,
    /// Timestamp of the last ADS-B message received for this aircraft
    #[reflect(ignore)]
    pub last_seen: DateTime<Utc>,
}

/// Component to link aircraft labels to their aircraft
#[derive(Component)]
pub struct AircraftLabel {
    pub aircraft_entity: Entity,
}

/// Links a visual Aircraft entity to its fusion track entity
#[derive(Component, Debug)]
pub struct FusionTrackLink {
    pub track_entity: Entity,
    pub track_id: airjedi_fusion::TrackId,
}

#[derive(Component, Debug, Default)]
pub struct FusionDiagnostics {
    pub filter_type: &'static str,
    pub mode_probabilities: Option<Vec<f64>>,
    pub dominant_mode: Option<usize>,
    pub track_status: Option<airjedi_fusion::TrackStatus>,
    pub observation_count: u32,
}
