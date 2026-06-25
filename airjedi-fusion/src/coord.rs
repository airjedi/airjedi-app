use std::f64::consts::PI;

const WGS84_A: f64 = 6_378_137.0;
const WGS84_F: f64 = 1.0 / 298.257_223_563;
const WGS84_B: f64 = WGS84_A * (1.0 - WGS84_F);
const WGS84_E2: f64 = 1.0 - (WGS84_B * WGS84_B) / (WGS84_A * WGS84_A);

#[derive(Debug, Clone, PartialEq)]
pub enum CoordinateFrame {
    Wgs84,
    Ecef,
    Enu {
        origin_lat_deg: f64,
        origin_lon_deg: f64,
        origin_alt_m: f64,
    },
    SensorSpherical {
        sensor_lat_deg: f64,
        sensor_lon_deg: f64,
        sensor_alt_m: f64,
    },
}

fn deg2rad(d: f64) -> f64 {
    d * PI / 180.0
}

fn rad2deg(r: f64) -> f64 {
    r * 180.0 / PI
}

#[must_use]
pub fn geodetic_to_ecef(lat_deg: f64, lon_deg: f64, alt_m: f64) -> [f64; 3] {
    let lat = deg2rad(lat_deg);
    let lon = deg2rad(lon_deg);
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let sin_lon = lon.sin();
    let cos_lon = lon.cos();

    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();

    [
        (n + alt_m) * cos_lat * cos_lon,
        (n + alt_m) * cos_lat * sin_lon,
        (n * (1.0 - WGS84_E2) + alt_m) * sin_lat,
    ]
}

#[must_use]
pub fn ecef_to_geodetic(ecef: &[f64; 3]) -> (f64, f64, f64) {
    let x = ecef[0];
    let y = ecef[1];
    let z = ecef[2];

    let lon = y.atan2(x);
    let p = (x * x + y * y).sqrt();

    // Iterative Bowring method
    let mut lat = (z / p).atan();
    for _ in 0..10 {
        let sin_lat = lat.sin();
        let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
        lat = (z + WGS84_E2 * n * sin_lat).atan2(p);
    }

    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
    let alt = if cos_lat.abs() > 1e-10 {
        p / cos_lat - n
    } else {
        z.abs() / sin_lat.abs() - n * (1.0 - WGS84_E2)
    };

    (rad2deg(lat), rad2deg(lon), alt)
}

#[must_use]
pub fn ecef_to_enu(
    ecef: &[f64; 3],
    ref_lat_deg: f64,
    ref_lon_deg: f64,
    ref_alt_m: f64,
) -> [f64; 3] {
    let ref_ecef = geodetic_to_ecef(ref_lat_deg, ref_lon_deg, ref_alt_m);
    let dx = ecef[0] - ref_ecef[0];
    let dy = ecef[1] - ref_ecef[1];
    let dz = ecef[2] - ref_ecef[2];

    let lat = deg2rad(ref_lat_deg);
    let lon = deg2rad(ref_lon_deg);
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let sin_lon = lon.sin();
    let cos_lon = lon.cos();

    let e = -sin_lon * dx + cos_lon * dy;
    let n = -sin_lat * cos_lon * dx - sin_lat * sin_lon * dy + cos_lat * dz;
    let u = cos_lat * cos_lon * dx + cos_lat * sin_lon * dy + sin_lat * dz;

    [e, n, u]
}

#[must_use]
pub fn spherical_to_ecef(
    range_m: f64,
    az_rad: f64,
    el_rad: f64,
    sensor_ecef: &[f64; 3],
    sensor_lat_deg: f64,
    sensor_lon_deg: f64,
) -> [f64; 3] {
    let cos_el = el_rad.cos();
    let e = range_m * cos_el * az_rad.sin();
    let n = range_m * cos_el * az_rad.cos();
    let u = range_m * el_rad.sin();

    let lat = deg2rad(sensor_lat_deg);
    let lon = deg2rad(sensor_lon_deg);
    let sin_lat = lat.sin();
    let cos_lat = lat.cos();
    let sin_lon = lon.sin();
    let cos_lon = lon.cos();

    // ENU to ECEF rotation (inverse of ECEF-to-ENU)
    let dx = -sin_lon * e - sin_lat * cos_lon * n + cos_lat * cos_lon * u;
    let dy = cos_lon * e - sin_lat * sin_lon * n + cos_lat * sin_lon * u;
    let dz = cos_lat * n + sin_lat * u;

    [
        sensor_ecef[0] + dx,
        sensor_ecef[1] + dy,
        sensor_ecef[2] + dz,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn geodetic_ecef_round_trip_wichita() {
        let lat = 37.6872;
        let lon = -97.3301;
        let alt = 0.0;
        let ecef = geodetic_to_ecef(lat, lon, alt);
        let (lat2, lon2, alt2) = ecef_to_geodetic(&ecef);
        assert_relative_eq!(lat, lat2, epsilon = 1e-9);
        assert_relative_eq!(lon, lon2, epsilon = 1e-9);
        assert_relative_eq!(alt, alt2, epsilon = 1e-3);
    }

    #[test]
    fn geodetic_ecef_round_trip_with_altitude() {
        let lat = 37.6872;
        let lon = -97.3301;
        let alt = 10_000.0;
        let ecef = geodetic_to_ecef(lat, lon, alt);
        let (lat2, lon2, alt2) = ecef_to_geodetic(&ecef);
        assert_relative_eq!(lat, lat2, epsilon = 1e-9);
        assert_relative_eq!(lon, lon2, epsilon = 1e-9);
        assert_relative_eq!(alt, alt2, epsilon = 1e-3);
    }

    #[test]
    fn geodetic_ecef_equator_prime_meridian() {
        let ecef = geodetic_to_ecef(0.0, 0.0, 0.0);
        assert_relative_eq!(ecef[0], WGS84_A, epsilon = 1.0);
        assert_relative_eq!(ecef[1], 0.0, epsilon = 1.0);
        assert_relative_eq!(ecef[2], 0.0, epsilon = 1.0);
    }

    #[test]
    fn geodetic_ecef_north_pole() {
        let ecef = geodetic_to_ecef(90.0, 0.0, 0.0);
        assert_relative_eq!(ecef[0], 0.0, epsilon = 1.0);
        assert_relative_eq!(ecef[1], 0.0, epsilon = 1.0);
        assert_relative_eq!(ecef[2], WGS84_B, epsilon = 1.0);
    }

    #[test]
    fn ecef_to_enu_origin_is_zero() {
        let ref_lat = 37.6872;
        let ref_lon = -97.3301;
        let ref_alt = 0.0;
        let ecef = geodetic_to_ecef(ref_lat, ref_lon, ref_alt);
        let enu = ecef_to_enu(&ecef, ref_lat, ref_lon, ref_alt);
        assert_relative_eq!(enu[0], 0.0, epsilon = 1e-6);
        assert_relative_eq!(enu[1], 0.0, epsilon = 1e-6);
        assert_relative_eq!(enu[2], 0.0, epsilon = 1e-6);
    }

    #[test]
    fn ecef_to_enu_north_displacement() {
        let ref_lat = 37.0;
        let ref_lon = -97.0;
        let ref_alt = 0.0;
        let target_ecef = geodetic_to_ecef(38.0, -97.0, 0.0);
        let enu = ecef_to_enu(&target_ecef, ref_lat, ref_lon, ref_alt);
        assert_relative_eq!(enu[0], 0.0, epsilon = 500.0);
        assert!(enu[1] > 110_000.0 && enu[1] < 112_000.0);
        // "Up" is negative due to Earth's curvature over 1 degree (~111km arc)
        assert!(enu[2].abs() < 1500.0);
    }

    #[test]
    fn spherical_to_ecef_north_target() {
        let sensor_lat = 37.0;
        let sensor_lon = -97.0;
        let sensor_alt = 0.0;
        let sensor_ecef = geodetic_to_ecef(sensor_lat, sensor_lon, sensor_alt);
        let az = 0.0_f64;
        let el = 0.0_f64;
        let range = 10_000.0;
        let target_ecef = spherical_to_ecef(range, az, el, &sensor_ecef, sensor_lat, sensor_lon);
        let (t_lat, t_lon, _) = ecef_to_geodetic(&target_ecef);
        assert!(t_lat > sensor_lat);
        assert_relative_eq!(t_lon, sensor_lon, epsilon = 0.01);
    }
}
