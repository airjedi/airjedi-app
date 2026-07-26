use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct TrailPointDto {
    pub lat: f64,
    pub lon: f64,
    pub alt: Option<i32>,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AircraftDto {
    pub icao: String,
    pub callsign: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<i32>,
    pub ground_speed: Option<f64>,
    pub heading: Option<f64>,
    pub vertical_rate: Option<i32>,
    pub squawk: Option<String>,
    pub on_ground: Option<bool>,
    pub last_seen: DateTime<Utc>,
    pub trail: Vec<TrailPointDto>,
}

impl AircraftDto {
    pub fn from_aircraft(ac: &adsb_client::Aircraft, max_trail: usize) -> Self {
        let trail_start = ac.position_history.len().saturating_sub(max_trail);
        let trail = ac.position_history[trail_start..]
            .iter()
            .map(|p| TrailPointDto {
                lat: p.lat,
                lon: p.lon,
                alt: p.altitude,
                ts: p.timestamp,
            })
            .collect();

        Self {
            icao: format!("{}", ac.icao),
            callsign: ac.callsign.clone(),
            latitude: ac.latitude,
            longitude: ac.longitude,
            altitude: ac.altitude,
            ground_speed: ac.velocity,
            heading: ac.track.or(ac.heading),
            vertical_rate: ac.vertical_rate,
            squawk: ac.squawk.clone(),
            on_ground: ac.is_on_ground,
            last_seen: ac.last_seen,
            trail,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedStatusDto {
    pub id: Uuid,
    pub address: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Snapshot { aircraft: Vec<AircraftDto> },
    Update { aircraft: Vec<AircraftDto> },
    Remove { icao: Vec<String> },
    Status { feeds: Vec<FeedStatusDto> },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    AddFeed { address: String, protocol: String },
    RemoveFeed { id: Uuid },
}
