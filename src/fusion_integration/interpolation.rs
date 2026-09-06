use crate::aircraft::components::FusionTrackLink;
use crate::aircraft::InterpolationState;
use airjedi_core::DisplayTrack;
use bevy::prelude::*;

/// Sub-frame interpolation between fusion updates.
///
/// The render bridge writes position data whenever TrackerState changes.
/// The existing InterpolationState system in aircraft/interpolation.rs
/// handles dead-reckoning and blending between those updates, so this
/// system only needs to ensure the prediction flag stays current - which it
/// now reads straight from the projected `DisplayTrack` instead of recomputing
/// the speed from the filter's ECEF velocity.
pub fn interpolate_display_positions(
    display_tracks: Query<&DisplayTrack>,
    mut visuals: Query<(&FusionTrackLink, &mut InterpolationState)>,
) {
    for (link, mut interp) in &mut visuals {
        let Ok(track) = display_tracks.get(link.track_entity) else {
            continue;
        };

        interp.predicting = track.predicting;
    }
}
