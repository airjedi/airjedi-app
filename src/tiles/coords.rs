use std::f64::consts::PI;

use super::types::ZoomLevel;

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
}
