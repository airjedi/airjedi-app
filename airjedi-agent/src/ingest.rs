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

/// One positioned target from the capture.
#[derive(Clone, Debug)]
pub struct Contact {
    pub icao: u32,
    pub lat: f64,
    pub lon: f64,
    pub alt_ft: Option<i32>,
    pub track: Option<f64>,
    pub vel_kts: Option<f64>,
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
    for chunk in bytes.chunks(4096) {
        framer.feed(chunk);
        while let Some(frame) = framer.next_frame() {
            for msg in decoder.decode(&frame) {
                tracker.process_message(msg);
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
    for line in text.lines() {
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

/// Build a fusion observation from a contact at time `now`.
#[must_use]
pub fn make_observation(c: &Contact, kind: SensorKind, now: DateTime<Utc>) -> SensorObservation {
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
        timestamp: now,
        receipt_time: now,
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
        metadata: ObservationMetadata::default(),
    }
}
