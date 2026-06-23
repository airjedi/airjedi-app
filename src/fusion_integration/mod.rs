#[cfg(feature = "fusion")]
mod adsb_adapter;
#[cfg(feature = "fusion")]
mod render_bridge;
#[cfg(feature = "fusion")]
mod interpolation;
#[cfg(feature = "fusion")]
mod uncertainty_viz;
#[cfg(feature = "fusion")]
pub(crate) mod estimated_track;
#[cfg(feature = "fusion")]
#[allow(dead_code)]
pub(crate) mod fusion_ui;
#[cfg(feature = "fusion")]
mod landing_detection;

use bevy::prelude::*;

pub struct FusionIntegrationPlugin;

#[cfg(feature = "fusion")]
impl Plugin for FusionIntegrationPlugin {
    fn build(&self, app: &mut App) {
        use airjedi_fusion::config::FusionConfig;
        use airjedi_fusion::FusionPlugin;
        use airjedi_fusion::systems::FusionSet;

        if !app.world().contains_resource::<FusionConfig>() {
            app.insert_resource(FusionConfig::default());
        }

        app.add_plugins(FusionPlugin)
            .register_type::<estimated_track::EstimatedTrackConfig>()
            .init_resource::<estimated_track::EstimatedTrackConfig>()
            .add_systems(
                Update,
                adsb_adapter::adsb_to_fusion_system
                    .before(FusionSet::Drain),
            )
            .add_systems(
                Update,
                (
                    render_bridge::sync_tracks_to_visuals
                        .after(FusionSet::Lifecycle),
                    interpolation::interpolate_display_positions
                        .after(render_bridge::sync_tracks_to_visuals),
                    uncertainty_viz::render_uncertainty_ellipses
                        .after(render_bridge::sync_tracks_to_visuals),
                    estimated_track::draw_estimated_track_cones
                        .after(render_bridge::sync_tracks_to_visuals)
                        .after(crate::ZoomSet::Change),
                    render_bridge::cleanup_orphaned_visuals
                        .after(render_bridge::sync_tracks_to_visuals),
                    landing_detection::detect_landings
                        .after(render_bridge::sync_tracks_to_visuals),
                    landing_detection::cleanup_landed_aircraft
                        .after(landing_detection::detect_landings),
                ),
            );
    }
}

#[cfg(not(feature = "fusion"))]
impl Plugin for FusionIntegrationPlugin {
    fn build(&self, _app: &mut App) {}
}
