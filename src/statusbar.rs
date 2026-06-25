use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::adsb::AdsbAircraftData;
use crate::aircraft::stats_panel::StatsPanelState;
use crate::recording::RecordingState;
use crate::theme::{to_egui_color32, AppTheme};
use crate::MapState;

/// FPS smoothing state using exponential moving average.
#[derive(Resource)]
pub struct StatusBarState {
    /// Smoothed FPS value
    pub fps: f32,
}

impl Default for StatusBarState {
    fn default() -> Self {
        Self { fps: 0.0 }
    }
}

/// Height of the statusbar in pixels.
const STATUSBAR_HEIGHT: f32 = 22.0;
/// Font size for all statusbar text.
const FONT_SIZE: f32 = 11.0;
/// EMA smoothing factor for FPS (lower = smoother, 0.05 = ~1s window at 60fps).
const FPS_SMOOTHING: f32 = 0.05;

/// Render the bottom statusbar as an egui BottomPanel.
///
/// Must run before the CentralPanel (dock tree) in EguiPrimaryContextPass.
pub fn render_statusbar(
    mut contexts: EguiContexts,
    theme: Res<AppTheme>,
    adsb_data: Option<Res<AdsbAircraftData>>,
    stats: Res<StatsPanelState>,
    recording: Res<RecordingState>,
    map_state: Res<MapState>,
    time: Res<Time>,
    mut state: ResMut<StatusBarState>,
) {
    // Update FPS with exponential moving average
    let dt = time.delta_secs();
    if dt > 0.0 {
        let instant_fps = 1.0 / dt;
        if state.fps == 0.0 {
            state.fps = instant_fps;
        } else {
            state.fps += FPS_SMOOTHING * (instant_fps - state.fps);
        }
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let panel_bg = to_egui_color32(theme.bg_secondary());
    let border_color = to_egui_color32(theme.bg_contrast());
    let dim = to_egui_color32(theme.text_dim());
    let primary = to_egui_color32(theme.text_primary());

    let frame = egui::Frame::NONE
        .fill(panel_bg)
        .inner_margin(egui::Margin::symmetric(8, 2));

    egui::TopBottomPanel::bottom("statusbar")
        .exact_height(STATUSBAR_HEIGHT)
        .frame(frame)
        .show_separator_line(false)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;

                // -- Connection status --
                render_connection_section(ui, &adsb_data, &theme);

                separator(ui, dim);

                // -- Aircraft count --
                let count = adsb_data
                    .as_ref()
                    .and_then(|d| d.try_aircraft_count())
                    .unwrap_or(0);
                ui.label(
                    egui::RichText::new(format!("{} aircraft", count))
                        .size(FONT_SIZE)
                        .color(primary),
                );

                separator(ui, dim);

                // -- Message rate --
                ui.label(
                    egui::RichText::new(format!("{:.0} msg/s", stats.message_rate))
                        .size(FONT_SIZE)
                        .color(primary),
                );

                separator(ui, dim);

                // -- FPS --
                ui.label(
                    egui::RichText::new(format!("{:.0} FPS", state.fps))
                        .size(FONT_SIZE)
                        .color(primary),
                );

                // -- Recording indicator (only when active) --
                if recording.is_recording {
                    separator(ui, dim);
                    let time_val = ui.input(|i| i.time);
                    let alpha = if (time_val * 2.0) as i32 % 2 == 0 {
                        255
                    } else {
                        100
                    };
                    let rec_color = egui::Color32::from_rgba_unmultiplied(255, 0, 0, alpha);
                    ui.label(
                        egui::RichText::new(format!("REC {}s", recording.duration_secs()))
                            .size(FONT_SIZE)
                            .color(rec_color)
                            .strong(),
                    );
                }

                // -- Right-aligned: map position + attribution --
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;

                    // Build version (rightmost)
                    ui.label(
                        egui::RichText::new(crate::build_info::version_short())
                            .size(FONT_SIZE)
                            .color(dim),
                    );

                    separator(ui, dim);

                    // Attribution
                    ui.label(
                        egui::RichText::new("\u{00A9} OSM, CartoDB")
                            .size(FONT_SIZE)
                            .color(dim),
                    );

                    separator(ui, dim);

                    // Map position + zoom
                    ui.label(
                        egui::RichText::new(format!(
                            "{:.4}, {:.4}  Z{}",
                            map_state.latitude,
                            map_state.longitude,
                            map_state.zoom_level.to_u8(),
                        ))
                        .size(FONT_SIZE)
                        .color(primary),
                    );
                });
            });
        });
}

/// Render the connection status dot and label.
fn render_connection_section(
    ui: &mut egui::Ui,
    adsb_data: &Option<Res<AdsbAircraftData>>,
    theme: &AppTheme,
) {
    let Some(data) = adsb_data else {
        let dim = to_egui_color32(theme.text_dim());
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, dim);
        ui.label(egui::RichText::new("No client").size(FONT_SIZE).color(dim));
        return;
    };

    let connection_state = data.get_connection_state();
    use adsb_client::ConnectionState;
    let (color, label) = match connection_state {
        ConnectionState::Connected => (to_egui_color32(theme.text_success()), "Connected"),
        ConnectionState::Connecting => (to_egui_color32(theme.text_warn()), "Connecting"),
        ConnectionState::Disconnected => (to_egui_color32(theme.text_error()), "Disconnected"),
        ConnectionState::Error(_) => (to_egui_color32(theme.text_error()), "Error"),
    };

    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    ui.label(egui::RichText::new(label).size(FONT_SIZE).color(color));
}

/// Draw a dim vertical separator between statusbar sections.
fn separator(ui: &mut egui::Ui, color: egui::Color32) {
    ui.label(egui::RichText::new("|").size(FONT_SIZE).color(color));
}
