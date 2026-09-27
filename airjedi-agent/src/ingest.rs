//! Fixture-replay ingest for the headless agent.
//!
//! Decodes the correlated BEAST + readsb-NDJSON capture (the same fixtures the
//! tier-4 test uses) into a static "scene" of positioned contacts, split into
//! ADS-B and MLAT sources. The agent re-emits this scene as live observations so
//! a thin client has something real to connect to. This keeps the spike
//! deterministic and dependency-free of a live dump1090; a live BEAST/SBS feed
//! is the obvious next ingest source (same `SensorObservation` output).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use adsb_client::{BeastFramer, Decoder, Framer, Icao, Rs1090Decoder, TrackerConfig};
use airjedi_core::{ObservationIdentity, TimeSourceQuality};
use airjedi_fusion::coord::CoordinateFrame;
use airjedi_fusion::sensor::*;
use airjedi_fusion::{IdentifierType, TargetCategory, TargetDomain, TargetId};
use chrono::{DateTime, Utc};
use nalgebra::{DMatrix, DVector};

pub const RECEIVER_LAT: f64 = 37.7139;
pub const RECEIVER_LON: f64 = -97.1364;
const POS_VAR_ADSB: f64 = 10_000.0;
const POS_VAR_MLAT: f64 = 250_000.0;

const BEAST_MLAT: &str = "beast_30005_20260902_mlat.bin.gz";
const NDJSON_MLAT: &str = "readsb_30047_20260902_mlat.ndjson.gz";
// The correlated capture's readsb `now` field starts at this Unix second. The
// value anchors BEAST's receiver ticks without consulting the replay process
// wall clock, and keeps the two fixture streams on the same timeline.
const REPLAY_CAPTURE_ANCHOR_UNIX_SECS: i64 = 1_788_385_464;

/// One positioned target from the capture.
#[derive(Clone, Debug)]
pub struct Contact {
    pub icao: u32,
    pub lat: f64,
    pub lon: f64,
    pub alt_ft: Option<i32>,
    pub track: Option<f64>,
    pub vel_kts: Option<f64>,
    pub observation_time: DateTime<Utc>,
    pub receipt_time: DateTime<Utc>,
    pub time_source: TimeSourceQuality,
    pub observation_id: ObservationIdentity,
}

/// The decoded scene: ADS-B contacts, MLAT contacts, and the set of ICAOs that
/// readsb tagged `type:"mlat"` (used to tag `DisplayTrack.position_source`).
#[derive(Debug)]
pub struct Scene {
    pub adsb: Vec<Contact>,
    pub mlat: Vec<Contact>,
    pub mlat_set: HashSet<u32>,
}

/// Default fixture directory, relative to this crate's manifest.
#[must_use]
pub fn default_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates/adsb-client/tests/fixtures/ingest")
}

fn read_gz(dir: &Path, name: &str) -> std::io::Result<Vec<u8>> {
    let path = dir.join(name);
    let file = std::fs::File::open(&path)?;
    let mut d = flate2::read::GzDecoder::new(file);
    let mut buf = Vec::new();
    d.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Decode the BEAST capture into positioned contacts, sorted by ICAO for
/// deterministic feed order.
fn decode_beast_contacts(dir: &Path) -> std::io::Result<Vec<Contact>> {
    let bytes = read_gz(dir, BEAST_MLAT)?;
    let mut framer = BeastFramer::new();
    let mut decoder = Rs1090Decoder::new();
    decoder.set_reference_position(RECEIVER_LAT, RECEIVER_LON);
    let mut tracker = adsb_client::AircraftTracker::new(TrackerConfig {
        center: Some((RECEIVER_LAT, RECEIVER_LON)),
        max_distance_miles: 450.0,
        aircraft_timeout_secs: 3600,
        position_history_secs: 3600,
        event_channel_capacity: 1024,
    });
    let replay_receipt = DateTime::from_timestamp(REPLAY_CAPTURE_ANCHOR_UNIX_SECS, 0)
        .expect("capture anchor is a valid timestamp");
    for chunk in bytes.chunks(4096) {
        framer.feed(chunk);
        while let Some(frame) = framer.next_frame() {
            for msg in decoder.decode_at(&frame, replay_receipt) {
                tracker.process_decoded_message(msg);
            }
        }
    }
    let mut contacts: Vec<Contact> = tracker
        .get_aircraft()
        .iter()
        .filter_map(|a| {
            let (lat, lon) = (a.latitude?, a.longitude?);
            Some(Contact {
                icao: a.icao.0,
                lat,
                lon,
                alt_ft: a.altitude,
                track: a.track,
                vel_kts: a.velocity,
                observation_time: a
                    .position_observation_time
                    .unwrap_or(a.last_observation_time),
                receipt_time: a.last_seen,
                time_source: a.position_time_source.unwrap_or(a.last_time_source),
                observation_id: a.position_observation_id.unwrap_or(a.last_observation_id),
            })
        })
        .collect();
    contacts.sort_by_key(|c| c.icao);
    Ok(contacts)
}

/// Parse the readsb NDJSON: collect the `type:"mlat"` ICAO set and the latest
/// MLAT position per ICAO.
fn parse_mlat(dir: &Path) -> std::io::Result<(HashSet<u32>, HashMap<u32, Contact>)> {
    let bytes = read_gz(dir, NDJSON_MLAT)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut set = HashSet::new();
    let mut positions: HashMap<u32, Contact> = HashMap::new();
    for (line_number, line) in text.lines().enumerate() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("mlat") {
            continue;
        }
        let Some(hex) = v.get("hex").and_then(|h| h.as_str()) else {
            continue;
        };
        let Some(icao) = Icao::from_hex(hex) else {
            continue;
        };
        set.insert(icao.0);
        if let (Some(lat), Some(lon)) = (
            v.get("lat").and_then(serde_json::Value::as_f64),
            v.get("lon").and_then(serde_json::Value::as_f64),
        ) {
            let capture_time = v
                .get("now")
                .and_then(serde_json::Value::as_f64)
                .and_then(|seconds| {
                    let whole = seconds.trunc() as i64;
                    let nanos = (seconds.fract().abs() * 1_000_000_000.0) as u32;
                    DateTime::from_timestamp(whole, nanos)
                })
                .unwrap_or_else(|| DateTime::from_timestamp(0, 0).expect("Unix epoch is valid"));
            let alt = v
                .get("alt_baro")
                .and_then(serde_json::Value::as_i64)
                .map(|a| a as i32);
            positions.insert(
                icao.0,
                Contact {
                    icao: icao.0,
                    lat,
                    lon,
                    alt_ft: alt,
                    track: None,
                    vel_kts: None,
                    observation_time: capture_time,
                    receipt_time: capture_time,
                    time_source: TimeSourceQuality::ProtocolTimestamp,
                    observation_id: ObservationIdentity {
                        frame_sequence: line_number as u64,
                        payload_index: 0,
                    },
                },
            );
        }
    }
    Ok((set, positions))
}

/// Load the full replay scene from a fixture directory.
///
/// # Errors
/// Fails if a fixture file is missing or cannot be decoded.
pub fn load_scene(dir: &Path) -> std::io::Result<Scene> {
    let beast = decode_beast_contacts(dir)?;
    let (mlat_set, mlat_positions) = parse_mlat(dir)?;

    // ADS-B = positioned BEAST contacts that readsb did not tag as MLAT.
    let adsb: Vec<Contact> = beast
        .into_iter()
        .filter(|c| !mlat_set.contains(&c.icao))
        .collect();

    // MLAT = the type:"mlat" contacts that carry a position, sorted for
    // deterministic feed order.
    let mut mlat: Vec<Contact> = mlat_positions.into_values().collect();
    mlat.sort_by_key(|c| c.icao);

    Ok(Scene {
        adsb,
        mlat,
        mlat_set,
    })
}

/// Build a fusion observation while preserving the contact's source timing.
#[must_use]
pub fn make_observation(c: &Contact, kind: SensorKind) -> SensorObservation {
    let pos_var = if matches!(kind, SensorKind::MlatNetwork) {
        POS_VAR_MLAT
    } else {
        POS_VAR_ADSB
    };
    let alt_m = c.alt_ft.map(|a| f64::from(a) * 0.3048);
    let (vn, ve) = match (c.track, c.vel_kts) {
        (Some(t), Some(v)) => {
            let mps = v * 0.514444;
            let r = t.to_radians();
            (Some(mps * r.cos()), Some(mps * r.sin()))
        }
        _ => (None, None),
    };
    let cov = DMatrix::from_diagonal(&DVector::from_vec(vec![
        pos_var, pos_var, pos_var, 100.0, 100.0, 100.0,
    ]));
    SensorObservation {
        sensor_id: SensorId {
            id: match kind {
                SensorKind::MlatNetwork => "mlat-agent".to_string(),
                _ => "adsb-agent".to_string(),
            },
            kind,
            tier: FusionTier::Regional,
            coordinate_frame: CoordinateFrame::Wgs84,
        },
        timestamp: c.observation_time,
        receipt_time: c.receipt_time,
        target_id: Some(TargetId {
            domain: TargetDomain::Air,
            id: format!("{}", Icao(c.icao)),
            id_type: IdentifierType::Icao,
        }),
        measurement: Measurement::PositionVelocity3D {
            lat_deg: c.lat,
            lon_deg: c.lon,
            alt_m,
            vel_north_mps: vn,
            vel_east_mps: ve,
            vel_down_mps: None,
            heading_deg: c.track,
        },
        covariance: ObservationCovariance { matrix: cov },
        classification_hint: Some(TargetCategory::FixedWing),
        metadata: ObservationMetadata {
            observation_id: Some(c.observation_id),
            time_source: Some(c.time_source),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_contact_reuses_source_timing_and_identity() {
        let observation_time = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let contact = Contact {
            icao: 0xA1B2C3,
            lat: 34.0,
            lon: -118.5,
            alt_ft: Some(35_000),
            track: Some(90.0),
            vel_kts: Some(100.0),
            observation_time,
            receipt_time: observation_time + chrono::Duration::seconds(2),
            time_source: TimeSourceQuality::ReceiverClock,
            observation_id: ObservationIdentity {
                frame_sequence: 12,
                payload_index: 0,
            },
        };

        let first = make_observation(&contact, SensorKind::AdsbReceiver);
        let second = make_observation(&contact, SensorKind::AdsbReceiver);

        assert_eq!(first.timestamp, observation_time);
        assert_eq!(second.timestamp, observation_time);
        assert_eq!(first.receipt_time, contact.receipt_time);
        assert_eq!(
            first.metadata.observation_id,
            second.metadata.observation_id
        );
        assert_eq!(first.metadata.time_source, second.metadata.time_source);
    }
}
