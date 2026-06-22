pub mod sync;
pub mod connection;

pub use sync::*;
pub use connection::*;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;

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

        #[cfg(not(feature = "fusion"))]
        app.add_systems(
            Update,
            (
                sync_aircraft_from_adsb,
                update_aircraft_label_text.after(sync_aircraft_from_adsb),
                apply_model_corrections.after(sync_aircraft_from_adsb),
            ),
        );

        #[cfg(feature = "fusion")]
        app.add_systems(
            Update,
            (
                update_aircraft_label_text,
                apply_model_corrections,
            ),
        );

        app.add_systems(Update, (update_connection_status, reconnect_on_config_change));
    }
}
