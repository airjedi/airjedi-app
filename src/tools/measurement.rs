use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy_slippy_tiles::{
    world_coords_to_world_pixel, world_pixel_to_world_coords, LatitudeLongitudeCoordinates,
};

use crate::geo::{haversine_distance_nm, initial_bearing, NM_TO_KM};
use crate::{MapState, ZoomState};

/// State for the measurement tool
#[derive(Resource, Default)]
pub struct MeasurementState {
    /// Whether measurement mode is active
    pub active: bool,
    /// Start point of measurement (lat, lon)
    pub start_point: Option<(f64, f64)>,
    /// End point of measurement (lat, lon)
    pub end_point: Option<(f64, f64)>,
    /// Current cursor position in lat/lon (for rubber-band line)
    pub cursor_latlon: Option<(f64, f64)>,
}

impl MeasurementState {
    /// Reset measurement state
    pub fn reset(&mut self) {
        self.start_point = None;
        self.end_point = None;
        self.cursor_latlon = None;
    }

    /// Get distance in nautical miles between start and end (or cursor)
    pub fn distance_nm(&self) -> Option<f64> {
        let start = self.start_point?;
        let end = self.end_point.or(self.cursor_latlon)?;
        Some(haversine_distance_nm(start.0, start.1, end.0, end.1))
    }

    /// Get distance in kilometers
    pub fn distance_km(&self) -> Option<f64> {
        self.distance_nm().map(|nm| nm * NM_TO_KM)
    }

    /// Get bearing from start to end (or cursor) in degrees
    pub fn bearing(&self) -> Option<f64> {
        let start = self.start_point?;
        let end = self.end_point.or(self.cursor_latlon)?;
        Some(initial_bearing(start.0, start.1, end.0, end.1))
    }
}

/// Component for measurement line entity
#[derive(Component)]
pub struct MeasurementLine;

/// Component for measurement point markers
#[derive(Component)]
pub struct MeasurementPoint {
    pub is_start: bool,
}

/// Toggle measurement mode with 'M' key
pub fn toggle_measurement_mode(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<MeasurementState>,
    mut contexts: EguiContexts,
    mut commands: Commands,
    line_query: Query<Entity, With<MeasurementLine>>,
    point_query: Query<Entity, With<MeasurementPoint>>,
) {
    // Don't toggle if egui wants input
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    // M key or Escape to toggle/cancel
    if keyboard.just_pressed(KeyCode::KeyM) {
        if state.active {
            // Deactivate and clear
            state.active = false;
            state.reset();
            // Remove visual elements
            for entity in line_query.iter() {
                commands.entity(entity).despawn();
            }
            for entity in point_query.iter() {
                commands.entity(entity).despawn();
            }
            info!("Measurement mode disabled");
        } else {
            state.active = true;
            state.reset();
            info!("Measurement mode enabled - click to set start point");
        }
    } else if keyboard.just_pressed(KeyCode::Escape) && state.active {
        state.active = false;
        state.reset();
        // Remove visual elements
        for entity in line_query.iter() {
            commands.entity(entity).despawn();
        }
        for entity in point_query.iter() {
            commands.entity(entity).despawn();
        }
        info!("Measurement mode cancelled");
    }
}

/// Handle clicks for setting measurement points
pub fn handle_measurement_clicks(
    mut state: ResMut<MeasurementState>,
    mouse_button: Res<ButtonInput<MouseButton>>,
    window_query: Query<&Window>,
    camera_query: Query<(&Camera, &GlobalTransform), With<crate::MapCamera>>,
    map_state: Res<MapState>,
    zoom_state: Res<ZoomState>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut contexts: EguiContexts,
) {
    if !state.active {
        return;
    }

    // Check if egui wants pointer
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_pointer_input() {
            return;
        }
    }

    let Ok(window) = window_query.single() else {
        return;
    };

    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };

    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };

    // Convert cursor position to world coordinates
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else {
        return;
    };

    // Convert world position to lat/lon
    let center_coords = LatitudeLongitudeCoordinates {
        latitude: map_state.latitude,
        longitude: map_state.longitude,
    };
    let (cx, cy) = world_coords_to_world_pixel(
        &center_coords,
        crate::constants::DEFAULT_TILE_SIZE,
        map_state.zoom_level,
    );

    // Calculate offset from center in world pixels
    let offset_x = world_pos.x as f64 / zoom_state.camera_zoom as f64;
    let offset_y = -world_pos.y as f64 / zoom_state.camera_zoom as f64; // Y is inverted

    let cursor_geo = world_pixel_to_world_coords(
        cx + offset_x,
        cy + offset_y,
        crate::constants::DEFAULT_TILE_SIZE,
        map_state.zoom_level,
    );

    // Update cursor position for rubber-band line
    state.cursor_latlon = Some((cursor_geo.latitude, cursor_geo.longitude));

    // Handle clicks
    if mouse_button.just_pressed(MouseButton::Left) {
        if state.start_point.is_none() {
            // Set start point
            state.start_point = Some((cursor_geo.latitude, cursor_geo.longitude));
            info!(
                "Measurement start: {:.4}, {:.4}",
                cursor_geo.latitude, cursor_geo.longitude
            );

            // Spawn start point marker
            commands.spawn((
                Mesh2d(meshes.add(Circle::new(5.0))),
                MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.0, 1.0, 0.0)))),
                Transform::from_xyz(world_pos.x, world_pos.y, 15.0),
                MeasurementPoint { is_start: true },
            ));
        } else if state.end_point.is_none() {
            // Set end point
            state.end_point = Some((cursor_geo.latitude, cursor_geo.longitude));
            info!(
                "Measurement end: {:.4}, {:.4}",
                cursor_geo.latitude, cursor_geo.longitude
            );

            // Spawn end point marker
            commands.spawn((
                Mesh2d(meshes.add(Circle::new(5.0))),
                MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(1.0, 0.0, 0.0)))),
                Transform::from_xyz(world_pos.x, world_pos.y, 15.0),
                MeasurementPoint { is_start: false },
            ));

            // Log result
            if let (Some(dist_nm), Some(bearing)) = (state.distance_nm(), state.bearing()) {
                let dist_km = state.distance_km().unwrap_or(0.0);
                info!(
                    "Distance: {:.2} nm ({:.2} km), Bearing: {:.0}",
                    dist_nm, dist_km, bearing
                );
            }
        } else {
            // Reset for new measurement
            state.reset();
        }
    }
}

/// Update measurement line visual
pub fn update_measurement_line(
    state: Res<MeasurementState>,
    map_state: Res<MapState>,
    zoom_state: Res<ZoomState>,
    mut commands: Commands,
    mut line_query: Query<(Entity, &mut Transform), With<MeasurementLine>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if !state.active || state.start_point.is_none() {
        // Remove line if not active or no start point
        for (entity, _) in line_query.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }

    let start = state.start_point.unwrap();
    let end = state.end_point.or(state.cursor_latlon);

    let Some(end) = end else {
        return;
    };

    // Convert lat/lon to screen coordinates
    let center_coords = LatitudeLongitudeCoordinates {
        latitude: map_state.latitude,
        longitude: map_state.longitude,
    };
    let (cx, cy) = world_coords_to_world_pixel(
        &center_coords,
        crate::constants::DEFAULT_TILE_SIZE,
        map_state.zoom_level,
    );

    let start_coords = LatitudeLongitudeCoordinates {
        latitude: start.0,
        longitude: start.1,
    };
    let (sx, sy) = world_coords_to_world_pixel(
        &start_coords,
        crate::constants::DEFAULT_TILE_SIZE,
        map_state.zoom_level,
    );

    let end_coords = LatitudeLongitudeCoordinates {
        latitude: end.0,
        longitude: end.1,
    };
    let (ex, ey) = world_coords_to_world_pixel(
        &end_coords,
        crate::constants::DEFAULT_TILE_SIZE,
        map_state.zoom_level,
    );

    // Calculate screen positions
    let start_screen = Vec2::new(
        (sx - cx) as f32 * zoom_state.camera_zoom,
        -(sy - cy) as f32 * zoom_state.camera_zoom,
    );
    let end_screen = Vec2::new(
        (ex - cx) as f32 * zoom_state.camera_zoom,
        -(ey - cy) as f32 * zoom_state.camera_zoom,
    );

    // Calculate line properties
    let midpoint = (start_screen + end_screen) / 2.0;
    let delta = end_screen - start_screen;
    let length = delta.length();
    let angle = delta.y.atan2(delta.x);

    // Update or create line
    if let Ok((entity, mut transform)) = line_query.single_mut() {
        transform.translation = Vec3::new(midpoint.x, midpoint.y, 14.0);
        transform.rotation = Quat::from_rotation_z(angle);
        transform.scale = Vec3::new(length, 2.0, 1.0);
    } else {
        // Create new line
        commands.spawn((
            Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
            MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgba(1.0, 1.0, 0.0, 0.8)))),
            Transform {
                translation: Vec3::new(midpoint.x, midpoint.y, 14.0),
                rotation: Quat::from_rotation_z(angle),
                scale: Vec3::new(length, 2.0, 1.0),
            },
            MeasurementLine,
        ));
    }
}

/// Render measurement tooltip with distance and bearing
pub fn render_measurement_tooltip(
    state: Res<MeasurementState>,
    mut contexts: EguiContexts,
    window_query: Query<&Window>,
) {
    if !state.active {
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let Ok(window) = window_query.single() else {
        return;
    };

    // Show mode indicator
    egui::Area::new(egui::Id::new("measurement_mode"))
        .fixed_pos(egui::pos2(10.0, 130.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("MEASURE")
                        .color(egui::Color32::YELLOW)
                        .strong(),
                );
                ui.label("(M to cancel)");
            });
        });

    // Show measurement results if we have a start point
    if state.start_point.is_some() {
        if let Some(cursor_pos) = window.cursor_position() {
            // Tooltip near cursor
            let tooltip_pos = egui::pos2(cursor_pos.x + 20.0, cursor_pos.y - 50.0);

            egui::Area::new(egui::Id::new("measurement_tooltip"))
                .fixed_pos(tooltip_pos)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .fill(egui::Color32::from_rgba_unmultiplied(30, 30, 30, 230))
                        .show(ui, |ui| {
                            if let Some(dist_nm) = state.distance_nm() {
                                let dist_km = state.distance_km().unwrap_or(0.0);
                                let bearing = state.bearing().unwrap_or(0.0);

                                ui.label(
                                    egui::RichText::new(format!("{:.2} nm", dist_nm))
                                        .color(egui::Color32::WHITE)
                                        .strong(),
                                );
                                ui.label(
                                    egui::RichText::new(format!("{:.2} km", dist_km))
                                        .color(egui::Color32::LIGHT_GRAY)
                                        .size(12.0),
                                );
                                ui.label(
                                    egui::RichText::new(format!("BRG {:03.0}", bearing))
                                        .color(egui::Color32::LIGHT_BLUE),
                                );
                            } else {
                                ui.label("Click to set start point");
                            }
                        });
                });
        }
    }
}
