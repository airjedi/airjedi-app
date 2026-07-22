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
//! - **Protocol layer**: Message parsing (BaseStation/SBS-1 and BEAST binary)
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
//!     let mut client = Client::spawn(ClientConfig {
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
//! # BEAST Binary Protocol
//!
//! For connecting to a BEAST feed (port 30005):
//!
//! ```no_run
//! use adsb_client::{Client, ClientConfig, ConnectionConfig, ProtocolType};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let mut client = Client::spawn(ClientConfig {
//!     connection: ConnectionConfig {
//!         address: "localhost:30005".to_string(),
//!         ..Default::default()
//!     },
//!     protocol: ProtocolType::Beast,
//!     ..Default::default()
//! });
//! # }
//! ```

pub mod decoder;
pub mod framing;
pub mod protocol;
pub mod tcp;
pub mod tracker;
pub mod transport;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use log::warn;
use tokio::sync::broadcast;

pub use decoder::{BaseStationDecoder, Decoder, NativeDecoder};
pub use framing::{BeastFramer, Frame, FrameType, Framer, LineFramer};
pub use protocol::{AircraftMessage, BaseStationParser, BeastParser, Icao, MessagePayload, ParseError, Protocol};
pub use tcp::{Connection, ConnectionConfig, ConnectionEvent, ConnectionState, FrameMode};
pub use tracker::{Aircraft, AircraftTracker, PositionPoint, TrackerConfig, TrackerEvent};
pub use transport::{TcpTransport, Transport, TransportEvent};

/// Protocol type for the client.
#[derive(Debug, Clone, Copy, Default)]
pub enum ProtocolType {
    /// BaseStation/SBS-1 CSV protocol (port 30003, default).
    #[default]
    BaseStation,
    /// BEAST binary protocol (port 30005).
    Beast,
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

/// Full-stack ADS-B client using layered Transport + Framer + Decoder architecture.
///
/// The client manages an async transport, extracts protocol frames via a framer,
/// decodes them through a pluggable decoder, and maintains aircraft state in a tracker.
pub struct Client {
    tracker: Arc<RwLock<AircraftTracker>>,
    transport: Box<dyn Transport>,
    framer: Box<dyn Framer>,
    decoder: Box<dyn Decoder>,
    connection_state: Arc<RwLock<ConnectionState>>,
    messages_processed: Arc<AtomicU64>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
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
        let tracker = Arc::new(RwLock::new(AircraftTracker::new(config.tracker.clone())));

        let mut conn_config = config.connection;
        let (transport, framer, decoder): (Box<dyn Transport>, Box<dyn Framer>, Box<dyn Decoder>) =
            match config.protocol {
                ProtocolType::BaseStation => {
                    conn_config.frame_mode = FrameMode::Line;
                    (
                        Box::new(TcpTransport::new(conn_config)),
                        Box::new(LineFramer::new()),
                        Box::new(BaseStationDecoder::new()),
                    )
                }
                ProtocolType::Beast => {
                    conn_config.frame_mode = FrameMode::Raw;
                    let mut dec = NativeDecoder::new();
                    if let Some((lat, lon)) = config.tracker.center {
                        dec.set_reference_position(lat, lon);
                    }
                    (
                        Box::new(TcpTransport::new(conn_config)),
                        Box::new(BeastFramer::new()),
                        Box::new(dec),
                    )
                }
            };

        let connection_state = Arc::new(RwLock::new(ConnectionState::Disconnected));

        let tracker_clone = Arc::clone(&tracker);
        let cleanup_interval = config.cleanup_interval;

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
            transport,
            framer,
            decoder,
            connection_state,
            messages_processed: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Total number of messages processed by this client across all protocols.
    #[must_use]
    pub fn messages_processed(&self) -> u64 {
        self.messages_processed.load(Ordering::Relaxed)
    }

    /// Process events from the connection.
    ///
    /// This should be called in a loop to process incoming data.
    pub async fn process_next(&mut self) -> bool {
        let event = match self.transport.recv().await {
            Some(event) => event,
            None => return false,
        };

        match event {
            TransportEvent::Connected => {
                self.framer.reset();
                self.decoder.reset();
                if let Ok(mut s) = self.connection_state.write() {
                    *s = ConnectionState::Connected;
                }
            }
            TransportEvent::Disconnected => {
                if let Ok(mut s) = self.connection_state.write() {
                    *s = ConnectionState::Disconnected;
                }
            }
            TransportEvent::Data(data) => {
                self.process_data(&data);
            }
            TransportEvent::Error(e) => {
                if let Ok(mut s) = self.connection_state.write() {
                    *s = ConnectionState::Error(e);
                }
            }
        }

        true
    }

    fn process_data(&mut self, data: &[u8]) {
        self.framer.feed(data);
        while let Some(frame) = self.framer.next_frame() {
            let messages = self.decoder.decode(&frame);
            for msg in messages {
                self.messages_processed.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut tracker) = self.tracker.write() {
                    tracker.process_message(msg);
                }
            }
        }
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
    pub fn get_by_icao(&self, icao: Icao) -> Option<Aircraft> {
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
    pub fn set_address(&self, address: String) {
        self.transport.set_address(address);
    }

    /// Get the current server address.
    #[must_use]
    pub fn current_address(&self) -> String {
        self.transport.current_address()
    }

    /// Set the center point for distance filtering.
    pub fn set_center(&self, lat: f64, lon: f64) {
        if let Ok(mut tracker) = self.tracker.write() {
            tracker.set_center(lat, lon);
        }
    }

    /// Shut down the client.
    pub fn shutdown(&self) {
        self.transport.shutdown();
    }
}
