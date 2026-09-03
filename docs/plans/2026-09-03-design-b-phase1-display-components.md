# Design B — Phase 1: Display Components (redrafted)

**Status:** ready to implement · **Supersedes:** the monolithic `TargetStore`
sketch from the earlier design-b discussion.

## Goal

Make the projection from fusion state to *render-ready display state* **complete
and serializable**, expressed as ECS components on the track/visual entity, so
that:

- **Fat mode (now):** agent-side systems write the components into the app's own
  `World`; the UI reads them directly. Zero transport, zero behavior change.
- **Thin mode (later):** a replication layer (`bevy_replicon` candidate) copies
  the same components agent `World` -> client `World`; the UI reads the *same*
  components and cannot tell which mode it is in.

This is the invariant that makes every later transport choice cheap and
reversible. Phase 1 delivers the fat-mode refactor only; transport is deferred.

## Governing principle

Today the UI does not read fused tracks — it reaches into fusion *internals*
(re-runs the filter, projects covariance, mutates track state, reads per-sensor
history). Six modules bypass the projection boundary via
`FusionTrackLink.track_entity`. Phase 1 moves **all** of that derivation behind
the boundary so the UI depends only on display components.

## The component set

All live on the visual/track entity, all `Serialize`/`Deserialize`, all keyed by
`TrackId` (never a raw `Entity`).

### `DisplayTrack` (supersedes `Aircraft` + `FusionDiagnostics`)
```
icao: String, callsign: Option<String>,
latitude: f64, longitude: f64, altitude_ft: Option<i32>,
heading: Option<f32>, velocity_kts: Option<f64>, vertical_rate: Option<i32>,
roll_angle: Option<f32>, track_angle_rate: Option<f32>,
squawk: Option<String>, is_on_ground: Option<bool>,
alert: Option<bool>, emergency: Option<bool>, spi: Option<bool>,
last_seen: DateTime<Utc>,
status: TrackStatus,                 // Confirmed / Coasting / ...
position_source: Option<PositionSource>,  // ADS-B / MLAT / TIS-B  <- from enrichment
h_uncertainty_m: Option<f64>,        // absorbs uncertainty_viz's scalar output
predicting: bool,                    // absorbs interpolation's flag
filter_type: &'static str, mode_probabilities: Option<Vec<f64>>,
dominant_mode: Option<usize>, observation_count: u32,
```

### `DisplayEstimate` (absorbs `estimated_track`)
```
samples: Vec<PredictedSample>,   // { lat, lon, h_uncertainty_m, heading_deg, time_ahead }
maneuver_prob: f32,              // drives cone coloring, client-side
```

### `SensorContributions` (absorbs `multi_sensor_debug`)
```
sources: Vec<SensorReport>,     // { sensor_id: String, lat: f64, lon: f64, kind: SensorKind }
```

### `DisplayTrail` (bounded history for mid-flight connect)
```
points: Vec<TrailPoint>,        // { lat, lon, alt_ft, ts } — bounded (e.g. 200)
```

## `airjedi-core` (new, dependency-light shared crate)

Extract the domain enums both agent and client must name without pulling in
`airjedi-fusion`: `TargetId`, `TargetDomain`, `TargetCategory`, `IdentifierType`,
`Affiliation`, `TrackStatus`, and a relocated `PositionSource` (today in
`src/adsb/enrichment.rs`). `sensor.rs` in fusion is already bevy-free;
`types.rs` needs a few `Reflect` derives stripped. The display components above
live here too, so both sides depend on `airjedi-core` only.

## Module-by-module transformation

| Module | Moves to agent (writes) | Stays client (reads + renders) |
|---|---|---|
| `render_bridge::sync_tracks_to_visuals` | the prefer-raw/prefer-filter **merge policy** + ECEF→heading/vrate → writes `DisplayTrack` | entity spawn/despawn, `AircraftModelRegistry` model load, `Transform` placement |
| `estimated_track` (858 L) | clone+`predict()` forward-sampling, `HeadingHistory`, IMM turn-rate weighting → writes `DisplayEstimate` | `draw_estimated_track_cones` gizmo drawing from `DisplayEstimate` |
| `landing_detection` | runway-proximity test + `track.is_on_ground = true` + `zero_velocity()` (needs runway data agent-side) | `LandedAircraft` marker + visual treatment from `DisplayTrack.is_on_ground` |
| `uncertainty_viz` | ECEF covariance → ENU → 1σ meters → `DisplayTrack.h_uncertainty_m` | meters→world-units + `gizmos.circle_2d` |
| `multi_sensor_debug` | `TimelineStore.latest_per_sensor` read → `SensorContributions` | per-sensor marker + line drawing |
| `interpolation` | (trivial) speed→`predicting` flag on `DisplayTrack` | existing `InterpolationState` dead-reckoning |

## Clock refactor (hard prerequisite — decision #1)

Projection/derivation systems must take time from an **injectable clock
resource**, never `chrono::Utc::now()` or `Time<Real>` directly. Today the
offenders are `render_bridge` (`Time<Real>`), `cleanup_orphaned_visuals` /
`refresh_aircraft_last_seen` / `landing_detection` (`Utc::now()`), and fusion
`predict()` (frame dt).

**Why this is non-negotiable, with evidence:** the ingest-simulation suite
(`crates/adsb-client/tests/ingest_replay.rs`) just caught `Rs1090Decoder`
timestamping frames with `Utc::now()`, which collapsed all replayed frames to
one instant and broke CPR — 90 of ~17k positions decoded until the fix
(`f2025d4`). A hidden wall-clock dependency breaking non-realtime ingest is
exactly what a replayable agent cannot tolerate. Route time through a resource
and the replay/snapshot harness drives it from fixture timestamps.

## Serialization discipline

- No `nalgebra::DMatrix`, `Entity`, or `bevy_asset::Handle` crosses the
  boundary. That is *why* `uncertainty_viz`/`estimated_track` pre-reduce the
  covariance to scalars/samples agent-side.
- Cross-references use `TrackId`, not `Entity` (replace `FusionTrackLink`'s
  `Entity` field usage with `TrackId` lookups).

## Verification

- **Fat-mode is a pure refactor:** the agent systems run in the same `App`,
  writing the same numbers the UI already shows. Verify screenshot-identical
  output against `main` (BRP screenshots at the reference Wichita FL300 view).
- **Tier-4 ingest→display snapshots:** extend the existing ingest-simulation
  harness — replay the correlated captures through decode → fusion → the new
  display components and snapshot them with `insta` (decision #2). The MLAT
  fixture (`ae5e13`) asserts `PositionSource::Mlat` reaches
  `DisplayTrack.position_source`; the fusion correlated test
  (`airjedi-fusion/tests/correlated_mlat.rs`) is the seed of this.

## Task breakdown (small, ordered commits)

1. **`airjedi-core` crate:** extract domain enums + relocate `PositionSource`;
   define the four display components. Wire `airjedi-fusion` and the app to it.
2. **Clock resource:** introduce an injectable `SimClock`/time resource; convert
   the four wall-clock offenders to read it. No behavior change in live mode.
3. **`DisplayTrack`:** move `render_bridge`'s merge/derivation into a system that
   writes `DisplayTrack`; leave spawn/model-load reading it. Delete `Aircraft`
   writes once panels/camera read `DisplayTrack`.
4. **`DisplayEstimate`:** move `estimated_track`'s sampling agent-side; rewrite
   the cone drawer to read `DisplayEstimate`.
5. **`h_uncertainty_m` + `SensorContributions`:** move the covariance and
   TimelineStore reads agent-side; rewrite the two gizmo drawers.
6. **`landing_detection`:** move the decision agent-side (with runway data);
   client reads the flag.
7. **`interpolation`:** read `DisplayTrack`; delete the `TrackerState` read.
8. **Tier-4 snapshot tests** over the display components; screenshot parity check.

## Explicitly deferred (not Phase 1)

- Transport / thin-mode / `bevy_replicon` spike (Phase 4 decision).
- Synthetic ADS-B encoder (decision #3).
- Admin API and bulk-history/query API.
- TIS-B display handling (no fixture yet).
