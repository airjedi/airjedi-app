use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

use adsb_client::{Aircraft, Client, ClientConfig, ConnectionConfig, ConnectionState, ProtocolType, TrackerConfig};

use crate::config::FeedConfig;
use crate::dto::{AircraftDto, FeedStatusDto};

struct ManagedFeed {
    id: Uuid,
    address: String,
    protocol: String,
    aircraft: Arc<RwLock<Vec<Aircraft>>>,
    connection_state: Arc<RwLock<ConnectionState>>,
    shutdown: tokio::sync::watch::Sender<bool>,
}

pub struct FeedManager {
    feeds: HashMap<Uuid, ManagedFeed>,
    max_trail_points: usize,
}

impl FeedManager {
    pub fn new() -> Self {
        Self {
            feeds: HashMap::new(),
            max_trail_points: 100,
        }
    }

    pub fn add_feed(&mut self, config: FeedConfig) {
        let protocol = match config.protocol.to_lowercase().as_str() {
            "basestation" | "sbs" | "sbs1" => ProtocolType::BaseStation,
            _ => ProtocolType::Beast,
        };

        let mut client = Client::spawn(ClientConfig {
            connection: ConnectionConfig {
                address: config.address.clone(),
                ..Default::default()
            },
            tracker: TrackerConfig {
                center: Some((37.6872, -97.3301)),
                ..Default::default()
            },
            protocol,
            ..Default::default()
        });

        let aircraft: Arc<RwLock<Vec<Aircraft>>> = Arc::new(RwLock::new(Vec::new()));
        let conn_state: Arc<RwLock<ConnectionState>> = Arc::new(RwLock::new(ConnectionState::Disconnected));
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        let ac_writer = Arc::clone(&aircraft);
        let cs_writer = Arc::clone(&conn_state);

        tokio::spawn(async move {
            let mut last_snapshot = std::time::Instant::now();
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        client.shutdown();
                        break;
                    }
                    result = client.process_next() => {
                        if !result {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            continue;
                        }

                        if last_snapshot.elapsed() >= std::time::Duration::from_millis(200) {
                            if let Ok(mut ac) = ac_writer.write() {
                                *ac = client.get_aircraft();
                            }
                            if let Ok(mut cs) = cs_writer.write() {
                                *cs = client.connection_state();
                            }
                            last_snapshot = std::time::Instant::now();
                        }
                    }
                }
            }
        });

        tracing::info!("added feed {}: {}", config.id, config.address);

        self.feeds.insert(
            config.id,
            ManagedFeed {
                id: config.id,
                address: config.address,
                protocol: config.protocol,
                aircraft,
                connection_state: conn_state,
                shutdown: shutdown_tx,
            },
        );
    }

    pub fn remove_feed(&mut self, id: Uuid) {
        if let Some(feed) = self.feeds.remove(&id) {
            let _ = feed.shutdown.send(true);
            tracing::info!("removed feed {}: {}", id, feed.address);
        }
    }

    pub fn get_all_aircraft(&self) -> Vec<AircraftDto> {
        let mut merged: HashMap<String, AircraftDto> = HashMap::new();

        for feed in self.feeds.values() {
            let aircraft = feed.aircraft.read().unwrap_or_else(|e| e.into_inner());
            for ac in aircraft.iter() {
                let dto = AircraftDto::from_aircraft(ac, self.max_trail_points);
                merged
                    .entry(dto.icao.clone())
                    .and_modify(|existing| {
                        if dto.last_seen > existing.last_seen {
                            *existing = dto.clone();
                        }
                    })
                    .or_insert(dto);
            }
        }

        merged.into_values().collect()
    }

    pub fn feed_statuses(&self) -> Vec<FeedStatusDto> {
        self.feeds
            .values()
            .map(|f| {
                let state = f.connection_state.read()
                    .map(|s| format!("{:?}", *s))
                    .unwrap_or_else(|_| "Unknown".to_string());
                FeedStatusDto {
                    id: f.id,
                    address: f.address.clone(),
                    state,
                }
            })
            .collect()
    }

    pub fn feed_configs(&self) -> Vec<FeedConfig> {
        self.feeds
            .values()
            .map(|f| FeedConfig {
                id: f.id,
                address: f.address.clone(),
                protocol: f.protocol.clone(),
            })
            .collect()
    }
}
