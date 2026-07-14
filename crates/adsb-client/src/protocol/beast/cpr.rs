// Copyright 2025 Chris Custine
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Compact Position Reporting (CPR) decoder.
//!
//! CPR encodes latitude and longitude in 17 bits each using a zone-based scheme.
//! Decoding requires either:
//! - Global decode: both an odd (F=1) and even (F=0) frame within 10 seconds
//! - Local decode: one frame plus a reference position (receiver location or last known)

use std::time::Instant;

/// Maximum age for CPR frame pairing (seconds).
const CPR_MAX_AGE_SECS: u64 = 10;

/// Number of latitude zones for airborne CPR.
const NZ: f64 = 15.0;

/// CPR scale factor (2^17).
const CPR_SCALE: f64 = 131072.0;

/// Per-aircraft CPR decode state.
pub struct CprState {
    even: Option<CprFrame>,
    odd: Option<CprFrame>,
    last_position: Option<(f64, f64)>,
}

struct CprFrame {
    lat_cpr: u32,
    lon_cpr: u32,
    timestamp: Instant,
}

impl CprState {
    pub fn new() -> Self {
        Self {
            even: None,
            odd: None,
            last_position: None,
        }
    }

    /// Update state with a new CPR frame and attempt position decode.
    pub fn update(
        &mut self,
        lat_cpr: u32,
        lon_cpr: u32,
        odd: bool,
        is_surface: bool,
        reference: Option<(f64, f64)>,
    ) -> Option<(f64, f64)> {
        let frame = CprFrame {
            lat_cpr,
            lon_cpr,
            timestamp: Instant::now(),
        };

        if odd {
            self.odd = Some(frame);
        } else {
            self.even = Some(frame);
        }

        // Try global decode first (needs both frames within time window)
        if let (Some(even), Some(odd_frame)) = (&self.even, &self.odd) {
            let elapsed = if even.timestamp > odd_frame.timestamp {
                even.timestamp.duration_since(odd_frame.timestamp)
            } else {
                odd_frame.timestamp.duration_since(even.timestamp)
            };

            if elapsed.as_secs() < CPR_MAX_AGE_SECS {
                let use_odd = odd_frame.timestamp > even.timestamp;
                let result = if is_surface {
                    global_decode_surface(
                        even.lat_cpr, even.lon_cpr,
                        odd_frame.lat_cpr, odd_frame.lon_cpr,
                        use_odd,
                        reference,
                    )
                } else {
                    global_decode_airborne(
                        even.lat_cpr, even.lon_cpr,
                        odd_frame.lat_cpr, odd_frame.lon_cpr,
                        use_odd,
                    )
                };

                if let Some(pos) = result {
                    if is_valid_position(pos.0, pos.1) {
                        self.last_position = Some(pos);
                        return Some(pos);
                    }
                }
            }
        }

        // Fall back to local decode if we have a reference position
        let ref_pos = self.last_position.or(reference)?;

        let (frame_lat, frame_lon, is_odd) = if odd {
            let f = self.odd.as_ref()?;
            (f.lat_cpr, f.lon_cpr, true)
        } else {
            let f = self.even.as_ref()?;
            (f.lat_cpr, f.lon_cpr, false)
        };

        let result = local_decode(frame_lat, frame_lon, is_odd, ref_pos.0, ref_pos.1, is_surface);

        if let Some(pos) = result {
            if is_valid_position(pos.0, pos.1) {
                self.last_position = Some(pos);
                return Some(pos);
            }
        }

        None
    }
}

fn is_valid_position(lat: f64, lon: f64) -> bool {
    (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)
}

/// Number of longitude zones at a given latitude.
/// Uses the standard CPR formula: NL = floor(2*pi / acos(1 - (1-cos(pi/(2*NZ))) / cos(pi/180*lat)^2))
#[allow(clippy::unreadable_literal)]
fn nl(lat: f64) -> u32 {
    // Use a precomputed lookup table for exactness and speed.
    // NL values by latitude band (each covers ~1.5 degrees).
    // Source: ICAO Doc 9871 / "The 1090MHz Riddle" by Junzi Sun.
    let lat = lat.abs();
    if lat < 10.47047130 { return 59; }
    if lat < 14.82817437 { return 58; }
    if lat < 18.18626357 { return 57; }
    if lat < 21.02939493 { return 56; }
    if lat < 23.54504487 { return 55; }
    if lat < 25.82924707 { return 54; }
    if lat < 27.93898710 { return 53; }
    if lat < 29.91135686 { return 52; }
    if lat < 31.77209708 { return 51; }
    if lat < 33.53993436 { return 50; }
    if lat < 35.22899598 { return 49; }
    if lat < 36.85025108 { return 48; }
    if lat < 38.41241892 { return 47; }
    if lat < 39.92256684 { return 46; }
    if lat < 41.38651832 { return 45; }
    if lat < 42.80914012 { return 44; }
    if lat < 44.19454951 { return 43; }
    if lat < 45.54626723 { return 42; }
    if lat < 46.86733252 { return 41; }
    if lat < 48.16039128 { return 40; }
    if lat < 49.42776439 { return 39; }
    if lat < 50.67150166 { return 38; }
    if lat < 51.89342469 { return 37; }
    if lat < 53.09516153 { return 36; }
    if lat < 54.27817472 { return 35; }
    if lat < 55.44378444 { return 34; }
    if lat < 56.59318756 { return 33; }
    if lat < 57.72747354 { return 32; }
    if lat < 58.84763776 { return 31; }
    if lat < 59.95459277 { return 30; }
    if lat < 61.04917774 { return 29; }
    if lat < 62.13216659 { return 28; }
    if lat < 63.20427479 { return 27; }
    if lat < 64.26616523 { return 26; }
    if lat < 65.31845310 { return 25; }
    if lat < 66.36171008 { return 24; }
    if lat < 67.39646774 { return 23; }
    if lat < 68.42322022 { return 22; }
    if lat < 69.44242631 { return 21; }
    if lat < 70.45451075 { return 20; }
    if lat < 71.45986473 { return 19; }
    if lat < 72.45884545 { return 18; }
    if lat < 73.45177442 { return 17; }
    if lat < 74.43893416 { return 16; }
    if lat < 75.42056257 { return 15; }
    if lat < 76.39684391 { return 14; }
    if lat < 77.36789461 { return 13; }
    if lat < 78.33374083 { return 12; }
    if lat < 79.29428225 { return 11; }
    if lat < 80.24923213 { return 10; }
    if lat < 81.19801349 { return 9; }
    if lat < 82.13956981 { return 8; }
    if lat < 83.07199445 { return 7; }
    if lat < 83.99173563 { return 6; }
    if lat < 84.89166191 { return 5; }
    if lat < 85.75541621 { return 4; }
    if lat < 86.53536998 { return 3; }
    if lat < 87.00000000 { return 2; }
    1
}

/// Latitude zone size.
fn dlat(odd: bool, is_surface: bool) -> f64 {
    let range = if is_surface { 90.0 } else { 360.0 };
    let nz4 = 4.0 * NZ; // 60
    let divisor = if odd { nz4 - 1.0 } else { nz4 };
    range / divisor
}

/// Global CPR decode for airborne positions.
fn global_decode_airborne(
    even_lat: u32, even_lon: u32,
    odd_lat: u32, odd_lon: u32,
    use_odd: bool,
) -> Option<(f64, f64)> {
    let lat_even = even_lat as f64 / CPR_SCALE;
    let lat_odd = odd_lat as f64 / CPR_SCALE;
    let lon_even = even_lon as f64 / CPR_SCALE;
    let lon_odd = odd_lon as f64 / CPR_SCALE;

    let dlat0 = 360.0 / (4.0 * NZ);
    let dlat1 = 360.0 / (4.0 * NZ - 1.0);

    let j = ((59.0 * lat_even - 60.0 * lat_odd + 0.5).floor()) as i32;

    let mut lat0 = dlat0 * (modulo(j, 60) as f64 + lat_even);
    let mut lat1 = dlat1 * (modulo(j, 59) as f64 + lat_odd);

    if lat0 >= 270.0 { lat0 -= 360.0; }
    if lat1 >= 270.0 { lat1 -= 360.0; }

    // Check longitude zone consistency
    let nl0 = nl(lat0);
    let nl1 = nl(lat1);
    if nl0 != nl1 {
        return None;
    }

    let (lat, lon) = if use_odd {
        let ni = std::cmp::max(nl1.saturating_sub(1), 1) as f64;
        let m = ((lon_even * (nl1 as f64 - 1.0) - lon_odd * nl1 as f64 + 0.5).floor()) as i32;
        let lon = (360.0 / ni) * (modulo(m, ni as i32) as f64 + lon_odd);
        let lon = if lon >= 180.0 { lon - 360.0 } else { lon };
        (lat1, lon)
    } else {
        let ni = std::cmp::max(nl0, 1) as f64;
        let m = ((lon_even * (nl0 as f64 - 1.0) - lon_odd * nl0 as f64 + 0.5).floor()) as i32;
        let lon = (360.0 / ni) * (modulo(m, ni as i32) as f64 + lon_even);
        let lon = if lon >= 180.0 { lon - 360.0 } else { lon };
        (lat0, lon)
    };

    Some((lat, lon))
}

/// Global CPR decode for surface positions.
fn global_decode_surface(
    even_lat: u32, even_lon: u32,
    odd_lat: u32, odd_lon: u32,
    use_odd: bool,
    reference: Option<(f64, f64)>,
) -> Option<(f64, f64)> {
    // Surface CPR uses 90-degree latitude range instead of 360
    let lat_even = even_lat as f64 / CPR_SCALE;
    let lat_odd = odd_lat as f64 / CPR_SCALE;
    let lon_even = even_lon as f64 / CPR_SCALE;
    let lon_odd = odd_lon as f64 / CPR_SCALE;

    let dlat0 = 90.0 / 60.0;
    let dlat1 = 90.0 / 59.0;

    let j = ((59.0 * lat_even - 60.0 * lat_odd + 0.5).floor()) as i32;

    let mut lat0 = dlat0 * (modulo(j, 60) as f64 + lat_even);
    let mut lat1 = dlat1 * (modulo(j, 59) as f64 + lat_odd);

    // Surface positions are always in the same hemisphere as the reference
    if let Some((ref_lat, _)) = reference {
        if ref_lat < 0.0 {
            lat0 -= 90.0;
            lat1 -= 90.0;
        }
    }

    let nl0 = nl(lat0);
    let nl1 = nl(lat1);
    if nl0 != nl1 {
        return None;
    }

    let (lat, lon) = if use_odd {
        let ni = std::cmp::max(nl1.saturating_sub(1), 1) as f64;
        let m = ((lon_even * (nl1 as f64 - 1.0) - lon_odd * nl1 as f64 + 0.5).floor()) as i32;
        let mut lon = (90.0 / ni) * (modulo(m, ni as i32) as f64 + lon_odd);
        if let Some((_, ref_lon)) = reference {
            lon = adjust_surface_longitude(lon, ref_lon);
        }
        (lat1, lon)
    } else {
        let ni = std::cmp::max(nl0, 1) as f64;
        let m = ((lon_even * (nl0 as f64 - 1.0) - lon_odd * nl0 as f64 + 0.5).floor()) as i32;
        let mut lon = (90.0 / ni) * (modulo(m, ni as i32) as f64 + lon_even);
        if let Some((_, ref_lon)) = reference {
            lon = adjust_surface_longitude(lon, ref_lon);
        }
        (lat0, lon)
    };

    Some((lat, lon))
}

fn adjust_surface_longitude(lon: f64, ref_lon: f64) -> f64 {
    // Pick the closest 90-degree quadrant to the reference longitude
    let mut result = lon;
    while (result - ref_lon).abs() > 45.0 {
        if result > ref_lon {
            result -= 90.0;
        } else {
            result += 90.0;
        }
    }
    if result > 180.0 { result - 360.0 }
    else if result < -180.0 { result + 360.0 }
    else { result }
}

/// Local CPR decode using a reference position.
fn local_decode(
    lat_cpr: u32, lon_cpr: u32,
    odd: bool,
    ref_lat: f64, ref_lon: f64,
    is_surface: bool,
) -> Option<(f64, f64)> {
    let d_lat = dlat(odd, is_surface);
    let lat_cpr_f = lat_cpr as f64 / CPR_SCALE;

    let j = (ref_lat / d_lat).floor() + ((ref_lat % d_lat) / d_lat - lat_cpr_f + 0.5).floor();
    let lat = d_lat * (j + lat_cpr_f);

    if lat.abs() > 90.0 {
        return None;
    }

    let nl_val = nl(lat);
    let ni = if odd {
        std::cmp::max(nl_val.saturating_sub(1), 1) as f64
    } else {
        std::cmp::max(nl_val, 1) as f64
    };

    let d_lon = if is_surface { 90.0 / ni } else { 360.0 / ni };
    let lon_cpr_f = lon_cpr as f64 / CPR_SCALE;

    let m = (ref_lon / d_lon).floor() + ((ref_lon % d_lon) / d_lon - lon_cpr_f + 0.5).floor();
    let mut lon = d_lon * (m + lon_cpr_f);

    if lon > 180.0 { lon -= 360.0; }
    if lon < -180.0 { lon += 360.0; }

    Some((lat, lon))
}

fn modulo(a: i32, b: i32) -> i32 {
    ((a % b) + b) % b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nl_values() {
        assert_eq!(nl(0.0), 59);
        assert_eq!(nl(87.0), 1);
        assert_eq!(nl(90.0), 1);
        assert_eq!(nl(-87.0), 1);
        assert_eq!(nl(45.0), 42);
        assert_eq!(nl(10.0), 59);
        assert_eq!(nl(11.0), 58);
        assert_eq!(nl(60.0), 29);
    }

    #[test]
    fn test_global_decode_airborne() {
        // Test vectors from "The 1090MHz Riddle" by Junzi Sun, Chapter 3
        // Even message: 8D40621D58C382D690C8AC2863A7
        //   CPR even: lat=93000, lon=51372
        // Odd message:  8D40621D58C386435CC412692AD6
        //   CPR odd:  lat=74158, lon=50194
        // Expected position: lat=52.2572, lon=3.9194
        let result = global_decode_airborne(93000, 51372, 74158, 50194, false);

        assert!(result.is_some(), "global decode returned None");
        let (lat, lon) = result.unwrap();
        assert!((lat - 52.2572).abs() < 0.01, "lat={lat}");
        assert!((lon - 3.9194).abs() < 0.05, "lon={lon}");
    }

    #[test]
    fn test_global_decode_southern_hemisphere() {
        // Verify decode doesn't panic for southern hemisphere
        let result = global_decode_airborne(108011, 110088, 75050, 36777, false);
        let _ = result;
    }

    #[test]
    fn test_local_decode() {
        // Local decode with reference position near expected result
        // Using even CPR values from the test above
        let result = local_decode(93000, 51372, false, 52.0, 4.0, false);
        assert!(result.is_some());
        let (lat, lon) = result.unwrap();
        assert!((lat - 52.0).abs() < 1.0, "lat={lat}");
        assert!((lon - 4.0).abs() < 1.0, "lon={lon}");
    }

    #[test]
    fn test_cpr_state_global_decode() {
        let mut state = CprState::new();

        // Use the same verified test vectors
        let result = state.update(93000, 51372, false, false, None);
        assert!(result.is_none());

        let result = state.update(74158, 50194, true, false, None);
        assert!(result.is_some());
        let (lat, lon) = result.unwrap();
        assert!((lat - 52.2572).abs() < 0.01, "lat={lat}");
        assert!((lon - 3.9194).abs() < 0.05, "lon={lon}");
    }

    #[test]
    fn test_is_valid_position() {
        assert!(is_valid_position(0.0, 0.0));
        assert!(is_valid_position(90.0, 180.0));
        assert!(is_valid_position(-90.0, -180.0));
        assert!(!is_valid_position(91.0, 0.0));
        assert!(!is_valid_position(0.0, 181.0));
    }

    #[test]
    fn test_modulo() {
        assert_eq!(modulo(5, 3), 2);
        assert_eq!(modulo(-1, 60), 59);
        assert_eq!(modulo(-5, 3), 1);
    }
}
