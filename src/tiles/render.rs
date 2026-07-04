use bevy::asset::AssetLoadFailedEvent;
use bevy::image::Image;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;

use super::*;

use crate::camera::MapCamera;
use crate::constants;
use crate::map::{MapState, ZoomState};
use crate::tile_cache;
use crate::view3d;
use crate::{clamp_latitude, clamp_longitude, ZoomSet};

// =============================================================================
// Components and Resources
// =============================================================================

/// Component to track tile fade state for smooth zoom transitions.
#[derive(Component)]
pub struct TileFadeState {
    pub alpha: f32,
    /// The zoom level this tile was spawned for
    pub tile_zoom: u8,
    /// When this tile was spawned (seconds since startup)
    pub spawn_time: f64,
}

/// Deprecated: was used to link tile entities to companion Mesh3d quads in the
/// old dual-entity design. Tiles are now unified Mesh3d entities. Kept as a
/// stub for terrain module compilation - no tiles have this component.
#[derive(Component)]
pub struct TileMeshQuad(pub Entity);

/// Per-zoom-level mesh handles for both 2D and 3D rendering.
/// 2D uses Rectangle (XY plane, faces -Z, no rotation needed).
/// 3D uses Plane3d (XZ plane, faces +Y, no rotation needed).
#[derive(Resource)]
pub struct TileQuadMesh {
    pub meshes_2d: std::collections::HashMap<u8, Handle<Mesh>>,
    pub meshes_3d: std::collections::HashMap<u8, Handle<Mesh>>,
}

/// Timer that triggers periodic tile re-requests in 3D mode so that camera
/// orbit, pan, and altitude changes continuously fill visible areas.
#[derive(Resource)]
pub struct Tile3DRefreshTimer(Timer);

impl Default for Tile3DRefreshTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.3, TimerMode::Repeating))
    }
}


/// Tracks the previous camera altitude to detect active altitude changes.
/// During rapid altitude changes, tile culling is softened to prevent
/// flashing while new tiles load.
#[derive(Resource)]
struct AltitudeChangeTracker {
    prev_altitude: f32,
    /// Seconds since the last significant altitude change.
    idle_secs: f32,
    /// Cooldown: seconds remaining before a zoom level change is allowed.
    zoom_cooldown: f32,
}

impl Default for AltitudeChangeTracker {
    fn default() -> Self {
        Self {
            prev_altitude: 10000.0,
            idle_secs: f32::MAX,
            zoom_cooldown: 0.0,
        }
    }
}

/// Stores the original tile image handle so the grid overlay can be toggled off.
#[derive(Component)]
pub struct TileOriginalImage(pub Handle<Image>);

/// Retains strong handles to recently-loaded tile images so they survive entity
/// despawns. Without this, Bevy drops the GPU texture when the last entity
/// referencing it is despawned, forcing a disk reload (and gray flash) when the
/// same tile is re-requested.
///
/// Each entry tracks its last-access time for LRU eviction. A periodic
/// eviction system removes entries not accessed in 30s and not currently
/// displayed, preventing unbounded VRAM growth during panning.
#[derive(Resource, Default)]
struct TileAssetCache {
    entries: std::collections::HashMap<String, (Handle<Image>, f64)>,
}

const TILE_ASSET_EVICTION_SECS: f64 = 30.0;
const TILE_ASSET_CACHE_HARD_CAP: usize = 2000;
const TILE_FADE_DURATION_SECS: f64 = 0.2;

#[derive(Resource)]
struct TileAssetEvictionTimer(Timer);

impl Default for TileAssetEvictionTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(5.0, TimerMode::Repeating))
    }
}

/// In-memory index of tile filenames known to exist in the disk cache.
/// Eliminates per-frame filesystem stat calls in load_visible_tiles by
/// replacing path.exists() with a HashSet lookup. Populated at startup
/// by scanning the cache directory, and updated when TileReady messages
/// arrive from the download pipeline.
#[derive(Resource, Default)]
pub struct CachedTileSet {
    pub filenames: std::collections::HashSet<String>,
}

/// Controls whether tiles display a procedural grid instead of their imagery.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct GridOverlay {
    pub enabled: bool,
    #[reflect(ignore)]
    pub texture: Handle<Image>,
}


// =============================================================================
// Plugin
// =============================================================================

pub(super) fn setup_render_systems(app: &mut App) {
        app.init_resource::<Tile3DRefreshTimer>()
            .init_resource::<AltitudeChangeTracker>()
            .init_resource::<TileAssetCache>()
            .init_resource::<CachedTileSet>()
            .init_resource::<TileAssetEvictionTimer>()
            .register_type::<GridOverlay>()
            .add_systems(Startup, (setup_tile_quad_mesh, setup_grid_overlay))
            .add_systems(Update, toggle_grid_overlay)
            .add_systems(Update, handle_basemap_change.before(load_visible_tiles))
            .add_systems(Update, handle_window_resize)
            .add_systems(Update, handle_3d_view_tile_refresh.before(load_visible_tiles))
            .add_systems(
                Update,
                update_3d_adaptive_zoom
                    .after(handle_3d_view_tile_refresh)
                    .in_set(ZoomSet::Change),
            )
            .add_systems(Update, track_altitude_changes)
            .add_systems(Update, handle_tile_load_failures)
            .add_systems(
                Update,
                request_3d_directional_downloads
                    .after(update_3d_adaptive_zoom),
            )
            .add_systems(
                Update,
                load_visible_tiles
                    .after(ZoomSet::Change),
            )
            .add_systems(Update, animate_tile_fades.after(load_visible_tiles))
            .add_systems(Update, cull_offscreen_tiles.after(load_visible_tiles))
            .add_systems(Update, orient_tiles_for_view_mode.after(load_visible_tiles))
            .add_systems(Update, update_cached_tile_set.before(load_visible_tiles))
            .add_systems(Update, evict_stale_tile_assets);
}

// =============================================================================
// Grid Overlay
// =============================================================================

/// Generate a procedural grid texture (512x512) with a dark background and
/// lighter grid lines showing tile subdivisions.
fn setup_grid_overlay(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let size = constants::DEFAULT_TILE_PIXELS as u32; // 512
    let mut data = vec![0u8; (size * size * 4) as usize];

    let bg = [40u8, 44, 52, 255]; // dark charcoal
    let line_major = [100u8, 110, 130, 255]; // lighter for outer border
    let line_minor = [70u8, 78, 90, 255]; // subtle inner grid

    let subdivisions = 4u32; // 4x4 inner grid
    let cell = size / subdivisions;

    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let on_border = x == 0 || x == size - 1 || y == 0 || y == size - 1 || x == 1 || y == 1;
            let on_minor = x % cell == 0 || y % cell == 0;

            let color = if on_border {
                &line_major
            } else if on_minor {
                &line_minor
            } else {
                &bg
            };
            data[idx..idx + 4].copy_from_slice(color);
        }
    }

    let image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );

    let handle = images.add(image);
    commands.insert_resource(GridOverlay {
        enabled: false,
        texture: handle,
    });
}

/// When `GridOverlay.enabled` changes, update all tile materials to swap
/// between the grid texture and their original imagery.
fn toggle_grid_overlay(
    grid: Res<GridOverlay>,
    tiles: Query<(&TileOriginalImage, &MeshMaterial3d<StandardMaterial>), With<MapTile>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    view3d_state: Res<view3d::View3DState>,
) {
    if !grid.is_changed() {
        return;
    }

    let boost = if view3d_state.is_3d_active() {
        super::TILE_EMISSIVE_BOOST
    } else {
        1.0
    };

    for (original, mat_handle) in tiles.iter() {
        let Some(mut mat) = materials.get_mut(mat_handle) else {
            continue;
        };
        let texture = if grid.enabled {
            grid.texture.clone()
        } else {
            original.0.clone()
        };
        mat.emissive_texture = Some(texture);
        mat.emissive = bevy::color::LinearRgba::new(boost, boost, boost, 1.0);
    }
}

// =============================================================================
// Altitude-Adaptive Zoom
// =============================================================================

/// Map camera altitude (feet) to an appropriate tile zoom level for 3D mode.
/// Uses a logarithmic mapping since each zoom level doubles resolution.
/// Higher altitudes use lower zoom levels (wider view), lower altitudes
/// use higher zoom levels (more detail).
fn raw_altitude_to_zoom(altitude_ft: f32) -> f32 {
    let reference_alt = 5000.0_f32;
    let reference_zoom = 16.0_f32;
    let ratio = (altitude_ft / reference_alt).max(1.0);
    (reference_zoom - ratio.log2() * 1.5).clamp(8.0, 18.0)
}

/// Compute the adaptive zoom level with hysteresis to prevent flashing.
/// Only changes zoom when the raw value has moved past the boundary by
/// the hysteresis amount, preventing rapid oscillation at zoom boundaries.
/// Uses asymmetric hysteresis: 0.7 to upgrade (biased toward staying at
/// lower zoom), 0.6 to downgrade, preventing the rapid 14↔15 oscillation
/// seen at altitude boundaries.
pub fn altitude_to_zoom_level(altitude_ft: f32, current_zoom: u8) -> u8 {
    let raw = raw_altitude_to_zoom(altitude_ft);
    let target = raw.round() as u8;
    let current = current_zoom as f32;

    // When far from target (e.g. entering 3D mode), jump directly
    // instead of climbing one level at a time every 300ms.
    if target.abs_diff(current_zoom) > 1 {
        return target.clamp(8, 18);
    }

    // Normal hysteresis for single-level transitions during scrolling
    if raw > current + 0.7 {
        (current as u8 + 1).min(18)
    } else if raw < current - 0.6 {
        (current as u8).saturating_sub(1).max(8)
    } else {
        current_zoom
    }
}

// =============================================================================
// Screen-Space Error LOD
// =============================================================================

/// Maximum screen-space error in pixels before a tile should be refined
/// to a higher zoom level. Lower values = more detail, more tiles.
const SSE_THRESHOLD: f32 = 128.0;

/// Minimum zoom level used for the coarsest horizon tiles.
const MIN_SSE_ZOOM: u8 = 4;

/// Compute the ideal zoom level for a tile at `ground_distance` meters from
/// the camera. Uses screen-space error: at distance `d`, a tile of size `s`
/// covers `s * screen_factor / d` pixels. We want that to be <= SSE_THRESHOLD.
/// Solving: s <= SSE_THRESHOLD * d / screen_factor, and s = extent / 2^zoom,
/// gives zoom >= log2(extent * screen_factor / (SSE_THRESHOLD * d)).
fn zoom_for_distance(
    ground_distance: f32,
    screen_factor: f32,
    max_zoom: u8,
) -> u8 {
    if ground_distance < 1.0 {
        return max_zoom;
    }
    let extent = 2.0 * super::WEB_MERCATOR_EXTENT as f32;
    let ideal = (extent * screen_factor / (SSE_THRESHOLD * ground_distance))
        .log2()
        .ceil() as i32;
    (ideal.max(MIN_SSE_ZOOM as i32) as u8).min(max_zoom)
}

/// Compute the screen_factor used by zoom_for_distance. This converts
/// world-space tile size to screen pixels: pixels = tile_size * screen_factor / distance.
fn compute_screen_factor(window_height: f32, fov: f32) -> f32 {
    window_height / (2.0 * (fov / 2.0).tan())
}

// =============================================================================
// Tile Helpers
// =============================================================================

/// Compute the tile download radius needed to cover the viewport.
///
/// In 2D (orthographic): each tile occupies `256 * camera_zoom` screen pixels.
/// In 3D (perspective): the tilted camera sees a larger ground footprint, so we
/// estimate the visible ground extent from the camera distance, pitch, and FOV.
pub fn compute_tile_radius(
    window_width: f32,
    window_height: f32,
    camera_zoom: f32,
    view3d_state: Option<&view3d::View3DState>,
    zoom_level: u8,
) -> u8 {
    let tile_size_meters = (2.0 * super::WEB_MERCATOR_EXTENT as f32)
        / (1u64 << zoom_level) as f32;

    if let Some(state) = view3d_state {
        if state.is_3d_active() {
            let fov = 60.0_f32.to_radians();
            let aspect = window_width / window_height;
            let half_vfov = fov / 2.0;
            let half_hfov = (aspect * half_vfov.tan()).atan();
            let pitch_rad = state.camera_pitch.to_radians();

            let effective_distance = state.altitude_to_distance();
            let camera_height = effective_distance * pitch_rad.sin();

            let far_angle = (pitch_rad - half_vfov).max(0.05);
            let far_ground_dist = camera_height / far_angle.tan();

            let center_ground_dist = effective_distance * pitch_rad.cos();
            let half_width = center_ground_dist * half_hfov.tan();

            let max_ground_extent = far_ground_dist.max(half_width);
            let tiles_needed = (max_ground_extent / tile_size_meters).ceil() as u16 + 1;
            return (tiles_needed.min(60)) as u8;
        }
    }

    let tile_screen_px = constants::DEFAULT_TILE_PIXELS * camera_zoom;
    let half_tiles_x = (window_width / (2.0 * tile_screen_px)).ceil() as u8 + 1;
    let half_tiles_y = (window_height / (2.0 * tile_screen_px)).ceil() as u8 + 1;
    half_tiles_x.max(half_tiles_y).clamp(3, 25)
}

/// Send a tile download request for the current map location.
pub fn request_tiles_at_location(
    download_events: &mut MessageWriter<DownloadTilesRequest>,
    latitude: f64,
    longitude: f64,
    zoom_level: ZoomLevel,
    radius: u8,
    use_cache: bool,
) {
    download_events.write(DownloadTilesRequest {
        latitude,
        longitude,
        zoom: zoom_level.to_u8(),
        radius: Radius(radius),
        priority: DownloadPriority::Near,
        use_cache,
    });
}

// =============================================================================
// Tile Systems
// =============================================================================

/// Detect basemap style changes and clear all stale tile entities so the new
/// basemap's tiles can load cleanly without old imagery bleeding through.
fn handle_basemap_change(
    mut commands: Commands,
    basemap_state: Res<crate::config::CurrentBasemapState>,
    tile_query: Query<Entity, With<MapTile>>,
    mut tile_asset_cache: ResMut<TileAssetCache>,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    mut tile_pool: ResMut<super::pool::TilePool>,
    mut downloaded_tiles: ResMut<super::download::DownloadedTiles>,
    mut download_events: MessageWriter<DownloadTilesRequest>,
    map_state: Res<MapState>,
    zoom_state: Res<ZoomState>,
    window_query: Query<&Window>,
    view3d_state: Res<view3d::View3DState>,
    mut cached_tile_set: ResMut<CachedTileSet>,
    mut last_style: Local<Option<crate::config::BasemapStyle>>,
) {
    let current = basemap_state.style;
    if *last_style == Some(current) {
        return;
    }
    let is_first_run = last_style.is_none();
    *last_style = Some(current);
    if is_first_run {
        return;
    }

    info!(
        "Basemap changed to {:?} - clearing all tile entities",
        current
    );

    for entity in tile_query.iter() {
        super::pool::release_tile(&mut commands, entity, &mut tile_pool);
    }

    tile_grid.occupied.clear();
    tile_asset_cache.entries.clear();
    super::download::clear_download_tracking(&mut downloaded_tiles);
    scan_tile_cache_for_style(&mut cached_tile_set, &basemap_state.style.cache_key());

    let radius = if let Ok(window) = window_query.single() {
        compute_tile_radius(window.width(), window.height(), zoom_state.camera_zoom, Some(&view3d_state), map_state.zoom_level.to_u8())
    } else {
        constants::TILE_DOWNLOAD_RADIUS
    };
    request_tiles_at_location(
        &mut download_events,
        map_state.latitude,
        map_state.longitude,
        map_state.zoom_level,
        radius,
        true,
    );
}

/// When a tile image fails to load, check if the cached file is corrupt and remove it.
fn handle_tile_load_failures(
    mut commands: Commands,
    mut failed_events: MessageReader<AssetLoadFailedEvent<Image>>,
    tile_query: Query<
        (Entity, &TileOriginalImage, &TileFadeState),
        With<MapTile>,
    >,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    mut tile_pool: ResMut<super::pool::TilePool>,
) {
    for event in failed_events.read() {
        let asset_path = event.path.path();
        let path_str = asset_path.to_string_lossy();
        if path_str.contains(".tile.") {
            warn!(
                "Tile asset load failed: {:?} - checking for corrupt cache file",
                asset_path
            );
            tile_cache::remove_corrupt_cached_tile(asset_path);

            let failed_id = event.id;
            for (entity, original, fade) in tile_query.iter() {
                if original.0.id() == failed_id {
                    if let Some(key) = parse_tile_key_from_path(&path_str, fade.tile_zoom) {
                        tile_grid.occupied.remove(&key);
                    }
                    super::pool::release_tile(&mut commands, entity, &mut tile_pool);
                    debug!(
                        "Despawned tile entity with failed texture: {:?}",
                        asset_path
                    );
                    break;
                }
            }
        }
    }
}

/// Parse tile coordinates from a tile filename for TileGrid key lookup.
fn parse_tile_key_from_path(path: &str, zoom: u8) -> Option<super::pool::TileGridKey> {
    // Filename: ".../{zoom}.{x}.{y}.{size}.tile.{ext}"
    let filename = path.rsplit('/').next()?;
    let parts: Vec<&str> = filename.split('.').collect();
    if parts.len() >= 3 {
        let x: u32 = parts[1].parse().ok()?;
        let y: u32 = parts[2].parse().ok()?;
        Some((x, y, zoom))
    } else {
        None
    }
}

/// Request tiles when the window is resized or maximized so newly exposed areas are filled.
fn handle_window_resize(
    mut resize_events: MessageReader<bevy::window::WindowResized>,
    mut download_events: MessageWriter<DownloadTilesRequest>,
    map_state: Res<MapState>,
    zoom_state: Res<ZoomState>,
    view3d_state: Res<view3d::View3DState>,
) {
    for event in resize_events.read() {
        let radius = compute_tile_radius(
            event.width,
            event.height,
            zoom_state.camera_zoom,
            Some(&view3d_state), map_state.zoom_level.to_u8(),
        );
        download_events.write(DownloadTilesRequest {
            latitude: map_state.latitude,
            longitude: map_state.longitude,
            zoom: map_state.zoom_level.to_u8(),
            radius: Radius(radius),
            priority: DownloadPriority::Near,
            use_cache: true,
        });
    }
}

/// Re-request tiles when 3D view state changes (entering/exiting 3D, orbit, pitch, distance)
/// so the larger perspective footprint is covered.
/// When returning to 2D, clears spawned tile tracking so tiles are freshly re-displayed.
fn handle_3d_view_tile_refresh(
    mut commands: Commands,
    view3d_state: Res<view3d::View3DState>,
    mut download_events: MessageWriter<DownloadTilesRequest>,
    mut map_state: ResMut<MapState>,
    zoom_state: Res<ZoomState>,
    window_query: Query<&Window>,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    tile_query: Query<Entity, With<MapTile>>,
) {
    if !view3d_state.is_changed() {
        return;
    }

    // Restore saved 2D zoom level when returning from 3D mode.
    // With Mercator meters, tile positions are zoom-independent, so we
    // no longer need to clear tile_grid. Tiles from 3D mode are despawned
    // by orient_tiles_for_view_mode on the mode transition.
    if matches!(view3d_state.mode, view3d::ViewMode::Map2D) && !view3d_state.is_transitioning() {
        if let Some(saved_zoom) = view3d_state.saved_2d_zoom_level {
            if let Ok(zoom) = ZoomLevel::try_from(saved_zoom) {
                map_state.zoom_level = zoom;
                debug!("Restored 2D zoom level: {}", saved_zoom);
            }
        }
    }

    let Ok(window) = window_query.single() else {
        return;
    };
    let radius = compute_tile_radius(
        window.width(),
        window.height(),
        zoom_state.camera_zoom,
        Some(&view3d_state), map_state.zoom_level.to_u8(),
    );
    download_events.write(DownloadTilesRequest {
        latitude: map_state.latitude,
        longitude: map_state.longitude,
        zoom: map_state.zoom_level.to_u8(),
        radius: Radius(radius),
        priority: DownloadPriority::Near,
        use_cache: true,
    });
}

/// Update the map zoom level in 3D mode based on camera altitude.
/// Runs on the refresh timer so zoom changes happen smoothly, not every frame.
/// When zoom changes, despawns out-of-band tiles to prevent accumulation.
fn update_3d_adaptive_zoom(
    mut commands: Commands,
    mut timer: ResMut<Tile3DRefreshTimer>,
    time: Res<Time>,
    view3d_state: Res<view3d::View3DState>,
    mut map_state: ResMut<MapState>,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    mut tile_pool: ResMut<super::pool::TilePool>,
    tile_query: Query<(Entity, &TileFadeState), With<MapTile>>,
    mut alt_tracker: ResMut<AltitudeChangeTracker>,
) {
    if !view3d_state.is_3d_active() {
        return;
    }

    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }

    let old_zoom = map_state.zoom_level.to_u8();
    let adaptive_zoom = altitude_to_zoom_level(view3d_state.camera_altitude, old_zoom);
    if let Ok(new_zoom) = ZoomLevel::try_from(adaptive_zoom) {
        if map_state.zoom_level != new_zoom && alt_tracker.zoom_cooldown <= 0.0 {
            alt_tracker.zoom_cooldown = 1.0;
            debug!(
                "3D adaptive zoom: altitude {:.0} ft -> zoom {}",
                view3d_state.camera_altitude, adaptive_zoom
            );
            map_state.zoom_level = new_zoom;
            // Don't despawn old tiles here - animate_tile_fades handles
            // gradual replacement once new-zoom tiles have loaded.
        }
    }
}

/// Track camera altitude changes to soften tile culling during rapid zoom.
fn track_altitude_changes(
    time: Res<Time>,
    view3d_state: Res<view3d::View3DState>,
    mut tracker: ResMut<AltitudeChangeTracker>,
) {
    let dt = time.delta_secs();
    let current = view3d_state.camera_altitude;
    let delta = (current - tracker.prev_altitude).abs();
    if delta > 50.0 {
        tracker.idle_secs = 0.0;
    } else {
        tracker.idle_secs += dt;
    }
    tracker.prev_altitude = current;
    tracker.zoom_cooldown = (tracker.zoom_cooldown - dt).max(0.0);
}

// rescale_tiles_on_zoom_change was removed: tile positions are now in
// zoom-independent Mercator meters, so no rescaling is needed when the
// discrete zoom level changes.

/// Primary tile loading system. Runs every frame. Checks disk cache directly,
/// spawns cached tiles immediately (zero latency), and requests network
/// downloads for uncached tiles.
///
/// Tile positions use Mercator meters relative to the LocalOrigin. This makes
/// positions zoom-independent: the same tile always appears at the same world
/// position regardless of zoom level. Only the tile's scale changes with zoom
/// (higher zoom = smaller tiles in meters).
///
/// In 2D: single band at current zoom.
/// In 3D: current zoom + lower-zoom bands centered on camera for coverage.
fn load_visible_tiles(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    dl_settings: Res<super::download::TileDownloadSettings>,
    map_state: Res<MapState>,
    local_origin: Res<super::coords::LocalOrigin>,
    mut tile_asset_cache: ResMut<TileAssetCache>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    quad_mesh: Option<Res<TileQuadMesh>>,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    mut tile_pool: ResMut<super::pool::TilePool>,
    mut download_events: MessageWriter<DownloadTilesRequest>,
    view3d_state: Res<view3d::View3DState>,
    window_query: Query<&Window>,
    zoom_state: Res<crate::map::ZoomState>,
    time: Res<Time<Real>>,
    cached_tile_set: Res<CachedTileSet>,
) {
    let Some(ref quad_meshes) = quad_mesh else {
        return;
    };
    let current_zoom = map_state.zoom_level.to_u8();
    let now = time.elapsed_secs_f64();
    let is_3d = view3d_state.is_3d_active();

    let radius = if let Ok(window) = window_query.single() {
        compute_tile_radius(
            window.width(), window.height(),
            zoom_state.camera_zoom, Some(&view3d_state), map_state.zoom_level.to_u8(),
        )
    } else {
        constants::TILE_DOWNLOAD_RADIUS
    };

    let ext = dl_settings.tile_format.extension();
    let tile_px = constants::DEFAULT_TILE_SIZE.to_pixels();

    let boost = if is_3d { super::TILE_EMISSIVE_BOOST } else { 1.0 };
    let tile_z = if is_3d {
        view3d_state.altitude_to_z(view3d_state.ground_elevation_ft)
    } else {
        -1.0
    };
    // No rotation needed: 2D uses Rectangle (XY plane), 3D uses Plane3d (XZ plane)

    let lat = map_state.latitude;
    let lon = map_state.longitude;
    let origin_xy = local_origin.mercator_origin().truncate();

    struct TileBand {
        lat: f64,
        lon: f64,
        zoom: u8,
        radius: u8,
    }

    let mut bands = Vec::with_capacity(8);
    bands.push(TileBand { lat, lon, zoom: current_zoom, radius });
    let mut requested_zooms = std::collections::HashSet::new();

    if is_3d {
        // SSE-driven LOD: each zoom level has a maximum distance threshold
        // beyond which tiles at that zoom are too detailed. We iterate from
        // current_zoom (finest, near camera) down to MIN_SSE_ZOOM (coarsest,
        // horizon), spawning tiles only within their SSE-appropriate distance.
        let fov = 60.0_f32.to_radians();
        let screen_factor = if let Ok(window) = window_query.single() {
            compute_screen_factor(window.height(), fov)
        } else {
            540.0
        };

        let cam_merc = super::coords::lonlat_to_mercator(lon, lat);
        let cam_local = bevy::math::Vec2::new(
            (cam_merc.x - origin_xy.x) as f32,
            (cam_merc.y - origin_xy.y) as f32,
        );

        let min_zoom = current_zoom.saturating_sub(5).max(MIN_SSE_ZOOM);
        bands.clear();

        // For each zoom level, compute the max distance where SSE is satisfied,
        // then compute how many tiles that covers as a radius.
        for zoom in (min_zoom..=current_zoom).rev() {
            let tile_size = (2.0 * super::WEB_MERCATOR_EXTENT as f32)
                / (1u64 << zoom) as f32;
            // max_distance = tile_size * screen_factor / SSE_THRESHOLD
            let max_dist = tile_size * screen_factor / SSE_THRESHOLD;
            let tile_radius = ((max_dist / tile_size).ceil() as u8).clamp(3, 25);

            bands.push(TileBand {
                lat, lon,
                zoom,
                radius: tile_radius,
            });
        }
    }

    for band in &bands {
        let Ok(band_zoom) = ZoomLevel::try_from(band.zoom) else { continue };
        let center = super::coords::SlippyTileCoordinates::from_latitude_longitude(
            band.lat, band.lon, band_zoom,
        );

        let r = band.radius as i64;
        let max_tile = 1i64 << band.zoom;

        for dx in -r..=r {
            for dy in -r..=r {
                let raw_x = center.x as i64 + dx;
                let y = center.y as i64 + dy;
                if y < 0 || y >= max_tile { continue; }
                let x = super::coords::wrap_tile_x(raw_x, band.zoom);

                let tile_key = (x, y as u32, band.zoom);
                if tile_grid.occupied.contains_key(&tile_key) {
                    continue;
                }

                let filename = format!(
                    "{}.{}.{}.{}.tile.{}", band.zoom, x, y as u32, tile_px, ext
                );
                let cached = cached_tile_set.filenames.contains(&filename);

                if !cached {
                    if !requested_zooms.contains(&band.zoom) {
                        requested_zooms.insert(band.zoom);
                        download_events.write(DownloadTilesRequest {
                            latitude: band.lat,
                            longitude: band.lon,
                            zoom: band.zoom,
                            radius: Radius(band.radius),
                            priority: DownloadPriority::Near,
                            use_cache: true,
                        });
                    }
                    continue;
                }

                let asset_path = format!("tiles/{}/{}", dl_settings.cache_key, filename);
                let tile_handle: Handle<Image> =
                    if let Some((h, access_time)) = tile_asset_cache.entries.get_mut(&asset_path) {
                        *access_time = now;
                        h.clone()
                    } else {
                        let h: Handle<Image> = asset_server.load(&asset_path);
                        tile_asset_cache.entries.insert(asset_path, (h.clone(), now));
                        h
                    };

                // Compute tile position in Mercator meters (size is baked into the per-zoom mesh)
                let aabb = super::coords::tile_to_mercator_aabb(x, y as u32, band.zoom);
                let center_merc = aabb.center();
                let local_x = (center_merc.x - origin_xy.x) as f32;
                let local_y = (center_merc.y - origin_xy.y) as f32;

                let material = materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: Some(tile_handle.clone()),
                    unlit: true,
                    alpha_mode: AlphaMode::Opaque,
                    ..default()
                });

                let (tile_pos, tile_mesh) = if is_3d {
                    // Plane3d in XZ plane, no rotation needed
                    let mesh = quad_meshes.meshes_3d.get(&band.zoom);
                    (Vec3::new(local_x, tile_z, -local_y), mesh)
                } else {
                    // Rectangle in XY plane, no rotation needed
                    let mesh = quad_meshes.meshes_2d.get(&band.zoom);
                    (Vec3::new(local_x, local_y, tile_z), mesh)
                };

                let Some(tile_mesh) = tile_mesh else { continue };

                let entity = match tile_pool.take() {
                    Some(e) => e,
                    None => {
                        super::pool::grow_pool(&mut commands, &mut tile_pool, 64);
                        tile_pool.take().expect("pool should have entities after grow")
                    }
                };
                super::pool::activate_tile(
                    &mut commands,
                    entity,
                    &mut tile_grid,
                    tile_key,
                    Transform::from_translation(tile_pos),
                    tile_mesh.clone(),
                    material,
                    tile_handle,
                    band.zoom,
                    now,
                );
            }
        }
    }
}

/// Send directional download requests in 3D mode so that tiles ahead of the
/// camera get fetched and cached. `load_visible_tiles` (running every frame)
/// picks them up from cache on the next frame.
fn request_3d_directional_downloads(
    timer: Res<Tile3DRefreshTimer>,
    view3d_state: Res<view3d::View3DState>,
    map_state: Res<MapState>,
    mut download_events: MessageWriter<DownloadTilesRequest>,
) {
    if !view3d_state.is_3d_active() || !timer.0.just_finished() {
        return;
    }

    let base_zoom = map_state.zoom_level.to_u8();
    let lat = map_state.latitude;
    let lon = map_state.longitude;
    let yaw_rad = view3d_state.camera_yaw.to_radians();
    let pitch = view3d_state.camera_pitch;
    let pitch_factor = ((pitch - 15.0) / (89.0 - 15.0)).clamp(0.0, 1.0);

    let near_radius = 3 + (3.0 * pitch_factor) as u8;
    let mid_radius = 3 + (2.0 * (1.0 - pitch_factor)) as u8;
    let far_radius = 2 + (3.0 * (1.0 - pitch_factor)) as u8;

    download_events.write(DownloadTilesRequest {
        latitude: lat, longitude: lon,
        zoom: base_zoom, radius: Radius(near_radius),
        priority: DownloadPriority::Near, use_cache: true,
    });

    let mut request_band = |zoom_offset: u8, fwd: f64, side: f64, radius: u8, priority: DownloadPriority| {
        if base_zoom < zoom_offset { return; }
        let z = base_zoom - zoom_offset;
        let deg_per_tile_lon = 360.0 / (1u64 << z) as f64;
        let deg_per_tile_lat = deg_per_tile_lon * lat.to_radians().cos();
        let offset_lat = fwd * deg_per_tile_lat * yaw_rad.cos() as f64
            - side * deg_per_tile_lat * yaw_rad.sin() as f64;
        let offset_lon = fwd * deg_per_tile_lon * yaw_rad.sin() as f64
            + side * deg_per_tile_lon * yaw_rad.cos() as f64;
        download_events.write(DownloadTilesRequest {
            latitude: clamp_latitude(lat + offset_lat),
            longitude: clamp_longitude(lon + offset_lon),
            zoom: z, radius: Radius(radius),
            priority, use_cache: true,
        });
    };

    request_band(1, 3.0, 0.0, mid_radius, DownloadPriority::Mid);
    request_band(1, 2.0, -4.0, mid_radius, DownloadPriority::Mid);
    request_band(1, 2.0, 4.0, mid_radius, DownloadPriority::Mid);

    request_band(2, 4.0, 0.0, far_radius, DownloadPriority::Far);
    request_band(2, 3.0, -5.0, far_radius, DownloadPriority::Far);
    request_band(2, 3.0, 5.0, far_radius, DownloadPriority::Far);

    let hr = 4 + (3.0 * (1.0 - pitch_factor)) as u8;
    for &fwd in &[2.0, 5.0, 8.0] {
        request_band(3, fwd, 0.0, hr, DownloadPriority::Far);
        let spread = fwd * 1.5 + 4.0;
        request_band(3, fwd, -spread, hr, DownloadPriority::Far);
        request_band(3, fwd, spread, hr, DownloadPriority::Far);
    }

    let ur = 4 + (2.0 * (1.0 - pitch_factor)) as u8;
    for &fwd in &[2.0, 5.0, 8.0] {
        request_band(4, fwd, 0.0, ur, DownloadPriority::Far);
        let spread = fwd * 2.0 + 5.0;
        request_band(4, fwd, -spread, ur, DownloadPriority::Far);
        request_band(4, fwd, spread, ur, DownloadPriority::Far);
    }
}

/// Maximum number of tile entities allowed at any time.
/// In 3D mode the multi-resolution band system (5 zoom levels with
/// directional requests) can generate 800-1200 tiles at steady state.
/// The budget must exceed this to prevent a spawn-cull-respawn cycle
/// where culled tiles are re-requested every 300ms, causing flashing
/// as they respawn at alpha 0 and fade back in.
fn max_tile_entities(view3d_state: Option<&view3d::View3DState>) -> usize {
    if let Some(state) = view3d_state {
        if state.is_3d_active() {
            return 5000;
        }
    }
    500
}

/// Despawn tile entities that are far outside the visible area.
/// Without this, tiles accumulate indefinitely as the user pans, causing
/// frame time to grow continuously until the app becomes unresponsive.
///
/// In 2D mode: uses axis-aligned distance from the MapCamera.
/// In 3D mode: uses ground-plane (XZ) distance from the AircraftCamera.
fn cull_offscreen_tiles(
    mut commands: Commands,
    camera_query: Query<&Transform, With<MapCamera>>,
    camera_3d_query: Query<&Transform, With<crate::camera::AircraftCamera>>,
    tile_query: Query<(Entity, &Transform, &TileFadeState), With<MapTile>>,
    window_query: Query<&Window>,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    mut tile_pool: ResMut<super::pool::TilePool>,
    view3d_state: Res<view3d::View3DState>,
    map_state: Res<MapState>,
    zoom_state: Res<crate::map::ZoomState>,
) {
    let is_3d = view3d_state.is_3d_active();

    let Ok(window) = window_query.single() else {
        return;
    };

    let tile_size_m = (2.0 * super::WEB_MERCATOR_EXTENT as f32) / (1u64 << map_state.zoom_level.to_u8()) as f32;
    let visible_radius = compute_tile_radius(
        window.width(), window.height(),
        zoom_state.camera_zoom, Some(&view3d_state), map_state.zoom_level.to_u8(),
    ) as f32;
    let cull_radius = tile_size_m * (visible_radius + 2.0);

    // In 3D, the lowest zoom band (zoom-5) has tiles 32x the current tile
    // size, with radius 8. The cull distance must cover the full extent of
    // the lowest band plus margin.
    let cull_radius = if is_3d {
        let lowest_band_tile_size = tile_size_m * 32.0;
        (lowest_band_tile_size * 12.0).max(cull_radius * 2.0)
    } else {
        cull_radius
    };

    let mut to_despawn: Vec<Entity> = Vec::new();

    if is_3d {
        // 3D mode: cull by ground-plane distance (XZ in Y-up space)
        let Ok(cam_tf) = camera_3d_query.single() else {
            return;
        };
        let cam_x = cam_tf.translation.x;
        let cam_z = cam_tf.translation.z;

        for (entity, tile_tf, _) in tile_query.iter() {
            let dx = (tile_tf.translation.x - cam_x).abs();
            let dz = (tile_tf.translation.z - cam_z).abs();
            if dx > cull_radius || dz > cull_radius {
                to_despawn.push(entity);
            }
        }
    } else {
        // 2D mode: cull by XY distance from MapCamera
        let Ok(camera_tf) = camera_query.single() else {
            return;
        };
        let cam_x = camera_tf.translation.x;
        let cam_y = camera_tf.translation.y;

        for (entity, tile_tf, _) in tile_query.iter() {
            let dx = (tile_tf.translation.x - cam_x).abs();
            let dy = (tile_tf.translation.y - cam_y).abs();
            if dx > cull_radius || dy > cull_radius {
                to_despawn.push(entity);
            }
        }
    }

    // Budget-based culling: if over limit, sort by distance and cull farthest
    let tile_limit = max_tile_entities(Some(&view3d_state));
    let total_tiles = tile_query.iter().count();
    if total_tiles > tile_limit && to_despawn.len() < total_tiles - tile_limit {
        let cam_pos = if is_3d {
            camera_3d_query.single().map(|tf| tf.translation).unwrap_or_default()
        } else {
            camera_query.single().map(|tf| tf.translation).unwrap_or_default()
        };
        let mut tiles_by_dist: Vec<(Entity, f32)> = tile_query
            .iter()
            .map(|(e, tf, _)| {
                let dist = if is_3d {
                    (tf.translation.x - cam_pos.x).abs().max((tf.translation.z - cam_pos.z).abs())
                } else {
                    (tf.translation.x - cam_pos.x).abs().max((tf.translation.y - cam_pos.y).abs())
                };
                (e, dist)
            })
            .collect();
        tiles_by_dist.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for &(e, _) in &tiles_by_dist[..tiles_by_dist.len().saturating_sub(tile_limit)] {
            if !to_despawn.contains(&e) {
                to_despawn.push(e);
            }
        }
    }

    if !to_despawn.is_empty() {
        let despawn_set: std::collections::HashSet<Entity> = to_despawn.iter().copied().collect();
        tile_grid.occupied.retain(|_, &mut e| !despawn_set.contains(&e));
        for entity in &to_despawn {
            super::pool::release_tile(&mut commands, *entity, &mut tile_pool);
        }
    }
}

/// Show tiles once their texture is loaded. In 3D mode, dominated tiles
/// (too detailed or too coarse for the current zoom) are kept visible until
/// a loaded tile at a closer zoom level covers the same area, preventing
/// flashing during zoom transitions.
fn animate_tile_fades(
    mut commands: Commands,
    map_state: Res<MapState>,
    images: Res<Assets<Image>>,
    mut tile_query: Query<
        (Entity, &mut TileFadeState, &mut Visibility, &TileOriginalImage),
        With<MapTile>,
    >,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    mut tile_pool: ResMut<super::pool::TilePool>,
    view3d_state: Res<view3d::View3DState>,
) {
    let current_zoom = map_state.zoom_level.to_u8();
    let is_3d = view3d_state.is_3d_active();

    // Show tiles whose textures have loaded
    for (_, mut fade_state, mut visibility, original) in tile_query.iter_mut() {
        if images.contains(&original.0) && *visibility == Visibility::Hidden {
            fade_state.alpha = 1.0;
            *visibility = Visibility::Inherited;
        }
    }

    // In 3D, depth ordering (update_tile_elevation) ensures higher-zoom
    // tiles render on top of lower-zoom tiles. We only need to remove tiles
    // that are way outside the useful zoom band. Distance-based culling
    // (cull_offscreen_tiles) handles geographic cleanup.
    if is_3d {
        let min_band = current_zoom.saturating_sub(5);
        let max_band = current_zoom + 2;
        let mut to_release: Vec<(Entity, u8)> = Vec::new();
        for (&(_, _, z), &ent) in tile_grid.occupied.iter() {
            if z < min_band || z > max_band {
                to_release.push((ent, z));
            }
        }
        for (entity, zoom) in to_release {
            tile_grid.occupied.retain(|&(_, _, z), &mut e| !(z == zoom && e == entity));
            super::pool::release_tile(&mut commands, entity, &mut tile_pool);
        }
    } else {
        // 2D: only keep current zoom tiles
        let mut has_current = false;
        for (&(_, _, z), &ent) in tile_grid.occupied.iter() {
            if z == current_zoom {
                if let Ok((_, _, vis, _)) = tile_query.get(ent) {
                    if *vis == Visibility::Inherited {
                        has_current = true;
                        break;
                    }
                }
            }
        }
        if has_current {
            let mut to_release: Vec<(Entity, u8)> = Vec::new();
            for (&(_, _, z), &ent) in tile_grid.occupied.iter() {
                if z != current_zoom {
                    to_release.push((ent, z));
                }
            }
            for (entity, zoom) in to_release {
                tile_grid.occupied.retain(|&(_, _, z), &mut e| !(z == zoom && e == entity));
                super::pool::release_tile(&mut commands, entity, &mut tile_pool);
            }
        }
    }
}

// =============================================================================
// 3D Mesh Quad Systems
// =============================================================================

/// Create per-zoom mesh handles for both 2D (Rectangle) and 3D (Plane3d).
fn setup_tile_quad_mesh(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    let mut meshes_2d = std::collections::HashMap::new();
    let mut meshes_3d = std::collections::HashMap::new();
    for zoom in 0..=19u8 {
        let tile_size = (2.0 * super::WEB_MERCATOR_EXTENT) / (1u64 << zoom) as f64;
        let size = tile_size as f32 * 1.001; // slight overlap to prevent seam gaps
        // Rectangle: XY plane, faces -Z. Perfect for Camera3d ortho looking at -Z.
        meshes_2d.insert(zoom, meshes.add(Rectangle::new(size, size)));
        // Plane3d: XZ plane, faces +Y. Perfect for 3D perspective looking down.
        let half = size / 2.0;
        meshes_3d.insert(zoom, meshes.add(Plane3d::new(Vec3::Y, Vec2::new(half, half))));
    }
    commands.insert_resource(TileQuadMesh { meshes_2d, meshes_3d });
}

/// When view mode changes, despawn all tiles and clear tracking so
/// load_visible_tiles respawns them fresh in the correct coordinate system.
fn orient_tiles_for_view_mode(
    mut commands: Commands,
    view3d_state: Res<view3d::View3DState>,
    tile_query: Query<Entity, With<MapTile>>,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    mut tile_pool: ResMut<super::pool::TilePool>,
    mut last_3d: Local<Option<bool>>,
) {
    let is_3d = view3d_state.is_3d_active();
    if *last_3d == Some(is_3d) {
        return;
    }
    let first_run = last_3d.is_none();
    *last_3d = Some(is_3d);

    if first_run {
        return;
    }

    for entity in tile_query.iter() {
        super::pool::release_tile(&mut commands, entity, &mut tile_pool);
    }
    tile_grid.occupied.clear();
}

// =============================================================================
// Tile Asset Eviction
// =============================================================================

/// Periodically evict stale entries from TileAssetCache to reclaim GPU memory.
/// Removes entries not accessed in 30s whose tiles are not currently displayed.
/// Also enforces a hard cap to prevent unbounded growth.
fn evict_stale_tile_assets(
    mut timer: ResMut<TileAssetEvictionTimer>,
    time: Res<Time>,
    mut tile_asset_cache: ResMut<TileAssetCache>,
    tile_grid: Res<super::pool::TileGrid>,
    tile_query: Query<&TileOriginalImage, With<MapTile>>,
) {
    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }

    let now = time.elapsed_secs_f64();
    let active_handles: std::collections::HashSet<bevy::asset::AssetId<Image>> =
        tile_query.iter().map(|img| img.0.id()).collect();

    let before = tile_asset_cache.entries.len();

    tile_asset_cache.entries.retain(|_, (handle, access_time)| {
        if now - *access_time > TILE_ASSET_EVICTION_SECS {
            !active_handles.contains(&handle.id())
        } else {
            true
        }
    });

    // Hard cap: if still over limit, evict oldest entries
    if tile_asset_cache.entries.len() > TILE_ASSET_CACHE_HARD_CAP {
        let mut by_time: Vec<(String, f64)> = tile_asset_cache
            .entries
            .iter()
            .map(|(k, (_, t))| (k.clone(), *t))
            .collect();
        by_time.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let to_remove = tile_asset_cache.entries.len() - TILE_ASSET_CACHE_HARD_CAP;
        for (key, _) in by_time.into_iter().take(to_remove) {
            if let Some((handle, _)) = tile_asset_cache.entries.get(&key) {
                if !active_handles.contains(&handle.id()) {
                    tile_asset_cache.entries.remove(&key);
                }
            }
        }
    }

    let evicted = before.saturating_sub(tile_asset_cache.entries.len());
    if evicted > 0 {
        debug!(
            "Evicted {} tile assets (remaining: {})",
            evicted,
            tile_asset_cache.entries.len()
        );
    }
}

// =============================================================================
// Cached Tile Set (in-memory disk cache index)
// =============================================================================

/// Scan the tile cache directory for the current basemap style and populate
/// the CachedTileSet with all tile filenames found on disk. Called at startup
/// and after basemap changes.
pub fn scan_tile_cache_for_style(cached_set: &mut CachedTileSet, style_key: &str) {
    cached_set.filenames.clear();
    let cache_dir = crate::tile_cache::tile_cache_dir_for_style(style_key);
    if let Ok(entries) = std::fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.contains(".tile.") {
                    cached_set.filenames.insert(name.to_string());
                }
            }
        }
    }
    info!(
        "CachedTileSet: indexed {} tiles for style '{}'",
        cached_set.filenames.len(),
        style_key
    );
}

/// Consume TileReady messages from the download pipeline and add newly
/// cached tile filenames to the in-memory index.
fn update_cached_tile_set(
    mut ready_events: MessageReader<super::download::TileReady>,
    mut cached_set: ResMut<CachedTileSet>,
) {
    for ready in ready_events.read() {
        if let Some(filename) = ready.path.file_name().and_then(|f| f.to_str()) {
            cached_set.filenames.insert(filename.to_string());
        }
    }
}

