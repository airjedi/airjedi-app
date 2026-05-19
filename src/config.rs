use bevy::prelude::*;
use bevy_egui::{egui, EguiPlugin};
use bevy_slippy_tiles::{SlippyTilesSettings, TileFormat};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::aircraft::TrailRenderer;
use crate::theme::{AppTheme, ThemeRegistry};

const CONFIG_FILE: &str = "config.toml";

fn default_interpolation_enabled() -> bool { true }

/// Available basemap styles for the map display
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BasemapStyle {
    #[default]
    CartoDark,
    CartoLight,
    OpenStreetMap,
    EsriSatellite,
}

impl BasemapStyle {
    /// Get the tile URL endpoint for this basemap style
    pub fn endpoint_url(&self) -> &'static str {
        match self {
            BasemapStyle::CartoDark => "https://basemaps.cartocdn.com/dark_all",
            BasemapStyle::CartoLight => "https://basemaps.cartocdn.com/light_all",
            BasemapStyle::OpenStreetMap => "https://tile.openstreetmap.org",
            BasemapStyle::EsriSatellite => "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile",
        }
    }

    /// Get the display name for UI
    pub fn display_name(&self) -> &'static str {
        match self {
            BasemapStyle::CartoDark => "CartoDB Dark",
            BasemapStyle::CartoLight => "CartoDB Light",
            BasemapStyle::OpenStreetMap => "OpenStreetMap",
            BasemapStyle::EsriSatellite => "ESRI Satellite",
        }
    }

    /// Get the tile image format served by this basemap provider
    pub fn tile_format(&self) -> TileFormat {
        match self {
            BasemapStyle::EsriSatellite => TileFormat::Jpg,
            _ => TileFormat::Png,
        }
    }

    /// Whether this basemap uses /{z}/{y}/{x} instead of /{z}/{x}/{y}
    pub fn reverse_axes(&self) -> bool {
        matches!(self, BasemapStyle::EsriSatellite)
    }

    /// The native tile pixel size served by this provider.
    /// CartoDB supports @2x (512px) tiles. OSM and ESRI only serve 256px
    /// tiles regardless of the requested size.
    pub fn native_tile_pixels(&self) -> u32 {
        match self {
            BasemapStyle::CartoDark | BasemapStyle::CartoLight => 512,
            BasemapStyle::OpenStreetMap | BasemapStyle::EsriSatellite => 256,
        }
    }

    /// Get all available styles
    pub fn all() -> &'static [BasemapStyle] {
        &[
            BasemapStyle::CartoDark,
            BasemapStyle::CartoLight,
            BasemapStyle::OpenStreetMap,
            BasemapStyle::EsriSatellite,
        ]
    }
}

#[derive(Resource, Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    pub feed: FeedConfig,
    pub map: MapConfig,
    #[serde(default)]
    pub overlays: OverlayConfig,
    #[serde(default)]
    pub trails: TrailsConfig,
    #[serde(default)]
    pub bookmarks: BookmarksConfig,
    #[serde(default)]
    pub appearance: AppearanceConfig,
    #[serde(default)]
    pub data_ingest: DataIngestConfig,
    #[serde(default = "default_interpolation_enabled")]
    pub interpolation_enabled: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OverlayConfig {
    pub show_airports: bool,
    pub show_runways: bool,
    pub show_navaids: bool,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            show_airports: true,
            show_runways: true,
            show_navaids: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrailsConfig {
    pub enabled: bool,
    pub max_age_seconds: u64,
    #[serde(default)]
    pub renderer_2d: TrailRenderer,
    #[serde(default = "TrailsConfig::default_renderer_3d")]
    pub renderer_3d: TrailRenderer,
}

impl TrailsConfig {
    fn default_renderer_3d() -> TrailRenderer {
        #[cfg(feature = "hanabi")]
        { TrailRenderer::Particle }
        #[cfg(not(feature = "hanabi"))]
        { TrailRenderer::Gizmo }
    }
}

impl Default for TrailsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_age_seconds: 300,
            renderer_2d: TrailRenderer::default(),
            renderer_3d: Self::default_renderer_3d(),
        }
    }
}

/// Bookmark for a specific aircraft by ICAO address
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AircraftBookmark {
    /// ICAO 24-bit address (hex string)
    pub icao: String,
    /// Optional callsign for display (may change between flights)
    pub callsign: Option<String>,
    /// User-defined note
    pub note: Option<String>,
}

/// Bookmark for a map location
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LocationBookmark {
    /// Display name for this location
    pub name: String,
    /// Latitude in degrees
    pub latitude: f64,
    /// Longitude in degrees
    pub longitude: f64,
    /// Zoom level
    pub zoom: u8,
}

/// Bookmarks configuration
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct BookmarksConfig {
    /// Bookmarked aircraft
    #[serde(default)]
    pub aircraft: Vec<AircraftBookmark>,
    /// Bookmarked locations
    #[serde(default)]
    pub locations: Vec<LocationBookmark>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppearanceConfig {
    pub theme: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "Catppuccin Mocha".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FeedConfig {
    pub endpoint_url: String,
    pub refresh_interval_ms: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MapConfig {
    pub default_latitude: f64,
    pub default_longitude: f64,
    pub default_zoom: u8,
    #[serde(default)]
    pub basemap_style: BasemapStyle,
}

/// Configuration for a single data ingest provider.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProviderConfig {
    pub enabled: bool,
    /// Cron schedule (6-field: sec min hour dom month dow).
    pub schedule: String,
    /// Optional API key / client_id for providers that require auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Optional API secret / client_secret for providers that require auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_secret: Option<String>,
}

/// Data ingest configuration with per-provider settings.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DataIngestConfig {
    #[serde(default = "DataIngestConfig::default_metar")]
    pub metar: ProviderConfig,
    #[serde(default = "DataIngestConfig::default_taf")]
    pub taf: ProviderConfig,
    #[serde(default = "DataIngestConfig::default_ourairports")]
    pub ourairports: ProviderConfig,
    #[serde(default = "DataIngestConfig::default_faa_nasr")]
    pub faa_nasr: ProviderConfig,
    #[serde(default = "DataIngestConfig::default_openaip")]
    pub openaip: ProviderConfig,
    #[serde(default = "DataIngestConfig::default_notam")]
    pub notam: ProviderConfig,
    #[serde(default = "DataIngestConfig::default_tfr")]
    pub tfr: ProviderConfig,
    #[serde(default = "DataIngestConfig::default_faa_airspace")]
    pub faa_airspace: ProviderConfig,
}

impl DataIngestConfig {
    fn default_metar() -> ProviderConfig {
        ProviderConfig { enabled: true, schedule: "0 */5 * * * *".into(), api_key: None, api_secret: None }
    }
    fn default_taf() -> ProviderConfig {
        ProviderConfig { enabled: true, schedule: "0 */15 * * * *".into(), api_key: None, api_secret: None }
    }
    fn default_ourairports() -> ProviderConfig {
        ProviderConfig { enabled: true, schedule: "0 0 3 * * *".into(), api_key: None, api_secret: None }
    }
    fn default_faa_nasr() -> ProviderConfig {
        ProviderConfig { enabled: false, schedule: "0 0 4 * * *".into(), api_key: None, api_secret: None }
    }
    fn default_openaip() -> ProviderConfig {
        ProviderConfig { enabled: false, schedule: "0 0 4 * * *".into(), api_key: None, api_secret: None }
    }
    fn default_notam() -> ProviderConfig {
        ProviderConfig { enabled: false, schedule: "0 */30 * * * *".into(), api_key: None, api_secret: None }
    }
    fn default_tfr() -> ProviderConfig {
        ProviderConfig { enabled: true, schedule: "0 */15 * * * *".into(), api_key: None, api_secret: None }
    }
    fn default_faa_airspace() -> ProviderConfig {
        ProviderConfig { enabled: true, schedule: "0 0 4 * * *".into(), api_key: None, api_secret: None }
    }
}

impl Default for DataIngestConfig {
    fn default() -> Self {
        Self {
            metar: Self::default_metar(),
            taf: Self::default_taf(),
            ourairports: Self::default_ourairports(),
            faa_nasr: Self::default_faa_nasr(),
            openaip: Self::default_openaip(),
            notam: Self::default_notam(),
            tfr: Self::default_tfr(),
            faa_airspace: Self::default_faa_airspace(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            feed: FeedConfig {
                // Raw TCP address for ADS-B connection (host:port format)
                endpoint_url: "192.168.1.10:30003".to_string(),
                refresh_interval_ms: 1000,
            },
            map: MapConfig {
                default_latitude: 37.6872,
                default_longitude: -97.3301,
                default_zoom: 10,
                basemap_style: BasemapStyle::default(),
            },
            overlays: OverlayConfig::default(),
            trails: TrailsConfig::default(),
            bookmarks: BookmarksConfig::default(),
            appearance: AppearanceConfig::default(),
            data_ingest: DataIngestConfig::default(),
            interpolation_enabled: true,
        }
    }
}

fn config_path() -> PathBuf {
    let dir = crate::paths::config_dir();
    crate::paths::ensure_dir(&dir);
    dir.join(CONFIG_FILE)
}

pub fn load_config() -> AppConfig {
    let path = config_path();
    if path.exists() {
        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => {
                    info!("Loaded config from {:?}", path);
                    return config;
                }
                Err(e) => {
                    warn!("Failed to parse config: {}, using defaults", e);
                }
            },
            Err(e) => {
                warn!("Failed to read config: {}, using defaults", e);
            }
        }
    }

    let config = AppConfig::default();
    save_config(&config);
    config
}

pub fn save_config(config: &AppConfig) {
    let path = config_path();
    match toml::to_string_pretty(config) {
        Ok(contents) => {
            if let Err(e) = fs::write(&path, contents) {
                error!("Failed to write config: {}", e);
            } else {
                info!("Saved config to {:?}", path);
            }
        }
        Err(e) => {
            error!("Failed to serialize config: {}", e);
        }
    }
}

#[derive(Resource, Default)]
pub struct SettingsUiState {
    pub open: bool,
    pub endpoint_url: String,
    pub refresh_interval_ms: String,
    pub default_latitude: String,
    pub default_longitude: String,
    pub default_zoom: String,
    // Map settings
    pub basemap_style: BasemapStyle,
    // Overlay settings
    pub show_airports: bool,
    pub show_runways: bool,
    pub show_navaids: bool,
    // Trail settings
    pub trails_enabled: bool,
    pub trails_max_age: String,
    pub trails_renderer_2d: TrailRenderer,
    pub trails_renderer_3d: TrailRenderer,
    pub data_ingest: DataIngestConfig,
    pub interpolation_enabled: bool,
    pub error_message: Option<String>,
    pub layout_reset_requested: bool,
}

impl SettingsUiState {
    pub fn populate_from_config(&mut self, config: &AppConfig) {
        self.endpoint_url = config.feed.endpoint_url.clone();
        self.refresh_interval_ms = config.feed.refresh_interval_ms.to_string();
        self.default_latitude = config.map.default_latitude.to_string();
        self.default_longitude = config.map.default_longitude.to_string();
        self.default_zoom = config.map.default_zoom.to_string();
        // Map settings
        self.basemap_style = config.map.basemap_style;
        // Overlay settings
        self.show_airports = config.overlays.show_airports;
        self.show_runways = config.overlays.show_runways;
        self.show_navaids = config.overlays.show_navaids;
        // Trail settings
        self.trails_enabled = config.trails.enabled;
        self.trails_max_age = config.trails.max_age_seconds.to_string();
        self.trails_renderer_2d = config.trails.renderer_2d;
        self.trails_renderer_3d = config.trails.renderer_3d;
        self.data_ingest = config.data_ingest.clone();
        self.interpolation_enabled = config.interpolation_enabled;
        self.error_message = None;
    }

    pub fn validate_and_build(&self) -> Result<AppConfig, String> {
        // Validate endpoint address (host:port format for raw TCP connection)
        let endpoint = self.endpoint_url.trim();
        if endpoint.is_empty() {
            return Err("Endpoint address is required".to_string());
        }
        // Check for host:port format
        let parts: Vec<&str> = endpoint.split(':').collect();
        if parts.len() != 2 {
            return Err("Endpoint must be in host:port format (e.g., 192.168.1.1:30003)".to_string());
        }
        if parts[1].parse::<u16>().is_err() {
            return Err("Port must be a valid number (1-65535)".to_string());
        }

        // Validate refresh interval
        let refresh_ms: u64 = self.refresh_interval_ms.trim().parse()
            .map_err(|_| "Refresh interval must be a number")?;
        if refresh_ms < 100 || refresh_ms > 60000 {
            return Err("Refresh interval must be 100-60000 ms".to_string());
        }

        // Validate latitude
        let lat: f64 = self.default_latitude.trim().parse()
            .map_err(|_| "Latitude must be a number")?;
        if lat < -90.0 || lat > 90.0 {
            return Err("Latitude must be -90 to 90".to_string());
        }

        // Validate longitude
        let lon: f64 = self.default_longitude.trim().parse()
            .map_err(|_| "Longitude must be a number")?;
        if lon < -180.0 || lon > 180.0 {
            return Err("Longitude must be -180 to 180".to_string());
        }

        // Validate zoom
        let zoom: u8 = self.default_zoom.trim().parse()
            .map_err(|_| "Zoom must be a number")?;
        if zoom > 19 {
            return Err("Zoom must be 0-19".to_string());
        }

        // Validate trails max age
        let trails_max_age: u64 = self.trails_max_age.trim().parse()
            .map_err(|_| "Trail max age must be a number")?;
        if trails_max_age < 30 || trails_max_age > 3600 {
            return Err("Trail max age must be 30-3600 seconds".to_string());
        }

        Ok(AppConfig {
            feed: FeedConfig {
                endpoint_url: endpoint.to_string(),
                refresh_interval_ms: refresh_ms,
            },
            map: MapConfig {
                default_latitude: lat,
                default_longitude: lon,
                default_zoom: zoom,
                basemap_style: self.basemap_style,
            },
            overlays: OverlayConfig {
                show_airports: self.show_airports,
                show_runways: self.show_runways,
                show_navaids: self.show_navaids,
            },
            trails: TrailsConfig {
                enabled: self.trails_enabled,
                max_age_seconds: trails_max_age,
                renderer_2d: self.trails_renderer_2d,
                renderer_3d: self.trails_renderer_3d,
            },
            bookmarks: BookmarksConfig::default(),
            appearance: AppearanceConfig::default(),
            data_ingest: self.data_ingest.clone(),
            interpolation_enabled: self.interpolation_enabled,
        })
    }
}

/// Renders the settings pane content into a bare `&mut egui::Ui`.
/// This is used by the docking system (egui_tiles) to render settings
/// content inside a managed pane, without the SidePanel wrapper.
pub fn render_settings_pane_content(
    ui: &mut egui::Ui,
    ui_state: &mut SettingsUiState,
    app_config: &mut AppConfig,
    app_theme: &mut AppTheme,
    theme_registry: &ThemeRegistry,
) {
    ui.heading("Settings");
    ui.separator();

    // Theme section
    ui.collapsing("Theme", |ui| {
        let current_name = app_theme.name().to_string();
        egui::ComboBox::from_id_salt("theme_selector")
            .selected_text(&current_name)
            .show_ui(ui, |ui| {
                for name in theme_registry.names() {
                    let selected = current_name == name;
                    if ui.selectable_label(selected, name).clicked() {
                        if let Some(new_theme) = theme_registry.get(name) {
                            *app_theme = new_theme;
                        }
                    }
                }
            });
    });

    ui.add_space(12.0);

    // Feed section
    ui.collapsing("Feed", |ui| {
        ui.label("Endpoint (host:port):");
        ui.text_edit_singleline(&mut ui_state.endpoint_url);
        ui.add_space(8.0);

        ui.label("Refresh Interval (ms):");
        ui.text_edit_singleline(&mut ui_state.refresh_interval_ms);
    });

    ui.add_space(12.0);

    // Map section
    ui.collapsing("Map Defaults", |ui| {
        ui.label("Basemap Style:");
        egui::ComboBox::from_id_salt("basemap_style")
            .selected_text(ui_state.basemap_style.display_name())
            .show_ui(ui, |ui| {
                for style in BasemapStyle::all() {
                    ui.selectable_value(
                        &mut ui_state.basemap_style,
                        *style,
                        style.display_name(),
                    );
                }
            });
        ui.add_space(8.0);

        ui.label("Default Latitude:");
        ui.text_edit_singleline(&mut ui_state.default_latitude);
        ui.add_space(8.0);

        ui.label("Default Longitude:");
        ui.text_edit_singleline(&mut ui_state.default_longitude);
        ui.add_space(8.0);

        ui.label("Default Zoom (0-19):");
        ui.text_edit_singleline(&mut ui_state.default_zoom);
    });

    ui.add_space(12.0);

    // Overlays section
    ui.collapsing("Overlays", |ui| {
        ui.checkbox(&mut ui_state.show_airports, "Show Airports");
        ui.checkbox(&mut ui_state.show_runways, "Show Runways");
        ui.checkbox(&mut ui_state.show_navaids, "Show Navaids");
    });

    ui.add_space(12.0);

    // Aircraft section
    ui.collapsing("Aircraft", |ui| {
        ui.checkbox(&mut ui_state.interpolation_enabled, "Smooth Interpolation");
        ui.label(
            egui::RichText::new("Predict aircraft positions between ADS-B updates for smooth motion")
                .size(10.0)
                .color(egui::Color32::GRAY),
        );
    });

    ui.add_space(12.0);

    // Trails section
    ui.collapsing("Flight Trails", |ui| {
        ui.checkbox(&mut ui_state.trails_enabled, "Enable Trails");
        ui.add_space(8.0);
        ui.add_enabled_ui(ui_state.trails_enabled, |ui| {
            ui.label("Max Age (seconds):");
            ui.text_edit_singleline(&mut ui_state.trails_max_age);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("2D Renderer:");
                egui::ComboBox::from_id_salt("trail_renderer_2d")
                    .selected_text(ui_state.trails_renderer_2d.to_string())
                    .show_ui(ui, |ui| {
                        for &r in TrailRenderer::ALL {
                            ui.selectable_value(&mut ui_state.trails_renderer_2d, r, r.to_string());
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.label("3D Renderer:");
                egui::ComboBox::from_id_salt("trail_renderer_3d")
                    .selected_text(ui_state.trails_renderer_3d.to_string())
                    .show_ui(ui, |ui| {
                        for &r in TrailRenderer::ALL {
                            ui.selectable_value(&mut ui_state.trails_renderer_3d, r, r.to_string());
                        }
                    });
            });
        });
    });

    ui.add_space(12.0);

    // Layout section
    ui.collapsing("Layout", |ui| {
        if ui.button("Reset Dock Layout").clicked() {
            ui_state.layout_reset_requested = true;
        }
        ui.label(
            egui::RichText::new("Restores all panels to their default positions")
                .size(10.0)
                .color(egui::Color32::GRAY),
        );
    });

    ui.add_space(16.0);

    // Error message
    if let Some(ref error) = ui_state.error_message {
        ui.colored_label(egui::Color32::RED, error);
        ui.add_space(8.0);
    }

    // Buttons
    ui.horizontal(|ui| {
        if ui.button("Cancel").clicked() {
            ui_state.open = false;
            ui_state.error_message = None;
        }

        if ui.button("Save").clicked() {
            match ui_state.validate_and_build() {
                Ok(mut new_config) => {
                    new_config.bookmarks = app_config.bookmarks.clone();
                    new_config.appearance.theme = app_theme.name().to_string();
                    save_config(&new_config);
                    *app_config = new_config;
                    ui_state.open = false;
                    ui_state.error_message = None;
                    info!("Configuration saved");
                }
                Err(e) => {
                    ui_state.error_message = Some(e);
                }
            }
        }
    });
}

pub fn toggle_settings_panel(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut ui_state: ResMut<SettingsUiState>,
    app_config: Res<AppConfig>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        ui_state.open = !ui_state.open;
        if ui_state.open {
            ui_state.populate_from_config(&app_config);
        }
    }
}

/// System to sync config changes to render state resources
pub fn sync_config_to_render_states(
    app_config: Res<AppConfig>,
    mut airport_state: Option<ResMut<crate::aviation::AirportRenderState>>,
    mut runway_state: Option<ResMut<crate::aviation::RunwayRenderState>>,
    mut navaid_state: Option<ResMut<crate::aviation::NavaidRenderState>>,
    mut trail_config: Option<ResMut<crate::aircraft::TrailConfig>>,
) {
    if !app_config.is_changed() {
        return;
    }

    if let Some(ref mut state) = airport_state {
        state.show_airports = app_config.overlays.show_airports;
    }
    if let Some(ref mut state) = runway_state {
        state.show_runways = app_config.overlays.show_runways;
    }
    if let Some(ref mut state) = navaid_state {
        state.show_navaids = app_config.overlays.show_navaids;
    }
    if let Some(ref mut config) = trail_config {
        config.enabled = app_config.trails.enabled;
        config.max_age_seconds = app_config.trails.max_age_seconds;
        config.renderer_2d = app_config.trails.renderer_2d;
        config.renderer_3d = app_config.trails.renderer_3d;
    }
}

/// Resource to track the last applied basemap style for change detection
#[derive(Resource)]
pub struct CurrentBasemapState {
    pub style: BasemapStyle,
}

impl Default for CurrentBasemapState {
    fn default() -> Self {
        Self {
            style: BasemapStyle::default(),
        }
    }
}

/// System to detect basemap changes and update SlippyTilesSettings
pub fn apply_basemap_changes(
    app_config: Res<AppConfig>,
    mut current_state: ResMut<CurrentBasemapState>,
    mut tile_settings: ResMut<SlippyTilesSettings>,
) {
    if !app_config.is_changed() {
        return;
    }

    let new_style = app_config.map.basemap_style;
    if current_state.style != new_style {
        info!("Basemap changed from {:?} to {:?}", current_state.style, new_style);
        current_state.style = new_style;
        tile_settings.endpoint = new_style.endpoint_url().to_string();
        tile_settings.tile_format = new_style.tile_format();
        tile_settings.reverse_axes = new_style.reverse_axes();
        // Note: Tile cache clearing happens in main.rs when cache is cleared
    }
}

pub struct ConfigPlugin;

impl Plugin for ConfigPlugin {
    fn build(&self, app: &mut App) {
        let config = load_config();
        let initial_basemap = CurrentBasemapState {
            style: config.map.basemap_style,
        };

        let registry = ThemeRegistry::new();
        let initial_theme = registry
            .get(&config.appearance.theme)
            .unwrap_or_else(|| registry.get("Nord Dark").unwrap());

        app.add_plugins(EguiPlugin::default())
            .insert_resource(config)
            .insert_resource(initial_basemap)
            .insert_resource(registry)
            .insert_resource(initial_theme)
            .init_resource::<SettingsUiState>()
            .add_systems(Update, (toggle_settings_panel, sync_config_to_render_states, apply_basemap_changes));
    }
}
