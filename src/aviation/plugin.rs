use bevy::prelude::*;

use super::{
    draw_navaids, draw_runways, poll_aviation_data_loading, spawn_airports,
    start_aviation_data_loading, update_airport_positions, update_airport_visibility,
    AirportRenderState, AviationData, NavaidRenderState, RunwayRenderState,
};
use crate::ZoomSet;

pub struct AviationPlugin;

impl Plugin for AviationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AviationData>()
            .init_resource::<AirportRenderState>()
            .init_resource::<RunwayRenderState>()
            .init_resource::<NavaidRenderState>()
            .add_systems(Startup, start_aviation_data_loading)
            .add_systems(
                Update,
                (
                    poll_aviation_data_loading,
                    spawn_airports,
                    update_airport_positions.after(ZoomSet::Change),
                    update_airport_visibility,
                    draw_runways.after(ZoomSet::Change),
                    draw_navaids.after(ZoomSet::Change),
                ),
            );
    }
}
