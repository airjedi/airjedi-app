use bevy::prelude::*;
use bevy_egui::EguiContexts;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::aviation::{AirportRenderState, AviationData};
use crate::geo::CoordinateConverter;
use crate::{MapState, ZoomState};
use bevy_slippy_tiles::*;

/// Flight category based on visibility and ceiling
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlightCategory {
    /// VFR: visibility > 5 sm, ceiling > 3000 ft AGL
    #[default]
    Vfr,
    /// MVFR: visibility 3-5 sm or ceiling 1000-3000 ft
    Mvfr,
    /// IFR: visibility 1-3 sm or ceiling 500-1000 ft
    Ifr,
    /// LIFR: visibility < 1 sm or ceiling < 500 ft
    Lifr,
}

impl FlightCategory {
    /// Get color for this flight category
    pub fn color(&self) -> Color {
        match self {
            FlightCategory::Vfr => Color::srgb(0.0, 0.8, 0.0), // Green
            FlightCategory::Mvfr => Color::srgb(0.0, 0.5, 1.0), // Blue
            FlightCategory::Ifr => Color::srgb(1.0, 0.0, 0.0), // Red
            FlightCategory::Lifr => Color::srgb(1.0, 0.0, 1.0), // Magenta
        }
    }

    /// Get display name
    pub fn name(&self) -> &'static str {
        match self {
            FlightCategory::Vfr => "VFR",
            FlightCategory::Mvfr => "MVFR",
            FlightCategory::Ifr => "IFR",
            FlightCategory::Lifr => "LIFR",
        }
    }
}

/// METAR data for an airport
#[derive(Debug, Clone)]
pub struct MetarData {
    /// ICAO airport identifier
    pub icao: String,
    /// Raw METAR text
    pub raw_text: String,
    /// Flight category
    pub flight_category: FlightCategory,
    /// Visibility in statute miles (None if not available)
    pub visibility_sm: Option<f32>,
    /// Ceiling in feet AGL (None if no ceiling/sky clear)
    pub ceiling_ft: Option<i32>,
    /// Wind direction in degrees
    pub wind_direction: Option<i32>,
    /// Wind speed in knots
    pub wind_speed: Option<i32>,
    /// Wind gust in knots
    pub wind_gust: Option<i32>,
    /// Temperature in Celsius
    pub temperature_c: Option<i32>,
    /// Dewpoint in Celsius
    pub dewpoint_c: Option<i32>,
    /// Altimeter setting in inHg
    pub altimeter_inhg: Option<f32>,
    /// Observation time
    pub observation_time: String,
    /// When this data was fetched
    pub fetched_at: Instant,
}

/// JSON response from NOAA Aviation Weather API
#[derive(Debug, Deserialize)]
struct MetarApiResponse {
    #[serde(default)]
    pub data: Vec<MetarJson>,
}

#[derive(Debug, Deserialize)]
struct MetarJson {
    #[serde(rename = "icaoId")]
    pub icao_id: Option<String>,
    #[serde(rename = "rawOb")]
    pub raw_ob: Option<String>,
    #[serde(rename = "obsTime")]
    pub obs_time: Option<String>,
    pub temp: Option<f32>,
    pub dewp: Option<f32>,
    pub wdir: Option<String>,
    pub wspd: Option<i32>,
    pub wgst: Option<i32>,
    pub visib: Option<String>,
    pub altim: Option<f32>,
    #[serde(rename = "fltcat")]
    pub flt_cat: Option<String>,
    pub clouds: Option<Vec<CloudLayer>>,
}

#[derive(Debug, Deserialize)]
struct CloudLayer {
    pub cover: Option<String>,
    pub base: Option<i32>,
}

impl MetarJson {
    fn to_metar_data(&self) -> Option<MetarData> {
        let icao = self.icao_id.clone()?;

        // Parse flight category
        let flight_category = match self.flt_cat.as_deref() {
            Some("VFR") => FlightCategory::Vfr,
            Some("MVFR") => FlightCategory::Mvfr,
            Some("IFR") => FlightCategory::Ifr,
            Some("LIFR") => FlightCategory::Lifr,
            _ => FlightCategory::Vfr, // Default to VFR if unknown
        };

        // Parse visibility
        let visibility_sm = self.visib.as_ref().and_then(|v| {
            // Handle special cases like "10+" or fractions
            if v.contains('+') {
                Some(10.0)
            } else if v.contains('/') {
                // Fraction like "1/2" or "1 1/2"
                let parts: Vec<&str> = v.split_whitespace().collect();
                if parts.len() == 2 {
                    // "1 1/2" format
                    let whole: f32 = parts[0].parse().ok()?;
                    let frac_parts: Vec<&str> = parts[1].split('/').collect();
                    if frac_parts.len() == 2 {
                        let num: f32 = frac_parts[0].parse().ok()?;
                        let den: f32 = frac_parts[1].parse().ok()?;
                        Some(whole + num / den)
                    } else {
                        Some(whole)
                    }
                } else if parts.len() == 1 && v.contains('/') {
                    // "1/2" format
                    let frac_parts: Vec<&str> = v.split('/').collect();
                    if frac_parts.len() == 2 {
                        let num: f32 = frac_parts[0].parse().ok()?;
                        let den: f32 = frac_parts[1].parse().ok()?;
                        Some(num / den)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                v.parse().ok()
            }
        });

        // Find ceiling (lowest BKN or OVC layer)
        let ceiling_ft = self.clouds.as_ref().and_then(|clouds| {
            clouds
                .iter()
                .filter(|c| matches!(c.cover.as_deref(), Some("BKN") | Some("OVC")))
                .filter_map(|c| c.base)
                .min()
        });

        // Parse wind direction
        let wind_direction = self.wdir.as_ref().and_then(|d| {
            if d == "VRB" {
                None // Variable
            } else {
                d.parse().ok()
            }
        });

        Some(MetarData {
            icao,
            raw_text: self.raw_ob.clone().unwrap_or_default(),
            flight_category,
            visibility_sm,
            ceiling_ft,
            wind_direction,
            wind_speed: self.wspd,
            wind_gust: self.wgst,
            temperature_c: self.temp.map(|t| t as i32),
            dewpoint_c: self.dewp.map(|d| d as i32),
            altimeter_inhg: self.altim,
            observation_time: self.obs_time.clone().unwrap_or_default(),
            fetched_at: Instant::now(),
        })
    }
}

/// Cache for METAR data
#[derive(Resource, Default)]
pub struct MetarCache {
    /// Cached METAR data keyed by ICAO
    pub data: HashMap<String, MetarData>,
    /// ICAOs that have been requested but not yet fetched
    pub pending: HashMap<String, Instant>,
    /// Last time we checked for updates
    pub last_update_check: Option<Instant>,
}

impl MetarCache {
    /// Cache duration in seconds (12 minutes)
    const CACHE_DURATION_SECS: u64 = 720;
    /// Minimum interval between API calls in seconds
    const MIN_FETCH_INTERVAL_SECS: u64 = 5;
    /// How long to wait before retrying a failed fetch
    const RETRY_DELAY_SECS: u64 = 60;

    /// Check if we should fetch METAR for an ICAO
    pub fn should_fetch(&self, icao: &str) -> bool {
        // Check if already pending
        if let Some(pending_time) = self.pending.get(icao) {
            if pending_time.elapsed() < Duration::from_secs(Self::RETRY_DELAY_SECS) {
                return false;
            }
        }

        // Check if cached data is still fresh
        if let Some(metar) = self.data.get(icao) {
            if metar.fetched_at.elapsed() < Duration::from_secs(Self::CACHE_DURATION_SECS) {
                return false;
            }
        }

        true
    }

    /// Check if enough time has passed since last fetch
    pub fn can_fetch(&self) -> bool {
        match self.last_update_check {
            Some(last) => last.elapsed() >= Duration::from_secs(Self::MIN_FETCH_INTERVAL_SECS),
            None => true,
        }
    }

    /// Mark ICAOs as pending
    pub fn mark_pending(&mut self, icaos: &[String]) {
        let now = Instant::now();
        for icao in icaos {
            self.pending.insert(icao.clone(), now);
        }
        self.last_update_check = Some(now);
    }

    /// Store fetched METAR data
    pub fn store(&mut self, metar: MetarData) {
        self.pending.remove(&metar.icao);
        self.data.insert(metar.icao.clone(), metar);
    }

    /// Get METAR data for an ICAO
    pub fn get(&self, icao: &str) -> Option<&MetarData> {
        self.data.get(icao)
    }
}

/// Weather overlay state
#[derive(Resource)]
pub struct WeatherState {
    /// Whether weather overlay is enabled
    pub enabled: bool,
    /// Minimum zoom level to show weather indicators
    pub min_zoom_level: u8,
}

impl Default for WeatherState {
    fn default() -> Self {
        Self {
            enabled: true,
            min_zoom_level: 6,
        }
    }
}

/// Component for weather indicator entities
#[derive(Component)]
pub struct WeatherIndicator {
    pub icao: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Shared state for fetched METAR data between threads
#[derive(Resource, Default)]
pub struct MetarFetchResults {
    pub results: std::sync::Arc<std::sync::Mutex<Vec<MetarData>>>,
}

/// Fetch METAR data for visible airports
pub fn fetch_metar_for_visible_airports(
    weather_state: Res<WeatherState>,
    airport_state: Option<Res<AirportRenderState>>,
    aviation_data: Option<Res<AviationData>>,
    map_state: Res<MapState>,
    zoom_state: Res<ZoomState>,
    mut metar_cache: ResMut<MetarCache>,
    fetch_results: Res<MetarFetchResults>,
) {
    // First, check for any completed fetches
    if let Ok(mut results) = fetch_results.results.try_lock() {
        for metar in results.drain(..) {
            metar_cache.store(metar);
        }
    }

    if !weather_state.enabled {
        return;
    }

    // Check if we can fetch
    if !metar_cache.can_fetch() {
        return;
    }

    // Get visible airports (weather is independent of airport display)
    let Some(aviation_data) = aviation_data else {
        return;
    };
    let Some(_airport_state) = airport_state else {
        return;
    };

    // Only fetch if zoomed in enough
    let zoom_level: u8 = map_state.zoom_level.to_u8();
    if zoom_level < weather_state.min_zoom_level {
        return;
    }

    // Calculate visible bounds (approximate)
    let lat_range = 2.0 / zoom_state.camera_zoom as f64;
    let lon_range = 4.0 / zoom_state.camera_zoom as f64;

    let min_lat = map_state.latitude - lat_range;
    let max_lat = map_state.latitude + lat_range;
    let min_lon = map_state.longitude - lon_range;
    let max_lon = map_state.longitude + lon_range;

    // Find airports that need METAR fetch (major airports with scheduled service)
    let icaos_to_fetch: Vec<String> = aviation_data
        .airports
        .iter()
        .filter(|a| {
            a.latitude_deg >= min_lat
                && a.latitude_deg <= max_lat
                && a.longitude_deg >= min_lon
                && a.longitude_deg <= max_lon
        })
        .filter(|a| a.has_scheduled_service() || a.is_major())
        .filter(|a| {
            // Only airports with ICAO codes
            a.ident.len() == 4 && a.ident.chars().all(|c| c.is_ascii_alphabetic())
        })
        .filter(|a| metar_cache.should_fetch(&a.ident))
        .take(10) // Limit batch size
        .map(|a| a.ident.clone())
        .collect();

    if icaos_to_fetch.is_empty() {
        return;
    }

    // Mark as pending
    metar_cache.mark_pending(&icaos_to_fetch);

    // Spawn blocking task to fetch METAR
    let icaos = icaos_to_fetch.join(",");
    let results_arc = std::sync::Arc::clone(&fetch_results.results);
    std::thread::spawn(move || {
        let fetched = fetch_metar_batch(&icaos);
        if let Ok(mut results) = results_arc.lock() {
            results.extend(fetched);
        }
    });
}

/// Fetch METAR for a batch of airports (blocking)
fn fetch_metar_batch(icaos: &str) -> Vec<MetarData> {
    let url = format!(
        "https://aviationweather.gov/api/data/metar?ids={}&format=json",
        icaos
    );

    let response = match reqwest::blocking::get(&url) {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to fetch METAR: {}", e);
            return Vec::new();
        }
    };

    let json: Vec<MetarJson> = match response.json() {
        Ok(j) => j,
        Err(e) => {
            warn!("Failed to parse METAR response: {}", e);
            return Vec::new();
        }
    };

    json.iter().filter_map(|m| m.to_metar_data()).collect()
}

/// Render weather indicators for airports with METAR data
pub fn render_weather_indicators(
    mut commands: Commands,
    weather_state: Res<WeatherState>,
    metar_cache: Res<MetarCache>,
    aviation_data: Option<Res<AviationData>>,
    existing_indicators: Query<(Entity, &WeatherIndicator)>,
    map_state: Res<MapState>,
    zoom_state: Res<ZoomState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if !weather_state.enabled {
        // Remove all indicators when disabled
        for (entity, _) in existing_indicators.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }

    let Some(aviation_data) = aviation_data else {
        return;
    };

    // Only show if zoomed in enough
    let zoom_level: u8 = map_state.zoom_level.to_u8();
    if zoom_level < weather_state.min_zoom_level {
        for (entity, _) in existing_indicators.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }

    // Calculate visible bounds
    let lat_range = 2.0 / zoom_state.camera_zoom as f64;
    let lon_range = 4.0 / zoom_state.camera_zoom as f64;
    let min_lat = map_state.latitude - lat_range;
    let max_lat = map_state.latitude + lat_range;
    let min_lon = map_state.longitude - lon_range;
    let max_lon = map_state.longitude + lon_range;

    // Track which indicators already exist
    let existing_icaos: HashMap<String, Entity> = existing_indicators
        .iter()
        .map(|(e, w)| (w.icao.clone(), e))
        .collect();

    // Create indicators for visible airports with METAR data
    for airport in aviation_data.airports.iter() {
        // Check if in view
        if airport.latitude_deg < min_lat
            || airport.latitude_deg > max_lat
            || airport.longitude_deg < min_lon
            || airport.longitude_deg > max_lon
        {
            // Remove if out of view
            if let Some(entity) = existing_icaos.get(&airport.ident) {
                commands.entity(*entity).despawn();
            }
            continue;
        }

        // Check if we have METAR data
        let Some(metar) = metar_cache.get(&airport.ident) else {
            continue;
        };

        // Skip if indicator already exists
        if existing_icaos.contains_key(&airport.ident) {
            continue;
        }

        // Create weather indicator
        let indicator_radius = 6.0;
        let color = metar.flight_category.color();

        commands.spawn((
            Mesh2d(meshes.add(Circle::new(indicator_radius))),
            MeshMaterial2d(materials.add(ColorMaterial::from(color))),
            Transform::from_xyz(0.0, 0.0, 9.0), // Just below aircraft layer
            WeatherIndicator {
                icao: airport.ident.clone(),
                latitude: airport.latitude_deg,
                longitude: airport.longitude_deg,
            },
        ));
    }
}

/// Update weather indicator positions based on map state.
/// Uses the same reference-point-based coordinate conversion as all other
/// rendering systems (airports, navaids, aircraft, etc.).
pub fn update_weather_indicator_positions(
    mut indicators: Query<(&WeatherIndicator, &mut Transform)>,
    map_state: Res<MapState>,
    tile_settings: Res<SlippyTilesSettings>,
    zoom_state: Res<ZoomState>,
    view3d_state: Res<crate::view3d::View3DState>,
) {
    let zoom = view3d_state.effective_zoom(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, zoom);

    for (indicator, mut transform) in indicators.iter_mut() {
        let pos = converter.latlon_to_world(indicator.latitude, indicator.longitude);

        transform.translation.x = pos.x;
        transform.translation.y = pos.y;

        // Scale based on zoom
        let scale = (zoom_state.camera_zoom * 0.5).clamp(0.5, 2.0);
        transform.scale = Vec3::splat(scale);
    }
}

/// Toggle weather overlay with 'W' key
pub fn toggle_weather_overlay(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut weather_state: ResMut<WeatherState>,
    mut contexts: EguiContexts,
) {
    // Don't toggle if egui wants input
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    if keyboard.just_pressed(KeyCode::KeyW) {
        weather_state.enabled = !weather_state.enabled;
        info!(
            "Weather overlay: {}",
            if weather_state.enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
    }
}
