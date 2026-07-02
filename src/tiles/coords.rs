use std::f64::consts::PI;

use bevy::math::{DVec2, DVec3};
use bevy::prelude::*;

use super::types::ZoomLevel;

// =============================================================================
// Web Mercator (EPSG:3857) coordinate system
// =============================================================================

pub const WEB_MERCATOR_EXTENT: f64 = 20037508.342789244;

const DEFAULT_RECENTER_DISTANCE: f64 = 25_000.0;

/// Convert longitude/latitude (degrees) to Web Mercator meters (EPSG:3857).
pub fn lonlat_to_mercator(lon: f64, lat: f64) -> DVec2 {
    let x = lon * WEB_MERCATOR_EXTENT / 180.0;
    let lat_rad = lat.to_radians();
    let y = (PI / 4.0 + lat_rad / 2.0).tan().ln() * WEB_MERCATOR_EXTENT / PI;
    DVec2::new(x, y)
}

/// Convert Web Mercator meters (EPSG:3857) back to longitude/latitude (degrees).
pub fn mercator_to_lonlat(mercator: DVec2) -> (f64, f64) {
    let lon = mercator.x * 180.0 / WEB_MERCATOR_EXTENT;
    let lat = (PI * mercator.y / WEB_MERCATOR_EXTENT).exp().atan() * 2.0 - PI / 2.0;
    (lon, lat.to_degrees())
}

/// Axis-aligned bounding box in Web Mercator meters.
#[derive(Debug, Clone, Copy)]
pub struct MercatorAabb {
    pub min: DVec2,
    pub max: DVec2,
}

impl MercatorAabb {
    pub fn center(&self) -> DVec2 {
        (self.min + self.max) * 0.5
    }

    pub fn size(&self) -> DVec2 {
        self.max - self.min
    }
}

/// Compute the Mercator-meter bounding box for a tile at (x, y, zoom).
/// Tile Y is in standard web tile convention (top-left origin, Y increases downward).
/// The returned AABB is in Mercator meters where Y increases upward (north).
pub fn tile_to_mercator_aabb(x: u32, y: u32, zoom: u8) -> MercatorAabb {
    let tile_size = (2.0 * WEB_MERCATOR_EXTENT) / (1u64 << zoom) as f64;
    let num_tiles = (1u64 << zoom) as f64;
    // Tile X: left-to-right from -EXTENT
    let min_x = x as f64 * tile_size - WEB_MERCATOR_EXTENT;
    let max_x = (x as f64 + 1.0) * tile_size - WEB_MERCATOR_EXTENT;
    // Tile Y: flip from top-down tile convention to bottom-up Mercator
    let min_y = (num_tiles - y as f64 - 1.0) * tile_size - WEB_MERCATOR_EXTENT;
    let max_y = (num_tiles - y as f64) * tile_size - WEB_MERCATOR_EXTENT;
    MercatorAabb {
        min: DVec2::new(min_x, min_y),
        max: DVec2::new(max_x, max_y),
    }
}

/// Floating origin in Web Mercator meters. All entity positions are stored
/// relative to this origin to keep f32 values near zero and avoid precision loss.
#[derive(Resource, Debug, Clone)]
pub struct LocalOrigin {
    mercator_origin: DVec3,
    recenter_distance: f64,
}

impl LocalOrigin {
    pub fn new(mercator_origin: DVec3) -> Self {
        Self {
            mercator_origin,
            recenter_distance: DEFAULT_RECENTER_DISTANCE,
        }
    }

    pub fn from_latlon(lat: f64, lon: f64) -> Self {
        let mercator = lonlat_to_mercator(lon, lat);
        Self::new(mercator.extend(0.0))
    }

    pub fn mercator_origin(&self) -> DVec3 {
        self.mercator_origin
    }

    pub fn recenter_distance(&self) -> f64 {
        self.recenter_distance
    }

    pub fn shift_mercator_origin(&mut self, delta: DVec3) {
        self.mercator_origin += delta;
    }
}

/// Trait to convert between local Bevy coordinates and Mercator meters.
/// Mercator values use f64 (DVec2/DVec3) to avoid precision loss;
/// local values use f32 (Vec2/Vec3) for Bevy transforms.
pub trait LocalOriginConversion {
    type MercatorOutput;
    fn mercator_to_local(&self, origin: &LocalOrigin) -> Self;
    fn local_to_mercator(&self, origin: &LocalOrigin) -> Self::MercatorOutput;
}

impl LocalOriginConversion for DVec2 {
    type MercatorOutput = Self;
    fn mercator_to_local(&self, origin: &LocalOrigin) -> Self {
        *self - origin.mercator_origin().truncate()
    }
    fn local_to_mercator(&self, origin: &LocalOrigin) -> Self {
        *self + origin.mercator_origin().truncate()
    }
}

impl LocalOriginConversion for DVec3 {
    type MercatorOutput = Self;
    fn mercator_to_local(&self, origin: &LocalOrigin) -> Self {
        self.truncate().mercator_to_local(origin).extend(self.z)
    }
    fn local_to_mercator(&self, origin: &LocalOrigin) -> Self {
        self.truncate().local_to_mercator(origin).extend(self.z)
    }
}

impl LocalOriginConversion for Vec2 {
    type MercatorOutput = DVec2;
    fn mercator_to_local(&self, origin: &LocalOrigin) -> Self {
        self.as_dvec2().mercator_to_local(origin).as_vec2()
    }
    fn local_to_mercator(&self, origin: &LocalOrigin) -> DVec2 {
        self.as_dvec2().local_to_mercator(origin)
    }
}

impl LocalOriginConversion for Vec3 {
    type MercatorOutput = DVec3;
    fn mercator_to_local(&self, origin: &LocalOrigin) -> Self {
        self.truncate().mercator_to_local(origin).extend(self.z)
    }
    fn local_to_mercator(&self, origin: &LocalOrigin) -> DVec3 {
        self.truncate().local_to_mercator(origin).extend(self.z as f64)
    }
}

/// Slippy map tile coordinates.
/// See: https://wiki.openstreetmap.org/wiki/Slippy_map_tilenames
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct SlippyTileCoordinates {
    pub x: u32,
    pub y: u32,
}

impl SlippyTileCoordinates {
    pub fn from_latitude_longitude(lat: f64, lon: f64, zoom_level: ZoomLevel) -> Self {
        let z = zoom_level.to_u8() as u32;
        Self {
            x: longitude_to_tile_x(lon, z),
            y: latitude_to_tile_y(lat, z),
        }
    }

    pub fn to_latitude_longitude(&self, zoom_level: ZoomLevel) -> LatLon {
        let z = zoom_level.to_u8() as u32;
        LatLon {
            latitude: tile_y_to_latitude(self.y, z),
            longitude: tile_x_to_longitude(self.x, z),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLon {
    pub latitude: f64,
    pub longitude: f64,
}

impl LatLon {
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }

    pub fn to_tile_coords(&self, zoom_level: ZoomLevel) -> SlippyTileCoordinates {
        SlippyTileCoordinates::from_latitude_longitude(self.latitude, self.longitude, zoom_level)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Coordinates {
    SlippyTile(SlippyTileCoordinates),
    LatitudeLongitude(LatLon),
}

impl Coordinates {
    pub fn from_slippy_tile(x: u32, y: u32) -> Self {
        Coordinates::SlippyTile(SlippyTileCoordinates { x, y })
    }

    pub fn from_slippy_tile_coordinates(x: u32, y: u32) -> Self {
        Self::from_slippy_tile(x, y)
    }

    pub fn from_lat_lon(latitude: f64, longitude: f64) -> Self {
        Coordinates::LatitudeLongitude(LatLon {
            latitude,
            longitude,
        })
    }

    pub fn from_latitude_longitude(latitude: f64, longitude: f64) -> Self {
        Self::from_lat_lon(latitude, longitude)
    }

    pub fn to_tile_coords(&self, zoom_level: ZoomLevel) -> SlippyTileCoordinates {
        match self {
            Coordinates::LatitudeLongitude(ll) => ll.to_tile_coords(zoom_level),
            Coordinates::SlippyTile(tc) => *tc,
        }
    }

    pub fn get_slippy_tile_coordinates(&self, zoom_level: ZoomLevel) -> SlippyTileCoordinates {
        self.to_tile_coords(zoom_level)
    }

    pub fn to_lat_lon(&self, zoom_level: ZoomLevel) -> LatLon {
        match self {
            Coordinates::LatitudeLongitude(ll) => *ll,
            Coordinates::SlippyTile(tc) => tc.to_latitude_longitude(zoom_level),
        }
    }
}

// https://wiki.openstreetmap.org/wiki/Slippy_map_tilenames#Implementations
pub fn latitude_to_tile_y(lat: f64, zoom: u32) -> u32 {
    let lat_rad = lat.to_radians();
    ((1 << zoom) as f64 * (1.0 - (lat_rad.tan() + (1.0 / lat_rad.cos())).ln() / PI) / 2.0) as u32
}

pub fn longitude_to_tile_x(lon: f64, zoom: u32) -> u32 {
    ((1 << zoom) as f64 * (lon + 180.0) / 360.0) as u32
}

pub fn tile_y_to_latitude(y: u32, zoom: u32) -> f64 {
    let n = PI * (1.0 - 2.0 * y as f64 / (1 << zoom) as f64);
    n.sinh().atan().to_degrees()
}

pub fn tile_x_to_longitude(x: u32, zoom: u32) -> f64 {
    x as f64 / (1 << zoom) as f64 * 360.0 - 180.0
}

pub fn max_tiles_in_dimension(zoom_level: ZoomLevel) -> f64 {
    (1u64 << zoom_level.to_u8()) as f64
}

pub fn max_pixels_in_dimension(zoom_level: ZoomLevel, tile_size: super::types::TileSize) -> f64 {
    tile_size.to_pixels() as f64 * max_tiles_in_dimension(zoom_level)
}

pub fn world_pixel_to_lat_lon(
    x_pixel: f64,
    y_pixel: f64,
    tile_pixels: u32,
    zoom_level: ZoomLevel,
) -> LatLon {
    let z = zoom_level.to_u8();
    let max_pixels = tile_pixels as f64 * (1u64 << z) as f64;
    let y_flipped = max_pixels - y_pixel;
    let (longitude, latitude) =
        googleprojection::Mercator::with_size(tile_pixels as usize)
            .from_pixel_to_ll(&(x_pixel, y_flipped), z.into())
            .unwrap_or_default();
    LatLon {
        latitude,
        longitude,
    }
}

pub fn lat_lon_to_world_pixel(
    coords: &LatLon,
    tile_pixels: u32,
    zoom_level: ZoomLevel,
) -> (f64, f64) {
    let z = zoom_level.to_u8();
    let (x, y) = googleprojection::Mercator::with_size(tile_pixels as usize)
        .from_ll_to_subpixel(&(coords.longitude, coords.latitude), z.into())
        .unwrap_or_default();
    let max_pixels = tile_pixels as f64 * (1u64 << z) as f64;
    let y_flipped = max_pixels - y;
    (x, y_flipped)
}

/// Wrap tile X coordinate to handle antimeridian crossing.
pub fn wrap_tile_x(x: i64, zoom: u8) -> u32 {
    let max = 1i64 << zoom;
    ((x % max + max) % max) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_coordinates_roundtrip() {
        let lat = 37.6872;
        let lon = -97.3301;
        let zoom = ZoomLevel::L10;
        let tc = SlippyTileCoordinates::from_latitude_longitude(lat, lon, zoom);
        let ll = tc.to_latitude_longitude(zoom);
        assert!((ll.latitude - lat).abs() < 0.5);
        assert!((ll.longitude - lon).abs() < 0.5);
    }

    #[test]
    fn test_wrap_tile_x() {
        assert_eq!(wrap_tile_x(-1, 10), 1023);
        assert_eq!(wrap_tile_x(1024, 10), 0);
        assert_eq!(wrap_tile_x(512, 10), 512);
    }

    // =========================================================================
    // Mercator coordinate tests
    // =========================================================================

    #[test]
    fn test_lonlat_to_mercator_origin() {
        let m = lonlat_to_mercator(0.0, 0.0);
        assert!(m.x.abs() < 1.0, "origin X should be ~0, got {}", m.x);
        assert!(m.y.abs() < 1.0, "origin Y should be ~0, got {}", m.y);
    }

    #[test]
    fn test_lonlat_mercator_roundtrip() {
        let lon = -97.3301;
        let lat = 37.6872;
        let m = lonlat_to_mercator(lon, lat);
        let (lon2, lat2) = mercator_to_lonlat(m);
        assert!((lon - lon2).abs() < 1e-8, "lon roundtrip: {} vs {}", lon, lon2);
        assert!((lat - lat2).abs() < 1e-8, "lat roundtrip: {} vs {}", lat, lat2);
    }

    #[test]
    fn test_mercator_wichita_values() {
        let m = lonlat_to_mercator(-97.3301, 37.6872);
        // Wichita should be in western hemisphere (negative X) and northern (positive Y)
        assert!(m.x < 0.0, "Wichita X should be negative (western), got {}", m.x);
        assert!(m.y > 0.0, "Wichita Y should be positive (northern), got {}", m.y);
        // Approximate expected values: x ~ -10,833,000, y ~ 4,539,000
        assert!((m.x - -10_833_000.0).abs() < 10_000.0, "X ~-10.8M, got {}", m.x);
        assert!((m.y - 4_539_000.0).abs() < 10_000.0, "Y ~4.5M, got {}", m.y);
    }

    #[test]
    fn test_tile_to_mercator_aabb_zoom10_size() {
        let aabb = tile_to_mercator_aabb(0, 0, 10);
        let size = aabb.size();
        let expected_size = (2.0 * WEB_MERCATOR_EXTENT) / 1024.0; // 2^10 = 1024
        assert!(
            (size.x - expected_size).abs() < 1.0,
            "zoom-10 tile width should be ~{:.0}m, got {:.0}m",
            expected_size, size.x
        );
        assert!(
            (size.y - expected_size).abs() < 1.0,
            "zoom-10 tile height should be ~{:.0}m, got {:.0}m",
            expected_size, size.y
        );
        // Should be ~39,135 meters
        assert!(size.x > 39_000.0 && size.x < 40_000.0,
            "zoom-10 tile should be ~39km, got {:.0}m", size.x);
    }

    #[test]
    fn test_tile_to_mercator_aabb_position_independent_of_zoom() {
        // A point's mercator position should be the same regardless of which zoom
        // tile contains it. The tile center moves, but the geographic point stays put.
        let lon = -97.3301_f64;
        let lat = 37.6872_f64;
        let point_mercator = lonlat_to_mercator(lon, lat);

        for zoom in [10u8, 12, 14, 16] {
            let tc = SlippyTileCoordinates::from_latitude_longitude(lat, lon, ZoomLevel::try_from(zoom).unwrap());
            let aabb = tile_to_mercator_aabb(tc.x, tc.y, zoom);
            assert!(
                point_mercator.x >= aabb.min.x && point_mercator.x <= aabb.max.x,
                "zoom {}: point X {:.0} not in tile [{:.0}, {:.0}]",
                zoom, point_mercator.x, aabb.min.x, aabb.max.x
            );
            assert!(
                point_mercator.y >= aabb.min.y && point_mercator.y <= aabb.max.y,
                "zoom {}: point Y {:.0} not in tile [{:.0}, {:.0}]",
                zoom, point_mercator.y, aabb.min.y, aabb.max.y
            );
        }
    }

    #[test]
    fn test_local_origin_conversion_roundtrip() {
        let origin = LocalOrigin::from_latlon(37.6872, -97.3301);
        let point = lonlat_to_mercator(-97.0, 38.0);
        let local = point.mercator_to_local(&origin);
        let back = local.local_to_mercator(&origin);
        assert!((point.x - back.x).abs() < 1e-6);
        assert!((point.y - back.y).abs() < 1e-6);
    }

    #[test]
    fn test_local_coords_are_small() {
        let origin = LocalOrigin::from_latlon(37.6872, -97.3301);
        // A point 0.5 degrees away should be within ~55km
        let nearby = lonlat_to_mercator(-96.8, 38.0);
        let local = nearby.mercator_to_local(&origin);
        assert!(local.x.abs() < 100_000.0, "local X should be <100km, got {:.0}", local.x);
        assert!(local.y.abs() < 100_000.0, "local Y should be <100km, got {:.0}", local.y);
    }

    #[test]
    fn test_f32_precision_near_origin() {
        let origin = LocalOrigin::from_latlon(37.6872, -97.3301);
        // Two points 1 meter apart near the origin
        let p1 = lonlat_to_mercator(-97.3301, 37.6872);
        let p2 = DVec2::new(p1.x + 1.0, p1.y + 1.0);
        let l1 = p1.mercator_to_local(&origin).as_vec2();
        let l2 = p2.mercator_to_local(&origin).as_vec2();
        let diff = (l2 - l1).length();
        // Should resolve 1m displacement as f32 near origin
        assert!((diff - std::f32::consts::SQRT_2).abs() < 0.01,
            "1m displacement should be preserved in f32, got {}", diff);
    }
}
