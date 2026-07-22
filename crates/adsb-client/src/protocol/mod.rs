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

//! Protocol layer for ADS-B message parsing.
//!
//! This module provides a trait-based abstraction for extensible protocol support.
//! Implements BaseStation/SBS-1 and BEAST binary protocols.

mod basestation;
pub mod beast;

pub use basestation::BaseStationParser;
#[cfg(feature = "decoder-native")]
pub use beast::BeastParser;

use std::fmt;
use thiserror::Error;

/// ICAO 24-bit aircraft address.
///
/// Stored as a `u32` (only lower 24 bits used) to avoid heap allocation.
/// Format as hex with `Display`: `format!("{icao}")` produces "A1B2C3".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Icao(pub u32);

impl Icao {
    /// Extract ICAO from bytes 1-3 of a Mode-S message (after the DF byte).
    #[must_use]
    pub fn from_message(data: &[u8]) -> Self {
        Self(u32::from(data[1]) << 16 | u32::from(data[2]) << 8 | u32::from(data[3]))
    }

    /// Extract ICAO from the CRC-24 parity remainder.
    #[must_use]
    pub fn from_parity(crc: u32) -> Self {
        Self(crc & 0x00FF_FFFF)
    }

    /// Parse a hex string (e.g., "A1B2C3") into an ICAO address.
    #[must_use]
    pub fn from_hex(s: &str) -> Option<Self> {
        u32::from_str_radix(s, 16).ok().map(Self)
    }
}

impl fmt::Display for Icao {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:06X}", self.0)
    }
}

/// Errors that can occur during message parsing.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid message format: {0}")]
    InvalidFormat(String),

    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("invalid value for field '{field}': {value}")]
    InvalidValue { field: &'static str, value: String },

    #[error("CRC check failed")]
    CrcFailed,

    #[error("unknown downlink format: {0}")]
    UnknownDownlinkFormat(u8),

    #[error("incomplete frame: need {expected} bytes, got {got}")]
    IncompleteFrame { expected: usize, got: usize },
}

/// Unified message type for all ADS-B protocols.
///
/// Wraps a `MessagePayload` variant with common per-message metadata.
/// The `icao` field is shared across all payload types, and `signal_level`
/// is populated by protocols that provide RSSI (e.g., BEAST).
#[derive(Debug, Clone, PartialEq)]
pub struct AircraftMessage {
    /// ICAO 24-bit aircraft address.
    pub icao: Icao,
    /// Signal level / RSSI (0.0 - 1.0), if provided by the protocol.
    pub signal_level: Option<f32>,
    /// The message payload.
    pub payload: MessagePayload,
}

impl AircraftMessage {
    /// Get the ICAO address.
    #[must_use]
    pub fn icao(&self) -> Icao {
        self.icao
    }

    /// Get the signal level, if available.
    #[must_use]
    pub fn signal_level(&self) -> Option<f32> {
        self.signal_level
    }
}

/// Payload variants for aircraft messages.
#[derive(Debug, Clone, PartialEq)]
pub enum MessagePayload {
    /// Aircraft identification (callsign).
    Identification {
        /// Aircraft callsign (e.g., "UAL123").
        callsign: String,
        /// Aircraft emitter category (from ADS-B TC 1-4). None for SBS-1.
        category: Option<u8>,
    },

    /// Aircraft position.
    Position {
        /// Latitude in degrees.
        latitude: f64,
        /// Longitude in degrees.
        longitude: f64,
        /// Barometric altitude in feet.
        altitude: Option<i32>,
        /// Ground speed in knots.
        ground_speed: Option<f64>,
        /// Track angle in degrees.
        track: Option<f64>,
        /// Whether the aircraft is on the ground.
        is_on_ground: Option<bool>,
        /// GNSS altitude in feet (from ADS-B TC 20-22). None for barometric.
        altitude_gnss: Option<i32>,
    },

    /// Aircraft velocity.
    Velocity {
        /// Ground speed in knots.
        speed: f64,
        /// Track angle in degrees (0-360, north = 0).
        track: f64,
        /// Vertical rate in feet per minute (positive = climb).
        vertical_rate: Option<i32>,
        /// Whether the aircraft is on the ground.
        is_on_ground: Option<bool>,
        /// Magnetic heading in degrees (from BEAST TC19 subtype 3/4). None for SBS-1.
        heading: Option<f64>,
        /// Indicated or true airspeed in knots (from BEAST TC19 subtype 3/4). None for SBS-1.
        airspeed: Option<f64>,
        /// Roll angle in degrees (from BDS 5,0). Positive = right wing down.
        roll_angle: Option<f64>,
        /// Track angle rate in degrees/second (from BDS 5,0). Positive = turning right.
        track_angle_rate: Option<f64>,
    },

    /// Surveillance update (altitude, squawk, and status flags).
    Altitude {
        /// Altitude in feet.
        altitude: Option<i32>,
        /// Squawk code (transponder code).
        squawk: Option<String>,
        /// Alert flag (squawk change).
        alert: Option<bool>,
        /// Emergency flag.
        emergency: Option<bool>,
        /// SPI (Special Position Identification) flag.
        spi: Option<bool>,
        /// Whether the aircraft is on the ground.
        is_on_ground: Option<bool>,
    },

    /// Selected vertical intention (BDS 4,0).
    SelectedAltitude {
        /// MCP/FCU selected altitude in feet.
        mcp_altitude: Option<i32>,
        /// FMS selected altitude in feet.
        fms_altitude: Option<i32>,
        /// Barometric pressure setting in hPa (QNH).
        barometric_setting: Option<f64>,
    },

    /// Meteorological routine air report (BDS 4,4).
    Meteorological {
        /// Wind speed in knots.
        wind_speed: Option<u16>,
        /// Wind direction in degrees.
        wind_direction: Option<f64>,
        /// Static air temperature in Celsius.
        temperature: f64,
        /// Static pressure in hPa.
        pressure: Option<u16>,
    },

    /// Meteorological hazard report (BDS 4,5).
    MeteorologicalHazard {
        /// Turbulence severity (0-3).
        turbulence: Option<u8>,
        /// Wind shear severity (0-3).
        wind_shear: Option<u8>,
        /// Icing severity (0-3).
        icing: Option<u8>,
        /// Wake vortex severity (0-3).
        wake_vortex: Option<u8>,
        /// Static air temperature in Celsius.
        temperature: Option<f64>,
        /// Static pressure in hPa.
        pressure: Option<u16>,
    },
}

/// Discriminant index for `MessagePayload` variants, used for counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum PayloadKind {
    Identification = 0,
    Position = 1,
    Velocity = 2,
    Altitude = 3,
    SelectedAltitude = 4,
    Meteorological = 5,
    MeteorologicalHazard = 6,
}

impl PayloadKind {
    pub const COUNT: usize = 7;

    pub const ALL: [Self; 7] = [
        Self::Identification,
        Self::Position,
        Self::Velocity,
        Self::Altitude,
        Self::SelectedAltitude,
        Self::Meteorological,
        Self::MeteorologicalHazard,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Identification => "Identification",
            Self::Position => "Position",
            Self::Velocity => "Velocity",
            Self::Altitude => "Altitude",
            Self::SelectedAltitude => "Selected Alt",
            Self::Meteorological => "Meteorological",
            Self::MeteorologicalHazard => "Met Hazard",
        }
    }
}

impl MessagePayload {
    #[must_use]
    pub fn kind(&self) -> PayloadKind {
        match self {
            Self::Identification { .. } => PayloadKind::Identification,
            Self::Position { .. } => PayloadKind::Position,
            Self::Velocity { .. } => PayloadKind::Velocity,
            Self::Altitude { .. } => PayloadKind::Altitude,
            Self::SelectedAltitude { .. } => PayloadKind::SelectedAltitude,
            Self::Meteorological { .. } => PayloadKind::Meteorological,
            Self::MeteorologicalHazard { .. } => PayloadKind::MeteorologicalHazard,
        }
    }
}

/// Trait for protocol parsers.
///
/// Implement this trait to add support for new ADS-B protocol formats.
/// Parsers may maintain internal state (buffers, CPR decode state) via `&mut self`.
pub trait Protocol {
    /// The message type produced by this parser.
    type Message;
    /// The error type for parsing failures.
    type Error;

    /// Parse input bytes into a message.
    ///
    /// Returns `Ok(Some(message))` if parsing succeeded,
    /// `Ok(None)` if the input is valid but doesn't produce a message
    /// (or no complete frame is available yet for binary protocols),
    /// or `Err(error)` if parsing failed.
    ///
    /// For binary protocols (BEAST), call with empty input `&[]` to drain
    /// remaining buffered frames after feeding data.
    fn parse(&mut self, input: &[u8]) -> Result<Option<Self::Message>, Self::Error>;

    /// Reset parser state after a reconnection.
    /// Clears accumulated buffers and decode state to prevent corruption
    /// from stale data when a new connection is established.
    fn reset(&mut self) {}
}
