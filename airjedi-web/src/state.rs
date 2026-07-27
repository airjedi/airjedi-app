use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

use crate::config::Config;
use crate::dto::ServerMessage;
use crate::feeds::FeedManager;

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub feeds: Mutex<FeedManager>,
    pub broadcast_tx: broadcast::Sender<ServerMessage>,
    pub config: Config,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let (broadcast_tx, _) = broadcast::channel(64);
        Self {
            feeds: Mutex::new(FeedManager::new(Some((
                config.server.center_lat(),
                config.server.center_lon(),
            )))),
            broadcast_tx,
            config,
        }
    }
}
