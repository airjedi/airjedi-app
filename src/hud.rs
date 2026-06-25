/// Camera HUD overlay for 3D mode.
///
/// Renders a compact heads-up display in the top-right corner of the map viewport
/// showing compass heading, pitch/tilt, and altitude. Only visible in 3D mode
/// when enabled (toggle with H key).
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::dock::DockTreeState;
use crate::theme::{to_egui_color32, to_egui_color32_alpha, AppTheme};
use crate::view3d::View3DState;

#[derive(Resource)]
pub struct HudState {
    pub visible: bool,
}

impl Default for HudState {
    fn default() -> Self {
        Self { visible: true }
    }
}

const HUD_MARGIN: f32 = 12.0;
const HUD_PADDING: f32 = 10.0;
const COMPASS_RADIUS: f32 = 28.0;
const HORIZON_WIDTH: f32 = 70.0;
const HORIZON_HEIGHT: f32 = 24.0;
const LABEL_SIZE: f32 = 10.0;
const VALUE_SIZE: f32 = 12.0;

pub fn render_camera_hud(
    mut contexts: EguiContexts,
    view3d: Res<View3DState>,
    dock_state: Res<DockTreeState>,
    theme: Res<AppTheme>,
    hud_state: Res<HudState>,
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

    let hud_width = 90.0;
    let pos = egui::pos2(
        map_rect.right() - hud_width - HUD_MARGIN - HUD_PADDING * 2.0,
        map_rect.top() + HUD_MARGIN,
    );

    let bg_color = to_egui_color32_alpha(theme.bg_secondary(), 200);
    let text_color = to_egui_color32(theme.text_primary());
    let dim_color = to_egui_color32(theme.text_dim());
    let accent_color = to_egui_color32(theme.accent_primary());

    egui::Area::new(egui::Id::new("camera_hud"))
        .fixed_pos(pos)
        .order(egui::Order::Middle)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(bg_color)
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(HUD_PADDING)
                .show(ui, |ui| {
                    ui.set_width(hud_width);
                    ui.spacing_mut().item_spacing.y = 6.0;

                    // -- Compass --
                    paint_compass(ui, view3d.camera_yaw, text_color, dim_color, accent_color);

                    let heading = ((view3d.camera_yaw % 360.0) + 360.0) % 360.0;
                    let cardinal = cardinal_direction(heading);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{} {:03.0}\u{00B0}", cardinal, heading))
                                .size(VALUE_SIZE)
                                .color(text_color)
                                .monospace(),
                        );
                    });

                    ui.add_space(2.0);

                    // -- Horizon / Tilt --
                    paint_horizon(ui, view3d.camera_pitch, dim_color, accent_color);

                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(format!("TILT {:.0}\u{00B0}", view3d.camera_pitch))
                                .size(LABEL_SIZE)
                                .color(dim_color)
                                .monospace(),
                        );
                    });

                    ui.add_space(2.0);

                    // -- Altitude --
                    ui.vertical_centered(|ui| {
                        let alt = view3d.camera_altitude;
                        let alt_text = if alt >= 18000.0 {
                            format!("FL{}", (alt / 100.0).round() as i32)
                        } else {
                            format!("{} ft", format_altitude(alt as i32))
                        };
                        ui.label(
                            egui::RichText::new(alt_text)
                                .size(VALUE_SIZE + 2.0)
                                .color(text_color)
                                .monospace()
                                .strong(),
                        );

                        let gnd = view3d.ground_elevation_ft;
                        ui.label(
                            egui::RichText::new(format!("GND {} ft", format_altitude(gnd)))
                                .size(LABEL_SIZE)
                                .color(dim_color)
                                .monospace(),
                        );
                    });
                });
        });
}

fn paint_compass(
    ui: &mut egui::Ui,
    yaw: f32,
    text_color: egui::Color32,
    dim_color: egui::Color32,
    accent_color: egui::Color32,
) {
    let size = COMPASS_RADIUS * 2.0 + 8.0;
    let (response, painter) = ui.allocate_painter(egui::vec2(size, size), egui::Sense::hover());
    let center = response.rect.center();
    let r = COMPASS_RADIUS;

    // Background circle
    painter.circle_filled(
        center,
        r,
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 80),
    );
    painter.circle_stroke(center, r, egui::Stroke::new(1.0, dim_color));

    // Fixed heading reference tick at top
    let tick_top = center + egui::vec2(0.0, -r - 3.0);
    let tick_left = center + egui::vec2(-4.0, -r + 2.0);
    let tick_right = center + egui::vec2(4.0, -r + 2.0);
    painter.add(egui::Shape::convex_polygon(
        vec![tick_top, tick_left, tick_right],
        accent_color,
        egui::Stroke::NONE,
    ));

    // Cardinal letters rotate with the compass dial (-yaw)
    // At yaw=0 (looking north), N is at the top.
    // Screen coords: +X right, +Y down. North = up = -Y.
    let cardinals = [("N", 0.0_f32), ("E", 90.0), ("S", 180.0), ("W", 270.0)];

    for (label, bearing) in cardinals {
        let effective = bearing.to_radians() - yaw.to_radians();
        let pos = center + egui::vec2(r * 0.7 * effective.sin(), -r * 0.7 * effective.cos());

        let color = if label == "N" {
            accent_color
        } else {
            text_color
        };
        let font_size = if label == "N" { 11.0 } else { 9.0 };
        painter.text(
            pos,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::new(font_size, egui::FontFamily::Monospace),
            color,
        );

        // Tick mark at the edge
        let tick_inner =
            center + egui::vec2((r - 4.0) * effective.sin(), -(r - 4.0) * effective.cos());
        let tick_outer = center + egui::vec2(r * effective.sin(), -r * effective.cos());
        painter.line_segment([tick_inner, tick_outer], egui::Stroke::new(1.5, color));
    }

    // Minor ticks every 30 degrees (skip cardinals)
    for i in 0..12 {
        let bearing = i as f32 * 30.0;
        if (bearing % 90.0).abs() < 0.1 {
            continue;
        }
        let effective = bearing.to_radians() - yaw.to_radians();
        let tick_inner =
            center + egui::vec2((r - 3.0) * effective.sin(), -(r - 3.0) * effective.cos());
        let tick_outer = center + egui::vec2(r * effective.sin(), -r * effective.cos());
        painter.line_segment([tick_inner, tick_outer], egui::Stroke::new(1.0, dim_color));
    }
}

fn paint_horizon(
    ui: &mut egui::Ui,
    pitch: f32,
    dim_color: egui::Color32,
    accent_color: egui::Color32,
) {
    let (response, painter) = ui.allocate_painter(
        egui::vec2(HORIZON_WIDTH, HORIZON_HEIGHT),
        egui::Sense::hover(),
    );
    let rect = response.rect;
    let center = rect.center();

    // Background
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(3),
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 80),
    );

    // Pitch offset: positive pitch = looking down = horizon shifts up
    let max_offset = HORIZON_HEIGHT / 2.0 - 2.0;
    let offset = -(pitch / 90.0) * max_offset;
    let horizon_y = center.y + offset;

    // Sky region (above horizon)
    let sky_color = egui::Color32::from_rgba_unmultiplied(60, 80, 120, 100);
    if horizon_y > rect.top() {
        let sky_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(rect.right(), horizon_y.min(rect.bottom())),
        );
        painter.rect_filled(sky_rect, egui::CornerRadius::ZERO, sky_color);
    }

    // Ground region (below horizon)
    let ground_color = egui::Color32::from_rgba_unmultiplied(80, 60, 40, 100);
    if horizon_y < rect.bottom() {
        let ground_rect =
            egui::Rect::from_min_max(egui::pos2(rect.left(), horizon_y.max(rect.top())), rect.max);
        painter.rect_filled(ground_rect, egui::CornerRadius::ZERO, ground_color);
    }

    // Horizon line
    let line_y = horizon_y.clamp(rect.top() + 1.0, rect.bottom() - 1.0);
    painter.line_segment(
        [
            egui::pos2(rect.left() + 2.0, line_y),
            egui::pos2(rect.right() - 2.0, line_y),
        ],
        egui::Stroke::new(1.5, accent_color),
    );

    // Center reference wings
    let wing_len = 8.0;
    painter.line_segment(
        [
            egui::pos2(center.x - wing_len, center.y),
            egui::pos2(center.x - 3.0, center.y),
        ],
        egui::Stroke::new(1.5, dim_color),
    );
    painter.line_segment(
        [
            egui::pos2(center.x + 3.0, center.y),
            egui::pos2(center.x + wing_len, center.y),
        ],
        egui::Stroke::new(1.5, dim_color),
    );
    painter.circle_filled(center, 1.5, dim_color);

    // Border
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(3),
        egui::Stroke::new(1.0, dim_color),
        egui::epaint::StrokeKind::Inside,
    );
}

fn cardinal_direction(degrees: f32) -> &'static str {
    let d = ((degrees % 360.0) + 360.0) % 360.0;
    let idx = ((d + 22.5) / 45.0) as usize % 8;
    ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][idx]
}

fn format_altitude(feet: i32) -> String {
    if feet.abs() >= 1000 {
        let sign = if feet < 0 { "-" } else { "" };
        let abs = feet.abs();
        format!("{}{},{:03}", sign, abs / 1000, abs % 1000)
    } else {
        format!("{}", feet)
    }
}
