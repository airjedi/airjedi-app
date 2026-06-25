pub mod associator;
pub mod classification;
pub mod config;
pub mod coord;
pub mod filter;
pub mod prelude_imports;
pub mod sensor;
pub mod store;
pub mod systems;
pub mod track;
pub mod transport;
pub mod types;

pub use classification::TargetClassification;
pub use config::FusionConfig;
pub use filter::TrackerState;
pub use sensor::{Measurement, SensorObservation};
pub use store::TimelineStore;
pub use track::{Track, TrackQuality, TrackStatus};
pub use types::*;

pub use nalgebra;

use prelude_imports::*;
use systems::FusionSet;

pub struct FusionPlugin;

impl Plugin for FusionPlugin {
    fn build(&self, app: &mut App) {
        let config = app
            .world()
            .get_resource::<FusionConfig>()
            .cloned()
            .unwrap_or_default();

        app.init_resource::<systems::ObservationBuffer>()
            .insert_resource(TimelineStore::new(config.store.clone()))
            .insert_resource(config.lifecycle.clone())
            .insert_resource(config.associator.clone())
            .insert_resource(config.clone())
            .insert_resource(associator::spatial_index::SpatialIndex::new(
                config.spatial_cell_size_deg,
            ))
            .configure_sets(
                Update,
                (
                    FusionSet::Drain,
                    FusionSet::Associate,
                    FusionSet::Fuse,
                    FusionSet::Lifecycle,
                )
                    .chain(),
            )
            .add_systems(Update, systems::drain_observations.in_set(FusionSet::Drain))
            .add_systems(
                Update,
                systems::update_spatial_index.in_set(FusionSet::Associate),
            )
            .add_systems(
                Update,
                systems::association_system
                    .in_set(FusionSet::Associate)
                    .after(systems::update_spatial_index),
            )
            .add_systems(
                Update,
                systems::fusion_update_system.in_set(FusionSet::Fuse),
            )
            .add_systems(
                Update,
                systems::track_status_system.in_set(FusionSet::Lifecycle),
            )
            .add_systems(
                Update,
                systems::track_initiation_system.in_set(FusionSet::Lifecycle),
            )
            .add_systems(
                Update,
                systems::track_cleanup_system.in_set(FusionSet::Lifecycle),
            )
            .add_systems(
                Update,
                systems::store_eviction_system.in_set(FusionSet::Lifecycle),
            );

        // Conditionally add NATS transport systems when feature is enabled
        #[cfg(feature = "nats")]
        if let Some(transport_config) = config.transport.clone() {
            let transport = transport::nats::NatsTransport::start(transport_config);
            app.insert_resource(transport)
                .add_systems(
                    Update,
                    transport::nats::nats_subscribe_drain_system.in_set(FusionSet::Drain),
                )
                .add_systems(
                    Update,
                    transport::nats::nats_publish_system.after(FusionSet::Fuse),
                );
        }
    }
}
