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
use crate::RenderCategory;
use crate::{clamp_latitude, clamp_longitude, ZoomDebugLogger, ZoomSet};
use bevy::camera::visibility::RenderLayers;

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

/// Links a tile entity to its 3D mesh quad companion (used in 3D mode only).
#[derive(Component)]
pub struct TileMeshQuad(pub Entity);

/// Marker on 3D mesh quad companion entities so orphans can be detected.
#[derive(Component)]
pub struct TileQuad3d;

/// Shared mesh handle for all 3D tile quads (sized to match DEFAULT_TILE_PIXELS).
#[derive(Resource)]
pub struct TileQuadMesh(pub Handle<Mesh>);

/// Timer that triggers periodic tile re-requests in 3D mode so that camera
/// orbit, pan, and altitude changes continuously fill visible areas.
#[derive(Resource)]
pub struct Tile3DRefreshTimer(Timer);

impl Default for Tile3DRefreshTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.3, TimerMode::Repeating))
    }
}

/// Tracks the previous 3D zoom level so tile transforms can be rescaled
/// when the zoom level changes. Without this, tiles spawned at zoom N
/// stay at zoom-N pixel coordinates while the camera and entities move
/// to zoom-(N+1) coordinates, making tiles appear at half size/position.
#[derive(Resource)]
struct Previous3DZoom(u8);

impl Default for Previous3DZoom {
    fn default() -> Self {
        Self(10) // matches default MapState zoom
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
#[derive(Resource, Default)]
struct TileAssetCache {
    entries: std::collections::HashMap<String, Handle<Image>>,
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
            .init_resource::<Previous3DZoom>()
            .init_resource::<TileAssetCache>()
            .register_type::<GridOverlay>()
            .add_systems(Startup, (setup_tile_quad_mesh, setup_grid_overlay))
            .add_systems(Update, toggle_grid_overlay)
            .add_systems(Update, handle_basemap_change.before(load_visible_tiles))
            .add_systems(Update, handle_window_resize)
            .add_systems(Update, handle_3d_view_tile_refresh)
            .add_systems(
                Update,
                update_3d_adaptive_zoom
                    .after(handle_3d_view_tile_refresh)
                    .in_set(ZoomSet::Change),
            )
            .add_systems(Update, rescale_tiles_on_zoom_change.after(ZoomSet::Change))
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
                    .after(ZoomSet::Change)
                    .after(rescale_tiles_on_zoom_change),
            )
            .add_systems(Update, animate_tile_fades.after(load_visible_tiles))
            .add_systems(Update, cull_offscreen_tiles.after(load_visible_tiles))
            .add_systems(Update, orient_tiles_for_view_mode.after(load_visible_tiles));
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
) -> u8 {
    // Check if we're in 3D perspective mode
    if let Some(state) = view3d_state {
        if state.is_3d_active() {
            let fov = 60.0_f32.to_radians();
            let aspect = window_width / window_height;
            let half_vfov = fov / 2.0;
            let half_hfov = (aspect * half_vfov.tan()).atan();
            let pitch_rad = state.camera_pitch.to_radians();

            // Camera height above the map plane
            let effective_distance = state.altitude_to_distance();
            let camera_height = effective_distance * pitch_rad.sin();

            // The far ground edge angle: pitch - half_vfov from horizontal
            // Ground distance = camera_height / tan(pitch - half_vfov)
            // Clamp the angle so we don't get infinity when looking near the horizon
            let far_angle = (pitch_rad - half_vfov).max(0.05);
            let far_ground_dist = camera_height / far_angle.tan();

            // Horizontal extent at the ground plane center
            let center_ground_dist = effective_distance * pitch_rad.cos();
            let half_width = center_ground_dist * half_hfov.tan();

            // Use whichever axis demands more tiles
            let max_ground_extent = far_ground_dist.max(half_width);
            let tile_world_size = constants::DEFAULT_TILE_PIXELS;
            let tiles_needed = (max_ground_extent / tile_world_size).ceil() as u8 + 1;
            return tiles_needed.clamp(3, 12);
        }
    }

    // 2D orthographic mode
    // +1 accounts for the camera's sub-tile offset: when the camera sits near
    // a tile edge, the far side needs one extra tile to avoid blank edges.
    let tile_screen_px = constants::DEFAULT_TILE_PIXELS * camera_zoom;
    let half_tiles_x = (window_width / (2.0 * tile_screen_px)).ceil() as u8 + 1;
    let half_tiles_y = (window_height / (2.0 * tile_screen_px)).ceil() as u8 + 1;
    half_tiles_x.max(half_tiles_y).clamp(3, 8)
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
    mut downloaded_tiles: ResMut<super::download::DownloadedTiles>,
    mut download_events: MessageWriter<DownloadTilesRequest>,
    map_state: Res<MapState>,
    zoom_state: Res<ZoomState>,
    window_query: Query<&Window>,
    view3d_state: Res<view3d::View3DState>,
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
        commands.entity(entity).despawn();
    }

    tile_grid.occupied.clear();
    tile_asset_cache.entries.clear();
    super::download::clear_download_tracking(&mut downloaded_tiles);

    let radius = if let Ok(window) = window_query.single() {
        compute_tile_radius(window.width(), window.height(), zoom_state.camera_zoom, Some(&view3d_state))
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
        (Entity, &TileOriginalImage, &Transform, &TileFadeState),
        With<MapTile>,
    >,
    mut tile_grid: ResMut<super::pool::TileGrid>,
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
            for (entity, original, transform, fade) in tile_query.iter() {
                if original.0.id() == failed_id {
                    let key = (
                        transform.translation.x as i32,
                        transform.translation.y as i32,
                        fade.tile_zoom,
                    );
                    tile_grid.occupied.remove(&key);
                    commands.entity(entity).despawn();
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
            Some(&view3d_state),
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
    view3d_state: Res<view3d::View3DState>,
    mut download_events: MessageWriter<DownloadTilesRequest>,
    mut map_state: ResMut<MapState>,
    zoom_state: Res<ZoomState>,
    window_query: Query<&Window>,
    mut tile_grid: ResMut<super::pool::TileGrid>,
) {
    if !view3d_state.is_changed() {
        return;
    }

    // When we've just returned to 2D mode, clear the tile grid tracker
    // and restore the saved 2D zoom level.
    // 3D mode uses multi-resolution tiles at different zoom levels and scales;
    // without clearing, the dedup check would skip re-spawning tiles at the
    // current zoom level, leaving a blank map.
    if matches!(view3d_state.mode, view3d::ViewMode::Map2D) && !view3d_state.is_transitioning() {
        tile_grid.occupied.clear();
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
        Some(&view3d_state),
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

            let new_z = new_zoom.to_u8();
            let min_band = new_z.saturating_sub(4);
            tile_grid
                .occupied
                .retain(|&(_, _, z), _| z >= min_band && z <= new_z);
            let mut despawned = 0u32;
            for (entity, fade_state) in tile_query.iter() {
                if fade_state.tile_zoom > new_z || fade_state.tile_zoom < min_band {
                    commands.entity(entity).despawn();
                    despawned += 1;
                }
            }
            if despawned > 0 {
                debug!(
                    "Zoom changed {}->{}: despawned {} out-of-band tiles",
                    old_zoom, new_z, despawned
                );
            }
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

/// Rescale all existing tile transforms when the 3D adaptive zoom changes.
///
/// Tiles are positioned in pixel space at their spawn-time zoom level. When
/// the discrete zoom changes, the pixel coordinate system scales by 2x per
/// level. This system applies that scale factor to existing tiles so they
/// remain correctly positioned relative to the camera and entities (which
/// recompute their positions at the new zoom level every frame).
fn rescale_tiles_on_zoom_change(
    map_state: Res<MapState>,
    view3d_state: Res<view3d::View3DState>,
    mut prev_zoom: ResMut<Previous3DZoom>,
    mut tile_query: Query<&mut Transform, With<MapTile>>,
    mut tile_grid: ResMut<super::pool::TileGrid>,
) {
    // In 3D mode, use the rendering zoom (stable coordinate space) so tile
    // positions stay consistent with entities. In 2D, use map_state directly.
    let current = if view3d_state.is_3d_active() {
        view3d_state.effective_zoom(map_state.zoom_level).to_u8()
    } else {
        map_state.zoom_level.to_u8()
    };

    if !view3d_state.is_3d_active() {
        prev_zoom.0 = current;
        return;
    }

    if current == prev_zoom.0 {
        return;
    }

    let zoom_diff = current as i32 - prev_zoom.0 as i32;
    let factor = 2.0_f32.powi(zoom_diff);

    let mut count = 0u32;
    for mut transform in tile_query.iter_mut() {
        if view3d_state.is_3d_active() {
            // Y-up: scale x and z (map axes), leave y (height) alone
            transform.translation.x *= factor;
            transform.translation.z *= factor;
            transform.scale.x *= factor;
            transform.scale.z *= factor;
        } else {
            transform.translation.x *= factor;
            transform.translation.y *= factor;
            transform.scale.x *= factor;
            transform.scale.y *= factor;
        }
        count += 1;
    }

    // Rescale tile_grid position keys so the dedup check correctly detects
    // existing tiles at their new coordinates and doesn't spawn duplicates.
    let old_entries: Vec<_> = tile_grid.occupied.drain().collect();
    for ((x, y, z), entity) in old_entries {
        let new_x = ((x as f64) * factor as f64).round() as i32;
        let new_y = ((y as f64) * factor as f64).round() as i32;
        tile_grid.occupied.insert((new_x, new_y, z), entity);
    }

    debug!(
        "Rescaled {} tiles by {}x for zoom {} -> {}",
        count, factor, prev_zoom.0, current
    );
    prev_zoom.0 = current;
}

/// Primary tile loading system. Runs every frame. Checks disk cache directly,
/// spawns cached tiles immediately (zero latency), and requests network
/// downloads for uncached tiles.
///
/// In 2D: single band at current zoom.
/// In 3D: current zoom + lower-zoom bands centered on camera for coverage.
/// Directional tile downloads are handled separately by
/// `request_3d_directional_downloads` which fills the cache; this system
/// picks up newly-cached tiles on the next frame.
fn load_visible_tiles(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    tile_settings: Res<TileRenderSettings>,
    dl_settings: Res<super::download::TileDownloadSettings>,
    map_state: Res<MapState>,
    mut tile_asset_cache: ResMut<TileAssetCache>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    quad_mesh: Option<Res<TileQuadMesh>>,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    mut download_events: MessageWriter<DownloadTilesRequest>,
    view3d_state: Res<view3d::View3DState>,
    basemap_state: Res<crate::config::CurrentBasemapState>,
    window_query: Query<&Window>,
    zoom_state: Res<crate::map::ZoomState>,
    time: Res<Time<Real>>,
) {
    let Some(ref quad_mesh) = quad_mesh else { return };
    let current_zoom = map_state.zoom_level.to_u8();
    let now = time.elapsed_secs_f64();
    let is_3d = view3d_state.is_3d_active();

    let radius = if let Ok(window) = window_query.single() {
        compute_tile_radius(
            window.width(), window.height(),
            zoom_state.camera_zoom, Some(&view3d_state),
        )
    } else {
        constants::TILE_DOWNLOAD_RADIUS
    };

    let cache_dir = crate::tile_cache::tile_cache_dir_for_style(&dl_settings.cache_key);
    let flat_cache_dir = crate::tile_cache::tile_cache_dir();
    let ext = dl_settings.tile_format.extension();
    let tile_px = constants::DEFAULT_TILE_SIZE.to_pixels();

    let boost = if is_3d { super::TILE_EMISSIVE_BOOST } else { 1.0 };
    let tile_z = if is_3d {
        view3d_state.altitude_to_z(view3d_state.ground_elevation_ft)
    } else {
        tile_settings.z_layer + 0.1
    };
    let [pr, pg, pb] = basemap_state.style.placeholder_color();
    // Plane3d mesh is in XZ plane (flat). For 2D mode, rotate to XY plane.
    let upright_rot = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);

    let lat = map_state.latitude;
    let lon = map_state.longitude;

    struct TileBand {
        lat: f64,
        lon: f64,
        zoom: u8,
        radius: u8,
    }

    let mut bands = Vec::with_capacity(8);
    bands.push(TileBand { lat, lon, zoom: current_zoom, radius });

    if is_3d {
        bands[0].radius = radius.max(10);

        for zoom_offset in 1..=3u8 {
            if current_zoom >= zoom_offset {
                bands.push(TileBand {
                    lat, lon,
                    zoom: current_zoom - zoom_offset,
                    radius: 10,
                });
            }
        }
    }

    let mut uncached_requested = false;

    for band in &bands {
        let Ok(band_zoom) = ZoomLevel::try_from(band.zoom) else { continue };
        let center = super::coords::SlippyTileCoordinates::from_latitude_longitude(
            band.lat, band.lon, band_zoom,
        );

        let render_zoom = if is_3d {
            view3d_state.effective_zoom(map_state.zoom_level)
        } else {
            map_state.zoom_level
        };
        let reference_point = LatitudeLongitudeCoordinates {
            latitude: tile_settings.reference_latitude,
            longitude: tile_settings.reference_longitude,
        };

        let r = band.radius as i64;
        let max_tile = 1i64 << band.zoom;

        let zoom_diff = render_zoom.to_u8().saturating_sub(band.zoom) as u32;
        let rescale = if zoom_diff > 0 {
            (1u32 << zoom_diff) as f32
        } else {
            1.0
        };

        let ref_at_band = world_coords_to_world_pixel(
            &reference_point, constants::DEFAULT_TILE_SIZE, band_zoom,
        );

        for dx in -r..=r {
            for dy in -r..=r {
                let raw_x = center.x as i64 + dx;
                let y = center.y as i64 + dy;
                if y < 0 || y >= max_tile { continue; }
                let x = super::coords::wrap_tile_x(raw_x, band.zoom);

                let tc = super::coords::SlippyTileCoordinates { x, y: y as u32 };
                let ll = tc.to_latitude_longitude(band_zoom);
                let (tile_x, tile_y) = world_coords_to_world_pixel(
                    &ll, constants::DEFAULT_TILE_SIZE, band_zoom,
                );
                let half = tile_px as f64 / 2.0;
                let tx = (((tile_x + half) - ref_at_band.0) * rescale as f64) as f32;
                let ty = (((tile_y - half) - ref_at_band.1) * rescale as f64) as f32;

                let tile_key = (tx as i32, ty as i32, band.zoom);
                if tile_grid.occupied.contains_key(&tile_key) {
                    continue;
                }

                let filename = format!(
                    "{}.{}.{}.{}.tile.{}", band.zoom, x, y as u32, tile_px, ext
                );
                let style_path = cache_dir.join(&filename);
                let flat_path = flat_cache_dir.join(&filename);

                let cached = if style_path.exists() {
                    true
                } else if flat_path.exists() {
                    if let Some(p) = style_path.parent() {
                        let _ = std::fs::create_dir_all(p);
                    }
                    let _ = std::fs::copy(&flat_path, &style_path);
                    true
                } else {
                    false
                };

                if !cached {
                    if !uncached_requested {
                        uncached_requested = true;
                        download_events.write(DownloadTilesRequest {
                            latitude: lat,
                            longitude: lon,
                            zoom: map_state.zoom_level.to_u8(),
                            radius: Radius(radius),
                            priority: DownloadPriority::Near,
                            use_cache: true,
                        });
                    }
                    continue;
                }

                let asset_path = format!("tiles/{}/{}", dl_settings.cache_key, filename);
                let tile_handle: Handle<Image> =
                    if let Some(h) = tile_asset_cache.entries.get(&asset_path) {
                        h.clone()
                    } else {
                        let h: Handle<Image> = asset_server.load(&asset_path);
                        tile_asset_cache.entries.insert(asset_path, h.clone());
                        h
                    };

                let material = materials.add(StandardMaterial {
                    base_color: Color::srgb(pr, pg, pb),
                    emissive: bevy::color::LinearRgba::new(boost, boost, boost, 1.0),
                    emissive_texture: Some(tile_handle.clone()),
                    emissive_exposure_weight: 0.0,
                    perceptual_roughness: 1.0,
                    metallic: 0.0,
                    alpha_mode: AlphaMode::Opaque,
                    ..default()
                });

                let (tile_pos, tile_rot, tile_sc) = if is_3d {
                    // Plane3d is already in XZ plane - no rotation needed
                    (
                        Vec3::new(tx, tile_z, -ty),
                        Quat::IDENTITY,
                        Vec3::new(rescale, 1.0, rescale),
                    )
                } else {
                    // Rotate from XZ to XY for 2D orthographic view
                    (
                        Vec3::new(tx, ty, tile_z),
                        upright_rot,
                        Vec3::ONE,
                    )
                };

                let entity = commands.spawn((
                    Name::new(format!("Map Tile z{}", band.zoom)),
                    Mesh3d(quad_mesh.0.clone()),
                    MeshMaterial3d(material),
                    Transform::from_translation(tile_pos)
                        .with_rotation(tile_rot)
                        .with_scale(tile_sc),
                    Visibility::Inherited,
                    TileOriginalImage(tile_handle),
                    MapTile,
                    TileFadeState {
                        alpha: 1.0,
                        tile_zoom: band.zoom,
                        spawn_time: now,
                    },
                    Pickable::IGNORE,
                    RenderLayers::layer(crate::RenderCategory::TILES),
                )).id();

                tile_grid.occupied.insert(tile_key, entity);
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
            return 1500;
        }
    }
    200 // 2D limit: a typical screen shows ~50-80 tiles at one zoom level
}

/// Despawn tile entities that are far outside the visible viewport.
/// Without this, tiles accumulate indefinitely as the user pans, causing
/// frame time to grow continuously until the app becomes unresponsive.
fn cull_offscreen_tiles(
    mut commands: Commands,
    camera_query: Query<(&Transform, &Projection), With<MapCamera>>,
    tile_query: Query<(Entity, &Transform, &TileFadeState), With<MapTile>>,
    window_query: Query<&Window>,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    view3d_state: Res<view3d::View3DState>,
    alt_tracker: Res<AltitudeChangeTracker>,
) {
    // In 3D mode, tiles use Y-up coordinates but MapCamera uses Z-up.
    // Skip viewport culling in 3D - tile count is managed by the entity
    // budget and animate_tile_fades handles zoom-level cleanup.
    if view3d_state.is_3d_active() {
        return;
    }

    let Ok((camera_tf, projection)) = camera_query.single() else {
        return;
    };
    let Ok(window) = window_query.single() else {
        return;
    };

    // MapCamera (Camera2d) position is in Z-up map space (x, y, z).
    // Tile meshes are in Y-up space (x, height, -z_map). Convert camera
    // position to tile coordinate space for distance comparison.
    let cam_x = camera_tf.translation.x;
    let cam_y = camera_tf.translation.y;

    // Compute culling extents depending on mode
    let (half_w, half_h, forward_bias_x, forward_bias_y) = if view3d_state.is_3d_active() {
        // 3D mode: compute ground footprint from frustum geometry
        let fov = 60.0_f32.to_radians();
        let aspect = window.width() / window.height();
        let half_vfov = fov / 2.0;
        let half_hfov = (aspect * half_vfov.tan()).atan();
        let pitch_rad = view3d_state.camera_pitch.to_radians();
        let effective_distance = view3d_state.altitude_to_distance();
        let camera_height = effective_distance * pitch_rad.sin();

        let far_angle = (pitch_rad - half_vfov).max(0.05);
        let far_ground_dist = camera_height / far_angle.tan();
        let center_ground_dist = effective_distance * pitch_rad.cos();
        let half_width_at_horizon = far_ground_dist * half_hfov.tan();

        // Widen margin during active altitude changes so tiles survive
        // long enough for replacements to load.  Cooldown of ~0.5s.
        // Base margin 3.5x covers the full perspective view to the horizon.
        let margin = if alt_tracker.idle_secs < 0.5 {
            5.0
        } else {
            3.5
        };
        let hw = half_width_at_horizon * margin;
        let hh = far_ground_dist.max(center_ground_dist) * margin;

        // Directional bias: extend forward culling margin by 1.5x, reduce backward to 1.0x
        let yaw_rad = view3d_state.camera_yaw.to_radians();
        let bias_magnitude = far_ground_dist * 0.25;
        let bias_x = bias_magnitude * yaw_rad.sin();
        let bias_y = bias_magnitude * yaw_rad.cos();

        (hw, hh, bias_x, bias_y)
    } else {
        // 2D mode: orthographic viewport extents
        let ortho_scale = if let Projection::Orthographic(ref ortho) = projection {
            ortho.scale
        } else {
            1.0
        };
        let margin = 1.5;
        let hw = (window.width() / 2.0) * ortho_scale * margin;
        let hh = (window.height() / 2.0) * ortho_scale * margin;
        (hw, hh, 0.0, 0.0)
    };

    // Effective center shifted by forward bias (tiles ahead of camera get extra margin)
    let center_x = cam_x + forward_bias_x;
    let center_y = cam_y + forward_bias_y;

    let mut tiles: Vec<(Entity, f32, i32, i32, u8)> = tile_query
        .iter()
        .map(|(entity, tile_tf, fade_state)| {
            let dx = (tile_tf.translation.x - cam_x).abs();
            let dy = (tile_tf.translation.y - cam_y).abs();
            let dist = dx.max(dy);
            (
                entity,
                dist,
                tile_tf.translation.x as i32,
                tile_tf.translation.y as i32,
                fade_state.tile_zoom,
            )
        })
        .collect();

    let mut culled = 0u32;

    // First pass: cull tiles outside the viewport margin (using biased center)
    tiles.retain(|&(entity, _, tx, ty, zoom)| {
        let dx = (tx as f32 - center_x).abs();
        let dy = (ty as f32 - center_y).abs();
        if dx > half_w || dy > half_h {
            tile_grid.occupied.remove(&(tx, ty, zoom));
            commands.entity(entity).despawn();
            culled += 1;
            false
        } else {
            true
        }
    });

    // Second pass: if still over budget, cull farthest tiles.
    let base_limit = max_tile_entities(Some(&view3d_state));
    let tile_limit = if view3d_state.is_3d_active() && alt_tracker.idle_secs < 0.5 {
        base_limit + 200
    } else {
        base_limit
    };
    if tiles.len() > tile_limit {
        tiles.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for &(entity, _, tx, ty, zoom) in &tiles[..tiles.len() - tile_limit] {
            tile_grid.occupied.remove(&(tx, ty, zoom));
            commands.entity(entity).despawn();
            culled += 1;
        }
    }

    if culled > 0 {
        debug!(
            "Culled {} tiles (remaining: {})",
            culled,
            tiles.len().min(tile_limit)
        );
    }
}

/// Show tiles once their texture is loaded. Despawn old-zoom tiles only
/// when their grid cell is fully covered by a loaded new-zoom tile.
/// No timeout-based removal - old tiles stay visible until replaced.
fn animate_tile_fades(
    mut commands: Commands,
    map_state: Res<MapState>,
    images: Res<Assets<Image>>,
    mut tile_query: Query<
        (Entity, &mut TileFadeState, &Transform, &TileOriginalImage),
        With<MapTile>,
    >,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    view3d_state: Res<view3d::View3DState>,
) {
    let current_zoom = map_state.zoom_level.to_u8();
    let is_3d = view3d_state.is_3d_active();

    // Track grid cells that have a fully-loaded tile at the current zoom.
    let mut loaded_cells: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let mut old_tiles: Vec<(Entity, i32, i32, u8)> = Vec::new();

    for (entity, mut fade_state, transform, original) in tile_query.iter_mut() {
        let dominated = if is_3d {
            fade_state.tile_zoom > current_zoom
                || current_zoom.saturating_sub(fade_state.tile_zoom) > 4
        } else {
            fade_state.tile_zoom != current_zoom
        };

        if !dominated {
            if images.contains(&original.0) {
                fade_state.alpha = 1.0;
                let map_y = if is_3d { -transform.translation.z } else { transform.translation.y };
                let cell = (
                    (transform.translation.x / super::DEFAULT_TILE_PIXELS).round() as i32,
                    (map_y / super::DEFAULT_TILE_PIXELS).round() as i32,
                );
                loaded_cells.insert(cell);
            }
        } else {
            let map_y = if is_3d { -transform.translation.z } else { transform.translation.y };
            let cell = (
                (transform.translation.x / super::DEFAULT_TILE_PIXELS).round() as i32,
                (map_y / super::DEFAULT_TILE_PIXELS).round() as i32,
            );
            old_tiles.push((entity, cell.0, cell.1, fade_state.tile_zoom));
        }
    }

    // Only despawn old-zoom tiles when their cell is covered by a loaded new tile.
    for (entity, cx, cy, zoom) in old_tiles {
        if loaded_cells.contains(&(cx, cy)) {
            let tx = (cx as f32 * super::DEFAULT_TILE_PIXELS) as i32;
            let ty = (cy as f32 * super::DEFAULT_TILE_PIXELS) as i32;
            tile_grid.occupied.remove(&(tx, ty, zoom));
            commands.entity(entity).despawn();
        }
    }

}

// =============================================================================
// 3D Mesh Quad Systems
// =============================================================================

/// Create the shared mesh used by all tile 3D quads.
fn setup_tile_quad_mesh(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    // Slightly oversized (0.5px overlap) to prevent sub-pixel gaps between
    // adjacent tiles from showing the background through at grazing angles.
    let overlap = 0.5;
    let size = constants::DEFAULT_TILE_PIXELS + overlap;
    // Use Plane3d (XZ plane, normal=Y) so tiles lie flat without rotation.
    let mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::new(size / 2.0, size / 2.0)));
    commands.insert_resource(TileQuadMesh(mesh));
}

/// When view mode changes, despawn all tiles and clear tracking so
/// load_visible_tiles respawns them fresh in the correct coordinate system.
fn orient_tiles_for_view_mode(
    mut commands: Commands,
    view3d_state: Res<view3d::View3DState>,
    tile_query: Query<Entity, With<MapTile>>,
    mut tile_grid: ResMut<super::pool::TileGrid>,
    mut last_3d: Local<Option<bool>>,
) {
    let is_3d = view3d_state.is_3d_active();
    if *last_3d == Some(is_3d) {
        return;
    }
    *last_3d = Some(is_3d);

    for entity in tile_query.iter() {
        commands.entity(entity).despawn();
    }
    tile_grid.occupied.clear();
}

// The following 5 systems were removed in the unified mesh redesign:
// - sync_tile_mesh_quads (was: spawn companion Mesh3d entities for Sprites)
// - sync_tile_mesh_alpha (was: sync companion visibility with sprite fade)
// - sync_tile_mesh_transforms (was: copy sprite position to companion mesh)
// - hide_tile_sprites_in_3d (was: zero sprite alpha so Camera2d doesn't show tiles)
// - cleanup_orphaned_tile_quads (was: despawn orphaned companions from deferred command races)
// Tiles are now unified Mesh3d entities rendered by Camera3d in both modes.
