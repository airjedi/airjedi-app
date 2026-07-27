use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use uuid::Uuid;

use crate::config::FeedConfig;
use crate::state::SharedState;

#[derive(Serialize)]
pub struct StatusResponse {
    pub aircraft_count: usize,
    pub feeds: Vec<crate::dto::FeedStatusDto>,
}

#[derive(Serialize)]
pub struct ClientConfig {
    pub cesium_ion_token: Option<String>,
    pub center_lat: f64,
    pub center_lon: f64,
}

pub async fn get_feeds(State(state): State<SharedState>) -> Json<Vec<FeedConfig>> {
    let feeds = state.feeds.lock().await;
    Json(feeds.feed_configs())
}

pub async fn add_feed(
    State(state): State<SharedState>,
    Json(mut config): Json<FeedConfig>,
) -> (StatusCode, Json<FeedConfig>) {
    config.id = Uuid::new_v4();
    let mut feeds = state.feeds.lock().await;
    feeds.add_feed(config.clone());
    (StatusCode::CREATED, Json(config))
}

pub async fn delete_feed(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut feeds = state.feeds.lock().await;
    feeds.remove_feed(id);
    StatusCode::NO_CONTENT
}

pub async fn get_status(State(state): State<SharedState>) -> Json<StatusResponse> {
    let feeds = state.feeds.lock().await;
    let aircraft = feeds.get_all_aircraft();
    let statuses = feeds.feed_statuses();
    Json(StatusResponse {
        aircraft_count: aircraft.len(),
        feeds: statuses,
    })
}

pub async fn get_config(State(state): State<SharedState>) -> Json<ClientConfig> {
    Json(ClientConfig {
        cesium_ion_token: state.config.cesium_ion_token(),
        center_lat: state.config.server.center_lat(),
        center_lon: state.config.server.center_lon(),
    })
}
