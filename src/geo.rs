//! Shared geodesic math, aviation constants, and coordinate conversion.
//!
//! Centralizes haversine distance, bearing, position prediction,
//! coordinate conversion helpers, and commonly used aviation constants
//! that were previously scattered across multiple modules.

use bevy::prelude::*;
use bevy_slippy_tiles::*;

// =============================================================================
// Aviation Constants
// =============================================================================

/// Earth radius in nautical miles (WGS-84 mean radius)
pub const EARTH_RADIUS_NM: f64 = 3440.065;

/// Conversion factor: feet to meters
pub const FEET_TO_METERS: f64 = 0.3048;

/// Conversion factor: nautical miles to kilometers
pub const NM_TO_KM: f64 = 1.852;

/// Flight level threshold in feet (at or above 18,000 ft, altitudes
/// are expressed as flight levels)
pub const FL_THRESHOLD: i32 = 18000;

/// Altitude below which aircraft is considered on the ground
pub const GROUND_TRAFFIC_ALT: i32 = 100;

// =============================================================================
// Geodesic Functions
// =============================================================================

/// Calculate the great-circle distance between two lat/lon points
/// using the Haversine formula. Returns distance in nautical miles.
pub fn haversine_distance_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lon = (lon2 - lon1).to_radians();

    let a = (delta_lat / 2.0).sin().powi(2)
        + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();

    EARTH_RADIUS_NM * c
}

/// Calculate the initial bearing (forward azimuth) from point 1 to point 2.
/// Returns bearing in degrees (0-360, clockwise from north).
pub fn initial_bearing(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let delta_lon = (lon2 - lon1).to_radians();

    let x = delta_lon.sin() * lat2_rad.cos();
    let y = lat1_rad.cos() * lat2_rad.sin() - lat1_rad.sin() * lat2_rad.cos() * delta_lon.cos();

    let bearing = x.atan2(y).to_degrees();
    (bearing + 360.0) % 360.0
}

/// Predict a future position given current position, heading, speed, and time.
/// Uses great-circle (spherical) trigonometry for accuracy over long distances.
///
/// - `lat`, `lon`: current position in degrees
/// - `heading_deg`: heading in degrees (0 = north, clockwise)
/// - `speed_knots`: ground speed in knots
/// - `minutes`: time horizon in minutes
///
/// Returns `(latitude, longitude)` in degrees.
pub fn predict_position(
    lat: f64,
    lon: f64,
    heading_deg: f32,
    speed_knots: f64,
    minutes: f32,
) -> (f64, f64) {
    // Convert speed from knots to nautical miles per minute
    let nm_per_minute = speed_knots / 60.0;

    // Distance to travel in nautical miles
    let distance_nm = nm_per_minute * minutes as f64;

    // Convert heading to radians (0 = north, clockwise positive)
    let heading_rad = (heading_deg as f64).to_radians();

    // Calculate angular distance
    let angular_distance = distance_nm / EARTH_RADIUS_NM;

    // Current position in radians
    let lat1 = lat.to_radians();
    let lon1 = lon.to_radians();

    // Calculate new latitude
    let lat2 = (lat1.sin() * angular_distance.cos()
        + lat1.cos() * angular_distance.sin() * heading_rad.cos())
    .asin();

    // Calculate new longitude
    let lon2 = lon1
        + (heading_rad.sin() * angular_distance.sin() * lat1.cos())
            .atan2(angular_distance.cos() - lat1.sin() * lat2.sin());

    (lat2.to_degrees(), lon2.to_degrees())
}

// =============================================================================
// Coordinate Converter
// =============================================================================

/// Helper that encapsulates the repeated reference-point-based coordinate
/// conversion boilerplate. Construct one per frame/system from
/// `SlippyTilesSettings` and `MapState`, then use `latlon_to_world` to
/// convert geographic coordinates to Bevy world-space positions.
pub struct CoordinateConverter {
    reference_pixel: (f64, f64),
    zoom_level: ZoomLevel,
}

impl CoordinateConverter {
    /// Build a converter from tile settings and a zoom level.
    pub fn new(tile_settings: &SlippyTilesSettings, zoom_level: ZoomLevel) -> Self {
        let reference_ll = LatitudeLongitudeCoordinates {
            latitude: tile_settings.reference_latitude,
            longitude: tile_settings.reference_longitude,
        };
        let reference_pixel = world_coords_to_world_pixel(
            &reference_ll,
            crate::constants::DEFAULT_TILE_SIZE,
            zoom_level,
        );
        Self {
            reference_pixel,
            zoom_level,
        }
    }

    /// Convert a latitude/longitude to a Bevy world-space Vec2 position,
    /// relative to the tile reference point.
    pub fn latlon_to_world(&self, lat: f64, lon: f64) -> Vec2 {
        let ll = LatitudeLongitudeCoordinates {
            latitude: lat,
            longitude: lon,
        };
        let pixel =
            world_coords_to_world_pixel(&ll, crate::constants::DEFAULT_TILE_SIZE, self.zoom_level);
        Vec2::new(
            (pixel.0 - self.reference_pixel.0) as f32,
            (pixel.1 - self.reference_pixel.1) as f32,
        )
    }
}
