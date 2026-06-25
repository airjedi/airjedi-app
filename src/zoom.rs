use bevy::ecs::schedule::ApplyDeferred;
use bevy::input::gestures::PinchGesture;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy_egui::EguiContexts;
use bevy_slippy_tiles::*;

use crate::camera::MapCamera;
use crate::constants::{self, ZOOM_DOWNGRADE_THRESHOLD, ZOOM_UPGRADE_THRESHOLD};
use crate::dock;
use crate::map::{MapState, ZoomState};
use crate::tiles::{request_tiles_at_location, SpawnedTiles, TileFadeState};
use crate::view3d;
use crate::{clamp_latitude, clamp_longitude, ZoomDebugLogger};

pub(crate) struct ZoomPlugin;

impl Plugin for ZoomPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_zoom)
            .add_systems(Update, handle_pinch_zoom)
            .add_systems(Update, ApplyDeferred.after(handle_zoom))
            .add_systems(Update, apply_camera_zoom.after(ApplyDeferred));
    }
}

// =============================================================================
// Zoom Calculation Helpers
// =============================================================================

/// Convert mouse wheel event to zoom delta factor.
/// Returns positive for zoom in, negative for zoom out.
fn calculate_zoom_delta(event: &MouseWheel) -> f32 {
    match event.unit {
        bevy::input::mouse::MouseScrollUnit::Line => event.y * constants::ZOOM_SENSITIVITY_LINE,
        bevy::input::mouse::MouseScrollUnit::Pixel => event.y * constants::ZOOM_SENSITIVITY_PIXEL,
    }
}

/// Calculate new map center to keep the point under cursor stationary during zoom.
///
/// Returns the new (latitude, longitude) for the map center.
fn calculate_zoom_to_cursor_center(
    cursor_viewport_pos: Vec2,
    window_size: (f32, f32),
    current_center: (f64, f64),
    camera_zoom_before: f32,
    camera_zoom_after: f32,
    old_tile_zoom: ZoomLevel,
    new_tile_zoom: ZoomLevel,
) -> (f64, f64) {
    // Calculate cursor offset from screen center
    let screen_center = (window_size.0 / 2.0, window_size.1 / 2.0);
    let cursor_offset = (
        (cursor_viewport_pos.x - screen_center.0) as f64,
        -(cursor_viewport_pos.y - screen_center.1) as f64, // Y inverted
    );

    // Convert to world pixels before zoom (using old camera zoom)
    let world_offset_before = (
        cursor_offset.0 / camera_zoom_before as f64,
        cursor_offset.1 / camera_zoom_before as f64,
    );

    // Get current center in world pixels at old zoom level
    let center_pixel = world_coords_to_world_pixel(
        &LatitudeLongitudeCoordinates {
            latitude: current_center.0,
            longitude: current_center.1,
        },
        crate::constants::DEFAULT_TILE_SIZE,
        old_tile_zoom,
    );

    // Calculate cursor geographic position at old zoom level
    let cursor_geo = world_pixel_to_world_coords(
        center_pixel.0 + world_offset_before.0,
        center_pixel.1 + world_offset_before.1,
        crate::constants::DEFAULT_TILE_SIZE,
        old_tile_zoom,
    );

    // Calculate world offset after zoom (using new camera zoom)
    let world_offset_after = (
        cursor_offset.0 / camera_zoom_after as f64,
        cursor_offset.1 / camera_zoom_after as f64,
    );

    // Convert cursor geo back to pixels at new zoom level
    let cursor_pixel_after = world_coords_to_world_pixel(
        &LatitudeLongitudeCoordinates {
            latitude: cursor_geo.latitude,
            longitude: cursor_geo.longitude,
        },
        crate::constants::DEFAULT_TILE_SIZE,
        new_tile_zoom,
    );

    // New center = cursor position minus the offset
    let new_center = world_pixel_to_world_coords(
        cursor_pixel_after.0 - world_offset_after.0,
        cursor_pixel_after.1 - world_offset_after.1,
        crate::constants::DEFAULT_TILE_SIZE,
        new_tile_zoom,
    );

    (new_center.latitude, new_center.longitude)
}

// =============================================================================
// Zoom Level Transition Helpers (shared by scroll and pinch zoom)
// =============================================================================

/// Check if the camera zoom has crossed a tile zoom level threshold.
/// If so, adjusts camera_zoom and map_state.zoom_level.
/// Returns (zoom_level_changed, old_tile_zoom_level).
fn check_zoom_level_transition(
    zoom_state: &mut ZoomState,
    map_state: &mut MapState,
) -> (bool, ZoomLevel) {
    let old_tile_zoom = map_state.zoom_level;
    let current_tile_zoom = old_tile_zoom.to_u8();

    if zoom_state.camera_zoom >= ZOOM_UPGRADE_THRESHOLD && current_tile_zoom < 19 {
        zoom_state.camera_zoom /= 2.0;
        if let Ok(new_zoom) = ZoomLevel::try_from(current_tile_zoom + 1) {
            map_state.zoom_level = new_zoom;
            return (true, old_tile_zoom);
        }
    } else if zoom_state.camera_zoom <= ZOOM_DOWNGRADE_THRESHOLD && current_tile_zoom > 0 {
        zoom_state.camera_zoom *= 2.0;
        if let Ok(new_zoom) = ZoomLevel::try_from(current_tile_zoom - 1) {
            map_state.zoom_level = new_zoom;
            return (true, old_tile_zoom);
        }
    }

    (false, old_tile_zoom)
}

/// After a zoom level transition, scale existing tiles to match the new
/// coordinate system and request fresh tiles at the new zoom level.
fn apply_zoom_level_transition(
    old_tile_zoom: ZoomLevel,
    map_state: &MapState,
    tile_query: &mut Query<(&mut TileFadeState, &mut Transform), With<MapTile>>,
    spawned_tiles: &mut SpawnedTiles,
    download_events: &mut MessageWriter<DownloadSlippyTilesMessage>,
    download_status: &mut SlippyTileDownloadStatus,
) {
    spawned_tiles.positions.clear();
    download_status.0.clear();
    let scale_factor = if map_state.zoom_level.to_u8() > old_tile_zoom.to_u8() {
        2.0_f32
    } else {
        0.5_f32
    };
    for (_fade_state, mut transform) in tile_query.iter_mut() {
        transform.translation.x *= scale_factor;
        transform.translation.y *= scale_factor;
        transform.scale *= scale_factor;
    }
    request_tiles_at_location(
        download_events,
        map_state.latitude,
        map_state.longitude,
        map_state.zoom_level,
        true,
    );
}

// =============================================================================
// Zoom Systems
// =============================================================================

pub(crate) fn handle_zoom(
    mut scroll_events: MessageReader<MouseWheel>,
    mut map_state: ResMut<MapState>,
    mut zoom_state: ResMut<ZoomState>,
    mut download_events: MessageWriter<DownloadSlippyTilesMessage>,
    window_query: Query<&Window>,
    mut tile_query: Query<(&mut TileFadeState, &mut Transform), With<MapTile>>,
    logger: Option<Res<ZoomDebugLogger>>,
    mut contexts: EguiContexts,
    dock_state: Res<dock::DockTreeState>,
    mut spawned_tiles: ResMut<SpawnedTiles>,
    view3d_state: Res<view3d::View3DState>,
    mut download_status: ResMut<SlippyTileDownloadStatus>,
) {
    // In 3D mode, scroll is handled by handle_3d_camera_controls
    if view3d_state.is_3d_active() || view3d_state.is_transitioning() {
        return;
    }

    // Shift+scroll in 2D mode: do nothing (pitch control is only in 3D mode).
    // Read shift from egui since bevy_egui absorbs modifier keys from ButtonInput.
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.input(|i| i.modifiers.shift) {
            return;
        }
    }
    // Don't zoom the map when pointer is over a dock panel (but allow zoom over the map viewport)
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.is_pointer_over_area() {
            // The egui CentralPanel covers the entire window, so is_pointer_over_area() is
            // always true. Check if the pointer is inside the map viewport pane -- if so,
            // allow zoom through to Bevy.
            if let Some(map_rect) = dock_state.map_viewport_rect {
                if let Some(pos) = ctx.pointer_latest_pos() {
                    if !map_rect.contains(pos) {
                        return;
                    }
                } else {
                    return;
                }
            }
        }
    }

    let Ok(window) = window_query.single() else {
        return;
    };

    // Macro to log to both console and file
    macro_rules! log_info {
        ($($arg:tt)*) => {
            {
                let msg = format!($($arg)*);
                debug!("{}", msg);
                if let Some(ref log) = logger {
                    log.log(&msg);
                }
            }
        };
    }

    for event in scroll_events.read() {
        log_info!("=== SCROLL EVENT START ===");
        log_info!("Event: unit={:?}, y={}", event.unit, event.y);
        log_info!(
            "Before: camera_zoom={}, zoom_level={}",
            zoom_state.camera_zoom,
            map_state.zoom_level.to_u8()
        );
        log_info!(
            "Before: map center=({:.6}, {:.6})",
            map_state.latitude,
            map_state.longitude
        );

        // === Calculate zoom delta from scroll event ===
        let zoom_delta = calculate_zoom_delta(event);
        log_info!("Zoom delta: {}", zoom_delta);

        // Get cursor position in viewport coordinates (None if cursor not in window)
        let Some(cursor_viewport_pos) = window.cursor_position() else {
            // No cursor, just zoom at center
            log_info!("No cursor - new camera_zoom={}", zoom_state.camera_zoom);
            continue;
        };

        log_info!(
            "Cursor position: ({:.2}, {:.2})",
            cursor_viewport_pos.x,
            cursor_viewport_pos.y
        );

        // Save old camera zoom BEFORE applying scroll zoom (needed for zoom-to-cursor)
        let camera_zoom_before_scroll = zoom_state.camera_zoom;

        // Update camera zoom (multiplicative for smooth feel)
        // Positive scroll (up/forward) = zoom in, negative = zoom out
        let zoom_factor = 1.0 + zoom_delta;
        let new_camera_zoom =
            (zoom_state.camera_zoom * zoom_factor).clamp(zoom_state.min_zoom, zoom_state.max_zoom);

        log_info!(
            "Camera zoom: {} -> {}",
            zoom_state.camera_zoom,
            new_camera_zoom
        );
        zoom_state.camera_zoom = new_camera_zoom;

        // === Check for zoom level transitions ===
        let (zoom_level_changed, old_tile_zoom) =
            check_zoom_level_transition(&mut zoom_state, &mut map_state);

        if zoom_level_changed {
            log_info!(
                "*** ZOOM LEVEL TRANSITION: {} -> {} ***",
                old_tile_zoom.to_u8(),
                map_state.zoom_level.to_u8()
            );
        }

        // === Calculate new center (zoom-to-cursor) ===
        log_info!("--- Zoom-to-cursor calculation ---");
        log_info!(
            "  old_zoom_level={}, new_zoom_level={}, zoom_level_changed={}",
            old_tile_zoom.to_u8(),
            map_state.zoom_level.to_u8(),
            zoom_level_changed
        );

        let old_lat = map_state.latitude;
        let old_lon = map_state.longitude;
        let (new_lat, new_lon) = calculate_zoom_to_cursor_center(
            cursor_viewport_pos,
            (window.width(), window.height()),
            (map_state.latitude, map_state.longitude),
            camera_zoom_before_scroll,
            zoom_state.camera_zoom,
            old_tile_zoom,
            map_state.zoom_level,
        );
        map_state.latitude = clamp_latitude(new_lat);
        map_state.longitude = clamp_longitude(new_lon);
        log_info!(
            "  Map center updated: ({:.6}, {:.6}) -> ({:.6}, {:.6})",
            old_lat,
            old_lon,
            map_state.latitude,
            map_state.longitude
        );

        // === Handle zoom level transition (scale old tiles, request new) ===
        if zoom_level_changed {
            apply_zoom_level_transition(
                old_tile_zoom,
                &map_state,
                &mut tile_query,
                &mut spawned_tiles,
                &mut download_events,
                &mut download_status,
            );
            log_info!(
                "  Requested new tiles at zoom level {}",
                map_state.zoom_level.to_u8()
            );
        }

        log_info!(
            "=== SCROLL EVENT END ===
"
        );
    }
}

/// Handle trackpad pinch-to-zoom gestures (macOS).
/// PinchGesture.0 is positive for zoom in, negative for zoom out.
pub(crate) fn handle_pinch_zoom(
    mut pinch_events: MessageReader<PinchGesture>,
    mut map_state: ResMut<MapState>,
    mut zoom_state: ResMut<ZoomState>,
    mut download_events: MessageWriter<DownloadSlippyTilesMessage>,
    window_query: Query<&Window>,
    mut tile_query: Query<(&mut TileFadeState, &mut Transform), With<MapTile>>,
    mut contexts: EguiContexts,
    dock_state: Res<dock::DockTreeState>,
    mut spawned_tiles: ResMut<SpawnedTiles>,
    view3d_state: Res<view3d::View3DState>,
    mut download_status: ResMut<SlippyTileDownloadStatus>,
) {
    // In 3D mode, zoom is handled by handle_3d_camera_controls
    if view3d_state.is_3d_active() || view3d_state.is_transitioning() {
        return;
    }

    // Don't zoom when pointer is over a dock panel (same logic as handle_zoom)
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.is_pointer_over_area() {
            if let Some(map_rect) = dock_state.map_viewport_rect {
                if let Some(pos) = ctx.pointer_latest_pos() {
                    if !map_rect.contains(pos) {
                        return;
                    }
                } else {
                    return;
                }
            }
        }
    }

    let Ok(window) = window_query.single() else {
        return;
    };

    for event in pinch_events.read() {
        let camera_zoom_before = zoom_state.camera_zoom;

        // Apply pinch directly as a multiplicative factor
        let zoom_factor = 1.0 + event.0;
        zoom_state.camera_zoom =
            (zoom_state.camera_zoom * zoom_factor).clamp(zoom_state.min_zoom, zoom_state.max_zoom);

        // Zoom-to-cursor: keep the point under cursor stationary
        if let Some(cursor_viewport_pos) = window.cursor_position() {
            let (zoom_level_changed, old_tile_zoom) =
                check_zoom_level_transition(&mut zoom_state, &mut map_state);

            let (new_lat, new_lon) = calculate_zoom_to_cursor_center(
                cursor_viewport_pos,
                (window.width(), window.height()),
                (map_state.latitude, map_state.longitude),
                camera_zoom_before,
                zoom_state.camera_zoom,
                old_tile_zoom,
                map_state.zoom_level,
            );
            map_state.latitude = clamp_latitude(new_lat);
            map_state.longitude = clamp_longitude(new_lon);

            if zoom_level_changed {
                apply_zoom_level_transition(
                    old_tile_zoom,
                    &map_state,
                    &mut tile_query,
                    &mut spawned_tiles,
                    &mut download_events,
                    &mut download_status,
                );
            }
        }
    }
}

/// Apply the camera zoom to the actual camera projection.
pub(crate) fn apply_camera_zoom(
    zoom_state: Res<ZoomState>,
    mut camera_query: Query<&mut Projection, With<MapCamera>>,
) {
    if let Ok(mut projection) = camera_query.single_mut() {
        // Access the OrthographicProjection within Projection
        if let Projection::Orthographic(ref mut ortho) = projection.as_mut() {
            // Use camera_zoom directly - tiles are already at correct world-space scale
            // Smaller scale = more zoomed in, larger scale = more zoomed out
            ortho.scale = 1.0 / zoom_state.camera_zoom;
        }
    }
}
