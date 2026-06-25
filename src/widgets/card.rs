use bevy_egui::egui;

use super::effects::{paint_gradient_rect, GradientDirection};
use super::shadow_frame::{ShadowFrame, ShadowPreset};
use super::WidgetTheme;

/// A card container with optional gradient header, shadow, and themed styling.
pub struct Card<'a> {
    theme: &'a WidgetTheme,
    header_text: Option<String>,
    header_gradient: Option<(egui::Color32, egui::Color32)>,
    shadow_preset: Option<ShadowPreset>,
    corner_radius: u8,
    glow_color: Option<egui::Color32>,
}

impl<'a> Card<'a> {
    pub fn new(theme: &'a WidgetTheme) -> Self {
        Self {
            theme,
            header_text: None,
            header_gradient: None,
            shadow_preset: Some(ShadowPreset::Subtle),
            corner_radius: 8,
            glow_color: None,
        }
    }

    /// Set the card header text.
    pub fn header(mut self, text: impl Into<String>) -> Self {
        self.header_text = Some(text.into());
        self
    }

    /// Set a gradient for the header bar.
    pub fn gradient_header(mut self, left: egui::Color32, right: egui::Color32) -> Self {
        self.header_gradient = Some((left, right));
        self
    }

    pub fn shadow(mut self, preset: ShadowPreset) -> Self {
        self.shadow_preset = Some(preset);
        self
    }

    pub fn no_shadow(mut self) -> Self {
        self.shadow_preset = None;
        self
    }

    pub fn corner_radius(mut self, radius: u8) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn glow(mut self, color: egui::Color32) -> Self {
        self.glow_color = Some(color);
        self
    }

    /// Show the card with content.
    pub fn show(
        self,
        ui: &mut egui::Ui,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) -> egui::InnerResponse<()> {
        let mut frame = ShadowFrame::new(self.theme)
            .corner_radius(self.corner_radius)
            .stroke(egui::Stroke::new(1.0, self.theme.border))
            .inner_margin(0.0);

        if let Some(preset) = self.shadow_preset {
            frame = frame.shadow(preset);
        }

        if let Some(glow_color) = self.glow_color {
            frame = frame.glow(glow_color, 16);
        }

        frame.show(ui, |ui| {
            // Header
            if let Some(ref header_text) = self.header_text {
                let header_rect = ui.allocate_space(egui::vec2(ui.available_width(), 28.0)).1;

                if let Some((left, right)) = self.header_gradient {
                    paint_gradient_rect(
                        ui.painter(),
                        header_rect,
                        left,
                        right,
                        GradientDirection::Horizontal,
                    );
                } else {
                    ui.painter().rect_filled(
                        header_rect,
                        egui::CornerRadius::ZERO,
                        self.theme.bg_secondary,
                    );
                }

                ui.painter().text(
                    header_rect.left_center() + egui::vec2(10.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    header_text,
                    egui::FontId::proportional(12.0),
                    self.theme.text,
                );

                ui.painter().hline(
                    header_rect.x_range(),
                    header_rect.bottom(),
                    egui::Stroke::new(1.0, self.theme.border),
                );
            }

            // Body content with margin
            egui::Frame::new().inner_margin(10.0).show(ui, |ui| {
                add_contents(ui);
            });
        })
    }
}
