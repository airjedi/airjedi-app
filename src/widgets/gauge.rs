use bevy_egui::egui;

use super::effects::paint_arc;
use crate::theme::WidgetTheme;

pub struct ArcGauge<'a> {
    value: f32,
    label: Option<&'a str>,
    value_text: Option<&'a str>,
    size: f32,
    sweep_degrees: f32,
    track_color: egui::Color32,
    fill_color: egui::Color32,
    text_color: egui::Color32,
    value_color: egui::Color32,
    tick_count: usize,
    track_width: f32,
    fill_width: f32,
}

impl<'a> ArcGauge<'a> {
    pub fn new(value: f32) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            label: None,
            value_text: None,
            size: 100.0,
            sweep_degrees: 270.0,
            track_color: egui::Color32::from_gray(60),
            fill_color: egui::Color32::from_rgb(100, 200, 255),
            text_color: egui::Color32::from_gray(180),
            value_color: egui::Color32::WHITE,
            tick_count: 0,
            track_width: 3.0,
            fill_width: 3.0,
        }
    }

    pub fn themed(value: f32, theme: &WidgetTheme) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            label: None,
            value_text: None,
            size: 100.0,
            sweep_degrees: 270.0,
            track_color: theme.border,
            fill_color: theme.accent,
            text_color: theme.text_dim,
            value_color: theme.text,
            tick_count: 0,
            track_width: 3.0,
            fill_width: 3.0,
        }
    }

    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    pub fn value_text(mut self, text: &'a str) -> Self {
        self.value_text = Some(text);
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn sweep(mut self, degrees: f32) -> Self {
        self.sweep_degrees = degrees;
        self
    }

    pub fn track_color(mut self, color: egui::Color32) -> Self {
        self.track_color = color;
        self
    }

    pub fn fill_color(mut self, color: egui::Color32) -> Self {
        self.fill_color = color;
        self
    }

    pub fn text_color(mut self, color: egui::Color32) -> Self {
        self.text_color = color;
        self
    }

    pub fn value_color(mut self, color: egui::Color32) -> Self {
        self.value_color = color;
        self
    }

    pub fn tick_count(mut self, count: usize) -> Self {
        self.tick_count = count;
        self
    }

    pub fn track_width(mut self, width: f32) -> Self {
        self.track_width = width;
        self
    }

    pub fn fill_width(mut self, width: f32) -> Self {
        self.fill_width = width;
        self
    }
}

impl egui::Widget for ArcGauge<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let desired_size = egui::vec2(self.size, self.size);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);

        let center = rect.center();
        let radius = self.size * 0.4;

        let sweep_rad = self.sweep_degrees.to_radians();
        let half_gap = (std::f32::consts::TAU - sweep_rad) / 2.0;
        let start_angle = std::f32::consts::FRAC_PI_2 + half_gap;
        let end_angle = std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU - half_gap;

        // Background track
        paint_arc(
            &painter,
            center,
            radius,
            start_angle,
            end_angle,
            egui::Stroke::new(self.track_width, self.track_color),
            64,
        );

        // Value arc
        let value_end = start_angle + self.value * (end_angle - start_angle);
        paint_arc(
            &painter,
            center,
            radius,
            start_angle,
            value_end,
            egui::Stroke::new(self.fill_width, self.fill_color),
            64,
        );

        // Tick marks
        if self.tick_count > 1 {
            let tick_inner = radius - self.size * 0.05;
            let tick_outer = radius + self.size * 0.05;
            let tick_stroke = egui::Stroke::new(1.0, self.track_color);
            for i in 0..=self.tick_count {
                let t = i as f32 / self.tick_count as f32;
                let angle = start_angle + t * (end_angle - start_angle);
                let cos_a = angle.cos();
                let sin_a = angle.sin();
                let inner_pt =
                    egui::pos2(center.x + tick_inner * cos_a, center.y + tick_inner * sin_a);
                let outer_pt =
                    egui::pos2(center.x + tick_outer * cos_a, center.y + tick_outer * sin_a);
                painter.line_segment([inner_pt, outer_pt], tick_stroke);
            }
        }

        // Label text
        if let Some(label) = self.label {
            let font_size = self.size * 0.12;
            let label_pos = egui::pos2(center.x, center.y + font_size * 0.8);
            painter.text(
                label_pos,
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(font_size),
                self.text_color,
            );
        }

        // Value text
        if let Some(value_text) = self.value_text {
            let font_size = self.size * 0.16;
            let value_pos = egui::pos2(center.x, center.y - font_size * 0.4);
            painter.text(
                value_pos,
                egui::Align2::CENTER_CENTER,
                value_text,
                egui::FontId::proportional(font_size),
                self.value_color,
            );
        }

        response
    }
}
