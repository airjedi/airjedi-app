//! Correlated ingest-simulation test ("actual ingest" tier).
//!
//! Replays real feeder captures through the full adsb-client ingest pipeline
//! (`BeastFramer` -> `Rs1090Decoder` -> `AircraftTracker`) and cross-references
//! the decoded aircraft against the correlated readsb NDJSON enrichment capture
//! recorded at the same time.
//!
//! Two fixture pairs (see `tests/fixtures/ingest/README.md`):
//!   - 2026-09-02          : pure ADS-B (`adsb_icao` only)
//!   - 2026-09-02 (_mlat)  : ADS-B + 17 MLAT-tagged aircraft
//!
//! Purpose: guard against regressions in framing, Mode-S decode, and tracking
//! over known-good real-world traffic, and prove the BEAST decode and the
//! NDJSON source tags stay synchronized. Assertions are robust invariants
//! (bands and set-overlap), not exact counts; exact-value golden snapshots
//! (via `insta`) arrive with the design-b display-component projection.

#![cfg(feature = "decoder-rs1090")]

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;

use adsb_client::{
    AircraftTracker, BeastFramer, Decoder, Framer, Icao, MessagePayload, Rs1090Decoder,
    TrackerConfig,
};
use chrono::{DateTime, Utc};

// Surveyed receiver location from infra/k3s-pi/configmap.yaml (FEEDER_LAT/LONG),
// used for CPR local decode fallback and distance filtering.
const RECEIVER_LAT: f64 = 37.7139;
const RECEIVER_LON: f64 = -97.1364;

// ADS-B-only fixture pair.
const BEAST_FIXTURE: &str = "beast_30005_20260902.bin.gz";
const NDJSON_FIXTURE: &str = "readsb_30047_20260902.ndjson.gz";

// MLAT-bearing fixture pair (17 MLAT aircraft).
const BEAST_MLAT_FIXTURE: &str = "beast_30005_20260902_mlat.bin.gz";
const NDJSON_MLAT_FIXTURE: &str = "readsb_30047_20260902_mlat.ndjson.gz";

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ingest")
        .join(name)
}

fn read_gz(name: &str) -> Vec<u8> {
    let path = fixture_path(name);
    let file = std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("open fixture {}: {e}", path.display()));
    let mut decoder = flate2::read::GzDecoder::new(file);
    let mut buf = Vec::new();
    decoder
        .read_to_end(&mut buf)
        .unwrap_or_else(|e| panic!("gunzip {}: {e}", path.display()));
    buf
}

#[derive(Default, Debug)]
struct PayloadCounts {
    identification: usize,
    position: usize,
    velocity: usize,
    altitude: usize,
    other: usize,
}

struct ReplayResult {
    tracker: AircraftTracker,
    counts: PayloadCounts,
    frames: usize,
    messages: usize,
}

/// Drive a BEAST fixture through the real pipeline exactly as a socket would:
/// bytes -> framer -> decoder -> tracker, feeding arbitrary chunk boundaries so
/// the framer's incremental buffering is exercised.
fn replay_beast(beast_fixture: &str) -> ReplayResult {
    let bytes = read_gz(beast_fixture);

    let mut framer = BeastFramer::new();
    let mut decoder = Rs1090Decoder::new();
    decoder.set_reference_position(RECEIVER_LAT, RECEIVER_LON);
    let mut tracker = AircraftTracker::new(TrackerConfig {
        center: Some((RECEIVER_LAT, RECEIVER_LON)),
        max_distance_miles: 450.0,
        aircraft_timeout_secs: 3600,
        position_history_secs: 3600,
        event_channel_capacity: 1024,
    });
    let replay_receipt = DateTime::<Utc>::from_timestamp(0, 0).expect("Unix epoch is valid");

    let mut frames = 0usize;
    let mut counts = PayloadCounts::default();
    for chunk in bytes.chunks(4096) {
        framer.feed(chunk);
        while let Some(frame) = framer.next_frame() {
            frames += 1;
            for msg in decoder.decode_at(&frame, replay_receipt) {
                match &msg.payload {
                    MessagePayload::Identification { .. } => counts.identification += 1,
                    MessagePayload::Position { .. } => counts.position += 1,
                    MessagePayload::Velocity { .. } => counts.velocity += 1,
                    MessagePayload::Altitude { .. } => counts.altitude += 1,
                    _ => counts.other += 1,
                }
                tracker.process_decoded_message(msg);
            }
        }
    }

    let messages = counts.identification
        + counts.position
        + counts.velocity
        + counts.altitude
        + counts.other;
    ReplayResult {
        tracker,
        counts,
        frames,
        messages,
    }
}

/// Lightweight extraction of a `"key":"value"` string field, avoiding a
/// serde_json dependency for a couple of correlation checks.
fn extract_str(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Parse an NDJSON enrichment fixture into (set of aircraft ICAOs, source-type
/// histogram).
fn parse_ndjson(ndjson_fixture: &str) -> (HashSet<u32>, HashMap<String, usize>) {
    let bytes = read_gz(ndjson_fixture);
    let text = String::from_utf8_lossy(&bytes);

    let mut icaos = HashSet::new();
    let mut types: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        if let Some(hex) = extract_str(line, "\"hex\":\"") {
            if let Some(icao) = Icao::from_hex(&hex) {
                icaos.insert(icao.0);
            }
        }
        if let Some(t) = extract_str(line, "\"type\":\"") {
            *types.entry(t).or_insert(0) += 1;
        }
    }
    (icaos, types)
}

/// ICAOs that appear at least once with `"type":"mlat"` in the NDJSON fixture.
fn parse_ndjson_mlat_icaos(ndjson_fixture: &str) -> HashSet<u32> {
    let bytes = read_gz(ndjson_fixture);
    let text = String::from_utf8_lossy(&bytes);
    let mut icaos = HashSet::new();
    for line in text.lines() {
        if extract_str(line, "\"type\":\"").as_deref() == Some("mlat") {
            if let Some(hex) = extract_str(line, "\"hex\":\"") {
                if let Some(icao) = Icao::from_hex(&hex) {
                    icaos.insert(icao.0);
                }
            }
        }
    }
    icaos
}

// ---------------------------------------------------------------------------
// ADS-B-only fixture
// ---------------------------------------------------------------------------

#[test]
fn beast_replay_decodes_expected_traffic() {
    let r = replay_beast(BEAST_FIXTURE);
    eprintln!(
        "[ingest] frames={} messages={} counts={:?} aircraft={}",
        r.frames,
        r.messages,
        r.counts,
        r.tracker.len()
    );

    assert!(r.frames > 50_000, "expected >50k frames, got {}", r.frames);
    assert!(
        r.counts.position > 1_000,
        "too few positions decoded: {}",
        r.counts.position
    );
    assert!(
        r.counts.velocity > 500,
        "too few velocities decoded: {}",
        r.counts.velocity
    );
    assert!(
        r.counts.identification > 50,
        "too few identifications decoded: {}",
        r.counts.identification
    );
    assert!(
        r.counts.altitude > 500,
        "too few altitude replies decoded: {}",
        r.counts.altitude
    );

    let ac = r.tracker.len();
    assert!(ac >= 40, "implausibly few aircraft tracked: {ac}");
    assert!(ac <= 150, "implausibly many aircraft tracked: {ac}");

    let positioned = r
        .tracker
        .get_aircraft()
        .iter()
        .filter(|a| a.latitude.is_some() && a.longitude.is_some())
        .count();
    assert!(positioned >= 30, "too few positioned aircraft: {positioned}");
}

#[test]
fn beast_and_ndjson_are_correlated() {
    let r = replay_beast(BEAST_FIXTURE);
    let (ndjson_icaos, types) = parse_ndjson(NDJSON_FIXTURE);

    // This window is pure ADS-B; document that as an invariant so a future
    // MLAT/TIS-B fixture is added deliberately rather than silently.
    let mut kinds: Vec<&String> = types.keys().collect();
    kinds.sort();
    assert_eq!(
        kinds,
        vec![&"adsb_icao".to_string()],
        "unexpected enrichment source types in this fixture: {types:?}"
    );
    assert!(
        ndjson_icaos.len() >= 70,
        "too few aircraft in NDJSON: {}",
        ndjson_icaos.len()
    );

    let decoded: HashSet<u32> = r.tracker.get_aircraft().iter().map(|a| a.icao.0).collect();
    let overlap = decoded.iter().filter(|i| ndjson_icaos.contains(i)).count();
    let overlap_frac = overlap as f64 / decoded.len().max(1) as f64;
    eprintln!(
        "[ingest] beast_aircraft={} ndjson_aircraft={} overlap={overlap} frac={overlap_frac:.3}",
        decoded.len(),
        ndjson_icaos.len()
    );
    assert!(
        overlap_frac > 0.8,
        "correlated captures should share aircraft; overlap only {overlap_frac:.3}"
    );
}

// ---------------------------------------------------------------------------
// MLAT fixture
// ---------------------------------------------------------------------------

#[test]
fn mlat_fixture_has_mlat_source_tags() {
    let (_icaos, types) = parse_ndjson(NDJSON_MLAT_FIXTURE);
    eprintln!("[ingest-mlat] source types: {types:?}");

    // Unlike the ADS-B-only fixture, this one must contain mlat-tagged lines.
    let mlat_lines = *types.get("mlat").unwrap_or(&0);
    assert!(
        mlat_lines >= 20,
        "expected mlat-tagged NDJSON lines, got {mlat_lines}"
    );
    assert!(
        types.contains_key("adsb_icao"),
        "fixture should still contain the ADS-B majority"
    );

    let mlat_icaos = parse_ndjson_mlat_icaos(NDJSON_MLAT_FIXTURE);
    eprintln!("[ingest-mlat] distinct type:mlat aircraft: {:?}", mlat_icaos);
    // This window has exactly one genuine MLAT position-source aircraft
    // (ae5e13). That single sample is what exercises PositionSource::Mlat -
    // readsb's `type:"mlat"` is the only signal enrichment.rs maps to Mlat.
    // (17 other aircraft carry MLAT-derived *fields* via a non-empty mlat[]
    // array but keep an adsb_icao position source; those are not MLAT here.)
    assert!(
        !mlat_icaos.is_empty(),
        "expected at least one type:mlat aircraft"
    );
    assert!(
        mlat_icaos.contains(&0x00ae_5e13),
        "expected the known MLAT aircraft ae5e13 to be present"
    );
}

#[test]
fn mlat_beast_and_ndjson_are_synchronized() {
    // The pair was captured simultaneously, so the aircraft the NDJSON tags as
    // MLAT must also be present in the BEAST decode. This is the core proof that
    // the two streams are correlated for the MLAT-specific path.
    let r = replay_beast(BEAST_MLAT_FIXTURE);
    let (ndjson_icaos, _types) = parse_ndjson(NDJSON_MLAT_FIXTURE);
    let mlat_icaos = parse_ndjson_mlat_icaos(NDJSON_MLAT_FIXTURE);

    let decoded: HashSet<u32> = r.tracker.get_aircraft().iter().map(|a| a.icao.0).collect();

    // Overall correlation, same as the ADS-B fixture.
    let overlap = decoded.iter().filter(|i| ndjson_icaos.contains(i)).count();
    let overlap_frac = overlap as f64 / decoded.len().max(1) as f64;
    assert!(
        overlap_frac > 0.8,
        "correlated captures should share aircraft; overlap only {overlap_frac:.3}"
    );

    // MLAT aircraft (Mode-S contacts multilaterated by the aggregator) emit
    // Mode-S replies + injected position frames, so their ICAOs must appear in
    // the BEAST decode too. Allow a small miss margin for aircraft that faded
    // in/out at the capture edges.
    let mlat_decoded = mlat_icaos.iter().filter(|i| decoded.contains(i)).count();
    let mlat_frac = mlat_decoded as f64 / mlat_icaos.len().max(1) as f64;
    eprintln!(
        "[ingest-mlat] mlat_icaos={} present_in_beast={mlat_decoded} frac={mlat_frac:.3}",
        mlat_icaos.len()
    );
    assert!(
        mlat_frac >= 0.8,
        "MLAT aircraft from NDJSON should appear in the BEAST decode; only {mlat_frac:.3} did"
    );
}
