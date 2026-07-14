use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use adsb_client::{
    Client as AdsbClient, ClientConfig, ConnectionConfig, ConnectionState, ProtocolType,
    TrackerConfig,
};

use crate::config::{self, FeedProtocol, FeedSourceConfig};
use crate::debug_panel::DebugPanelState;
use crate::{constants, MapState};

/// Shared state for aircraft data from a single ADS-B client.
/// Updated by a background tokio thread and read by Bevy systems.
#[derive(Clone)]
pub struct AdsbAircraftData {
    pub aircraft: Arc<Mutex<Vec<adsb_client::Aircraft>>>,
    pub connection_state: Arc<Mutex<ConnectionState>>,
    pub message_count: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
    pub endpoint_url: String,
}

impl AdsbAircraftData {
    pub fn new(endpoint_url: &str) -> Self {
        Self {
            aircraft: Arc::new(Mutex::new(Vec::new())),
            connection_state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
            message_count: Arc::new(AtomicU64::new(0)),
            shutdown: Arc::new(AtomicBool::new(false)),
            endpoint_url: endpoint_url.to_string(),
        }
    }

    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    pub fn try_get_aircraft(&self) -> Option<Vec<adsb_client::Aircraft>> {
        self.aircraft.try_lock().ok().map(|a| a.clone())
    }

    pub fn try_aircraft_count(&self) -> Option<usize> {
        self.aircraft.try_lock().ok().map(|a| a.len())
    }

    pub fn get_connection_state(&self) -> ConnectionState {
        self.connection_state
            .try_lock()
            .map(|s| s.clone())
            .unwrap_or(ConnectionState::Disconnected)
    }
}

/// A single live feed connection.
pub struct FeedConnection {
    pub config: FeedSourceConfig,
    pub data: AdsbAircraftData,
}

/// Per-feed statistics for the status popup.
pub struct FeedStats {
    pub name: String,
    pub state: ConnectionState,
    pub aircraft_count: usize,
    pub message_count: u64,
}

/// Resource managing multiple simultaneous ADS-B feed connections.
#[derive(Resource, Default)]
pub struct FeedConnectionManager {
    pub connections: HashMap<String, FeedConnection>,
    prev_feed_snapshot: Vec<FeedSourceConfig>,
}

impl FeedConnectionManager {
    pub fn unique_aircraft_count(&self) -> usize {
        let mut seen = HashSet::new();
        for conn in self.connections.values() {
            if let Some(aircraft) = conn.data.try_get_aircraft() {
                for ac in &aircraft {
                    seen.insert(ac.icao.clone());
                }
            }
        }
        seen.len()
    }

    pub fn total_message_count(&self) -> u64 {
        self.connections
            .values()
            .map(|c| c.data.message_count.load(Ordering::Relaxed))
            .sum()
    }

    pub fn connected_count(&self) -> usize {
        self.connections
            .values()
            .filter(|c| c.data.get_connection_state() == ConnectionState::Connected)
            .count()
    }

    pub fn connecting_count(&self) -> usize {
        self.connections
            .values()
            .filter(|c| c.data.get_connection_state() == ConnectionState::Connecting)
            .count()
    }

    pub fn per_feed_stats(&self) -> Vec<FeedStats> {
        self.connections
            .values()
            .map(|conn| FeedStats {
                name: conn.config.name.clone(),
                state: conn.data.get_connection_state(),
                aircraft_count: conn.data.try_aircraft_count().unwrap_or(0),
                message_count: conn.data.message_count.load(Ordering::Relaxed),
            })
            .collect()
    }

    pub fn all_aircraft(&self) -> Vec<(String, adsb_client::Aircraft)> {
        let mut result = Vec::new();
        for conn in self.connections.values() {
            if let Some(aircraft) = conn.data.try_get_aircraft() {
                for ac in aircraft {
                    result.push((conn.config.name.clone(), ac));
                }
            }
        }
        result
    }
}

/// Component to mark the connection status UI text.
#[derive(Component)]
pub struct ConnectionStatusText;

/// Startup system: spawn connections for all enabled feeds.
pub fn setup_feed_connections(
    mut commands: Commands,
    map_state: Res<MapState>,
    app_config: Res<config::AppConfig>,
) {
    let mut manager = FeedConnectionManager::default();

    for feed in &app_config.feeds {
        if feed.enabled {
            let data = spawn_feed_client(feed, map_state.latitude, map_state.longitude);
            manager.connections.insert(
                feed.id.clone(),
                FeedConnection {
                    config: feed.clone(),
                    data,
                },
            );
        }
    }

    manager.prev_feed_snapshot = app_config.feeds.clone();
    commands.insert_resource(manager);
    info!(
        "Feed connections started ({} feeds)",
        app_config.feeds.iter().filter(|f| f.enabled).count()
    );
}

fn spawn_feed_client(
    feed: &FeedSourceConfig,
    center_lat: f64,
    center_lon: f64,
) -> AdsbAircraftData {
    let adsb_data = AdsbAircraftData::new(&feed.endpoint);
    let aircraft_data = Arc::clone(&adsb_data.aircraft);
    let connection_state = Arc::clone(&adsb_data.connection_state);
    let msg_count_shared = Arc::clone(&adsb_data.message_count);
    let shutdown = Arc::clone(&adsb_data.shutdown);
    let endpoint = feed.endpoint.clone();
    let feed_name = feed.name.clone();

    let protocol = match feed.protocol {
        FeedProtocol::Sbs1 => ProtocolType::BaseStation,
        FeedProtocol::Beast => ProtocolType::Beast,
    };

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for ADS-B client");

        rt.block_on(async move {
            info!(
                "[{}] Starting ADS-B client, connecting to {} ({:?})",
                feed_name, endpoint, protocol
            );

            let mut client = AdsbClient::spawn(ClientConfig {
                connection: ConnectionConfig {
                    address: endpoint.clone(),
                    ..Default::default()
                },
                tracker: TrackerConfig {
                    center: Some((center_lat, center_lon)),
                    max_distance_miles: constants::ADSB_MAX_DISTANCE_MILES,
                    aircraft_timeout_secs: constants::ADSB_AIRCRAFT_TIMEOUT_SECS,
                    ..Default::default()
                },
                protocol,
                ..Default::default()
            });

            loop {
                if shutdown.load(Ordering::Acquire) {
                    info!("[{}] ADS-B client shutting down", feed_name);
                    return;
                }

                if !client.process_next().await {
                    if shutdown.load(Ordering::Acquire) {
                        return;
                    }
                    warn!("[{}] Connection closed, restarting...", feed_name);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }

                msg_count_shared.store(client.messages_processed(), Ordering::Relaxed);

                if let Ok(mut state) = connection_state.lock() {
                    *state = client.connection_state();
                }

                if let Ok(mut data) = aircraft_data.lock() {
                    *data = client.get_aircraft();
                }
            }
        });
    });

    adsb_data
}

/// Detect feed config changes and spawn/shutdown connections as needed.
pub fn reconnect_on_feed_changes(
    app_config: Res<config::AppConfig>,
    mut manager: ResMut<FeedConnectionManager>,
    map_state: Res<MapState>,
) {
    if !app_config.is_changed() {
        return;
    }

    if app_config.feeds == manager.prev_feed_snapshot {
        return;
    }

    let new_feeds: HashMap<String, &FeedSourceConfig> = app_config
        .feeds
        .iter()
        .map(|f| (f.id.clone(), f))
        .collect();

    let mut to_remove = Vec::new();
    for (id, conn) in &manager.connections {
        match new_feeds.get(id) {
            None => {
                info!("[{}] Feed removed, shutting down", conn.config.name);
                conn.data.request_shutdown();
                to_remove.push(id.clone());
            }
            Some(new_config) => {
                if !new_config.enabled {
                    info!("[{}] Feed disabled, shutting down", conn.config.name);
                    conn.data.request_shutdown();
                    to_remove.push(id.clone());
                } else if new_config.endpoint != conn.config.endpoint
                    || new_config.protocol != conn.config.protocol
                {
                    info!("[{}] Feed config changed, reconnecting", conn.config.name);
                    conn.data.request_shutdown();
                    to_remove.push(id.clone());
                }
            }
        }
    }

    for id in &to_remove {
        manager.connections.remove(id);
    }

    for feed in &app_config.feeds {
        if feed.enabled && !manager.connections.contains_key(&feed.id) {
            info!(
                "[{}] Spawning new feed connection to {}",
                feed.name, feed.endpoint
            );
            let data = spawn_feed_client(feed, map_state.latitude, map_state.longitude);
            manager.connections.insert(
                feed.id.clone(),
                FeedConnection {
                    config: feed.clone(),
                    data,
                },
            );
        }
    }

    manager.prev_feed_snapshot = app_config.feeds.clone();
}

/// Update the connection status UI indicator (aggregate across all feeds).
pub fn update_connection_status(
    manager: Option<Res<FeedConnectionManager>>,
    mut status_query: Query<(&mut Text, &mut TextColor), With<ConnectionStatusText>>,
    mut debug: Option<ResMut<DebugPanelState>>,
    mut prev_state: Local<String>,
    theme: Res<crate::theme::AppTheme>,
) {
    let Some(manager) = manager else {
        return;
    };

    let total_feeds = manager.connections.len();
    let connected = manager.connected_count();
    let connecting = manager.connecting_count();
    let aircraft_count = manager.unique_aircraft_count();

    let state_label = format!("{}/{} connected", connected, total_feeds);
    if *prev_state != state_label {
        if let Some(ref mut dbg) = debug {
            dbg.push_log(format!("Feeds: {}", state_label));
        }
        *prev_state = state_label;
    }

    for (mut text, mut color) in status_query.iter_mut() {
        let (status_text, status_color) = if total_feeds == 0 {
            ("ADS-B: No feeds".to_string(), theme.text_error())
        } else if connected == total_feeds {
            (
                format!("ADS-B: {} aircraft", aircraft_count),
                theme.text_success(),
            )
        } else if connecting > 0 && connected == 0 {
            ("ADS-B: Connecting...".to_string(), theme.text_warn())
        } else if connected > 0 {
            (
                format!(
                    "ADS-B: {}/{} feeds, {} ac",
                    connected, total_feeds, aircraft_count
                ),
                theme.text_warn(),
            )
        } else {
            ("ADS-B: Disconnected".to_string(), theme.text_error())
        };

        **text = status_text;
        *color = TextColor(status_color);
    }
}
