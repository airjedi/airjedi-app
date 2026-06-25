use std::collections::HashMap;
use std::ops::DerefMut;

use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContext, PrimaryEguiContext};
use egui_tiles::{Behavior, SimplificationOptions, TabState, TileId, Tiles, UiResponse};

use crate::aircraft::{
    list_panel::render_aircraft_list_pane_content, stats_panel::render_stats_pane_content,
    AircraftDisplayList, AircraftListState, AircraftTypeInfo, CameraFollowState, DetailPanelState,
    SessionClock, StatsPanelState, TrailHistory,
};
use crate::airspace::{AirspaceData, AirspaceDisplayState};
use crate::bookmarks::{self, BookmarksPanelState};
use crate::config::{self, AppConfig, SettingsUiState};
use crate::coverage::CoverageState;
use crate::data_sources::DataSourceManager;
use crate::debug_panel::{self, DebugPanelState};
use crate::export::ExportState;
use crate::inspector;
use crate::recording::{PlaybackState, RecordingState};
use crate::theme::{to_egui_color32, to_egui_color32_alpha, AppTheme, ThemeRegistry};
use crate::tools_window;
use crate::ui_panels::{PanelId, UiPanelManager};
use crate::view3d::sky::{SunState, TimeState};
use crate::view3d::View3DState;
use crate::{Aircraft, MapState, ZoomState};

// =============================================================================
// DockPane - identifies each dockable panel
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DockPane {
    MapViewport,
    Debug,
    Coverage,
    Airspace,
    DataSources,
    Export,
    Recording,
    View3D,
    Ingest,
    Settings,
    AircraftList,
    AircraftDetail,
    Bookmarks,
    Stats,
    Inspector,
}

impl DockPane {
    pub fn display_name(&self) -> &'static str {
        match self {
            DockPane::MapViewport => "Map",
            DockPane::Debug => "Debug",
            DockPane::Inspector => "Inspector",
            DockPane::Coverage => "Coverage",
            DockPane::Airspace => "Airspace",
            DockPane::DataSources => "Data Sources",
            DockPane::Export => "Export",
            DockPane::Recording => "Recording",
            DockPane::View3D => "3D View",
            DockPane::Ingest => "Ingest",
            DockPane::Settings => "Settings",
            DockPane::AircraftList => "Aircraft",
            DockPane::AircraftDetail => "Detail",
            DockPane::Bookmarks => "Bookmarks",
            DockPane::Stats => "Statistics",
        }
    }
}

// =============================================================================
// DockTreeState - persistent Bevy resource holding the tile tree
// =============================================================================

#[derive(Resource)]
pub struct DockTreeState {
    pub tree: egui_tiles::Tree<DockPane>,
    pub pane_tile_ids: HashMap<DockPane, TileId>,
    /// Container tile IDs for auto-collapse when all children are hidden.
    pub bottom_tabs_id: TileId,
    pub right_tabs_id: TileId,
    /// When true, the dock layout resets to defaults on the next frame.
    pub reset_requested: bool,
    /// Captured each frame from MapViewport pane for camera viewport adjustment
    pub map_viewport_rect: Option<egui::Rect>,
}

/// Panes grouped in the bottom tab container.
const BOTTOM_PANES: &[DockPane] = &[
    DockPane::Coverage,
    DockPane::DataSources,
    DockPane::Export,
    DockPane::Recording,
];

/// Panes grouped in the right tab container.
const RIGHT_PANES: &[DockPane] = &[
    DockPane::AircraftList,
    DockPane::AircraftDetail,
    DockPane::Airspace,
    DockPane::Bookmarks,
    DockPane::Stats,
    DockPane::Settings,
    DockPane::Ingest,
    DockPane::View3D,
    DockPane::Debug,
    DockPane::Inspector,
];

impl Default for DockTreeState {
    fn default() -> Self {
        let mut tiles = Tiles::default();
        let mut pane_tile_ids = HashMap::new();

        // Insert all panes and record their TileIds
        let all_panes = [
            DockPane::MapViewport,
            DockPane::Debug,
            DockPane::Coverage,
            DockPane::Airspace,
            DockPane::DataSources,
            DockPane::Export,
            DockPane::Recording,
            DockPane::View3D,
            DockPane::Ingest,
            DockPane::Settings,
            DockPane::AircraftList,
            DockPane::AircraftDetail,
            DockPane::Bookmarks,
            DockPane::Stats,
            DockPane::Inspector,
        ];

        for pane in all_panes {
            let id = tiles.insert_pane(pane);
            pane_tile_ids.insert(pane, id);
        }

        // Bottom tabs: Coverage, DataSources, Export, Recording
        let bottom_tabs_id = tiles.insert_tab_tile(vec![
            pane_tile_ids[&DockPane::Coverage],
            pane_tile_ids[&DockPane::DataSources],
            pane_tile_ids[&DockPane::Export],
            pane_tile_ids[&DockPane::Recording],
        ]);

        // Right tabs: AircraftList, AircraftDetail, Airspace, Bookmarks, Stats, Settings, Ingest, View3D, Debug, Inspector
        let right_tabs_id = tiles.insert_tab_tile(vec![
            pane_tile_ids[&DockPane::AircraftList],
            pane_tile_ids[&DockPane::AircraftDetail],
            pane_tile_ids[&DockPane::Airspace],
            pane_tile_ids[&DockPane::Bookmarks],
            pane_tile_ids[&DockPane::Stats],
            pane_tile_ids[&DockPane::Settings],
            pane_tile_ids[&DockPane::Ingest],
            pane_tile_ids[&DockPane::View3D],
            pane_tile_ids[&DockPane::Debug],
            pane_tile_ids[&DockPane::Inspector],
        ]);

        // Top area: horizontal split - MapViewport (left, ~75%) + right tabs (right, ~25%)
        let map_id = pane_tile_ids[&DockPane::MapViewport];
        let top_area_id = tiles.insert_horizontal_tile(vec![map_id, right_tabs_id]);
        if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(linear))) =
            tiles.get_mut(top_area_id)
        {
            linear.shares.set_share(map_id, 3.0);
            linear.shares.set_share(right_tabs_id, 1.0);
        }

        // Root: vertical split - top area (~75%) + bottom tabs (~25%)
        let root_id = tiles.insert_vertical_tile(vec![top_area_id, bottom_tabs_id]);
        if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(linear))) =
            tiles.get_mut(root_id)
        {
            linear.shares.set_share(top_area_id, 3.0);
            linear.shares.set_share(bottom_tabs_id, 1.0);
        }

        // Set most panes to not visible initially (only MapViewport and AircraftList visible)
        let hidden_panes = [
            DockPane::Debug,
            DockPane::Coverage,
            DockPane::Airspace,
            DockPane::DataSources,
            DockPane::Export,
            DockPane::Recording,
            DockPane::View3D,
            DockPane::Ingest,
            DockPane::AircraftDetail,
            DockPane::Bookmarks,
            DockPane::Stats,
            DockPane::Settings,
            DockPane::Inspector,
        ];
        for pane in hidden_panes {
            tiles.set_visible(pane_tile_ids[&pane], false);
        }

        let tree = egui_tiles::Tree::new("dock_tree", root_id, tiles);

        Self {
            tree,
            pane_tile_ids,
            bottom_tabs_id,
            right_tabs_id,
            reset_requested: false,
            map_viewport_rect: None,
        }
    }
}

// =============================================================================
// CachedThemeColors - pre-cached egui colors to avoid borrowing world from &self
// =============================================================================

#[derive(Clone, Copy)]
struct CachedThemeColors {
    bg_primary: egui::Color32,
    bg_secondary: egui::Color32,
    bg_secondary_alpha: egui::Color32,
    text_primary: egui::Color32,
    text_dim: egui::Color32,
}

impl CachedThemeColors {
    fn from_theme(theme: &AppTheme) -> Self {
        Self {
            bg_primary: to_egui_color32(theme.bg_primary()),
            bg_secondary: to_egui_color32(theme.bg_secondary()),
            bg_secondary_alpha: to_egui_color32_alpha(theme.bg_secondary(), 180),
            text_primary: to_egui_color32(theme.text_primary()),
            text_dim: to_egui_color32(theme.text_dim()),
        }
    }
}

// =============================================================================
// DockBehavior - holds &mut World for exclusive system access
// =============================================================================

struct DockBehavior<'a> {
    world: &'a mut World,
    map_viewport_rect: &'a mut Option<egui::Rect>,
    /// Panes closed via dock tab X button, processed after tree rendering.
    closed_panes: Vec<DockPane>,
    colors: CachedThemeColors,
}

/// Paint opaque background with subtle gradient and wrap content in a vertical scroll area.
fn render_pane_with_bg(bg: egui::Color32, ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    let top_color = crate::widgets::lerp_color(bg, egui::Color32::WHITE, 0.04);
    crate::widgets::paint_gradient_rect(
        ui.painter(),
        ui.max_rect(),
        top_color,
        bg,
        crate::widgets::GradientDirection::Vertical,
    );
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let margin = egui::Margin::symmetric(6, 4);
            egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                content(ui);
            });
        });
}

/// Build a convex polygon with rounded top corners and flat bottom edge.
fn build_rounded_top_tab(rect: egui::Rect, corner_radius: f32) -> Vec<egui::Pos2> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let r = corner_radius;
    let min = rect.min;
    let max = rect.max;
    const STEPS: usize = 8;
    let mut pts = Vec::with_capacity(STEPS * 2 + 4);

    // Top-left arc: center=(min.x+r, min.y+r), angles π → 3π/2
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let a = PI + t * FRAC_PI_2;
        pts.push(egui::pos2(min.x + r + r * a.cos(), min.y + r + r * a.sin()));
    }

    // Top-right arc: center=(max.x-r, min.y+r), angles 3π/2 → 2π
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let a = 3.0 * FRAC_PI_2 + t * FRAC_PI_2;
        pts.push(egui::pos2(max.x - r + r * a.cos(), min.y + r + r * a.sin()));
    }

    // Bottom-right and bottom-left corners (square)
    pts.push(egui::pos2(max.x, max.y));
    pts.push(egui::pos2(min.x, max.y));

    pts
}

// Tab geometry constants
const TAB_CORNER_RADIUS: f32 = 6.0;
const TAB_H_PAD: f32 = 10.0;
const TAB_GAP: f32 = 3.0;

impl<'a> Behavior<DockPane> for DockBehavior<'a> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, pane: &mut DockPane) -> UiResponse {
        let bg = self.colors.bg_primary;

        match pane {
            DockPane::MapViewport => {
                *self.map_viewport_rect = Some(ui.max_rect());
            }

            DockPane::Debug => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state = SystemState::<(
                        ResMut<DebugPanelState>,
                        Option<Res<MapState>>,
                        Option<Res<ZoomState>>,
                    )>::new(world);
                    let (mut debug, map, zoom) = state.get_mut(world);
                    debug_panel::render_debug_pane_content(
                        ui,
                        &mut debug,
                        map.as_deref(),
                        zoom.as_deref(),
                    );
                });
            }

            DockPane::Settings => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state = SystemState::<(
                        ResMut<SettingsUiState>,
                        ResMut<AppConfig>,
                        ResMut<AppTheme>,
                        Res<ThemeRegistry>,
                    )>::new(world);
                    let (mut settings_ui, mut app_config, mut theme, theme_registry) =
                        state.get_mut(world);
                    config::render_settings_pane_content(
                        ui,
                        &mut settings_ui,
                        &mut app_config,
                        &mut theme,
                        &theme_registry,
                    );
                });
            }

            DockPane::AircraftList => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state = SystemState::<(
                        ResMut<AircraftListState>,
                        ResMut<DetailPanelState>,
                        ResMut<CameraFollowState>,
                        Res<AircraftDisplayList>,
                        Res<AppConfig>,
                        Res<SessionClock>,
                        Query<(
                            &'static Aircraft,
                            &'static TrailHistory,
                            Option<&'static AircraftTypeInfo>,
                        )>,
                        Res<AppTheme>,
                    )>::new(world);
                    let (
                        mut list,
                        mut detail,
                        mut follow,
                        display,
                        app_config,
                        clock,
                        query,
                        theme,
                    ) = state.get_mut(world);
                    render_aircraft_list_pane_content(
                        ui,
                        &mut list,
                        &mut detail,
                        &mut follow,
                        &display,
                        &app_config,
                        &clock,
                        &query,
                        &theme,
                    );
                });
            }

            DockPane::AircraftDetail => {
                render_pane_with_bg(bg, ui, |ui| {
                    ui.label("Select an aircraft from the Aircraft tab to view details.");
                });
            }

            DockPane::Stats => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state = SystemState::<(
                        Res<StatsPanelState>,
                        Query<&'static Aircraft>,
                        Res<AppTheme>,
                    )>::new(world);
                    let (stats, query, theme) = state.get_mut(world);
                    render_stats_pane_content(ui, &stats, &query, &theme);
                });
            }

            DockPane::Bookmarks => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state = SystemState::<(
                        ResMut<BookmarksPanelState>,
                        ResMut<AppConfig>,
                        ResMut<MapState>,
                        ResMut<ZoomState>,
                        Res<AircraftListState>,
                        Query<&'static Aircraft>,
                        Res<AppTheme>,
                    )>::new(world);
                    let (mut bookmarks, mut config, mut map, mut zoom, list, query, theme) =
                        state.get_mut(world);
                    bookmarks::render_bookmarks_pane_content(
                        ui,
                        &mut bookmarks,
                        &mut config,
                        &mut map,
                        &mut zoom,
                        &list,
                        &query,
                        &theme,
                    );
                });
            }

            DockPane::Coverage => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state = SystemState::<ResMut<CoverageState>>::new(world);
                    let mut coverage = state.get_mut(world);
                    tools_window::render_coverage_tab(ui, &mut coverage);
                });
            }

            DockPane::Airspace => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state = SystemState::<(
                        ResMut<AirspaceDisplayState>,
                        ResMut<AirspaceData>,
                    )>::new(world);
                    let (mut display, mut data) = state.get_mut(world);
                    tools_window::render_airspace_tab(ui, &mut display, &mut data);
                });
            }

            DockPane::DataSources => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state = SystemState::<ResMut<DataSourceManager>>::new(world);
                    let mut mgr = state.get_mut(world);
                    tools_window::render_data_sources_tab(ui, &mut mgr);
                });
            }

            DockPane::Export => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state = SystemState::<ResMut<ExportState>>::new(world);
                    let mut export = state.get_mut(world);
                    tools_window::render_export_tab(ui, &mut export);
                });
            }

            DockPane::Recording => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state =
                        SystemState::<(ResMut<RecordingState>, ResMut<PlaybackState>)>::new(world);
                    let (mut recording, mut playback) = state.get_mut(world);
                    tools_window::render_recording_tab(ui, &mut recording, &mut playback);
                });
            }

            DockPane::View3D => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state = SystemState::<(
                        ResMut<View3DState>,
                        ResMut<crate::terrain::TerrainState>,
                        ResMut<TimeState>,
                        Res<SunState>,
                        Option<ResMut<crate::tiles::GridOverlay>>,
                    )>::new(world);
                    let (mut view3d, mut terrain, mut time, sun, mut grid) = state.get_mut(world);
                    tools_window::render_view3d_tab(
                        ui,
                        &mut view3d,
                        &mut terrain,
                        &mut time,
                        &sun,
                        grid.as_deref_mut(),
                    );
                });
            }

            DockPane::Ingest => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    let mut state = SystemState::<(
                        Option<Res<crate::data_ingest::IngestStatus>>,
                        ResMut<AppConfig>,
                        Option<ResMut<crate::data_ingest::IngestUiState>>,
                    )>::new(world);
                    let (ingest_status, mut app_config, mut ingest_ui) = state.get_mut(world);
                    tools_window::render_ingest_tab(
                        ui,
                        ingest_status.as_deref(),
                        &mut app_config,
                        ingest_ui.as_deref_mut(),
                    );
                });
            }

            DockPane::Inspector => {
                let world = &mut *self.world;
                render_pane_with_bg(bg, ui, |ui| {
                    inspector::render_inspector_pane_content(world, ui);
                });
            }
        }
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &DockPane) -> egui::WidgetText {
        pane.display_name().into()
    }

    fn is_tab_closable(&self, _tiles: &Tiles<DockPane>, _tile_id: TileId) -> bool {
        true
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<DockPane>, tile_id: TileId) -> bool {
        tiles.set_visible(tile_id, false);
        if let Some(egui_tiles::Tile::Pane(pane)) = tiles.get(tile_id) {
            self.closed_panes.push(*pane);
        }
        false // prevent removal, just hide
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions {
            prune_empty_tabs: false,
            prune_empty_containers: false,
            prune_single_child_tabs: false,
            prune_single_child_containers: false,
            all_panes_must_have_tabs: true,
            join_nested_linear_containers: false,
        }
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        1.0
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> egui::Color32 {
        self.colors.bg_secondary
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &Tiles<DockPane>,
        _tile_id: TileId,
        state: &TabState,
    ) -> egui::Color32 {
        if state.active {
            self.colors.bg_primary
        } else {
            self.colors.bg_secondary_alpha
        }
    }

    fn tab_text_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &Tiles<DockPane>,
        _tile_id: TileId,
        state: &TabState,
    ) -> egui::Color32 {
        if state.active {
            self.colors.text_primary
        } else {
            self.colors.text_dim
        }
    }

    fn tab_bar_hline_stroke(&self, _visuals: &egui::Visuals) -> egui::Stroke {
        egui::Stroke::NONE
    }

    fn tab_ui(
        &mut self,
        tiles: &mut egui_tiles::Tiles<DockPane>,
        ui: &mut egui::Ui,
        id: egui::Id,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        // Get the pane to retrieve its title
        let pane = match tiles.get(tile_id) {
            Some(egui_tiles::Tile::Pane(pane)) => *pane,
            _ => return ui.allocate_response(egui::vec2(0.0, 0.0), egui::Sense::hover()),
        };

        let title = self.tab_title_for_pane(&pane);
        let is_closable = self.is_tab_closable(tiles, tile_id);
        let text_color = if state.active {
            self.colors.text_primary
        } else {
            self.colors.text_dim
        };
        let close_w = if is_closable { 18.0 } else { 0.0 };

        // Measure title text
        let title_str = title.text();
        let font_id = egui::FontId::proportional(13.0);
        // Simple text width estimate: ~7 pixels per character for proportional font at size 13
        let text_w = title_str.len() as f32 * 7.0;
        let tab_h = ui.available_height();
        let tab_w = text_w + 2.0 * TAB_H_PAD + close_w + TAB_GAP;

        let (tab_rect, mut response) =
            ui.allocate_exact_size(egui::vec2(tab_w, tab_h), egui::Sense::click_and_drag());

        // Inset the visual tab rect by the gap so there's space between tabs
        let visual_rect = egui::Rect::from_min_max(
            tab_rect.min,
            egui::pos2(tab_rect.max.x - TAB_GAP, tab_rect.max.y),
        );

        if ui.is_rect_visible(tab_rect) {
            let painter = ui.painter();

            if state.active {
                let pts = build_rounded_top_tab(visual_rect, TAB_CORNER_RADIUS);
                painter.add(egui::Shape::convex_polygon(
                    pts,
                    self.colors.bg_primary,
                    egui::Stroke::NONE,
                ));
            } else {
                // Inactive: rounded top corners with transparency
                let hovered = response.hovered();
                let fill = if hovered {
                    egui::Color32::from_rgba_unmultiplied(
                        self.colors.bg_primary.r(),
                        self.colors.bg_primary.g(),
                        self.colors.bg_primary.b(),
                        80,
                    )
                } else {
                    egui::Color32::from_rgba_unmultiplied(
                        self.colors.bg_secondary.r(),
                        self.colors.bg_secondary.g(),
                        self.colors.bg_secondary.b(),
                        160,
                    )
                };
                let pts = build_rounded_top_tab(visual_rect, TAB_CORNER_RADIUS);
                painter.add(egui::Shape::convex_polygon(pts, fill, egui::Stroke::NONE));
            }

            // Paint title text (vertically centered, left-padded)
            let text_pos = egui::pos2(visual_rect.left() + TAB_H_PAD, visual_rect.center().y);
            painter.text(
                text_pos,
                egui::Align2::LEFT_CENTER,
                title_str,
                font_id,
                text_color,
            );

            // Close button (only when closable)
            if is_closable {
                let close_x = visual_rect.right() - 2.0 - 12.0;
                let close_center = egui::pos2(close_x, visual_rect.center().y);
                let close_rect = egui::Rect::from_center_size(close_center, egui::vec2(14.0, 14.0));

                let close_resp = ui.interact(close_rect, id.with("close"), egui::Sense::click());
                let close_color = if close_resp.hovered() {
                    egui::Color32::from_rgb(220, 80, 60)
                } else {
                    self.colors.text_dim
                };
                painter.text(
                    close_center,
                    egui::Align2::CENTER_CENTER,
                    "\u{00d7}",
                    egui::FontId::proportional(13.0),
                    close_color,
                );
                if close_resp.clicked() {
                    self.on_tab_close(tiles, tile_id);
                }
            }
        }

        self.on_tab_button(tiles, tile_id, response)
    }
}

// =============================================================================
// Mapping from UiPanelManager / ToolsWindowState to DockPane visibility
// =============================================================================

const PANEL_DOCK_MAP: &[(PanelId, DockPane)] = &[
    (PanelId::Debug, DockPane::Debug),
    (PanelId::Inspector, DockPane::Inspector),
    (PanelId::Settings, DockPane::Settings),
    (PanelId::AircraftList, DockPane::AircraftList),
    (PanelId::AircraftDetail, DockPane::AircraftDetail),
    (PanelId::Bookmarks, DockPane::Bookmarks),
    (PanelId::Statistics, DockPane::Stats),
    (PanelId::Coverage, DockPane::Coverage),
    (PanelId::Airspace, DockPane::Airspace),
    (PanelId::DataSources, DockPane::DataSources),
    (PanelId::Export, DockPane::Export),
    (PanelId::Recording, DockPane::Recording),
    (PanelId::View3D, DockPane::View3D),
    (PanelId::Ingest, DockPane::Ingest),
];

// =============================================================================
// render_dock_tree - exclusive Bevy system for full World access
// =============================================================================

pub fn render_dock_tree(world: &mut World) {
    // 1. Clone egui context to release the world borrow
    let mut egui_context = {
        let mut q = world.query_filtered::<&mut EguiContext, With<PrimaryEguiContext>>();
        let Ok(mut ctx) = q.single_mut(world) else {
            return;
        };
        ctx.deref_mut().clone()
    };

    // 2. Pre-cache theme colors (avoids borrowing world from &self trait methods)
    let colors = {
        let theme = world.resource::<AppTheme>();
        CachedThemeColors::from_theme(theme)
    };

    // 3. Use resource_scope for DockTreeState so DockBehavior can hold &mut World
    world.resource_scope(|world, mut dock_state: Mut<DockTreeState>| {
        // 3a. Handle layout reset request (from Settings panel button)
        {
            let mut settings_ui = world.resource_mut::<SettingsUiState>();
            if settings_ui.layout_reset_requested {
                settings_ui.layout_reset_requested = false;
                dock_state.reset_requested = true;
            }
        }
        if dock_state.reset_requested {
            *dock_state = DockTreeState::default();
            info!("Dock layout reset to defaults");
        }

        // 4. Sync UiPanelManager state to dock tile visibility
        {
            let panels = world.resource::<UiPanelManager>();
            for &(panel_id, dock_pane) in PANEL_DOCK_MAP {
                if let Some(&tile_id) = dock_state.pane_tile_ids.get(&dock_pane) {
                    let should_be_visible = panels.is_open(panel_id);
                    dock_state
                        .tree
                        .tiles
                        .set_visible(tile_id, should_be_visible);
                }
            }
        }

        // 5. Auto-collapse containers when all their children are hidden
        let bottom_id = dock_state.bottom_tabs_id;
        let right_id = dock_state.right_tabs_id;
        let bottom_has_visible = BOTTOM_PANES.iter().any(|p| {
            dock_state
                .pane_tile_ids
                .get(p)
                .is_some_and(|&id| dock_state.tree.tiles.is_visible(id))
        });
        let right_has_visible = RIGHT_PANES.iter().any(|p| {
            dock_state
                .pane_tile_ids
                .get(p)
                .is_some_and(|&id| dock_state.tree.tiles.is_visible(id))
        });
        dock_state
            .tree
            .tiles
            .set_visible(bottom_id, bottom_has_visible);
        dock_state
            .tree
            .tiles
            .set_visible(right_id, right_has_visible);

        // 6. Build DockBehavior and render the dock tree
        let mut map_viewport_rect: Option<egui::Rect> = None;
        let closed_panes;

        {
            let mut behavior = DockBehavior {
                world,
                map_viewport_rect: &mut map_viewport_rect,
                closed_panes: Vec::new(),
                colors,
            };

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(egui::Color32::TRANSPARENT))
                .show(egui_context.get_mut(), |ui| {
                    dock_state.tree.ui(&mut behavior, ui);
                });

            closed_panes = behavior.closed_panes;
        }

        // 7. Route dock tab close events back to UiPanelManager
        if !closed_panes.is_empty() {
            let mut panels = world.resource_mut::<UiPanelManager>();
            for closed_pane in &closed_panes {
                if let Some(&(panel_id, _)) =
                    PANEL_DOCK_MAP.iter().find(|(_, dp)| dp == closed_pane)
                {
                    panels.close_panel(panel_id);
                }
            }
        }

        dock_state.map_viewport_rect = map_viewport_rect;
    });
}
