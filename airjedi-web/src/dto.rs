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
    pub emergency: Option<bool>,
    pub alert: Option<bool>,
    pub category: Option<u8>,
    pub airspeed: Option<f64>,
    pub roll_angle: Option<f64>,
    pub track_angle_rate: Option<f64>,
    pub selected_altitude: Option<i32>,
    pub barometric_setting: Option<f64>,
    pub wind_speed: Option<u16>,
    pub wind_direction: Option<f64>,
    pub temperature: Option<f64>,
    pub signal_level: Option<f32>,
    pub distance_nm: Option<f64>,
}

fn haversine_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r_nm = 3440.065;
    let d_lat = (lat2 - lat1).to_radians();
    let d_lon = (lon2 - lon1).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    r_nm * 2.0 * a.sqrt().asin()
}

impl AircraftDto {
    pub fn from_aircraft(
        ac: &adsb_client::Aircraft,
        max_trail: usize,
        center: Option<(f64, f64)>,
    ) -> Self {
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

        let distance_nm = match (ac.latitude, ac.longitude, center) {
            (Some(lat), Some(lon), Some((clat, clon))) => Some(haversine_nm(clat, clon, lat, lon)),
            _ => None,
        };

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
            emergency: ac.emergency,
            alert: ac.alert,
            category: ac.category,
            airspeed: ac.airspeed,
            roll_angle: ac.roll_angle,
            track_angle_rate: ac.track_angle_rate,
            selected_altitude: ac.selected_altitude,
            barometric_setting: ac.barometric_setting,
            wind_speed: ac.wind_speed,
            wind_direction: ac.wind_direction,
            temperature: ac.temperature,
            signal_level: ac.signal_level,
            distance_nm,
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
