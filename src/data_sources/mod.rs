//! Multiple Data Sources Module
//!
//! Support for multiple simultaneous ADS-B data feeds.
//! Allows configuring multiple TCP endpoints and merging aircraft data.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a single data source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceConfig {
    /// Display name for this source
    pub name: String,
    /// Connection endpoint (host:port)
    pub endpoint: String,
    /// Whether this source is enabled
    pub enabled: bool,
    /// Priority (higher = preferred for duplicate aircraft)
    pub priority: u8,
    /// Optional: receiver location for this feed
    pub receiver_location: Option<(f64, f64)>,
}

impl Default for DataSourceConfig {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            endpoint: "127.0.0.1:30003".to_string(),
            enabled: true,
            priority: 100,
            receiver_location: None,
        }
    }
}

/// Status of a data source connection
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DataSourceStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

impl DataSourceStatus {
    pub fn is_connected(&self) -> bool {
        matches!(self, DataSourceStatus::Connected)
    }
}

/// Runtime state for a data source
#[derive(Debug, Default, Clone)]
pub struct DataSourceState {
    /// Current connection status
    pub status: DataSourceStatus,
    /// Number of aircraft currently tracked from this source
    pub aircraft_count: usize,
    /// Total messages received
    pub messages_received: u64,
    /// Last message timestamp
    pub last_message_time: Option<std::time::Instant>,
}

/// Tracking data for an aircraft from a specific source
#[derive(Debug, Clone)]
pub struct SourcedAircraftData {
    /// ICAO 24-bit address
    pub icao: String,
    /// Source name
    pub source: String,
    /// Source priority
    pub priority: u8,
    /// Last update time
    pub last_update: std::time::Instant,
    /// Position data
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<i32>,
    pub heading: Option<f32>,
    pub velocity: Option<f64>,
    pub vertical_rate: Option<i32>,
    pub callsign: Option<String>,
    pub squawk: Option<String>,
}

/// Resource managing multiple data sources
#[derive(Resource)]
pub struct DataSourceManager {
    /// Configured data sources
    pub sources: Vec<DataSourceConfig>,
    /// Runtime state for each source (indexed by source name)
    pub states: HashMap<String, DataSourceState>,
    /// Merged aircraft data (ICAO -> best data from all sources)
    pub aircraft: HashMap<String, MergedAircraftData>,
    /// Whether to show source indicator on aircraft
    pub show_source_indicator: bool,
    /// Whether to show data source panel
    pub show_panel: bool,
}

impl Default for DataSourceManager {
    fn default() -> Self {
        Self {
            sources: vec![DataSourceConfig::default()],
            states: HashMap::new(),
            aircraft: HashMap::new(),
            show_source_indicator: false,
            show_panel: false,
        }
    }
}

/// Merged aircraft data from multiple sources
#[derive(Debug, Clone)]
pub struct MergedAircraftData {
    pub icao: String,
    /// Best callsign (from highest priority source)
    pub callsign: Option<String>,
    /// Best position (from highest priority source with valid position)
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Option<i32>,
    pub heading: Option<f32>,
    pub velocity: Option<f64>,
    pub vertical_rate: Option<i32>,
    pub squawk: Option<String>,
    /// Which sources are reporting this aircraft
    pub sources: Vec<String>,
    /// Primary source name (highest priority with data)
    pub primary_source: String,
}

impl DataSourceManager {
    /// Add a new data source
    pub fn add_source(&mut self, config: DataSourceConfig) {
        if !self.sources.iter().any(|s| s.name == config.name) {
            self.states
                .insert(config.name.clone(), DataSourceState::default());
            self.sources.push(config);
        }
    }

    /// Remove a data source by name
    pub fn remove_source(&mut self, name: &str) {
        self.sources.retain(|s| s.name != name);
        self.states.remove(name);
    }

    /// Update aircraft data from a source
    ///
    /// This handles merging data from multiple sources for the same aircraft.
    pub fn update_aircraft(&mut self, data: SourcedAircraftData) {
        let entry = self
            .aircraft
            .entry(data.icao.clone())
            .or_insert_with(|| MergedAircraftData {
                icao: data.icao.clone(),
                callsign: None,
                latitude: 0.0,
                longitude: 0.0,
                altitude: None,
                heading: None,
                velocity: None,
                vertical_rate: None,
                squawk: None,
                sources: Vec::new(),
                primary_source: data.source.clone(),
            });

        // Track which sources report this aircraft
        if !entry.sources.contains(&data.source) {
            entry.sources.push(data.source.clone());
        }

        // Find priority of current primary source
        let primary_priority = self
            .sources
            .iter()
            .find(|s| s.name == entry.primary_source)
            .map(|s| s.priority)
            .unwrap_or(0);

        // Update if this source has higher or equal priority
        if data.priority >= primary_priority {
            entry.primary_source = data.source.clone();

            if let Some(lat) = data.latitude {
                entry.latitude = lat;
            }
            if let Some(lon) = data.longitude {
                entry.longitude = lon;
            }
            if data.altitude.is_some() {
                entry.altitude = data.altitude;
            }
            if data.heading.is_some() {
                entry.heading = data.heading;
            }
            if data.velocity.is_some() {
                entry.velocity = data.velocity;
            }
            if data.vertical_rate.is_some() {
                entry.vertical_rate = data.vertical_rate;
            }
            if data.callsign.is_some() {
                entry.callsign = data.callsign.clone();
            }
            if data.squawk.is_some() {
                entry.squawk = data.squawk.clone();
            }
        }
    }

    /// Get source statistics
    pub fn get_stats(&self) -> DataSourceStats {
        let connected_count = self
            .states
            .values()
            .filter(|s| s.status.is_connected())
            .count();
        let total_aircraft = self.aircraft.len();
        let total_messages: u64 = self.states.values().map(|s| s.messages_received).sum();

        DataSourceStats {
            total_sources: self.sources.len(),
            connected_sources: connected_count,
            total_aircraft,
            total_messages,
        }
    }
}

/// Summary statistics
#[derive(Debug, Clone)]
pub struct DataSourceStats {
    pub total_sources: usize,
    pub connected_sources: usize,
    pub total_aircraft: usize,
    pub total_messages: u64,
}

/// System to toggle data sources panel
pub fn toggle_data_sources_panel(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut manager: ResMut<DataSourceManager>,
    mut contexts: EguiContexts,
) {
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    // Shift+D - Toggle data sources panel
    if keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight) {
        if keyboard.just_pressed(KeyCode::KeyD) {
            manager.show_panel = !manager.show_panel;
        }
    }
}

/// System to render data sources panel
pub fn render_data_sources_panel(
    mut contexts: EguiContexts,
    mut manager: ResMut<DataSourceManager>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    if !manager.show_panel {
        return;
    }

    let stats = manager.get_stats();

    egui::Window::new("Data Sources")
        .collapsible(true)
        .resizable(true)
        .default_width(350.0)
        .show(ctx, |ui| {
            // Summary
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{}/{} sources connected",
                    stats.connected_sources, stats.total_sources
                ));
                ui.separator();
                ui.label(format!("{} aircraft", stats.total_aircraft));
            });

            ui.separator();

            // Source list
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                let sources: Vec<_> = manager.sources.iter().cloned().collect();
                let states = manager.states.clone();

                for source in &sources {
                    let state = states.get(&source.name);
                    let status = state.map(|s| &s.status).unwrap_or(&DataSourceStatus::Disconnected);

                    ui.horizontal(|ui| {
                        // Status indicator
                        let (color, text) = match status {
                            DataSourceStatus::Connected => (egui::Color32::GREEN, "OK"),
                            DataSourceStatus::Connecting => (egui::Color32::YELLOW, "..."),
                            DataSourceStatus::Disconnected => (egui::Color32::GRAY, "OFF"),
                            DataSourceStatus::Error(_) => (egui::Color32::RED, "ERR"),
                        };
                        ui.colored_label(color, text);

                        ui.label(&source.name);
                        ui.label(
                            egui::RichText::new(&source.endpoint)
                                .size(11.0)
                                .color(egui::Color32::GRAY)
                        );

                        if let Some(state) = state {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(format!("{} ac", state.aircraft_count));
                            });
                        }
                    });

                    if let DataSourceStatus::Error(ref msg) = status {
                        ui.label(
                            egui::RichText::new(msg)
                                .size(10.0)
                                .color(egui::Color32::RED)
                        );
                    }
                }
            });

            ui.separator();

            // Note about implementation status
            ui.label(
                egui::RichText::new("Note: Multi-source connection requires\nmodifications to the adsb_client integration")
                    .size(11.0)
                    .color(egui::Color32::GRAY)
            );

            ui.horizontal(|ui| {
                ui.checkbox(&mut manager.show_source_indicator, "Show source on aircraft");
            });
        });
}

/// Plugin for multiple data sources
pub struct DataSourcesPlugin;

impl Plugin for DataSourcesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DataSourceManager>()
            .add_systems(Update, toggle_data_sources_panel);
        // Data sources panel is rendered via the consolidated Tools window (tools_window.rs)
    }
}
