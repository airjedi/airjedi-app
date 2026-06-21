pub mod messages;
#[cfg(feature = "nats")]
pub mod nats;

use std::time::Duration;
use crate::sensor::FusionTier;

#[derive(Debug, Clone)]
pub struct NatsTransportConfig {
    pub server_url: String,
    pub node_id: String,
    pub tier: FusionTier,
    pub publish_interval: Duration,
    pub subscriptions: Vec<SubConfig>,
    pub jetstream: JetStreamConfig,
}

impl Default for NatsTransportConfig {
    fn default() -> Self {
        Self {
            server_url: "nats://localhost:4222".to_string(),
            node_id: "local".to_string(),
            tier: FusionTier::Regional,
            publish_interval: Duration::from_secs(1),
            subscriptions: Vec::new(),
            jetstream: JetStreamConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SubConfig {
    pub subject: String,
}

#[derive(Debug, Clone)]
pub struct JetStreamConfig {
    pub stream_name: String,
    pub max_age: Duration,
    pub max_bytes: u64,
    pub replicas: u8,
}

impl Default for JetStreamConfig {
    fn default() -> Self {
        Self {
            stream_name: "FUSION_TRACKS".to_string(),
            max_age: Duration::from_secs(300),
            max_bytes: 100 * 1024 * 1024,
            replicas: 1,
        }
    }
}
