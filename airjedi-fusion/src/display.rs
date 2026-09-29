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

use crate::sensor::{Measurement, SensorObservation};
use crate::{
    IdentifierType, StateVectorType, TimelineStore, Track, TrackQuality, TrackStatus, TrackerState,
};
use airjedi_core::{
    AltitudeReference, DisplayProvenance, DisplayTrack, DisplayValueSource, FieldFreshness,
    FieldProvenance, HeadingReference, ObservationFreshness, ObservationIdentity, PositionSource,
    RawOverride, TimeSourceQuality, Timestamp, VerticalRateReference,
};
use chrono::Duration;

/// Below this ground speed the client should not dead-reckon between updates.
/// Owned here so `DisplayTrack.predicting` and the client interpolation agree.
pub const MIN_PREDICTION_SPEED_KTS: f64 = 10.0;

/// A raw report remains eligible to override the fused state for this long
/// after receipt. Measurement time is retained separately for history and
/// provenance; receipt time determines whether the report is current enough to
/// override a state that has already advanced.
pub const RAW_HINT_FRESHNESS_SECS: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationStamp {
    pub sensor_id: String,
    pub observation_time: Timestamp,
    pub receipt_time: Timestamp,
    pub time_source: Option<TimeSourceQuality>,
    pub observation_id: Option<ObservationIdentity>,
}

impl ObservationStamp {
    fn from_observation(observation: &SensorObservation) -> Self {
        Self {
            sensor_id: observation.sensor_id.id.clone(),
            observation_time: observation.timestamp,
            receipt_time: observation.receipt_time,
            time_source: observation.metadata.time_source,
            observation_id: observation.metadata.observation_id,
        }
    }

    fn is_newer_than(&self, other: &Self) -> bool {
        if self.observation_time != other.observation_time {
            return self.observation_time > other.observation_time;
        }
        if self.receipt_time != other.receipt_time {
            return self.receipt_time > other.receipt_time;
        }
        if self.observation_id != other.observation_id {
            return observation_id_is_newer(
                self.observation_id.as_ref(),
                other.observation_id.as_ref(),
            );
        }
        self.sensor_id > other.sensor_id
    }

    fn is_fresh_at(&self, state_time: Timestamp) -> bool {
        state_time.signed_duration_since(self.receipt_time)
            <= Duration::seconds(RAW_HINT_FRESHNESS_SECS)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimestampedRaw<T> {
    pub value: T,
    pub stamp: ObservationStamp,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawAltitude {
    pub feet: i32,
    pub reference: AltitudeReference,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawDirection {
    pub degrees: f64,
    pub reference: HeadingReference,
}

/// Timestamped raw-observation values that may override the filter when they
/// are current for the track state. Both embedded and headless callers build
/// this through [`raw_observation_hint_for`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawObservationHint {
    pub altitude: Option<TimestampedRaw<RawAltitude>>,
    pub vertical_rate: Option<TimestampedRaw<i32>>,
    pub track: Option<TimestampedRaw<RawDirection>>,
    pub ground_speed_kts: Option<TimestampedRaw<f64>>,
    pub airspeed_kts: Option<TimestampedRaw<f64>>,
    pub latitude: Option<TimestampedRaw<f64>>,
    pub longitude: Option<TimestampedRaw<f64>>,
    pub squawk: Option<TimestampedRaw<String>>,
    pub is_on_ground: Option<TimestampedRaw<bool>>,
    pub alert: Option<TimestampedRaw<bool>>,
    pub emergency: Option<TimestampedRaw<bool>>,
    pub spi: Option<TimestampedRaw<bool>>,
    pub roll_angle: Option<TimestampedRaw<f32>>,
    pub track_angle_rate: Option<TimestampedRaw<f32>>,
    pub callsign: Option<TimestampedRaw<String>>,
    pub position_freshness: Option<ObservationFreshness>,
    pub altitude_freshness: Option<ObservationFreshness>,
    pub velocity_freshness: Option<ObservationFreshness>,
}

impl RawObservationHint {
    #[must_use]
    pub fn from_observation(observation: &SensorObservation) -> Self {
        let stamp = ObservationStamp::from_observation(observation);
        let mut hint = Self::default();
        let field_freshness_known = observation.has_field_freshness();

        match &observation.measurement {
            Measurement::PositionVelocity3D {
                lat_deg,
                lon_deg,
                alt_m,
                vel_north_mps,
                vel_east_mps,
                vel_down_mps,
                heading_deg,
            } => {
                if !field_freshness_known || observation.metadata.position_freshness.is_some() {
                    hint.latitude = Some(timestamped(*lat_deg, &stamp));
                    hint.longitude = Some(timestamped(*lon_deg, &stamp));
                }
                if !field_freshness_known || observation.metadata.altitude_freshness.is_some() {
                    if let Some(alt_m) = alt_m {
                        hint.altitude = Some(timestamped(
                            RawAltitude {
                                feet: (alt_m / 0.3048) as i32,
                                reference: observation
                                    .metadata
                                    .altitude_reference
                                    .unwrap_or(AltitudeReference::Unknown),
                            },
                            &stamp,
                        ));
                    }
                }
                if !field_freshness_known || observation.metadata.velocity_freshness.is_some() {
                    if let (Some(north), Some(east)) = (vel_north_mps, vel_east_mps) {
                        hint.ground_speed_kts =
                            Some(timestamped((north.hypot(*east)) / 0.514444, &stamp));
                    }
                }
                if !field_freshness_known || observation.metadata.velocity_freshness.is_some() {
                    if let Some(heading) = heading_deg {
                        hint.track = Some(timestamped(
                            RawDirection {
                                degrees: *heading,
                                reference: observation
                                    .metadata
                                    .heading_reference
                                    .unwrap_or(HeadingReference::GroundTrack),
                            },
                            &stamp,
                        ));
                    }
                }
                if !field_freshness_known || observation.metadata.velocity_freshness.is_some() {
                    if let Some(rate) = observation
                        .metadata
                        .vertical_rate_fpm
                        .or_else(|| vel_down_mps.map(|down| (-down / 0.00508) as i32))
                    {
                        hint.vertical_rate = Some(timestamped(rate, &stamp));
                    }
                }
            }
            Measurement::PositionVelocity2D {
                lat_deg,
                lon_deg,
                speed_over_ground_mps,
                course_over_ground_deg,
            } => {
                if !field_freshness_known || observation.metadata.position_freshness.is_some() {
                    hint.latitude = Some(timestamped(*lat_deg, &stamp));
                    hint.longitude = Some(timestamped(*lon_deg, &stamp));
                }
                if !field_freshness_known || observation.metadata.velocity_freshness.is_some() {
                    if let Some(speed) = speed_over_ground_mps {
                        hint.ground_speed_kts = Some(timestamped(*speed / 0.514444, &stamp));
                    }
                }
                if !field_freshness_known || observation.metadata.velocity_freshness.is_some() {
                    if let Some(course) = course_over_ground_deg {
                        hint.track = Some(timestamped(
                            RawDirection {
                                degrees: *course,
                                reference: HeadingReference::GroundTrack,
                            },
                            &stamp,
                        ));
                    }
                }
            }
            Measurement::Spherical { .. }
            | Measurement::BearingOnly { .. }
            | Measurement::DepthBearing { .. }
            | Measurement::FusedEstimate { .. } => {}
        }

        hint.airspeed_kts = observation
            .metadata
            .airspeed_kts
            .map(|value| timestamped(value, &stamp));
        hint.is_on_ground = observation
            .metadata
            .is_on_ground
            .map(|value| timestamped(value, &stamp));
        hint.alert = observation
            .metadata
            .alert
            .map(|value| timestamped(value, &stamp));
        hint.emergency = observation
            .metadata
            .emergency
            .map(|value| timestamped(value, &stamp));
        hint.spi = observation
            .metadata
            .spi
            .map(|value| timestamped(value, &stamp));
        hint.squawk = observation
            .metadata
            .squawk
            .clone()
            .map(|value| timestamped(value, &stamp));
        hint.callsign = observation
            .metadata
            .callsign
            .clone()
            .map(|value| timestamped(value, &stamp));
        hint.roll_angle = observation
            .metadata
            .roll_angle
            .map(|value| timestamped(value, &stamp));
        hint.track_angle_rate = observation
            .metadata
            .track_angle_rate
            .map(|value| timestamped(value, &stamp));
        hint.position_freshness = observation.metadata.position_freshness;
        hint.altitude_freshness = observation.metadata.altitude_freshness;
        hint.velocity_freshness = observation.metadata.velocity_freshness;

        hint
    }

    fn merge(&mut self, other: Self) {
        merge_timestamped(&mut self.altitude, other.altitude);
        merge_timestamped(&mut self.vertical_rate, other.vertical_rate);
        merge_timestamped(&mut self.track, other.track);
        merge_timestamped(&mut self.ground_speed_kts, other.ground_speed_kts);
        merge_timestamped(&mut self.airspeed_kts, other.airspeed_kts);
        merge_timestamped(&mut self.latitude, other.latitude);
        merge_timestamped(&mut self.longitude, other.longitude);
        merge_timestamped(&mut self.squawk, other.squawk);
        merge_timestamped(&mut self.is_on_ground, other.is_on_ground);
        merge_timestamped(&mut self.alert, other.alert);
        merge_timestamped(&mut self.emergency, other.emergency);
        merge_timestamped(&mut self.spi, other.spi);
        merge_timestamped(&mut self.roll_angle, other.roll_angle);
        merge_timestamped(&mut self.track_angle_rate, other.track_angle_rate);
        merge_timestamped(&mut self.callsign, other.callsign);
        merge_freshness(&mut self.position_freshness, other.position_freshness);
        merge_freshness(&mut self.altitude_freshness, other.altitude_freshness);
        merge_freshness(&mut self.velocity_freshness, other.velocity_freshness);
    }

    fn has_values(&self) -> bool {
        self.altitude.is_some()
            || self.vertical_rate.is_some()
            || self.track.is_some()
            || self.ground_speed_kts.is_some()
            || self.airspeed_kts.is_some()
            || self.latitude.is_some()
            || self.longitude.is_some()
            || self.squawk.is_some()
            || self.is_on_ground.is_some()
            || self.alert.is_some()
            || self.emergency.is_some()
            || self.spi.is_some()
            || self.roll_angle.is_some()
            || self.track_angle_rate.is_some()
            || self.callsign.is_some()
    }

    #[must_use]
    pub fn has_field_freshness(&self) -> bool {
        self.position_freshness.is_some()
            || self.altitude_freshness.is_some()
            || self.velocity_freshness.is_some()
    }
}

/// Build the same raw hint in embedded and headless modes from the shared
/// timestamped observation store. Unassociated observations are included while
/// their target identity is being promoted into a track.
#[must_use]
pub fn raw_observation_hint_for(
    store: &TimelineStore,
    track: &Track,
) -> Option<RawObservationHint> {
    let mut hint = RawObservationHint::default();
    for stored in store.observations_for_track(&track.id, &track.cooperative_ids) {
        hint.merge(RawObservationHint::from_observation(&stored.observation));
    }
    hint.has_values().then_some(hint)
}

fn timestamped<T>(value: T, stamp: &ObservationStamp) -> TimestampedRaw<T> {
    TimestampedRaw {
        value,
        stamp: stamp.clone(),
    }
}

fn merge_timestamped<T>(
    slot: &mut Option<TimestampedRaw<T>>,
    candidate: Option<TimestampedRaw<T>>,
) {
    let Some(candidate) = candidate else { return };
    if slot
        .as_ref()
        .is_none_or(|current| candidate.stamp.is_newer_than(&current.stamp))
    {
        *slot = Some(candidate);
    }
}

fn merge_freshness(
    slot: &mut Option<ObservationFreshness>,
    candidate: Option<ObservationFreshness>,
) {
    let Some(candidate) = candidate else { return };
    if slot.as_ref().is_none_or(|current| {
        candidate.observation_time > current.observation_time
            || (candidate.observation_time == current.observation_time
                && (candidate.receipt_time > current.receipt_time
                    || (candidate.receipt_time == current.receipt_time
                        && observation_id_is_newer(
                            Some(&candidate.identity),
                            Some(&current.identity),
                        ))))
    }) {
        *slot = Some(candidate);
    }
}

fn observation_id_is_newer(
    current: Option<&ObservationIdentity>,
    previous: Option<&ObservationIdentity>,
) -> bool {
    match (current, previous) {
        (Some(current), Some(previous)) => {
            (current.frame_sequence, current.payload_index)
                > (previous.frame_sequence, previous.payload_index)
        }
        (Some(_), None) => true,
        _ => false,
    }
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
/// freshness-aware raw/filter merge policy and pre-reduces the covariance to a
/// scalar horizontal uncertainty so nothing un-serializable crosses the
/// boundary. `Track::last_update` is the shared T1 state time used by both the
/// embedded and headless callers.
#[must_use]
pub fn derive_display_track(
    track: &Track,
    tracker: &TrackerState,
    quality: &TrackQuality,
    hint: Option<&RawObservationHint>,
    position_source: Option<PositionSource>,
) -> DisplayTrack {
    let (filter_lat, filter_lon, alt_m) = tracker.position_geodetic();
    let filter_alt_ft = (alt_m / 0.3048) as i32;

    let vel_ecef = tracker.velocity_ecef();
    let (east_mps, north_mps, _up_mps) = ecef_velocity_to_enu(&vel_ecef, filter_lat, filter_lon);
    let filter_ground_speed_kts = east_mps.hypot(north_mps) / 0.514444;
    let filter_track = if east_mps.hypot(north_mps) >= 1.0 {
        Some(east_mps.atan2(north_mps).to_degrees().rem_euclid(360.0))
    } else {
        None
    };
    let filter_vertical_rate = compute_vertical_rate(&vel_ecef, filter_lat, filter_lon);
    let is_coasting = quality.status == TrackStatus::Coasting;
    let state_time = track.last_update;

    let (raw_lat, lat_override) = select_raw(
        hint.and_then(|h| h.latitude.as_ref()),
        state_time,
        is_coasting,
    );
    let (raw_lon, lon_override) = select_raw(
        hint.and_then(|h| h.longitude.as_ref()),
        state_time,
        is_coasting,
    );
    let (raw_altitude, altitude_override) = select_raw(
        hint.and_then(|h| h.altitude.as_ref()),
        state_time,
        is_coasting,
    );
    let (raw_track, heading_override) =
        select_raw(hint.and_then(|h| h.track.as_ref()), state_time, is_coasting);
    let (raw_ground_speed, ground_speed_override) = select_raw(
        hint.and_then(|h| h.ground_speed_kts.as_ref()),
        state_time,
        is_coasting,
    );
    let (raw_airspeed, airspeed_override) = select_raw(
        hint.and_then(|h| h.airspeed_kts.as_ref()),
        state_time,
        is_coasting,
    );
    let (raw_vertical_rate, vertical_rate_override) = select_raw(
        hint.and_then(|h| h.vertical_rate.as_ref()),
        state_time,
        is_coasting,
    );

    let position_raw = raw_lat.is_some() && raw_lon.is_some();
    let position_override = if position_raw {
        RawOverride::Applied
    } else if lat_override == RawOverride::IgnoredStale || lon_override == RawOverride::IgnoredStale
    {
        RawOverride::IgnoredStale
    } else {
        RawOverride::Unavailable
    };

    let (lat, lon) = if position_raw {
        (raw_lat.unwrap().value, raw_lon.unwrap().value)
    } else {
        (filter_lat, filter_lon)
    };

    let altitude_ft = raw_altitude
        .map(|raw| raw.value.feet)
        .unwrap_or(filter_alt_ft);
    let altitude_reference = hint
        .and_then(|h| h.altitude.as_ref())
        .map(|raw| raw.value.reference)
        .unwrap_or(AltitudeReference::Unknown);

    let heading = raw_track.map(|raw| raw.value.degrees).or(filter_track);
    let heading_reference = raw_track.map(|raw| raw.value.reference).unwrap_or_else(|| {
        filter_track
            .map(|_| HeadingReference::GroundTrack)
            .unwrap_or(HeadingReference::Unknown)
    });

    let ground_speed_kts = raw_ground_speed
        .map(|raw| raw.value)
        .unwrap_or(filter_ground_speed_kts);
    let airspeed_kts = raw_airspeed.map(|raw| raw.value);
    let vertical_rate = raw_vertical_rate
        .map(|raw| raw.value)
        .or(filter_vertical_rate);
    let vertical_rate_reference = if vertical_rate.is_some() {
        VerticalRateReference::FeetPerMinute
    } else {
        VerticalRateReference::Unknown
    };

    let (raw_is_on_ground, _) = select_raw(
        hint.and_then(|h| h.is_on_ground.as_ref()),
        state_time,
        is_coasting,
    );
    let is_on_ground = raw_is_on_ground
        .map(|raw| raw.value)
        .or(Some(track.is_on_ground));

    let (raw_squawk, _) = select_raw(
        hint.and_then(|h| h.squawk.as_ref()),
        state_time,
        is_coasting,
    );
    let (raw_alert, _) = select_raw(hint.and_then(|h| h.alert.as_ref()), state_time, is_coasting);
    let (raw_emergency, _) = select_raw(
        hint.and_then(|h| h.emergency.as_ref()),
        state_time,
        is_coasting,
    );
    let (raw_spi, _) = select_raw(hint.and_then(|h| h.spi.as_ref()), state_time, is_coasting);
    let (raw_roll, _) = select_raw(
        hint.and_then(|h| h.roll_angle.as_ref()),
        state_time,
        is_coasting,
    );
    let (raw_track_rate, _) = select_raw(
        hint.and_then(|h| h.track_angle_rate.as_ref()),
        state_time,
        is_coasting,
    );
    let (raw_callsign, _) = select_raw(
        hint.and_then(|h| h.callsign.as_ref()),
        state_time,
        is_coasting,
    );

    let squawk = raw_squawk.map(|raw| raw.value.clone());
    let alert = raw_alert.map(|raw| raw.value);
    let emergency = raw_emergency.map(|raw| raw.value);
    let spi = raw_spi.map(|raw| raw.value);
    let roll_angle = raw_roll.map(|raw| raw.value);
    let track_angle_rate = raw_track_rate.map(|raw| raw.value);

    // Client dead reckoning must use horizontal ground track and ground speed,
    // never airspeed or a vertical component of the fused velocity.
    let prediction_heading = (heading_reference == HeadingReference::GroundTrack)
        .then_some(heading)
        .flatten();
    let predicting = is_on_ground != Some(true)
        && prediction_heading.is_some()
        && ground_speed_kts > MIN_PREDICTION_SPEED_KTS;

    let icao = track
        .cooperative_ids
        .iter()
        .find(|id| id.id_type == IdentifierType::Icao)
        .map(|id| id.id.clone())
        .unwrap_or_else(|| format!("TRK-{}", &track.id.0.to_string()[..8]));

    let callsign = raw_callsign
        .map(|raw| raw.value.clone())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            track
                .cooperative_ids
                .iter()
                .find(|id| id.id_type == IdentifierType::Callsign)
                .map(|id| id.id.clone())
        });

    let mode = tracker.mode_info();

    let last_seen = [
        Some(track.last_update),
        hint.and_then(|h| h.position_freshness.map(|f| f.receipt_time)),
        hint.and_then(|h| h.altitude_freshness.map(|f| f.receipt_time)),
        hint.and_then(|h| h.velocity_freshness.map(|f| f.receipt_time)),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(track.last_update);
    let fused_source = if is_coasting {
        DisplayValueSource::PredictedEstimate
    } else {
        DisplayValueSource::FusedEstimate
    };
    let fused_freshness = if is_coasting {
        FieldFreshness::Stale
    } else {
        FieldFreshness::Fresh
    };
    let provenance = DisplayProvenance {
        position: provenance_for(
            hint.and_then(|h| h.latitude.as_ref()),
            position_raw,
            true,
            fused_source,
            fused_freshness,
            position_override,
        ),
        altitude: provenance_for(
            hint.and_then(|h| h.altitude.as_ref()),
            raw_altitude.is_some(),
            true,
            fused_source,
            fused_freshness,
            altitude_override,
        ),
        ground_speed: provenance_for(
            hint.and_then(|h| h.ground_speed_kts.as_ref()),
            raw_ground_speed.is_some(),
            true,
            fused_source,
            fused_freshness,
            ground_speed_override,
        ),
        airspeed: provenance_for(
            hint.and_then(|h| h.airspeed_kts.as_ref()),
            raw_airspeed.is_some(),
            airspeed_kts.is_some(),
            fused_source,
            fused_freshness,
            airspeed_override,
        ),
        vertical_rate: provenance_for(
            hint.and_then(|h| h.vertical_rate.as_ref()),
            raw_vertical_rate.is_some(),
            vertical_rate.is_some(),
            fused_source,
            fused_freshness,
            vertical_rate_override,
        ),
        heading: provenance_for(
            hint.and_then(|h| h.track.as_ref()),
            raw_track.is_some(),
            heading.is_some(),
            fused_source,
            fused_freshness,
            heading_override,
        ),
    };

    DisplayTrack {
        track_id: track.id.clone(),
        icao,
        callsign,
        latitude: lat,
        longitude: lon,
        altitude_ft: Some(altitude_ft),
        position_freshness: hint.and_then(|h| h.position_freshness),
        altitude_freshness: hint.and_then(|h| h.altitude_freshness),
        velocity_freshness: hint.and_then(|h| h.velocity_freshness),
        altitude_reference,
        heading: heading.map(|h| h as f32),
        heading_reference,
        velocity_kts: Some(ground_speed_kts),
        airspeed_kts,
        vertical_rate,
        vertical_rate_reference,
        roll_angle,
        track_angle_rate,
        squawk,
        is_on_ground,
        alert,
        emergency,
        spi,
        last_seen,
        status: quality.status,
        position_source,
        h_uncertainty_m: horizontal_uncertainty_m(tracker),
        predicting,
        filter_type: filter_type_label(tracker).to_string(),
        mode_probabilities: mode.as_ref().map(|m| m.probabilities.clone()),
        dominant_mode: mode.as_ref().map(|m| m.dominant_mode),
        observation_count: quality.observation_count,
        provenance,
    }
}

fn ecef_velocity_to_enu(vel_ecef: &[f64; 3], lat_deg: f64, lon_deg: f64) -> (f64, f64, f64) {
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
    let vu =
        cos_lat * cos_lon * vel_ecef[0] + cos_lat * sin_lon * vel_ecef[1] + sin_lat * vel_ecef[2];
    (ve, vn, vu)
}

fn compute_vertical_rate(vel_ecef: &[f64; 3], lat_deg: f64, lon_deg: f64) -> Option<i32> {
    let (_, _, vu) = ecef_velocity_to_enu(vel_ecef, lat_deg, lon_deg);

    let vr_fpm = vu / 0.00508;
    if vr_fpm.abs() > 0.1 {
        Some(vr_fpm as i32)
    } else {
        None
    }
}

fn select_raw<'a, T>(
    raw: Option<&'a TimestampedRaw<T>>,
    state_time: Timestamp,
    is_coasting: bool,
) -> (Option<&'a TimestampedRaw<T>>, RawOverride) {
    match raw {
        None => (None, RawOverride::Unavailable),
        Some(raw) if is_coasting || !raw.stamp.is_fresh_at(state_time) => {
            (None, RawOverride::IgnoredStale)
        }
        Some(raw) => (Some(raw), RawOverride::Applied),
    }
}

fn provenance_for<T>(
    raw: Option<&TimestampedRaw<T>>,
    raw_selected: bool,
    has_value: bool,
    fused_source: DisplayValueSource,
    fused_freshness: FieldFreshness,
    raw_override: RawOverride,
) -> FieldProvenance {
    let (source, freshness) = if raw_selected {
        (DisplayValueSource::RawObservation, FieldFreshness::Fresh)
    } else if has_value {
        (fused_source, fused_freshness)
    } else {
        (DisplayValueSource::Unknown, FieldFreshness::Unknown)
    };
    let stamp = raw.map(|value| &value.stamp);
    FieldProvenance {
        source,
        freshness,
        raw_override,
        observation_time: stamp.map(|value| value.observation_time),
        receipt_time: stamp.map(|value| value.receipt_time),
        time_source: stamp.and_then(|value| value.time_source),
        observation_id: stamp.and_then(|value| value.observation_id),
        sensor_id: stamp.map(|value| value.sensor_id.clone()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::CoordinateFrame;
    use crate::sensor::{
        FusionTier, Measurement, ObservationCovariance, ObservationMetadata, SensorId, SensorKind,
        SensorObservation,
    };
    use crate::types::{IdentifierType, TargetCategory, TargetDomain, TargetId, TrackId};
    use crate::{FusionConfig, Track};
    use airjedi_core::{ObservationFreshness, ObservationIdentity, TimeSourceQuality};
    use chrono::Utc;
    use nalgebra::DMatrix;

    fn freshness(frame_sequence: u64) -> ObservationFreshness {
        let time = Utc::now();
        ObservationFreshness {
            observation_time: time,
            receipt_time: time,
            time_source: TimeSourceQuality::ReceiptTime,
            identity: ObservationIdentity {
                frame_sequence,
                payload_index: 0,
            },
        }
    }

    #[test]
    fn telemetry_only_hint_updates_telemetry_without_overwriting_position() {
        let observation = SensorObservation {
            sensor_id: SensorId {
                id: "test-adsb".to_string(),
                kind: SensorKind::AdsbReceiver,
                tier: FusionTier::Regional,
                coordinate_frame: CoordinateFrame::Wgs84,
            },
            timestamp: Utc::now(),
            receipt_time: Utc::now(),
            target_id: Some(TargetId {
                domain: TargetDomain::Air,
                id: "ABC123".to_string(),
                id_type: IdentifierType::Icao,
            }),
            measurement: Measurement::PositionVelocity3D {
                lat_deg: 37.0,
                lon_deg: -97.0,
                alt_m: Some(10_000.0),
                vel_north_mps: Some(100.0),
                vel_east_mps: Some(0.0),
                vel_down_mps: Some(0.0),
                heading_deg: Some(0.0),
            },
            covariance: ObservationCovariance {
                matrix: DMatrix::identity(6, 6) * 100.0,
            },
            classification_hint: Some(TargetCategory::FixedWing),
            metadata: ObservationMetadata::default(),
        };
        let mut tracker = FusionConfig::default().create_tracker(&TargetCategory::FixedWing);
        tracker.variant.initialize(&observation);
        let expected_position = tracker.position_geodetic();
        let track = Track {
            id: TrackId::new(),
            cooperative_ids: vec![observation.target_id.clone().unwrap()],
            created_at: observation.timestamp,
            last_update: observation.timestamp,
            is_on_ground: false,
        };
        let quality = TrackQuality {
            status: TrackStatus::Confirmed,
            ..Default::default()
        };
        let stamp = ObservationStamp {
            sensor_id: "test-adsb".to_string(),
            observation_time: observation.timestamp,
            receipt_time: observation.receipt_time,
            time_source: Some(TimeSourceQuality::ReceiptTime),
            observation_id: Some(ObservationIdentity {
                frame_sequence: 1,
                payload_index: 0,
            }),
        };
        let hint = RawObservationHint {
            altitude: Some(TimestampedRaw {
                value: RawAltitude {
                    feet: 35_000,
                    reference: AltitudeReference::Barometric,
                },
                stamp: stamp.clone(),
            }),
            ground_speed_kts: Some(TimestampedRaw {
                value: 250.0,
                stamp,
            }),
            position_freshness: None,
            altitude_freshness: Some(freshness(2)),
            velocity_freshness: Some(freshness(3)),
            ..Default::default()
        };

        let display = derive_display_track(&track, &tracker, &quality, Some(&hint), None);

        assert!((display.latitude - expected_position.0).abs() < 1e-6);
        assert!((display.longitude - expected_position.1).abs() < 1e-6);
        assert_eq!(display.altitude_ft, Some(35_000));
        assert_eq!(display.velocity_kts, Some(250.0));
        assert!(display.position_freshness.is_none());
        assert_eq!(
            display.altitude_freshness.unwrap().identity.frame_sequence,
            2
        );
        assert_eq!(
            display.velocity_freshness.unwrap().identity.frame_sequence,
            3
        );
    }
}
