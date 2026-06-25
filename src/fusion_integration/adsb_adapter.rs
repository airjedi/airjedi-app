use airjedi_fusion::coord::CoordinateFrame;
use airjedi_fusion::nalgebra;
use airjedi_fusion::sensor::*;
use airjedi_fusion::systems::ObservationBuffer;
use airjedi_fusion::types::*;
use bevy::prelude::*;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use crate::adsb::connection::AdsbAircraftData;

/// Tracks the last-pushed state per ICAO to avoid sending stale observations.
pub(crate) struct LastPushedState {
    last_seen: DateTime<Utc>,
    lat: f64,
    lon: f64,
}

pub fn adsb_to_fusion_system(
    adsb_data: Option<Res<AdsbAircraftData>>,
    mut buffer: ResMut<ObservationBuffer>,
    mut last_pushed: Local<HashMap<String, LastPushedState>>,
) {
    let Some(adsb_data) = adsb_data else {
        return;
    };

    let aircraft_list = match adsb_data.aircraft.try_lock() {
        Ok(list) => list,
        Err(_) => return,
    };

    for ac in aircraft_list.iter() {
        let (Some(lat), Some(lon)) = (ac.latitude, ac.longitude) else {
            continue;
        };

        if let Some(prev) = last_pushed.get(&ac.icao) {
            if prev.last_seen == ac.last_seen {
                continue;
            }
            if (prev.lat - lat).abs() < f64::EPSILON && (prev.lon - lon).abs() < f64::EPSILON {
                continue;
            }
        }
        last_pushed.insert(
            ac.icao.clone(),
            LastPushedState {
                last_seen: ac.last_seen,
                lat,
                lon,
            },
        );

        if let Some(obs) = adsb_aircraft_to_observation(ac, lat, lon) {
            buffer.observations.push(obs);
        }
    }

    // Clean up stale entries for aircraft no longer in the list
    if last_pushed.len() > aircraft_list.len() * 2 {
        let active: std::collections::HashSet<&str> =
            aircraft_list.iter().map(|ac| ac.icao.as_str()).collect();
        last_pushed.retain(|icao, _| active.contains(icao.as_str()));
    }
}

fn adsb_aircraft_to_observation(
    ac: &adsb_client::Aircraft,
    lat: f64,
    lon: f64,
) -> Option<SensorObservation> {
    let alt_m = ac.altitude.map(|a| f64::from(a) * 0.3048);

    let (vel_north, vel_east) = match (ac.track, ac.velocity) {
        (Some(track_deg), Some(speed_kts)) => {
            let speed_mps = speed_kts * 0.514444;
            let track_rad = track_deg.to_radians();
            (
                Some(speed_mps * track_rad.cos()),
                Some(speed_mps * track_rad.sin()),
            )
        }
        _ => (None, None),
    };

    let vel_down = ac.vertical_rate.map(|vr| f64::from(-vr) * 0.00508);

    let pos_var = 10_000.0_f64; // 100m sigma squared
    let vel_var = 100.0_f64; // 10 m/s sigma squared
    let cov = nalgebra::DMatrix::from_diagonal(&nalgebra::DVector::from_vec(vec![
        pos_var, pos_var, pos_var, vel_var, vel_var, vel_var,
    ]));

    Some(SensorObservation {
        sensor_id: SensorId {
            id: "adsb-primary".to_string(),
            kind: SensorKind::AdsbReceiver,
            tier: FusionTier::Regional,
            coordinate_frame: CoordinateFrame::Wgs84,
        },
        timestamp: ac.last_seen,
        receipt_time: Utc::now(),
        target_id: Some(TargetId {
            domain: TargetDomain::Air,
            id: ac.icao.clone(),
            id_type: IdentifierType::Icao,
        }),
        measurement: Measurement::PositionVelocity3D {
            lat_deg: lat,
            lon_deg: lon,
            alt_m,
            vel_north_mps: vel_north,
            vel_east_mps: vel_east,
            vel_down_mps: vel_down,
            heading_deg: ac.track,
        },
        covariance: ObservationCovariance { matrix: cov },
        classification_hint: Some(TargetCategory::FixedWing),
        metadata: ObservationMetadata {
            source_label: "ADS-B SBS1".to_string(),
            is_on_ground: ac.is_on_ground,
            ..Default::default()
        },
    })
}
