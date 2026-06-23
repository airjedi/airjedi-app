use bevy::prelude::*;

use super::{
    SessionClock, TrailConfig, TrailRecordTimer,
    AircraftListState, AircraftDisplayList,
    DetailPanelState, CameraFollowState,
    EmergencyAlertState, StatsPanelState,
    AircraftTypeDatabase,
    components::Aircraft,
};
use super::interpolation::{InterpolationState, interpolate_aircraft_positions};
use super::trail_renderer::{draw_trails, prune_trails};
use super::trails::record_trail_points;
use super::staleness::dim_stale_aircraft;
use super::list_panel::{toggle_aircraft_list, update_aircraft_display_list, highlight_selected_aircraft};
use super::detail_panel::{render_detail_panel, toggle_detail_panel, open_detail_on_selection, detect_aircraft_click};
use super::emergency::{detect_emergencies, draw_emergency_rings, update_emergency_banner, update_emergency_banner_text};
use super::typeloader::{start_aircraft_type_loading, poll_aircraft_type_loading, attach_aircraft_type_info};
use super::picking::{manage_selection_outline, deselect_on_escape, clear_stale_selection, follow_aircraft_3d, pick_aircraft_3d};
use super::mesh_trails::{spawn_mesh_trails, update_mesh_trails, cleanup_mesh_trails};

pub struct AircraftPlugin;

impl Plugin for AircraftPlugin {
    fn build(&self, app: &mut App) {
        app
            .register_type::<Aircraft>()
            .register_type::<InterpolationState>()
            .register_type::<CameraFollowState>()
            .init_resource::<SessionClock>()
            .init_resource::<TrailConfig>()
            .init_resource::<TrailRecordTimer>()
            .init_resource::<AircraftListState>()
            .init_resource::<AircraftDisplayList>()
            .init_resource::<DetailPanelState>()
            .init_resource::<CameraFollowState>()
            .init_resource::<EmergencyAlertState>()
            .init_resource::<StatsPanelState>()
            .init_resource::<AircraftTypeDatabase>()
            .add_systems(Startup, start_aircraft_type_loading)
            .add_systems(Update, (
                record_trail_points,
                draw_trails.after(crate::ZoomSet::Change),
                prune_trails,
                toggle_aircraft_list,
                update_aircraft_display_list,
                highlight_selected_aircraft,
                toggle_detail_panel,
                open_detail_on_selection,
                detect_aircraft_click,
                detect_emergencies,
                draw_emergency_rings.after(crate::ZoomSet::Change),
                update_emergency_banner,
                update_emergency_banner_text,
                dim_stale_aircraft,
            ))
            .add_systems(Update, render_detail_panel)
            .add_systems(Update, interpolate_aircraft_positions
                .after(crate::adsb::sync_aircraft_from_adsb))
            .add_systems(Update, (poll_aircraft_type_loading, attach_aircraft_type_info))
            .add_systems(Update, (
                manage_selection_outline,
                deselect_on_escape,
                clear_stale_selection,
                follow_aircraft_3d,
                pick_aircraft_3d,
            ));

        app.add_systems(Update, (
            spawn_mesh_trails,
            update_mesh_trails.after(crate::ZoomSet::Change),
            cleanup_mesh_trails,
        ));
    }
}
