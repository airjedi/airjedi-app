//! 3D View Mode Module
//!
//! Provides a tilted perspective view showing aircraft at their altitudes
//! above a flat map plane. Uses Camera2d with perspective projection so that
//! all existing 2D content (tiles, trails, sprites) renders correctly.
//! Aircraft altitude is shown by adjusting sprite Z positions.

pub mod sky;

use bevy::prelude::*;

/// Convert a position from Z-up (X=east, Y=north, Z=up) to
/// Y-up (X=east, Y=up, Z=south) coordinate space.
pub(crate) fn zup_to_yup(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

/// Convert a position from Y-up back to Z-up coordinate space.
pub(crate) fn yup_to_zup(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.z, v.y)
}

/// Build the rotation quaternion that transforms Z-up to Y-up.
/// This is a -90 degree rotation around the X axis.
pub(crate) fn zup_to_yup_rotation() -> Quat {
    Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
}
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::input::gestures::PinchGesture;
use bevy_egui::{egui, EguiContexts};

// Constants for 3D view
const TRANSITION_DURATION: f32 = 2.0;
const DEFAULT_PITCH: f32 = 25.0;
const DEFAULT_CAMERA_ALTITUDE: f32 = 10000.0;
const MIN_PITCH: f32 = -89.9;
const MAX_PITCH: f32 = 89.9;
const MIN_CAMERA_ALTITUDE: f32 = 1000.0;
const MAX_CAMERA_ALTITUDE: f32 = 120000.0;
const ALTITUDE_EXAGGERATION: f32 = 20.0;
pub(crate) const CHASE_OFFSET_BEHIND_FT: f32 = 8000.0;
pub(crate) const CHASE_OFFSET_ABOVE_FT: f32 = 2000.0;
pub(crate) const CHASE_PITCH: f32 = 5.0;
pub(crate) const CHASE_TRANSITION_DURATION: f32 = 2.0;

/// Scale factor to convert altitude/distance values to pixel-space.
pub(crate) const PIXEL_SCALE: f32 = 20.0;

/// View mode for the application
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Reflect)]
pub enum ViewMode {
    #[default]
    Map2D,
    Perspective3D,
}

/// Transition state between view modes
#[derive(Debug, Clone, Copy, PartialEq, Default, Reflect)]
pub enum TransitionState {
    #[default]
    Idle,
    TransitioningTo3D { progress: f32 },
    TransitioningTo2D { progress: f32 },
}

/// Resource for 3D view state
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct View3DState {
    pub mode: ViewMode,
    pub transition: TransitionState,
    pub camera_pitch: f32,
    pub camera_altitude: f32,
    pub camera_yaw: f32,
    pub altitude_scale: f32,
    /// Saved 2D camera position (pixel coords) when entering 3D mode
    pub saved_2d_center: Vec2,
    /// Ground plane elevation in feet ASL (from nearest airport)
    pub ground_elevation_ft: i32,
    /// Name of the detected nearest airport (for UI display)
    pub detected_airport_name: Option<String>,
    /// Distance (world units) before fog reaches full opacity
    pub visibility_range: f32,
    /// Whether atmosphere effects (scattering, fog, exposure) are enabled
    pub atmosphere_enabled: bool,
    /// Accumulated drag distance since mouse-down (for click vs drag disambiguation)
    #[reflect(ignore)]
    pub drag_accumulated: f32,
    /// Whether the current drag has exceeded the dead zone threshold
    #[reflect(ignore)]
    pub drag_active: bool,
    /// When following an aircraft, its altitude in feet for orbit center
    pub follow_altitude_ft: Option<i32>,
    /// Saved 2D zoom level when entering 3D mode, restored on return
    pub saved_2d_zoom_level: Option<u8>,
    /// Whether the camera is in chase mode (tracking aircraft heading)
    pub chase_active: bool,
    /// Progress of the initial transition into chase position (0.0 to 1.0)
    pub chase_transition: f32,
    /// Saved orbit parameters from before chase started
    pub pre_chase_pitch: f32,
    pub pre_chase_yaw: f32,
    pub pre_chase_altitude: f32,
    /// User orbited/scrolled during chase — keep position tracking but stop heading tracking
    pub chase_orbit_override: bool,
}

/// Minimum mouse movement (pixels) before a click becomes a drag.
/// Allows picking to work even with slight mouse movement during click.
const DRAG_DEAD_ZONE: f32 = 5.0;

impl Default for View3DState {
    fn default() -> Self {
        Self {
            mode: ViewMode::Map2D,
            transition: TransitionState::Idle,
            camera_pitch: DEFAULT_PITCH,
            camera_altitude: DEFAULT_CAMERA_ALTITUDE,
            camera_yaw: 0.0,
            altitude_scale: ALTITUDE_EXAGGERATION,
            saved_2d_center: Vec2::ZERO,
            ground_elevation_ft: 0,
            detected_airport_name: None,
            visibility_range: 5000.0,
            atmosphere_enabled: true,
            drag_accumulated: 0.0,
            drag_active: false,
            follow_altitude_ft: None,
            saved_2d_zoom_level: None,
            chase_active: false,
            chase_transition: 0.0,
            pre_chase_pitch: DEFAULT_PITCH,
            pre_chase_yaw: 0.0,
            pre_chase_altitude: DEFAULT_CAMERA_ALTITUDE,
            chase_orbit_override: false,
        }
    }
}

impl View3DState {
    pub fn is_3d_active(&self) -> bool {
        matches!(self.mode, ViewMode::Perspective3D)
            || matches!(self.transition, TransitionState::TransitioningTo3D { .. })
    }

    pub fn is_transitioning(&self) -> bool {
        !matches!(self.transition, TransitionState::Idle)
    }

    /// Convert altitude in feet to pixel-space Z offset
    pub fn altitude_to_z(&self, altitude_feet: i32) -> f32 {
        // Convert feet to km, then scale to pixel space
        let alt_km = altitude_feet as f32 * 0.3048 / 1000.0;
        alt_km * PIXEL_SCALE * self.altitude_scale
    }

    /// Convert camera altitude in feet to pixel-space vertical height
    pub fn altitude_to_distance(&self) -> f32 {
        let alt_km = self.camera_altitude * 0.3048 / 1000.0;
        alt_km * PIXEL_SCALE * self.altitude_scale
    }

    /// Calculate the 3D camera transform in Y-up space.
    /// The orbit center is provided in Y-up coordinates.
    ///
    /// `camera_altitude` represents the true vertical altitude above the orbit
    /// center. The orbit distance is derived so the camera stays at the stated
    /// altitude regardless of pitch angle. At low pitch the horizontal distance
    /// increases naturally (shallow viewing angle from altitude).
    fn calculate_camera_transform_yup(&self, center: Vec3) -> Transform {
        let pitch_rad = self.camera_pitch.to_radians();
        let yaw_rad = self.camera_yaw.to_radians();

        // camera_altitude is true vertical height above orbit center.
        // Derive orbit distance so vertical component always equals altitude.
        // Clamp sin(pitch) to prevent infinite distance at very low angles.
        let vertical_dist = self.altitude_to_distance();
        let min_sin = 5.0_f32.to_radians().sin(); // ~0.087, caps orbit at ~11.5x altitude
        let clamped_sin = pitch_rad.sin().max(min_sin);
        let orbit_distance = vertical_dist / clamped_sin;
        let horizontal_dist = orbit_distance * pitch_rad.cos();

        // Y is "up" (altitude), orbit in XZ plane.
        // At yaw=0, camera is south of center (+Z direction in Y-up)
        // looking north (-Z), so north stays up on screen.
        let camera_pos = Vec3::new(
            center.x - horizontal_dist * yaw_rad.sin(),
            center.y + vertical_dist,
            center.z + horizontal_dist * yaw_rad.cos(),
        );

        Transform::from_translation(camera_pos).looking_at(center, Vec3::Y)
    }

    /// Calculate chase camera transform in Y-up space.
    /// Places camera at a fixed offset behind and above the orbit center,
    /// rotated by the chase yaw, with a fixed downward pitch.
    fn calculate_chase_transform_yup(&self, center: Vec3) -> Transform {
        let yaw_rad = self.camera_yaw.to_radians();

        let behind_dist = CHASE_OFFSET_BEHIND_FT * 0.3048 / 1000.0 * PIXEL_SCALE * self.altitude_scale;
        let above_dist = CHASE_OFFSET_ABOVE_FT * 0.3048 / 1000.0 * PIXEL_SCALE * self.altitude_scale;

        // Camera position: behind along yaw direction, above center
        // At yaw=0, camera is south (+Z in Y-up), looking north (-Z)
        let camera_pos = Vec3::new(
            center.x - behind_dist * yaw_rad.sin(),
            center.y + above_dist,
            center.z + behind_dist * yaw_rad.cos(),
        );

        // Look at a point ahead of center for the downward pitch effect
        let pitch_rad = CHASE_PITCH.to_radians();
        let look_ahead_dist = behind_dist * 2.0;
        let look_target = Vec3::new(
            center.x + look_ahead_dist * yaw_rad.sin(),
            center.y - look_ahead_dist * pitch_rad.tan(),
            center.z - look_ahead_dist * yaw_rad.cos(),
        );

        Transform::from_translation(camera_pos).looking_at(look_target, Vec3::Y)
    }
}

/// System to toggle 3D view mode with smooth transition
pub fn toggle_3d_view(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<View3DState>,
    mut contexts: EguiContexts,
    camera_query: Query<&Transform, With<crate::MapCamera>>,
    map_state: Res<crate::MapState>,
    aviation_data: Res<crate::aviation::AviationData>,
) {
    let egui_wants_input = contexts.ctx_mut()
        .map(|ctx| ctx.wants_keyboard_input())
        .unwrap_or(false);

    if egui_wants_input {
        return;
    }

    if keyboard.just_pressed(KeyCode::Digit3) {
        // Don't start new transition if one is in progress
        if state.is_transitioning() {
            return;
        }

        match state.mode {
            ViewMode::Map2D => {
                // Save current 2D camera center before transitioning
                if let Ok(cam_transform) = camera_query.single() {
                    state.saved_2d_center = Vec2::new(
                        cam_transform.translation.x,
                        cam_transform.translation.y,
                    );
                }

                // Save the 2D zoom level so we can restore it when returning
                state.saved_2d_zoom_level = Some(map_state.zoom_level.to_u8());

                // Auto-detect ground elevation from nearest airport
                detect_ground_elevation(&mut state, &map_state, &aviation_data);

                state.transition = TransitionState::TransitioningTo3D { progress: 0.0 };
                info!("Starting transition to 3D view (ground elevation: {} ft)", state.ground_elevation_ft);
            }
            ViewMode::Perspective3D => {
                state.transition = TransitionState::TransitioningTo2D { progress: 0.0 };
                info!("Starting transition to 2D view");
            }
        }
    }
}

/// Find the nearest airport to the current map center and set ground elevation.
fn detect_ground_elevation(
    state: &mut View3DState,
    map_state: &crate::MapState,
    aviation_data: &crate::aviation::AviationData,
) {
    use crate::geo::haversine_distance_nm;

    let center_lat = map_state.latitude;
    let center_lon = map_state.longitude;

    let mut best_dist = f64::MAX;
    let mut best_elevation: i32 = 0;
    let mut best_name: Option<String> = None;

    for airport in &aviation_data.airports {
        let dist = haversine_distance_nm(center_lat, center_lon, airport.latitude_deg, airport.longitude_deg);
        if dist < best_dist && dist <= 50.0 {
            best_dist = dist;
            best_elevation = airport.elevation_ft.unwrap_or(0);
            best_name = Some(format!("{} ({})", airport.name, airport.ident));
        }
    }

    if best_name.is_some() {
        state.ground_elevation_ft = best_elevation;
        state.detected_airport_name = best_name;
    } else {
        state.ground_elevation_ft = 0;
        state.detected_airport_name = None;
    }
}

/// Render the "Time of Day" UI section (shared between panel and dock tab).
pub fn render_time_of_day_section(
    ui: &mut egui::Ui,
    time_state: &mut sky::TimeState,
    sun_state: &sky::SunState,
) {
    ui.heading("Time of Day");

    let mut manual = time_state.is_manual();
    if ui.checkbox(&mut manual, "Manual time override").changed() {
        if manual {
            // Initialize override to the current moment in the local timezone
            // (based on map longitude). Using set_hour() here would cause a
            // timezone mismatch: current_datetime() returns UTC, but set_hour()
            // interprets the hour as local time, causing a multi-hour time jump
            // that changes sun position and breaks rendering.
            let offset_secs = (time_state.utc_offset_hours * 3600.0) as i32;
            let offset = chrono::FixedOffset::east_opt(offset_secs)
                .unwrap_or(chrono::FixedOffset::east_opt(0).unwrap());
            time_state.override_time = Some(chrono::Utc::now().with_timezone(&offset));
        } else {
            time_state.reset_to_live();
        }
    }

    if time_state.is_manual() {
        use chrono::Timelike;
        let current = time_state.current_datetime();
        let mut hour = current.hour() as f32 + current.minute() as f32 / 60.0;

        let h = hour.floor() as u32;
        let m = ((hour.fract()) * 60.0).floor() as u32;
        let time_label = format!("{:02}:{:02}", h, m);

        ui.horizontal(|ui| {
            ui.label("Time:");
            if ui.add(
                egui::Slider::new(&mut hour, 0.0..=23.99)
                    .text(time_label)
                    .step_by(1.0 / 60.0),
            ).changed() {
                time_state.set_hour(hour);
            }
        });
    } else {
        use chrono::Timelike;
        let now = time_state.current_datetime();
        ui.label(
            egui::RichText::new(format!(
                "Live: {:02}:{:02}:{:02} UTC{:+.0}",
                now.hour(),
                now.minute(),
                now.second(),
                time_state.utc_offset_hours,
            ))
            .size(11.0)
            .color(egui::Color32::LIGHT_GREEN),
        );
    }

    // Sun elevation display with twilight zone label
    let elev = sun_state.elevation;
    let zone = if elev > 0.0 {
        "Day"
    } else if elev > -6.0 {
        "Civil twilight"
    } else if elev > -12.0 {
        "Nautical twilight"
    } else if elev > -18.0 {
        "Astronomical twilight"
    } else {
        "Night"
    };

    ui.horizontal(|ui| {
        ui.label("Sun:");
        ui.label(
            egui::RichText::new(format!("{:.1}\u{00B0} ({})", elev, zone))
                .size(11.0)
                .color(if elev > 0.0 {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::LIGHT_BLUE
                }),
        );
    });
}

/// System to animate the view transition
pub fn animate_view_transition(
    time: Res<Time>,
    mut state: ResMut<View3DState>,
) {
    let delta = time.delta_secs() / TRANSITION_DURATION;

    match state.transition {
        TransitionState::TransitioningTo3D { progress } => {
            let new_progress = (progress + delta).min(1.0);
            if new_progress >= 1.0 {
                state.mode = ViewMode::Perspective3D;
                state.transition = TransitionState::Idle;
                info!("Transition to 3D complete");
            } else {
                state.transition = TransitionState::TransitioningTo3D { progress: new_progress };
            }
        }
        TransitionState::TransitioningTo2D { progress } => {
            let new_progress = (progress + delta).min(1.0);
            // Don't finalize here — let update_3d_camera reset the camera
            // before clearing the transition state, avoiding the one-frame
            // race where the early return skips the camera reset.
            state.transition = TransitionState::TransitioningTo2D { progress: new_progress };
        }
        TransitionState::Idle => {}
    }
}

/// System to update cameras for 3D perspective view.
/// Camera3d is primary in Y-up space; Camera2d derives via rotation for tile rendering.
///
/// The orbit center is computed from geographic coordinates (`map_state.latitude/longitude`)
/// using the current zoom level, ensuring the camera is always in the same coordinate
/// system as entity positions (aircraft, airports, etc.), regardless of system execution
/// order within the frame.
pub fn update_3d_camera(
    mut state: ResMut<View3DState>,
    mut camera_2d: Query<
        (&mut Transform, &mut Projection),
        (With<crate::MapCamera>, Without<crate::AircraftCamera>),
    >,
    mut camera_3d: Query<
        (&mut Transform, &mut Projection),
        (With<crate::AircraftCamera>, Without<crate::MapCamera>),
    >,
    window_query: Query<&Window>,
    zoom_state: Res<crate::ZoomState>,
    map_state: Res<crate::MapState>,
    tile_settings: Res<bevy_slippy_tiles::SlippyTilesSettings>,
) {
    if matches!(state.mode, ViewMode::Map2D) && !state.is_transitioning() {
        return;
    }

    let Ok((mut tf_2d, mut proj_2d)) = camera_2d.single_mut() else {
        return;
    };
    let Ok((mut tf_3d, mut proj_3d)) = camera_3d.single_mut() else {
        return;
    };

    let t = match state.transition {
        TransitionState::Idle => match state.mode {
            ViewMode::Map2D => 0.0,
            ViewMode::Perspective3D => 1.0,
        },
        TransitionState::TransitioningTo3D { progress } => smooth_step(progress),
        TransitionState::TransitioningTo2D { progress } => smooth_step(1.0 - progress),
    };

    // Compute orbit center from geographic coordinates at the current zoom level.
    // This ensures the camera is in the same pixel-space coordinate system as
    // all entity positions (which also use CoordinateConverter with map_state.zoom_level).
    // Using lat/lon as the authoritative source eliminates frame-ordering issues
    // where saved_2d_center might be at a different zoom's scale than entities.
    let converter = crate::geo::CoordinateConverter::new(&tile_settings, map_state.zoom_level);
    let center_2d = converter.latlon_to_world(map_state.latitude, map_state.longitude);

    // When following an aircraft, orbit around its altitude instead of ground.
    let orbit_alt_ft = state.follow_altitude_ft.unwrap_or(state.ground_elevation_ft);
    let orbit_alt = state.altitude_to_z(orbit_alt_ft);
    let center_yup = zup_to_yup(Vec3::new(
        center_2d.x,
        center_2d.y,
        orbit_alt,
    ));
    let orbit_yup = if state.chase_active {
        let t = smooth_step(state.chase_transition);
        let orbit = state.calculate_camera_transform_yup(center_yup);
        let chase = state.calculate_chase_transform_yup(center_yup);
        Transform {
            translation: orbit.translation.lerp(chase.translation, t),
            rotation: orbit.rotation.slerp(chase.rotation, t),
            scale: Vec3::ONE,
        }
    } else {
        state.calculate_camera_transform_yup(center_yup)
    };

    // Matching height: perspective altitude that shows the same area as orthographic
    let base_fov = 60.0_f32.to_radians();
    let matching_height = if let Ok(window) = window_query.single() {
        window.height() / (2.0 * zoom_state.camera_zoom * (base_fov / 2.0).tan())
    } else {
        orbit_yup.translation.y * 0.5
    };

    if t < 0.001 {
        // Pure 2D — restore orthographic, flat position, identity rotation
        let pos_2d = Vec3::new(center_2d.x, center_2d.y, 0.0);
        *proj_2d = Projection::Orthographic(OrthographicProjection::default_2d());
        tf_2d.translation = pos_2d;
        tf_2d.rotation = Quat::IDENTITY;

        // Camera3d mirrors Camera2d in 2D mode
        *tf_3d = *tf_2d;
        *proj_3d = proj_2d.clone();

        if matches!(state.transition, TransitionState::TransitioningTo2D { .. }) {
            state.mode = ViewMode::Map2D;
            state.transition = TransitionState::Idle;
            info!("Transition to 2D complete");
        }
        return;
    }

    let perspective = PerspectiveProjection {
        fov: base_fov,
        far: 100_000.0,
        ..default()
    };

    if t > 0.999 {
        // Pure 3D — Camera3d at Y-up orbit, Camera2d derived via rotation
        *tf_3d = orbit_yup;
        *proj_3d = Projection::Perspective(perspective.clone());

        // Derive Camera2d: rotate Y-up transform to Z-up for tile rendering
        let rotation = zup_to_yup_rotation().inverse(); // Y-up -> Z-up
        tf_2d.translation = yup_to_zup(tf_3d.translation);
        tf_2d.rotation = rotation * tf_3d.rotation;
        *proj_2d = Projection::Perspective(perspective);
    } else {
        // Transition: arc the camera from overhead to orbit, always looking at center.
        //
        // Interpolate XZ position linearly but arc the altitude so the camera
        // stays high and sweeps down in a gentle curve. This prevents the
        // camera from showing the entire map or dipping below ground.
        let orbit_pos = orbit_yup.translation;
        let start_height = matching_height.max(orbit_pos.y - center_yup.y);
        let end_height = orbit_pos.y - center_yup.y;

        // Enforce minimum 5,000 ft AGL during transition
        let min_height = state.altitude_to_z(5_000);

        // Arc: blend heights with a sine curve so the camera stays high
        // through mid-transition, then descends to the orbit altitude.
        // At t=0: start_height, at t=0.5: biased toward start, at t=1: end_height
        let arc_blend = (t * std::f32::consts::FRAC_PI_2).sin(); // 0→1, slow start fast end
        let height = start_height + (end_height - start_height) * arc_blend;
        let height = height.max(min_height);

        // XZ: lerp horizontally from directly above to the orbit offset
        let start_xz = Vec3::new(center_yup.x, 0.0, center_yup.z);
        let end_xz = Vec3::new(orbit_pos.x, 0.0, orbit_pos.z);
        let xz = start_xz.lerp(end_xz, t);

        tf_3d.translation = Vec3::new(xz.x, center_yup.y + height, xz.z);

        // Always look at the orbit center for a stable transition
        let up = if height > (orbit_pos - center_yup).length() * 0.95 {
            // Near-overhead: use north as up to avoid gimbal flip
            Vec3::NEG_Z
        } else {
            Vec3::Y
        };
        tf_3d.rotation = Transform::from_translation(tf_3d.translation)
            .looking_at(center_yup, up)
            .rotation;

        *proj_3d = Projection::Perspective(perspective.clone());

        // Derive Camera2d from Camera3d
        let rotation = zup_to_yup_rotation().inverse();
        tf_2d.translation = yup_to_zup(tf_3d.translation);
        tf_2d.rotation = rotation * tf_3d.rotation;
        *proj_2d = Projection::Perspective(perspective);
    }
}

/// Smooth step function for easing transitions
fn smooth_step(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

const ORBIT_SENSITIVITY: f32 = 0.3;
const PAN_3D_SENSITIVITY: f32 = 0.003;
const PITCH_SCROLL_SENSITIVITY: f32 = 2.0;
const ALTITUDE_SCROLL_SENSITIVITY: f32 = 1000.0;

/// System to handle 3D camera controls.
///
/// - **Click+drag**: Pan (translate camera and target in XY, no rotation)
/// - **Shift+click+drag**: Orbit (rotate yaw and pitch around target)
/// - **Scroll**: Change camera altitude (zoom in/out)
/// - **Shift+scroll**: Change camera pitch
/// - **Pinch**: Change camera altitude
pub fn handle_3d_camera_controls(
    mouse_button: Res<ButtonInput<MouseButton>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut mouse_motion: MessageReader<MouseMotion>,
    mut scroll_events: MessageReader<MouseWheel>,
    mut pinch_events: MessageReader<PinchGesture>,
    mut state: ResMut<View3DState>,
    mut map_state: ResMut<crate::MapState>,
    mut follow_state: ResMut<crate::aircraft::CameraFollowState>,
    tile_settings: Res<bevy_slippy_tiles::SlippyTilesSettings>,
    mut contexts: EguiContexts,
    dock_state: Res<crate::dock::DockTreeState>,
) {
    // Only active in 3D mode
    if !matches!(state.mode, ViewMode::Perspective3D) {
        mouse_motion.clear();
        scroll_events.clear();
        pinch_events.clear();
        return;
    }

    // Read shift state from egui's input (bevy_egui absorbs modifier keys from ButtonInput)
    let shift_held = contexts.ctx_mut()
        .map(|ctx| ctx.input(|i| i.modifiers.shift))
        .unwrap_or(false);

    // Don't process input when pointer is over UI panels (allow over map viewport)
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.is_pointer_over_area() {
            let over_map = if let Some(map_rect) = dock_state.map_viewport_rect {
                ctx.pointer_latest_pos().is_some_and(|pos| map_rect.contains(pos))
            } else {
                false
            };
            if !over_map {
                mouse_motion.clear();
                scroll_events.clear();
                pinch_events.clear();
                return;
            }
        }
    }

    // Mouse drag handling with dead zone for click vs drag disambiguation.
    // Small movements (< DRAG_DEAD_ZONE pixels) are ignored so picking can
    // detect clicks even with slight trackpad movement.
    if mouse_button.just_pressed(MouseButton::Left) {
        state.drag_accumulated = 0.0;
        state.drag_active = false;
    }

    if mouse_button.pressed(MouseButton::Left) {
        for event in mouse_motion.read() {
            state.drag_accumulated += event.delta.length();

            if !state.drag_active && state.drag_accumulated < DRAG_DEAD_ZONE {
                continue; // Still in dead zone — let picking handle this as a click
            }
            state.drag_active = true;

            if shift_held {
                // Shift+drag = Orbit (rotate around target)
                // Override chase heading tracking so user orbit sticks
                if state.chase_active && !state.chase_orbit_override {
                    state.chase_orbit_override = true;
                }
                state.camera_yaw += event.delta.x * ORBIT_SENSITIVITY;
                if state.camera_yaw < 0.0 { state.camera_yaw += 360.0; }
                if state.camera_yaw >= 360.0 { state.camera_yaw -= 360.0; }
                state.camera_pitch = (state.camera_pitch - event.delta.y * ORBIT_SENSITIVITY)
                    .clamp(MIN_PITCH, MAX_PITCH);
            } else {
                // Plain drag = Pan (translate XY only, no rotation)
                if follow_state.following_icao.is_some() {
                    follow_state.following_icao = None;
                }

                let pan_speed = state.altitude_to_distance() * PAN_3D_SENSITIVITY;
                let yaw_rad = state.camera_yaw.to_radians();

                // Camera basis vectors projected onto the ground plane.
                // At yaw=0 the camera is south of center looking north, so
                // camera-right = east (+X) and camera-forward = north (+Y).
                let cam_right_x = yaw_rad.cos();
                let cam_right_y = -yaw_rad.sin();
                let cam_fwd_x = yaw_rad.sin();
                let cam_fwd_y = yaw_rad.cos();

                // Negate X: dragging right moves the map right (center left).
                // Y is NOT negated so dragging toward the top moves the view backward.
                let dx = -event.delta.x * pan_speed;
                let dy = event.delta.y * pan_speed;

                state.saved_2d_center.x += dx * cam_right_x + dy * cam_fwd_x;
                state.saved_2d_center.y += dx * cam_right_y + dy * cam_fwd_y;

                sync_center_to_map_state(&state, &tile_settings, &mut map_state);
            }
        }
    } else {
        mouse_motion.clear();
    }

    // Scroll = altitude (zoom), Shift+Scroll = pitch.
    // On macOS, shift+scroll is converted to horizontal scroll by the OS and absorbed
    // by bevy_egui, so we read shift+scroll from egui's input directly.
    if shift_held {
        if let Ok(ctx) = contexts.ctx_mut() {
            let scroll_delta = ctx.input(|i| i.smooth_scroll_delta);
            // macOS shift+scroll arrives as horizontal delta
            let scroll_y = if scroll_delta.y.abs() > scroll_delta.x.abs() {
                scroll_delta.y
            } else {
                scroll_delta.x
            };
            if scroll_y.abs() > 0.1 {
                // Override chase heading tracking so user pitch sticks
                if state.chase_active && !state.chase_orbit_override {
                    state.chase_orbit_override = true;
                }
                let pitch_delta = scroll_y * 0.05;
                state.camera_pitch = (state.camera_pitch + pitch_delta)
                    .clamp(MIN_PITCH, MAX_PITCH);
            }
        }
    } else {
        for event in scroll_events.read() {
            let scroll_y = match event.unit {
                bevy::input::mouse::MouseScrollUnit::Line => event.y,
                bevy::input::mouse::MouseScrollUnit::Pixel => event.y * 0.01,
            };
            // Override chase heading tracking so user altitude sticks
            if state.chase_active && !state.chase_orbit_override {
                state.chase_orbit_override = true;
            }
            state.camera_altitude = (state.camera_altitude - scroll_y * ALTITUDE_SCROLL_SENSITIVITY)
                .clamp(MIN_CAMERA_ALTITUDE, MAX_CAMERA_ALTITUDE);
        }
    }

    // Pinch = altitude (zoom)
    for event in pinch_events.read() {
        // Override chase heading tracking so user altitude sticks
        if state.chase_active && !state.chase_orbit_override {
            state.chase_orbit_override = true;
        }
        state.camera_altitude = (state.camera_altitude * (1.0 - event.0))
            .clamp(MIN_CAMERA_ALTITUDE, MAX_CAMERA_ALTITUDE);
    }
}

/// Convert saved_2d_center (pixel-space offset from tile reference point) back to
/// geographic coordinates and update the shared map state so tiles are loaded.
fn sync_center_to_map_state(
    state: &View3DState,
    tile_settings: &bevy_slippy_tiles::SlippyTilesSettings,
    map_state: &mut crate::MapState,
) {
    use bevy_slippy_tiles::*;

    let reference_ll = LatitudeLongitudeCoordinates {
        latitude: tile_settings.reference_latitude,
        longitude: tile_settings.reference_longitude,
    };
    let reference_pixel = world_coords_to_world_pixel(
        &reference_ll,
        crate::constants::DEFAULT_TILE_SIZE,
        map_state.zoom_level,
    );

    let center_geo = world_pixel_to_world_coords(
        state.saved_2d_center.x as f64 + reference_pixel.0,
        state.saved_2d_center.y as f64 + reference_pixel.1,
        crate::constants::DEFAULT_TILE_SIZE,
        map_state.zoom_level,
    );

    map_state.latitude = crate::clamp_latitude(center_geo.latitude);
    map_state.longitude = crate::clamp_longitude(center_geo.longitude);
}

/// System to raise map tiles to ground elevation in 3D mode.
/// In 2D mode, tiles sit at TILE_Z_LAYER + 0.1; in 3D mode, they are raised
/// to match the ground elevation so the map surface appears at terrain height.
/// Lower-zoom multi-resolution tiles sit slightly below so higher-zoom tiles
/// win depth tests and render on top.
pub fn update_tile_elevation(
    state: Res<View3DState>,
    map_state: Res<crate::MapState>,
    mut tile_query: Query<(&mut Transform, &crate::tiles::TileFadeState), With<bevy_slippy_tiles::MapTile>>,
) {
    if state.is_3d_active() {
        let ground_z = state.altitude_to_z(state.ground_elevation_ft);
        let current_zoom = map_state.zoom_level.to_u8();
        for (mut transform, fade_state) in tile_query.iter_mut() {
            let zoom_diff = current_zoom.saturating_sub(fade_state.tile_zoom);
            transform.translation.z = ground_z - zoom_diff as f32 * 0.05;
        }
    } else if !state.is_transitioning() {
        for (mut transform, _) in tile_query.iter_mut() {
            transform.translation.z = crate::constants::TILE_Z_LAYER + 0.1;
        }
    }
}

/// Remap aircraft transforms to Y-up space in 3D mode.
/// In 2D mode, aircraft Z is the fixed layer constant.
/// In 3D mode, positions are converted from Z-up pixel space (set by
/// update_aircraft_positions) to Y-up for Camera3d rendering.
pub fn update_aircraft_3d_transform(
    state: Res<View3DState>,
    config: Res<crate::config::AppConfig>,
    mut aircraft_query: Query<(&crate::Aircraft, Option<&crate::aircraft::InterpolationState>, &mut Transform), Without<crate::AircraftLabel>>,
    mut label_query: Query<(&crate::AircraftLabel, &mut Visibility)>,
) {
    if state.is_3d_active() {
        let ground_y = state.altitude_to_z(state.ground_elevation_ft);
        let min_aircraft_y = ground_y + 10.0;

        for (aircraft, interp_opt, mut transform) in aircraft_query.iter_mut() {
            let px = transform.translation.x;
            let py = transform.translation.y;

            // Use interpolated altitude/heading if available and enabled
            let (alt, heading) = if config.interpolation_enabled {
                if let Some(interp) = interp_opt {
                    (
                        interp.display_altitude.map(|a| a as i32).unwrap_or(0),
                        interp.display_heading,
                    )
                } else {
                    (aircraft.altitude.unwrap_or(0), aircraft.heading)
                }
            } else {
                (aircraft.altitude.unwrap_or(0), aircraft.heading)
            };

            let alt_y = state.altitude_to_z(alt).max(min_aircraft_y);
            transform.translation = Vec3::new(px, alt_y, -py);

            let base_rot = crate::camera::BASE_ROT_YUP;
            if let Some(heading) = heading {
                transform.rotation =
                    Quat::from_rotation_y((-heading).to_radians()) * base_rot;
            } else {
                transform.rotation = base_rot;
            }
        }
        for (_label, mut vis) in label_query.iter_mut() {
            *vis = Visibility::Hidden;
        }
    } else if !state.is_transitioning() {
        for (_aircraft, _interp, mut transform) in aircraft_query.iter_mut() {
            transform.translation.z = crate::constants::AIRCRAFT_Z_LAYER;
        }
        for (_label, mut vis) in label_query.iter_mut() {
            if *vis == Visibility::Hidden {
                *vis = Visibility::Inherited;
            }
        }
    }
}

/// Fade aircraft sprites based on distance from Camera2d in 3D mode.
/// Tiles are fogged by DistanceFog via their 3D mesh quad companions.
pub fn fade_distant_sprites(
    state: Res<View3DState>,
    camera_query: Query<&Transform, With<crate::MapCamera>>,
    mut aircraft_query: Query<(&Transform, &mut Sprite), (With<crate::Aircraft>, Without<crate::MapCamera>)>,
) {
    if !state.is_3d_active() {
        // Reset aircraft alpha when leaving 3D mode
        for (_, mut sprite) in aircraft_query.iter_mut() {
            sprite.color = Color::srgba(1.0, 1.0, 1.0, 1.0);
        }
        return;
    }

    let Ok(cam_transform) = camera_query.single() else {
        return;
    };

    let cam_pos = cam_transform.translation;

    // Fade range matches the fog: starts at 40% of visibility_range, fully gone at 100%
    let fade_start = state.visibility_range * 0.4;
    let fade_end = state.visibility_range;
    let fade_range = fade_end - fade_start;

    if fade_range <= 0.0 {
        return;
    }

    // Fade aircraft
    for (transform, mut sprite) in aircraft_query.iter_mut() {
        let dist = cam_pos.distance(transform.translation);
        let alpha = if dist <= fade_start {
            1.0
        } else if dist >= fade_end {
            0.0
        } else {
            1.0 - ((dist - fade_start) / fade_range)
        };
        sprite.color = Color::srgba(1.0, 1.0, 1.0, alpha);
    }
}

/// Update DistanceFog falloff when visibility_range changes.
fn update_distance_fog(
    state: Res<View3DState>,
    mut fog_query: Query<&mut DistanceFog, With<Camera3d>>,
) {
    if !state.is_changed() || !state.is_3d_active() {
        return;
    }
    let Ok(mut fog) = fog_query.single_mut() else {
        return;
    };
    fog.falloff = FogFalloff::Linear {
        start: state.visibility_range * 0.4,
        end: state.visibility_range,
    };
}

/// Fix aircraft model materials for the current view mode.
///
/// In 2D mode, materials are set to `unlit = true` so aircraft render at full
/// brightness regardless of scene lighting (sun position, ambient level, etc.).
/// In 3D mode, materials are lit normally so they interact with the sun,
/// atmosphere, and environment lighting.
///
/// Also forces `AlphaMode::Opaque` in all modes. GLB models may export with
/// transparent or alpha-blended materials, which skip depth writes and get
/// overwritten by the atmosphere post-process.
fn fix_aircraft_model_materials(
    state: Res<View3DState>,
    aircraft_query: Query<&Children, With<crate::Aircraft>>,
    children_query: Query<&Children>,
    mesh_query: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let want_unlit = !state.is_3d_active();
    for children in aircraft_query.iter() {
        fix_materials_in_hierarchy(children, &children_query, &mesh_query, &mut materials, want_unlit);
    }
}

fn fix_materials_in_hierarchy(
    children: &Children,
    children_query: &Query<&Children>,
    mesh_query: &Query<&MeshMaterial3d<StandardMaterial>>,
    materials: &mut Assets<StandardMaterial>,
    want_unlit: bool,
) {
    for child in children.iter() {
        if let Ok(mat_handle) = mesh_query.get(child) {
            let needs_fix = materials
                .get(mat_handle.id())
                .is_some_and(|m| {
                    !matches!(m.alpha_mode, AlphaMode::Opaque) || m.unlit != want_unlit
                });
            if needs_fix {
                if let Some(material) = materials.get_mut(mat_handle.id()) {
                    material.alpha_mode = AlphaMode::Opaque;
                    material.unlit = want_unlit;
                }
            }
        }
        if let Ok(grandchildren) = children_query.get(child) {
            fix_materials_in_hierarchy(grandchildren, children_query, mesh_query, materials, want_unlit);
        }
    }
}

/// Plugin for 3D view functionality
pub struct View3DPlugin;

impl Plugin for View3DPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<View3DState>()
            .register_type::<sky::SunState>()
            .register_type::<sky::TimeState>()
            .init_resource::<View3DState>()
            .init_resource::<sky::SunState>()
            .init_resource::<sky::MoonState>()
            .init_resource::<sky::TimeState>()
            .add_systems(Startup, sky::setup_sky)
            .add_systems(Update, (
                toggle_3d_view,
                animate_view_transition,
                handle_3d_camera_controls,
                update_3d_camera
                    .after(animate_view_transition)
                    .after(crate::ZoomSet::Change),
            ))
            .add_systems(Update, update_tile_elevation
                .after(animate_view_transition)
                .after(crate::ZoomSet::Change))
            .add_systems(Update, update_aircraft_3d_transform
                .after(crate::camera::update_aircraft_positions))
            .add_systems(Update, fix_aircraft_model_materials)
            .add_systems(Update, sky::update_sky_visibility)
            .add_systems(Update, sky::sync_sky_camera.after(update_3d_camera))
            .add_systems(Update, sky::sync_time_offset)
            .add_systems(Update, sky::update_sun_position.after(sky::sync_time_offset))
            .add_systems(Update, sky::update_moon_position.after(sky::sync_time_offset))
            .add_systems(Update, sky::update_star_visibility)
            .add_systems(Update, sky::manage_camera_mode
                .after(animate_view_transition)
                .after(update_3d_camera)
                .after(sky::update_sun_position))
            .add_systems(Update, sky::sync_ground_plane.after(update_3d_camera))
            .add_systems(Update, sky::sync_sky_dome.after(update_3d_camera))
            .add_systems(Update, sky::update_sky_dome_colors.after(sky::update_sun_position))
            .add_systems(Update, sky::update_ground_plane_color.after(sky::update_sun_position))
            .add_systems(Update, sky::update_exposure_for_time.after(sky::update_sun_position))
            .add_systems(Update, sky::update_fog_color_for_time.after(sky::update_sun_position))
            .add_systems(Update, fade_distant_sprites
                .after(update_3d_camera)
                .after(update_tile_elevation))
            .add_systems(Update, update_distance_fog
                .after(animate_view_transition))
            .add_systems(Update, crate::hud::render_camera_hud)
            .init_resource::<crate::debug_3d_hud::Debug3DHudState>()
            .add_systems(Update, crate::debug_3d_hud::render_debug_3d_hud);
        // 3D view settings panel is rendered via the consolidated Tools window (tools_window.rs)
    }
}
