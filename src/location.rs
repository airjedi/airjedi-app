//! Per-feed receiver GPS location management.
//!
//! Provides edit buffers for lat/lon text entry and IP-based geolocation
//! as a convenient "current location" fallback. The IP-based lookup
//! (city-level accuracy) is good enough for fixed RTL-SDR installations
//! where the Mac running AirJedi is at the same location as the receiver.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::config::AppConfig;
use crate::coverage::CoverageState;

/// In-progress IP geolocation fetch for a single feed.
pub struct PendingFetch {
    pub feed_id: String,
    pub receiver: crossbeam_channel::Receiver<Result<(f64, f64), String>>,
}

/// UI state for the feed receiver location editor.
///
/// Stored as a Bevy resource so text buffers survive across frames.
#[derive(Resource, Default)]
pub struct FeedLocationUiState {
    /// Lat/lon string edit buffers keyed by feed ID.
    pub buffers: HashMap<String, (String, String)>,
    /// In-progress background location fetches.
    pub fetches: Vec<PendingFetch>,
    /// Per-feed status message displayed after a fetch completes.
    pub status: HashMap<String, String>,
}

impl FeedLocationUiState {
    /// Ensure edit buffers exist for all current feeds, seeded from config values.
    pub fn sync_buffers_from_config(&mut self, config: &AppConfig) {
        for feed in &config.feeds {
            self.buffers.entry(feed.id.clone()).or_insert_with(|| {
                buf_from_location(feed.receiver_location)
            });
        }
    }

    /// Overwrite the buffer for a feed with the value from config (after a fetch lands).
    pub fn update_buffer_from_config(&mut self, feed_id: &str, loc: Option<(f64, f64)>) {
        self.buffers
            .insert(feed_id.to_string(), buf_from_location(loc));
    }

    /// Whether a fetch is running for the given feed.
    pub fn is_fetching(&self, feed_id: &str) -> bool {
        self.fetches.iter().any(|f| f.feed_id == feed_id)
    }
}

fn buf_from_location(loc: Option<(f64, f64)>) -> (String, String) {
    match loc {
        Some((lat, lon)) => (format!("{:.6}", lat), format!("{:.6}", lon)),
        None => (String::new(), String::new()),
    }
}

/// Start an IP-based location fetch for the given feed.
///
/// Does nothing if a fetch is already pending for that feed.
pub fn request_ip_location(feed_id: String, ui_state: &mut FeedLocationUiState) {
    if ui_state.is_fetching(&feed_id) {
        return;
    }
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send(fetch_ip_location_blocking());
    });
    ui_state.fetches.push(PendingFetch {
        feed_id,
        receiver: rx,
    });
}

fn fetch_ip_location_blocking() -> Result<(f64, f64), String> {
    let response = reqwest::blocking::get("https://ipinfo.io/json")
        .map_err(|e| format!("Network error: {}", e))?;
    let json: serde_json::Value = response
        .json()
        .map_err(|e| format!("Parse error: {}", e))?;

    // ipinfo.io returns loc as "lat,lon" string
    let loc_str = json["loc"]
        .as_str()
        .ok_or_else(|| "Missing location field".to_string())?;
    let mut parts = loc_str.splitn(2, ',');
    let lat = parts
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or_else(|| "Invalid latitude".to_string())?;
    let lon = parts
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or_else(|| "Invalid longitude".to_string())?;
    Ok((lat, lon))
}

/// System: poll completed location fetches and apply results to AppConfig + buffers.
pub fn poll_location_fetches(
    mut location_ui: ResMut<FeedLocationUiState>,
    mut app_config: ResMut<AppConfig>,
) {
    // Collect results first so the immutable borrow of fetches is released before
    // we mutate buffers/status on the same resource.
    let results: Vec<(usize, String, Result<(f64, f64), String>)> = location_ui
        .fetches
        .iter()
        .enumerate()
        .filter_map(|(i, pending)| {
            pending
                .receiver
                .try_recv()
                .ok()
                .map(|r| (i, pending.feed_id.clone(), r))
        })
        .collect();

    let mut config_changed = false;
    for (_, feed_id, result) in &results {
        match result {
            Ok((lat, lon)) => {
                if let Some(feed) = app_config.feeds.iter_mut().find(|f| f.id == *feed_id) {
                    feed.receiver_location = Some((*lat, *lon));
                    config_changed = true;
                }
                location_ui.buffers.insert(
                    feed_id.clone(),
                    (format!("{:.6}", lat), format!("{:.6}", lon)),
                );
                location_ui
                    .status
                    .insert(feed_id.clone(), format!("Located: {:.4}, {:.4}", lat, lon));
            }
            Err(e) => {
                location_ui
                    .status
                    .insert(feed_id.clone(), format!("Error: {}", e));
            }
        }
    }

    if config_changed {
        crate::config::save_config(&app_config);
    }

    // Remove completed entries in reverse index order to keep indices valid.
    for (i, _, _) in results.into_iter().rev() {
        location_ui.fetches.remove(i);
    }
}

/// System: propagate the first enabled feed's receiver location to CoverageState.
pub fn sync_receiver_location_to_coverage(
    app_config: Res<AppConfig>,
    mut coverage: ResMut<CoverageState>,
) {
    if !app_config.is_changed() {
        return;
    }
    if let Some(loc) = app_config
        .feeds
        .iter()
        .filter(|f| f.enabled)
        .filter_map(|f| f.receiver_location)
        .next()
    {
        coverage.receiver_location = loc;
    }
}

pub struct LocationPlugin;

impl Plugin for LocationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FeedLocationUiState>()
            .add_systems(Update, (poll_location_fetches, sync_receiver_location_to_coverage));
    }
}
