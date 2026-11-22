//! Drone Remote ID (ASTM F3411 / EN 4709-002) as a fusion sensor source.
//!
//! Decoding is done by the `remoteid` crate; this module only maps a decoded
//! aircraft ([`remoteid::UasData`], filled from BLE, Wi-Fi Beacon/NAN or a
//! serial receiver) onto a [`SensorObservation`] keyed by
//! [`IdentifierType::RemoteId`].

use crate::sensor::SensorObservation;
use crate::sensor::{Measurement, ObservationCovariance, ObservationMetadata, SensorId};
use crate::types::{IdentifierType, TargetCategory, TargetDomain, TargetId, Timestamp};
use airjedi_core::{AltitudeReference, HeadingReference};
use nalgebra::{DMatrix, DVector};
use remoteid::{Location, UaType, UasData};

/// Position variance (m^2) when the transmitter declares no horizontal accuracy.
const POS_VAR_UNKNOWN: f64 = 10_000.0;
/// Velocity variance ((m/s)^2) when the transmitter declares no speed accuracy.
const VEL_VAR_UNKNOWN: f64 = 100.0;

/// Builds an observation from the current state of one Remote ID transmitter.
///
/// Returns `None` until both a Basic ID and a position have been heard, or
/// when the Location message fails the F3411 range checks.
#[must_use]
pub fn uas_to_observation(
    uas: &UasData,
    sensor_id: &SensorId,
    receipt_time: Timestamp,
) -> Option<SensorObservation> {
    let id = uas.primary_id()?;
    let loc = uas.location.as_ref()?;
    if !loc.is_plausible() {
        return None;
    }
    let (lat_deg, lon_deg) = loc.position()?;

    let (alt_m, altitude_reference) = match (loc.altitude_geo, loc.altitude_baro) {
        (Some(geo), _) => (Some(f64::from(geo)), Some(AltitudeReference::Geometric)),
        (None, Some(baro)) => (Some(f64::from(baro)), Some(AltitudeReference::Barometric)),
        (None, None) => (None, None),
    };

    let (vel_north_mps, vel_east_mps) = match (loc.direction, loc.speed_horizontal) {
        (Some(dir), Some(speed)) => {
            let (sin, cos) = f64::from(dir).to_radians().sin_cos();
            (Some(f64::from(speed) * cos), Some(f64::from(speed) * sin))
        }
        _ => (None, None),
    };

    let pos_var = loc
        .horizontal_accuracy
        .meters()
        .map_or(POS_VAR_UNKNOWN, |m| f64::from(m).powi(2));
    let alt_var = loc
        .vertical_accuracy
        .meters()
        .map_or(POS_VAR_UNKNOWN, |m| f64::from(m).powi(2));
    let vel_var = loc
        .speed_accuracy
        .meters_per_second()
        .map_or(VEL_VAR_UNKNOWN, |m| f64::from(m).powi(2));
    let cov = DMatrix::from_diagonal(&DVector::from_vec(vec![
        pos_var, pos_var, alt_var, vel_var, vel_var, vel_var,
    ]));

    let ua_type = uas
        .basic_id
        .iter()
        .flatten()
        .map(|b| b.ua_type)
        .find(|t| *t != UaType::None);

    Some(SensorObservation {
        sensor_id: sensor_id.clone(),
        timestamp: position_time(loc, receipt_time),
        receipt_time,
        target_id: Some(TargetId {
            domain: TargetDomain::Air,
            id: id.to_string(),
            id_type: IdentifierType::RemoteId,
        }),
        measurement: Measurement::PositionVelocity3D {
            lat_deg,
            lon_deg,
            alt_m,
            vel_north_mps,
            vel_east_mps,
            vel_down_mps: loc.speed_vertical.map(|v| -f64::from(v)),
            heading_deg: loc.direction.map(f64::from),
        },
        covariance: ObservationCovariance { matrix: cov },
        classification_hint: Some(category(ua_type)),
        metadata: ObservationMetadata {
            source_label: format!("Remote ID {id}"),
            altitude_reference,
            heading_reference: loc.direction.map(|_| HeadingReference::GroundTrack),
            #[allow(clippy::cast_possible_truncation)]
            vertical_rate_fpm: loc
                .speed_vertical
                .map(|v| (f64::from(v) * 196.850_394).round() as i32),
            ..Default::default()
        },
    })
}

/// Location timestamps are tenths of a second after the UTC hour; anchor them
/// to the hour of receipt, stepping back one hour across the boundary.
fn position_time(loc: &Location, receipt_time: Timestamp) -> Timestamp {
    use chrono::{DurationRound, TimeDelta};
    let Some(secs) = loc.timestamp else {
        return receipt_time;
    };
    let Ok(hour) = receipt_time.duration_trunc(TimeDelta::hours(1)) else {
        return receipt_time;
    };
    #[allow(clippy::cast_possible_truncation)]
    let t = hour + TimeDelta::milliseconds((f64::from(secs) * 1000.0).round() as i64);
    if t > receipt_time + TimeDelta::minutes(30) {
        t - TimeDelta::hours(1)
    } else {
        t
    }
}

fn category(ua_type: Option<UaType>) -> TargetCategory {
    match ua_type {
        Some(UaType::FreeBalloon | UaType::CaptiveBalloon | UaType::Airship) => {
            TargetCategory::Balloon
        }
        Some(UaType::Rocket) => TargetCategory::Rocket,
        _ => TargetCategory::Drone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::CoordinateFrame;
    use crate::sensor::{FusionTier, SensorKind};
    use chrono::{TimeZone, Utc};
    use remoteid::{BasicId, HorizontalAccuracy, IdType, Message, Status};

    fn sensor() -> SensorId {
        SensorId {
            id: "rid-dongle".to_string(),
            kind: SensorKind::RfTracker,
            tier: FusionTier::Edge,
            coordinate_frame: CoordinateFrame::Wgs84,
        }
    }

    /// Round-trips real F3411 frames through the decoder, as a receiver would.
    fn heard(loc: Location) -> UasData {
        let basic = BasicId::new(
            IdType::SerialNumber,
            UaType::HelicopterOrMultirotor,
            "1596F1234567890ABCDE",
        )
        .unwrap();
        let mut uas = UasData::default();
        for frame in [
            Message::BasicId(basic).encode().unwrap(),
            Message::Location(loc).encode().unwrap(),
        ] {
            assert!(uas.update(&Message::decode(&frame).unwrap()));
        }
        uas
    }

    #[test]
    fn remote_id_drone_becomes_fusion_observation() {
        let rx = Utc.with_ymd_and_hms(2026, 11, 22, 14, 0, 3).unwrap();
        let uas = heard(Location {
            status: Status::Airborne,
            direction: Some(90.0),
            speed_horizontal: Some(10.0),
            speed_vertical: Some(2.0),
            latitude: 37.687_2,
            longitude: -97.330_1,
            altitude_geo: Some(120.0),
            horizontal_accuracy: HorizontalAccuracy(10),
            timestamp: Some(3599.0),
            ..Location::default()
        });

        let obs = uas_to_observation(&uas, &sensor(), rx).unwrap();
        let target = obs.target_id.unwrap();
        assert_eq!(target.id_type, IdentifierType::RemoteId);
        assert_eq!(target.id, "1596F1234567890ABCDE");
        assert_eq!(obs.classification_hint, Some(TargetCategory::Drone));
        // 3599 s after the hour, heard 3 s into the next hour.
        assert_eq!(obs.timestamp, rx - chrono::TimeDelta::seconds(4));
        let Measurement::PositionVelocity3D {
            lat_deg,
            alt_m,
            vel_north_mps,
            vel_east_mps,
            vel_down_mps,
            ..
        } = obs.measurement
        else {
            panic!("expected 3D measurement");
        };
        assert!((lat_deg - 37.687_2).abs() < 1e-6);
        assert!((alt_m.unwrap() - 120.0).abs() < 0.5);
        assert!(vel_north_mps.unwrap().abs() < 1e-6);
        assert!((vel_east_mps.unwrap() - 10.0).abs() < 1e-6);
        assert!((vel_down_mps.unwrap() + 2.0).abs() < 1e-6);
        // Declared 10 m accuracy, not the ADS-B default.
        assert!((obs.covariance.matrix[(0, 0)] - 100.0).abs() < 1e-6);
    }

    #[test]
    fn unknown_position_is_not_a_track() {
        let uas = heard(Location {
            status: Status::Ground,
            ..Location::default()
        });
        assert!(uas_to_observation(&uas, &sensor(), Utc::now()).is_none());
    }
}
