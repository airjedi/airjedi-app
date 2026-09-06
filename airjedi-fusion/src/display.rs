//! The fusion -> display projection: turn a fused track (`Track` +
//! `TrackerState` + `TrackQuality`) plus an optional raw-observation hint into a
//! serializable [`DisplayTrack`].
//!
//! This is the agent side of the design-b boundary. It lives in the fusion crate
//! (not the app binary) so it is reachable from headless tests - the tier-4
//! ingest->fusion->display snapshot test drives it directly. It stays free of
//! any sensor-specific dependency (no `adsb-client`): the caller passes the raw
//! ADS-B overrides through [`RawObservationHint`], so the same projection works
//! for any sensor that can fill that hint.

use crate::{IdentifierType, StateVectorType, Track, TrackQuality, TrackStatus, TrackerState};
use airjedi_core::{DisplayTrack, PositionSource};

/// Below this ground speed the client should not dead-reckon between updates.
/// Owned here so `DisplayTrack.predicting` and the client interpolation agree.
pub const MIN_PREDICTION_SPEED_KTS: f64 = 10.0;

/// Optional raw-observation overrides that take priority over the filter's
/// estimate for a confirmed (non-coasting) track. The app fills this from the
/// latest ADS-B report; a test can leave any field `None` to project the pure
/// filter estimate.
#[derive(Debug, Clone, Default)]
pub struct RawObservationHint {
    pub altitude_ft: Option<i32>,
    pub vertical_rate: Option<i32>,
    pub track_deg: Option<f64>,
    pub velocity_kts: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub squawk: Option<String>,
    pub is_on_ground: Option<bool>,
    pub alert: Option<bool>,
    pub emergency: Option<bool>,
    pub spi: Option<bool>,
    pub roll_angle: Option<f32>,
    pub track_angle_rate: Option<f32>,
    pub callsign: Option<String>,
}

/// Human label for the active filter, used in diagnostics UIs.
#[must_use]
pub fn filter_type_label(tracker: &TrackerState) -> &'static str {
    if tracker.mode_info().is_some() {
        "IMM"
    } else {
        match tracker.state_type {
            StateVectorType::Surface4Dof => "Surface",
            _ => "EKF",
        }
    }
}

/// Derive the render-ready [`DisplayTrack`] for a fused track. Applies the
/// prefer-raw/prefer-filter merge policy (raw wins when confirmed; the filter's
/// forward-propagated estimate wins while coasting) and pre-reduces the
/// covariance to a scalar horizontal uncertainty so nothing un-serializable
/// crosses the boundary.
#[must_use]
pub fn derive_display_track(
    track: &Track,
    tracker: &TrackerState,
    quality: &TrackQuality,
    hint: Option<&RawObservationHint>,
    position_source: Option<PositionSource>,
) -> DisplayTrack {
    let (lat, lon, alt_m) = tracker.position_geodetic();
    let filter_alt_ft = (alt_m / 0.3048) as i32;

    let vel_ecef = tracker.velocity_ecef();
    let speed_mps = (vel_ecef[0].powi(2) + vel_ecef[1].powi(2) + vel_ecef[2].powi(2)).sqrt();
    let speed_kts = speed_mps / 0.514444;

    // Matches the client interpolation: `predicting` follows the filter's own
    // speed, captured before the raw-vs-filter merge below.
    let predicting = speed_kts > MIN_PREDICTION_SPEED_KTS;

    let heading = compute_heading_from_ecef(lat, lon, &vel_ecef, speed_mps);

    let alt_ft = hint.and_then(|h| h.altitude_ft).unwrap_or(filter_alt_ft);
    let vrate = hint
        .and_then(|h| h.vertical_rate)
        .or_else(|| compute_vertical_rate(&vel_ecef, lat, lon));

    let is_coasting = quality.status == TrackStatus::Coasting;

    // During coasting the raw ADS-B values are stale (pre-gap); prefer the
    // filter's forward-propagated estimate. When confirmed, the freshest raw
    // observation wins.
    let heading = if is_coasting {
        heading
    } else {
        hint.and_then(|h| h.track_deg).or(heading)
    };
    let speed_kts = if is_coasting {
        speed_kts
    } else {
        hint.and_then(|h| h.velocity_kts).unwrap_or(speed_kts)
    };
    let lat = if is_coasting {
        lat
    } else {
        hint.and_then(|h| h.latitude).unwrap_or(lat)
    };
    let lon = if is_coasting {
        lon
    } else {
        hint.and_then(|h| h.longitude).unwrap_or(lon)
    };

    let squawk = hint.and_then(|h| h.squawk.clone());
    let is_on_ground = hint
        .and_then(|h| h.is_on_ground)
        .or(Some(track.is_on_ground));
    let alert = hint.and_then(|h| h.alert);
    let emergency = hint.and_then(|h| h.emergency);
    let spi = hint.and_then(|h| h.spi);
    let roll_angle = hint.and_then(|h| h.roll_angle);
    let track_angle_rate = hint.and_then(|h| h.track_angle_rate);

    let icao = track
        .cooperative_ids
        .iter()
        .find(|id| id.id_type == IdentifierType::Icao)
        .map(|id| id.id.clone())
        .unwrap_or_else(|| format!("TRK-{}", &track.id.0.to_string()[..8]));

    let callsign = hint
        .and_then(|h| h.callsign.clone())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            track
                .cooperative_ids
                .iter()
                .find(|id| id.id_type == IdentifierType::Callsign)
                .map(|id| id.id.clone())
        });

    let mode = tracker.mode_info();

    DisplayTrack {
        track_id: track.id.clone(),
        icao,
        callsign,
        latitude: lat,
        longitude: lon,
        altitude_ft: Some(alt_ft),
        heading: heading.map(|h| h as f32),
        velocity_kts: Some(speed_kts),
        vertical_rate: vrate,
        roll_angle,
        track_angle_rate,
        squawk,
        is_on_ground,
        alert,
        emergency,
        spi,
        last_seen: track.last_update,
        status: quality.status,
        position_source,
        h_uncertainty_m: horizontal_uncertainty_m(tracker),
        predicting,
        filter_type: filter_type_label(tracker).to_string(),
        mode_probabilities: mode.as_ref().map(|m| m.probabilities.clone()),
        dominant_mode: mode.as_ref().map(|m| m.dominant_mode),
        observation_count: quality.observation_count,
    }
}

fn compute_heading_from_ecef(
    lat_deg: f64,
    lon_deg: f64,
    vel_ecef: &[f64; 3],
    speed_mps: f64,
) -> Option<f64> {
    if speed_mps < 1.0 {
        return None;
    }

    let lat_rad = lat_deg.to_radians();
    let lon_rad = lon_deg.to_radians();

    let sin_lat = lat_rad.sin();
    let cos_lat = lat_rad.cos();
    let sin_lon = lon_rad.sin();
    let cos_lon = lon_rad.cos();

    // ECEF to ENU rotation
    let ve = -sin_lon * vel_ecef[0] + cos_lon * vel_ecef[1];
    let vn =
        -sin_lat * cos_lon * vel_ecef[0] - sin_lat * sin_lon * vel_ecef[1] + cos_lat * vel_ecef[2];

    let heading = ve.atan2(vn).to_degrees();
    Some(((heading % 360.0) + 360.0) % 360.0)
}

fn compute_vertical_rate(vel_ecef: &[f64; 3], lat_deg: f64, lon_deg: f64) -> Option<i32> {
    let lat_rad = lat_deg.to_radians();
    let lon_rad = lon_deg.to_radians();

    let sin_lat = lat_rad.sin();
    let cos_lat = lat_rad.cos();
    let sin_lon = lon_rad.sin();
    let cos_lon = lon_rad.cos();

    let vu = cos_lat * cos_lon * vel_ecef[0]
        + cos_lat * sin_lon * vel_ecef[1]
        + sin_lat * vel_ecef[2];

    let vr_fpm = vu / 0.00508;
    if vr_fpm.abs() > 0.1 {
        Some(vr_fpm as i32)
    } else {
        None
    }
}

/// 1-sigma horizontal position uncertainty in meters, reduced from the filter's
/// ECEF covariance to the local ENU frame. `None` when the covariance has no
/// position block.
fn horizontal_uncertainty_m(tracker: &TrackerState) -> Option<f64> {
    let cov = tracker.variant.covariance_mat();
    if cov.nrows() < 3 {
        return None;
    }

    let (lat, lon, _) = tracker.position_geodetic();
    let lat_rad = lat.to_radians();
    let lon_rad = lon.to_radians();

    let sin_lat = lat_rad.sin();
    let cos_lat = lat_rad.cos();
    let sin_lon = lon_rad.sin();
    let cos_lon = lon_rad.cos();

    let pos_cov = cov.view((0, 0), (3, 3));

    let var_east = sin_lon * sin_lon * pos_cov[(0, 0)] + cos_lon * cos_lon * pos_cov[(1, 1)]
        - 2.0 * sin_lon * cos_lon * pos_cov[(0, 1)];

    let var_north = (sin_lat * cos_lon).powi(2) * pos_cov[(0, 0)]
        + (sin_lat * sin_lon).powi(2) * pos_cov[(1, 1)]
        + cos_lat.powi(2) * pos_cov[(2, 2)]
        + 2.0 * sin_lat.powi(2) * sin_lon * cos_lon * pos_cov[(0, 1)]
        - 2.0 * sin_lat * cos_lat * cos_lon * pos_cov[(0, 2)]
        - 2.0 * sin_lat * cos_lat * sin_lon * pos_cov[(1, 2)];

    Some((var_east.abs() + var_north.abs()).sqrt())
}
