//! Hover popup for aircraft on the map: a small card, styled like the
//! aircraft list panel's rows, shown beside (not on top of) the icon while
//! the cursor is near it.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use super::list_panel::{full_position_source_label, get_altitude_color};
use super::picking::HoverOutline;
use super::staleness::{aircraft_age_secs, format_last_seen};
use super::typeinfo::AircraftTypeInfo;
use super::Aircraft;
use crate::aircraft::altitude::format_altitude_with_indicator;
use crate::aircraft::components::FusionDiagnostics;
use crate::theme::AppTheme;
use crate::widgets::{Card, WidgetTheme};
use crate::AircraftCamera;

/// Screen-space offset from the aircraft icon to the popup's anchor corner,
/// chosen to clear both the icon itself and its ~30px hover pick radius
/// (`PICK_RADIUS_PX` in `picking.rs`) so the popup never sits on top of it.
const POPUP_OFFSET_X: f32 = 28.0;
const POPUP_OFFSET_Y: f32 = -70.0;

pub fn render_aircraft_hover_popup(
    mut contexts: EguiContexts,
    theme: Res<AppTheme>,
    camera_query: Query<(&Camera, &GlobalTransform), With<AircraftCamera>>,
    hovered_query: Query<
        (
            &Aircraft,
            &GlobalTransform,
            Option<&AircraftTypeInfo>,
            Option<&FusionDiagnostics>,
        ),
        With<HoverOutline>,
    >,
) {
    let Ok((aircraft, aircraft_gtf, type_info, diag)) = hovered_query.single() else {
        return;
    };
    let Ok((camera, camera_gtf)) = camera_query.single() else {
        return;
    };
    let Ok(screen_pos) = camera.world_to_viewport(camera_gtf, aircraft_gtf.translation()) else {
        return;
    };
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let wt = WidgetTheme::from(&*theme);

    let registration = type_info.and_then(|i| i.registration.clone());
    let is_military = type_info.map(|i| i.is_military).unwrap_or(false);
    let id_label = registration
        .as_deref()
        .or(aircraft.callsign.as_deref().map(str::trim))
        .filter(|s| !s.is_empty())
        .unwrap_or(&aircraft.icao);
    let has_better_id = registration.is_some() || aircraft.callsign.is_some();

    let type_label = type_info.and_then(|i| {
        i.manufacturer_model
            .clone()
            .or_else(|| i.type_code.clone())
    });
    let operator = type_info.and_then(|i| i.operator.clone());
    let (source_label, source_color) =
        full_position_source_label(diag.and_then(|d| d.last_position_source));

    let (alt_color, alt_indicator) = get_altitude_color(aircraft.altitude);

    let anchor = egui::pos2(screen_pos.x + POPUP_OFFSET_X, screen_pos.y + POPUP_OFFSET_Y);

    // ~40% opacity (102/255) — bumped up 20 points from the initial 20%.
    const POPUP_ALPHA: u8 = 102;

    egui::Area::new(egui::Id::new("aircraft_hover_popup"))
        .fixed_pos(anchor)
        .order(egui::Order::Middle)
        .interactable(false)
        .show(ctx, |ui| {
            // Constrain width *before* building the card so its header row
            // (which sizes itself off `ui.available_width()`) shrinks to fit
            // instead of spanning the Area's full default width.
            ui.set_max_width(170.0);

            let bg = wt.bg_primary;
            let mut card = Card::new(&wt)
                .header(id_label)
                .fill(egui::Color32::from_rgba_unmultiplied(
                    bg.r(),
                    bg.g(),
                    bg.b(),
                    POPUP_ALPHA,
                ))
                .body_margin(6.0);
            if aircraft.altitude.is_some() {
                card = card.gradient_header(
                    egui::Color32::from_rgba_unmultiplied(
                        alt_color.r(),
                        alt_color.g(),
                        alt_color.b(),
                        POPUP_ALPHA,
                    ),
                    egui::Color32::TRANSPARENT,
                );
            }

            card.show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;

                ui.horizontal(|ui| {
                    if is_military {
                        ui.label(
                            egui::RichText::new("MIL")
                                .color(egui::Color32::from_rgb(220, 180, 60))
                                .size(10.0)
                                .strong(),
                        );
                    }
                    if aircraft.is_on_ground == Some(true) {
                        ui.label(
                            egui::RichText::new("GND")
                                .color(egui::Color32::from_rgb(180, 140, 80))
                                .size(10.0)
                                .strong(),
                        );
                    }
                    if has_better_id {
                        ui.label(
                            egui::RichText::new(&aircraft.icao)
                                .color(wt.text_dim)
                                .size(10.0)
                                .monospace(),
                        );
                    }
                    if let Some(alt) = aircraft.altitude {
                        ui.label(
                            egui::RichText::new(format_altitude_with_indicator(
                                alt,
                                alt_indicator,
                            ))
                            .color(alt_color)
                            .size(12.0)
                            .monospace(),
                        );
                    }
                });

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if let Some(vel) = aircraft.velocity {
                        ui.label(
                            egui::RichText::new(format!("{:03}kt", vel as i32))
                                .color(wt.text_dim)
                                .size(11.0)
                                .monospace(),
                        );
                    }
                    if let Some(heading) = aircraft.heading {
                        ui.label(
                            egui::RichText::new(format!("{:03}\u{00B0}", heading as i32))
                                .color(wt.text_dim)
                                .size(11.0)
                                .monospace(),
                        );
                    }
                    ui.label(
                        egui::RichText::new(source_label)
                            .color(source_color)
                            .size(10.0)
                            .monospace(),
                    );
                    ui.label(
                        egui::RichText::new(format_last_seen(aircraft_age_secs(aircraft)))
                            .color(wt.text_dim)
                            .size(10.0)
                            .monospace(),
                    );
                });

                if let Some(type_label) = &type_label {
                    ui.label(
                        egui::RichText::new(type_label)
                            .color(wt.text)
                            .size(10.0),
                    );
                }
                if let Some(operator) = &operator {
                    ui.label(
                        egui::RichText::new(operator)
                            .color(wt.text_dim)
                            .size(10.0),
                    );
                }
            });
        });
}
