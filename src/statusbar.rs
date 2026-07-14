use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::collections::HashMap;
use std::time::Instant;

use crate::adsb::FeedConnectionManager;
use crate::aircraft::stats_panel::StatsPanelState;
use crate::recording::RecordingState;
use crate::theme::{to_egui_color32, AppTheme};
use crate::MapState;

/// FPS smoothing state and message rate tracking.
#[derive(Resource)]
pub struct StatusBarState {
    pub fps: f32,
    pub show_feed_popup: bool,
    last_msg_count: u64,
    last_msg_check: Instant,
    pub message_rate: f32,
    last_per_feed_counts: HashMap<String, u64>,
    pub per_feed_rates: HashMap<String, f32>,
}

impl Default for StatusBarState {
    fn default() -> Self {
        Self {
            fps: 0.0,
            show_feed_popup: false,
            last_msg_count: 0,
            last_msg_check: Instant::now(),
            message_rate: 0.0,
            last_per_feed_counts: HashMap::new(),
            per_feed_rates: HashMap::new(),
        }
    }
}

const STATUSBAR_HEIGHT: f32 = 22.0;
const FONT_SIZE: f32 = 11.0;
const FPS_SMOOTHING: f32 = 0.05;
const MSG_RATE_INTERVAL_SECS: f32 = 1.0;

pub fn render_statusbar(
    mut contexts: EguiContexts,
    theme: Res<AppTheme>,
    feed_mgr: Option<Res<FeedConnectionManager>>,
    stats: Res<StatsPanelState>,
    recording: Res<RecordingState>,
    map_state: Res<MapState>,
    time: Res<Time>,
    mut state: ResMut<StatusBarState>,
) {
    let dt = time.delta_secs();
    if dt > 0.0 {
        let instant_fps = 1.0 / dt;
        if state.fps == 0.0 {
            state.fps = instant_fps;
        } else {
            state.fps += FPS_SMOOTHING * (instant_fps - state.fps);
        }
    }

    // Update message rate from feed manager (total and per-feed)
    if let Some(ref mgr) = feed_mgr {
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_msg_check).as_secs_f32();
        if elapsed >= MSG_RATE_INTERVAL_SECS {
            let current_count = mgr.total_message_count();
            let delta = current_count.saturating_sub(state.last_msg_count);
            state.message_rate = delta as f32 / elapsed;
            state.last_msg_count = current_count;
            state.last_msg_check = now;

            for feed_stats in mgr.per_feed_stats() {
                let prev = state.last_per_feed_counts.get(&feed_stats.name).copied().unwrap_or(0);
                let feed_delta = feed_stats.message_count.saturating_sub(prev);
                state.per_feed_rates.insert(feed_stats.name.clone(), feed_delta as f32 / elapsed);
                state.last_per_feed_counts.insert(feed_stats.name, feed_stats.message_count);
            }
        }
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let panel_bg = to_egui_color32(theme.bg_secondary());
    let dim = to_egui_color32(theme.text_dim());
    let primary = to_egui_color32(theme.text_primary());

    let frame = egui::Frame::NONE
        .fill(panel_bg)
        .inner_margin(egui::Margin::symmetric(8, 2));

    let show_popup = state.show_feed_popup;
    let msg_rate = state.message_rate;

    egui::TopBottomPanel::bottom("statusbar")
        .exact_height(STATUSBAR_HEIGHT)
        .frame(frame)
        .show_separator_line(false)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;

                // -- Clickable feed status area --
                let feed_section_response = render_feed_status_section(
                    ui, &feed_mgr, &theme, msg_rate,
                );

                if feed_section_response.clicked() {
                    state.show_feed_popup = !state.show_feed_popup;
                }

                // Show popup below the clickable area
                if show_popup {
                    let per_feed_rates = state.per_feed_rates.clone();
                    render_feed_popup(ui, &feed_mgr, &theme, msg_rate, &per_feed_rates, &mut state.show_feed_popup);
                }

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

                    ui.label(
                        egui::RichText::new(crate::build_info::version_short())
                            .size(FONT_SIZE)
                            .color(dim),
                    );

                    separator(ui, dim);

                    ui.label(
                        egui::RichText::new("\u{00A9} OSM, CartoDB")
                            .size(FONT_SIZE)
                            .color(dim),
                    );

                    separator(ui, dim);

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

/// Render the clickable feed status section: connection dot + aircraft count + msg/s.
/// Returns the response for click detection.
fn render_feed_status_section(
    ui: &mut egui::Ui,
    feed_mgr: &Option<Res<FeedConnectionManager>>,
    theme: &AppTheme,
    msg_rate: f32,
) -> egui::Response {
    let Some(mgr) = feed_mgr else {
        let dim = to_egui_color32(theme.text_dim());
        return ui.label(egui::RichText::new("No feeds").size(FONT_SIZE).color(dim));
    };

    let total = mgr.connections.len();
    let connected = mgr.connected_count();
    let connecting = mgr.connecting_count();
    let aircraft_count = mgr.unique_aircraft_count();
    let primary = to_egui_color32(theme.text_primary());

    let (dot_color, status_label) = if total == 0 {
        (to_egui_color32(theme.text_dim()), "No feeds".to_string())
    } else if connected == total {
        (to_egui_color32(theme.text_success()), format!("{} feeds", total))
    } else if connecting > 0 && connected == 0 {
        (to_egui_color32(theme.text_warn()), "Connecting...".to_string())
    } else if connected > 0 {
        (to_egui_color32(theme.text_warn()), format!("{}/{} feeds", connected, total))
    } else {
        (to_egui_color32(theme.text_error()), "Disconnected".to_string())
    };

    // Group the clickable area
    let response = ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, dot_color);
        ui.label(egui::RichText::new(status_label).size(FONT_SIZE).color(dot_color));

        ui.label(egui::RichText::new("|").size(FONT_SIZE).color(to_egui_color32(theme.text_dim())));
        ui.label(egui::RichText::new(format!("{} aircraft", aircraft_count)).size(FONT_SIZE).color(primary));

        ui.label(egui::RichText::new("|").size(FONT_SIZE).color(to_egui_color32(theme.text_dim())));
        ui.label(egui::RichText::new(format!("{:.0} msg/s", msg_rate)).size(FONT_SIZE).color(primary));
    });

    response.response.interact(egui::Sense::click())
}

/// Render the per-feed detail popup.
fn render_feed_popup(
    ui: &mut egui::Ui,
    feed_mgr: &Option<Res<FeedConnectionManager>>,
    theme: &AppTheme,
    total_msg_rate: f32,
    per_feed_rates: &HashMap<String, f32>,
    show: &mut bool,
) {
    let Some(mgr) = feed_mgr else {
        return;
    };

    let panel_bg = to_egui_color32(theme.bg_secondary());
    let border = to_egui_color32(theme.bg_contrast());
    let dim = to_egui_color32(theme.text_dim());
    let primary = to_egui_color32(theme.text_primary());

    let popup_frame = egui::Frame::default()
        .fill(panel_bg)
        .stroke(egui::Stroke::new(1.0, border))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(8));

    egui::Area::new(egui::Id::new("feed_status_popup"))
        .fixed_pos(egui::pos2(50.0, ui.ctx().screen_rect().bottom() - STATUSBAR_HEIGHT - 10.0))
        .pivot(egui::Align2::LEFT_BOTTOM)
        .interactable(true)
        .show(ui.ctx(), |ui| {
            popup_frame.show(ui, |ui| {
                ui.set_min_width(300.0);

                ui.strong("Feed Status");
                ui.separator();

                egui::Grid::new("feed_stats_grid")
                    .num_columns(4)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Feed").size(10.0).color(dim));
                        ui.label(egui::RichText::new("Status").size(10.0).color(dim));
                        ui.label(egui::RichText::new("Aircraft").size(10.0).color(dim));
                        ui.label(egui::RichText::new("msg/s").size(10.0).color(dim));
                        ui.end_row();

                        for stats in mgr.per_feed_stats() {
                            use adsb_client::ConnectionState;
                            let (state_color, state_text) = match &stats.state {
                                ConnectionState::Connected => (to_egui_color32(theme.text_success()), "Connected"),
                                ConnectionState::Connecting => (to_egui_color32(theme.text_warn()), "Connecting"),
                                ConnectionState::Disconnected => (to_egui_color32(theme.text_error()), "Disconnected"),
                                ConnectionState::Error(_) => (to_egui_color32(theme.text_error()), "Error"),
                            };

                            let feed_rate = per_feed_rates.get(&stats.name).copied().unwrap_or(0.0);

                            ui.label(egui::RichText::new(&stats.name).size(11.0).color(primary));
                            ui.label(egui::RichText::new(state_text).size(11.0).color(state_color));
                            ui.label(egui::RichText::new(format!("{}", stats.aircraft_count)).size(11.0).color(primary));
                            ui.label(egui::RichText::new(format!("{:.0}", feed_rate)).size(11.0).color(primary));
                            ui.end_row();
                        }

                        ui.separator();
                        ui.end_row();
                        ui.label(egui::RichText::new("Total (unique)").size(11.0).color(primary).strong());
                        ui.label(egui::RichText::new("").size(11.0));
                        ui.label(egui::RichText::new(format!("{}", mgr.unique_aircraft_count())).size(11.0).color(primary).strong());
                        ui.label(egui::RichText::new(format!("{:.0}", total_msg_rate)).size(11.0).color(primary).strong());
                        ui.end_row();
                    });
            });
        });

    // Close popup on Escape
    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        *show = false;
    }
}

fn separator(ui: &mut egui::Ui, color: egui::Color32) {
    ui.label(egui::RichText::new("|").size(FONT_SIZE).color(color));
}
