mod adsb_adapter;
pub(crate) mod estimated_track;
#[allow(dead_code)]
pub(crate) mod fusion_ui;
mod interpolation;
mod landing_detection;
pub(crate) mod multi_sensor_debug;
mod render_bridge;
mod uncertainty_viz;

use bevy::prelude::*;

pub struct FusionIntegrationPlugin;

impl Plugin for FusionIntegrationPlugin {
    fn build(&self, app: &mut App) {
        use airjedi_fusion::config::FusionConfig;
        use airjedi_fusion::systems::FusionSet;
        use airjedi_fusion::FusionPlugin;

        if !app.world().contains_resource::<FusionConfig>() {
            app.insert_resource(FusionConfig::default());
        }

        app.add_plugins(FusionPlugin)
            .register_type::<estimated_track::EstimatedTrackConfig>()
            .init_resource::<estimated_track::EstimatedTrackConfig>()
            .init_resource::<estimated_track::HeadingHistory>()
            .register_type::<multi_sensor_debug::MultiSensorDebugConfig>()
            .init_resource::<multi_sensor_debug::MultiSensorDebugConfig>()
            .add_systems(
                Update,
                adsb_adapter::adsb_to_fusion_system.before(FusionSet::Drain),
            )
            .add_systems(
                Update,
                (
                    render_bridge::sync_tracks_to_visuals.after(FusionSet::Lifecycle),
                    interpolation::interpolate_display_positions
                        .after(render_bridge::sync_tracks_to_visuals),
                    uncertainty_viz::render_uncertainty_ellipses
                        .after(render_bridge::sync_tracks_to_visuals),
                    estimated_track::update_heading_history
                        .after(render_bridge::sync_tracks_to_visuals),
                    estimated_track::draw_estimated_track_cones
                        .after(estimated_track::update_heading_history)
                        .after(crate::aircraft::interpolation::interpolate_aircraft_positions)
                        .after(crate::ZoomSet::Change),
                    estimated_track::draw_all_aircraft_predictions
                        .after(render_bridge::sync_tracks_to_visuals)
                        .after(crate::aircraft::interpolation::interpolate_aircraft_positions)
                        .after(crate::ZoomSet::Change),
                    render_bridge::refresh_aircraft_last_seen
                        .after(render_bridge::sync_tracks_to_visuals),
                    render_bridge::cleanup_orphaned_visuals
                        .after(render_bridge::refresh_aircraft_last_seen),
                    landing_detection::detect_landings.after(render_bridge::sync_tracks_to_visuals),
                    landing_detection::cleanup_landed_aircraft
                        .after(landing_detection::detect_landings),
                    multi_sensor_debug::draw_multi_sensor_sources
                        .after(render_bridge::sync_tracks_to_visuals)
                        .after(crate::ZoomSet::Change),
                ),
            );
    }
}
