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
//! Currently implements BaseStation/SBS-1 protocol, with future support planned
//! for BEAST, AVR, and other formats.

mod basestation;

pub use basestation::BaseStationParser;

use thiserror::Error;

/// Errors that can occur during message parsing.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid message format: {0}")]
    InvalidFormat(String),

    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("invalid value for field '{field}': {value}")]
    InvalidValue { field: &'static str, value: String },
}

/// Unified message type for all ADS-B protocols.
///
/// Represents the core aircraft data that can be extracted from any ADS-B feed,
/// regardless of the underlying protocol format.
#[derive(Debug, Clone, PartialEq)]
pub enum AircraftMessage {
    /// Aircraft identification message (callsign).
    Identification {
        /// ICAO 24-bit address (hex string, e.g., "A1B2C3").
        icao: String,
        /// Aircraft callsign (e.g., "UAL123").
        callsign: String,
    },

    /// Aircraft position message.
    Position {
        /// ICAO 24-bit address.
        icao: String,
        /// Latitude in degrees.
        latitude: f64,
        /// Longitude in degrees.
        longitude: f64,
        /// Altitude in feet (optional, may not be present in all messages).
        altitude: Option<i32>,
        /// Ground speed in knots (from MSG type 2 surface position).
        ground_speed: Option<f64>,
        /// Track angle in degrees (from MSG type 2 surface position).
        track: Option<f64>,
        /// Whether the aircraft is on the ground.
        is_on_ground: Option<bool>,
    },

    /// Aircraft velocity message.
    Velocity {
        /// ICAO 24-bit address.
        icao: String,
        /// Ground speed in knots.
        speed: f64,
        /// Track angle in degrees (0-360, north = 0).
        track: f64,
        /// Vertical rate in feet per minute (positive = climb, negative = descend).
        vertical_rate: Option<i32>,
        /// Whether the aircraft is on the ground.
        is_on_ground: Option<bool>,
    },

    /// Surveillance update (altitude, squawk, and status flags).
    Altitude {
        /// ICAO 24-bit address.
        icao: String,
        /// Altitude in feet (absent in some MSG types like MSG,8).
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
}

impl AircraftMessage {
    /// Get the ICAO address from any message variant.
    #[must_use]
    pub fn icao(&self) -> &str {
        match self {
            Self::Identification { icao, .. }
            | Self::Position { icao, .. }
            | Self::Velocity { icao, .. }
            | Self::Altitude { icao, .. } => icao,
        }
    }
}

/// Trait for protocol parsers.
///
/// Implement this trait to add support for new ADS-B protocol formats.
pub trait Protocol {
    /// The message type produced by this parser.
    type Message;
    /// The error type for parsing failures.
    type Error;

    /// Parse input bytes into a message.
    ///
    /// Returns `Ok(Some(message))` if parsing succeeded,
    /// `Ok(None)` if the input is valid but doesn't produce a message,
    /// or `Err(error)` if parsing failed.
    fn parse(&mut self, input: &[u8]) -> Result<Option<Self::Message>, Self::Error>;
}
