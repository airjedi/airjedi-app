use bevy::prelude::*;
use airjedi_fusion::TrackerState;
use crate::aircraft::components::FusionTrackLink;
use crate::aircraft::InterpolationState;

/// Sub-frame interpolation between fusion updates.
///
/// The render bridge writes position data whenever TrackerState changes.
/// The existing InterpolationState system in aircraft/interpolation.rs
/// handles dead-reckoning and blending between those updates, so this
/// system only needs to ensure the prediction flag stays current.
pub fn interpolate_display_positions(
    fusion_tracks: Query<&TrackerState>,
    mut visuals: Query<(&FusionTrackLink, &mut InterpolationState)>,
) {
    for (link, mut interp) in &mut visuals {
        let Ok(tracker) = fusion_tracks.get(link.track_entity) else {
            continue;
        };

        let vel = tracker.velocity_ecef();
        let speed_mps = (vel[0].powi(2) + vel[1].powi(2) + vel[2].powi(2)).sqrt();
        let speed_kts = speed_mps / 0.514444;

        interp.predicting = speed_kts > crate::aircraft::interpolation::MIN_PREDICTION_SPEED_KTS;
    }
}
