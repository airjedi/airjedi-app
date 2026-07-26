use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub cesium: CesiumConfig,
    #[serde(default)]
    pub feeds: Vec<FeedConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
        }
    }
}

fn default_listen() -> String {
    "0.0.0.0:3000".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct CesiumConfig {
    pub ion_token: Option<String>,
}

impl Default for CesiumConfig {
    fn default() -> Self {
        Self { ion_token: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedConfig {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub address: String,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

fn default_protocol() -> String {
    "basestation".to_string()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    pub fn cesium_ion_token(&self) -> Option<String> {
        std::env::var("CESIUM_ION_TOKEN")
            .ok()
            .or_else(|| self.cesium.ion_token.clone())
    }
}
