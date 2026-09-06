pub mod connection;
pub mod enrichment;
pub mod sync;

pub use connection::*;
pub use sync::*;

use bevy::prelude::*;

/// When present, the app opens no local ADS-B feeds or enrichment streams.
/// Thin-client mode (design-b) inserts this so the headless fusion agent is the
/// sole source of tracks; `AdsbPlugin` still provides the model registry and
/// aircraft post-processing systems the thin-client hydrator relies on.
#[derive(Resource, Default)]
pub struct LocalIngestDisabled;

pub struct AdsbPlugin;

impl Plugin for AdsbPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            (
                setup_aircraft_models,
                setup_feed_connections.after(crate::setup_map),
                enrichment::setup_enrichment_connections,
            ),
        );

        app.add_systems(
            Update,
            (
                apply_model_corrections,
                make_aircraft_unlit.after(apply_model_corrections),
            ),
        );

        app.add_systems(
            Update,
            (
                update_connection_status,
                reconnect_on_feed_changes,
                enrichment::reconnect_on_enrichment_changes,
            ),
        );
    }
}
