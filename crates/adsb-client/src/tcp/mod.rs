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

//! Async TCP connection layer with automatic reconnection.
//!
//! Provides a connection handle that manages TCP connections to ADS-B feeds
//! with automatic reconnection, address hot-reload, and graceful shutdown.
//! Supports both line-based (SBS-1) and raw binary (BEAST) reading modes.

use std::time::Duration;

use log::{error, info, warn};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

/// Framing mode for TCP data reading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameMode {
    /// Read newline-delimited text (for SBS-1/BaseStation, AVR).
    #[default]
    Line,
    /// Read raw binary chunks (for BEAST binary protocol).
    Raw,
}

/// Configuration for TCP connections.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// Server address in "host:port" format.
    pub address: String,
    /// Delay before reconnecting after disconnect.
    pub reconnect_delay: Duration,
    /// Optional read timeout.
    pub read_timeout: Option<Duration>,
    /// Channel buffer size for received data.
    pub buffer_size: usize,
    /// Data framing mode.
    pub frame_mode: FrameMode,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            address: "localhost:30003".to_string(),
            reconnect_delay: Duration::from_secs(5),
            read_timeout: None,
            buffer_size: 1024,
            frame_mode: FrameMode::Line,
        }
    }
}

/// Connection state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// Attempting to connect.
    Connecting,
    /// Successfully connected.
    Connected,
    /// Disconnected (will attempt reconnect).
    Disconnected,
    /// Connection error occurred.
    Error(String),
}

/// Events emitted by the connection.
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    /// Connection state changed.
    StateChanged(ConnectionState),
    /// Data received (one line in Line mode, raw chunk in Raw mode).
    DataReceived(Vec<u8>),
}

/// Handle to a managed TCP connection.
///
/// The connection runs in a background task and automatically reconnects
/// on disconnect. Use `recv()` to receive events and `set_address()` to
/// change the server address at runtime.
pub struct Connection {
    event_rx: mpsc::Receiver<ConnectionEvent>,
    address_tx: watch::Sender<String>,
    cancel_token: CancellationToken,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("cancel_token", &self.cancel_token)
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Spawn a new connection task with the given configuration.
    #[must_use]
    pub fn spawn(config: ConnectionConfig) -> Self {
        let (event_tx, event_rx) = mpsc::channel(config.buffer_size);
        let (address_tx, address_rx) = watch::channel(config.address.clone());
        let cancel_token = CancellationToken::new();

        let task_cancel = cancel_token.clone();
        let reconnect_delay = config.reconnect_delay;
        let frame_mode = config.frame_mode;

        tokio::spawn(async move {
            connection_loop(event_tx, address_rx, task_cancel, reconnect_delay, frame_mode).await;
        });

        Self {
            event_rx,
            address_tx,
            cancel_token,
        }
    }

    /// Receive the next event from the connection.
    pub async fn recv(&mut self) -> Option<ConnectionEvent> {
        self.event_rx.recv().await
    }

    /// Change the server address.
    pub fn set_address(&self, address: String) {
        let _ = self.address_tx.send(address);
    }

    /// Get the current server address.
    #[must_use]
    pub fn current_address(&self) -> String {
        self.address_tx.borrow().clone()
    }

    /// Shut down the connection.
    pub fn shutdown(&self) {
        self.cancel_token.cancel();
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

async fn connection_loop(
    event_tx: mpsc::Sender<ConnectionEvent>,
    mut address_rx: watch::Receiver<String>,
    cancel_token: CancellationToken,
    reconnect_delay: Duration,
    frame_mode: FrameMode,
) {
    loop {
        if cancel_token.is_cancelled() {
            info!("Connection cancelled");
            return;
        }

        let current_address = address_rx.borrow_and_update().clone();

        if event_tx
            .send(ConnectionEvent::StateChanged(ConnectionState::Connecting))
            .await
            .is_err()
        {
            return;
        }

        info!("Connecting to {}...", current_address);

        let result = match frame_mode {
            FrameMode::Line => {
                connect_and_process_lines(
                    &current_address,
                    &event_tx,
                    &mut address_rx,
                    &cancel_token,
                ).await
            }
            FrameMode::Raw => {
                connect_and_process_raw(
                    &current_address,
                    &event_tx,
                    &mut address_rx,
                    &cancel_token,
                ).await
            }
        };

        match result {
            Ok(reason) => match reason {
                ReconnectReason::AddressChanged => {
                    info!("Server address changed, reconnecting immediately...");
                    continue;
                }
                ReconnectReason::ConnectionClosed => {
                    info!("Connection closed normally");
                    let _ = event_tx
                        .send(ConnectionEvent::StateChanged(ConnectionState::Disconnected))
                        .await;
                }
                ReconnectReason::Cancelled => {
                    info!("Connection cancelled");
                    return;
                }
            },
            Err(e) => {
                error!("Connection error: {}", e);
                let _ = event_tx
                    .send(ConnectionEvent::StateChanged(ConnectionState::Error(
                        e.to_string(),
                    )))
                    .await;
            }
        }

        warn!("Reconnecting in {} seconds...", reconnect_delay.as_secs());

        tokio::select! {
            () = sleep(reconnect_delay) => {}
            () = cancel_token.cancelled() => {
                info!("Connection cancelled during reconnect delay");
                return;
            }
        }
    }
}

enum ReconnectReason {
    AddressChanged,
    ConnectionClosed,
    Cancelled,
}

/// Line-based reading for text protocols (SBS-1, AVR).
async fn connect_and_process_lines(
    address: &str,
    event_tx: &mpsc::Sender<ConnectionEvent>,
    address_rx: &mut watch::Receiver<String>,
    cancel_token: &CancellationToken,
) -> Result<ReconnectReason, Box<dyn std::error::Error + Send + Sync>> {
    let stream = TcpStream::connect(address).await?;
    info!("Connected to {}", address);

    if event_tx
        .send(ConnectionEvent::StateChanged(ConnectionState::Connected))
        .await
        .is_err()
    {
        return Ok(ReconnectReason::Cancelled);
    }

    let reader = BufReader::new(stream);
    let mut lines = reader.lines();

    loop {
        tokio::select! {
            line_result = lines.next_line() => {
                match line_result {
                    Ok(Some(line)) => {
                        let mut bytes = line.into_bytes();
                        bytes.push(b'\n');
                        if event_tx
                            .send(ConnectionEvent::DataReceived(bytes))
                            .await
                            .is_err()
                        {
                            return Ok(ReconnectReason::Cancelled);
                        }
                    }
                    Ok(None) => {
                        info!("Connection closed by server");
                        return Ok(ReconnectReason::ConnectionClosed);
                    }
                    Err(e) => {
                        return Err(Box::new(e));
                    }
                }
            }

            _ = address_rx.changed() => {
                let new_address = address_rx.borrow_and_update().clone();
                if new_address != address {
                    info!("Server address changed from {} to {}", address, new_address);
                    return Ok(ReconnectReason::AddressChanged);
                }
            }

            () = cancel_token.cancelled() => {
                return Ok(ReconnectReason::Cancelled);
            }
        }
    }
}

/// Raw binary reading for binary protocols (BEAST).
async fn connect_and_process_raw(
    address: &str,
    event_tx: &mpsc::Sender<ConnectionEvent>,
    address_rx: &mut watch::Receiver<String>,
    cancel_token: &CancellationToken,
) -> Result<ReconnectReason, Box<dyn std::error::Error + Send + Sync>> {
    let mut stream = TcpStream::connect(address).await?;
    info!("Connected to {} (raw binary mode)", address);

    if event_tx
        .send(ConnectionEvent::StateChanged(ConnectionState::Connected))
        .await
        .is_err()
    {
        return Ok(ReconnectReason::Cancelled);
    }

    let mut buf = vec![0u8; 4096];

    loop {
        tokio::select! {
            read_result = stream.read(&mut buf) => {
                match read_result {
                    Ok(0) => {
                        info!("Connection closed by server");
                        return Ok(ReconnectReason::ConnectionClosed);
                    }
                    Ok(n) => {
                        if event_tx
                            .send(ConnectionEvent::DataReceived(buf[..n].to_vec()))
                            .await
                            .is_err()
                        {
                            return Ok(ReconnectReason::Cancelled);
                        }
                    }
                    Err(e) => {
                        return Err(Box::new(e));
                    }
                }
            }

            _ = address_rx.changed() => {
                let new_address = address_rx.borrow_and_update().clone();
                if new_address != address {
                    info!("Server address changed from {} to {}", address, new_address);
                    return Ok(ReconnectReason::AddressChanged);
                }
            }

            () = cancel_token.cancelled() => {
                return Ok(ReconnectReason::Cancelled);
            }
        }
    }
}
