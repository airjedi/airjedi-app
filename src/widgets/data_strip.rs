use super::WidgetTheme;
use bevy_egui::egui;

/// A horizontal data ribbon with an optional colored accent border on the left.
pub struct DataStrip<'a> {
    theme: &'a WidgetTheme,
    accent_color: Option<egui::Color32>,
    accent_width: f32,
    glow_blur: Option<u8>,
    fill: Option<egui::Color32>,
    inner_margin: f32,
}

impl<'a> DataStrip<'a> {
    pub fn new(theme: &'a WidgetTheme) -> Self {
        Self {
            theme,
            accent_color: None,
            accent_width: 3.0,
            glow_blur: None,
            fill: None,
            inner_margin: 6.0,
        }
    }

    /// Add a colored accent bar on the left edge.
    pub fn accent_left(mut self, color: egui::Color32, width: f32) -> Self {
        self.accent_color = Some(color);
        self.accent_width = width;
        self
    }

    /// Add a glow effect around the accent border.
    pub fn glow(mut self, blur: u8) -> Self {
        self.glow_blur = Some(blur);
        self
    }

    pub fn fill(mut self, fill: egui::Color32) -> Self {
        self.fill = Some(fill);
        self
    }

    pub fn inner_margin(mut self, margin: f32) -> Self {
        self.inner_margin = margin;
        self
    }

    /// Show the data strip with content.
    pub fn show(
        self,
        ui: &mut egui::Ui,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) -> egui::InnerResponse<()> {
        let fill = self.fill.unwrap_or(self.theme.bg_secondary);

        let left_margin = self.inner_margin
            + if self.accent_color.is_some() {
                self.accent_width + 2.0
            } else {
                0.0
            };
        let frame = egui::Frame::new()
            .inner_margin(egui::Margin {
                left: left_margin as i8,
                right: self.inner_margin as i8,
                top: (self.inner_margin / 2.0) as i8,
                bottom: (self.inner_margin / 2.0) as i8,
            })
            .fill(fill)
            .corner_radius(4);

        let mut prepared = frame.begin(ui);
        add_contents(&mut prepared.content_ui);
        let content_rect = prepared.content_ui.min_rect();
        let fill_rect = prepared.frame.fill_rect(content_rect);

        // Paint accent bar on left edge
        if let Some(accent) = self.accent_color {
            let accent_rect = egui::Rect::from_min_size(
                fill_rect.left_top(),
                egui::vec2(self.accent_width, fill_rect.height()),
            );

            // Optional glow behind accent bar
            if let Some(blur) = self.glow_blur {
                let glow_shape = egui::epaint::RectShape::filled(
                    accent_rect.expand(2.0),
                    egui::CornerRadius::ZERO,
                    egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 60),
                )
                .with_blur_width(blur as f32);
                prepared
                    .content_ui
                    .painter()
                    .add(egui::Shape::Rect(glow_shape));
            }

            prepared.content_ui.painter().rect_filled(
                accent_rect,
                egui::CornerRadius::ZERO,
                accent,
            );
        }

        let response = prepared.end(ui);
        egui::InnerResponse::new((), response)
    }
}
