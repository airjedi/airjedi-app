use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::MapState;
use crate::geo::{haversine_distance_nm, CoordinateConverter};
use crate::theme::{AppTheme, to_egui_color32, to_egui_color32_alpha};
use crate::widgets::{DataStrip, ArcGauge, WidgetTheme};
use super::{CameraFollowState, DetailPanelState, TrailHistory, SessionClock};
use super::typeinfo::AircraftTypeInfo;
use super::altitude::{format_altitude, format_altitude_with_indicator};

/// Sort criteria for aircraft list
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortCriteria {
    #[default]
    Distance,
    Altitude,
    Speed,
    Callsign,
    Type,
}

impl SortCriteria {
    pub fn label(&self) -> &'static str {
        match self {
            SortCriteria::Distance => "Distance",
            SortCriteria::Altitude => "Altitude",
            SortCriteria::Speed => "Speed",
            SortCriteria::Callsign => "Callsign",
            SortCriteria::Type => "Type",
        }
    }
}

/// Filter settings for aircraft list
#[derive(Debug, Clone)]
pub struct AircraftFilters {
    pub min_altitude: i32,
    pub max_altitude: i32,
    pub min_speed: f64,
    pub max_speed: f64,
    pub max_distance: f64,
    /// Callsign/operator prefix filter (e.g., "AAL", "UAL", "SWA")
    pub callsign_prefix: String,
    /// Whether to include ground traffic (altitude = 0 or very low)
    pub include_ground_traffic: bool,
    /// Whether to only show aircraft with valid position data
    pub require_position: bool,
}

impl Default for AircraftFilters {
    fn default() -> Self {
        Self {
            min_altitude: 0,
            max_altitude: 60000,
            min_speed: 0.0,
            max_speed: 600.0,
            max_distance: 250.0,
            callsign_prefix: String::new(),
            include_ground_traffic: true,
            require_position: true,
        }
    }
}

/// State for the aircraft list panel
#[derive(Resource)]
pub struct AircraftListState {
    pub expanded: bool,
    pub width: f32,
    pub sort_by: SortCriteria,
    pub sort_ascending: bool,
    pub filters: AircraftFilters,
    pub search_text: String,
    pub selected_icao: Option<String>,
    pub show_filter_popup: bool,
}

impl Default for AircraftListState {
    fn default() -> Self {
        Self {
            expanded: false, // Start collapsed
            width: 304.0, // Match desktop panel width
            sort_by: SortCriteria::Distance,
            sort_ascending: true,
            filters: AircraftFilters::default(),
            search_text: String::new(),
            selected_icao: None,
            show_filter_popup: false,
        }
    }
}

/// Component to mark the aircraft list toggle button
#[derive(Component)]
pub struct AircraftListButton;

/// Cached aircraft data for display
#[derive(Clone)]
pub struct AircraftDisplayData {
    pub icao: String,
    pub callsign: Option<String>,
    pub altitude: Option<i32>,
    pub velocity: Option<f64>,
    pub heading: Option<f32>,
    pub vertical_rate: Option<i32>,
    pub distance: f64,
    pub squawk: Option<String>,
    pub type_code: Option<String>,
    pub manufacturer_model: Option<String>,
    pub registration: Option<String>,
}

/// Resource holding sorted/filtered aircraft for display
#[derive(Resource, Default)]
pub struct AircraftDisplayList {
    pub aircraft: Vec<AircraftDisplayData>,
}

/// System to populate and sort the aircraft display list
pub fn update_aircraft_display_list(
    list_state: Res<AircraftListState>,
    app_config: Res<crate::config::AppConfig>,
    aircraft_query: Query<(&crate::Aircraft, Option<&AircraftTypeInfo>)>,
    mut display_list: ResMut<AircraftDisplayList>,
) {
    let center_lat = app_config.map.default_latitude;
    let center_lon = app_config.map.default_longitude;
    let search = list_state.search_text.to_lowercase();

    // Get callsign prefix filter (lowercase for comparison)
    let callsign_prefix = list_state.filters.callsign_prefix.to_lowercase();

    // Collect and filter aircraft
    let mut aircraft: Vec<AircraftDisplayData> = aircraft_query
        .iter()
        .filter_map(|(a, type_info)| {
            let distance = haversine_distance_nm(center_lat, center_lon, a.latitude, a.longitude);

            // Apply filters
            if distance > list_state.filters.max_distance {
                return None;
            }

            // Ground traffic filter
            if !list_state.filters.include_ground_traffic {
                // Consider aircraft on ground if altitude is 0 or below 100 ft
                if let Some(alt) = a.altitude {
                    if alt < 100 {
                        return None;
                    }
                }
            }

            if let Some(alt) = a.altitude {
                if alt < list_state.filters.min_altitude || alt > list_state.filters.max_altitude {
                    return None;
                }
            }

            if let Some(vel) = a.velocity {
                if vel < list_state.filters.min_speed || vel > list_state.filters.max_speed {
                    return None;
                }
            }

            // Apply callsign prefix filter
            if !callsign_prefix.is_empty() {
                let matches_prefix = a.callsign.as_ref()
                    .map(|c| c.to_lowercase().starts_with(&callsign_prefix))
                    .unwrap_or(false);
                if !matches_prefix {
                    return None;
                }
            }

            // Apply search filter
            if !search.is_empty() {
                let callsign_match = a.callsign.as_ref()
                    .map(|c| c.to_lowercase().contains(&search))
                    .unwrap_or(false);
                let icao_match = a.icao.to_lowercase().contains(&search);
                if !callsign_match && !icao_match {
                    return None;
                }
            }

            Some(AircraftDisplayData {
                icao: a.icao.clone(),
                callsign: a.callsign.clone(),
                altitude: a.altitude,
                velocity: a.velocity,
                heading: a.heading,
                vertical_rate: a.vertical_rate,
                distance,
                squawk: a.squawk.clone(),
                type_code: type_info.and_then(|ti| ti.type_code.clone()),
                manufacturer_model: type_info.and_then(|ti| ti.manufacturer_model.clone()),
                registration: type_info.and_then(|ti| ti.registration.clone()),
            })
        })
        .collect();

    // Sort
    match list_state.sort_by {
        SortCriteria::Distance => {
            aircraft.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        }
        SortCriteria::Altitude => {
            aircraft.sort_by(|a, b| {
                a.altitude.unwrap_or(0).cmp(&b.altitude.unwrap_or(0))
            });
        }
        SortCriteria::Speed => {
            aircraft.sort_by(|a, b| {
                a.velocity.unwrap_or(0.0).total_cmp(&b.velocity.unwrap_or(0.0))
            });
        }
        SortCriteria::Callsign => {
            aircraft.sort_by(|a, b| {
                let a_call = a.callsign.as_deref().unwrap_or(&a.icao);
                let b_call = b.callsign.as_deref().unwrap_or(&b.icao);
                a_call.cmp(b_call)
            });
        }
        SortCriteria::Type => {
            aircraft.sort_by(|a, b| {
                let a_type = a.type_code.as_deref().unwrap_or("");
                let b_type = b.type_code.as_deref().unwrap_or("");
                a_type.cmp(b_type)
            });
        }
    }

    if !list_state.sort_ascending {
        aircraft.reverse();
    }

    display_list.aircraft = aircraft;
}

/// Helper function to get altitude color based on altitude value
fn get_altitude_color(altitude: Option<i32>) -> (egui::Color32, &'static str) {
    match altitude {
        Some(alt) if alt >= 30000 => (egui::Color32::from_rgb(200, 100, 255), "▲"), // High - purple
        Some(alt) if alt >= 20000 => (egui::Color32::from_rgb(255, 150, 50), "▲"),  // Medium-high - orange
        Some(alt) if alt >= 10000 => (egui::Color32::from_rgb(200, 200, 100), "▲"), // Medium - yellow
        Some(_) => (egui::Color32::from_rgb(100, 200, 200), "▼"),                   // Low - cyan
        None => (egui::Color32::from_rgb(100, 100, 100), "─"),                      // Unknown - grey
    }
}

/// System to render the aircraft list panel with stacked detail section.
///
/// The right panel contains an upper scrollable aircraft list and a lower
/// collapsible detail section for the selected aircraft. This replaces the
/// previous separate floating detail window.
pub fn render_aircraft_list_panel(
    mut contexts: EguiContexts,
    mut list_state: ResMut<AircraftListState>,
    _detail_state: ResMut<DetailPanelState>,
    mut follow_state: ResMut<CameraFollowState>,
    display_list: Res<AircraftDisplayList>,
    map_state: Res<MapState>,
    app_config: Res<crate::config::AppConfig>,
    clock: Res<SessionClock>,
    aircraft_query: Query<(&crate::Aircraft, &TrailHistory, Option<&AircraftTypeInfo>)>,
    theme: Res<AppTheme>,
) {
    if !list_state.expanded {
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    // Define colors matching desktop theme
    let panel_bg = to_egui_color32_alpha(theme.bg_secondary(), 230);
    let border_color = to_egui_color32(theme.bg_contrast());
    let selected_bg = egui::Color32::from_rgba_unmultiplied(100, 140, 180, 26);
    let header_color = egui::Color32::from_rgb(150, 150, 150);
    let icao_color = egui::Color32::from_rgb(200, 220, 255);
    let callsign_color = egui::Color32::from_rgb(150, 220, 150);
    let callsign_selected_color = egui::Color32::from_rgb(255, 50, 50);
    let metrics_color = egui::Color32::from_rgb(170, 170, 170);
    let range_color = egui::Color32::from_rgb(100, 200, 255);
    let status_active = egui::Color32::from_rgb(100, 255, 100);

    // Create custom frame for the panel
    let panel_frame = egui::Frame::default()
        .fill(panel_bg)
        .stroke(egui::Stroke::new(1.0, border_color))
        .inner_margin(egui::Margin::same(8));

    egui::SidePanel::right("aircraft_list_panel")
        .default_width(list_state.width)
        .resizable(true)
        .frame(panel_frame)
        .show(ctx, |ui| {
            // Header
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("Aircraft ({})", display_list.aircraft.len()))
                    .color(egui::Color32::from_rgb(200, 200, 200))
                    .size(14.0)
                    .strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(egui::RichText::new("X").size(12.0)).clicked() {
                        list_state.expanded = false;
                    }
                });
            });

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            // Sort dropdown
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Sort:")
                    .color(header_color)
                    .size(11.0));
                egui::ComboBox::from_id_salt("sort_by")
                    .selected_text(list_state.sort_by.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut list_state.sort_by, SortCriteria::Distance, "Distance");
                        ui.selectable_value(&mut list_state.sort_by, SortCriteria::Altitude, "Altitude");
                        ui.selectable_value(&mut list_state.sort_by, SortCriteria::Speed, "Speed");
                        ui.selectable_value(&mut list_state.sort_by, SortCriteria::Callsign, "Callsign");
                    });

                if ui.button(if list_state.sort_ascending { "\u{2191}" } else { "\u{2193}" }).clicked() {
                    list_state.sort_ascending = !list_state.sort_ascending;
                }

                if ui.button("Filter").clicked() {
                    list_state.show_filter_popup = !list_state.show_filter_popup;
                }
            });

            // Search box
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Search:")
                    .color(header_color)
                    .size(11.0));
                ui.text_edit_singleline(&mut list_state.search_text);
            });

            ui.add_space(4.0);

            // Filter popup
            if list_state.show_filter_popup {
                egui::Frame::group(ui.style())
                    .fill(egui::Color32::from_rgba_unmultiplied(35, 40, 45, 230))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Altitude (ft):")
                            .color(header_color)
                            .size(10.0));
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut list_state.filters.min_altitude)
                                .range(0..=60000)
                                .prefix("Min: "));
                            ui.add(egui::DragValue::new(&mut list_state.filters.max_altitude)
                                .range(0..=60000)
                                .prefix("Max: "));
                        });

                        ui.label(egui::RichText::new("Speed (kts):")
                            .color(header_color)
                            .size(10.0));
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut list_state.filters.min_speed)
                                .range(0.0..=600.0)
                                .prefix("Min: "));
                            ui.add(egui::DragValue::new(&mut list_state.filters.max_speed)
                                .range(0.0..=600.0)
                                .prefix("Max: "));
                        });

                        ui.label(egui::RichText::new("Distance (nm):")
                            .color(header_color)
                            .size(10.0));
                        ui.add(egui::DragValue::new(&mut list_state.filters.max_distance)
                            .range(0.0..=500.0)
                            .prefix("Max: "));

                        ui.add_space(4.0);

                        // Callsign/operator prefix filter
                        ui.label(egui::RichText::new("Callsign Prefix:")
                            .color(header_color)
                            .size(10.0));
                        ui.add(egui::TextEdit::singleline(&mut list_state.filters.callsign_prefix)
                            .hint_text("e.g., AAL, UAL, SWA")
                            .desired_width(120.0));

                        ui.add_space(4.0);

                        // Ground traffic toggle
                        ui.checkbox(&mut list_state.filters.include_ground_traffic,
                            egui::RichText::new("Include ground traffic")
                                .color(header_color)
                                .size(10.0));

                        ui.add_space(4.0);

                        if ui.button("Close").clicked() {
                            list_state.show_filter_popup = false;
                        }
                    });
                ui.add_space(4.0);
            }

            // Aircraft count
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("TOTAL: {}", display_list.aircraft.len()))
                    .color(header_color)
                    .size(11.0)
                    .monospace());
            });

            ui.add_space(4.0);

            // -- Aircraft list --
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;

                    for aircraft in &display_list.aircraft {
                        let is_selected = list_state.selected_icao.as_ref() == Some(&aircraft.icao);
                        let (alt_color, alt_indicator) = get_altitude_color(aircraft.altitude);

                        // Animated expand/collapse
                        let anim_id = ui.id().with(&aircraft.icao).with("expand");
                        let expand_t = ui.ctx().animate_bool_with_time(anim_id, is_selected, 0.15);

                        // Create card frame with selection highlighting (no outline)
                        let card_frame = if is_selected {
                            egui::Frame::NONE
                                .fill(selected_bg)
                                .inner_margin(egui::Margin::symmetric(4, 3))
                        } else {
                            egui::Frame::NONE
                                .inner_margin(egui::Margin::symmetric(4, 3))
                        };

                        let card_response = card_frame.show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;

                            // Row 1: Chevron + Status + ICAO + Callsign + Altitude + Follow button
                            ui.horizontal(|ui| {
                                // Collapse/expand chevron
                                let chevron = if is_selected { "\u{25BE}" } else { "\u{25B8}" };
                                ui.label(egui::RichText::new(chevron)
                                    .color(egui::Color32::from_rgb(120, 120, 120))
                                    .size(11.0));

                                // Status indicator
                                ui.label(egui::RichText::new("\u{25CF}")
                                    .color(status_active)
                                    .size(13.0));

                                // Display tail number if known, otherwise ICAO
                                let id_label = aircraft.registration.as_deref()
                                    .unwrap_or(&aircraft.icao);
                                ui.label(egui::RichText::new(id_label)
                                    .color(icao_color)
                                    .size(13.0)
                                    .monospace()
                                    .strong());

                                // Callsign
                                if let Some(ref callsign) = aircraft.callsign {
                                    let cs_color = if is_selected {
                                        callsign_selected_color
                                    } else {
                                        callsign_color
                                    };
                                    ui.label(egui::RichText::new(format!("{}", callsign.trim()))
                                        .color(cs_color)
                                        .size(13.0)
                                        .strong());
                                }

                                // Altitude with indicator
                                if let Some(alt) = aircraft.altitude {
                                    let alt_text = format!("{}", format_altitude_with_indicator(alt, alt_indicator));
                                    ui.label(egui::RichText::new(alt_text)
                                        .color(alt_color)
                                        .size(12.0)
                                        .monospace());
                                }

                                // Follow button (top right)
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let is_following = follow_state.following_icao.as_ref() == Some(&aircraft.icao);
                                    let follow_text = if is_following { "Unfollow" } else { "Follow" };
                                    let follow_color = if is_following {
                                        egui::Color32::from_rgb(255, 100, 100)
                                    } else {
                                        egui::Color32::from_rgb(100, 180, 255)
                                    };
                                    if ui.add(egui::Button::new(
                                        egui::RichText::new(follow_text)
                                            .color(follow_color)
                                            .size(10.0)
                                    ).small()).clicked() {
                                        if is_following {
                                            follow_state.following_icao = None;
                                        } else {
                                            follow_state.following_icao = Some(aircraft.icao.clone());
                                        }
                                    }
                                });
                            });

                            // Row 2: Speed + Heading + Range
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;

                                if let Some(vel) = aircraft.velocity {
                                    ui.label(egui::RichText::new(format!("{:03}kt", vel as i32))
                                        .color(metrics_color)
                                        .size(11.0)
                                        .monospace());
                                }

                                if let Some(heading) = aircraft.heading {
                                    ui.label(egui::RichText::new(format!("{:03}\u{00B0}", heading as i32))
                                        .color(metrics_color)
                                        .size(11.0)
                                        .monospace());
                                }

                                if let Some(ref sq) = aircraft.squawk {
                                    ui.label(egui::RichText::new(sq)
                                        .color(metrics_color)
                                        .size(11.0)
                                        .monospace());
                                }

                                ui.label(egui::RichText::new(format!("{:.1}nm", aircraft.distance))
                                    .color(range_color)
                                    .size(11.0)
                                    .monospace());
                            });

                            // Row 3: Vertical rate + manufacturer/model (bottom right)
                            ui.horizontal(|ui| {
                                if let Some(vr) = aircraft.vertical_rate {
                                    let (vr_color, vr_symbol) = if vr > 100 {
                                        (egui::Color32::from_rgb(100, 255, 100), "\u{2191}")
                                    } else if vr < -100 {
                                        (egui::Color32::from_rgb(255, 150, 100), "\u{2193}")
                                    } else {
                                        (egui::Color32::from_rgb(150, 150, 150), "\u{2500}")
                                    };
                                    ui.label(egui::RichText::new(format!("{} {}ft/min", vr_symbol, vr))
                                        .color(vr_color)
                                        .size(10.0)
                                        .monospace());
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if let Some(ref mm) = aircraft.manufacturer_model {
                                        ui.label(egui::RichText::new(mm)
                                            .color(egui::Color32::from_rgb(180, 160, 220))
                                            .size(10.0));
                                    }
                                });
                            });

                            // Inline expanded detail section (animated)
                            if expand_t > 0.0 {
                                render_inline_detail(
                                    ui,
                                    &aircraft.icao,
                                    expand_t,
                                    &mut follow_state,
                                    &app_config,
                                    &clock,
                                    &aircraft_query,
                                    &theme,
                                );
                            }
                        });

                        // Handle click to select/deselect (toggle)
                        if card_response.response.interact(egui::Sense::click()).clicked() {
                            if is_selected {
                                list_state.selected_icao = None;
                            } else {
                                list_state.selected_icao = Some(aircraft.icao.clone());
                            }
                        }

                        // Subtle separator line between items
                        ui.add_space(2.0);
                        let separator_color = egui::Color32::from_rgb(50, 55, 65);
                        let rect = ui.available_rect_before_wrap();
                        ui.painter().hline(
                            rect.x_range(),
                            rect.top(),
                            egui::Stroke::new(1.0, separator_color),
                        );
                        ui.add_space(2.0);
                    }
                });
        });
}

/// Render aircraft list content into a bare `egui::Ui` (for dock/tab usage).
///
/// This contains the same content as `render_aircraft_list_panel` but without
/// the `SidePanel` wrapper or expanded-state check, so it can be embedded in
/// an `egui_tiles` pane.
pub fn render_aircraft_list_pane_content(
    ui: &mut egui::Ui,
    list_state: &mut AircraftListState,
    _detail_state: &mut DetailPanelState,
    follow_state: &mut CameraFollowState,
    display_list: &AircraftDisplayList,
    app_config: &crate::config::AppConfig,
    clock: &SessionClock,
    aircraft_query: &Query<(&crate::Aircraft, &TrailHistory, Option<&AircraftTypeInfo>)>,
    theme: &AppTheme,
) {
    let selected_bg = egui::Color32::from_rgba_unmultiplied(100, 140, 180, 26);
    let header_color = egui::Color32::from_rgb(150, 150, 150);
    let icao_color = egui::Color32::from_rgb(200, 220, 255);
    let callsign_color = egui::Color32::from_rgb(150, 220, 150);
    let callsign_selected_color = egui::Color32::from_rgb(255, 50, 50);
    let metrics_color = egui::Color32::from_rgb(170, 170, 170);
    let range_color = egui::Color32::from_rgb(100, 200, 255);

    // Header
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("Aircraft ({})", display_list.aircraft.len()))
            .color(egui::Color32::from_rgb(200, 200, 200))
            .size(14.0)
            .strong());
    });

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    // Sort dropdown
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Sort:")
            .color(header_color)
            .size(11.0));
        egui::ComboBox::from_id_salt("sort_by")
            .selected_text(list_state.sort_by.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut list_state.sort_by, SortCriteria::Distance, "Distance");
                ui.selectable_value(&mut list_state.sort_by, SortCriteria::Altitude, "Altitude");
                ui.selectable_value(&mut list_state.sort_by, SortCriteria::Speed, "Speed");
                ui.selectable_value(&mut list_state.sort_by, SortCriteria::Callsign, "Callsign");
            });

        if ui.button(if list_state.sort_ascending { "\u{2191}" } else { "\u{2193}" }).clicked() {
            list_state.sort_ascending = !list_state.sort_ascending;
        }

        if ui.button("Filter").clicked() {
            list_state.show_filter_popup = !list_state.show_filter_popup;
        }
    });

    // Search box
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Search:")
            .color(header_color)
            .size(11.0));
        ui.text_edit_singleline(&mut list_state.search_text);
    });

    ui.add_space(4.0);

    // Filter popup
    if list_state.show_filter_popup {
        egui::Frame::group(ui.style())
            .fill(egui::Color32::from_rgba_unmultiplied(35, 40, 45, 230))
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Altitude (ft):")
                    .color(header_color)
                    .size(10.0));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut list_state.filters.min_altitude)
                        .range(0..=60000)
                        .prefix("Min: "));
                    ui.add(egui::DragValue::new(&mut list_state.filters.max_altitude)
                        .range(0..=60000)
                        .prefix("Max: "));
                });

                ui.label(egui::RichText::new("Speed (kts):")
                    .color(header_color)
                    .size(10.0));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut list_state.filters.min_speed)
                        .range(0.0..=600.0)
                        .prefix("Min: "));
                    ui.add(egui::DragValue::new(&mut list_state.filters.max_speed)
                        .range(0.0..=600.0)
                        .prefix("Max: "));
                });

                ui.label(egui::RichText::new("Distance (nm):")
                    .color(header_color)
                    .size(10.0));
                ui.add(egui::DragValue::new(&mut list_state.filters.max_distance)
                    .range(0.0..=500.0)
                    .prefix("Max: "));

                ui.add_space(4.0);

                // Callsign/operator prefix filter
                ui.label(egui::RichText::new("Callsign Prefix:")
                    .color(header_color)
                    .size(10.0));
                ui.add(egui::TextEdit::singleline(&mut list_state.filters.callsign_prefix)
                    .hint_text("e.g., AAL, UAL, SWA")
                    .desired_width(120.0));

                ui.add_space(4.0);

                // Ground traffic toggle
                ui.checkbox(&mut list_state.filters.include_ground_traffic,
                    egui::RichText::new("Include ground traffic")
                        .color(header_color)
                        .size(10.0));

                ui.add_space(4.0);

                if ui.button("Close").clicked() {
                    list_state.show_filter_popup = false;
                }
            });
        ui.add_space(4.0);
    }

    // Aircraft count
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("TOTAL: {}", display_list.aircraft.len()))
            .color(header_color)
            .size(11.0)
            .monospace());
    });

    ui.add_space(4.0);

    // -- Aircraft list --
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;

            for aircraft in &display_list.aircraft {
                let is_selected = list_state.selected_icao.as_ref() == Some(&aircraft.icao);
                let (alt_color, alt_indicator) = get_altitude_color(aircraft.altitude);

                // Animated expand/collapse
                let anim_id = ui.id().with(&aircraft.icao).with("expand");
                let expand_t = ui.ctx().animate_bool_with_time(anim_id, is_selected, 0.15);

                let card_frame = if is_selected {
                    egui::Frame::NONE
                        .fill(selected_bg)
                        .inner_margin(egui::Margin::symmetric(4, 3))
                } else {
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(4, 3))
                };

                let card_response = card_frame.show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;

                    // Row 1: Chevron + Status + ICAO + Callsign + Altitude + Follow button
                    ui.horizontal(|ui| {
                        // Collapse/expand chevron
                        let chevron = if is_selected { "\u{25BE}" } else { "\u{25B8}" };
                        ui.label(egui::RichText::new(chevron)
                            .color(egui::Color32::from_rgb(120, 120, 120))
                            .size(11.0));

                        ui.label(egui::RichText::new(&aircraft.icao)
                            .color(icao_color)
                            .size(13.0)
                            .monospace()
                            .strong());

                        if let Some(ref callsign) = aircraft.callsign {
                            let cs_color = if is_selected {
                                callsign_selected_color
                            } else {
                                callsign_color
                            };
                            ui.label(egui::RichText::new(format!("{}", callsign.trim()))
                                .color(cs_color)
                                .size(13.0)
                                .strong());
                        }

                        if let Some(alt) = aircraft.altitude {
                            let alt_text = format!("{}", format_altitude_with_indicator(alt, alt_indicator));
                            ui.label(egui::RichText::new(alt_text)
                                .color(alt_color)
                                .size(12.0)
                                .monospace());
                        }

                        // Follow button (top right)
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let is_following = follow_state.following_icao.as_ref() == Some(&aircraft.icao);
                            let follow_text = if is_following { "Unfollow" } else { "Follow" };
                            let follow_color = if is_following {
                                egui::Color32::from_rgb(255, 100, 100)
                            } else {
                                egui::Color32::from_rgb(100, 180, 255)
                            };
                            if ui.add(egui::Button::new(
                                egui::RichText::new(follow_text)
                                    .color(follow_color)
                                    .size(10.0)
                            ).small()).clicked() {
                                if is_following {
                                    follow_state.following_icao = None;
                                } else {
                                    follow_state.following_icao = Some(aircraft.icao.clone());
                                }
                            }
                        });
                    });

                    // Row 2: Speed + Heading + Range
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;

                        if let Some(vel) = aircraft.velocity {
                            ui.label(egui::RichText::new(format!("{:03}kt", vel as i32))
                                .color(metrics_color)
                                .size(11.0)
                                .monospace());
                        }

                        if let Some(heading) = aircraft.heading {
                            ui.label(egui::RichText::new(format!("{:03}\u{00B0}", heading as i32))
                                .color(metrics_color)
                                .size(11.0)
                                .monospace());
                        }

                        if let Some(ref sq) = aircraft.squawk {
                            ui.label(egui::RichText::new(sq)
                                .color(metrics_color)
                                .size(11.0)
                                .monospace());
                        }

                        ui.label(egui::RichText::new(format!("{:.1}nm", aircraft.distance))
                            .color(range_color)
                            .size(11.0)
                            .monospace());
                    });

                    // Row 3: Vertical rate + manufacturer/model (bottom right)
                    ui.horizontal(|ui| {
                        if let Some(vr) = aircraft.vertical_rate {
                            let (vr_color, vr_symbol) = if vr > 100 {
                                (egui::Color32::from_rgb(100, 255, 100), "\u{2191}")
                            } else if vr < -100 {
                                (egui::Color32::from_rgb(255, 150, 100), "\u{2193}")
                            } else {
                                (egui::Color32::from_rgb(150, 150, 150), "\u{2500}")
                            };
                            ui.label(egui::RichText::new(format!("{} {}ft/min", vr_symbol, vr))
                                .color(vr_color)
                                .size(10.0)
                                .monospace());
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if let Some(ref mm) = aircraft.manufacturer_model {
                                ui.label(egui::RichText::new(mm)
                                    .color(egui::Color32::from_rgb(180, 160, 220))
                                    .size(10.0));
                            }
                        });
                    });

                    // Inline expanded detail section (animated)
                    if expand_t > 0.0 {
                        render_inline_detail(
                            ui,
                            &aircraft.icao,
                            expand_t,
                            follow_state,
                            app_config,
                            clock,
                            aircraft_query,
                            theme,
                        );
                    }
                });

                // Handle click to select/deselect (toggle)
                if card_response.response.interact(egui::Sense::click()).clicked() {
                    if is_selected {
                        list_state.selected_icao = None;
                    } else {
                        list_state.selected_icao = Some(aircraft.icao.clone());
                    }
                }

                // Subtle separator line between items
                ui.add_space(2.0);
                let separator_color = egui::Color32::from_rgb(50, 55, 65);
                let rect = ui.available_rect_before_wrap();
                ui.painter().hline(
                    rect.x_range(),
                    rect.top(),
                    egui::Stroke::new(1.0, separator_color),
                );
                ui.add_space(2.0);
            }
        });
}

/// Render inline detail content within an expanded aircraft card.
///
/// `expand_t` is 0.0..1.0 animation progress; content is height-clipped accordingly.
fn render_inline_detail(
    ui: &mut egui::Ui,
    selected_icao: &str,
    expand_t: f32,
    follow_state: &mut CameraFollowState,
    app_config: &crate::config::AppConfig,
    clock: &SessionClock,
    aircraft_query: &Query<(&crate::Aircraft, &TrailHistory, Option<&AircraftTypeInfo>)>,
    theme: &AppTheme,
) {
    let Some((aircraft, trail, type_info)) = aircraft_query.iter().find(|(a, _, _)| a.icao == selected_icao) else {
        return;
    };

    let wt = WidgetTheme::from(theme);

    let distance_nm = haversine_distance_nm(
        app_config.map.default_latitude,
        app_config.map.default_longitude,
        aircraft.latitude,
        aircraft.longitude,
    );

    let oldest_point_age = trail.points.front().map(|p| clock.age_secs(p.timestamp) as u64);

    // Measure the full detail content height, then clip to expand_t fraction
    let detail_id = ui.id().with(selected_icao).with("detail_content");
    let full_height = ui.ctx().memory(|mem| {
        mem.data.get_temp::<f32>(detail_id).unwrap_or(200.0)
    });
    let visible_height = full_height * expand_t;

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(2.0);

    let response = ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), visible_height),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));

            // Position data strip
            DataStrip::new(&wt)
                .accent_left(wt.accent, 3.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Pos").color(wt.text_dim).size(10.0));
                        ui.label(
                            egui::RichText::new(format!("{:.4}, {:.4}", aircraft.latitude, aircraft.longitude))
                                .color(wt.text).size(10.0).monospace(),
                        );
                    });
                });

            ui.add_space(2.0);

            // Key metrics as data strips
            let mut pairs: Vec<(&str, String, egui::Color32)> = Vec::new();
            pairs.push(("Dist", format!("{:.1}nm", distance_nm), wt.accent));

            if let Some(ti) = type_info {
                if let Some(ref reg) = ti.registration {
                    pairs.push(("Reg", reg.clone(), wt.text));
                }
                if let Some(ref tc) = ti.type_code {
                    pairs.push(("Type", tc.clone(), wt.text));
                }
                if let Some(ref op) = ti.operator {
                    pairs.push(("Oper", op.clone(), wt.text));
                }
            }

            pairs.push(("Trk", format!("{}", trail.points.len()), wt.text));

            let dur_text = oldest_point_age
                .map(|secs| format!("{}:{:02}", secs / 60, secs % 60))
                .unwrap_or_else(|| "---".to_string());
            pairs.push(("Dur", dur_text, wt.text));

            // Render pairs as data strips, 2 per row
            for chunk in pairs.chunks(2) {
                DataStrip::new(&wt)
                    .accent_left(wt.border, 2.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(chunk[0].0).color(wt.text_dim).size(10.0));
                            ui.label(egui::RichText::new(&chunk[0].1).color(chunk[0].2).size(10.0).monospace());
                            if let Some(pair) = chunk.get(1) {
                                ui.add_space(12.0);
                                ui.label(egui::RichText::new(pair.0).color(wt.text_dim).size(10.0));
                                ui.label(egui::RichText::new(&pair.1).color(pair.2).size(10.0).monospace());
                            }
                        });
                    });
                ui.add_space(1.0);
            }

            ui.add_space(2.0);

            // Gauges row for key flight metrics
            ui.horizontal(|ui| {
                if let Some(alt) = aircraft.altitude {
                    let alt_norm = (alt as f32 / 45000.0).clamp(0.0, 1.0);
                    ui.add(ArcGauge::themed(alt_norm, &wt)
                        .size(60.0)
                        .label("ALT")
                        .value_text(&format!("{}", alt))
                        .tick_count(5)
                        .track_width(4.0)
                        .fill_width(4.0));
                }

                if let Some(speed) = aircraft.velocity {
                    let spd_norm = (speed as f32 / 600.0).clamp(0.0, 1.0);
                    ui.add(ArcGauge::themed(spd_norm, &wt)
                        .size(60.0)
                        .label("SPD")
                        .value_text(&format!("{:.0}", speed))
                        .fill_color(egui::Color32::from_rgb(100, 220, 150))
                        .tick_count(5)
                        .track_width(4.0)
                        .fill_width(4.0));
                }

                if let Some(hdg) = aircraft.heading {
                    let hdg_norm = hdg / 360.0;
                    ui.add(ArcGauge::themed(hdg_norm, &wt)
                        .size(60.0)
                        .label("HDG")
                        .value_text(&format!("{:.0}", hdg))
                        .fill_color(egui::Color32::from_rgb(220, 180, 100))
                        .sweep(360.0)
                        .tick_count(8)
                        .track_width(4.0)
                        .fill_width(4.0));
                }
            });

            ui.add_space(2.0);

            // Follow/Unfollow button
            ui.horizontal(|ui| {
                let is_following = follow_state.following_icao.as_deref() == Some(selected_icao);
                let follow_text = if is_following { "Unfollow" } else { "Follow" };
                let follow_color = if is_following {
                    egui::Color32::from_rgb(255, 100, 100)
                } else {
                    wt.accent
                };
                if ui.add(egui::Button::new(
                    egui::RichText::new(follow_text)
                        .color(follow_color)
                        .size(10.0)
                ).small()).clicked() {
                    if is_following {
                        follow_state.following_icao = None;
                    } else {
                        follow_state.following_icao = Some(selected_icao.to_string());
                    }
                }
            });
        },
    );

    let actual_height = response.response.rect.height().max(10.0);
    if expand_t >= 1.0 {
        ui.ctx().memory_mut(|mem| {
            mem.data.insert_temp(detail_id, actual_height);
        });
    }
}

/// Render the aircraft detail section inside the stacked right panel.
/// (Kept for reference; no longer called from the main panel flow.)
#[allow(dead_code)]
fn render_detail_section(
    ui: &mut egui::Ui,
    selected_icao: &str,
    detail_state: &mut DetailPanelState,
    follow_state: &mut CameraFollowState,
    app_config: &crate::config::AppConfig,
    clock: &SessionClock,
    aircraft_query: &Query<(&crate::Aircraft, &TrailHistory, Option<&AircraftTypeInfo>)>,
) {
    let label_color = egui::Color32::from_rgb(150, 150, 150);
    let value_color = egui::Color32::from_rgb(220, 220, 220);
    let callsign_color = egui::Color32::from_rgb(150, 220, 150);
    let highlight_color = egui::Color32::from_rgb(100, 200, 255);

    // Find the selected aircraft
    let Some((aircraft, trail, type_info)) = aircraft_query.iter().find(|(a, _, _)| a.icao == selected_icao) else {
        detail_state.open = false;
        detail_state.track_start = None;
        return;
    };

    let distance_nm = haversine_distance_nm(
        app_config.map.default_latitude,
        app_config.map.default_longitude,
        aircraft.latitude,
        aircraft.longitude,
    );

    let track_duration = detail_state.track_start.map(|start| start.elapsed().as_secs());
    let oldest_point_age = trail.points.front().map(|p| clock.age_secs(p.timestamp) as u64);

    ui.separator();
    ui.add_space(4.0);

    // Detail header
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(&aircraft.icao)
                .color(highlight_color)
                .size(14.0)
                .strong()
                .monospace(),
        );
        if let Some(ref callsign) = aircraft.callsign {
            ui.label(
                egui::RichText::new(callsign)
                    .color(callsign_color)
                    .size(12.0)
                    .strong(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(egui::RichText::new("X").size(11.0)).clicked() {
                detail_state.open = false;
            }
        });
    });

    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("detail_grid_stacked")
                .num_columns(2)
                .spacing([30.0, 4.0])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Position").color(label_color).size(11.0));
                    ui.label(
                        egui::RichText::new(format!("{:.4}, {:.4}", aircraft.latitude, aircraft.longitude))
                            .color(value_color).size(11.0).monospace(),
                    );
                    ui.end_row();

                    ui.label(egui::RichText::new("Altitude").color(label_color).size(11.0));
                    let alt_text = format_altitude(aircraft.altitude);
                    ui.label(egui::RichText::new(alt_text).color(value_color).size(11.0).monospace());
                    ui.end_row();

                    ui.label(egui::RichText::new("Speed").color(label_color).size(11.0));
                    let speed_text = aircraft.velocity
                        .map(|v| format!("{} kts", v as i32))
                        .unwrap_or_else(|| "---".to_string());
                    ui.label(egui::RichText::new(speed_text).color(value_color).size(11.0).monospace());
                    ui.end_row();

                    ui.label(egui::RichText::new("Heading").color(label_color).size(11.0));
                    let heading_text = aircraft.heading
                        .map(|h| format!("{:03}\u{00B0}", h as i32))
                        .unwrap_or_else(|| "---".to_string());
                    ui.label(egui::RichText::new(heading_text).color(value_color).size(11.0).monospace());
                    ui.end_row();

                    ui.label(egui::RichText::new("V/Rate").color(label_color).size(11.0));
                    let vr_text = aircraft.vertical_rate
                        .map(|vr| {
                            let sym = if vr > 100 { "+" } else { "" };
                            format!("{}{} ft/min", sym, vr)
                        })
                        .unwrap_or_else(|| "---".to_string());
                    ui.label(egui::RichText::new(vr_text).color(value_color).size(11.0).monospace());
                    ui.end_row();

                    ui.label(egui::RichText::new("Squawk").color(label_color).size(11.0));
                    let squawk_text = aircraft.squawk.as_deref().unwrap_or("---");
                    ui.label(egui::RichText::new(squawk_text).color(value_color).size(11.0).monospace());
                    ui.end_row();

                    ui.label(egui::RichText::new("Distance").color(label_color).size(11.0));
                    ui.label(egui::RichText::new(format!("{:.1} nm", distance_nm))
                        .color(highlight_color).size(11.0).monospace());
                    ui.end_row();

                    ui.label(egui::RichText::new("Track Pts").color(label_color).size(11.0));
                    ui.label(egui::RichText::new(format!("{}", trail.points.len()))
                        .color(value_color).size(11.0).monospace());
                    ui.end_row();

                    ui.label(egui::RichText::new("Duration").color(label_color).size(11.0));
                    let dur_text = oldest_point_age.or(track_duration)
                        .map(|secs| format!("{}:{:02}", secs / 60, secs % 60))
                        .unwrap_or_else(|| "---".to_string());
                    ui.label(egui::RichText::new(dur_text).color(value_color).size(11.0).monospace());
                    ui.end_row();

                    // Aircraft type info (from OpenSky database)
                    if let Some(ti) = type_info {
                        if let Some(ref reg) = ti.registration {
                            ui.label(egui::RichText::new("Registration").color(label_color).size(11.0));
                            ui.label(egui::RichText::new(reg).color(value_color).size(11.0).monospace());
                            ui.end_row();
                        }
                        if let Some(ref mm) = ti.manufacturer_model {
                            ui.label(egui::RichText::new("Aircraft").color(label_color).size(11.0));
                            ui.label(egui::RichText::new(mm).color(value_color).size(11.0).monospace());
                            ui.end_row();
                        }
                        if let Some(ref tc) = ti.type_code {
                            ui.label(egui::RichText::new("Type Code").color(label_color).size(11.0));
                            ui.label(egui::RichText::new(tc).color(value_color).size(11.0).monospace());
                            ui.end_row();
                        }
                        if let Some(ref op) = ti.operator {
                            ui.label(egui::RichText::new("Operator").color(label_color).size(11.0));
                            ui.label(egui::RichText::new(op).color(value_color).size(11.0).monospace());
                            ui.end_row();
                        }
                    }
                });

            ui.add_space(8.0);

            // Action buttons
            ui.horizontal(|ui| {
                let is_following = follow_state.following_icao.as_deref() == Some(selected_icao);
                let follow_text = if is_following { "Unfollow" } else { "Follow" };
                if ui.button(follow_text).clicked() {
                    if is_following {
                        follow_state.following_icao = None;
                    } else {
                        follow_state.following_icao = Some(selected_icao.to_string());
                    }
                }
            });
        });
}

/// System to toggle aircraft list visibility
pub fn toggle_aircraft_list(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut list_state: ResMut<AircraftListState>,
) {
    if keyboard.just_pressed(KeyCode::KeyL) {
        list_state.expanded = !list_state.expanded;
    }
}

/// System to highlight selected aircraft with a ring
pub fn highlight_selected_aircraft(
    mut gizmos: Gizmos,
    list_state: Res<AircraftListState>,
    tile_settings: Res<bevy_slippy_tiles::SlippyTilesSettings>,
    map_state: Res<MapState>,
    view3d_state: Res<crate::view3d::View3DState>,
    aircraft_query: Query<&crate::Aircraft>,
) {
    let Some(selected_icao) = &list_state.selected_icao else {
        return;
    };

    // Find the selected aircraft
    let Some(aircraft) = aircraft_query.iter().find(|a| &a.icao == selected_icao) else {
        return;
    };

    let zoom = view3d_state.effective_zoom(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, zoom);
    let pos = converter.latlon_to_world(aircraft.latitude, aircraft.longitude);

    // Draw selection ring (yellow)
    let color = Color::srgb(1.0, 1.0, 0.0);
    gizmos.circle_2d(pos, 15.0, color);
    gizmos.circle_2d(pos, 18.0, color);
}
