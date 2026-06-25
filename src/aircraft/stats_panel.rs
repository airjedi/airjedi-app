use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::time::Instant;

use crate::theme::{to_egui_color32, to_egui_color32_alpha, AppTheme};
use crate::Aircraft;

/// State for the statistics panel
#[derive(Resource)]
pub struct StatsPanelState {
    /// Whether the stats panel is expanded
    pub expanded: bool,
    /// Session start time
    pub session_start: Instant,
    /// Last message count (for rate calculation)
    pub last_message_count: u64,
    /// Last rate check time
    pub last_rate_check: Instant,
    /// Current message rate (messages per second)
    pub message_rate: f32,
}

impl Default for StatsPanelState {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            expanded: false,
            session_start: now,
            last_message_count: 0,
            last_rate_check: now,
            message_rate: 0.0,
        }
    }
}

/// Statistics about aircraft by altitude band
#[derive(Default)]
pub struct AltitudeBandStats {
    pub ground_to_10k: usize,
    pub ten_to_25k: usize,
    pub twentyfive_to_40k: usize,
    pub above_40k: usize,
    pub unknown: usize,
}

impl AltitudeBandStats {
    pub fn from_aircraft<'a>(aircraft: impl Iterator<Item = &'a Aircraft>) -> Self {
        let mut stats = Self::default();
        for ac in aircraft {
            match ac.altitude {
                Some(alt) if alt < 10000 => stats.ground_to_10k += 1,
                Some(alt) if alt < 25000 => stats.ten_to_25k += 1,
                Some(alt) if alt < 40000 => stats.twentyfive_to_40k += 1,
                Some(_) => stats.above_40k += 1,
                None => stats.unknown += 1,
            }
        }
        stats
    }

    pub fn total(&self) -> usize {
        self.ground_to_10k
            + self.ten_to_25k
            + self.twentyfive_to_40k
            + self.above_40k
            + self.unknown
    }
}

/// Format duration as HH:MM:SS
fn format_duration(secs: u64) -> String {
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;
    format!("{:02}:{:02}:{:02}", hours, mins, secs)
}

/// Component to mark the stats panel toggle button
#[derive(Component)]
pub struct StatsPanelButton;

/// System to render the statistics panel
pub fn render_stats_panel(
    mut contexts: EguiContexts,
    mut stats_state: ResMut<StatsPanelState>,
    aircraft_query: Query<&Aircraft>,
    theme: Res<AppTheme>,
) {
    if !stats_state.expanded {
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    // Calculate statistics
    let total_aircraft = aircraft_query.iter().count();
    let altitude_stats = AltitudeBandStats::from_aircraft(aircraft_query.iter());
    let session_duration = stats_state.session_start.elapsed().as_secs();

    // Connection status is shown elsewhere in UI already
    let connection_status = "See status bar".to_string();

    // Define colors from theme
    let panel_bg = to_egui_color32_alpha(theme.bg_secondary(), 230);
    let border_color = to_egui_color32(theme.bg_contrast());
    let label_color = to_egui_color32(theme.text_dim());
    let value_color = to_egui_color32(theme.text_primary());
    let alt_low_color = to_egui_color32(theme.altitude_low());
    let alt_med_color = to_egui_color32(theme.text_warn());
    let alt_high_color = to_egui_color32(theme.altitude_high());
    let alt_ultra_color = to_egui_color32(theme.accent_secondary());

    let panel_frame = egui::Frame::default()
        .fill(panel_bg)
        .stroke(egui::Stroke::new(1.0, border_color))
        .inner_margin(egui::Margin::same(8));

    let mut window_open = true;
    egui::Window::new("Statistics")
        .open(&mut window_open)
        .collapsible(true)
        .resizable(false)
        .frame(panel_frame)
        .show(ctx, |ui| {
            // Total aircraft
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Total Aircraft:")
                        .color(label_color)
                        .size(11.0),
                );
                ui.label(
                    egui::RichText::new(format!("{}", total_aircraft))
                        .color(value_color)
                        .size(12.0)
                        .strong()
                        .monospace(),
                );
            });

            ui.add_space(8.0);

            // Altitude bands section
            ui.label(
                egui::RichText::new("By Altitude")
                    .color(label_color)
                    .size(10.0),
            );

            egui::Grid::new("altitude_grid")
                .num_columns(2)
                .spacing([20.0, 2.0])
                .show(ui, |ui| {
                    // Ground to 10k
                    ui.label(
                        egui::RichText::new("0 - 10,000 ft")
                            .color(alt_low_color)
                            .size(9.0),
                    );
                    ui.label(
                        egui::RichText::new(format!("{}", altitude_stats.ground_to_10k))
                            .color(value_color)
                            .size(10.0)
                            .monospace(),
                    );
                    ui.end_row();

                    // 10k to 25k
                    ui.label(
                        egui::RichText::new("10,000 - 25,000 ft")
                            .color(alt_med_color)
                            .size(9.0),
                    );
                    ui.label(
                        egui::RichText::new(format!("{}", altitude_stats.ten_to_25k))
                            .color(value_color)
                            .size(10.0)
                            .monospace(),
                    );
                    ui.end_row();

                    // 25k to 40k
                    ui.label(
                        egui::RichText::new("25,000 - 40,000 ft")
                            .color(alt_high_color)
                            .size(9.0),
                    );
                    ui.label(
                        egui::RichText::new(format!("{}", altitude_stats.twentyfive_to_40k))
                            .color(value_color)
                            .size(10.0)
                            .monospace(),
                    );
                    ui.end_row();

                    // Above 40k
                    ui.label(
                        egui::RichText::new("40,000+ ft")
                            .color(alt_ultra_color)
                            .size(9.0),
                    );
                    ui.label(
                        egui::RichText::new(format!("{}", altitude_stats.above_40k))
                            .color(value_color)
                            .size(10.0)
                            .monospace(),
                    );
                    ui.end_row();

                    // Unknown
                    if altitude_stats.unknown > 0 {
                        ui.label(
                            egui::RichText::new("Unknown")
                                .color(to_egui_color32(theme.bg_overlay()))
                                .size(9.0),
                        );
                        ui.label(
                            egui::RichText::new(format!("{}", altitude_stats.unknown))
                                .color(value_color)
                                .size(10.0)
                                .monospace(),
                        );
                        ui.end_row();
                    }
                });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            // Connection info
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Connection:")
                        .color(label_color)
                        .size(10.0),
                );
                ui.label(
                    egui::RichText::new(&connection_status)
                        .color(value_color)
                        .size(10.0),
                );
            });

            // Session duration
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Session:")
                        .color(label_color)
                        .size(10.0),
                );
                ui.label(
                    egui::RichText::new(format_duration(session_duration))
                        .color(value_color)
                        .size(10.0)
                        .monospace(),
                );
            });
        });

    if !window_open {
        stats_state.expanded = false;
    }
}

/// Render statistics content into a bare `egui::Ui` (for dock/tab usage).
///
/// This contains the same content as `render_stats_panel` but without
/// the `Window` wrapper or expanded-state check, so it can be embedded
/// in an `egui_tiles` pane.
pub fn render_stats_pane_content(
    ui: &mut egui::Ui,
    stats_state: &StatsPanelState,
    aircraft_query: &Query<&Aircraft>,
    theme: &AppTheme,
) {
    let total_aircraft = aircraft_query.iter().count();
    let altitude_stats = AltitudeBandStats::from_aircraft(aircraft_query.iter());
    let session_duration = stats_state.session_start.elapsed().as_secs();
    let connection_status = "See status bar".to_string();

    let label_color = to_egui_color32(theme.text_dim());
    let value_color = to_egui_color32(theme.text_primary());
    let alt_low_color = to_egui_color32(theme.altitude_low());
    let alt_med_color = to_egui_color32(theme.text_warn());
    let alt_high_color = to_egui_color32(theme.altitude_high());
    let alt_ultra_color = to_egui_color32(theme.accent_secondary());

    // Total aircraft
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Total Aircraft:")
                .color(label_color)
                .size(11.0),
        );
        ui.label(
            egui::RichText::new(format!("{}", total_aircraft))
                .color(value_color)
                .size(12.0)
                .strong()
                .monospace(),
        );
    });

    ui.add_space(8.0);

    // Altitude bands section
    ui.label(
        egui::RichText::new("By Altitude")
            .color(label_color)
            .size(10.0),
    );

    egui::Grid::new("altitude_grid")
        .num_columns(2)
        .spacing([20.0, 2.0])
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("0 - 10,000 ft")
                    .color(alt_low_color)
                    .size(9.0),
            );
            ui.label(
                egui::RichText::new(format!("{}", altitude_stats.ground_to_10k))
                    .color(value_color)
                    .size(10.0)
                    .monospace(),
            );
            ui.end_row();

            ui.label(
                egui::RichText::new("10,000 - 25,000 ft")
                    .color(alt_med_color)
                    .size(9.0),
            );
            ui.label(
                egui::RichText::new(format!("{}", altitude_stats.ten_to_25k))
                    .color(value_color)
                    .size(10.0)
                    .monospace(),
            );
            ui.end_row();

            ui.label(
                egui::RichText::new("25,000 - 40,000 ft")
                    .color(alt_high_color)
                    .size(9.0),
            );
            ui.label(
                egui::RichText::new(format!("{}", altitude_stats.twentyfive_to_40k))
                    .color(value_color)
                    .size(10.0)
                    .monospace(),
            );
            ui.end_row();

            ui.label(
                egui::RichText::new("40,000+ ft")
                    .color(alt_ultra_color)
                    .size(9.0),
            );
            ui.label(
                egui::RichText::new(format!("{}", altitude_stats.above_40k))
                    .color(value_color)
                    .size(10.0)
                    .monospace(),
            );
            ui.end_row();

            if altitude_stats.unknown > 0 {
                ui.label(
                    egui::RichText::new("Unknown")
                        .color(to_egui_color32(theme.bg_overlay()))
                        .size(9.0),
                );
                ui.label(
                    egui::RichText::new(format!("{}", altitude_stats.unknown))
                        .color(value_color)
                        .size(10.0)
                        .monospace(),
                );
                ui.end_row();
            }
        });

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);

    // Connection info
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Connection:")
                .color(label_color)
                .size(10.0),
        );
        ui.label(
            egui::RichText::new(&connection_status)
                .color(value_color)
                .size(10.0),
        );
    });

    // Session duration
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Session:")
                .color(label_color)
                .size(10.0),
        );
        ui.label(
            egui::RichText::new(format_duration(session_duration))
                .color(value_color)
                .size(10.0)
                .monospace(),
        );
    });
}
