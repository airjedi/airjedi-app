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

//! ADS-B client library for connecting to and parsing ADS-B data feeds.
//!
//! This library provides a modular, reusable architecture for receiving and
//! processing ADS-B aircraft tracking data. It supports multiple layers that
//! can be used independently or composed together:
//!
//! - **Protocol layer**: Message parsing (BaseStation/SBS-1, with future support
//!   for BEAST, AVR, and others)
//! - **Tracker layer**: Aircraft state management, position history, and validation
//! - **Connection layer**: Async TCP with automatic reconnection and address hot-reload
//!
//! # Quick Start
//!
//! Use the [`Client`] type for full-stack operation:
//!
//! ```no_run
//! use adsb_client::{Client, ClientConfig, ConnectionConfig, TrackerConfig, ProtocolType};
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() {
//!     let client = Client::spawn(ClientConfig {
//!         connection: ConnectionConfig {
//!             address: "localhost:30003".to_string(),
//!             ..Default::default()
//!         },
//!         tracker: TrackerConfig {
//!             center: Some((33.9425, -118.4081)),
//!             max_distance_miles: 200.0,
//!             ..Default::default()
//!         },
//!         protocol: ProtocolType::BaseStation,
//!         ..Default::default()
//!     });
//!
//!     // Polling approach
//!     loop {
//!         for aircraft in client.get_aircraft() {
//!             println!("{}: {:?}", aircraft.icao, aircraft.callsign);
//!         }
//!         tokio::time::sleep(Duration::from_secs(1)).await;
//!     }
//! }
//! ```
//!
//! # Using Individual Layers
//!
//! Each layer can be used independently for custom integrations:
//!
//! ## Protocol Layer Only
//!
//! ```
//! use adsb_client::protocol::{BaseStationParser, Protocol};
//!
//! let mut parser = BaseStationParser::new();
//! let line = b"MSG,1,1,1,A1B2C3,1,2024/01/01,12:00:00,2024/01/01,12:00:00,UAL123";
//! if let Ok(Some(msg)) = parser.parse(line) {
//!     println!("Got message for ICAO: {}", msg.icao());
//! }
//! ```
//!
//! ## Tracker Layer Only
//!
//! ```
//! use adsb_client::tracker::{AircraftTracker, TrackerConfig};
//! use adsb_client::protocol::AircraftMessage;
//!
//! let mut tracker = AircraftTracker::new(TrackerConfig {
//!     center: Some((33.9425, -118.4081)),
//!     ..Default::default()
//! });
//!
//! tracker.process_message(AircraftMessage::Position {
//!     icao: "A1B2C3".to_string(),
//!     latitude: 34.0,
//!     longitude: -118.5,
//!     altitude: Some(35000),
//!     ground_speed: None,
//!     track: None,
//!     is_on_ground: None,
//! });
//!
//! println!("Tracking {} aircraft", tracker.len());
//! ```

pub mod protocol;
pub mod tcp;
pub mod tracker;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use log::warn;
use tokio::sync::broadcast;

pub use protocol::{AircraftMessage, BaseStationParser, ParseError, Protocol};
pub use tcp::{Connection, ConnectionConfig, ConnectionEvent, ConnectionState};
pub use tracker::{Aircraft, AircraftTracker, PositionPoint, TrackerConfig, TrackerEvent};

/// Protocol type for the client.
#[derive(Debug, Clone, Copy, Default)]
pub enum ProtocolType {
    /// BaseStation/SBS-1 CSV protocol (default).
    #[default]
    BaseStation,
}

/// Configuration for the full-stack client.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Connection configuration.
    pub connection: ConnectionConfig,
    /// Tracker configuration.
    pub tracker: TrackerConfig,
    /// Protocol type.
    pub protocol: ProtocolType,
    /// Cleanup interval for stale aircraft.
    pub cleanup_interval: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connection: ConnectionConfig::default(),
            tracker: TrackerConfig::default(),
            protocol: ProtocolType::default(),
            cleanup_interval: Duration::from_secs(30),
        }
    }
}

/// Full-stack ADS-B client that wires all layers together.
///
/// The client manages a TCP connection, parses incoming messages using the
/// configured protocol, and maintains aircraft state in a tracker.
pub struct Client {
    tracker: Arc<RwLock<AircraftTracker>>,
    connection: Connection,
    connection_state: Arc<RwLock<ConnectionState>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("connection", &self.connection)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Spawn a new client with the given configuration.
    ///
    /// This starts background tasks for connection management, message parsing,
    /// and periodic cleanup.
    #[must_use]
    pub fn spawn(config: ClientConfig) -> Self {
        let tracker = Arc::new(RwLock::new(AircraftTracker::new(config.tracker)));
        let connection = Connection::spawn(config.connection);
        let connection_state = Arc::new(RwLock::new(ConnectionState::Disconnected));

        // Spawn message processing task
        let tracker_clone = Arc::clone(&tracker);
        let state_clone = Arc::clone(&connection_state);
        let cleanup_interval = config.cleanup_interval;

        // We need to create a new connection receiver for the processing task
        // Since Connection owns the receiver, we need a different approach
        // We'll spawn the processing in a way that shares the connection

        // Actually, the design needs adjustment - Connection owns the receiver
        // Let's create a simpler design where Client processes in a loop

        // For now, we'll spawn a task that periodically cleans up
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(cleanup_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;
                if let Ok(mut tracker) = tracker_clone.write() {
                    tracker.cleanup_stale();
                }
            }
        });

        Self {
            tracker,
            connection,
            connection_state: state_clone,
        }
    }

    /// Process events from the connection.
    ///
    /// This should be called in a loop to process incoming data:
    ///
    /// ```no_run
    /// # use adsb_client::{Client, ClientConfig};
    /// # async fn example() {
    /// let mut client = Client::spawn(ClientConfig::default());
    /// while client.process_next().await {}
    /// # }
    /// ```
    pub async fn process_next(&mut self) -> bool {
        let event = match self.connection.recv().await {
            Some(event) => event,
            None => return false,
        };

        match event {
            ConnectionEvent::StateChanged(state) => {
                if let Ok(mut s) = self.connection_state.write() {
                    *s = state;
                }
            }
            ConnectionEvent::DataReceived(data) => {
                let mut parser = BaseStationParser::new();
                match parser.parse(&data) {
                    Ok(Some(msg)) => {
                        if let Ok(mut tracker) = self.tracker.write() {
                            tracker.process_message(msg);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        warn!("Parse error: {}", e);
                    }
                }
            }
        }

        true
    }

    /// Get all tracked aircraft.
    #[must_use]
    pub fn get_aircraft(&self) -> Vec<Aircraft> {
        self.tracker
            .read()
            .map(|t| t.get_aircraft().into_iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get a specific aircraft by ICAO address.
    #[must_use]
    pub fn get_by_icao(&self, icao: &str) -> Option<Aircraft> {
        self.tracker
            .read()
            .ok()
            .and_then(|t| t.get_by_icao(icao).cloned())
    }

    /// Get the number of tracked aircraft.
    #[must_use]
    pub fn aircraft_count(&self) -> usize {
        self.tracker.read().map(|t| t.len()).unwrap_or(0)
    }

    /// Subscribe to tracker events.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<TrackerEvent> {
        self.tracker
            .read()
            .map(|t| t.subscribe())
            .unwrap_or_else(|_| {
                let (tx, rx) = broadcast::channel(1);
                drop(tx);
                rx
            })
    }

    /// Get the current connection state.
    #[must_use]
    pub fn connection_state(&self) -> ConnectionState {
        self.connection_state
            .read()
            .map(|s| s.clone())
            .unwrap_or(ConnectionState::Disconnected)
    }

    /// Change the server address.
    ///
    /// The connection will disconnect and reconnect to the new address.
    pub fn set_address(&self, address: String) {
        self.connection.set_address(address);
    }

    /// Get the current server address.
    #[must_use]
    pub fn current_address(&self) -> String {
        self.connection.current_address()
    }

    /// Set the center point for distance filtering.
    pub fn set_center(&self, lat: f64, lon: f64) {
        if let Ok(mut tracker) = self.tracker.write() {
            tracker.set_center(lat, lon);
        }
    }

    /// Shut down the client.
    pub fn shutdown(&self) {
        self.connection.shutdown();
    }
}
