use bevy::prelude::*;
use bevy_egui::EguiContexts;
use crate::tiles::*;

use crate::constants;
use crate::dock;
use crate::map::{MapState, ZoomState};
use crate::tiles::{compute_tile_radius, request_tiles_at_location};
use crate::view3d;
use crate::{clamp_latitude, clamp_longitude};

// =============================================================================
// Resources
// =============================================================================

/// Resource to track pan/drag state.
#[derive(Resource, Default)]
pub(crate) struct DragState {
    is_dragging: bool,
    last_position: Option<Vec2>,
    last_tile_request_coords: Option<(f64, f64)>,
}

/// Tracks whether egui wants pointer input this frame, used to prevent
/// map interactions when clicking/scrolling over UI panels.
#[derive(Resource, Default)]
pub(crate) struct EguiWantsPointer(pub(crate) bool);

// =============================================================================
// Plugin
// =============================================================================

pub(crate) struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DragState>()
            .init_resource::<EguiWantsPointer>()
            .add_systems(
                Update,
                check_egui_wants_input
                    .before(handle_pan_drag)
                    .before(crate::zoom::handle_zoom),
            )
            .add_systems(Update, handle_pan_drag);
    }
}

// =============================================================================
// Input Systems
// =============================================================================

pub(crate) fn check_egui_wants_input(
    mut contexts: EguiContexts,
    mut drag_state: ResMut<DragState>,
    mut egui_wants: ResMut<EguiWantsPointer>,
    dock_state: Res<dock::DockTreeState>,
) {
    egui_wants.0 = false;
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.egui_wants_keyboard_input() || ctx.egui_wants_pointer_input() {
            // Check if pointer is over the map viewport (allow interaction there)
            let over_map = if let Some(map_rect) = dock_state.map_viewport_rect {
                ctx.pointer_latest_pos()
                    .is_some_and(|pos| map_rect.contains(pos))
            } else {
                false
            };
            if !over_map {
                egui_wants.0 = true;
                drag_state.is_dragging = false;
                drag_state.last_position = None;
            }
        }
    }
}

pub(crate) fn handle_pan_drag(
    mouse_button: Res<ButtonInput<MouseButton>>,
    mut cursor_moved: MessageReader<CursorMoved>,
    mut map_state: ResMut<MapState>,
    mut drag_state: ResMut<DragState>,
    zoom_state: Res<ZoomState>,
    mut download_events: MessageWriter<DownloadTilesRequest>,
    mut follow_state: ResMut<crate::aircraft::CameraFollowState>,
    window_query: Query<&Window>,
    egui_wants: Res<EguiWantsPointer>,
    view3d_state: Res<view3d::View3DState>,
) {
    let Ok(window) = window_query.single() else {
        return;
    };

    // In 3D mode, panning is handled by handle_3d_camera_controls
    if view3d_state.is_3d_active() || view3d_state.is_transitioning() {
        drag_state.is_dragging = false;
        drag_state.last_position = None;
        return;
    }

    // Only start a new drag if pointer is not over a UI panel
    if mouse_button.just_pressed(MouseButton::Left) && !egui_wants.0 {
        drag_state.is_dragging = true;
        drag_state.last_position = None;
    }

    if mouse_button.just_released(MouseButton::Left) {
        if drag_state.is_dragging {
            let radius = compute_tile_radius(
                window.width(),
                window.height(),
                zoom_state.camera_zoom,
                Some(&view3d_state),
            );
            request_tiles_at_location(
                &mut download_events,
                map_state.latitude,
                map_state.longitude,
                map_state.zoom_level,
                radius,
                true,
            );
            drag_state.last_tile_request_coords = Some((map_state.latitude, map_state.longitude));
        }
        drag_state.is_dragging = false;
        drag_state.last_position = None;
    }

    // Handle dragging
    if drag_state.is_dragging {
        for event in cursor_moved.read() {
            if let Some(last_pos) = drag_state.last_position {
                let delta = event.position - last_pos;

                if delta.length() > 2.0 && follow_state.following_icao.is_some() {
                    follow_state.following_icao = None;
                }

                // Convert screen delta to world delta in meters.
                // ortho.scale = 1/camera_zoom, so 1 screen pixel = ortho.scale world meters
                let ortho_scale = 1.0 / zoom_state.camera_zoom as f64;
                let delta_meters_x = -(delta.x as f64) * ortho_scale;
                let delta_meters_y = (delta.y as f64) * ortho_scale;

                // Get current center in Mercator meters, apply delta, convert back
                let center_merc = lonlat_to_mercator(map_state.longitude, map_state.latitude);
                let new_merc = bevy::math::DVec2::new(
                    center_merc.x + delta_meters_x,
                    center_merc.y + delta_meters_y,
                );
                let (new_lon, new_lat) = mercator_to_lonlat(new_merc);

                map_state.latitude = clamp_latitude(new_lat);
                map_state.longitude = clamp_longitude(new_lon);

                // Request tiles periodically during drag to fill visible area
                let should_request = match drag_state.last_tile_request_coords {
                    Some((last_lat, last_lon)) => {
                        let lat_diff = (map_state.latitude - last_lat).abs();
                        let lon_diff = (map_state.longitude - last_lon).abs();
                        lat_diff > constants::PAN_TILE_REQUEST_THRESHOLD
                            || lon_diff > constants::PAN_TILE_REQUEST_THRESHOLD
                    }
                    None => true,
                };
                if should_request {
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
                    drag_state.last_tile_request_coords =
                        Some((map_state.latitude, map_state.longitude));
                }
            }
            drag_state.last_position = Some(event.position);
        }
    }
}
