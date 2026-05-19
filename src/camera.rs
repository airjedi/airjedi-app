use bevy::prelude::*;
use bevy_slippy_tiles::*;

use crate::constants;
use crate::geo;
use crate::map::{MapState, ZoomState};
use crate::view3d;
use crate::{clamp_latitude, clamp_longitude, Aircraft, AircraftLabel, ZoomDebugLogger, ZoomSet};

// =============================================================================
// Constants
// =============================================================================

/// Base rotation for aircraft GLB models in Y-up 3D space.
/// GLB model: nose=+Z, top=+Y, right-wing=+X.
/// Y-up world: north=-Z, up=+Y.
/// Rotate 180 deg around Y so nose points -Z (north).
/// Then heading rotation is applied around Y axis.
pub(crate) const BASE_ROT_YUP: Quat = Quat::from_xyzw(0.0, 1.0, 0.0, 0.0); // 180 deg around Y

// =============================================================================
// Components and Resources
// =============================================================================

/// Marker for the 3D camera that renders aircraft models (HDR, with Atmosphere).
#[derive(Component)]
pub(crate) struct AircraftCamera;

/// Marker for the lightweight 3D camera that renders aircraft in 2D mode (no HDR).
#[derive(Component)]
pub(crate) struct AircraftCamera2d;

/// Marker for the primary 2D map camera (distinguishes it from the egui UI camera).
#[derive(Component)]
pub(crate) struct MapCamera;

// =============================================================================
// Plugin
// =============================================================================

pub(crate) struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            follow_aircraft.after(crate::adsb::sync_aircraft_from_adsb),
        )
        .add_systems(
            Update,
            update_camera_position
                .after(crate::input::handle_pan_drag)
                .after(crate::zoom::apply_camera_zoom)
                .after(follow_aircraft),
        )
        .add_systems(
            Update,
            sync_aircraft_camera
                .after(update_camera_position)
                .after(crate::zoom::apply_camera_zoom)
                .after(view3d::update_3d_camera),
        )
        .add_systems(
            Update,
            update_aircraft_positions
                .after(update_camera_position)
                .after(crate::adsb::sync_aircraft_from_adsb)
                .after(crate::aircraft::interpolation::interpolate_aircraft_positions)
                .after(ZoomSet::Change),
        )
        .add_systems(
            Update,
            scale_aircraft_and_labels.after(crate::zoom::apply_camera_zoom),
        )
        .add_systems(
            Update,
            update_aircraft_labels.after(update_aircraft_positions),
        )
        .add_systems(
            Update,
            cull_offscreen_aircraft
                .after(update_aircraft_positions)
                .after(crate::view3d::update_aircraft_3d_transform),
        );
    }
}

// =============================================================================
// Camera Systems
// =============================================================================

/// System to follow a selected aircraft (moves map center to aircraft position).
fn follow_aircraft(
    mut map_state: ResMut<MapState>,
    follow_state: Res<crate::aircraft::CameraFollowState>,
    aircraft_query: Query<&Aircraft>,
    time: Res<Time>,
) {
    let Some(ref following_icao) = follow_state.following_icao else {
        return;
    };

    // Find the aircraft we're following
    let Some(aircraft) = aircraft_query.iter().find(|a| &a.icao == following_icao) else {
        return;
    };

    // Lerp towards the aircraft position for smooth following
    let lerp_speed = 3.0; // How fast to catch up (higher = faster)
    let t = (lerp_speed * time.delta_secs()).min(1.0);

    let new_lat = map_state.latitude + (aircraft.latitude - map_state.latitude) * t as f64;
    let new_lon = map_state.longitude + (aircraft.longitude - map_state.longitude) * t as f64;

    map_state.latitude = clamp_latitude(new_lat);
    map_state.longitude = clamp_longitude(new_lon);
}

fn update_camera_position(
    map_state: Res<MapState>,
    tile_settings: Res<SlippyTilesSettings>,
    mut camera_query: Query<&mut Transform, With<MapCamera>>,
    logger: Option<Res<ZoomDebugLogger>>,
    view3d_state: Res<view3d::View3DState>,
) {
    // Don't fight with update_3d_camera during 3D mode or transitions
    if view3d_state.is_3d_active() || view3d_state.is_transitioning() {
        return;
    }

    let zoom_level = map_state.zoom_level;

    if let Ok(mut camera_transform) = camera_query.single_mut() {
        let reference_ll = LatitudeLongitudeCoordinates {
            latitude: tile_settings.reference_latitude,
            longitude: tile_settings.reference_longitude,
        };
        let reference_pixel = world_coords_to_world_pixel(
            &reference_ll,
            crate::constants::DEFAULT_TILE_SIZE,
            zoom_level
        );

        let center_ll = LatitudeLongitudeCoordinates {
            latitude: map_state.latitude,
            longitude: map_state.longitude,
        };
        let center_pixel = world_coords_to_world_pixel(
            &center_ll,
            crate::constants::DEFAULT_TILE_SIZE,
            zoom_level
        );

        let offset_x = center_pixel.0 - reference_pixel.0;
        let offset_y = center_pixel.1 - reference_pixel.1;

        if let Some(ref log) = logger {
            if map_state.is_changed() {
                log.log(&format!("=== CAMERA POS UPDATE (zoom: {}) ===", zoom_level.to_u8()));
                log.log(&format!("  center: ({:.6}, {:.6}) -> pixel ({:.2}, {:.2})",
                    map_state.latitude, map_state.longitude, center_pixel.0, center_pixel.1));
                log.log(&format!("  camera offset: ({:.2}, {:.2})", offset_x, offset_y));
            }
        }

        camera_transform.translation.x = offset_x as f32;
        camera_transform.translation.y = offset_y as f32;
    }
}

/// Sync Camera3d transform and projection to match Camera2d in 2D mode.
/// In 3D mode, update_3d_camera handles both cameras directly.
fn sync_aircraft_camera(
    view3d_state: Res<view3d::View3DState>,
    camera_2d: Query<(&Transform, &Projection), (With<MapCamera>, Without<AircraftCamera>, Without<AircraftCamera2d>)>,
    mut camera_3d: Query<(&mut Transform, &mut Projection), (With<AircraftCamera>, Without<AircraftCamera2d>, Without<Camera2d>)>,
    mut camera_ac2d: Query<(&mut Transform, &mut Projection), (With<AircraftCamera2d>, Without<AircraftCamera>, Without<MapCamera>)>,
) {
    // In 3D mode or during transitions, update_3d_camera owns both cameras
    if view3d_state.is_3d_active() || view3d_state.is_transitioning() {
        return;
    }

    let Ok((t2, p2)) = camera_2d.single() else {
        return;
    };
    // Sync both aircraft cameras to Camera2d's view
    if let Ok((mut t3, mut p3)) = camera_3d.single_mut() {
        *t3 = *t2;
        *p3 = p2.clone();
    }
    if let Ok((mut t, mut p)) = camera_ac2d.single_mut() {
        *t = *t2;
        *p = p2.clone();
    }
}

// =============================================================================
// Aircraft Rendering Systems
// =============================================================================

/// Keep aircraft and labels at constant screen size despite zoom changes.
/// In 2D mode, scale inversely with camera zoom for constant screen size.
/// In 3D perspective mode, use a fixed world-space scale and let perspective
/// projection handle apparent size (closer = bigger, farther = smaller).
fn scale_aircraft_and_labels(
    zoom_state: Res<ZoomState>,
    view3d_state: Res<crate::view3d::View3DState>,
    mut aircraft_query: Query<&mut Transform, (With<Aircraft>, Without<AircraftLabel>)>,
    mut label_query: Query<(&mut Transform, &mut TextFont), With<AircraftLabel>>,
    new_aircraft: Query<(), Added<Aircraft>>,
) {
    // Update scales when zoom changes, mode changes, or new aircraft are spawned
    if !zoom_state.is_changed() && !view3d_state.is_changed() && new_aircraft.is_empty() {
        return;
    }

    if view3d_state.is_3d_active() {
        // 3D perspective: fixed world-space scale. Perspective projection
        // naturally makes distant aircraft smaller and nearby aircraft larger.
        // Scale up significantly so aircraft are visible at altitude distances.
        let scale = constants::AIRCRAFT_MODEL_SCALE * 10.0;
        for mut transform in aircraft_query.iter_mut() {
            transform.scale = Vec3::splat(scale);
        }
    } else {
        // 2D orthographic: scale inversely with zoom for constant screen size
        let scale = constants::AIRCRAFT_MODEL_SCALE / zoom_state.camera_zoom;
        for mut transform in aircraft_query.iter_mut() {
            transform.scale = Vec3::splat(scale);
        }
    }

    // Labels are always 2D (hidden in 3D mode by update_aircraft_3d_transform)
    let label_scale = 1.0 / zoom_state.camera_zoom;
    for (mut transform, mut text_font) in label_query.iter_mut() {
        transform.scale = Vec3::splat(label_scale);
        text_font.font_size = constants::BASE_FONT_SIZE;
    }
}

pub(crate) fn update_aircraft_positions(
    map_state: Res<MapState>,
    tile_settings: Res<SlippyTilesSettings>,
    config: Res<crate::config::AppConfig>,
    mut aircraft_query: Query<(&Aircraft, Option<&crate::aircraft::InterpolationState>, &mut Transform)>,
) {
    let converter = geo::CoordinateConverter::new(&tile_settings, map_state.zoom_level);

    for (aircraft, interp_opt, mut transform) in aircraft_query.iter_mut() {
        // Use interpolated display position if available and enabled, otherwise raw ADS-B
        let (lat, lon, heading) = if config.interpolation_enabled {
            if let Some(interp) = interp_opt {
                (interp.display_lat, interp.display_lon, interp.display_heading)
            } else {
                (aircraft.latitude, aircraft.longitude, aircraft.heading)
            }
        } else {
            (aircraft.latitude, aircraft.longitude, aircraft.heading)
        };

        let pos = converter.latlon_to_world(lat, lon);

        transform.translation.x = pos.x;
        transform.translation.y = pos.y;

        let base_rot = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
            * Quat::from_rotation_z(std::f32::consts::PI);
        if let Some(heading) = heading {
            transform.rotation = Quat::from_rotation_z((-heading).to_radians()) * base_rot;
        } else {
            transform.rotation = base_rot;
        }
    }
}

fn update_aircraft_labels(
    zoom_state: Res<ZoomState>,
    aircraft_query: Query<&Transform, With<Aircraft>>,
    mut label_query: Query<(&AircraftLabel, &mut Transform), Without<Aircraft>>,
) {
    let world_space_offset = constants::LABEL_SCREEN_OFFSET / zoom_state.camera_zoom;

    for (label, mut label_transform) in label_query.iter_mut() {
        if let Ok(aircraft_transform) = aircraft_query.get(label.aircraft_entity) {
            label_transform.translation.x = aircraft_transform.translation.x + world_space_offset;
            label_transform.translation.y = aircraft_transform.translation.y + world_space_offset;
        }
    }
}

/// Hide aircraft (and their entire scene hierarchy) when they are outside
/// the camera viewport. Setting Visibility::Hidden on the root entity
/// causes Bevy to skip rendering all child mesh/material entities,
/// which is the primary performance win for off-screen aircraft.
fn cull_offscreen_aircraft(
    camera_query: Query<(&Transform, &Projection), With<MapCamera>>,
    mut aircraft_query: Query<(&Transform, &mut Visibility), (With<Aircraft>, Without<MapCamera>)>,
    mut label_query: Query<(&AircraftLabel, &mut Visibility), (Without<Aircraft>, Without<MapCamera>)>,
    window_query: Query<&Window>,
    view3d_state: Res<view3d::View3DState>,
) {
    // In 3D mode, perspective frustum culling is handled by Bevy's built-in
    // system via Aabb, so we only do manual viewport culling in 2D.
    if view3d_state.is_3d_active() || view3d_state.is_transitioning() {
        return;
    }

    let Ok((camera_tf, projection)) = camera_query.single() else {
        return;
    };
    let Ok(window) = window_query.single() else {
        return;
    };

    let ortho_scale = if let Projection::Orthographic(ref ortho) = projection {
        ortho.scale
    } else {
        1.0
    };

    let margin = 1.3;
    let half_w = (window.width() / 2.0) * ortho_scale * margin;
    let half_h = (window.height() / 2.0) * ortho_scale * margin;
    let cam_x = camera_tf.translation.x;
    let cam_y = camera_tf.translation.y;

    for (transform, mut visibility) in aircraft_query.iter_mut() {
        let dx = (transform.translation.x - cam_x).abs();
        let dy = (transform.translation.y - cam_y).abs();
        let in_view = dx < half_w && dy < half_h;

        let target = if in_view { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != target {
            *visibility = target;
        }
    }

    for (label, mut visibility) in label_query.iter_mut() {
        if let Ok((ac_tf, _)) = aircraft_query.get(label.aircraft_entity) {
            let dx = (ac_tf.translation.x - cam_x).abs();
            let dy = (ac_tf.translation.y - cam_y).abs();
            let in_view = dx < half_w && dy < half_h;
            let target = if in_view { Visibility::Inherited } else { Visibility::Hidden };
            if *visibility != target {
                *visibility = target;
            }
        }
    }
}
