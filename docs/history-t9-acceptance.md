# History T9 Acceptance

The fusion agent owns authoritative display history. It records projected
`DisplayHistorySample` values with observation-derived timestamps, field-level
provenance, stable sample sequences, continuity segments, and a session-wide
operation revision. The renderer and thin client do not record live history.

## Runtime Flow

1. `FusionClock` supplies UTC and monotonic scheduling time. Tests and replay
   can install a fixed clock without sleeping.
2. `HistoryRecorder` samples projected display state independently of client
   connections and trail visibility.
3. `DisplayTrail` is the bounded visible preview replicated with the current
   display state.
4. Selected history uses the existing Replicon/Renet connection. The agent
   sends a snapshot at a revision watermark followed by bounded operations.
5. `ClientHistoryStore` is the read model consumed by selected trails and
   altitude/ground-speed charts. A complete snapshot is authoritative for its
   retained range, so reconnect hydration removes samples pruned while the
   client was absent.
6. Embedded and headless modes use the same projected history vocabulary.
   Playback continues to use the existing `RecordedFrame` NDJSON adapter.

## Defaults And Limits

| Setting | Default |
| --- | ---: |
| Agent retention per track | 30 minutes |
| Sampling target | 2 seconds |
| Visible preview | 5 minutes |
| Per-track retained samples | 901 |
| Global retained samples | 100,000 |
| Snapshot chunk size | 64 samples |
| Maximum snapshot | 901 samples |
| Client retained samples | 3,604 |
| Client active requests | 4 |
| Buffered operations per request | 256 |

The wire protocol identifier is `0xA17E_D100_0003`. A change to the replicated
component set or history message shape requires a protocol identifier update.
The limits are in-memory bounds; restart persistence is not part of T9.

## Deterministic Acceptance Harness

The harness is in `src/history_acceptance.rs` and uses `FusionClock::fixed`:

```bash
cargo test --locked --no-default-features --bin airjedi_bevy history_acceptance::tests::full_late_join_experience_converges_through_reconnect_and_retention
```

It collects a moving timestamped trajectory for 20 simulated minutes before a
late client connects, compares it with an early continuously connected client,
then continues ingestion through a correction, interruption, reconnect, and
the 30-minute retention boundary. It asserts values, original timestamps,
field provenance, sequence identity, gaps, revisions, coverage, selected and
background loading, chart points, 2D/3D trail continuity, recorder parity, and
existing playback format compatibility.

## Verification Limits

The deterministic and headless checks run on the development host. No target
Raspberry Pi was available for T9 resource and latency measurements. No visual
BRP or equivalent screenshot harness was available in this worktree, so the
gizmo and mesh-strip checks are geometry/continuity checks rather than rendered
frame evidence. Those limitations are not counted as passing visual or target
hardware acceptance.

The completed host checks for this revision were:

- `cargo test --locked -p adsb-client`: passed, including 79 unit, 5 ICAO,
  4 ingest replay, and 2 doc tests.
- `cargo test --locked -p airjedi-core`: passed, 7 unit tests.
- `cargo test --locked -p airjedi-fusion`: passed, including 112 unit and all
  ingest, fusion, projection, history, and display integration tests.
- `cargo test --locked -p airjedi-net`: passed, including history transfer and
  replication round-trip tests.
- `cargo check --locked -p airjedi-agent`: passed.
- `cargo check --locked -p airjedi_bevy`: passed.
- `cargo check --locked -p airjedi_bevy --features thin-client`: passed.
- `cargo test --locked --no-default-features --bin airjedi_bevy`: 201 passed;
  four pre-existing NOTAM/TFR fixture tests failed in
  `src/data_ingest/fixture_tests.rs` because the checked-in fixtures currently
  yield different counts/shapes. No history test depends on those fixtures.
- Touched Rust files pass targeted rustfmt and `git diff --check`. A
  repository-wide `cargo fmt --all -- --check` still reports unrelated
  pre-existing formatting drift outside this change.
