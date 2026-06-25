/// 3D Debug Overlay for development.
///
/// Shows axis gizmo, side-view schematic, and numerical readouts in the
/// bottom-left corner of the map viewport. Toggled with F10. Only renders
/// when 3D mode is active.
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::camera::AircraftCamera;
use crate::dock::DockTreeState;
use crate::theme::{to_egui_color32, to_egui_color32_alpha, AppTheme};
use crate::view3d::{TransitionState, View3DState};

#[derive(Resource, Default)]
pub struct Debug3DHudState {
    pub visible: bool,
}

// Catppuccin Mocha axis colors
const AXIS_RED: egui::Color32 = egui::Color32::from_rgb(0xf3, 0x8b, 0xa8);
const AXIS_GREEN: egui::Color32 = egui::Color32::from_rgb(0xa6, 0xe3, 0xa1);
const AXIS_BLUE: egui::Color32 = egui::Color32::from_rgb(0x89, 0xb4, 0xfa);
const ORBIT_ORANGE: egui::Color32 = egui::Color32::from_rgb(0xfa, 0xb3, 0x87);
const CHASE_YELLOW: egui::Color32 = egui::Color32::from_rgb(0xf9, 0xe2, 0xaf);
fn gizmo_bg() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(0x31, 0x32, 0x44, 180)
}

const GIZMO_SIZE: f32 = 64.0;
const SIDE_VIEW_WIDTH: f32 = 64.0;
const SIDE_VIEW_HEIGHT: f32 = 48.0;
const GIZMO_RADIUS: f32 = 24.0;
const HUD_MARGIN: f32 = 12.0;
const HUD_PADDING: f32 = 8.0;
const LABEL_SIZE: f32 = 9.0;
const VALUE_SIZE: f32 = 10.0;

pub fn render_debug_3d_hud(
    mut contexts: EguiContexts,
    view3d: Res<View3DState>,
    dock_state: Res<DockTreeState>,
    theme: Res<AppTheme>,
    hud_state: Res<Debug3DHudState>,
    camera_query: Query<&Transform, With<AircraftCamera>>,
) {
    if !view3d.is_3d_active() || !hud_state.visible {
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let Some(map_rect) = dock_state.map_viewport_rect else {
        return;
    };

    let Ok(camera_transform) = camera_query.single() else {
        return;
    };

    let pos = egui::pos2(
        map_rect.left() + HUD_MARGIN,
        map_rect.bottom() - HUD_MARGIN - 200.0,
    );

    let bg_color = to_egui_color32_alpha(theme.bg_secondary(), 200);
    let text_color = to_egui_color32(theme.text_primary());
    let dim_color = to_egui_color32(theme.text_dim());

    egui::Area::new(egui::Id::new("debug_3d_hud"))
        .fixed_pos(pos)
        .order(egui::Order::Middle)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(bg_color)
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(HUD_PADDING)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;

                    ui.horizontal(|ui| {
                        // Left column: gizmo + side view
                        ui.vertical(|ui| {
                            paint_axis_gizmo(ui, camera_transform);
                            ui.add_space(4.0);
                            paint_side_view(ui, &view3d, camera_transform);
                        });

                        ui.add_space(6.0);

                        // Right column: text readouts
                        ui.vertical(|ui| {
                            paint_readouts(ui, &view3d, camera_transform, text_color, dim_color);
                        });
                    });
                });
        });
}

fn paint_axis_gizmo(ui: &mut egui::Ui, camera_transform: &Transform) {
    let (response, painter) =
        ui.allocate_painter(egui::vec2(GIZMO_SIZE, GIZMO_SIZE), egui::Sense::hover());
    let center = response.rect.center();

    // Background circle
    painter.circle_filled(center, GIZMO_RADIUS + 4.0, gizmo_bg());

    let inv = camera_transform.rotation.inverse();

    // Project each world axis into view space
    let axes = [
        ("X", inv * Vec3::X, AXIS_RED),
        ("Y", inv * Vec3::Y, AXIS_GREEN),
        ("Z", inv * Vec3::Z, AXIS_BLUE),
    ];

    // Sort by z (depth): most negative = farthest = draw first
    let mut sorted: Vec<(&str, Vec3, egui::Color32)> = axes.to_vec();
    sorted.sort_by(|a, b| {
        a.1.z
            .partial_cmp(&b.1.z)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for (label, view_dir, color) in &sorted {
        let screen_x = view_dir.x * GIZMO_RADIUS;
        let screen_y = -view_dir.y * GIZMO_RADIUS; // negate Y: screen Y points down

        let endpoint = center + egui::vec2(screen_x, screen_y);

        // Line from center to endpoint
        painter.line_segment([center, endpoint], egui::Stroke::new(2.0, *color));

        // Label at the tip
        let label_pos = center + egui::vec2(screen_x * 1.2, screen_y * 1.2);
        painter.text(
            label_pos,
            egui::Align2::CENTER_CENTER,
            *label,
            egui::FontId::new(9.0, egui::FontFamily::Monospace),
            *color,
        );
    }

    // Origin dot
    let origin_color = egui::Color32::from_rgb(0xcd, 0xd6, 0xf4);
    painter.circle_filled(center, 2.0, origin_color);
}

fn paint_side_view(ui: &mut egui::Ui, view3d: &View3DState, camera_transform: &Transform) {
    let (response, painter) = ui.allocate_painter(
        egui::vec2(SIDE_VIEW_WIDTH, SIDE_VIEW_HEIGHT),
        egui::Sense::hover(),
    );
    let rect = response.rect;

    // Background
    painter.rect_filled(rect, egui::CornerRadius::same(3), gizmo_bg());

    let cam_height = camera_transform.translation.y;
    let ground_y = view3d.altitude_to_z(view3d.ground_elevation_ft);
    let orbit_y = view3d.altitude_to_z(
        view3d
            .follow_altitude_ft
            .unwrap_or(view3d.ground_elevation_ft),
    );

    // Vertical range for normalization
    let max_val = cam_height.max(ground_y).max(orbit_y);
    let min_val = ground_y;
    let range = (max_val - min_val).max(1.0) * 1.2;

    let margin_top = 6.0;
    let margin_bottom = 6.0;
    let draw_height = rect.height() - margin_top - margin_bottom;

    let normalize = |val: f32| -> f32 {
        let t = (val - min_val) / range;
        rect.bottom() - margin_bottom - t * draw_height
    };

    let ground_screen_y = normalize(ground_y);
    let cam_screen_y = normalize(cam_height);
    let orbit_screen_y = normalize(orbit_y);

    // Ground plane line
    let ground_color = egui::Color32::from_rgba_unmultiplied(80, 60, 40, 150);
    painter.line_segment(
        [
            egui::pos2(rect.left() + 2.0, ground_screen_y),
            egui::pos2(rect.right() - 2.0, ground_screen_y),
        ],
        egui::Stroke::new(1.5, ground_color),
    );
    // Fill below ground
    if ground_screen_y < rect.bottom() - 1.0 {
        let fill_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 2.0, ground_screen_y),
            egui::pos2(rect.right() - 2.0, rect.bottom() - 2.0),
        );
        painter.rect_filled(
            fill_rect,
            egui::CornerRadius::ZERO,
            egui::Color32::from_rgba_unmultiplied(60, 45, 30, 80),
        );
    }

    // Camera position (small triangle)
    let cam_x = rect.center().x - 8.0;
    let cam_screen_y = cam_screen_y.clamp(rect.top() + margin_top, rect.bottom() - margin_bottom);
    let tri_size = 4.0;
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(cam_x, cam_screen_y - tri_size),
            egui::pos2(cam_x - tri_size, cam_screen_y + tri_size),
            egui::pos2(cam_x + tri_size, cam_screen_y + tri_size),
        ],
        AXIS_GREEN,
        egui::Stroke::NONE,
    ));

    // Orbit center (orange dot)
    let orbit_x = rect.center().x + 8.0;
    let orbit_screen_y =
        orbit_screen_y.clamp(rect.top() + margin_top, rect.bottom() - margin_bottom);
    painter.circle_filled(egui::pos2(orbit_x, orbit_screen_y), 3.0, ORBIT_ORANGE);

    // View direction line from camera toward orbit center
    let dash_color = egui::Color32::from_rgba_unmultiplied(0xcd, 0xd6, 0xf4, 100);
    painter.line_segment(
        [
            egui::pos2(cam_x, cam_screen_y),
            egui::pos2(orbit_x, orbit_screen_y),
        ],
        egui::Stroke::new(1.0, dash_color),
    );

    // Border
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(3),
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(0x45, 0x47, 0x5a, 120),
        ),
        egui::epaint::StrokeKind::Inside,
    );
}

fn paint_readouts(
    ui: &mut egui::Ui,
    view3d: &View3DState,
    camera_transform: &Transform,
    text_color: egui::Color32,
    dim_color: egui::Color32,
) {
    let cam = camera_transform.translation;

    // CAM section
    ui.label(
        egui::RichText::new("CAM")
            .size(LABEL_SIZE)
            .color(dim_color)
            .monospace(),
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        colored_axis_label(ui, "x", AXIS_RED, cam.x);
        ui.add_space(2.0);
        colored_axis_label(ui, "y", AXIS_GREEN, cam.y);
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        colored_axis_label(ui, "z", AXIS_BLUE, cam.z);
    });

    ui.add_space(2.0);

    // ORBIT section
    let orbit_alt_ft = view3d
        .follow_altitude_ft
        .unwrap_or(view3d.ground_elevation_ft);
    let orbit_y = view3d.altitude_to_z(orbit_alt_ft);
    // Orbit center x/z are approximately 0 relative to camera (derived from map center)
    ui.label(
        egui::RichText::new("ORBIT")
            .size(LABEL_SIZE)
            .color(dim_color)
            .monospace(),
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        colored_axis_label(ui, "y", AXIS_GREEN, orbit_y);
    });

    ui.add_space(2.0);

    // STATE section
    ui.label(
        egui::RichText::new("STATE")
            .size(LABEL_SIZE)
            .color(dim_color)
            .monospace(),
    );

    // Mode
    let (mode_text, mode_color) = if view3d.chase_active {
        ("chase", CHASE_YELLOW)
    } else if matches!(view3d.transition, TransitionState::TransitioningTo3D { .. }) {
        ("trans->3D", ORBIT_ORANGE)
    } else if matches!(view3d.transition, TransitionState::TransitioningTo2D { .. }) {
        ("trans->2D", ORBIT_ORANGE)
    } else {
        ("orbit", AXIS_GREEN)
    };
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("MODE ")
                .size(VALUE_SIZE)
                .color(dim_color)
                .monospace(),
        );
        ui.label(
            egui::RichText::new(mode_text)
                .size(VALUE_SIZE)
                .color(mode_color)
                .monospace(),
        );
    });

    // Pitch/Yaw
    ui.label(
        egui::RichText::new(format!(
            "PIT {:.0}  YAW {:.0}",
            view3d.camera_pitch, view3d.camera_yaw
        ))
        .size(VALUE_SIZE)
        .color(text_color)
        .monospace(),
    );

    // Altitude
    ui.label(
        egui::RichText::new(format!(
            "ALT {} ft",
            format_number(view3d.camera_altitude as i32)
        ))
        .size(VALUE_SIZE)
        .color(text_color)
        .monospace(),
    );

    // Distance from camera to orbit center
    let dist = cam.y - orbit_y; // simplified vertical distance
    ui.label(
        egui::RichText::new(format!("DIST {}", format_number(dist.abs() as i32)))
            .size(VALUE_SIZE)
            .color(text_color)
            .monospace(),
    );

    // Ground elevation (raw, not exaggerated)
    ui.label(
        egui::RichText::new(format!(
            "GND {} ft",
            format_number(view3d.ground_elevation_ft)
        ))
        .size(VALUE_SIZE)
        .color(text_color)
        .monospace(),
    );
}

fn colored_axis_label(ui: &mut egui::Ui, axis: &str, color: egui::Color32, value: f32) {
    ui.label(
        egui::RichText::new(axis)
            .size(VALUE_SIZE)
            .color(color)
            .monospace(),
    );
    ui.label(
        egui::RichText::new(format!("{:>7.0} ", value))
            .size(VALUE_SIZE)
            .color(egui::Color32::from_rgb(0xcd, 0xd6, 0xf4))
            .monospace(),
    );
}

fn format_number(n: i32) -> String {
    if n.abs() >= 1000 {
        let sign = if n < 0 { "-" } else { "" };
        let abs = n.abs();
        format!("{}{},{:03}", sign, abs / 1000, abs % 1000)
    } else {
        format!("{}", n)
    }
}
