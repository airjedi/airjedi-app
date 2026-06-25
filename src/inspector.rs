/// ECS Inspector pane rendered inside the dock.
///
/// Uses `bevy_inspector_egui` to display editable views of resources,
/// entities, and assets.  Requires `&mut World` access, which is why
/// the dock system runs as an exclusive system.
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_inspector_egui::bevy_inspector;

use crate::debug_panel::DebugPanelState;
use crate::map::{MapState, ZoomState};
use crate::view3d::View3DState;

/// Resource controlling inspector visibility (kept for compatibility).
#[derive(Resource, Default)]
pub struct InspectorState {
    pub open: bool,
}

/// Render inspector content into a bare `egui::Ui` (for dock/tab usage).
///
/// This requires `&mut World` because `bevy_inspector_egui` functions
/// inspect and mutate ECS data directly.
pub fn render_inspector_pane_content(world: &mut World, ui: &mut egui::Ui) {
    // Section 1: Curated app resources (open by default)
    egui::CollapsingHeader::new("App Resources")
        .default_open(true)
        .show(ui, |ui| {
            ui.label("MapState");
            ui.push_id("map_state", |ui| {
                bevy_inspector::ui_for_resource::<MapState>(world, ui);
            });
            ui.separator();

            ui.label("ZoomState");
            ui.push_id("zoom_state", |ui| {
                bevy_inspector::ui_for_resource::<ZoomState>(world, ui);
            });
            ui.separator();

            ui.label("View3DState");
            ui.push_id("view3d_state", |ui| {
                bevy_inspector::ui_for_resource::<View3DState>(world, ui);
            });
            ui.separator();

            ui.label("DebugPanelState");
            ui.push_id("debug_state", |ui| {
                bevy_inspector::ui_for_resource::<DebugPanelState>(world, ui);
            });
        });

    ui.separator();

    // Section 2: All entities
    egui::CollapsingHeader::new("Entities")
        .default_open(false)
        .show(ui, |ui| {
            bevy_inspector::ui_for_entities(world, ui);
        });

    ui.separator();

    // Section 3: All resources
    egui::CollapsingHeader::new("Resources")
        .default_open(false)
        .show(ui, |ui| {
            bevy_inspector::ui_for_resources(world, ui);
        });

    ui.separator();

    // Section 4: All assets
    egui::CollapsingHeader::new("Assets")
        .default_open(false)
        .show(ui, |ui| {
            bevy_inspector::ui_for_all_assets(world, ui);
        });
}
