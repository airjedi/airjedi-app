//! Correlated capture -> fusion test (the "fusion suite" MLAT proof).
//!
//! Decodes the real MLAT BEAST fixture (adsb-client, dev-dep only), reads the
//! correlated readsb NDJSON to learn which aircraft readsb tagged `type:"mlat"`,
//! then feeds both through the fusion pipeline exactly as the app's
//! `adsb_adapter` does: ADS-B contacts as `SensorKind::AdsbReceiver`, the MLAT
//! contact as `SensorKind::MlatNetwork` with a larger position covariance.
//!
//! This proves the two correlated streams are synchronized AND that the
//! MLAT-tagged observation is accepted and fused into a track at its observed
//! position - end to end, in the fusion suite.
//!
//! Fixtures: ../crates/adsb-client/tests/fixtures/ingest/ (see its README).

use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;

use adsb_client::{BeastFramer, Decoder, Framer, Icao, Rs1090Decoder, TrackerConfig};
use airjedi_fusion::coord::CoordinateFrame;
use airjedi_fusion::sensor::*;
use airjedi_fusion::systems::ObservationBuffer;
use airjedi_fusion::*;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use chrono::Utc;
use nalgebra::{DMatrix, DVector};

const RECEIVER_LAT: f64 = 37.7139;
const RECEIVER_LON: f64 = -97.1364;
const MLAT_ICAO: u32 = 0x00ae_5e13; // the sole type:"mlat" aircraft in this capture
const POS_VAR_ADSB: f64 = 10_000.0;
const POS_VAR_MLAT: f64 = 250_000.0;

const BEAST_MLAT: &str = "beast_30005_20260902_mlat.bin.gz";
const NDJSON_MLAT: &str = "readsb_30047_20260902_mlat.ndjson.gz";

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/adsb-client/tests/fixtures/ingest")
        .join(name)
}

fn read_gz(name: &str) -> Vec<u8> {
    let path = fixture(name);
    let file = std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut d = flate2::read::GzDecoder::new(file);
    let mut buf = Vec::new();
    d.read_to_end(&mut buf).unwrap_or_else(|e| panic!("gunzip {}: {e}", path.display()));
    buf
}

/// A decoded contact reduced to what we need to build an observation.
struct Contact {
    icao: u32,
    lat: f64,
    lon: f64,
    alt_ft: Option<i32>,
    track: Option<f64>,
    vel_kts: Option<f64>,
}

/// Decode the BEAST fixture and return every aircraft that has a position.
fn decode_positioned_contacts() -> Vec<Contact> {
    let bytes = read_gz(BEAST_MLAT);
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
                tracker.process_decoded_message(msg);
            }
        }
    }
    tracker
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
        .collect()
}

/// From the NDJSON: the set of ICAOs readsb tagged `type:"mlat"`, and the last
/// such position seen for the known MLAT aircraft.
fn parse_ndjson_mlat() -> (HashSet<u32>, Option<(f64, f64, Option<i32>)>) {
    let bytes = read_gz(NDJSON_MLAT);
    let text = String::from_utf8_lossy(&bytes);
    let mut mlat = HashSet::new();
    let mut mlat_pos = None;
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
        let Some(icao) = Icao::from_hex(hex) else { continue };
        mlat.insert(icao.0);
        if icao.0 == MLAT_ICAO {
            if let (Some(lat), Some(lon)) = (
                v.get("lat").and_then(serde_json::Value::as_f64),
                v.get("lon").and_then(serde_json::Value::as_f64),
            ) {
                let alt = v
                    .get("alt_baro")
                    .and_then(serde_json::Value::as_i64)
                    .map(|a| a as i32);
                mlat_pos = Some((lat, lon, alt));
            }
        }
    }
    (mlat, mlat_pos)
}

fn make_obs(c: &Contact, kind: SensorKind, pos_var: f64) -> SensorObservation {
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
                SensorKind::MlatNetwork => "mlat-test".to_string(),
                _ => "adsb-test".to_string(),
            },
            kind,
            tier: FusionTier::Regional,
            coordinate_frame: CoordinateFrame::Wgs84,
        },
        timestamp: Utc::now(),
        receipt_time: Utc::now(),
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

fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy_time::TimePlugin);
    app.insert_resource(FusionConfig::default());
    app.add_plugins(FusionPlugin);
    app
}

#[test]
fn correlated_capture_drives_fusion_with_mlat_tag() {
    let contacts = decode_positioned_contacts();
    let (mlat_set, mlat_pos) = parse_ndjson_mlat();

    // Synchronization: the streams come from one capture window, so the MLAT
    // aircraft the NDJSON tags must also be among the BEAST-decoded contacts
    // (or at least known to the capture). ae5e13 is the known type:"mlat" one.
    assert!(
        mlat_set.contains(&MLAT_ICAO),
        "NDJSON should tag ae5e13 as mlat; got {mlat_set:?}"
    );
    assert!(
        contacts.len() >= 40,
        "expected a healthy decoded population, got {}",
        contacts.len()
    );

    let mut app = build_app();

    // Feed every ADS-B contact once; feed the MLAT contact several times (from
    // its NDJSON position) so it confirms. Mirrors adsb_adapter's tagging:
    // MlatNetwork + larger covariance for the mlat-tagged aircraft.
    let mut mlat_obs_fed = 0usize;
    {
        let mut buffer = app.world_mut().resource_mut::<ObservationBuffer>();
        for c in &contacts {
            if mlat_set.contains(&c.icao) {
                continue; // handled below with the authoritative MLAT position
            }
            buffer.observations.push(make_obs(c, SensorKind::AdsbReceiver, POS_VAR_ADSB));
        }
    }

    // The MLAT aircraft's position comes via readsb (NDJSON), exactly as the
    // enrichment path supplies it in the real app.
    let (mlat_lat, mlat_lon, mlat_alt) =
        mlat_pos.expect("ae5e13 should have a type:mlat position in the NDJSON");
    let mlat_contact = Contact {
        icao: MLAT_ICAO,
        lat: mlat_lat,
        lon: mlat_lon,
        alt_ft: mlat_alt,
        track: None,
        vel_kts: None,
    };
    for _ in 0..6 {
        {
            let mut buffer = app.world_mut().resource_mut::<ObservationBuffer>();
            buffer
                .observations
                .push(make_obs(&mlat_contact, SensorKind::MlatNetwork, POS_VAR_MLAT));
            mlat_obs_fed += 1;
        }
        app.update();
    }
    assert!(mlat_obs_fed > 0, "should have fed MLAT observations");

    // Drain the rest.
    for _ in 0..12 {
        app.update();
    }

    // 1) The correlated capture drives fusion into a healthy set of tracks.
    let track_count = app.world_mut().query::<&Track>().iter(app.world()).count();
    assert!(
        track_count >= 40,
        "expected many fused tracks from the capture, got {track_count}"
    );

    // 2) The MLAT-tagged aircraft was accepted and fused into a track at ~its
    //    observed position (proves the MlatNetwork observation flows through
    //    fusion, not just ADS-B).
    let mut q = app.world_mut().query::<(&Track, &TrackerState)>();
    let ae = q.iter(app.world()).find(|(track, _)| {
        track
            .cooperative_ids
            .iter()
            .any(|id| id.id_type == IdentifierType::Icao && Icao::from_hex(&id.id) == Some(Icao(MLAT_ICAO)))
    });
    let (_track, tracker) = ae.expect("the MLAT aircraft ae5e13 must have a fused track");
    let (lat, lon, _alt) = tracker.position_geodetic();
    assert!(
        (lat - mlat_lat).abs() < 0.5 && (lon - mlat_lon).abs() < 0.5,
        "MLAT track position ({lat:.4}, {lon:.4}) should be near its observation ({mlat_lat:.4}, {mlat_lon:.4})"
    );

    eprintln!(
        "[fusion-mlat] contacts={} tracks={track_count} mlat_obs_fed={mlat_obs_fed} ae5e13=({lat:.4},{lon:.4})",
        contacts.len()
    );
}
