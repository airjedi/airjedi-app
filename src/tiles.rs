use bevy::prelude::*;
use bevy::asset::AssetLoadFailedEvent;
use bevy::image::Image;
use bevy::pbr::StandardMaterial;
use bevy_slippy_tiles::*;

use crate::constants;
use crate::map::{MapState, ZoomState};
use crate::tile_cache;
use crate::view3d;
use crate::camera::MapCamera;
use crate::RenderCategory;
use crate::{clamp_latitude, clamp_longitude, ZoomDebugLogger, ZoomSet};
use bevy::camera::visibility::RenderLayers;

// =============================================================================
// Components and Resources
// =============================================================================

/// Component to track tile fade state for smooth zoom transitions.
#[derive(Component)]
pub(crate) struct TileFadeState {
    pub(crate) alpha: f32,
    /// The zoom level this tile was spawned for
    pub(crate) tile_zoom: u8,
    /// When this tile was spawned (seconds since startup)
    pub(crate) spawn_time: f64,
}

/// Links a tile entity to its 3D mesh quad companion (used in 3D mode only).
#[derive(Component)]
pub(crate) struct TileMeshQuad(pub Entity);

/// Marker on 3D mesh quad companion entities so orphans can be detected.
#[derive(Component)]
pub(crate) struct TileQuad3d;

/// Shared mesh handle for all 3D tile quads (sized to match DEFAULT_TILE_PIXELS).
#[derive(Resource)]
pub(crate) struct TileQuadMesh(pub Handle<Mesh>);

/// Tracks which tile positions have been spawned to prevent duplicate entities.
/// Key is (transform_x rounded, transform_y rounded, zoom_level).
#[derive(Resource, Default)]
pub(crate) struct SpawnedTiles {
    pub(crate) positions: std::collections::HashSet<(i32, i32, u8)>,
}

/// Timer that triggers periodic tile re-requests in 3D mode so that camera
/// orbit, pan, and altitude changes continuously fill visible areas.
#[derive(Resource)]
pub(crate) struct Tile3DRefreshTimer(Timer);

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
}

impl Default for AltitudeChangeTracker {
    fn default() -> Self {
        Self {
            prev_altitude: 10000.0,
            idle_secs: f32::MAX,
        }
    }
}

/// Stores the original tile image handle so the grid overlay can be toggled off.
#[derive(Component)]
pub(crate) struct TileOriginalImage(pub Handle<Image>);

/// Controls whether tiles display a procedural grid instead of their imagery.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub(crate) struct GridOverlay {
    pub enabled: bool,
    #[reflect(ignore)]
    pub texture: Handle<Image>,
}

/// Fixed emissive boost for 3D tile materials. With emissive_exposure_weight=0.0
/// the emissive bypasses both per-material and global exposure in the tonemapping
/// pass, so a fixed multiplier gives consistent brightness at any EV.
const TILE_EMISSIVE_BOOST: f32 = 8.0;

// =============================================================================
// Plugin
// =============================================================================

pub(crate) struct TilesPlugin;

impl Plugin for TilesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnedTiles>()
            .init_resource::<Tile3DRefreshTimer>()
            .init_resource::<AltitudeChangeTracker>()
            .init_resource::<Previous3DZoom>()
            .register_type::<GridOverlay>()
            .add_systems(Startup, (setup_tile_quad_mesh, setup_grid_overlay))
            .add_systems(Update, toggle_grid_overlay)
            .add_systems(Update, handle_basemap_change)
            .add_systems(Update, handle_window_resize)
            .add_systems(Update, handle_3d_view_tile_refresh)
            .add_systems(Update, request_3d_tiles_continuous
                .after(handle_3d_view_tile_refresh)
                .in_set(ZoomSet::Change))
            .add_systems(Update, rescale_tiles_on_zoom_change
                .after(ZoomSet::Change))
            .add_systems(Update, track_altitude_changes)
            .add_systems(Update, handle_tile_load_failures)
            .add_systems(Update, display_tiles_filtered
                .after(bevy::ecs::schedule::ApplyDeferred)
                .after(rescale_tiles_on_zoom_change))
            .add_systems(Update, animate_tile_fades.after(display_tiles_filtered))
            .add_systems(Update, cull_offscreen_tiles.after(display_tiles_filtered))
            .add_systems(Update, sync_tile_mesh_quads.after(animate_tile_fades))
            .add_systems(Update, sync_tile_mesh_alpha.after(sync_tile_mesh_quads))
            .add_systems(Update, sync_tile_mesh_transforms.after(sync_tile_mesh_quads))
            .add_systems(Update, hide_tile_sprites_in_3d.after(sync_tile_mesh_quads))
            .add_systems(Update, cleanup_orphaned_tile_quads.after(sync_tile_mesh_quads));
    }
}

// =============================================================================
// Grid Overlay
// =============================================================================

/// Generate a procedural grid texture (512x512) with a dark background and
/// lighter grid lines showing tile subdivisions.
fn setup_grid_overlay(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let size = constants::DEFAULT_TILE_PIXELS as u32; // 512
    let mut data = vec![0u8; (size * size * 4) as usize];

    let bg = [40u8, 44, 52, 255];         // dark charcoal
    let line_major = [100u8, 110, 130, 255]; // lighter for outer border
    let line_minor = [70u8, 78, 90, 255];    // subtle inner grid

    let subdivisions = 4u32; // 4x4 inner grid
    let cell = size / subdivisions;

    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let on_border = x == 0 || x == size - 1 || y == 0 || y == size - 1
                || x == 1 || y == 1;
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

/// When `GridOverlay.enabled` changes, swap all tile sprite images between
/// the grid texture and their original imagery. Also force 3D mesh quad
/// recreation by stripping `TileMeshQuad` and despawning companions.
fn toggle_grid_overlay(
    mut commands: Commands,
    grid: Res<GridOverlay>,
    mut tiles: Query<(Entity, &mut Sprite, &TileOriginalImage, Option<&TileMeshQuad>), With<MapTile>>,
    quad_entities: Query<Entity, With<TileQuad3d>>,
) {
    if !grid.is_changed() {
        return;
    }

    for (entity, mut sprite, original, mesh_quad) in tiles.iter_mut() {
        if grid.enabled {
            sprite.image = grid.texture.clone();
        } else {
            sprite.image = original.0.clone();
        }
        // Force mesh quad recreation so the 3D material picks up the new texture
        if let Some(quad) = mesh_quad {
            commands.entity(quad.0).despawn();
            commands.entity(entity).remove::<TileMeshQuad>();
        }
    }

    if grid.enabled {
        // Also despawn any orphaned quads
        for quad_entity in quad_entities.iter() {
            commands.entity(quad_entity).despawn();
        }
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
pub(crate) fn altitude_to_zoom_level(altitude_ft: f32, current_zoom: u8) -> u8 {
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
pub(crate) fn compute_tile_radius(
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
            let tiles_needed = (max_ground_extent / tile_world_size).ceil() as u8;
            return tiles_needed.clamp(3, 12);
        }
    }

    // 2D orthographic mode
    let tile_screen_px = constants::DEFAULT_TILE_PIXELS * camera_zoom;
    let half_tiles_x = (window_width / (2.0 * tile_screen_px)).ceil() as u8;
    let half_tiles_y = (window_height / (2.0 * tile_screen_px)).ceil() as u8;
    half_tiles_x.max(half_tiles_y).clamp(3, 8)
}

/// Send a tile download request for the current map location.
pub(crate) fn request_tiles_at_location(
    download_events: &mut MessageWriter<DownloadSlippyTilesMessage>,
    latitude: f64,
    longitude: f64,
    zoom_level: ZoomLevel,
    use_cache: bool,
) {
    download_events.write(DownloadSlippyTilesMessage {
        tile_size: constants::DEFAULT_TILE_SIZE,
        zoom_level,
        coordinates: Coordinates::from_latitude_longitude(latitude, longitude),
        radius: Radius(constants::TILE_DOWNLOAD_RADIUS),
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
    tile_query: Query<(Entity, Option<&TileMeshQuad>), With<MapTile>>,
    mut spawned_tiles: ResMut<SpawnedTiles>,
    mut download_status: ResMut<SlippyTileDownloadStatus>,
    mut download_events: MessageWriter<DownloadSlippyTilesMessage>,
    map_state: Res<MapState>,
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

    info!("Basemap changed to {:?} - clearing all tile entities", current);

    for (entity, mesh_quad) in tile_query.iter() {
        if let Some(quad) = mesh_quad {
            commands.entity(quad.0).despawn();
        }
        commands.entity(entity).despawn();
    }

    spawned_tiles.positions.clear();
    download_status.0.clear();

    request_tiles_at_location(
        &mut download_events,
        map_state.latitude,
        map_state.longitude,
        map_state.zoom_level,
        true,
    );
}

/// When a tile image fails to load, check if the cached file is corrupt and remove it.
/// The tile will be re-requested automatically by bevy_slippy_tiles on the next frame.
fn handle_tile_load_failures(
    mut commands: Commands,
    mut failed_events: MessageReader<AssetLoadFailedEvent<Image>>,
    tile_query: Query<(Entity, &Sprite, &Transform, Option<&TileMeshQuad>, &TileFadeState), With<MapTile>>,
    mut spawned_tiles: ResMut<SpawnedTiles>,
) {
    for event in failed_events.read() {
        let asset_path = event.path.path();
        // Only handle tile files
        let path_str = asset_path.to_string_lossy();
        if path_str.contains(".tile.") {
            warn!("Tile asset load failed: {:?} — checking for corrupt cache file", asset_path);
            tile_cache::remove_corrupt_cached_tile(asset_path);

            // Despawn the tile entity (and its mesh quad) that references
            // the failed texture so it doesn't render as a black rectangle.
            // Clearing its spawned_tiles entry allows re-download.
            let failed_id = event.id;
            for (entity, sprite, transform, mesh_quad, fade) in tile_query.iter() {
                if sprite.image.id() == failed_id {
                    if let Some(quad) = mesh_quad {
                        commands.entity(quad.0).despawn();
                    }
                    let key = (transform.translation.x as i32, transform.translation.y as i32, fade.tile_zoom);
                    spawned_tiles.positions.remove(&key);
                    commands.entity(entity).despawn();
                    debug!("Despawned tile entity with failed texture: {:?}", asset_path);
                    break;
                }
            }
        }
    }
}

/// Request tiles when the window is resized or maximized so newly exposed areas are filled.
fn handle_window_resize(
    mut resize_events: MessageReader<bevy::window::WindowResized>,
    mut download_events: MessageWriter<DownloadSlippyTilesMessage>,
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
        download_events.write(DownloadSlippyTilesMessage {
            tile_size: constants::DEFAULT_TILE_SIZE,
            zoom_level: map_state.zoom_level,
            coordinates: Coordinates::from_latitude_longitude(map_state.latitude, map_state.longitude),
            radius: Radius(radius),
            use_cache: true,
        });
    }
}

/// Re-request tiles when 3D view state changes (entering/exiting 3D, orbit, pitch, distance)
/// so the larger perspective footprint is covered.
/// When returning to 2D, clears spawned tile tracking so tiles are freshly re-displayed.
fn handle_3d_view_tile_refresh(
    view3d_state: Res<view3d::View3DState>,
    mut download_events: MessageWriter<DownloadSlippyTilesMessage>,
    mut map_state: ResMut<MapState>,
    zoom_state: Res<ZoomState>,
    window_query: Query<&Window>,
    mut spawned_tiles: ResMut<SpawnedTiles>,
) {
    if !view3d_state.is_changed() {
        return;
    }

    // When we've just returned to 2D mode, clear the spawned tiles tracker
    // and restore the saved 2D zoom level.
    // 3D mode uses multi-resolution tiles at different zoom levels and scales;
    // without clearing, the dedup check in display_tiles_filtered would skip
    // re-spawning tiles at the current zoom level, leaving a blank map.
    if matches!(view3d_state.mode, view3d::ViewMode::Map2D) && !view3d_state.is_transitioning() {
        spawned_tiles.positions.clear();
        // Restore the 2D zoom level that was saved when entering 3D mode
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
    download_events.write(DownloadSlippyTilesMessage {
        tile_size: constants::DEFAULT_TILE_SIZE,
        zoom_level: map_state.zoom_level,
        coordinates: Coordinates::from_latitude_longitude(map_state.latitude, map_state.longitude),
        radius: Radius(radius),
        use_cache: true,
    });
}

/// Continuously request multi-resolution tiles in 3D mode so that camera orbit,
/// pan, and altitude changes fill the visible area without waiting for explicit
/// View3DState change events.
///
/// Three distance bands load tiles at decreasing zoom levels:
/// - Near (0-40% of ground extent): current zoom_level
/// - Mid  (40-70%): zoom_level - 1
/// - Far  (70-100%): zoom_level - 2
///
/// Mid and far bands are offset in the camera look direction so tiles load ahead
/// of where the user is looking. Band radii adapt to pitch: low pitch (looking
/// toward horizon) favours far tiles; high pitch (looking down) favours near tiles.
fn request_3d_tiles_continuous(
    mut commands: Commands,
    mut timer: ResMut<Tile3DRefreshTimer>,
    time: Res<Time>,
    mut view3d_state: ResMut<view3d::View3DState>,
    mut map_state: ResMut<MapState>,
    tile_settings: Res<SlippyTilesSettings>,
    mut download_events: MessageWriter<DownloadSlippyTilesMessage>,
    mut spawned_tiles: ResMut<SpawnedTiles>,
    tile_query: Query<(Entity, &TileFadeState, Option<&TileMeshQuad>), With<MapTile>>,
) {
    if !view3d_state.is_3d_active() {
        return;
    }

    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }

    // Compute zoom level from camera altitude so that flying close to the
    // ground automatically loads higher-detail tiles.
    let old_zoom = map_state.zoom_level.to_u8();
    let adaptive_zoom = altitude_to_zoom_level(view3d_state.camera_altitude, old_zoom);
    if let Ok(new_zoom) = ZoomLevel::try_from(adaptive_zoom) {
        if map_state.zoom_level != new_zoom {
            debug!("3D adaptive zoom: altitude {:.0} ft -> zoom {}", view3d_state.camera_altitude, adaptive_zoom);
            map_state.zoom_level = new_zoom;

            // Recalculate the camera orbit center (saved_2d_center) in the
            // new zoom level's pixel space. Without this, the camera center
            // stays in the old zoom's coordinates while all entity positions
            // recalculate at the new zoom, causing everything to jump.
            let converter = crate::geo::CoordinateConverter::new(
                &tile_settings, new_zoom,
            );
            let new_center = converter.latlon_to_world(
                map_state.latitude, map_state.longitude,
            );
            view3d_state.saved_2d_center = new_center;

            // When zoom level changes, despawn tiles that are now outside
            // the new multi-resolution band [new_zoom-4, new_zoom].
            // Don't clear spawned_tiles — existing in-band tiles should
            // remain to prevent flashing gaps during transitions.
            let new_z = new_zoom.to_u8();
            let min_band = new_z.saturating_sub(4);
            spawned_tiles.positions.retain(|&(_, _, z)| z >= min_band && z <= new_z);
            let mut despawned = 0u32;
            for (entity, fade_state, mesh_quad) in tile_query.iter() {
                if fade_state.tile_zoom > new_z || fade_state.tile_zoom < min_band {
                    // Despawn the mesh quad companion first to prevent orphans
                    if let Some(quad) = mesh_quad {
                        commands.entity(quad.0).despawn();
                    }
                    commands.entity(entity).despawn();
                    despawned += 1;
                }
            }
            if despawned > 0 {
                debug!("Zoom changed {}->{}: despawned {} out-of-band tiles", old_zoom, new_z, despawned);
            }
        }
    }

    let base_zoom = map_state.zoom_level.to_u8();
    let lat = map_state.latitude;
    let lon = map_state.longitude;
    let yaw_rad = view3d_state.camera_yaw.to_radians();
    let pitch = view3d_state.camera_pitch;

    // pitch_factor: 0.0 = low pitch (horizon), 1.0 = high pitch (looking down)
    let pitch_factor = ((pitch - 15.0) / (89.0 - 15.0)).clamp(0.0, 1.0);

    // Adaptive band radii based on pitch
    let near_radius = 3 + (3.0 * pitch_factor) as u8;          // 3-6
    let mid_radius  = 3 + (2.0 * (1.0 - pitch_factor)) as u8;  // 3-5
    let far_radius  = 2 + (3.0 * (1.0 - pitch_factor)) as u8;  // 2-5

    // --- Near band: current zoom level, centered on map position ---
    download_events.write(DownloadSlippyTilesMessage {
        tile_size: constants::DEFAULT_TILE_SIZE,
        zoom_level: map_state.zoom_level,
        coordinates: Coordinates::from_latitude_longitude(lat, lon),
        radius: Radius(near_radius),
        use_cache: true,
    });

    // Helper: send a tile request at a given zoom level, offset from (lat, lon)
    // by `fwd` tiles forward and `side` tiles sideways relative to the camera yaw.
    let mut request_band = |zoom_offset: u8, fwd: f64, side: f64, radius: u8| {
        if base_zoom < zoom_offset {
            return;
        }
        let z = base_zoom - zoom_offset;
        let Ok(zoom) = ZoomLevel::try_from(z) else {
            return;
        };
        let deg_per_tile_lon = 360.0 / (1u64 << z) as f64;
        let deg_per_tile_lat = deg_per_tile_lon * lat.to_radians().cos();
        // Forward = along yaw, sideways = perpendicular (yaw + 90°)
        let offset_lat = fwd * deg_per_tile_lat * yaw_rad.cos() as f64
            - side * deg_per_tile_lat * yaw_rad.sin() as f64;
        let offset_lon = fwd * deg_per_tile_lon * yaw_rad.sin() as f64
            + side * deg_per_tile_lon * yaw_rad.cos() as f64;
        download_events.write(DownloadSlippyTilesMessage {
            tile_size: constants::DEFAULT_TILE_SIZE,
            zoom_level: zoom,
            coordinates: Coordinates::from_latitude_longitude(
                clamp_latitude(lat + offset_lat),
                clamp_longitude(lon + offset_lon),
            ),
            radius: Radius(radius),
            use_cache: true,
        });
    };

    // --- Mid band: zoom_level - 1 ---
    request_band(1, 3.0, 0.0, mid_radius);
    request_band(1, 2.0, -4.0, mid_radius);
    request_band(1, 2.0, 4.0, mid_radius);

    // --- Far band: zoom_level - 2 ---
    request_band(2, 4.0, 0.0, far_radius);
    request_band(2, 3.0, -5.0, far_radius);
    request_band(2, 3.0, 5.0, far_radius);

    // --- Horizon bands: zoom_level - 3 and - 4 ---
    let hr = 4 + (3.0 * (1.0 - pitch_factor)) as u8; // 4-7

    // zoom-3: sweep at multiple forward distances
    for &fwd in &[2.0, 5.0, 8.0] {
        request_band(3, fwd, 0.0, hr);
        let spread = fwd * 1.5 + 4.0;
        request_band(3, fwd, -spread, hr);
        request_band(3, fwd, spread, hr);
    }

    // zoom-4: coarser tiles for the far horizon, even wider sweep
    let ur = 4 + (2.0 * (1.0 - pitch_factor)) as u8; // 4-6
    for &fwd in &[2.0, 5.0, 8.0] {
        request_band(4, fwd, 0.0, ur);
        let spread = fwd * 2.0 + 5.0;
        request_band(4, fwd, -spread, ur);
        request_band(4, fwd, spread, ur);
    }
}

/// Track camera altitude changes to soften tile culling during rapid zoom.
fn track_altitude_changes(
    time: Res<Time>,
    view3d_state: Res<view3d::View3DState>,
    mut tracker: ResMut<AltitudeChangeTracker>,
) {
    let current = view3d_state.camera_altitude;
    let delta = (current - tracker.prev_altitude).abs();
    if delta > 50.0 {
        // Significant altitude change — reset cooldown
        tracker.idle_secs = 0.0;
    } else {
        tracker.idle_secs += time.delta_secs();
    }
    tracker.prev_altitude = current;
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
    mut spawned_tiles: ResMut<SpawnedTiles>,
) {
    let current = map_state.zoom_level.to_u8();

    if !view3d_state.is_3d_active() {
        // Keep tracking zoom even in 2D so transition to 3D starts correct
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
        transform.translation.x *= factor;
        transform.translation.y *= factor;
        // Z is managed by update_tile_elevation, don't touch it
        transform.scale.x *= factor;
        transform.scale.y *= factor;
        count += 1;
    }

    // Rescale spawned_tiles position keys so the dedup check in
    // display_tiles_filtered correctly detects existing tiles at
    // their new coordinates and doesn't spawn duplicates.
    let old_positions: Vec<_> = spawned_tiles.positions.drain().collect();
    for (x, y, z) in old_positions {
        let new_x = ((x as f64) * factor as f64).round() as i32;
        let new_y = ((y as f64) * factor as f64).round() as i32;
        spawned_tiles.positions.insert((new_x, new_y, z));
    }

    debug!("Rescaled {} tiles by {}x for zoom {} -> {}", count, factor, prev_zoom.0, current);
    prev_zoom.0 = current;
}

/// Custom tile display system that filters tiles by current zoom level.
/// When new tiles arrive at the current zoom, old tiles from previous zoom levels
/// are marked for delayed despawn so the screen is never blank.
fn display_tiles_filtered(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    tile_settings: Res<SlippyTilesSettings>,
    map_state: Res<MapState>,
    mut tile_events: MessageReader<SlippyTileDownloadedMessage>,
    mut _tile_query: Query<(Entity, &mut TileFadeState), With<MapTile>>,
    mut spawned_tiles: ResMut<SpawnedTiles>,
    logger: Option<Res<ZoomDebugLogger>>,
    view3d_state: Res<view3d::View3DState>,
    grid: Option<Res<GridOverlay>>,
    basemap_state: Res<crate::config::CurrentBasemapState>,
    time: Res<Time<Real>>,
) {
    let current_zoom = map_state.zoom_level.to_u8();
    let now = time.elapsed_secs_f64();

    // Scale factor to compensate when the tile server returns smaller images
    // than the requested tile size (e.g. ESRI returns 256px for @2x/512 requests).
    let native_px = basemap_state.style.native_tile_pixels() as f32;
    let requested_px = constants::DEFAULT_TILE_SIZE.to_pixels() as f32;
    let tile_scale = requested_px / native_px;

    for event in tile_events.read() {
        // In 3D mode, accept tiles within 4 zoom levels below current (multi-resolution bands).
        // In 2D mode, only accept tiles at the exact current zoom level.
        let event_zoom = event.zoom_level.to_u8();
        if view3d_state.is_3d_active() {
            if event_zoom > current_zoom || current_zoom - event_zoom > 4 {
                continue;
            }
        } else if event.zoom_level != map_state.zoom_level {
            continue;
        }

        // Calculate tile position (same logic as bevy_slippy_tiles display_tiles)
        let reference_point = LatitudeLongitudeCoordinates {
            latitude: tile_settings.reference_latitude,
            longitude: tile_settings.reference_longitude,
        };
        let (ref_x, ref_y) = world_coords_to_world_pixel(
            &reference_point,
            event.tile_size,
            event.zoom_level
        );

        let current_coords = match &event.coordinates {
            Coordinates::LatitudeLongitude(coords) => *coords,
            Coordinates::SlippyTile(coords) => coords.to_latitude_longitude(event.zoom_level),
        };
        let (tile_x, tile_y) = world_coords_to_world_pixel(
            &current_coords,
            event.tile_size,
            event.zoom_level
        );

        let half_tile = event.tile_size.to_pixels() as f64 / 2.0;
        let tile_center_x = tile_x + half_tile;
        let tile_center_y = tile_y - half_tile;

        let mut transform_x = (tile_center_x - ref_x) as f32;
        let mut transform_y = (tile_center_y - ref_y) as f32;

        // In 3D mode, lower-zoom tiles are in a different pixel coordinate
        // system. Rescale their position and size so they align with the
        // current zoom level's world space.
        let zoom_diff = current_zoom.saturating_sub(event_zoom) as u32;
        let rescale = if view3d_state.is_3d_active() && zoom_diff > 0 {
            let s = (1u32 << zoom_diff) as f32; // 2, 4, 8, 16
            transform_x *= s;
            transform_y *= s;
            s
        } else {
            1.0
        };

        // Skip if a tile entity already exists at this position and zoom level
        let tile_key = (transform_x as i32, transform_y as i32, event_zoom);
        if !spawned_tiles.positions.insert(tile_key) {
            continue; // Already spawned
        }

        let tile_path = event.path.clone();
        let tile_handle: Handle<Image> = asset_server.load(tile_path.clone());
        asset_server.reload(tile_path);

        // Use grid texture if the overlay is enabled, otherwise use the tile image
        let display_handle = match &grid {
            Some(g) if g.enabled => g.texture.clone(),
            _ => tile_handle.clone(),
        };

        // Spawn new tiles translucent and slightly above old tiles so they
        // fade in on top, hiding the old zoom level progressively.
        // In 3D mode, spawn at ground elevation so tiles are coplanar with
        // airports, runways, and other ground-level features.
        // Lower-zoom tiles sit slightly below so higher-zoom tiles win depth.
        let tile_z = if view3d_state.is_3d_active() {
            view3d_state.altitude_to_z(view3d_state.ground_elevation_ft)
                - zoom_diff as f32 * 0.05
        } else {
            tile_settings.z_layer + 0.1
        };
        commands.spawn((
            Name::new(format!("Map Tile z{}", event_zoom)),
            Sprite {
                image: display_handle,
                color: Color::srgba(1.0, 1.0, 1.0, 0.0),
                custom_size: if tile_scale != 1.0 {
                    Some(Vec2::splat(requested_px))
                } else {
                    None
                },
                ..default()
            },
            TileOriginalImage(tile_handle),
            Transform::from_xyz(transform_x, transform_y, tile_z)
                .with_scale(Vec3::splat(rescale)),
            MapTile,
            TileFadeState {
                alpha: 0.0,
                tile_zoom: event_zoom,
                spawn_time: now,
            },
            RenderLayers::layer(RenderCategory::TILES_2D),
        ));

        if let Some(ref log) = logger {
            log.log(&format!("TILE DISPLAYED: zoom={} pos=({:.0}, {:.0})",
                current_zoom, transform_x, transform_y));
        }
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
    mut spawned_tiles: ResMut<SpawnedTiles>,
    mut download_status: ResMut<SlippyTileDownloadStatus>,
    view3d_state: Res<view3d::View3DState>,
    alt_tracker: Res<AltitudeChangeTracker>,
) {
    let Ok((camera_tf, projection)) = camera_query.single() else {
        return;
    };
    let Ok(window) = window_query.single() else {
        return;
    };

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
        let margin = if alt_tracker.idle_secs < 0.5 { 5.0 } else { 3.5 };
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

    // Collect all tiles with their distance from camera for sorting
    let mut tiles: Vec<(Entity, f32, i32, i32, u8)> = tile_query
        .iter()
        .map(|(entity, tile_tf, fade_state)| {
            let dx = (tile_tf.translation.x - cam_x).abs();
            let dy = (tile_tf.translation.y - cam_y).abs();
            let dist = dx.max(dy); // Chebyshev distance
            (entity, dist, tile_tf.translation.x as i32, tile_tf.translation.y as i32, fade_state.tile_zoom)
        })
        .collect();

    let mut culled = 0u32;

    // First pass: cull tiles outside the viewport margin (using biased center)
    tiles.retain(|&(entity, _, tx, ty, zoom)| {
        let dx = (tx as f32 - center_x).abs();
        let dy = (ty as f32 - center_y).abs();
        if dx > half_w || dy > half_h {
            spawned_tiles.positions.remove(&(tx, ty, zoom));
            commands.entity(entity).despawn();
            culled += 1;
            false
        } else {
            true
        }
    });

    // Second pass: if still over budget, cull farthest tiles.
    // Raise the limit during active altitude changes to avoid thrashing.
    let base_limit = max_tile_entities(Some(&view3d_state));
    let tile_limit = if view3d_state.is_3d_active() && alt_tracker.idle_secs < 0.5 {
        base_limit + 200
    } else {
        base_limit
    };
    if tiles.len() > tile_limit {
        tiles.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for &(entity, _, tx, ty, zoom) in &tiles[..tiles.len() - tile_limit] {
            spawned_tiles.positions.remove(&(tx, ty, zoom));
            commands.entity(entity).despawn();
            culled += 1;
        }
    }

    if culled > 0 {
        debug!("Culled {} tiles (remaining: {})", culled, tiles.len().min(tile_limit));
    }
}

/// Animate tile fade-in and despawn old tiles only when covered by fully-loaded new tiles.
fn animate_tile_fades(
    mut commands: Commands,
    time: Res<Time>,
    real_time: Res<Time<Real>>,
    map_state: Res<MapState>,
    mut tile_query: Query<(Entity, &mut TileFadeState, &mut Sprite, &Transform), With<MapTile>>,
    mut spawned_tiles: ResMut<SpawnedTiles>,
    mut download_status: ResMut<SlippyTileDownloadStatus>,
    view3d_state: Res<view3d::View3DState>,
) {
    let delta = time.delta_secs();
    let now = real_time.elapsed_secs_f64();
    let current_zoom = map_state.zoom_level.to_u8();

    // In 3D mode, tiles within the multi-resolution band (current_zoom to
    // current_zoom - 4) are intentional and should NOT be treated as "old."
    let is_3d = view3d_state.is_3d_active();

    // Collect grid cells covered by fully-opaque new tiles.
    // Quantize positions to 256px cells so old (rescaled) tiles can be matched.
    let mut loaded_cells: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let mut old_tiles: Vec<(Entity, i32, i32, u8, f64)> = Vec::new();

    for (entity, mut fade_state, mut sprite, transform) in tile_query.iter_mut() {
        let dominated = if is_3d {
            // In 3D mode, tiles within the multi-resolution band
            // (current_zoom to current_zoom - 4) are intentional. Tiles
            // outside this band are stale from a previous zoom level.
            fade_state.tile_zoom > current_zoom
                || current_zoom.saturating_sub(fade_state.tile_zoom) > 4
        } else {
            fade_state.tile_zoom != current_zoom
        };

        if !dominated {
            // Current / active tile: fade in.
            // In 3D mode, use fast fade (appear in ~2 frames) to minimize
            // gaps during zoom transitions while still giving textures a
            // frame to load (prevents Bevy's default magenta showing).
            if fade_state.alpha < 1.0 {
                let speed = if is_3d { 30.0 } else { constants::TILE_FADE_SPEED };
                fade_state.alpha += speed * delta;
                fade_state.alpha = fade_state.alpha.min(1.0);
                sprite.color = Color::srgba(1.0, 1.0, 1.0, fade_state.alpha);
            }
            // Track fully-opaque tiles by grid cell
            if fade_state.alpha >= 1.0 {
                let cell = (
                    (transform.translation.x / constants::DEFAULT_TILE_PIXELS).round() as i32,
                    (transform.translation.y / constants::DEFAULT_TILE_PIXELS).round() as i32,
                );
                loaded_cells.insert(cell);
            }
        } else {
            // Old-zoom tile: record for coverage check
            let cell = (
                (transform.translation.x / constants::DEFAULT_TILE_PIXELS).round() as i32,
                (transform.translation.y / constants::DEFAULT_TILE_PIXELS).round() as i32,
            );
            old_tiles.push((entity, cell.0, cell.1, fade_state.tile_zoom, fade_state.spawn_time));
        }
    }

    // Despawn old-zoom tiles when their grid cell is covered by a fully-loaded
    // new tile. In 2D mode, also expire after 3 seconds as a fallback to prevent
    // accumulation from grid cell misalignment. In 3D mode, skip time-based expiry
    // since lower-zoom tiles provide intentional fallback coverage at distance.
    for (entity, cx, cy, zoom, spawn_time) in old_tiles {
        let covered = loaded_cells.contains(&(cx, cy));
        let expired = !is_3d && (now - spawn_time) > 3.0;
        if covered || expired {
            let tx = (cx as f32 * constants::DEFAULT_TILE_PIXELS) as i32;
            let ty = (cy as f32 * constants::DEFAULT_TILE_PIXELS) as i32;
            spawned_tiles.positions.remove(&(tx, ty, zoom));
            commands.entity(entity).despawn();
        }
    }
}

// =============================================================================
// 3D Mesh Quad Systems
// =============================================================================

/// Create the shared mesh used by all tile 3D quads.
fn setup_tile_quad_mesh(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    let mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(constants::DEFAULT_TILE_PIXELS / 2.0)));
    commands.insert_resource(TileQuadMesh(mesh));
}

/// Spawn or despawn 3D mesh quad companions for tile entities based on view mode.
///
/// In 3D mode: for each tile lacking a `TileMeshQuad`, spawn a `StandardMaterial`
/// mesh entity using the sprite's texture. In 2D mode: despawn all companions.
fn sync_tile_mesh_quads(
    mut commands: Commands,
    view3d_state: Res<view3d::View3DState>,
    map_state: Res<MapState>,
    quad_mesh: Option<Res<TileQuadMesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    tiles_without_quad: Query<(Entity, &Sprite, &Transform, &TileFadeState), (With<MapTile>, Without<TileMeshQuad>)>,
    tiles_with_quad: Query<(Entity, &TileMeshQuad, &TileFadeState), With<MapTile>>,
    all_quad_entities: Query<Entity, With<TileQuad3d>>,
) {
    let Some(quad_mesh) = quad_mesh else { return };

    if view3d_state.is_3d_active() {
        let current_zoom = map_state.zoom_level.to_u8();

        // Create mesh quads for tiles at current zoom AND one zoom level
        // below (fallback). When zoom changes, the previous zoom's mesh
        // quads persist as visual fallback until new tiles arrive and their
        // mesh quads replace them. This prevents the ground from going
        // blank during zoom transitions. Minor Z-fighting between adjacent
        // zoom levels at grazing angles near the horizon is acceptable
        // since those tiles are already faded by distance fog.
        let min_quad_zoom = current_zoom.saturating_sub(1);
        for (tile_entity, sprite, transform, fade_state) in tiles_without_quad.iter() {
            if fade_state.tile_zoom < min_quad_zoom || fade_state.tile_zoom > current_zoom {
                continue;
            }

            // Emissive material with exposure_weight=0.0 bypasses all
            // camera exposure so tiles render at consistent brightness.
            let m = TILE_EMISSIVE_BOOST;
            let material = materials.add(StandardMaterial {
                base_color: Color::BLACK,
                emissive: bevy::color::LinearRgba::new(m, m, m, 1.0),
                emissive_texture: Some(sprite.image.clone()),
                emissive_exposure_weight: 0.0,
                perceptual_roughness: 1.0,
                metallic: 0.0,
                alpha_mode: AlphaMode::Opaque,
                ..default()
            });

            let pos_yup = view3d::zup_to_yup(transform.translation);
            let mesh_entity = commands.spawn((
                TileQuad3d,
                Mesh3d(quad_mesh.0.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(pos_yup)
                    .with_scale(Vec3::new(transform.scale.x, 1.0, transform.scale.x)),
                Pickable::IGNORE,
                RenderLayers::layer(RenderCategory::TILES_3D),
            )).id();

            commands.entity(tile_entity).queue_silenced(move |mut entity: EntityWorldMut| {
                entity.insert(TileMeshQuad(mesh_entity));
            });
        }

        // Despawn mesh quads for tiles more than 1 zoom level away from
        // current. Adjacent-zoom quads are kept as fallback.
        for (tile_entity, quad, fade_state) in tiles_with_quad.iter() {
            if fade_state.tile_zoom < min_quad_zoom || fade_state.tile_zoom > current_zoom {
                commands.entity(quad.0).despawn();
                commands.entity(tile_entity).remove::<TileMeshQuad>();
            }
        }
    } else {
        // 2D mode: remove TileMeshQuad component from tiles
        for (tile_entity, _, _) in tiles_with_quad.iter() {
            commands.entity(tile_entity).remove::<TileMeshQuad>();
        }
        // Despawn all companion mesh entities (referenced + orphans)
        for quad_entity in all_quad_entities.iter() {
            commands.entity(quad_entity).despawn();
        }
    }
}

/// Sync mesh companion visibility with the tile's fade state.
/// Mesh quads are always AlphaMode::Opaque (required for depth writes so the
/// atmosphere post-process doesn't overwrite them). Tiles that haven't finished
/// loading (alpha near 0) are hidden entirely to avoid showing placeholder white.
fn sync_tile_mesh_alpha(
    view3d_state: Res<view3d::View3DState>,
    tile_query: Query<(&TileFadeState, &TileMeshQuad), With<MapTile>>,
    mut vis_query: Query<&mut Visibility>,
) {
    if !view3d_state.is_3d_active() {
        return;
    }

    for (fade_state, quad) in tile_query.iter() {
        let Ok(mut vis) = vis_query.get_mut(quad.0) else { continue };
        // Show the mesh quad once the tile texture has started loading.
        // A very low threshold (0.01) makes tiles appear within 1 frame of
        // spawning, reducing the dark-flash gap during rapid zoom.  The mesh
        // uses AlphaMode::Opaque so any non-zero alpha is fully visible.
        *vis = if fade_state.alpha > 0.01 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Sync mesh companion transforms with tile sprite transforms.
pub(crate) fn sync_tile_mesh_transforms(
    view3d_state: Res<view3d::View3DState>,
    tile_query: Query<(&Transform, &TileMeshQuad), With<MapTile>>,
    mut mesh_transforms: Query<&mut Transform, Without<MapTile>>,
) {
    if !view3d_state.is_3d_active() {
        return;
    }

    for (tile_tf, quad) in tile_query.iter() {
        let Ok(mut mesh_tf) = mesh_transforms.get_mut(quad.0) else { continue };
        let pos_yup = view3d::zup_to_yup(tile_tf.translation);
        mesh_tf.translation = pos_yup;
        mesh_tf.scale = Vec3::new(tile_tf.scale.x, 1.0, tile_tf.scale.x);
    }
}

/// Hide ALL tile sprites in 3D mode so Camera2d shows nothing for tiles.
/// Camera2d subscribes to TILES_2D at startup (can't change at runtime
/// without deferred commands that break Metal). Any visible tile sprite
/// would render on Camera2d (order 1) and paint over Camera3d's aircraft.
/// Sets sprite alpha to 0 for ALL tiles, not just those with mesh quads.
/// In 2D mode this is a no-op since sprites already have correct alpha.
fn hide_tile_sprites_in_3d(
    view3d_state: Res<view3d::View3DState>,
    mut tile_query: Query<&mut Sprite, With<MapTile>>,
) {
    if !view3d_state.is_3d_active() {
        return;
    }

    for mut sprite in tile_query.iter_mut() {
        sprite.color = Color::srgba(1.0, 1.0, 1.0, 0.0);
    }
}

/// Despawn companion mesh entities that are no longer referenced by any tile.
/// This catches orphans created when tiles are culled in the same frame as
/// companion spawning (deferred commands mean the tile despawn and companion
/// spawn can race).
fn cleanup_orphaned_tile_quads(
    mut commands: Commands,
    view3d_state: Res<view3d::View3DState>,
    quad_entities: Query<Entity, With<TileQuad3d>>,
    tile_quads: Query<&TileMeshQuad, With<MapTile>>,
) {
    // In 2D mode, sync_tile_mesh_quads already despawns all companions
    if !view3d_state.is_3d_active() {
        return;
    }
    let referenced: std::collections::HashSet<Entity> =
        tile_quads.iter().map(|q| q.0).collect();
    for entity in quad_entities.iter() {
        if !referenced.contains(&entity) {
            commands.entity(entity).despawn();
        }
    }
}
