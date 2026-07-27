mod api;
mod config;
mod dto;
mod feeds;
mod state;
mod ws;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::dto::ServerMessage;
use crate::state::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config_path = Path::new("airjedi-web.toml");
    let config = if config_path.exists() {
        Config::load(config_path).expect("failed to load config")
    } else {
        tracing::info!("no airjedi-web.toml found, using defaults");
        Config {
            server: Default::default(),
            cesium: Default::default(),
            feeds: vec![],
        }
    };

    let listen_addr = config.server.listen.clone();
    let state = Arc::new(AppState::new(config));

    {
        let feed_configs: Vec<_> = state.config.feeds.clone();
        let mut feeds = state.feeds.lock().await;
        for feed_config in feed_configs {
            feeds.add_feed(feed_config);
        }
    }

    let broadcast_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            let feeds = broadcast_state.feeds.lock().await;
            let aircraft = feeds.get_all_aircraft();
            if !aircraft.is_empty() {
                let msg = ServerMessage::Update { aircraft };
                let _ = broadcast_state.broadcast_tx.send(msg);
            }
        }
    });

    let app = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/api/feeds", get(api::get_feeds))
        .route("/api/feeds", post(api::add_feed))
        .route("/api/feeds/{id}", delete(api::delete_feed))
        .route("/api/status", get(api::get_status))
        .route("/api/config", get(api::get_config))
        .route("/api/health", get(|| async { "ok" }))
        .layer(CorsLayer::permissive())
        .fallback_service(ServeDir::new("airjedi-web/client/dist"))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen_addr).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
