use crate::aircraft::components::FusionTrackLink;
use crate::map::MapState;
use airjedi_fusion::{TrackQuality, TrackStatus, TrackerState};
use bevy::prelude::*;

pub fn render_uncertainty_ellipses(
    fusion_tracks: Query<(&TrackerState, &TrackQuality)>,
    visuals: Query<(&FusionTrackLink, &Transform)>,
    map_state: Res<MapState>,
    mut gizmos: Gizmos,
) {
    for (link, transform) in &visuals {
        let Ok((tracker, quality)) = fusion_tracks.get(link.track_entity) else {
            continue;
        };

        if quality.status != TrackStatus::Coasting {
            continue;
        }

        let cov = tracker.variant.covariance_mat();
        if cov.nrows() < 3 {
            continue;
        }

        // Project ECEF covariance to local ENU to get horizontal uncertainty.
        // R_enu transforms ECEF deltas to ENU: [east, north, up].
        // Horizontal uncertainty = sqrt(var_east + var_north).
        let (lat, lon, _) = tracker.position_geodetic();
        let lat_rad = lat.to_radians();
        let lon_rad = lon.to_radians();

        let sin_lat = lat_rad.sin();
        let cos_lat = lat_rad.cos();
        let sin_lon = lon_rad.sin();
        let cos_lon = lon_rad.cos();

        // ENU rotation rows (only need east and north for horizontal)
        // east  = [-sin_lon,  cos_lon,  0]
        // north = [-sin_lat*cos_lon, -sin_lat*sin_lon, cos_lat]
        let pos_cov = cov.view((0, 0), (3, 3));

        let var_east = sin_lon * sin_lon * pos_cov[(0, 0)] + cos_lon * cos_lon * pos_cov[(1, 1)]
            - 2.0 * sin_lon * cos_lon * pos_cov[(0, 1)];

        let var_north = (sin_lat * cos_lon).powi(2) * pos_cov[(0, 0)]
            + (sin_lat * sin_lon).powi(2) * pos_cov[(1, 1)]
            + cos_lat.powi(2) * pos_cov[(2, 2)]
            + 2.0 * sin_lat.powi(2) * sin_lon * cos_lon * pos_cov[(0, 1)]
            - 2.0 * sin_lat * cos_lat * cos_lon * pos_cov[(0, 2)]
            - 2.0 * sin_lat * cos_lat * sin_lon * pos_cov[(1, 2)];

        // 1-sigma horizontal position uncertainty in meters
        let h_uncertainty_m = (var_east.abs() + var_north.abs()).sqrt();

        // Convert meters to world units:
        // At zoom Z, one tile = 256 px covers (360 / 2^Z) degrees longitude at equator.
        // 1 degree latitude ~ 111,320 meters.
        // World units per degree = 256 * 2^Z / 360 (approx, ignoring Mercator stretch).
        // So world_units = meters / 111320 * 256 * 2^Z / 360...
        // Simpler: at the current zoom, the camera scale determines pixels per meter.
        // Use the tile-based approximation from the geo module.
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
