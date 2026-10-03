use bevy::prelude::*;
use crate::tiles::*;

use crate::constants;
use crate::geo;
use crate::map::{MapState, ZoomState};
use crate::view3d;
use crate::{clamp_latitude, clamp_longitude, Aircraft, ZoomDebugLogger, ZoomSet};

// =============================================================================
// Constants
// =============================================================================

/// Base rotation for aircraft GLB models in Y-up 3D space.
/// GLB model: nose=+Z, top=+Y, right-wing=+X.
/// Y-up world: north=-Z, up=+Y.
/// Rotate 180 deg around Y so nose points -Z (north).
/// Then heading rotation is applied around Y axis.
pub(crate) const BASE_ROT_YUP: Quat = Quat::from_xyzw(0.0, 1.0, 0.0, 0.0); // 180 deg around Y

/// Upper bound for the 2D-mode aircraft Z offset above the opaque tile plane.
/// `AircraftCamera` syncs its `OrthographicProjection` from `MapCamera`, whose
/// default 2D projection clips at `near: -1000.0, far: 1000.0` around a camera
/// sitting at `z = 0`. Anything offset further than this is silently clipped by
/// the far plane - invisible regardless of `Visibility`/`InheritedVisibility` -
/// so this cap must stay comfortably inside that range even as `scale` grows
/// into the thousands at close zoom levels.
const AIRCRAFT_MAX_Z_OFFSET_2D: f32 = 500.0;

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

/// One-shot animated pan of the map center to a target lat/lon, leaving zoom
/// untouched. Triggered by selecting an aircraft in the list; unlike
/// `CameraFollowState`, it eases toward a fixed point and then stops instead
/// of continuously chasing a moving aircraft.
#[derive(Resource, Default)]
pub(crate) struct PanToTarget {
    pub target: Option<(f64, f64)>,
}

// =============================================================================
// Plugin
// =============================================================================

pub(crate) struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PanToTarget>()
        .add_systems(
            Update,
            follow_aircraft.after(airjedi_fusion::systems::FusionSet::Lifecycle),
        )
        .add_systems(
            Update,
            animate_pan_to_target.after(airjedi_fusion::systems::FusionSet::Lifecycle),
        )
        .add_systems(
            Update,
            update_camera_position
                .after(crate::input::handle_pan_drag)
                .after(crate::zoom::apply_camera_zoom)
                .after(follow_aircraft)
                .after(animate_pan_to_target),
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
                .after(airjedi_fusion::systems::FusionSet::Lifecycle)
                .after(crate::aircraft::interpolation::interpolate_aircraft_positions)
                .after(ZoomSet::Change),
        )
        .add_systems(
            Update,
            scale_aircraft_and_labels
                .after(crate::zoom::apply_camera_zoom)
                .after(view3d::update_aircraft_3d_transform),
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

/// Eases `MapState`'s center toward `PanToTarget`'s target, then clears
/// itself once close enough. Zoom is never touched.
fn animate_pan_to_target(
    mut pan: ResMut<PanToTarget>,
    mut map_state: ResMut<MapState>,
    time: Res<Time>,
) {
    let Some((target_lat, target_lon)) = pan.target else {
        return;
    };

    let lerp_speed = 4.0;
    let t = (lerp_speed * time.delta_secs()).min(1.0) as f64;

    let new_lat = map_state.latitude + (target_lat - map_state.latitude) * t;
    let new_lon = map_state.longitude + (target_lon - map_state.longitude) * t;

    map_state.latitude = clamp_latitude(new_lat);
    map_state.longitude = clamp_longitude(new_lon);

    let remaining = (target_lat - map_state.latitude).hypot(target_lon - map_state.longitude);
    if remaining < 1e-5 {
        map_state.latitude = target_lat;
        map_state.longitude = target_lon;
        pan.target = None;
    }
}

fn update_camera_position(
    map_state: Res<MapState>,
    local_origin: Res<LocalOrigin>,
    mut camera_query: Query<&mut Transform, With<MapCamera>>,
    view3d_state: Res<view3d::View3DState>,
) {
    if view3d_state.is_3d_active() || view3d_state.is_transitioning() {
        return;
    }

    if let Ok(mut camera_transform) = camera_query.single_mut() {
        let converter = geo::CoordinateConverter::new(&local_origin);
        let pos = converter.latlon_to_world(map_state.latitude, map_state.longitude);
        camera_transform.translation.x = pos.x;
        camera_transform.translation.y = pos.y;
    }
}

/// Sync Camera3d transform and projection to match Camera2d in 2D mode.
/// In 3D mode, update_3d_camera handles both cameras directly.
fn sync_aircraft_camera(
    view3d_state: Res<view3d::View3DState>,
    camera_2d: Query<
        (&Transform, &Projection),
        (
            With<MapCamera>,
            Without<AircraftCamera>,
            Without<AircraftCamera2d>,
        ),
    >,
    mut camera_3d: Query<
        (&mut Transform, &mut Projection),
        (
            With<AircraftCamera>,
            Without<AircraftCamera2d>,
            Without<Camera2d>,
        ),
    >,
    mut camera_ac2d: Query<
        (&mut Transform, &mut Projection),
        (
            With<AircraftCamera2d>,
            Without<AircraftCamera>,
            Without<MapCamera>,
        ),
    >,
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

/// Keep aircraft at constant screen size despite zoom changes.
/// In 2D mode, scale inversely with camera zoom for constant screen size.
/// In 3D perspective mode, use a fixed world-space scale and let perspective
/// projection handle apparent size (closer = bigger, farther = smaller).
fn scale_aircraft_and_labels(
    zoom_state: Res<ZoomState>,
    map_state: Res<MapState>,
    view3d_state: Res<crate::view3d::View3DState>,
    mut aircraft_query: Query<&mut Transform, With<Aircraft>>,
    new_aircraft: Query<(), Added<Aircraft>>,
) {
    if !zoom_state.is_changed() && !view3d_state.is_changed() && new_aircraft.is_empty() {
        return;
    }

    let tile_size_meters = (2.0 * crate::tiles::WEB_MERCATOR_EXTENT)
        / (1u64 << map_state.zoom_level.to_u8()) as f64;
    let meters_per_tile_pixel = (tile_size_meters / constants::DEFAULT_TILE_PIXELS as f64) as f32;

    // Blend between 2D and 3D scales during transitions to avoid a size flash
    let t_3d = match view3d_state.transition {
        view3d::TransitionState::TransitioningTo3D { progress } => {
            view3d::smooth_step(progress)
        }
        view3d::TransitionState::TransitioningTo2D { progress } => {
            view3d::smooth_step(1.0 - progress)
        }
        _ if view3d_state.is_3d_active() => 1.0,
        _ => 0.0,
    };

    let scale_2d = constants::AIRCRAFT_MODEL_SCALE * meters_per_tile_pixel / zoom_state.camera_zoom;
    // Perspective mode uses world-space scale. The 2D tile-pixel conversion
    // makes the model grow with zoom and lets the loaded tile surface occlude it.
    let scale_3d = constants::AIRCRAFT_MODEL_SCALE * 10.0;
    let scale = scale_2d + (scale_3d - scale_2d) * t_3d;
    for mut transform in aircraft_query.iter_mut() {
        transform.scale = Vec3::splat(scale);
        if t_3d == 0.0 {
            // Keep the full model in front of the opaque 2D tile plane, but
            // never approach the AircraftCamera's far clip plane (see
            // AIRCRAFT_MAX_Z_OFFSET_2D) - at close zoom levels `scale` grows
            // into the thousands, and multiplying it unbounded pushed every
            // aircraft past the clip range, making them invisible regardless
            // of position or Visibility state.
            transform.translation.z =
                (scale * 12.0).min(AIRCRAFT_MAX_Z_OFFSET_2D).max(constants::AIRCRAFT_Z_LAYER);
        }
    }
}

pub(crate) fn update_aircraft_positions(
    map_state: Res<MapState>,
    local_origin: Res<LocalOrigin>,
    config: Res<crate::config::AppConfig>,
    view3d_state: Res<view3d::View3DState>,
    mut aircraft_query: Query<(
        &Aircraft,
        Option<&crate::aircraft::InterpolationState>,
        &mut Transform,
    )>,
) {
    let converter = geo::CoordinateConverter::new(&local_origin);

    for (aircraft, interp_opt, mut transform) in aircraft_query.iter_mut() {
        // Use interpolated display position if available and enabled, otherwise raw ADS-B
        let (lat, lon, heading) = if config.interpolation_enabled {
            if let Some(interp) = interp_opt {
                (
                    interp.display_lat,
                    interp.display_lon,
                    interp.display_heading,
                )
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

/// Hide aircraft (and their entire scene hierarchy) when they are outside
/// the camera viewport. Setting Visibility::Hidden on the root entity
/// causes Bevy to skip rendering all child mesh/material entities,
/// which is the primary performance win for off-screen aircraft.
fn cull_offscreen_aircraft(
    camera_query: Query<(&Transform, &Projection), With<MapCamera>>,
    mut aircraft_query: Query<(&Transform, &mut Visibility), (With<Aircraft>, Without<MapCamera>)>,
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

        let target = if in_view {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != target {
            *visibility = target;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{MapState, ZoomState};
    use crate::view3d::View3DState;

    fn minimal_aircraft() -> Aircraft {
        Aircraft {
            icao: "TEST01".to_string(),
            callsign: None,
            latitude: 0.0,
            longitude: 0.0,
            altitude: None,
            heading: None,
            velocity: None,
            vertical_rate: None,
            roll_angle: None,
            track_angle_rate: None,
            roll_last_seen: None,
            squawk: None,
            is_on_ground: None,
            alert: None,
            emergency: None,
            spi: None,
            last_seen: chrono::Utc::now(),
        }
    }

    /// Regression test for the b4ad80a z-translation bug: at the minimum
    /// allowed 2D camera zoom, `scale` grows into the thousands, and the
    /// unbounded `scale * 12.0` offset pushed every aircraft's Z translation
    /// far past the AircraftCamera's synced orthographic far clip plane
    /// (1000.0), making aircraft invisible regardless of XY position or
    /// Visibility state. The Z offset must stay bounded inside that range.
    #[test]
    fn aircraft_z_offset_stays_inside_the_2d_camera_clip_range_at_minimum_zoom() {
        let mut app = App::new();
        app.insert_resource(ZoomState {
            camera_zoom: constants::MIN_CAMERA_ZOOM,
            ..ZoomState::new()
        })
        .insert_resource(MapState::default())
        .insert_resource(View3DState::default())
        .add_systems(Update, scale_aircraft_and_labels);

        let entity = app
            .world_mut()
            .spawn((minimal_aircraft(), Transform::default()))
            .id();

        app.update();

        let z = app
            .world()
            .entity(entity)
            .get::<Transform>()
            .unwrap()
            .translation
            .z;
        assert!(
            z <= AIRCRAFT_MAX_Z_OFFSET_2D,
            "aircraft Z offset {z} exceeded the documented cap {AIRCRAFT_MAX_Z_OFFSET_2D}"
        );
        assert!(
            z < 1000.0,
            "aircraft Z offset {z} must stay inside the AircraftCamera's far clip plane (1000.0), \
             or the aircraft will be invisible regardless of Visibility state"
        );
    }

    /// At a sufficiently zoomed-in discrete tile level, the proportional
    /// offset stays below the cap - the clamp only engages at the larger
    /// `scale` values typical of less-zoomed discrete tile levels (see the
    /// minimum-zoom test above), so this confirms the proportional branch
    /// still does something rather than the cap always winning.
    #[test]
    fn aircraft_z_offset_scales_with_model_size_below_the_cap() {
        let mut app = App::new();
        app.insert_resource(ZoomState {
            camera_zoom: constants::MAX_CAMERA_ZOOM,
            ..ZoomState::new()
        })
        .insert_resource(MapState {
            zoom_level: crate::tiles::ZoomLevel::L15,
            ..MapState::default()
        })
        .insert_resource(View3DState::default())
        .add_systems(Update, scale_aircraft_and_labels);

        let entity = app
            .world_mut()
            .spawn((minimal_aircraft(), Transform::default()))
            .id();

        app.update();

        let z = app
            .world()
            .entity(entity)
            .get::<Transform>()
            .unwrap()
            .translation
            .z;
        assert!(z >= constants::AIRCRAFT_Z_LAYER);
        assert!(z < AIRCRAFT_MAX_Z_OFFSET_2D);
    }
}
