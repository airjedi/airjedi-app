mod measurement;

pub use measurement::*;

use bevy::prelude::*;

pub struct ToolsPlugin;

impl Plugin for ToolsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MeasurementState>().add_systems(
            Update,
            (
                toggle_measurement_mode,
                handle_measurement_clicks,
                update_measurement_line,
                render_measurement_tooltip,
            ),
        );
    }
}
