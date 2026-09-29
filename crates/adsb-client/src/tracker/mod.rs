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

use airjedi_core::{ObservationFreshness, ObservationIdentity, TimeSourceQuality};
use chrono::{DateTime, Utc};
#[cfg(feature = "tracker-jump-detection")]
use log::{info, warn};
use tokio::sync::broadcast;

use crate::protocol::{AircraftMessage, DecodedMessage, Icao, MessagePayload, MessageTiming};

// Constants for position validation and tracking
const NAUTICAL_MILE_CONVERSION: f64 = 1.15078; // 1 nautical mile = 1.15078 statute miles
#[cfg(feature = "tracker-jump-detection")]
const JUMP_DETECTION_TIME_WINDOW_SECONDS: i64 = 20;
#[cfg(feature = "tracker-jump-detection")]
const JUMP_DETECTION_THRESHOLD_MILES: f64 = 10.0;
#[cfg(feature = "tracker-jump-detection")]
const MAX_CONSECUTIVE_REJECTIONS: u32 = 3;
const POSITION_CHANGE_THRESHOLD_DEGREES: f64 = 0.001; // ~100 meters at mid-latitudes

/// Accept a field report once, and only let measurement time move forward.
///
/// Receipt time is deliberately not used to make an older measurement fresh:
/// a delayed report may be live traffic, but it must not replace a newer field
/// value. Distinct reports with the same measurement time remain admissible.
fn accepts_freshness(
    current: Option<ObservationFreshness>,
    candidate: ObservationFreshness,
) -> bool {
    let Some(current) = current else {
        return true;
    };
    if current.identity == candidate.identity {
        return false;
    }

    candidate.observation_time > current.observation_time
        || (candidate.observation_time == current.observation_time
            && candidate.receipt_time >= current.receipt_time)
}

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
    /// ICAO 24-bit aircraft address.
    pub icao: Icao,
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
    /// Roll angle in degrees (from BDS 5,0). Positive = right wing down.
    pub roll_angle: Option<f64>,
    /// Track angle rate in degrees/second (from BDS 5,0). Positive = turning right.
    pub track_angle_rate: Option<f64>,
    /// MCP/FCU selected altitude in feet (from BDS 4,0).
    pub selected_altitude: Option<i32>,
    /// Barometric pressure setting in hPa (from BDS 4,0).
    pub barometric_setting: Option<f64>,
    /// Wind speed in knots (from BDS 4,4).
    pub wind_speed: Option<u16>,
    /// Wind direction in degrees (from BDS 4,4).
    pub wind_direction: Option<f64>,
    /// Static air temperature in Celsius (from BDS 4,4/4,5).
    pub temperature: Option<f64>,
    /// Last received signal level (0.0-1.0, from BEAST protocol).
    pub signal_level: Option<f32>,
    /// Timestamp of last received message.
    pub last_seen: DateTime<Utc>,
    /// Measurement time of the latest decoded report.
    pub last_observation_time: DateTime<Utc>,
    /// Timestamp quality of the latest decoded report.
    pub last_time_source: TimeSourceQuality,
    /// Identity of the latest decoded report.
    pub last_observation_id: ObservationIdentity,
    /// Measurement time of the latest accepted position report.
    pub position_observation_time: Option<DateTime<Utc>>,
    /// Timestamp quality of the latest accepted position report.
    pub position_time_source: Option<TimeSourceQuality>,
    /// Identity of the latest accepted position report.
    pub position_observation_id: Option<ObservationIdentity>,
    /// Timing and identity of the latest accepted position field.
    pub position_freshness: Option<ObservationFreshness>,
    /// Timing and identity of the latest accepted altitude field.
    pub altitude_freshness: Option<ObservationFreshness>,
    /// Timing and identity of the latest accepted velocity/track fields.
    pub velocity_freshness: Option<ObservationFreshness>,
    /// Timestamp of last accepted position update (for jump detection).
    last_position_time: Option<DateTime<Utc>>,
    /// Position history for trail rendering.
    pub position_history: Vec<PositionPoint>,
    #[cfg(feature = "tracker-jump-detection")]
    consecutive_rejections: u32,
}

impl Aircraft {
    fn new(icao: Icao, receipt_time: DateTime<Utc>) -> Self {
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
            roll_angle: None,
            track_angle_rate: None,
            selected_altitude: None,
            barometric_setting: None,
            wind_speed: None,
            wind_direction: None,
            temperature: None,
            signal_level: None,
            last_seen: receipt_time,
            last_observation_time: receipt_time,
            last_time_source: TimeSourceQuality::ReceiptTime,
            last_observation_id: ObservationIdentity {
                frame_sequence: 0,
                payload_index: 0,
            },
            position_observation_time: None,
            position_time_source: None,
            position_observation_id: None,
            position_freshness: None,
            altitude_freshness: None,
            velocity_freshness: None,
            last_position_time: None,
            position_history: Vec::new(),
            #[cfg(feature = "tracker-jump-detection")]
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
        observation_time: DateTime<Utc>,
    ) -> bool {
        // Check if position is within max distance from center
        let distance_from_center = haversine_distance(center_lat, center_lon, lat, lon);
        if distance_from_center > max_distance {
            return false;
        }

        // Redundant with rs1090 CPR validation; enable for hardware decoders that bypass it.
        #[cfg(feature = "tracker-jump-detection")]
        if let (Some(last_lat), Some(last_lon)) = (self.latitude, self.longitude) {
            let time_since_last_position = self
                .last_position_time
                .map(|t| (observation_time - t).num_seconds())
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
                timestamp: observation_time,
            });
        }

        self.latitude = Some(lat);
        self.longitude = Some(lon);
        self.last_position_time = Some(observation_time);
        #[cfg(feature = "tracker-jump-detection")]
        {
            self.consecutive_rejections = 0;
        }

        true
    }

    fn cleanup_old_history_at(&mut self, now: DateTime<Utc>, max_age_seconds: i64) {
        self.position_history
            .retain(|point| (now - point.timestamp).num_seconds() < max_age_seconds);
    }
}

/// Events emitted by the tracker when aircraft state changes.
#[derive(Debug, Clone)]
pub enum TrackerEvent {
    /// A new aircraft was added to tracking.
    AircraftAdded(Icao),
    /// An aircraft's position was updated.
    PositionUpdated(Icao),
    /// An aircraft was removed due to timeout.
    AircraftRemoved(Icao),
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
            position_history_secs: i64::MAX,
            event_channel_capacity: 256,
        }
    }
}

/// Aircraft tracker that maintains state and emits events.
pub struct AircraftTracker {
    aircraft: HashMap<Icao, Aircraft>,
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
        let now = Utc::now();
        self.process_decoded_message(DecodedMessage::new(
            msg,
            MessageTiming::receipt_time(
                now,
                ObservationIdentity {
                    frame_sequence: 0,
                    payload_index: 0,
                },
            ),
        ));
    }

    /// Process a decoded message without replacing its observation timing.
    pub fn process_decoded_message(&mut self, decoded: DecodedMessage) {
        let timing = decoded.timing;
        let msg = decoded.message;
        let freshness = ObservationFreshness {
            observation_time: timing.observation_time,
            receipt_time: timing.receipt_time,
            time_source: timing.time_source,
            identity: timing.identity,
        };
        let icao = msg.icao();
        let is_new = !self.aircraft.contains_key(&icao);

        let aircraft = self
            .aircraft
            .entry(icao)
            .or_insert_with(|| Aircraft::new(icao, timing.receipt_time));

        if timing.receipt_time >= aircraft.last_seen {
            aircraft.last_seen = timing.receipt_time;
            aircraft.last_observation_time = timing.observation_time;
            aircraft.last_time_source = timing.time_source;
            aircraft.last_observation_id = timing.identity;
        }

        if let Some(sl) = msg.signal_level {
            aircraft.signal_level = Some(sl);
        }

        if is_new {
            let _ = self.event_tx.send(TrackerEvent::AircraftAdded(icao));
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
                let altitude_is_fresh = accepts_freshness(aircraft.altitude_freshness, freshness);
                let velocity_is_fresh = accepts_freshness(aircraft.velocity_freshness, freshness);
                let updated = accepts_freshness(aircraft.position_freshness, freshness)
                    && aircraft.update_position(
                        latitude,
                        longitude,
                        self.center_lat,
                        self.center_lon,
                        self.max_distance_miles,
                        timing.observation_time,
                    );
                if updated {
                    if let Some(alt) = altitude.filter(|_| altitude_is_fresh) {
                        aircraft.altitude = Some(alt);
                        aircraft.altitude_freshness = Some(freshness);
                    }
                    if velocity_is_fresh {
                        if let Some(gs) = ground_speed {
                            aircraft.velocity = Some(gs);
                        }
                        if let Some(trk) = track {
                            aircraft.track = Some(trk);
                        }
                        if ground_speed.is_some() || track.is_some() {
                            aircraft.velocity_freshness = Some(freshness);
                        }
                    }
                    if let Some(on_ground) = is_on_ground {
                        aircraft.is_on_ground = Some(on_ground);
                    }
                    aircraft.position_observation_time = Some(timing.observation_time);
                    aircraft.position_time_source = Some(timing.time_source);
                    aircraft.position_observation_id = Some(timing.identity);
                    aircraft.position_freshness = Some(freshness);
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
                roll_angle,
                track_angle_rate,
            } => {
                if accepts_freshness(aircraft.velocity_freshness, freshness) {
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
                    if let Some(ra) = roll_angle {
                        aircraft.roll_angle = Some(ra);
                    }
                    if let Some(tar) = track_angle_rate {
                        aircraft.track_angle_rate = Some(tar);
                    }
                    aircraft.velocity_freshness = Some(freshness);
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
                if let Some(alt) =
                    altitude.filter(|_| accepts_freshness(aircraft.altitude_freshness, freshness))
                {
                    aircraft.altitude = Some(alt);
                    aircraft.altitude_freshness = Some(freshness);
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
            MessagePayload::SelectedAltitude {
                mcp_altitude,
                barometric_setting,
                ..
            } => {
                if let Some(alt) = mcp_altitude {
                    aircraft.selected_altitude = Some(alt);
                }
                if let Some(baro) = barometric_setting {
                    aircraft.barometric_setting = Some(baro);
                }
            }
            MessagePayload::Meteorological {
                wind_speed,
                wind_direction,
                temperature,
                ..
            } => {
                aircraft.wind_speed = wind_speed;
                aircraft.wind_direction = wind_direction;
                aircraft.temperature = Some(temperature);
            }
            MessagePayload::MeteorologicalHazard { temperature, .. } => {
                if let Some(t) = temperature {
                    aircraft.temperature = Some(t);
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
    pub fn get_by_icao(&self, icao: Icao) -> Option<&Aircraft> {
        self.aircraft.get(&icao)
    }

    /// Get the number of tracked aircraft.
    #[must_use]
    pub fn len(&self) -> usize {
        self.aircraft.len()
    }

    /// Count aircraft with a currently known position.
    #[must_use]
    pub fn positioned_len(&self) -> usize {
        self.aircraft
            .values()
            .filter(|aircraft| aircraft.latitude.is_some() && aircraft.longitude.is_some())
            .count()
    }

    /// Count retained position samples across all aircraft.
    #[must_use]
    pub fn position_history_len(&self) -> usize {
        self.aircraft
            .values()
            .map(|aircraft| aircraft.position_history.len())
            .sum()
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
        self.cleanup_stale_at(Utc::now());
    }

    /// Remove stale aircraft using an injected receipt-time clock.
    pub fn cleanup_stale_at(&mut self, now: DateTime<Utc>) {
        // Clean up old position history
        for aircraft in self.aircraft.values_mut() {
            aircraft.cleanup_old_history_at(now, self.position_history_secs);
        }

        // Remove aircraft that haven't been seen recently
        let removed: Vec<_> = self
            .aircraft
            .iter()
            .filter(|(_, a)| (now - a.last_seen).num_seconds() >= self.aircraft_timeout_secs)
            .map(|(icao, _)| *icao)
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

    fn position_message(timing: MessageTiming, latitude: f64) -> DecodedMessage {
        DecodedMessage::new(
            AircraftMessage {
                icao: Icao(0xA1B2C3),
                signal_level: None,
                payload: MessagePayload::Position {
                    latitude,
                    longitude: -118.5,
                    altitude: Some(35000),
                    ground_speed: None,
                    track: None,
                    is_on_ground: None,
                    altitude_gnss: None,
                },
            },
            timing,
        )
    }

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
            icao: Icao(0xA1B2C3),
            signal_level: None,
            payload: MessagePayload::Identification {
                callsign: "UAL123".to_string(),
                category: None,
            },
        });

        assert_eq!(tracker.len(), 1);
        let aircraft = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        assert_eq!(aircraft.callsign.as_deref(), Some("UAL123"));
    }

    #[test]
    fn test_tracker_position_update() {
        let mut tracker = AircraftTracker::new(TrackerConfig {
            center: Some((33.9425, -118.4081)),
            ..Default::default()
        });

        tracker.process_message(AircraftMessage {
            icao: Icao(0xA1B2C3),
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

        let aircraft = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        assert_eq!(aircraft.latitude, Some(34.0));
        assert_eq!(aircraft.longitude, Some(-118.5));
        assert_eq!(aircraft.altitude, Some(35000));
        assert_eq!(tracker.positioned_len(), 1);
        assert_eq!(tracker.position_history_len(), 1);
    }

    #[test]
    fn repeated_cached_position_keeps_measurement_time() {
        let observation_time = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let receipt_time = observation_time + chrono::Duration::seconds(10);
        let timing = MessageTiming {
            observation_time,
            receipt_time,
            time_source: TimeSourceQuality::ReceiverClock,
            identity: ObservationIdentity {
                frame_sequence: 7,
                payload_index: 0,
            },
        };
        let mut tracker = AircraftTracker::new(TrackerConfig {
            center: Some((33.9425, -118.4081)),
            ..Default::default()
        });

        tracker.process_decoded_message(position_message(timing, 34.0));
        tracker.process_decoded_message(position_message(timing, 34.0));

        let aircraft = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        assert_eq!(aircraft.last_seen, receipt_time);
        assert_eq!(aircraft.position_observation_time, Some(observation_time));
        assert_eq!(aircraft.position_observation_id, Some(timing.identity));
        assert_eq!(aircraft.position_history.len(), 1);
        assert_eq!(aircraft.position_history[0].timestamp, observation_time);
    }

    #[test]
    fn stationary_report_is_fresh_even_when_position_does_not_move() {
        let observation_time = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let mut tracker = AircraftTracker::new(TrackerConfig {
            center: Some((33.9425, -118.4081)),
            ..Default::default()
        });

        tracker.process_decoded_message(position_message(
            MessageTiming {
                observation_time,
                receipt_time: observation_time,
                time_source: TimeSourceQuality::ReceiverClock,
                identity: ObservationIdentity {
                    frame_sequence: 1,
                    payload_index: 0,
                },
            },
            34.0,
        ));
        tracker.process_decoded_message(position_message(
            MessageTiming {
                observation_time: observation_time + chrono::Duration::seconds(2),
                receipt_time: observation_time + chrono::Duration::seconds(2),
                time_source: TimeSourceQuality::ReceiverClock,
                identity: ObservationIdentity {
                    frame_sequence: 2,
                    payload_index: 0,
                },
            },
            34.0,
        ));

        let aircraft = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        assert_eq!(aircraft.position_history.len(), 1);
        assert_eq!(
            aircraft.position_freshness.unwrap().identity.frame_sequence,
            2
        );
        assert_eq!(
            aircraft.altitude_freshness.unwrap().identity.frame_sequence,
            2
        );
    }

    #[test]
    fn rejected_position_does_not_refresh_its_telemetry_fields() {
        let observation_time = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let timing = |frame_sequence| MessageTiming {
            observation_time: observation_time + chrono::Duration::seconds(frame_sequence as i64),
            receipt_time: observation_time + chrono::Duration::seconds(frame_sequence as i64),
            time_source: TimeSourceQuality::ReceiverClock,
            identity: ObservationIdentity {
                frame_sequence,
                payload_index: 0,
            },
        };
        let mut tracker = AircraftTracker::new(TrackerConfig {
            center: Some((33.9425, -118.4081)),
            max_distance_miles: 100.0,
            ..Default::default()
        });

        tracker.process_decoded_message(position_message(timing(1), 34.0));
        tracker.process_decoded_message(position_message(timing(2), 40.0));

        let aircraft = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        assert_eq!(aircraft.altitude, Some(35000));
        assert_eq!(
            aircraft.altitude_freshness.unwrap().identity.frame_sequence,
            1
        );
        assert_eq!(
            aircraft.position_freshness.unwrap().identity.frame_sequence,
            1
        );
    }

    #[test]
    fn late_position_does_not_replace_newer_field_freshness() {
        let observation_time = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let timing = |frame_sequence, seconds| MessageTiming {
            observation_time: observation_time + chrono::Duration::seconds(seconds),
            receipt_time: observation_time + chrono::Duration::seconds(10 + frame_sequence as i64),
            time_source: TimeSourceQuality::ReceiverClock,
            identity: ObservationIdentity {
                frame_sequence,
                payload_index: 0,
            },
        };
        let mut tracker = AircraftTracker::new(TrackerConfig {
            center: Some((33.9425, -118.4081)),
            ..Default::default()
        });

        tracker.process_decoded_message(position_message(timing(1, 10), 34.0));
        tracker.process_decoded_message(position_message(timing(2, 5), 34.1));

        let aircraft = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        assert_eq!(aircraft.latitude, Some(34.0));
        assert_eq!(
            aircraft.position_freshness.unwrap().identity.frame_sequence,
            1
        );
        assert_eq!(
            aircraft.altitude_freshness.unwrap().identity.frame_sequence,
            1
        );
        assert_eq!(
            aircraft.last_seen,
            observation_time + chrono::Duration::seconds(12)
        );
    }

    #[test]
    fn silent_feed_ages_from_receipt_time() {
        let receipt_time = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let timing = MessageTiming::receipt_time(
            receipt_time,
            ObservationIdentity {
                frame_sequence: 1,
                payload_index: 0,
            },
        );
        let mut tracker = AircraftTracker::new(TrackerConfig {
            aircraft_timeout_secs: 5,
            ..Default::default()
        });

        tracker.process_decoded_message(position_message(timing, 34.0));
        tracker.cleanup_stale_at(receipt_time + chrono::Duration::seconds(6));

        assert!(tracker.is_empty());
    }

    #[test]
    fn test_cleanup_removes_stale_aircraft() {
        let mut tracker = AircraftTracker::new(TrackerConfig {
            aircraft_timeout_secs: 1,
            ..Default::default()
        });

        tracker.process_message(AircraftMessage {
            icao: Icao(0xA1B2C3),
            signal_level: None,
            payload: MessagePayload::Identification {
                callsign: "UAL123".to_string(),
                category: None,
            },
        });
        tracker
            .aircraft
            .get_mut(&Icao(0xA1B2C3))
            .expect("aircraft was inserted")
            .last_seen = Utc::now() - chrono::Duration::seconds(2);

        tracker.cleanup_stale();

        assert!(tracker.is_empty());
    }

    #[test]
    fn test_position_rejected_too_far() {
        let mut tracker = AircraftTracker::new(TrackerConfig {
            center: Some((33.9425, -118.4081)),
            max_distance_miles: 100.0,
            ..Default::default()
        });

        tracker.process_message(AircraftMessage {
            icao: Icao(0xA1B2C3),
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

        let aircraft = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        assert!(aircraft.latitude.is_none());
    }
}
