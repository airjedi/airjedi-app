//! Tier-4 ingest -> fusion -> display snapshot test.
//!
//! Replays the correlated MLAT capture through decode (adsb-client, dev-dep) ->
//! the fusion pipeline -> the `derive_display_track` projection, and checks the
//! render-ready `DisplayTrack`s the client would consume. This is the end of the
//! test pyramid: it proves the whole path produces valid display state, and that
//! the enrichment source tag (`type:"mlat"` for ae5e13) reaches
//! `DisplayTrack.position_source` - the design-b boundary, exercised headless.
//!
//! Fixtures: ../crates/adsb-client/tests/fixtures/ingest/ (see its README).

use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;

use adsb_client::{BeastFramer, Decoder, Framer, Icao, Rs1090Decoder, TrackerConfig};
use airjedi_core::PositionSource;
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
const MLAT_ICAO: u32 = 0x00ae_5e13;
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
    let file =
        std::fs::File::open(&path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut d = flate2::read::GzDecoder::new(file);
    let mut buf = Vec::new();
    d.read_to_end(&mut buf)
        .unwrap_or_else(|e| panic!("gunzip {}: {e}", path.display()));
    buf
}

struct Contact {
    icao: u32,
    lat: f64,
    lon: f64,
    alt_ft: Option<i32>,
    track: Option<f64>,
    vel_kts: Option<f64>,
}

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
    // Deterministic feed order so the fused output (and this snapshot) is stable.
    contacts.sort_by_key(|c| c.icao);
    contacts
}

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

fn track_icao(track: &Track) -> Option<u32> {
    track
        .cooperative_ids
        .iter()
        .find(|id| id.id_type == IdentifierType::Icao)
        .and_then(|id| Icao::from_hex(&id.id))
        .map(|i| i.0)
}

#[test]
fn ingest_to_display_projection_tags_mlat() {
    let contacts = decode_positioned_contacts();
    let (mlat_set, mlat_pos) = parse_ndjson_mlat();
    assert!(
        mlat_set.contains(&MLAT_ICAO),
        "fixture NDJSON should tag ae5e13 as mlat"
    );
    assert!(contacts.len() >= 40, "thin decode: {}", contacts.len());

    let mut app = build_app();

    // Feed ADS-B contacts (three passes to confirm), and the MLAT aircraft from
    // its authoritative NDJSON position (as the enrichment path supplies it).
    let (mlat_lat, mlat_lon, mlat_alt) =
        mlat_pos.expect("ae5e13 should have a type:mlat position");
    let mlat_contact = Contact {
        icao: MLAT_ICAO,
        lat: mlat_lat,
        lon: mlat_lon,
        alt_ft: mlat_alt,
        track: None,
        vel_kts: None,
    };

    for _ in 0..3 {
        {
            let mut buffer = app.world_mut().resource_mut::<ObservationBuffer>();
            for c in &contacts {
                if mlat_set.contains(&c.icao) {
                    continue;
                }
                buffer
                    .observations
                    .push(make_obs(c, SensorKind::AdsbReceiver, POS_VAR_ADSB));
            }
            buffer
                .observations
                .push(make_obs(&mlat_contact, SensorKind::MlatNetwork, POS_VAR_MLAT));
        }
        app.update();
    }
    for _ in 0..3 {
        {
            let mut buffer = app.world_mut().resource_mut::<ObservationBuffer>();
            buffer
                .observations
                .push(make_obs(&mlat_contact, SensorKind::MlatNetwork, POS_VAR_MLAT));
        }
        app.update();
    }

    // Project every fused track to its render-ready DisplayTrack, exactly as the
    // agent does. The enrichment source tag comes from the NDJSON mlat set.
    let mut q = app.world_mut().query::<(&Track, &TrackerState, &TrackQuality)>();
    let mut projected: Vec<(u32, airjedi_core::DisplayTrack)> = q
        .iter(app.world())
        .filter_map(|(track, tracker, quality)| {
            let icao = track_icao(track)?;
            let source = if mlat_set.contains(&icao) {
                PositionSource::Mlat
            } else {
                PositionSource::AdsbIcao
            };
            let dt = derive_display_track(track, tracker, quality, None, Some(source));
            Some((icao, dt))
        })
        .collect();
    projected.sort_by_key(|(icao, _)| *icao);

    // Every projected track is render-valid.
    assert!(
        projected.len() >= 40,
        "too few projected display tracks: {}",
        projected.len()
    );
    for (icao, dt) in &projected {
        assert!(
            dt.latitude.is_finite() && dt.longitude.is_finite(),
            "{icao:06x} has non-finite position"
        );
        assert!(dt.altitude_ft.is_some(), "{icao:06x} missing altitude");
        assert!(!dt.icao.is_empty(), "{icao:06x} empty icao string");
    }

    // Exactly the MLAT aircraft carries PositionSource::Mlat.
    let mlat_projected: Vec<u32> = projected
        .iter()
        .filter(|(_, dt)| dt.position_source == Some(PositionSource::Mlat))
        .map(|(icao, _)| *icao)
        .collect();
    assert_eq!(
        mlat_projected,
        vec![MLAT_ICAO],
        "exactly ae5e13 should project as MLAT"
    );

    let (_, ae) = projected
        .iter()
        .find(|(icao, _)| *icao == MLAT_ICAO)
        .expect("ae5e13 must have a projected DisplayTrack");
    assert_eq!(ae.position_source, Some(PositionSource::Mlat));
    assert!(
        (ae.latitude - mlat_lat).abs() < 0.5 && (ae.longitude - mlat_lon).abs() < 0.5,
        "ae5e13 projected position ({:.4},{:.4}) should be near its MLAT fix ({mlat_lat:.4},{mlat_lon:.4})",
        ae.latitude,
        ae.longitude
    );

    // Deterministic snapshot of the MLAT aircraft's projected display state
    // (exact floats rounded so fusion tuning does not churn the snapshot).
    let summary = format!(
        "ae5e13 DisplayTrack: source={:?} status={:?} filter={} predicting={} on_ground={:?} altitude_ft_present={} lat_round={} lon_round={}",
        ae.position_source,
        ae.status,
        ae.filter_type,
        ae.predicting,
        ae.is_on_ground,
        ae.altitude_ft.is_some(),
        ae.latitude.round() as i64,
        ae.longitude.round() as i64,
    );
    insta::assert_snapshot!("ae5e13_display_projection", summary);

    eprintln!(
        "[tier4] projected={} mlat_tagged={} ae5e13=({:.4},{:.4})",
        projected.len(),
        mlat_projected.len(),
        ae.latitude,
        ae.longitude
    );
}
