use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy_slippy_tiles::ZoomLevel;

use crate::aircraft::AircraftListState;
use crate::config::{save_config, AircraftBookmark, AppConfig, LocationBookmark};
use crate::theme::{to_egui_color32, to_egui_color32_alpha, AppTheme};
use crate::{Aircraft, MapState, ZoomState};

pub struct BookmarksPlugin;

impl Plugin for BookmarksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BookmarksPanelState>().add_systems(
            Update,
            (toggle_bookmarks_panel, highlight_bookmarked_aircraft),
        );
    }
}

/// State for the bookmarks panel
#[derive(Resource, Default)]
pub struct BookmarksPanelState {
    /// Whether the panel is open
    pub open: bool,
    /// Input for new location name
    pub new_location_name: String,
    /// Input for aircraft note when bookmarking
    pub aircraft_note: String,
    /// Whether we're in "add location" mode
    pub adding_location: bool,
    /// Selected tab (0 = Locations, 1 = Aircraft)
    pub selected_tab: usize,
}

/// System to render the bookmarks panel
pub fn render_bookmarks_panel(
    mut contexts: EguiContexts,
    mut panel_state: ResMut<BookmarksPanelState>,
    mut app_config: ResMut<AppConfig>,
    mut map_state: ResMut<MapState>,
    mut zoom_state: ResMut<ZoomState>,
    list_state: Res<AircraftListState>,
    aircraft_query: Query<&Aircraft>,
    theme: Res<AppTheme>,
) {
    if !panel_state.open {
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let panel_bg = to_egui_color32_alpha(theme.bg_secondary(), 240);
    let border_color = to_egui_color32(theme.bg_contrast());
    let bookmark_color = to_egui_color32(theme.accent_secondary());

    let panel_frame = egui::Frame::default()
        .fill(panel_bg)
        .stroke(egui::Stroke::new(1.0, border_color))
        .inner_margin(egui::Margin::same(8));

    let mut window_open = true;
    egui::Window::new("Bookmarks")
        .open(&mut window_open)
        .collapsible(true)
        .resizable(true)
        .default_width(280.0)
        .min_width(250.0)
        .frame(panel_frame)
        .show(ctx, |ui| {
            // Tab bar
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(panel_state.selected_tab == 0, "Locations")
                    .clicked()
                {
                    panel_state.selected_tab = 0;
                }
                if ui
                    .selectable_label(panel_state.selected_tab == 1, "Aircraft")
                    .clicked()
                {
                    panel_state.selected_tab = 1;
                }
            });

            ui.add_space(8.0);

            match panel_state.selected_tab {
                0 => {
                    // Location bookmarks
                    render_location_bookmarks(
                        ui,
                        &mut panel_state,
                        &mut app_config,
                        &mut map_state,
                        &mut zoom_state,
                        bookmark_color,
                    );
                }
                1 => {
                    // Aircraft bookmarks
                    render_aircraft_bookmarks(
                        ui,
                        &mut panel_state,
                        &mut app_config,
                        &list_state,
                        &aircraft_query,
                        bookmark_color,
                    );
                }
                _ => {}
            }
        });

    if !window_open {
        panel_state.open = false;
    }
}

/// Render bookmarks content into a bare `egui::Ui` (for dock/tab usage).
///
/// This contains the same content as `render_bookmarks_panel` but without
/// the `Window` wrapper or open-state check, so it can be embedded in
/// an `egui_tiles` pane.
pub fn render_bookmarks_pane_content(
    ui: &mut egui::Ui,
    panel_state: &mut BookmarksPanelState,
    app_config: &mut AppConfig,
    map_state: &mut MapState,
    zoom_state: &mut ZoomState,
    list_state: &AircraftListState,
    aircraft_query: &Query<&Aircraft>,
    theme: &AppTheme,
) {
    let bookmark_color = to_egui_color32(theme.accent_secondary());

    // Tab bar
    ui.horizontal(|ui| {
        if ui
            .selectable_label(panel_state.selected_tab == 0, "Locations")
            .clicked()
        {
            panel_state.selected_tab = 0;
        }
        if ui
            .selectable_label(panel_state.selected_tab == 1, "Aircraft")
            .clicked()
        {
            panel_state.selected_tab = 1;
        }
    });

    ui.add_space(8.0);

    match panel_state.selected_tab {
        0 => {
            render_location_bookmarks(
                ui,
                panel_state,
                app_config,
                map_state,
                zoom_state,
                bookmark_color,
            );
        }
        1 => {
            render_aircraft_bookmarks(
                ui,
                panel_state,
                app_config,
                list_state,
                aircraft_query,
                bookmark_color,
            );
        }
        _ => {}
    }
}

fn render_location_bookmarks(
    ui: &mut egui::Ui,
    panel_state: &mut BookmarksPanelState,
    app_config: &mut AppConfig,
    map_state: &mut MapState,
    zoom_state: &mut ZoomState,
    bookmark_color: egui::Color32,
) {
    // Add current location button
    if panel_state.adding_location {
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut panel_state.new_location_name);
        });
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() && !panel_state.new_location_name.is_empty() {
                let bookmark = LocationBookmark {
                    name: panel_state.new_location_name.clone(),
                    latitude: map_state.latitude,
                    longitude: map_state.longitude,
                    zoom: map_state.zoom_level.to_u8(),
                };
                app_config.bookmarks.locations.push(bookmark);
                save_config(app_config);
                panel_state.new_location_name.clear();
                panel_state.adding_location = false;
            }
            if ui.button("Cancel").clicked() {
                panel_state.adding_location = false;
                panel_state.new_location_name.clear();
            }
        });
    } else {
        if ui.button("+ Add Current Location").clicked() {
            panel_state.adding_location = true;
        }
    }

    ui.add_space(8.0);

    // List of location bookmarks
    let mut to_remove: Option<usize> = None;
    let mut to_jump: Option<usize> = None;

    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            for (idx, bookmark) in app_config.bookmarks.locations.iter().enumerate() {
                ui.horizontal(|ui| {
                    // Bookmark name (clickable to jump)
                    if ui
                        .button(egui::RichText::new(&bookmark.name).color(bookmark_color))
                        .clicked()
                    {
                        to_jump = Some(idx);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("X").clicked() {
                            to_remove = Some(idx);
                        }
                        ui.label(
                            egui::RichText::new(format!("z{}", bookmark.zoom))
                                .size(10.0)
                                .color(egui::Color32::GRAY),
                        );
                    });
                });
            }

            if app_config.bookmarks.locations.is_empty() {
                ui.label(egui::RichText::new("No location bookmarks").color(egui::Color32::GRAY));
            }
        });

    // Handle actions
    if let Some(idx) = to_jump {
        if let Some(bookmark) = app_config.bookmarks.locations.get(idx) {
            map_state.latitude = bookmark.latitude;
            map_state.longitude = bookmark.longitude;
            if let Ok(zoom) = ZoomLevel::try_from(bookmark.zoom) {
                map_state.zoom_level = zoom;
            }
            zoom_state.camera_zoom = 1.0;
            info!("Jumped to bookmark: {}", bookmark.name);
        }
    }

    if let Some(idx) = to_remove {
        app_config.bookmarks.locations.remove(idx);
        save_config(app_config);
    }
}

fn render_aircraft_bookmarks(
    ui: &mut egui::Ui,
    panel_state: &mut BookmarksPanelState,
    app_config: &mut AppConfig,
    list_state: &AircraftListState,
    aircraft_query: &Query<&Aircraft>,
    bookmark_color: egui::Color32,
) {
    // Add selected aircraft button
    if let Some(ref selected_icao) = list_state.selected_icao {
        // Check if already bookmarked
        let is_bookmarked = app_config
            .bookmarks
            .aircraft
            .iter()
            .any(|b| &b.icao == selected_icao);
        if !is_bookmarked {
            ui.horizontal(|ui| {
                if ui.button("+ Bookmark Selected").clicked() {
                    // Find the aircraft to get its callsign
                    let callsign = aircraft_query
                        .iter()
                        .find(|a| &a.icao == selected_icao)
                        .and_then(|a| a.callsign.clone());

                    let bookmark = AircraftBookmark {
                        icao: selected_icao.clone(),
                        callsign,
                        note: if panel_state.aircraft_note.is_empty() {
                            None
                        } else {
                            Some(panel_state.aircraft_note.clone())
                        },
                    };
                    app_config.bookmarks.aircraft.push(bookmark);
                    save_config(app_config);
                    panel_state.aircraft_note.clear();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Note:");
                ui.text_edit_singleline(&mut panel_state.aircraft_note);
            });
        } else {
            ui.label(
                egui::RichText::new(format!("{} is bookmarked", selected_icao))
                    .color(bookmark_color),
            );
        }
    } else {
        ui.label(egui::RichText::new("Select an aircraft to bookmark").color(egui::Color32::GRAY));
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);

    // List of aircraft bookmarks
    let mut to_remove: Option<usize> = None;

    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            for (idx, bookmark) in app_config.bookmarks.aircraft.iter().enumerate() {
                // Check if this aircraft is currently visible
                let is_visible = aircraft_query.iter().any(|a| a.icao == bookmark.icao);
                let status_color = if is_visible {
                    egui::Color32::from_rgb(100, 200, 100) // Green for visible
                } else {
                    egui::Color32::GRAY
                };

                ui.horizontal(|ui| {
                    // Status indicator
                    ui.label(
                        egui::RichText::new(if is_visible { "[*]" } else { "[ ]" })
                            .color(status_color),
                    );

                    // ICAO
                    ui.label(
                        egui::RichText::new(&bookmark.icao)
                            .color(bookmark_color)
                            .monospace(),
                    );

                    // Callsign if available
                    if let Some(ref callsign) = bookmark.callsign {
                        ui.label(egui::RichText::new(callsign).color(egui::Color32::LIGHT_GRAY));
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("X").clicked() {
                            to_remove = Some(idx);
                        }
                    });
                });

                // Show note if present
                if let Some(ref note) = bookmark.note {
                    ui.indent("note", |ui| {
                        ui.label(
                            egui::RichText::new(note)
                                .size(10.0)
                                .color(egui::Color32::GRAY),
                        );
                    });
                }
            }

            if app_config.bookmarks.aircraft.is_empty() {
                ui.label(egui::RichText::new("No aircraft bookmarks").color(egui::Color32::GRAY));
            }
        });

    // Handle removal
    if let Some(idx) = to_remove {
        app_config.bookmarks.aircraft.remove(idx);
        save_config(app_config);
    }
}

/// Toggle bookmarks panel with 'B' key
pub fn toggle_bookmarks_panel(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut panel_state: ResMut<BookmarksPanelState>,
    mut contexts: EguiContexts,
) {
    // Don't toggle if egui wants input
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    if keyboard.just_pressed(KeyCode::KeyB) {
        panel_state.open = !panel_state.open;
    }
}

/// Component to mark bookmarked aircraft with a special indicator
#[derive(Component)]
pub struct BookmarkedAircraftIndicator;

/// System to highlight bookmarked aircraft in the list
pub fn highlight_bookmarked_aircraft(
    app_config: Res<AppConfig>,
    _list_state: ResMut<AircraftListState>,
) {
    // Update the list state with bookmarked ICAOs for highlighting
    // This is used by the aircraft list panel to show special styling
    if !app_config.is_changed() {
        return;
    }

    // The actual highlighting will be handled in the list_panel render function
    // by checking if the ICAO is in the bookmarks list
}
