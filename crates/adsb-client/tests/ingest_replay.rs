//! Correlated ingest-simulation test ("actual ingest" tier).
//!
//! Replays a real 5-minute BEAST capture (taken from the live feeder on
//! `192.168.1.10:30005`) through the full adsb-client ingest pipeline
//! (`BeastFramer` -> `Rs1090Decoder` -> `AircraftTracker`) and cross-references
//! the decoded aircraft against the correlated readsb NDJSON enrichment capture
//! (`:30047`) recorded at the same time.
//!
//! Purpose: guard against regressions in framing, Mode-S decode, and tracking
//! over known-good real-world traffic. This is deliberately assertion-based on
//! robust invariants (bands and set-overlap), not exact counts. Exact-value
//! golden snapshots (via `insta`) arrive with the design-b display-component
//! projection as the higher "Tier 4" end-to-end gate; this file is the layer
//! below it that proves the raw ingest still works.
//!
//! Fixtures + capture recipe: `crates/adsb-client/tests/fixtures/ingest/README.md`.
//!
//! Note: this capture window is pure ADS-B (`adsb_icao`). MLAT and TIS-B
//! source-tagged targets are sparse and did not appear in it; a future
//! opportunistic capture will add a fixture that exercises those paths.

#![cfg(feature = "decoder-rs1090")]

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;

use adsb_client::{
    AircraftTracker, BeastFramer, Decoder, Framer, Icao, MessagePayload, Rs1090Decoder,
    TrackerConfig,
};

// Surveyed receiver location from infra/k3s-pi/configmap.yaml (FEEDER_LAT/LONG),
// used for CPR local decode fallback and distance filtering.
const RECEIVER_LAT: f64 = 37.7139;
const RECEIVER_LON: f64 = -97.1364;

const BEAST_FIXTURE: &str = "beast_30005_20260902.bin.gz";
const NDJSON_FIXTURE: &str = "readsb_30047_20260902.ndjson.gz";

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

/// Drive the BEAST fixture through the real pipeline exactly as a socket would:
/// bytes -> framer -> decoder -> tracker, feeding arbitrary chunk boundaries so
/// the framer's incremental buffering is exercised.
fn replay_beast() -> ReplayResult {
    let bytes = read_gz(BEAST_FIXTURE);

    let mut framer = BeastFramer::new();
    let mut decoder = Rs1090Decoder::new();
    decoder.set_reference_position(RECEIVER_LAT, RECEIVER_LON);
    let mut tracker = AircraftTracker::new(TrackerConfig {
        center: Some((RECEIVER_LAT, RECEIVER_LON)),
        max_distance_miles: 450.0,
        // Replay happens in a burst, so keep everything "fresh" — we are not
        // testing staleness here.
        aircraft_timeout_secs: 3600,
        position_history_secs: 3600,
        event_channel_capacity: 1024,
    });

    let mut frames = 0usize;
    let mut counts = PayloadCounts::default();
    for chunk in bytes.chunks(4096) {
        framer.feed(chunk);
        while let Some(frame) = framer.next_frame() {
            frames += 1;
            for msg in decoder.decode(&frame) {
                match &msg.payload {
                    MessagePayload::Identification { .. } => counts.identification += 1,
                    MessagePayload::Position { .. } => counts.position += 1,
                    MessagePayload::Velocity { .. } => counts.velocity += 1,
                    MessagePayload::Altitude { .. } => counts.altitude += 1,
                    _ => counts.other += 1,
                }
                tracker.process_message(msg);
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
/// serde_json dependency for a two-field correlation check.
fn extract_str(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Parse the NDJSON enrichment fixture into (set of aircraft ICAOs, source-type
/// histogram).
fn parse_ndjson() -> (HashSet<u32>, HashMap<String, usize>) {
    let bytes = read_gz(NDJSON_FIXTURE);
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

#[test]
fn beast_replay_decodes_expected_traffic() {
    let r = replay_beast();
    eprintln!(
        "[ingest] frames={} messages={} counts={:?} aircraft={}",
        r.frames,
        r.messages,
        r.counts,
        r.tracker.len()
    );

    // The 5-minute capture holds ~105k Mode-S frames; framing must recover the bulk.
    assert!(r.frames > 50_000, "expected >50k frames, got {}", r.frames);

    // All four core ADS-B / Mode-S message classes must be present and decoding.
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

    // Aircraft count should be in the same ballpark as readsb saw (86 in NDJSON).
    let ac = r.tracker.len();
    assert!(ac >= 40, "implausibly few aircraft tracked: {ac}");
    assert!(ac <= 150, "implausibly many aircraft tracked: {ac}");

    // Most tracked aircraft should have a decoded position.
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
    let r = replay_beast();
    let (ndjson_icaos, types) = parse_ndjson();

    // This capture window is pure ADS-B; document that as an invariant so a
    // future MLAT/TIS-B fixture is added deliberately rather than silently.
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

    // The two captures were recorded simultaneously, so the BEAST-decoded
    // aircraft must overlap heavily with the NDJSON aircraft set. This is the
    // core proof that the correlated pair actually joins by ICAO.
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
