/// Left-side egui icon toolbar.
///
/// Replaces the Bevy native buttons (Clear Cache, Settings, Aircraft List)
/// with a narrow egui side panel containing icon toggle buttons for all features.
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy_slippy_tiles::{DownloadSlippyTilesMessage, MapTile, SlippyTileDownloadStatus};

use egui_phosphor::regular;

use crate::theme::{to_egui_color32, to_egui_color32_alpha, AppTheme};
use crate::ui_panels::{PanelId, UiPanelManager};
use crate::MapState;

/// Width of the toolbar in pixels.
const TOOLBAR_WIDTH: f32 = 44.0;

/// Render the left-side icon toolbar as a narrow egui SidePanel.
pub fn render_toolbar(
    mut contexts: EguiContexts,
    mut panels: ResMut<UiPanelManager>,
    map_state: Res<MapState>,
    mut download_events: MessageWriter<DownloadSlippyTilesMessage>,
    mut commands: Commands,
    tile_query: Query<Entity, With<MapTile>>,
    mut slippy_tile_download_status: ResMut<SlippyTileDownloadStatus>,
    theme: Res<AppTheme>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let panel_bg = to_egui_color32_alpha(theme.bg_secondary(), 245);

    let toolbar_frame = egui::Frame::NONE
        .fill(panel_bg)
        .inner_margin(egui::Margin::symmetric(4, 4));

    let panel_response = egui::SidePanel::left("toolbar")
        .exact_width(TOOLBAR_WIDTH)
        .resizable(false)
        .frame(toolbar_frame)
        .show_separator_line(false)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let active_color = to_egui_color32(theme.accent_primary());
                let inactive_color = to_egui_color32(theme.text_dim());
                let active_bg = to_egui_color32_alpha(theme.accent_primary(), 30);

                // -- Panel toggle buttons --
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Settings,
                    regular::GEAR,
                    "Settings",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::AircraftList,
                    regular::AIRPLANE_TILT,
                    "Aircraft List (L)",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Bookmarks,
                    regular::STAR,
                    "Bookmarks (B)",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Statistics,
                    regular::CHART_BAR,
                    "Statistics (S)",
                    active_color,
                    inactive_color,
                    active_bg,
                );

                ui.separator();

                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Measurement,
                    regular::RULER,
                    "Measurement (M)",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Export,
                    regular::DOWNLOAD_SIMPLE,
                    "Export (E)",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Coverage,
                    regular::TARGET,
                    "Coverage (V)",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Airspace,
                    regular::STACK,
                    "Airspace (Shift+A)",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::DataSources,
                    regular::DATABASE,
                    "Data Sources (Shift+D)",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Ingest,
                    regular::CLOUD_ARROW_DOWN,
                    "Ingest (I)",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Recording,
                    regular::RECORD,
                    "Recording (Ctrl+R)",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::View3D,
                    regular::CUBE,
                    "3D View (3)",
                    active_color,
                    inactive_color,
                    active_bg,
                );

                ui.separator();

                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Debug,
                    regular::HASH,
                    "Debug (`)",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Inspector,
                    regular::MAGNIFYING_GLASS,
                    "Inspector (F12)",
                    active_color,
                    inactive_color,
                    active_bg,
                );
                toolbar_button(
                    ui,
                    &mut panels,
                    PanelId::Help,
                    regular::QUESTION,
                    "Help (?)",
                    active_color,
                    inactive_color,
                    active_bg,
                );

                // -- Clear Cache button (action, not a panel toggle) --
                let icon_dim = to_egui_color32(theme.text_dim());
                let clear_btn = ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(regular::X)
                                .font(crate::theme::icon_font_id(16.0, ctx))
                                .color(icon_dim),
                        )
                        .min_size(egui::vec2(28.0, 22.0)),
                    )
                    .on_hover_text("Clear tile cache");

                if clear_btn.clicked() {
                    // Clear download status tracking
                    slippy_tile_download_status.0.clear();

                    // Despawn all tile entities
                    for entity in tile_query.iter() {
                        commands.entity(entity).despawn();
                    }

                    // Clear tile cache from disk
                    crate::clear_tile_cache();

                    // Request fresh tiles
                    crate::tiles::request_tiles_at_location(
                        &mut download_events,
                        map_state.latitude,
                        map_state.longitude,
                        map_state.zoom_level,
                        false,
                    );

                    info!("Tile cache cleared via toolbar");
                }
            });
        });

    // Paint over the egui panel separator gap to the right of the toolbar.
    // egui's SidePanel reserves ~3px for separator/interaction at the inner
    // edge. The frame fill stops at TOOLBAR_WIDTH but the panel rect extends
    // further, leaving a transparent strip. Fill it with the toolbar color.
    let panel_rect = panel_response.response.rect;
    let fill_start = panel_rect.right() - 3.0;
    let gap_rect = egui::Rect::from_min_max(
        egui::pos2(fill_start, panel_rect.top()),
        egui::pos2(panel_rect.right(), panel_rect.bottom()),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        "toolbar_gap".into(),
    ))
    .rect_filled(gap_rect, 0.0, panel_bg);
}

/// Render a toolbar toggle button that highlights when its panel is open.
fn toolbar_button(
    ui: &mut egui::Ui,
    panels: &mut UiPanelManager,
    panel_id: PanelId,
    icon: &str,
    tooltip: &str,
    active_color: egui::Color32,
    inactive_color: egui::Color32,
    active_bg: egui::Color32,
) {
    let is_open = panels.is_open(panel_id);
    let icon_color = if is_open {
        active_color
    } else {
        inactive_color
    };

    let btn = ui
        .add(
            egui::Button::new(
                egui::RichText::new(icon)
                    .font(crate::theme::icon_font_id(18.0, ui.ctx()))
                    .color(icon_color),
            )
            .fill(if is_open {
                active_bg
            } else {
                egui::Color32::TRANSPARENT
            })
            .min_size(egui::vec2(28.0, 22.0)),
        )
        .on_hover_text(tooltip);

    if btn.clicked() {
        panels.toggle_panel(panel_id);
    }
}
