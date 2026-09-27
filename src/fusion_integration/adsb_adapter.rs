use airjedi_core::ObservationFreshness;
use airjedi_fusion::coord::CoordinateFrame;
use airjedi_fusion::nalgebra;
use airjedi_fusion::sensor::*;
use airjedi_fusion::systems::ObservationBuffer;
use airjedi_fusion::types::*;
use bevy::prelude::*;
use std::collections::HashMap;

use crate::adsb::connection::FeedConnectionManager;
use crate::adsb::enrichment::{EnrichmentConnectionManager, EnrichmentInfo, PositionSource};
use adsb_client::Icao;

/// Position covariance (m^2) for a direct ADS-B/GPS-derived position.
const POS_VAR_ADSB: f64 = 10_000.0;
/// Position covariance (m^2) for an MLAT-derived position (~500m std dev).
/// MLAT triangulates from time-difference-of-arrival across ground
/// receivers and is meaningfully less precise than GPS-derived ADS-B.
const POS_VAR_MLAT: f64 = 250_000.0;

/// Tracks the last-pushed state per ICAO to avoid sending redundant observations.
pub(crate) struct LastPushedState {
    position: Option<ObservationFreshness>,
    altitude: Option<ObservationFreshness>,
    velocity: Option<ObservationFreshness>,
}

#[derive(Clone, Copy, Debug, Default)]
struct ChangedFields {
    position: Option<ObservationFreshness>,
    altitude: Option<ObservationFreshness>,
    velocity: Option<ObservationFreshness>,
}

impl ChangedFields {
    fn from_aircraft(aircraft: &adsb_client::Aircraft, previous: Option<&LastPushedState>) -> Self {
        let changed = |current: Option<ObservationFreshness>,
                       previous: Option<ObservationFreshness>| {
            current.filter(|value| previous.map(|old| old.identity) != Some(value.identity))
        };

        Self {
            position: changed(
                aircraft.position_freshness,
                previous.and_then(|state| state.position),
            ),
            altitude: changed(
                aircraft.altitude_freshness,
                previous.and_then(|state| state.altitude),
            ),
            velocity: changed(
                aircraft.velocity_freshness,
                previous.and_then(|state| state.velocity),
            ),
        }
    }

    fn any(self) -> bool {
        self.position.is_some() || self.altitude.is_some() || self.velocity.is_some()
    }
}

pub fn adsb_to_fusion_system(
    feed_mgr: Option<Res<FeedConnectionManager>>,
    enrichment_mgr: Option<Res<EnrichmentConnectionManager>>,
    mut buffer: ResMut<ObservationBuffer>,
    mut last_pushed: Local<HashMap<(String, Icao), LastPushedState>>,
) {
    let Some(feed_mgr) = feed_mgr else {
        return;
    };

    let mut seen_sources = Vec::new();

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

            let source_key = (feed_name.clone(), ac.icao);
            seen_sources.push(source_key.clone());
            let changed = ChangedFields::from_aircraft(ac, last_pushed.get(&source_key));
            if !changed.any() {
                continue;
            }

            let enrichment = enrichment_mgr.as_ref().and_then(|mgr| mgr.lookup(ac.icao));

            if let Some(obs) = adsb_aircraft_to_observation(
                ac,
                lat,
                lon,
                &sensor_id_str,
                &source_label,
                enrichment,
                changed,
            ) {
                buffer.observations.push(obs);
                last_pushed.insert(
                    source_key,
                    LastPushedState {
                        position: ac.position_freshness,
                        altitude: ac.altitude_freshness,
                        velocity: ac.velocity_freshness,
                    },
                );
            }
        }
    }

    // Clean up stale entries
    if last_pushed.len() > seen_sources.len() * 2 {
        let active: std::collections::HashSet<(String, Icao)> = seen_sources.into_iter().collect();
        last_pushed.retain(|key, _| active.contains(key));
    }
}

fn adsb_aircraft_to_observation(
    ac: &adsb_client::Aircraft,
    lat: f64,
    lon: f64,
    sensor_id_str: &str,
    source_label: &str,
    enrichment: Option<EnrichmentInfo>,
    changed: ChangedFields,
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

    let is_mlat = matches!(enrichment.map(|e| e.source), Some(PositionSource::Mlat));
    let sensor_kind = if is_mlat {
        SensorKind::MlatNetwork
    } else {
        SensorKind::AdsbReceiver
    };
    let pos_var = if is_mlat { POS_VAR_MLAT } else { POS_VAR_ADSB };
    let vel_var = 100.0_f64;
    let cov = nalgebra::DMatrix::from_diagonal(&nalgebra::DVector::from_vec(vec![
        pos_var, pos_var, pos_var, vel_var, vel_var, vel_var,
    ]));

    let latest = [changed.position, changed.altitude, changed.velocity]
        .into_iter()
        .flatten()
        .max_by_key(|freshness| freshness.receipt_time);

    Some(SensorObservation {
        sensor_id: SensorId {
            id: sensor_id_str.to_string(),
            kind: sensor_kind,
            tier: FusionTier::Regional,
            coordinate_frame: CoordinateFrame::Wgs84,
        },
        timestamp: latest
            .map(|freshness| freshness.observation_time)
            .unwrap_or(ac.last_observation_time),
        receipt_time: latest
            .map(|freshness| freshness.receipt_time)
            .unwrap_or(ac.last_seen),
        target_id: Some(TargetId {
            domain: TargetDomain::Air,
            id: ac.icao.to_string(),
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
            accuracy_category: enrichment.and_then(|e| e.nic),
            observation_id: latest
                .map(|freshness| freshness.identity)
                .or(Some(ac.last_observation_id)),
            time_source: latest
                .map(|freshness| freshness.time_source)
                .or(Some(ac.last_time_source)),
            position_freshness: changed.position,
            altitude_freshness: changed.altitude,
            velocity_freshness: changed.velocity,
            ..Default::default()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use adsb_client::protocol::{AircraftMessage, DecodedMessage, MessagePayload, MessageTiming};
    use airjedi_core::{ObservationIdentity, TimeSourceQuality};
    use chrono::DateTime;

    fn timing(frame_sequence: u64) -> MessageTiming {
        let time = DateTime::from_timestamp(1_700_000_000, 0).unwrap()
            + chrono::Duration::seconds(frame_sequence as i64);
        MessageTiming {
            observation_time: time,
            receipt_time: time,
            time_source: TimeSourceQuality::ReceiverClock,
            identity: ObservationIdentity {
                frame_sequence,
                payload_index: 0,
            },
        }
    }

    fn decoded(payload: MessagePayload, frame_sequence: u64) -> DecodedMessage {
        DecodedMessage::new(
            AircraftMessage {
                icao: Icao(0xA1B2C3),
                signal_level: None,
                payload,
            },
            timing(frame_sequence),
        )
    }

    #[test]
    fn adapter_distinguishes_stationary_reports_from_polling_and_callsign_updates() {
        let mut tracker = adsb_client::AircraftTracker::new(adsb_client::TrackerConfig {
            center: Some((37.0, -97.0)),
            ..Default::default()
        });
        tracker.process_decoded_message(decoded(
            MessagePayload::Position {
                latitude: 37.0,
                longitude: -97.0,
                altitude: Some(10_000),
                ground_speed: None,
                track: None,
                is_on_ground: None,
                altitude_gnss: None,
            },
            1,
        ));

        let first = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        let first_changed = ChangedFields::from_aircraft(first, None);
        let previous = LastPushedState {
            position: first.position_freshness,
            altitude: first.altitude_freshness,
            velocity: first.velocity_freshness,
        };

        tracker.process_decoded_message(decoded(
            MessagePayload::Identification {
                callsign: "TEST01".to_string(),
                category: None,
            },
            2,
        ));
        let after_callsign = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        assert!(!ChangedFields::from_aircraft(after_callsign, Some(&previous)).any());

        tracker.process_decoded_message(decoded(
            MessagePayload::Position {
                latitude: 37.0,
                longitude: -97.0,
                altitude: None,
                ground_speed: None,
                track: None,
                is_on_ground: None,
                altitude_gnss: None,
            },
            3,
        ));
        let stationary = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        let changed = ChangedFields::from_aircraft(stationary, Some(&previous));
        assert_eq!(changed.position.unwrap().identity.frame_sequence, 3);
        assert!(changed.altitude.is_none());

        let observation = adsb_aircraft_to_observation(
            stationary,
            stationary.latitude.unwrap(),
            stationary.longitude.unwrap(),
            "adsb-test",
            "ADS-B test",
            None,
            changed,
        )
        .unwrap();
        assert_eq!(observation.metadata.position_freshness, changed.position);
        assert_eq!(observation.metadata.altitude_freshness, changed.altitude);
        assert_eq!(first_changed.position.unwrap().identity.frame_sequence, 1);
        assert_eq!(observation.timestamp, timing(3).observation_time);
        assert_eq!(observation.receipt_time, timing(3).receipt_time);
    }

    #[test]
    fn adapter_emits_telemetry_only_fields_without_position_freshness() {
        let mut tracker = adsb_client::AircraftTracker::new(adsb_client::TrackerConfig {
            center: Some((37.0, -97.0)),
            ..Default::default()
        });
        tracker.process_decoded_message(decoded(
            MessagePayload::Position {
                latitude: 37.0,
                longitude: -97.0,
                altitude: Some(10_000),
                ground_speed: None,
                track: None,
                is_on_ground: None,
                altitude_gnss: None,
            },
            1,
        ));

        let first = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        let previous = LastPushedState {
            position: first.position_freshness,
            altitude: first.altitude_freshness,
            velocity: first.velocity_freshness,
        };

        tracker.process_decoded_message(decoded(
            MessagePayload::Velocity {
                speed: 250.0,
                track: 90.0,
                vertical_rate: Some(500),
                is_on_ground: Some(false),
                heading: None,
                airspeed: None,
                roll_angle: None,
                track_angle_rate: None,
            },
            2,
        ));
        let after_velocity = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        let velocity_changed = ChangedFields::from_aircraft(after_velocity, Some(&previous));
        assert!(velocity_changed.position.is_none());
        assert!(velocity_changed.altitude.is_none());
        assert_eq!(
            velocity_changed.velocity.unwrap().identity.frame_sequence,
            2
        );

        let velocity_observation = adsb_aircraft_to_observation(
            after_velocity,
            after_velocity.latitude.unwrap(),
            after_velocity.longitude.unwrap(),
            "adsb-test",
            "ADS-B test",
            None,
            velocity_changed,
        )
        .unwrap();
        assert!(velocity_observation.is_telemetry_only());
        assert!(velocity_observation.metadata.position_freshness.is_none());
        assert!(velocity_observation.metadata.altitude_freshness.is_none());
        assert!(velocity_observation.metadata.velocity_freshness.is_some());

        let previous_after_velocity = LastPushedState {
            position: after_velocity.position_freshness,
            altitude: after_velocity.altitude_freshness,
            velocity: after_velocity.velocity_freshness,
        };

        tracker.process_decoded_message(decoded(
            MessagePayload::Altitude {
                altitude: Some(11_000),
                squawk: None,
                alert: None,
                emergency: None,
                spi: None,
                is_on_ground: None,
            },
            3,
        ));
        let after_altitude = tracker.get_by_icao(Icao(0xA1B2C3)).unwrap();
        let altitude_changed =
            ChangedFields::from_aircraft(after_altitude, Some(&previous_after_velocity));
        assert!(altitude_changed.position.is_none());
        assert_eq!(
            altitude_changed.altitude.unwrap().identity.frame_sequence,
            3
        );
        assert!(altitude_changed.velocity.is_none());
    }
}
