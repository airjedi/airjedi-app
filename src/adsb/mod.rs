pub mod connection;
pub mod sync;

pub use connection::*;
pub use sync::*;

use bevy::prelude::*;

pub struct AdsbPlugin;

impl Plugin for AdsbPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            (
                setup_aircraft_models,
                setup_adsb_client.after(crate::setup_map),
            ),
        );

        app.add_systems(
            Update,
            (
                update_aircraft_label_text,
                apply_model_corrections,
                make_aircraft_unlit.after(apply_model_corrections),
            ),
        );

        app.add_systems(
            Update,
            (update_connection_status, reconnect_on_config_change),
        );
    }
}
