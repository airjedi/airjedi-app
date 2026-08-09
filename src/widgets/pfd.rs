use bevy_egui::egui;

use super::effects::{paint_arc, paint_gradient_rect, GradientDirection};
use crate::theme::WidgetTheme;

#[derive(Clone, Copy, Default)]
#[allow(dead_code)]
pub struct PfdData {
    pub airspeed_kts: Option<f64>,
    pub altitude_ft: Option<i32>,
    pub heading_deg: Option<f32>,
    pub vertical_rate_fpm: Option<i32>,
    pub roll_deg: Option<f32>,
    pub is_on_ground: bool,
}

const SKY_TOP: egui::Color32 = egui::Color32::from_rgb(0, 50, 140);
const SKY_BOTTOM: egui::Color32 = egui::Color32::from_rgb(30, 100, 200);
const GROUND_TOP: egui::Color32 = egui::Color32::from_rgb(100, 60, 20);
const GROUND_BOTTOM: egui::Color32 = egui::Color32::from_rgb(60, 35, 10);
const TAPE_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(15, 15, 20, 220);
const READOUT_BG: egui::Color32 = egui::Color32::from_rgb(0, 0, 0);
const READOUT_TEXT: egui::Color32 = egui::Color32::WHITE;
const CLIMB_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 220, 120);
const DESCENT_COLOR: egui::Color32 = egui::Color32::from_rgb(240, 160, 40);
const BANK_ARC_COLOR: egui::Color32 = egui::Color32::from_gray(180);
const PITCH_LINE_COLOR: egui::Color32 = egui::Color32::from_gray(200);

pub struct MiniPfd<'a> {
    data: &'a PfdData,
    size: egui::Vec2,
    dim_color: egui::Color32,
    accent_color: egui::Color32,
    border_color: egui::Color32,
}

impl<'a> MiniPfd<'a> {
    pub fn themed(data: &'a PfdData, theme: &WidgetTheme) -> Self {
        Self {
            data,
            size: egui::vec2(120.0, 160.0),
            dim_color: theme.text_dim,
            accent_color: theme.accent,
            border_color: theme.border,
        }
    }

    pub fn size(mut self, size: egui::Vec2) -> Self {
        self.size = size;
        self
    }
}

impl MiniPfd<'_> {
    fn scale(&self) -> f32 {
        (self.size.x.min(self.size.y) / 120.0).max(0.5)
    }

    fn paint_attitude_indicator(&self, painter: &egui::Painter, rect: egui::Rect) {
        let center = rect.center();
        let half_h = rect.height() / 2.0;
        let scale = self.scale();

        let roll_rad = self.data.roll_deg.unwrap_or(0.0).to_radians();

        // Estimate pitch from vertical rate and airspeed
        let pitch_deg = match (self.data.vertical_rate_fpm, self.data.airspeed_kts) {
            (Some(vr), Some(spd)) if spd > 30.0 => {
                let vr_fps = vr as f64 / 60.0;
                let spd_fps = spd * 1.68781;
                (vr_fps / spd_fps).atan().to_degrees() as f32
            }
            _ => 0.0,
        };

        let pitch_px = (pitch_deg / 30.0) * half_h;
        let horizon_y = center.y - pitch_px;

        // Sky region (above horizon)
        if horizon_y > rect.top() {
            let sky_rect = egui::Rect::from_min_max(
                rect.min,
                egui::pos2(rect.right(), horizon_y.min(rect.bottom())),
            );
            paint_gradient_rect(painter, sky_rect, SKY_TOP, SKY_BOTTOM, GradientDirection::Vertical);
        }

        // Ground region (below horizon)
        if horizon_y < rect.bottom() {
            let ground_rect = egui::Rect::from_min_max(
                egui::pos2(rect.left(), horizon_y.max(rect.top())),
                rect.max,
            );
            paint_gradient_rect(
                painter,
                ground_rect,
                GROUND_TOP,
                GROUND_BOTTOM,
                GradientDirection::Vertical,
            );
        }

        // Horizon line (rotated by roll)
        let line_half = rect.width() * 0.45;
        let cos_r = roll_rad.cos();
        let sin_r = roll_rad.sin();
        let horizon_center = egui::pos2(center.x, horizon_y.clamp(rect.top() + 1.0, rect.bottom() - 1.0));
        let h_left = egui::pos2(
            horizon_center.x - line_half * cos_r,
            horizon_center.y + line_half * sin_r,
        );
        let h_right = egui::pos2(
            horizon_center.x + line_half * cos_r,
            horizon_center.y - line_half * sin_r,
        );
        painter.line_segment([h_left, h_right], egui::Stroke::new(1.5 * scale, READOUT_TEXT));

        // Pitch ladder lines (only +/-10 and +/-20 at small sizes)
        let pitch_marks = if scale >= 1.5 {
            vec![-20.0, -10.0, 10.0, 20.0]
        } else {
            vec![-10.0, 10.0]
        };
        for deg in &pitch_marks {
            let offset_y = horizon_y - (deg / 30.0) * half_h;
            if offset_y > rect.top() + 4.0 && offset_y < rect.bottom() - 4.0 {
                let bar_half = rect.width() * 0.12;
                let left = egui::pos2(
                    center.x - bar_half * cos_r - (offset_y - horizon_y) * sin_r,
                    center.y + bar_half * sin_r + (offset_y - center.y),
                );
                let right = egui::pos2(
                    center.x + bar_half * cos_r - (offset_y - horizon_y) * sin_r,
                    center.y - bar_half * sin_r + (offset_y - center.y),
                );
                let stroke = if deg.abs() > 15.0 {
                    egui::Stroke::new(1.0 * scale, egui::Color32::from_white_alpha(140))
                } else {
                    egui::Stroke::new(1.0 * scale, PITCH_LINE_COLOR)
                };
                painter.line_segment([left, right], stroke);
            }
        }

        // Bank angle arc at the top
        let arc_radius = rect.width() * 0.38;
        let arc_center = egui::pos2(center.x, rect.top() + arc_radius + 4.0 * scale);
        let arc_start = std::f32::consts::PI + 30.0_f32.to_radians();
        let arc_end = 2.0 * std::f32::consts::PI - 30.0_f32.to_radians();
        paint_arc(
            painter,
            arc_center,
            arc_radius,
            arc_start,
            arc_end,
            egui::Stroke::new(1.0 * scale, BANK_ARC_COLOR),
            32,
        );

        // Bank angle tick marks
        let bank_ticks: &[(f32, f32)] = &[
            (0.0, 1.5),
            (10.0, 1.0),
            (-10.0, 1.0),
            (20.0, 1.0),
            (-20.0, 1.0),
            (30.0, 1.5),
            (-30.0, 1.5),
            (45.0, 1.0),
            (-45.0, 1.0),
            (60.0, 1.0),
            (-60.0, 1.0),
        ];
        for &(deg, width_mul) in bank_ticks {
            let angle = -std::f32::consts::FRAC_PI_2 - deg.to_radians();
            let cos_a = angle.cos();
            let sin_a = angle.sin();
            let inner = egui::pos2(
                arc_center.x + (arc_radius - 3.0 * scale) * cos_a,
                arc_center.y + (arc_radius - 3.0 * scale) * sin_a,
            );
            let outer = egui::pos2(
                arc_center.x + (arc_radius + 3.0 * scale) * cos_a,
                arc_center.y + (arc_radius + 3.0 * scale) * sin_a,
            );
            painter.line_segment(
                [inner, outer],
                egui::Stroke::new(width_mul * scale, BANK_ARC_COLOR),
            );
        }

        // Roll pointer (triangle at current bank angle on the arc)
        if self.data.roll_deg.is_some() {
            let roll_clamped = self.data.roll_deg.unwrap().clamp(-60.0, 60.0);
            let ptr_angle = -std::f32::consts::FRAC_PI_2 - roll_clamped.to_radians();
            let ptr_tip = egui::pos2(
                arc_center.x + (arc_radius - 3.0 * scale) * ptr_angle.cos(),
                arc_center.y + (arc_radius - 3.0 * scale) * ptr_angle.sin(),
            );
            let ptr_size = 4.0 * scale;
            let perp = ptr_angle + std::f32::consts::FRAC_PI_2;
            let base_center = egui::pos2(
                arc_center.x + (arc_radius + 4.0 * scale) * ptr_angle.cos(),
                arc_center.y + (arc_radius + 4.0 * scale) * ptr_angle.sin(),
            );
            let ptr_left = egui::pos2(
                base_center.x - ptr_size * 0.5 * perp.cos(),
                base_center.y - ptr_size * 0.5 * perp.sin(),
            );
            let ptr_right = egui::pos2(
                base_center.x + ptr_size * 0.5 * perp.cos(),
                base_center.y + ptr_size * 0.5 * perp.sin(),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![ptr_tip, ptr_left, ptr_right],
                READOUT_TEXT,
                egui::Stroke::NONE,
            ));
        }

        // Center reference wings (fixed aircraft symbol)
        let wing_len = rect.width() * 0.12;
        let wing_gap = 3.0 * scale;
        painter.line_segment(
            [
                egui::pos2(center.x - wing_len - wing_gap, center.y),
                egui::pos2(center.x - wing_gap, center.y),
            ],
            egui::Stroke::new(2.0 * scale, self.accent_color),
        );
        painter.line_segment(
            [
                egui::pos2(center.x + wing_gap, center.y),
                egui::pos2(center.x + wing_len + wing_gap, center.y),
            ],
            egui::Stroke::new(2.0 * scale, self.accent_color),
        );
        painter.circle_filled(center, 2.0 * scale, self.accent_color);

        // Border
        painter.rect_stroke(
            rect,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.0, self.border_color),
            egui::epaint::StrokeKind::Inside,
        );
    }

    fn paint_vertical_tape(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        value: Option<f64>,
        visible_range: f64,
        major_interval: f64,
        minor_interval: f64,
        ticks_on_right: bool,
        format_label: &dyn Fn(i64) -> String,
        format_readout: &dyn Fn(f64) -> String,
    ) {
        let scale = self.scale();

        // Background
        painter.rect_filled(rect, egui::CornerRadius::ZERO, TAPE_BG);

        let center_y = rect.center().y;
        let half_range = visible_range / 2.0;

        let Some(val) = value else {
            // No data - show dashes
            let readout_rect = self.readout_rect(rect, center_y, ticks_on_right);
            let cr = (2.0 * scale) as u8;
            painter.rect_filled(readout_rect, egui::CornerRadius::same(cr), READOUT_BG);
            painter.rect_stroke(
                readout_rect,
                egui::CornerRadius::same(cr),
                egui::Stroke::new(1.0, self.border_color),
                egui::epaint::StrokeKind::Inside,
            );
            painter.text(
                readout_rect.center(),
                egui::Align2::CENTER_CENTER,
                "---",
                egui::FontId::new(8.0 * scale, egui::FontFamily::Monospace),
                self.dim_color,
            );
            painter.rect_stroke(
                rect,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, self.border_color),
                egui::epaint::StrokeKind::Inside,
            );
            return;
        };

        let px_per_unit = rect.height() / visible_range as f32;

        // Draw ticks and labels
        let start = ((val - half_range) / minor_interval).floor() as i64 * minor_interval as i64;
        let end = ((val + half_range) / minor_interval).ceil() as i64 * minor_interval as i64;
        let mut tick_val = start;
        while tick_val <= end {
            let offset = tick_val as f64 - val;
            let y = center_y - (offset as f32 * px_per_unit);

            if y >= rect.top() - 1.0 && y <= rect.bottom() + 1.0 {
                let is_major = (tick_val % major_interval as i64) == 0;
                let tick_len = if is_major {
                    rect.width() * 0.35
                } else {
                    rect.width() * 0.2
                };

                let (tick_start_x, tick_end_x) = if ticks_on_right {
                    (rect.right() - tick_len, rect.right())
                } else {
                    (rect.left(), rect.left() + tick_len)
                };

                painter.line_segment(
                    [egui::pos2(tick_start_x, y), egui::pos2(tick_end_x, y)],
                    egui::Stroke::new(1.0 * scale, self.dim_color),
                );

                if is_major && scale >= 0.7 {
                    let label = format_label(tick_val);
                    let (anchor, label_x) = if ticks_on_right {
                        (egui::Align2::RIGHT_CENTER, rect.right() - tick_len - 1.0)
                    } else {
                        (egui::Align2::LEFT_CENTER, rect.left() + tick_len + 1.0)
                    };
                    painter.text(
                        egui::pos2(label_x, y),
                        anchor,
                        &label,
                        egui::FontId::new(6.0 * scale, egui::FontFamily::Monospace),
                        self.dim_color,
                    );
                }
            }
            tick_val += minor_interval as i64;
        }

        // Readout box with pointer
        let readout_rect = self.readout_rect(rect, center_y, ticks_on_right);
        let cr = (2.0 * scale) as u8;
        painter.rect_filled(readout_rect, egui::CornerRadius::same(cr), READOUT_BG);
        painter.rect_stroke(
            readout_rect,
            egui::CornerRadius::same(cr),
            egui::Stroke::new(1.0, self.accent_color),
            egui::epaint::StrokeKind::Inside,
        );

        let readout_text = format_readout(val);
        painter.text(
            readout_rect.center(),
            egui::Align2::CENTER_CENTER,
            &readout_text,
            egui::FontId::new(7.5 * scale, egui::FontFamily::Monospace),
            READOUT_TEXT,
        );

        // Pointer triangle
        let ptr_size = 4.0 * scale;
        if ticks_on_right {
            let tip = egui::pos2(readout_rect.right() + ptr_size, center_y);
            let top = egui::pos2(readout_rect.right(), center_y - ptr_size * 0.6);
            let bot = egui::pos2(readout_rect.right(), center_y + ptr_size * 0.6);
            painter.add(egui::Shape::convex_polygon(
                vec![tip, top, bot],
                READOUT_BG,
                egui::Stroke::new(1.0, self.accent_color),
            ));
        } else {
            let tip = egui::pos2(readout_rect.left() - ptr_size, center_y);
            let top = egui::pos2(readout_rect.left(), center_y - ptr_size * 0.6);
            let bot = egui::pos2(readout_rect.left(), center_y + ptr_size * 0.6);
            painter.add(egui::Shape::convex_polygon(
                vec![tip, top, bot],
                READOUT_BG,
                egui::Stroke::new(1.0, self.accent_color),
            ));
        }

        // Border
        painter.rect_stroke(
            rect,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.0, self.border_color),
            egui::epaint::StrokeKind::Inside,
        );
    }

    fn readout_rect(&self, tape_rect: egui::Rect, center_y: f32, ticks_on_right: bool) -> egui::Rect {
        let scale = self.scale();
        let readout_h = 12.0 * scale;
        let readout_w = tape_rect.width() - 4.0 * scale;
        let x = if ticks_on_right {
            tape_rect.left() + 1.0
        } else {
            tape_rect.right() - readout_w - 1.0
        };
        egui::Rect::from_min_size(
            egui::pos2(x, center_y - readout_h / 2.0),
            egui::vec2(readout_w, readout_h),
        )
    }

    fn paint_vsi(&self, painter: &egui::Painter, rect: egui::Rect) {
        let scale = self.scale();

        painter.rect_filled(rect, egui::CornerRadius::ZERO, TAPE_BG);

        let center_y = rect.center().y;
        let center_x = rect.center().x;
        let max_fpm = 3000.0_f32;
        let usable_half = rect.height() / 2.0 - 2.0;

        // Center zero line
        painter.line_segment(
            [
                egui::pos2(rect.left(), center_y),
                egui::pos2(rect.right(), center_y),
            ],
            egui::Stroke::new(1.0, self.dim_color),
        );

        // Scale ticks at +/-1000, +/-2000
        for &fpm in &[1000.0_f32, 2000.0, -1000.0, -2000.0] {
            let y = center_y - (fpm / max_fpm) * usable_half;
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 1.0, y),
                    egui::pos2(rect.right() - 1.0, y),
                ],
                egui::Stroke::new(0.5 * scale, self.dim_color),
            );
        }

        // VSI pointer
        if let Some(vr) = self.data.vertical_rate_fpm {
            let clamped = (vr as f32).clamp(-max_fpm, max_fpm);
            let y = center_y - (clamped / max_fpm) * usable_half;
            let color = if vr > 100 {
                CLIMB_COLOR
            } else if vr < -100 {
                DESCENT_COLOR
            } else {
                self.dim_color
            };

            // Filled bar from center to pointer
            let bar_rect = if y < center_y {
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 1.0, y),
                    egui::pos2(rect.right() - 1.0, center_y),
                )
            } else {
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 1.0, center_y),
                    egui::pos2(rect.right() - 1.0, y),
                )
            };
            let bar_color = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 80);
            painter.rect_filled(bar_rect, egui::CornerRadius::ZERO, bar_color);

            // Pointer line
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 1.0, y),
                    egui::pos2(rect.right() - 1.0, y),
                ],
                egui::Stroke::new(2.0 * scale, color),
            );

            // Numeric label at top or bottom
            if scale >= 0.8 {
                let label = format!("{:.0}", (vr as f32 / 100.0).round() * 100.0);
                let label_y = if vr >= 0 {
                    rect.top() + 6.0 * scale
                } else {
                    rect.bottom() - 6.0 * scale
                };
                painter.text(
                    egui::pos2(center_x, label_y),
                    egui::Align2::CENTER_CENTER,
                    &label,
                    egui::FontId::new(5.5 * scale, egui::FontFamily::Monospace),
                    color,
                );
            }
        }

        // Border
        painter.rect_stroke(
            rect,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.0, self.border_color),
            egui::epaint::StrokeKind::Inside,
        );
    }

    fn paint_heading_tape(&self, painter: &egui::Painter, rect: egui::Rect) {
        let scale = self.scale();

        painter.rect_filled(rect, egui::CornerRadius::ZERO, TAPE_BG);

        let center_x = rect.center().x;
        let center_y = rect.center().y;
        let visible_range = 60.0_f32;
        let px_per_deg = rect.width() / visible_range;

        let Some(hdg) = self.data.heading_deg else {
            painter.text(
                egui::pos2(center_x, center_y),
                egui::Align2::CENTER_CENTER,
                "---",
                egui::FontId::new(8.0 * scale, egui::FontFamily::Monospace),
                self.dim_color,
            );
            painter.rect_stroke(
                rect,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, self.border_color),
                egui::epaint::StrokeKind::Inside,
            );
            return;
        };

        let half_range = visible_range / 2.0;

        // Draw ticks
        let start_deg = (hdg - half_range).floor() as i32 - 1;
        let end_deg = (hdg + half_range).ceil() as i32 + 1;
        for d in start_deg..=end_deg {
            let bearing = ((d % 360) + 360) % 360;
            let mut offset = d as f32 - hdg;
            if offset > 180.0 {
                offset -= 360.0;
            }
            if offset < -180.0 {
                offset += 360.0;
            }

            let x = center_x + offset * px_per_deg;
            if x < rect.left() - 1.0 || x > rect.right() + 1.0 {
                continue;
            }

            let is_cardinal = bearing % 90 == 0;
            let is_major = bearing % 30 == 0;
            let is_minor = bearing % 10 == 0;

            if is_cardinal {
                let tick_len = rect.height() * 0.35;
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.top() + tick_len)],
                    egui::Stroke::new(1.5 * scale, self.accent_color),
                );
                let cardinal = match bearing {
                    0 => "N",
                    90 => "E",
                    180 => "S",
                    270 => "W",
                    _ => "",
                };
                painter.text(
                    egui::pos2(x, center_y + 1.0),
                    egui::Align2::CENTER_CENTER,
                    cardinal,
                    egui::FontId::new(7.0 * scale, egui::FontFamily::Monospace),
                    self.accent_color,
                );
            } else if is_major {
                let tick_len = rect.height() * 0.3;
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.top() + tick_len)],
                    egui::Stroke::new(1.0 * scale, self.dim_color),
                );
                if scale >= 0.8 {
                    let label = format!("{:02}", bearing / 10);
                    painter.text(
                        egui::pos2(x, center_y + 1.0),
                        egui::Align2::CENTER_CENTER,
                        &label,
                        egui::FontId::new(5.5 * scale, egui::FontFamily::Monospace),
                        self.dim_color,
                    );
                }
            } else if is_minor {
                let tick_len = rect.height() * 0.2;
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.top() + tick_len)],
                    egui::Stroke::new(0.5 * scale, self.dim_color),
                );
            }
        }

        // Fixed reference triangle at top center
        let tri_size = 4.0 * scale;
        let tip = egui::pos2(center_x, rect.top() + 1.0);
        let left = egui::pos2(center_x - tri_size, rect.top() - 0.5);
        let right = egui::pos2(center_x + tri_size, rect.top() - 0.5);
        painter.add(egui::Shape::convex_polygon(
            vec![tip, left, right],
            self.accent_color,
            egui::Stroke::NONE,
        ));

        // Heading readout box (above the tape, overlapping slightly)
        let readout_w = 24.0 * scale;
        let readout_h = 10.0 * scale;
        let readout_rect = egui::Rect::from_center_size(
            egui::pos2(center_x, rect.top() - readout_h * 0.3),
            egui::vec2(readout_w, readout_h),
        );
        let cr = (2.0 * scale) as u8;
        painter.rect_filled(readout_rect, egui::CornerRadius::same(cr), READOUT_BG);
        painter.rect_stroke(
            readout_rect,
            egui::CornerRadius::same(cr),
            egui::Stroke::new(1.0, self.accent_color),
            egui::epaint::StrokeKind::Inside,
        );
        painter.text(
            readout_rect.center(),
            egui::Align2::CENTER_CENTER,
            &format!("{:03.0}", hdg),
            egui::FontId::new(7.0 * scale, egui::FontFamily::Monospace),
            READOUT_TEXT,
        );

        // Border
        painter.rect_stroke(
            rect,
            egui::CornerRadius::ZERO,
            egui::Stroke::new(1.0, self.border_color),
            egui::epaint::StrokeKind::Inside,
        );
    }
}

impl egui::Widget for MiniPfd<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(self.size, egui::Sense::hover());
        if !ui.is_rect_visible(rect) {
            return response;
        }

        let w = rect.width();
        let h = rect.height();

        // Sub-region proportions
        let spd_w = w * 0.18;
        let alt_x = w * 0.72;
        let alt_w = w * 0.18;
        let vsi_x = w * 0.90;
        let vsi_w = w * 0.10;
        let main_h = h * 0.78;
        let hdg_h = h * 0.22;

        let spd_rect = egui::Rect::from_min_size(rect.min, egui::vec2(spd_w, main_h));
        let adi_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + spd_w, rect.top()),
            egui::vec2(alt_x - spd_w, main_h),
        );
        let alt_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + alt_x, rect.top()),
            egui::vec2(alt_w, main_h),
        );
        let vsi_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + vsi_x, rect.top()),
            egui::vec2(vsi_w, main_h),
        );
        let hdg_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + spd_w, rect.top() + main_h),
            egui::vec2(alt_x - spd_w, hdg_h),
        );

        // Paint each sub-instrument with its own clip rect
        let adi_painter = ui.painter_at(adi_rect);
        self.paint_attitude_indicator(&adi_painter, adi_rect);

        let spd_painter = ui.painter_at(spd_rect);
        self.paint_vertical_tape(
            &spd_painter,
            spd_rect,
            self.data.airspeed_kts,
            60.0,
            20.0,
            10.0,
            true,
            &|v| format!("{}", v),
            &|v| format!("{:.0}", v),
        );

        let alt_painter = ui.painter_at(alt_rect);
        self.paint_vertical_tape(
            &alt_painter,
            alt_rect,
            self.data.altitude_ft.map(|a| a as f64),
            600.0,
            200.0,
            100.0,
            false,
            &|v| {
                if v >= 1000 {
                    format!("{}", v / 100)
                } else {
                    format!("{}", v)
                }
            },
            &|v| {
                let ft = v as i32;
                if ft >= 18000 {
                    format!("F{}", ft / 100)
                } else {
                    format!("{}", ft)
                }
            },
        );

        let vsi_painter = ui.painter_at(vsi_rect);
        self.paint_vsi(&vsi_painter, vsi_rect);

        let hdg_painter = ui.painter_at(hdg_rect);
        self.paint_heading_tape(&hdg_painter, hdg_rect);

        response
    }
}
