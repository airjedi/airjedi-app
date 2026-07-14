// Copyright 2025 Chris Custine
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Aircraft tracking and state management.
//!
//! This module maintains aircraft state from ADS-B messages and emits change events.
//! It provides position validation, history tracking, and spatial filtering.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use log::{info, warn};
use tokio::sync::broadcast;

use crate::protocol::{AircraftMessage, MessagePayload};

// Constants for position validation and tracking
const NAUTICAL_MILE_CONVERSION: f64 = 1.15078; // 1 nautical mile = 1.15078 statute miles
const JUMP_DETECTION_TIME_WINDOW_SECONDS: i64 = 20;
const JUMP_DETECTION_THRESHOLD_MILES: f64 = 10.0;
const MAX_CONSECUTIVE_REJECTIONS: u32 = 3;
const POSITION_CHANGE_THRESHOLD_DEGREES: f64 = 0.001; // ~100 meters at mid-latitudes

/// Calculate distance between two lat/lon points using Haversine formula (in miles).
fn haversine_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 3958.8; // Earth's radius in miles

    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lon = (lon2 - lon1).to_radians();

    let a = (delta_lat / 2.0).sin().powi(2)
        + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    r * c
}

/// Calculate distance in nautical miles between two lat/lon points.
#[must_use]
pub fn haversine_distance_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let statute_miles = haversine_distance(lat1, lon1, lat2, lon2);
    statute_miles / NAUTICAL_MILE_CONVERSION
}

/// A single position sample with timestamp and altitude.
#[derive(Debug, Clone)]
pub struct PositionPoint {
    pub lat: f64,
    pub lon: f64,
    pub altitude: Option<i32>,
    pub timestamp: DateTime<Utc>,
}

/// Aircraft data.
#[derive(Debug, Clone)]
pub struct Aircraft {
    /// ICAO 24-bit address (hex string).
    pub icao: String,
    /// Aircraft callsign.
    pub callsign: Option<String>,
    /// Current latitude in degrees.
    pub latitude: Option<f64>,
    /// Current longitude in degrees.
    pub longitude: Option<f64>,
    /// Current altitude in feet.
    pub altitude: Option<i32>,
    /// Track angle in degrees (0-360, north = 0).
    pub track: Option<f64>,
    /// Ground speed in knots.
    pub velocity: Option<f64>,
    /// Vertical rate in feet per minute.
    pub vertical_rate: Option<i32>,
    /// Squawk code (transponder code).
    pub squawk: Option<String>,
    /// Whether the aircraft is on the ground.
    pub is_on_ground: Option<bool>,
    /// Alert flag (squawk change).
    pub alert: Option<bool>,
    /// Emergency flag.
    pub emergency: Option<bool>,
    /// SPI (Special Position Identification) flag.
    pub spi: Option<bool>,
    /// Aircraft emitter category (from ADS-B).
    pub category: Option<u8>,
    /// Magnetic heading in degrees (from ADS-B velocity subtype 3/4).
    pub heading: Option<f64>,
    /// Indicated or true airspeed in knots (from ADS-B velocity subtype 3/4).
    pub airspeed: Option<f64>,
    /// Last received signal level (0.0-1.0, from BEAST protocol).
    pub signal_level: Option<f32>,
    /// Timestamp of last received message.
    pub last_seen: DateTime<Utc>,
    /// Timestamp of last accepted position update (for jump detection).
    last_position_time: Option<DateTime<Utc>>,
    /// Position history for trail rendering.
    pub position_history: Vec<PositionPoint>,
    /// Counter for consecutive position rejections (internal use).
    consecutive_rejections: u32,
}

impl Aircraft {
    fn new(icao: String) -> Self {
        Self {
            icao,
            callsign: None,
            latitude: None,
            longitude: None,
            altitude: None,
            track: None,
            velocity: None,
            vertical_rate: None,
            squawk: None,
            is_on_ground: None,
            alert: None,
            emergency: None,
            spi: None,
            category: None,
            heading: None,
            airspeed: None,
            signal_level: None,
            last_seen: Utc::now(),
            last_position_time: None,
            position_history: Vec::new(),
            consecutive_rejections: 0,
        }
    }

    /// Calculate distance in nautical miles from a given point to this aircraft.
    #[must_use]
    pub fn distance_from_nm(&self, from_lat: f64, from_lon: f64) -> Option<f64> {
        if let (Some(lat), Some(lon)) = (self.latitude, self.longitude) {
            Some(haversine_distance_nm(from_lat, from_lon, lat, lon))
        } else {
            None
        }
    }

    /// Update position with validation.
    fn update_position(
        &mut self,
        lat: f64,
        lon: f64,
        center_lat: f64,
        center_lon: f64,
        max_distance: f64,
    ) -> bool {
        // Check if position is within max distance from center
        let distance_from_center = haversine_distance(center_lat, center_lon, lat, lon);
        if distance_from_center > max_distance {
            return false;
        }

        // Check if position is within threshold of previous position (only if recent update)
        if let (Some(last_lat), Some(last_lon)) = (self.latitude, self.longitude) {
            let now = Utc::now();
            let time_since_last_position = self
                .last_position_time
                .map(|t| (now - t).num_seconds())
                .unwrap_or(i64::MAX);

            if time_since_last_position <= JUMP_DETECTION_TIME_WINDOW_SECONDS {
                let distance_from_last = haversine_distance(last_lat, last_lon, lat, lon);
                if distance_from_last > JUMP_DETECTION_THRESHOLD_MILES {
                    if self.consecutive_rejections >= MAX_CONSECUTIVE_REJECTIONS {
                        info!(
                            "Accepting position for {} after {} consecutive rejections (jumped {:.1} miles)",
                            self.icao, self.consecutive_rejections, distance_from_last
                        );
                        self.consecutive_rejections = 0;
                    } else {
                        self.consecutive_rejections += 1;
                        warn!(
                            "Rejected position for {}: jumped {:.1} miles (rejection {} of 3)",
                            self.icao, distance_from_last, self.consecutive_rejections
                        );
                        return false;
                    }
                }
            }
        }

        // Only add to history if position has changed significantly
        let should_add = if let (Some(last_lat), Some(last_lon)) = (self.latitude, self.longitude) {
            let distance = ((lat - last_lat).powi(2) + (lon - last_lon).powi(2)).sqrt();
            distance > POSITION_CHANGE_THRESHOLD_DEGREES
        } else {
            true
        };

        if should_add {
            self.position_history.push(PositionPoint {
                lat,
                lon,
                altitude: self.altitude,
                timestamp: Utc::now(),
            });
        }

        self.latitude = Some(lat);
        self.longitude = Some(lon);
        self.last_position_time = Some(Utc::now());
        self.consecutive_rejections = 0;

        true
    }

    fn cleanup_old_history(&mut self, max_age_seconds: i64) {
        let now = Utc::now();
        self.position_history
            .retain(|point| (now - point.timestamp).num_seconds() < max_age_seconds);
    }
}

/// Events emitted by the tracker when aircraft state changes.
#[derive(Debug, Clone)]
pub enum TrackerEvent {
    /// A new aircraft was added to tracking.
    AircraftAdded(String),
    /// An aircraft's position was updated.
    PositionUpdated(String),
    /// An aircraft was removed due to timeout.
    AircraftRemoved(String),
}

/// Configuration for the aircraft tracker.
#[derive(Debug, Clone)]
pub struct TrackerConfig {
    /// Center point for distance filtering (lat, lon).
    pub center: Option<(f64, f64)>,
    /// Maximum distance from center in miles.
    pub max_distance_miles: f64,
    /// Aircraft timeout in seconds.
    pub aircraft_timeout_secs: i64,
    /// Position history retention in seconds.
    pub position_history_secs: i64,
    /// Broadcast channel capacity for events.
    pub event_channel_capacity: usize,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            center: None,
            max_distance_miles: 400.0,
            aircraft_timeout_secs: 180,
            position_history_secs: 300,
            event_channel_capacity: 256,
        }
    }
}

/// Aircraft tracker that maintains state and emits events.
pub struct AircraftTracker {
    aircraft: HashMap<String, Aircraft>,
    center_lat: f64,
    center_lon: f64,
    max_distance_miles: f64,
    aircraft_timeout_secs: i64,
    position_history_secs: i64,
    event_tx: broadcast::Sender<TrackerEvent>,
}

impl std::fmt::Debug for AircraftTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AircraftTracker")
            .field("aircraft_count", &self.aircraft.len())
            .field("center", &(self.center_lat, self.center_lon))
            .field("max_distance_miles", &self.max_distance_miles)
            .finish()
    }
}

impl AircraftTracker {
    /// Create a new tracker with the given configuration.
    #[must_use]
    pub fn new(config: TrackerConfig) -> Self {
        let (center_lat, center_lon) = config.center.unwrap_or((0.0, 0.0));
        let (event_tx, _) = broadcast::channel(config.event_channel_capacity);

        Self {
            aircraft: HashMap::new(),
            center_lat,
            center_lon,
            max_distance_miles: config.max_distance_miles,
            aircraft_timeout_secs: config.aircraft_timeout_secs,
            position_history_secs: config.position_history_secs,
            event_tx,
        }
    }

    /// Set the center point for distance filtering.
    pub fn set_center(&mut self, lat: f64, lon: f64) {
        self.center_lat = lat;
        self.center_lon = lon;
    }

    /// Get the current center point.
    #[must_use]
    pub fn center(&self) -> (f64, f64) {
        (self.center_lat, self.center_lon)
    }

    /// Process an incoming aircraft message.
    pub fn process_message(&mut self, msg: AircraftMessage) {
        let icao = msg.icao().to_string();
        let is_new = !self.aircraft.contains_key(&icao);

        let aircraft = self
            .aircraft
            .entry(icao.clone())
            .or_insert_with(|| Aircraft::new(icao.clone()));

        aircraft.last_seen = Utc::now();

        if let Some(sl) = msg.signal_level {
            aircraft.signal_level = Some(sl);
        }

        if is_new {
            let _ = self.event_tx.send(TrackerEvent::AircraftAdded(icao.clone()));
        }

        match msg.payload {
            MessagePayload::Identification { callsign, category } => {
                let is_adsb_source = category.is_some();
                if is_adsb_source {
                    // ADS-B identification (DF=17 TC 1-4) is authoritative
                    aircraft.callsign = Some(callsign);
                    aircraft.category = category;
                } else if aircraft.callsign.is_none() {
                    // BDS 2,0 callsign only used when no ADS-B callsign exists
                    aircraft.callsign = Some(callsign);
                }
            }
            MessagePayload::Position {
                latitude,
                longitude,
                altitude,
                ground_speed,
                track,
                is_on_ground,
                ..
            } => {
                if let Some(alt) = altitude {
                    aircraft.altitude = Some(alt);
                }
                if let Some(gs) = ground_speed {
                    aircraft.velocity = Some(gs);
                }
                if let Some(trk) = track {
                    aircraft.track = Some(trk);
                }
                if let Some(on_ground) = is_on_ground {
                    aircraft.is_on_ground = Some(on_ground);
                }
                let updated = aircraft.update_position(
                    latitude,
                    longitude,
                    self.center_lat,
                    self.center_lon,
                    self.max_distance_miles,
                );
                if updated {
                    let _ = self.event_tx.send(TrackerEvent::PositionUpdated(icao));
                }
            }
            MessagePayload::Velocity {
                speed,
                track,
                vertical_rate,
                is_on_ground,
                heading,
                airspeed,
            } => {
                aircraft.velocity = Some(speed);
                aircraft.track = Some(track);
                aircraft.vertical_rate = vertical_rate;
                if let Some(on_ground) = is_on_ground {
                    aircraft.is_on_ground = Some(on_ground);
                }
                if let Some(hdg) = heading {
                    aircraft.heading = Some(hdg);
                }
                if let Some(aspd) = airspeed {
                    aircraft.airspeed = Some(aspd);
                }
            }
            MessagePayload::Altitude {
                altitude,
                squawk,
                alert,
                emergency,
                spi,
                is_on_ground,
            } => {
                if let Some(alt) = altitude {
                    aircraft.altitude = Some(alt);
                }
                if let Some(sq) = squawk {
                    aircraft.squawk = Some(sq);
                }
                if let Some(a) = alert {
                    aircraft.alert = Some(a);
                }
                if let Some(e) = emergency {
                    aircraft.emergency = Some(e);
                }
                if let Some(s) = spi {
                    aircraft.spi = Some(s);
                }
                if let Some(on_ground) = is_on_ground {
                    aircraft.is_on_ground = Some(on_ground);
                }
            }
        }
    }

    /// Get all tracked aircraft.
    #[must_use]
    pub fn get_aircraft(&self) -> Vec<&Aircraft> {
        self.aircraft.values().collect()
    }

    /// Get a specific aircraft by ICAO address.
    #[must_use]
    pub fn get_by_icao(&self, icao: &str) -> Option<&Aircraft> {
        self.aircraft.get(icao)
    }

    /// Get the number of tracked aircraft.
    #[must_use]
    pub fn len(&self) -> usize {
        self.aircraft.len()
    }

    /// Check if there are no tracked aircraft.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.aircraft.is_empty()
    }

    /// Subscribe to tracker events.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<TrackerEvent> {
        self.event_tx.subscribe()
    }

    /// Remove stale aircraft and clean up old position history.
    pub fn cleanup_stale(&mut self) {
        let now = Utc::now();

        // Clean up old position history
        for aircraft in self.aircraft.values_mut() {
            aircraft.cleanup_old_history(self.position_history_secs);
        }

        // Remove aircraft that haven't been seen recently
        let removed: Vec<_> = self
            .aircraft
            .iter()
            .filter(|(_, a)| (now - a.last_seen).num_seconds() >= self.aircraft_timeout_secs)
            .map(|(icao, _)| icao.clone())
            .collect();

        for icao in removed {
            self.aircraft.remove(&icao);
            let _ = self.event_tx.send(TrackerEvent::AircraftRemoved(icao));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_haversine_distance() {
        // LAX to JFK is approximately 2,475 miles
        let distance = haversine_distance(33.9425, -118.4081, 40.6413, -73.7781);
        assert!((distance - 2475.0).abs() < 10.0);
    }

    #[test]
    fn test_tracker_new_aircraft() {
        let mut tracker = AircraftTracker::new(TrackerConfig::default());

        tracker.process_message(AircraftMessage {
            icao: "A1B2C3".to_string(),
            signal_level: None,
            payload: MessagePayload::Identification {
                callsign: "UAL123".to_string(),
                category: None,
            },
        });

        assert_eq!(tracker.len(), 1);
        let aircraft = tracker.get_by_icao("A1B2C3").unwrap();
        assert_eq!(aircraft.callsign.as_deref(), Some("UAL123"));
    }

    #[test]
    fn test_tracker_position_update() {
        let mut tracker = AircraftTracker::new(TrackerConfig {
            center: Some((33.9425, -118.4081)),
            ..Default::default()
        });

        tracker.process_message(AircraftMessage {
            icao: "A1B2C3".to_string(),
            signal_level: None,
            payload: MessagePayload::Position {
                latitude: 34.0,
                longitude: -118.5,
                altitude: Some(35000),
                ground_speed: None,
                track: None,
                is_on_ground: None,
                altitude_gnss: None,
            },
        });

        let aircraft = tracker.get_by_icao("A1B2C3").unwrap();
        assert_eq!(aircraft.latitude, Some(34.0));
        assert_eq!(aircraft.longitude, Some(-118.5));
        assert_eq!(aircraft.altitude, Some(35000));
    }

    #[test]
    fn test_position_rejected_too_far() {
        let mut tracker = AircraftTracker::new(TrackerConfig {
            center: Some((33.9425, -118.4081)),
            max_distance_miles: 100.0,
            ..Default::default()
        });

        // Position far from center should be rejected (LAX to NYC)
        tracker.process_message(AircraftMessage {
            icao: "A1B2C3".to_string(),
            signal_level: None,
            payload: MessagePayload::Position {
                latitude: 40.6413,
                longitude: -73.7781,
                altitude: Some(35000),
                ground_speed: None,
                track: None,
                is_on_ground: None,
                altitude_gnss: None,
            },
        });

        let aircraft = tracker.get_by_icao("A1B2C3").unwrap();
        assert!(aircraft.latitude.is_none());
    }
}
