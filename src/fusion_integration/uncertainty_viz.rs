use crate::aircraft::components::FusionTrackLink;
use crate::map::MapState;
use airjedi_core::{DisplayTrack, TrackStatus};
use bevy::prelude::*;

/// Draw a 1-sigma horizontal uncertainty circle around coasting tracks.
///
/// Reads the pre-reduced `DisplayTrack.h_uncertainty_m` (the agent computes the
/// ECEF covariance -> ENU 1-sigma reduction) rather than reaching into the
/// filter's covariance matrix. This system only converts meters to world units
/// and draws.
pub fn render_uncertainty_ellipses(
    display_tracks: Query<&DisplayTrack>,
    visuals: Query<(&FusionTrackLink, &Transform)>,
    map_state: Res<MapState>,
    mut gizmos: Gizmos,
) {
    for (link, transform) in &visuals {
        let Ok(track) = display_tracks.get(link.track_entity) else {
            continue;
        };

        if track.status != TrackStatus::Coasting {
            continue;
        }

        let Some(h_uncertainty_m) = track.h_uncertainty_m else {
            continue;
        };

        let cos_lat = track.latitude.to_radians().cos();

        // Convert meters to world units:
        // At zoom Z, one tile = 256 px covers (360 / 2^Z) degrees longitude at equator.
        // 1 degree latitude ~ 111,320 meters.
        // World units per degree = 256 * 2^Z / 360 (approx, ignoring Mercator stretch).
        let zoom = i32::from(map_state.zoom_level.to_u8());
        let tiles_around_earth = (1u64 << zoom) as f64;
        let world_units_per_degree = 256.0 * tiles_around_earth / 360.0;
        let meters_per_degree = 111_320.0 * cos_lat; // longitude shrinks with latitude
        let world_units_per_meter = world_units_per_degree / meters_per_degree;

        let radius = (h_uncertainty_m * world_units_per_meter) as f32;

        // Clamp to reasonable display range
        if radius > 2.0 && radius < 500.0 {
            let color = Color::srgba(1.0, 0.8, 0.2, 0.3);
            gizmos.circle_2d(
                Isometry2d::from_translation(transform.translation.truncate()),
                radius,
                color,
            );
        }
    }
}
