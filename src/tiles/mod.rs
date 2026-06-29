//! Integrated tile system for AirJedi.
//!
//! This module replaces the external `bevy_slippy_tiles` crate with an
//! integrated download + rendering pipeline optimized for aviation use.
//!
//! ## Module structure
//! - `coords` - Slippy map coordinate math (Mercator projection)
//! - `types` - Tile types (ZoomLevel, TileSize, TileFormat, TileKey)
//! - `download` - Async tile fetcher with rate limiting and caching
//! - `render` - Tile display, fade-in, 3D mesh quads (current rendering code)

pub mod coords;
pub mod download;
pub mod elevation;
pub mod pool;
pub mod prefetch;
pub mod render;
pub mod types;

// Re-export everything the rest of the codebase needs.
// These maintain API compatibility with the old bevy_slippy_tiles imports.

pub use coords::{
    lat_lon_to_world_pixel, world_pixel_to_lat_lon, LatLon, SlippyTileCoordinates,
    Coordinates, wrap_tile_x,
};
pub use download::{DownloadTilesRequest, TileDownloadSettings, TileReady};
pub use types::*;

// Re-export rendering types so `use crate::tiles::*` works as before
pub use render::{
    TileFadeState, TileMeshQuad, TileQuad3d, TileQuadMesh, SpawnedTiles,
    Tile3DRefreshTimer, TileOriginalImage, GridOverlay,
    altitude_to_zoom_level, compute_tile_radius, request_tiles_at_location,
};

// sync_tile_mesh_transforms was removed in the unified mesh redesign.
// Provide a no-op stub for any external references.
pub fn sync_tile_mesh_transforms() {}

use bevy::prelude::*;

// ---------------------------------------------------------------------------
// Backward-compatible type aliases for bevy_slippy_tiles API
// ---------------------------------------------------------------------------

pub type LatitudeLongitudeCoordinates = LatLon;

/// Compatibility shim: wraps new download types to match old bevy_slippy_tiles API.
/// This resource provides reference coordinates for tile positioning, matching
/// the old SlippyTilesSettings fields used throughout the codebase.
#[derive(Clone, Resource)]
pub struct SlippyTilesSettings {
    pub endpoint: String,
    pub tiles_directory: std::path::PathBuf,
    pub max_concurrent_downloads: usize,
    pub max_retries: u32,
    pub tile_format: TileFormat,
    pub tile_size: TileSize,
    pub rate_limit_requests: usize,
    pub rate_limit_window: std::time::Duration,
    pub reverse_axes: bool,
    // Display settings (used by camera, geo, tiles rendering)
    pub reference_latitude: f64,
    pub reference_longitude: f64,
    pub transform_offset: Option<Transform>,
    pub z_layer: f32,
    pub auto_render: bool,
}

impl Default for SlippyTilesSettings {
    fn default() -> Self {
        Self {
            endpoint: "https://tile.openstreetmap.org".into(),
            tiles_directory: std::path::PathBuf::from("tiles/"),
            max_concurrent_downloads: 8,
            max_retries: 3,
            tile_format: TileFormat::default(),
            tile_size: TileSize::Large,
            rate_limit_requests: 20,
            rate_limit_window: std::time::Duration::from_secs(1),
            reverse_axes: false,
            reference_latitude: 0.0,
            reference_longitude: 0.0,
            transform_offset: None,
            z_layer: 0.0,
            auto_render: false,
        }
    }
}

impl SlippyTilesSettings {
    pub fn get_tiles_directory_string(&self) -> String {
        self.tiles_directory.as_path().to_str().unwrap_or("tiles/").to_string()
    }
}

/// Compatibility: MapTile marker component (was in bevy_slippy_tiles)
#[derive(Component)]
pub struct MapTile;

/// Compatibility: DownloadSlippyTilesMessage (maps to new DownloadTilesRequest)
#[derive(bevy::ecs::message::Message)]
pub struct DownloadSlippyTilesMessage {
    pub tile_size: TileSize,
    pub zoom_level: ZoomLevel,
    pub coordinates: Coordinates,
    pub radius: Radius,
    pub use_cache: bool,
}

/// Compatibility: SlippyTileDownloadedMessage (maps to new TileReady)
#[derive(bevy::ecs::message::Message, Clone)]
pub struct SlippyTileDownloadedMessage {
    pub zoom_level: ZoomLevel,
    pub tile_size: TileSize,
    pub coordinates: Coordinates,
    pub path: std::path::PathBuf,
}

/// Compatibility: Download status tracking
#[derive(Resource, Default)]
pub struct SlippyTileDownloadStatus(
    pub bevy_platform::collections::HashMap<SlippyTileDownloadTaskKey, TileDownloadStatus>,
);

impl SlippyTileDownloadStatus {
    pub fn new() -> Self {
        Self(bevy_platform::collections::HashMap::new())
    }

    pub fn contains_key(&self, x: u32, y: u32, zoom_level: ZoomLevel, tile_size: TileSize, tile_format: &TileFormat) -> bool {
        self.0.contains_key(&SlippyTileDownloadTaskKey {
            slippy_tile_coordinates: SlippyTileCoordinates { x, y },
            zoom_level,
            tile_size,
            tile_format: *tile_format,
        })
    }

    pub fn contains_key_with_coords(
        &self,
        coords: SlippyTileCoordinates,
        zoom_level: ZoomLevel,
        tile_size: TileSize,
    ) -> bool {
        self.0.contains_key(&SlippyTileDownloadTaskKey {
            slippy_tile_coordinates: coords,
            zoom_level,
            tile_size,
            tile_format: TileFormat::default(),
        })
    }

    pub fn insert_with_coords(
        &mut self,
        coords: SlippyTileCoordinates,
        zoom_level: ZoomLevel,
        tile_size: TileSize,
        tile_format: TileFormat,
        filename: String,
        download_status: DownloadStatus,
    ) {
        self.0.insert(
            SlippyTileDownloadTaskKey {
                slippy_tile_coordinates: coords,
                zoom_level,
                tile_size,
                tile_format,
            },
            TileDownloadStatus {
                path: std::path::PathBuf::from(filename),
                load_status: download_status,
            },
        );
    }
}

#[derive(Eq, PartialEq, Hash, Clone)]
pub struct SlippyTileDownloadTaskKey {
    pub slippy_tile_coordinates: SlippyTileCoordinates,
    pub zoom_level: ZoomLevel,
    pub tile_size: TileSize,
    pub tile_format: TileFormat,
}

pub struct TileDownloadStatus {
    pub path: std::path::PathBuf,
    pub load_status: DownloadStatus,
}

pub fn world_coords_to_world_pixel(
    coords: &LatLon,
    tile_size: TileSize,
    zoom_level: ZoomLevel,
) -> (f64, f64) {
    coords::lat_lon_to_world_pixel(coords, tile_size.to_pixels(), zoom_level)
}

pub fn world_pixel_to_world_coords(
    x_pixel: f64,
    y_pixel: f64,
    tile_size: TileSize,
    zoom_level: ZoomLevel,
) -> LatLon {
    coords::world_pixel_to_lat_lon(x_pixel, y_pixel, tile_size.to_pixels(), zoom_level)
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub const TILE_EMISSIVE_BOOST: f32 = 8.0;
pub const TILE_FADE_SPEED: f32 = 3.0;
pub const DEFAULT_TILE_PIXELS: f32 = 512.0;

pub struct TilesPlugin;

impl Plugin for TilesPlugin {
    fn build(&self, app: &mut App) {
        // Register compatibility message types
        app.add_message::<DownloadSlippyTilesMessage>()
            .add_message::<SlippyTileDownloadedMessage>()
            .init_resource::<SlippyTileDownloadStatus>();

        app.init_resource::<TileDownloadSettings>();
        download::setup_download_systems(app);
        pool::setup_pool_systems(app);
        elevation::setup_elevation_systems(app);
        prefetch::setup_prefetch_systems(app);
        render::setup_render_systems(app);

        app.add_systems(Startup, sync_download_settings_from_slippy)
            .add_systems(Update, sync_download_settings_on_change)
            .add_systems(Update, (bridge_download_requests, bridge_tile_ready));
    }
}

/// Copy settings from SlippyTilesSettings (configured in main.rs) to TileDownloadSettings
/// (used by the new download system). Run at startup.
fn sync_download_settings_from_slippy(
    slippy: Res<SlippyTilesSettings>,
    basemap: Res<crate::config::CurrentBasemapState>,
    mut dl: ResMut<TileDownloadSettings>,
) {
    dl.endpoint = slippy.endpoint.clone();
    dl.tiles_directory = slippy.tiles_directory.clone();
    dl.max_concurrent_downloads = slippy.max_concurrent_downloads;
    dl.max_retries = slippy.max_retries;
    dl.tile_format = slippy.tile_format;
    dl.tile_size = slippy.tile_size;
    dl.rate_limit_requests = slippy.rate_limit_requests;
    dl.rate_limit_window = slippy.rate_limit_window;
    dl.reverse_axes = slippy.reverse_axes;
    dl.supports_retina = basemap.style.supports_retina();
    dl.uses_extension_in_url = basemap.style.uses_extension_in_url();
    dl.cache_key = basemap.style.cache_key().to_string();
    crate::tile_cache::setup_tile_cache_for_style(&dl.cache_key);
}

fn sync_download_settings_on_change(
    slippy: Res<SlippyTilesSettings>,
    basemap: Res<crate::config::CurrentBasemapState>,
    mut dl: ResMut<TileDownloadSettings>,
) {
    if !slippy.is_changed() && !basemap.is_changed() {
        return;
    }
    dl.endpoint = slippy.endpoint.clone();
    dl.tiles_directory = slippy.tiles_directory.clone();
    dl.tile_format = slippy.tile_format;
    dl.tile_size = slippy.tile_size;
    dl.reverse_axes = slippy.reverse_axes;
    dl.supports_retina = basemap.style.supports_retina();
    dl.uses_extension_in_url = basemap.style.uses_extension_in_url();
    dl.cache_key = basemap.style.cache_key().to_string();
    crate::tile_cache::setup_tile_cache_for_style(&dl.cache_key);
}

/// Bridge: convert old DownloadSlippyTilesMessage into new DownloadTilesRequest.
/// This lets existing systems (input, zoom, 3D refresh) work unchanged.
fn bridge_download_requests(
    mut old_events: MessageReader<DownloadSlippyTilesMessage>,
    mut new_events: MessageWriter<download::DownloadTilesRequest>,
) {
    for event in old_events.read() {
        let coords = event.coordinates.to_lat_lon(event.zoom_level);
        new_events.write(download::DownloadTilesRequest {
            latitude: coords.latitude,
            longitude: coords.longitude,
            zoom: event.zoom_level.to_u8(),
            radius: Radius(event.radius.0),
            priority: DownloadPriority::Near,
            use_cache: event.use_cache,
        });
    }
}

/// Bridge: convert new TileReady into old SlippyTileDownloadedMessage.
/// This lets the existing display_tiles_filtered system work unchanged.
fn bridge_tile_ready(
    mut new_events: MessageReader<download::TileReady>,
    mut old_events: MessageWriter<SlippyTileDownloadedMessage>,
    settings: Res<SlippyTilesSettings>,
) {
    for event in new_events.read() {
        let zoom = match ZoomLevel::try_from(event.key.zoom) {
            Ok(z) => z,
            Err(_) => continue,
        };
        old_events.write(SlippyTileDownloadedMessage {
            zoom_level: zoom,
            tile_size: event.key.tile_size,
            coordinates: Coordinates::from_slippy_tile(event.key.x, event.key.y),
            path: event.path.clone(),
        });
    }
}
