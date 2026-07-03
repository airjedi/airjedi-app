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

pub use coords::{
    lat_lon_to_world_pixel, world_pixel_to_lat_lon, LatLon, SlippyTileCoordinates,
    Coordinates, wrap_tile_x,
    // Mercator meter coordinate system
    lonlat_to_mercator, mercator_to_lonlat, tile_to_mercator_aabb,
    MercatorAabb, LocalOrigin, LocalOriginConversion, LocalOriginShifted,
    WEB_MERCATOR_EXTENT,
};
pub use download::{DownloadTilesRequest, TileDownloadSettings, TileReady};
pub use types::*;

// Re-export rendering types so `use crate::tiles::*` works as before
pub use render::{
    TileFadeState, TileMeshQuad, TileQuadMesh,
    Tile3DRefreshTimer, TileOriginalImage, GridOverlay,
    altitude_to_zoom_level, compute_tile_radius, request_tiles_at_location,
};

/// Deprecated: was part of the old dual-entity tile design. Kept as a no-op
/// stub because terrain/mod.rs orders systems `.after()` this function.
pub fn sync_tile_mesh_transforms() {}

use bevy::prelude::*;

// ---------------------------------------------------------------------------
// Core type aliases
// ---------------------------------------------------------------------------

pub type LatitudeLongitudeCoordinates = LatLon;

// ---------------------------------------------------------------------------
// Tile Render Settings
// ---------------------------------------------------------------------------

/// Settings for tile rendering and coordinate conversion. Used by camera,
/// aircraft, aviation, terrain, and other positioning systems to convert
/// between geographic and world coordinates.
///
/// This is separate from `TileDownloadSettings` which holds download-specific
/// config (endpoint, rate limits, format, cache key).
#[derive(Clone, Resource)]
pub struct TileRenderSettings {
    // Display settings (used by camera, geo, tiles rendering)
    pub reference_latitude: f64,
    pub reference_longitude: f64,
    pub transform_offset: Option<Transform>,
    pub z_layer: f32,
}

impl Default for TileRenderSettings {
    fn default() -> Self {
        Self {
            reference_latitude: 0.0,
            reference_longitude: 0.0,
            transform_offset: None,
            z_layer: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Map Tile marker
// ---------------------------------------------------------------------------

/// Marker component for tile entities.
#[derive(Component)]
pub struct MapTile;

// ---------------------------------------------------------------------------
// Coordinate helpers
// ---------------------------------------------------------------------------

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
pub const DEFAULT_TILE_PIXELS: f32 = 512.0;

pub struct TilesPlugin;

impl Plugin for TilesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TileDownloadSettings>();
        app.add_message::<LocalOriginShifted>();
        download::setup_download_systems(app);
        pool::setup_pool_systems(app);
        elevation::setup_elevation_systems(app);
        prefetch::setup_prefetch_systems(app);
        render::setup_render_systems(app);

        app.add_systems(Update, sync_download_settings_on_basemap_change);
        app.add_systems(Update, recenter_local_origin);
        app.add_systems(Update, apply_origin_shift.after(recenter_local_origin));
    }
}

/// When the basemap style changes, update TileDownloadSettings to match.
fn sync_download_settings_on_basemap_change(
    basemap: Res<crate::config::CurrentBasemapState>,
    mut dl: ResMut<TileDownloadSettings>,
) {
    if !basemap.is_changed() {
        return;
    }
    dl.supports_retina = basemap.style.supports_retina();
    dl.uses_extension_in_url = basemap.style.uses_extension_in_url();
    dl.cache_key = basemap.style.cache_key().to_string();
    crate::tile_cache::setup_tile_cache_for_style(&dl.cache_key);
}

// ---------------------------------------------------------------------------
// LocalOrigin recentering
// ---------------------------------------------------------------------------

fn recenter_local_origin(
    mut local_origin: ResMut<LocalOrigin>,
    camera_query: Query<&Transform, With<crate::camera::MapCamera>>,
    mut shift_events: MessageWriter<LocalOriginShifted>,
    view3d_state: Res<crate::view3d::View3DState>,
) {
    let cam_pos = if view3d_state.is_3d_active() {
        Vec2::new(
            view3d_state.saved_2d_center.x,
            view3d_state.saved_2d_center.y,
        )
    } else if let Ok(tf) = camera_query.single() {
        Vec2::new(tf.translation.x, tf.translation.y)
    } else {
        return;
    };

    let dist = cam_pos.length() as f64;
    if dist < local_origin.recenter_distance() {
        return;
    }

    let shift_merc = bevy::math::DVec3::new(cam_pos.x as f64, cam_pos.y as f64, 0.0);
    local_origin.shift_mercator_origin(shift_merc);

    let delta = Vec3::new(cam_pos.x, cam_pos.y, 0.0);
    shift_events.write(LocalOriginShifted { delta });
    info!(
        "Recentered LocalOrigin - shifted by ({:.0}, {:.0})m",
        delta.x, delta.y
    );
}

fn apply_origin_shift(
    mut shift_events: MessageReader<LocalOriginShifted>,
    mut tile_grid: ResMut<pool::TileGrid>,
    tile_query: Query<Entity, With<MapTile>>,
    mut commands: Commands,
    mut view3d_state: ResMut<crate::view3d::View3DState>,
    local_origin: Res<LocalOrigin>,
    map_state: Res<crate::map::MapState>,
) {
    let Some(_event) = shift_events.read().last() else {
        return;
    };

    // Despawn all tiles and clear the grid. Tiles will be respawned by
    // load_visible_tiles using the new origin on the next frame.
    for entity in tile_query.iter() {
        commands.entity(entity).despawn();
    }
    tile_grid.occupied.clear();

    // Recompute saved_2d_center from map_state lat/lon using the new origin.
    // In 3D mode this is the camera orbit center, accumulated via mouse drags
    // rather than recomputed every frame, so it needs explicit correction.
    let converter = crate::geo::CoordinateConverter::new(&local_origin);
    let new_center = converter.latlon_to_world(map_state.latitude, map_state.longitude);
    view3d_state.saved_2d_center = new_center;

    // Camera, aircraft, airports, navaids, airspace, etc. all recompute
    // their positions from lat/lon every frame using CoordinateConverter,
    // which reads the already-updated LocalOrigin. No manual transform
    // shifting needed.
}
