use airjedi_fusion::coord::CoordinateFrame;
use airjedi_fusion::nalgebra;
use airjedi_fusion::sensor::*;
use airjedi_fusion::systems::ObservationBuffer;
use airjedi_fusion::types::*;
use bevy::prelude::*;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::time::Instant;

use crate::adsb::connection::FeedConnectionManager;

/// Tracks the last-pushed state per ICAO to avoid sending redundant observations.
pub(crate) struct LastPushedState {
    last_seen: DateTime<Utc>,
    lat: f64,
    lon: f64,
    pushed_at: Instant,
}

const MAX_PUSH_INTERVAL_SECS: u64 = 5;

pub fn adsb_to_fusion_system(
    feed_mgr: Option<Res<FeedConnectionManager>>,
    mut buffer: ResMut<ObservationBuffer>,
    mut last_pushed: Local<HashMap<String, LastPushedState>>,
) {
    let Some(feed_mgr) = feed_mgr else {
        return;
    };

    let mut seen_icaos = Vec::new();

    for (feed_name, conn) in &feed_mgr.connections {
        let aircraft_list = match conn.data.aircraft.try_lock() {
            Ok(list) => list,
            Err(_) => continue,
        };

        let source_label = format!("ADS-B {}", feed_name);
        let sensor_id_str = format!("adsb-{}", feed_name);

        for ac in aircraft_list.iter() {
            let (Some(lat), Some(lon)) = (ac.latitude, ac.longitude) else {
                continue;
            };

            seen_icaos.push(ac.icao.clone());

            if let Some(prev) = last_pushed.get(&ac.icao) {
                if prev.last_seen == ac.last_seen {
                    continue;
                }
                let position_unchanged = (prev.lat - lat).abs() < f64::EPSILON
                    && (prev.lon - lon).abs() < f64::EPSILON;
                let secs_since_push = prev.pushed_at.elapsed().as_secs();
                if position_unchanged && secs_since_push < MAX_PUSH_INTERVAL_SECS {
                    continue;
                }
            }
            last_pushed.insert(
                ac.icao.clone(),
                LastPushedState {
                    last_seen: ac.last_seen,
                    lat,
                    lon,
                    pushed_at: Instant::now(),
                },
            );

            if let Some(obs) = adsb_aircraft_to_observation(ac, lat, lon, &sensor_id_str, &source_label) {
                buffer.observations.push(obs);
            }
        }
    }

    // Clean up stale entries
    if last_pushed.len() > seen_icaos.len() * 2 {
        let active: std::collections::HashSet<String> = seen_icaos.into_iter().collect();
        last_pushed.retain(|icao, _| active.contains(icao));
    }
}

fn adsb_aircraft_to_observation(
    ac: &adsb_client::Aircraft,
    lat: f64,
    lon: f64,
    sensor_id_str: &str,
    source_label: &str,
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

    let pos_var = 10_000.0_f64;
    let vel_var = 100.0_f64;
    let cov = nalgebra::DMatrix::from_diagonal(&nalgebra::DVector::from_vec(vec![
        pos_var, pos_var, pos_var, vel_var, vel_var, vel_var,
    ]));

    Some(SensorObservation {
        sensor_id: SensorId {
            id: sensor_id_str.to_string(),
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
            source_label: source_label.to_string(),
            is_on_ground: ac.is_on_ground,
            ..Default::default()
        },
    })
}
