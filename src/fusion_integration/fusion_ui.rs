use airjedi_fusion::{TrackQuality, TrackStatus, TargetClassification};
use bevy_egui::egui;

pub fn fusion_status_text(quality: &TrackQuality) -> &'static str {
    match quality.status {
        TrackStatus::Tentative => "TENTATIVE",
        TrackStatus::Confirmed => "CONFIRMED",
        TrackStatus::Coasting => "COASTING",
        TrackStatus::Lost => "LOST",
    }
}

pub fn fusion_status_color(quality: &TrackQuality) -> egui::Color32 {
    match quality.status {
        TrackStatus::Tentative => egui::Color32::from_rgb(180, 180, 100),
        TrackStatus::Confirmed => egui::Color32::from_rgb(100, 200, 100),
        TrackStatus::Coasting => egui::Color32::from_rgb(200, 150, 50),
        TrackStatus::Lost => egui::Color32::from_rgb(200, 80, 80),
    }
}

pub fn render_fusion_info(
    ui: &mut egui::Ui,
    quality: &TrackQuality,
    classification: &TargetClassification,
) {
    ui.horizontal(|ui| {
        ui.label("Track Status:");
        let color = fusion_status_color(quality);
        ui.colored_label(color, fusion_status_text(quality));
    });

    ui.horizontal(|ui| {
        ui.label("Sensors:");
        ui.label(format!("{}", quality.sensor_count));
    });

    ui.horizontal(|ui| {
        ui.label("Confidence:");
        ui.label(format!("{:.0}%", quality.confidence * 100.0));
    });

    ui.horizontal(|ui| {
        ui.label("Category:");
        ui.label(format!("{:?}", classification.category));
    });

    if quality.staleness.as_secs() > 0 {
        ui.horizontal(|ui| {
            ui.label("Stale:");
            ui.label(format!("{}s", quality.staleness.as_secs()));
        });
    }
}
