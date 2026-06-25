use bevy::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use adsb_client::{
    Client as AdsbClient, ClientConfig, ConnectionConfig, ConnectionState, TrackerConfig,
};

use crate::debug_panel::DebugPanelState;
use crate::{config, constants, MapState};

/// Shared state for aircraft data from the ADS-B client.
/// Updated by the background tokio thread and read by Bevy systems.
#[derive(Resource, Clone)]
pub struct AdsbAircraftData {
    /// Aircraft data keyed by ICAO address
    pub aircraft: Arc<Mutex<Vec<adsb_client::Aircraft>>>,
    /// Current connection state
    pub connection_state: Arc<Mutex<ConnectionState>>,
    /// Signal to stop the background thread
    shutdown: Arc<AtomicBool>,
    /// The endpoint URL this client is connected to
    pub endpoint_url: String,
}

impl AdsbAircraftData {
    pub fn new(endpoint_url: &str) -> Self {
        Self {
            aircraft: Arc::new(Mutex::new(Vec::new())),
            connection_state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
            shutdown: Arc::new(AtomicBool::new(false)),
            endpoint_url: endpoint_url.to_string(),
        }
    }

    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    /// Try to get a snapshot of all tracked aircraft without blocking.
    /// Returns `None` if the background thread currently holds the lock.
    pub fn try_get_aircraft(&self) -> Option<Vec<adsb_client::Aircraft>> {
        self.aircraft.try_lock().ok().map(|a| a.clone())
    }

    /// Try to get the aircraft count without cloning the full Vec.
    /// Returns `None` if the lock is held.
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

/// Component to mark the connection status UI text
#[derive(Component)]
pub struct ConnectionStatusText;

/// Setup the ADS-B client in a background thread with its own tokio runtime.
pub fn setup_adsb_client(
    mut commands: Commands,
    map_state: Res<MapState>,
    app_config: Res<config::AppConfig>,
) {
    let endpoint_url = app_config.feed.endpoint_url.clone();
    let center_lat = map_state.latitude;
    let center_lon = map_state.longitude;

    let adsb_data = spawn_adsb_client(&endpoint_url, center_lat, center_lon);
    commands.insert_resource(adsb_data);
    info!("ADS-B client background thread started");
}

fn spawn_adsb_client(endpoint_url: &str, center_lat: f64, center_lon: f64) -> AdsbAircraftData {
    let adsb_data = AdsbAircraftData::new(endpoint_url);
    let aircraft_data = Arc::clone(&adsb_data.aircraft);
    let connection_state = Arc::clone(&adsb_data.connection_state);
    let shutdown = Arc::clone(&adsb_data.shutdown);
    let endpoint_url = endpoint_url.to_string();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for ADS-B client");

        rt.block_on(async move {
            info!("Starting ADS-B client, connecting to {}", endpoint_url);

            let mut client = AdsbClient::spawn(ClientConfig {
                connection: ConnectionConfig {
                    address: endpoint_url.clone(),
                    ..Default::default()
                },
                tracker: TrackerConfig {
                    center: Some((center_lat, center_lon)),
                    max_distance_miles: constants::ADSB_MAX_DISTANCE_MILES,
                    aircraft_timeout_secs: constants::ADSB_AIRCRAFT_TIMEOUT_SECS,
                    ..Default::default()
                },
                ..Default::default()
            });

            loop {
                if shutdown.load(Ordering::Acquire) {
                    info!("ADS-B client shutting down (endpoint changed)");
                    return;
                }

                if !client.process_next().await {
                    if shutdown.load(Ordering::Acquire) {
                        return;
                    }
                    warn!("ADS-B client connection closed, restarting...");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }

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

pub fn reconnect_on_config_change(
    mut commands: Commands,
    app_config: Res<config::AppConfig>,
    existing: Option<Res<AdsbAircraftData>>,
    map_state: Res<MapState>,
) {
    if !app_config.is_changed() {
        return;
    }

    let Some(existing) = existing else {
        return;
    };

    if existing.endpoint_url == app_config.feed.endpoint_url {
        return;
    }

    info!(
        "Feed endpoint changed from {} to {}, reconnecting",
        existing.endpoint_url, app_config.feed.endpoint_url
    );

    existing.request_shutdown();

    let new_data = spawn_adsb_client(
        &app_config.feed.endpoint_url,
        map_state.latitude,
        map_state.longitude,
    );
    commands.insert_resource(new_data);
}

/// Update the connection status UI indicator
pub fn update_connection_status(
    adsb_data: Option<Res<AdsbAircraftData>>,
    mut status_query: Query<(&mut Text, &mut TextColor), With<ConnectionStatusText>>,
    mut debug: Option<ResMut<DebugPanelState>>,
    mut prev_state: Local<String>,
    theme: Res<crate::theme::AppTheme>,
) {
    let Some(adsb_data) = adsb_data else {
        return;
    };

    let connection_state = adsb_data.get_connection_state();
    let aircraft_count = adsb_data.try_aircraft_count().unwrap_or(0);

    // Log connection state transitions
    let state_label = format!("{:?}", connection_state);
    if *prev_state != state_label {
        if let Some(ref mut dbg) = debug {
            dbg.push_log(format!("Connection: {}", state_label));
        }
        *prev_state = state_label;
    }

    for (mut text, mut color) in status_query.iter_mut() {
        let (status_text, status_color) = match connection_state {
            ConnectionState::Connected => (
                format!("ADS-B: {} aircraft", aircraft_count),
                theme.text_success(),
            ),
            ConnectionState::Connecting => ("ADS-B: Connecting...".to_string(), theme.text_warn()),
            ConnectionState::Disconnected => {
                ("ADS-B: Disconnected".to_string(), theme.text_error())
            }
            ConnectionState::Error(ref msg) => {
                (format!("ADS-B: Error - {}", msg), theme.text_error())
            }
        };

        **text = status_text;
        *color = TextColor(status_color);
    }
}
