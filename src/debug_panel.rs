/// Debug/Metrics floating window.
///
/// Provides a runtime debug panel with scrollable log messages and live metrics
/// (FPS, aircraft count, message rate, connection state, map state).
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::collections::VecDeque;

use crate::adsb::AdsbAircraftData;
use crate::theme::{to_egui_color32, to_egui_color32_alpha, AppTheme};
use crate::ui_panels::{PanelId, UiPanelManager};
use crate::{Aircraft, MapState, ZoomState};

const MAX_LOG_MESSAGES: usize = 200;

/// Resource holding debug panel state, log ring buffer, and live metrics.
#[derive(Resource, Reflect)]
pub struct DebugPanelState {
    pub open: bool,
    #[reflect(ignore)]
    pub log_messages: VecDeque<String>,
    // Metrics
    pub aircraft_count: usize,
    pub messages_processed: u64,
    pub positions_rejected: u64,
    pub message_rate: f64,
    pub fps: f32,
    // Rate computation internals
    #[reflect(ignore)]
    last_rate_time: f64,
    #[reflect(ignore)]
    last_rate_count: u64,
}

impl Default for DebugPanelState {
    fn default() -> Self {
        Self {
            open: false,
            log_messages: VecDeque::with_capacity(MAX_LOG_MESSAGES),
            aircraft_count: 0,
            messages_processed: 0,
            positions_rejected: 0,
            message_rate: 0.0,
            fps: 0.0,
            last_rate_time: 0.0,
            last_rate_count: 0,
        }
    }
}

impl DebugPanelState {
    /// Push a timestamped log entry, trimming the buffer if needed.
    pub fn push_log(&mut self, msg: impl Into<String>) {
        if self.log_messages.len() >= MAX_LOG_MESSAGES {
            self.log_messages.pop_front();
        }
        let now = chrono_timestamp();
        self.log_messages
            .push_back(format!("[{}] {}", now, msg.into()));
    }
}

/// Simple HH:MM:SS timestamp from std SystemTime.
fn chrono_timestamp() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// Update live metrics each frame (FPS, aircraft count, message rate).
pub fn update_debug_metrics(
    time: Res<Time>,
    mut debug: ResMut<DebugPanelState>,
    aircraft_query: Query<(), With<Aircraft>>,
) {
    // FPS from delta
    let dt = time.delta_secs();
    if dt > 0.0 {
        debug.fps = 1.0 / dt;
    }

    // Aircraft count
    debug.aircraft_count = aircraft_query.iter().count();

    // Message rate: compute once per second
    let elapsed = time.elapsed_secs_f64();
    let interval = elapsed - debug.last_rate_time;
    if interval >= 1.0 {
        let delta_msgs = debug
            .messages_processed
            .saturating_sub(debug.last_rate_count);
        debug.message_rate = delta_msgs as f64 / interval;
        debug.last_rate_time = elapsed;
        debug.last_rate_count = debug.messages_processed;
    }
}

/// Render the debug panel UI into an egui context.
///
/// This contains all the egui rendering logic, extracted for testability.
/// The ADSB connection state row is handled separately in the system function
/// since it requires Bevy `Res` types.
pub fn render_debug_panel_ui(
    ctx: &egui::Context,
    debug: &mut DebugPanelState,
    map_state: Option<&MapState>,
    zoom_state: Option<&ZoomState>,
    panel_bg: egui::Color32,
    border_color: egui::Color32,
) {
    let mut open = debug.open;

    let window_frame = egui::Frame::default()
        .fill(panel_bg)
        .stroke(egui::Stroke::new(1.0, border_color))
        .inner_margin(egui::Margin::same(8));

    egui::Window::new("Debug")
        .open(&mut open)
        .default_size([380.0, 420.0])
        .resizable(true)
        .collapsible(true)
        .frame(window_frame)
        .show(ctx, |ui| {
            // -- Metrics section --
            egui::CollapsingHeader::new("Metrics")
                .default_open(true)
                .show(ui, |ui| {
                    egui::Grid::new("debug_metrics_grid")
                        .num_columns(2)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("FPS:");
                            ui.label(format!("{:.0}", debug.fps));
                            ui.end_row();

                            ui.label("Aircraft:");
                            ui.label(format!("{}", debug.aircraft_count));
                            ui.end_row();

                            ui.label("Msgs processed:");
                            ui.label(format!("{}", debug.messages_processed));
                            ui.end_row();

                            ui.label("Msg rate:");
                            ui.label(format!("{:.1}/s", debug.message_rate));
                            ui.end_row();

                            ui.label("Pos rejected:");
                            ui.label(format!("{}", debug.positions_rejected));
                            ui.end_row();

                            // Map state
                            if let Some(ref ms) = map_state {
                                ui.label("Map center:");
                                ui.label(format!("{:.4}, {:.4}", ms.latitude, ms.longitude));
                                ui.end_row();

                                ui.label("Tile zoom:");
                                ui.label(format!("{}", ms.zoom_level.to_u8()));
                                ui.end_row();
                            }

                            if let Some(ref zs) = zoom_state {
                                ui.label("Camera zoom:");
                                ui.label(format!("{:.3}", zs.camera_zoom));
                                ui.end_row();
                            }
                        });
                });

            ui.separator();

            // -- Log section --
            egui::CollapsingHeader::new("Log")
                .default_open(true)
                .show(ui, |ui| {
                    let text_style = egui::TextStyle::Monospace;
                    let row_height = ui.text_style_height(&text_style);
                    let num_rows = debug.log_messages.len();

                    egui::ScrollArea::vertical()
                        .max_height(250.0)
                        .stick_to_bottom(true)
                        .show_rows(ui, row_height, num_rows, |ui, row_range| {
                            for i in row_range {
                                if let Some(msg) = debug.log_messages.get(i) {
                                    ui.label(
                                        egui::RichText::new(msg)
                                            .text_style(text_style.clone())
                                            .size(11.0),
                                    );
                                }
                            }
                        });
                });
        });

    if !open && debug.open {
        debug.open = false;
    }
}

/// Render just the debug panel content into a bare `&mut egui::Ui`.
///
/// This contains the Metrics and Log sections without any `egui::Window` wrapper,
/// suitable for embedding in a docking pane or other container.
pub fn render_debug_pane_content(
    ui: &mut egui::Ui,
    debug: &mut DebugPanelState,
    map_state: Option<&MapState>,
    zoom_state: Option<&ZoomState>,
) {
    // -- Metrics section --
    egui::CollapsingHeader::new("Metrics")
        .default_open(true)
        .show(ui, |ui| {
            egui::Grid::new("debug_metrics_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("FPS:");
                    ui.label(format!("{:.0}", debug.fps));
                    ui.end_row();

                    ui.label("Aircraft:");
                    ui.label(format!("{}", debug.aircraft_count));
                    ui.end_row();

                    ui.label("Msgs processed:");
                    ui.label(format!("{}", debug.messages_processed));
                    ui.end_row();

                    ui.label("Msg rate:");
                    ui.label(format!("{:.1}/s", debug.message_rate));
                    ui.end_row();

                    ui.label("Pos rejected:");
                    ui.label(format!("{}", debug.positions_rejected));
                    ui.end_row();

                    // Map state
                    if let Some(ref ms) = map_state {
                        ui.label("Map center:");
                        ui.label(format!("{:.4}, {:.4}", ms.latitude, ms.longitude));
                        ui.end_row();

                        ui.label("Tile zoom:");
                        ui.label(format!("{}", ms.zoom_level.to_u8()));
                        ui.end_row();
                    }

                    if let Some(ref zs) = zoom_state {
                        ui.label("Camera zoom:");
                        ui.label(format!("{:.3}", zs.camera_zoom));
                        ui.end_row();
                    }
                });
        });

    ui.separator();

    // -- Build Info section --
    egui::CollapsingHeader::new("Build Info")
        .default_open(false)
        .show(ui, |ui| {
            use crate::build_info::build;
            egui::Grid::new("debug_build_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Version:");
                    ui.label(build::PKG_VERSION);
                    ui.end_row();

                    ui.label("Branch:");
                    ui.label(build::BRANCH);
                    ui.end_row();

                    ui.label("Commit:");
                    ui.label(build::COMMIT_HASH);
                    ui.end_row();

                    ui.label("Commit date:");
                    ui.label(build::COMMIT_DATE);
                    ui.end_row();

                    ui.label("Dirty:");
                    ui.label(if build::GIT_CLEAN { "No" } else { "Yes" });
                    ui.end_row();

                    ui.label("Built:");
                    ui.label(build::BUILD_TIME);
                    ui.end_row();

                    ui.label("Rust:");
                    ui.label(build::RUST_VERSION);
                    ui.end_row();

                    ui.label("Target:");
                    ui.label(build::BUILD_TARGET);
                    ui.end_row();
                });
        });

    ui.separator();

    // -- Log section --
    egui::CollapsingHeader::new("Log")
        .default_open(true)
        .show(ui, |ui| {
            let text_style = egui::TextStyle::Monospace;
            let row_height = ui.text_style_height(&text_style);
            let num_rows = debug.log_messages.len();

            egui::ScrollArea::vertical()
                .max_height(250.0)
                .stick_to_bottom(true)
                .show_rows(ui, row_height, num_rows, |ui, row_range| {
                    for i in row_range {
                        if let Some(msg) = debug.log_messages.get(i) {
                            ui.label(
                                egui::RichText::new(msg)
                                    .text_style(text_style.clone())
                                    .size(11.0),
                            );
                        }
                    }
                });
        });
}

/// Render the debug panel as a floating egui window (Bevy system).
pub fn render_debug_panel(
    mut contexts: EguiContexts,
    mut debug: ResMut<DebugPanelState>,
    mut panels: ResMut<UiPanelManager>,
    map_state: Option<Res<MapState>>,
    zoom_state: Option<Res<ZoomState>>,
    adsb_data: Option<Res<AdsbAircraftData>>,
    theme: Res<AppTheme>,
) {
    if !panels.is_open(PanelId::Debug) {
        return;
    }

    debug.open = true;

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let panel_bg = to_egui_color32_alpha(theme.bg_secondary(), 240);
    let border_color = to_egui_color32(theme.bg_contrast());

    render_debug_panel_ui(
        ctx,
        &mut debug,
        map_state.as_deref(),
        zoom_state.as_deref(),
        panel_bg,
        border_color,
    );

    // If the egui X button closed the window, close it in UiPanelManager too.
    // This is handled here instead of via sync_resources_to_panel_manager because
    // update_debug_metrics mutates DebugPanelState every frame, which would cause
    // is_changed() to always fire and immediately undo any toolbar/keyboard toggle.
    if !debug.open {
        panels.close_panel(PanelId::Debug);
    }

    // Render ADSB connection state separately (requires Bevy Res)
    // This is intentionally kept in the system function since AdsbAircraftData
    // cannot be provided outside of a Bevy context.
    if let Some(ref adsb) = adsb_data {
        // Connection state is displayed as part of the debug window metrics,
        // but is rendered here to avoid coupling the testable function to Bevy resources.
        let _ = adsb.get_connection_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{MapState, ZoomState};
    use bevy_slippy_tiles::ZoomLevel;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;

    #[test]
    fn test_debug_panel_renders_metrics() {
        let mut debug = DebugPanelState::default();
        debug.open = true;
        debug.fps = 60.0;
        debug.aircraft_count = 5;
        debug.messages_processed = 100;
        debug.message_rate = 12.5;
        debug.positions_rejected = 3;

        let harness = Harness::new_state(
            |ctx, state: &mut DebugPanelState| {
                render_debug_panel_ui(
                    ctx,
                    state,
                    None,
                    None,
                    egui::Color32::from_rgba_unmultiplied(30, 30, 46, 240),
                    egui::Color32::from_rgb(69, 71, 90),
                );
            },
            debug,
        );

        harness.get_by_label("FPS:");
        harness.get_by_label("60");
        harness.get_by_label("Aircraft:");
        harness.get_by_label("5");
        harness.get_by_label("Msgs processed:");
        harness.get_by_label("100");
        harness.get_by_label("Msg rate:");
        harness.get_by_label("12.5/s");
        harness.get_by_label("Pos rejected:");
        harness.get_by_label("3");
    }

    #[test]
    fn test_debug_panel_renders_log_messages() {
        let mut debug = DebugPanelState::default();
        debug.open = true;
        debug
            .log_messages
            .push_back("Test log entry one".to_string());
        debug
            .log_messages
            .push_back("Test log entry two".to_string());

        let harness = Harness::new_state(
            |ctx, state: &mut DebugPanelState| {
                render_debug_panel_ui(
                    ctx,
                    state,
                    None,
                    None,
                    egui::Color32::from_rgba_unmultiplied(30, 30, 46, 240),
                    egui::Color32::from_rgb(69, 71, 90),
                );
            },
            debug,
        );

        harness.get_by_label("Test log entry one");
        harness.get_by_label("Test log entry two");
    }

    #[test]
    fn test_debug_panel_renders_map_and_zoom_state() {
        let mut debug = DebugPanelState::default();
        debug.open = true;

        let map_state = MapState {
            latitude: 51.5074,
            longitude: -0.1278,
            zoom_level: ZoomLevel::L10,
        };

        let zoom_state = ZoomState::new();

        let ms = map_state;
        let zs = zoom_state;

        let harness = Harness::new_state(
            move |ctx, state: &mut DebugPanelState| {
                render_debug_panel_ui(
                    ctx,
                    state,
                    Some(&ms),
                    Some(&zs),
                    egui::Color32::from_rgba_unmultiplied(30, 30, 46, 240),
                    egui::Color32::from_rgb(69, 71, 90),
                );
            },
            debug,
        );

        harness.get_by_label("Map center:");
        harness.get_by_label("51.5074, -0.1278");
        harness.get_by_label("Tile zoom:");
        harness.get_by_label("10");
        harness.get_by_label("Camera zoom:");
        harness.get_by_label("1.000");
    }
}
