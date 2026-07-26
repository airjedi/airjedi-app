use std::collections::HashMap;
use uuid::Uuid;

use adsb_client::{Client, ClientConfig, ConnectionConfig, ProtocolType, TrackerConfig};

use crate::config::FeedConfig;
use crate::dto::{AircraftDto, FeedStatusDto};

struct ManagedFeed {
    id: Uuid,
    address: String,
    protocol: String,
    client: Client,
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
            "beast" => ProtocolType::Beast,
            _ => ProtocolType::BaseStation,
        };

        let client = Client::spawn(ClientConfig {
            connection: ConnectionConfig {
                address: config.address.clone(),
                ..Default::default()
            },
            tracker: TrackerConfig::default(),
            protocol,
            ..Default::default()
        });

        tracing::info!("added feed {}: {}", config.id, config.address);

        self.feeds.insert(
            config.id,
            ManagedFeed {
                id: config.id,
                address: config.address,
                protocol: config.protocol,
                client,
            },
        );
    }

    pub fn remove_feed(&mut self, id: Uuid) {
        if let Some(feed) = self.feeds.remove(&id) {
            feed.client.shutdown();
            tracing::info!("removed feed {}: {}", id, feed.address);
        }
    }

    pub fn get_all_aircraft(&self) -> Vec<AircraftDto> {
        let mut merged: HashMap<String, AircraftDto> = HashMap::new();

        for feed in self.feeds.values() {
            for ac in feed.client.get_aircraft() {
                let dto = AircraftDto::from_aircraft(&ac, self.max_trail_points);
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
            .map(|f| FeedStatusDto {
                id: f.id,
                address: f.address.clone(),
                state: format!("{:?}", f.client.connection_state()),
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
