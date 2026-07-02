use bevy::ecs::schedule::ApplyDeferred;
use bevy::input::gestures::PinchGesture;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy_egui::EguiContexts;
use crate::tiles::*;

use crate::camera::MapCamera;
use crate::constants::{self, ZOOM_DOWNGRADE_THRESHOLD, ZOOM_UPGRADE_THRESHOLD};
use crate::dock;
use crate::map::{MapState, ZoomState};
use crate::tiles::{compute_tile_radius, request_tiles_at_location, TileFadeState};
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
/// Uses Mercator meter coordinates which are zoom-independent, so changing
/// the discrete tile zoom level doesn't affect the calculation.
fn calculate_zoom_to_cursor_center(
    cursor_viewport_pos: Vec2,
    window_size: (f32, f32),
    current_center: (f64, f64),
    camera_zoom_before: f32,
    camera_zoom_after: f32,
    _old_tile_zoom: ZoomLevel,
    _new_tile_zoom: ZoomLevel,
) -> (f64, f64) {
    let screen_center = (window_size.0 / 2.0, window_size.1 / 2.0);
    let cursor_offset = (
        (cursor_viewport_pos.x - screen_center.0) as f64,
        -(cursor_viewport_pos.y - screen_center.1) as f64,
    );

    // Convert cursor offset to Mercator meters before and after zoom.
    // ortho.scale = 1/camera_zoom, so 1 screen pixel = (1/camera_zoom) meters.
    let ortho_before = 1.0 / camera_zoom_before as f64;
    let ortho_after = 1.0 / camera_zoom_after as f64;

    let center_merc = lonlat_to_mercator(current_center.1, current_center.0);

    // Cursor position in Mercator meters (same point, same meters, zoom-independent)
    let cursor_merc_x = center_merc.x + cursor_offset.0 * ortho_before;
    let cursor_merc_y = center_merc.y + cursor_offset.1 * ortho_before;

    // New center: keep cursor at same Mercator position but at new screen offset
    let new_center_x = cursor_merc_x - cursor_offset.0 * ortho_after;
    let new_center_y = cursor_merc_y - cursor_offset.1 * ortho_after;

    let (new_lon, new_lat) = mercator_to_lonlat(bevy::math::DVec2::new(new_center_x, new_center_y));
    (new_lat, new_lon)
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
    let mut changed = false;

    const MIN_TILE_ZOOM: u8 = 3;

    // Loop to handle multiple zoom level crossings from a single large scroll
    loop {
        let current_tile_zoom = map_state.zoom_level.to_u8();
        if zoom_state.camera_zoom >= ZOOM_UPGRADE_THRESHOLD && current_tile_zoom < 19 {
            zoom_state.camera_zoom /= 2.0;
            if let Ok(new_zoom) = ZoomLevel::try_from(current_tile_zoom + 1) {
                map_state.zoom_level = new_zoom;
                changed = true;
                continue;
            }
        } else if zoom_state.camera_zoom <= ZOOM_DOWNGRADE_THRESHOLD && current_tile_zoom > MIN_TILE_ZOOM {
            zoom_state.camera_zoom *= 2.0;
            if let Ok(new_zoom) = ZoomLevel::try_from(current_tile_zoom - 1) {
                map_state.zoom_level = new_zoom;
                changed = true;
                continue;
            }
        }
        break;
    }

    // Clamp camera_zoom so the map always fills the screen at the minimum zoom
    if map_state.zoom_level.to_u8() == MIN_TILE_ZOOM {
        zoom_state.camera_zoom = zoom_state.camera_zoom.max(ZOOM_DOWNGRADE_THRESHOLD + 0.01);
    }

    (changed, old_tile_zoom)
}

/// After a zoom level transition, request fresh tiles at the new zoom level.
/// In the Mercator meter coordinate system, tile positions are zoom-independent
/// so no rescaling is needed. Old-zoom tiles are kept visible until new-zoom
/// tiles load (handled by animate_tile_fades).
fn apply_zoom_level_transition(
    _old_tile_zoom: ZoomLevel,
    map_state: &MapState,
    _tile_query: &mut Query<(&mut TileFadeState, &mut Transform), With<MapTile>>,
    download_events: &mut MessageWriter<DownloadTilesRequest>,
    _tile_grid: &mut crate::tiles::pool::TileGrid,
    radius: u8,
) {
    request_tiles_at_location(
        download_events,
        map_state.latitude,
        map_state.longitude,
        map_state.zoom_level,
        radius,
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
    mut download_events: MessageWriter<DownloadTilesRequest>,
    window_query: Query<&Window>,
    mut tile_query: Query<(&mut TileFadeState, &mut Transform), With<MapTile>>,
    logger: Option<Res<ZoomDebugLogger>>,
    mut contexts: EguiContexts,
    dock_state: Res<dock::DockTreeState>,
    view3d_state: Res<view3d::View3DState>,
    mut tile_grid: ResMut<crate::tiles::pool::TileGrid>,
    mut last_requested_radius: Local<u8>,
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
        if ctx.is_pointer_over_egui() {
            // The egui CentralPanel covers the entire window, so is_pointer_over_egui() is
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
        let radius = compute_tile_radius(
            window.width(),
            window.height(),
            zoom_state.camera_zoom,
            Some(&view3d_state), map_state.zoom_level.to_u8(),
        );
        if zoom_level_changed {
            apply_zoom_level_transition(
                old_tile_zoom,
                &map_state,
                &mut tile_query,
                &mut download_events,
                &mut tile_grid,
                radius,
            );
            *last_requested_radius = radius;
            log_info!(
                "  Requested new tiles at zoom level {}",
                map_state.zoom_level.to_u8()
            );
        } else if radius > *last_requested_radius {
            download_events.write(DownloadTilesRequest {
                latitude: map_state.latitude,
                longitude: map_state.longitude,
                zoom: map_state.zoom_level.to_u8(),
                radius: Radius(radius),
                priority: DownloadPriority::Near,
                use_cache: true,
            });
            *last_requested_radius = radius;
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
    mut download_events: MessageWriter<DownloadTilesRequest>,
    window_query: Query<&Window>,
    mut tile_query: Query<(&mut TileFadeState, &mut Transform), With<MapTile>>,
    mut contexts: EguiContexts,
    dock_state: Res<dock::DockTreeState>,
    view3d_state: Res<view3d::View3DState>,
    mut tile_grid: ResMut<crate::tiles::pool::TileGrid>,
    mut last_requested_radius: Local<u8>,
) {
    // In 3D mode, zoom is handled by handle_3d_camera_controls
    if view3d_state.is_3d_active() || view3d_state.is_transitioning() {
        return;
    }

    // Don't zoom when pointer is over a dock panel (same logic as handle_zoom)
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.is_pointer_over_egui() {
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

            let radius = compute_tile_radius(
                window.width(),
                window.height(),
                zoom_state.camera_zoom,
                Some(&view3d_state), map_state.zoom_level.to_u8(),
            );
            if zoom_level_changed {
                apply_zoom_level_transition(
                    old_tile_zoom,
                    &map_state,
                    &mut tile_query,
                    &mut download_events,
                    &mut tile_grid,
                    radius,
                );
                *last_requested_radius = radius;
            } else if radius > *last_requested_radius {
                download_events.write(DownloadTilesRequest {
                    latitude: map_state.latitude,
                    longitude: map_state.longitude,
                    zoom: map_state.zoom_level.to_u8(),
                    radius: Radius(radius),
                    priority: DownloadPriority::Near,
                    use_cache: true,
                });
                *last_requested_radius = radius;
            }
        }
    }
}

/// Apply the camera zoom to the actual camera projection.
/// In the Mercator meter coordinate system, ortho.scale must account for
/// the tile zoom level since tiles at different zooms have different meter sizes.
pub(crate) fn apply_camera_zoom(
    mut zoom_state: ResMut<ZoomState>,
    map_state: Res<crate::map::MapState>,
    mut camera_query: Query<&mut Projection, With<MapCamera>>,
    window_query: Query<&Window>,
) {
    if let Ok(mut projection) = camera_query.single_mut() {
        if let Projection::Orthographic(ref mut ortho) = projection.as_mut() {
            let tile_size_meters = (2.0 * super::tiles::WEB_MERCATOR_EXTENT)
                / (1u64 << map_state.zoom_level.to_u8()) as f64;
            let meters_per_tile_pixel = tile_size_meters / crate::constants::DEFAULT_TILE_PIXELS as f64;

            // Clamp camera_zoom so the map always fills the viewport width.
            // min_camera_zoom = window_width * meters_per_tile_pixel / map_width
            let map_width = 2.0 * super::tiles::WEB_MERCATOR_EXTENT;
            if let Ok(window) = window_query.single() {
                let min_zoom = (window.width() as f64 * meters_per_tile_pixel / map_width) as f32;
                if zoom_state.camera_zoom < min_zoom {
                    zoom_state.camera_zoom = min_zoom;
                }
            }

            ortho.scale = (meters_per_tile_pixel / zoom_state.camera_zoom as f64) as f32;
        }
    }
}
