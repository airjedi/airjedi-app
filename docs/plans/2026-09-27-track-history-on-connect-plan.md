# Track History on Connect: Agent Implementation Plan

**Date:** 2026-09-27
**Status:** Approved work breakdown; issues published, awaiting human implementation authorization
**Design:** [Track History on Connect: Design](2026-09-27-track-history-on-connect-design.md)
**Parent issue:** [#31 - Restore authoritative track history for late-joining clients](https://github.com/airjedi/airjedi-app/issues/31)
**Baseline:** `1850be4`

## Goal and Delivery Contract

A client joining after 20 minutes of agent operation receives the historical
path, altitude, and ground speed collected during those 20 minutes, then keeps
up with live updates and recovers retained history after reconnecting.

Agreed scope: configurable 30-minute in-memory retention per active track,
two-second initial sampling target, five-minute default visible trail,
visible-first loading, selected-track priority, background fill, altitude and
ground-speed charts, and shared embedded/headless behavior.

## Dependency Map

| Ticket | Deliverable | Blocked by | GitHub issue |
| --- | --- | --- | --- |
| T1 | Preserve observation time from live feed to current track | None | [#32](https://github.com/airjedi/airjedi-app/issues/32) |
| T2 | Handle stationary and telemetry-only updates | T1 | [#33](https://github.com/airjedi/airjedi-app/issues/33) |
| T3 | Align embedded and headless display projection | T1 | [#34](https://github.com/airjedi/airjedi-app/issues/34) |
| T4 | Accumulate history and deliver a bounded join preview | T2, T3 | [#35](https://github.com/airjedi/airjedi-app/issues/35) |
| T5 | Load selected-track history and join the live stream | T4 | [#36](https://github.com/airjedi/airjedi-app/issues/36) |
| T6 | Render authoritative trails in 2D and 3D | T5 | [#37](https://github.com/airjedi/airjedi-app/issues/37) |
| T7 | Show altitude and ground-speed history charts | T5 | [#39](https://github.com/airjedi/airjedi-app/issues/39) |
| T8 | Harden reconnect, transfer limits, and multi-client loading | T5 | [#38](https://github.com/airjedi/airjedi-app/issues/38) |
| T9 | Verify the full late-join experience | T6, T7, T8 | [#40](https://github.com/airjedi/airjedi-app/issues/40) |

```text
T1 -> T2 --+
 |        +-> T4 -> T5 -> T6 --+
 +--> T3 -+          |-> T7 --+-> T9
                     +-> T8 --+
```

Each slice must land with its own meaningful verification. T9 integrates and
validates the complete experience; it is not the first opportunity to test a slice.

The nine issues are native sub-issues of #31 with native blocking relationships.
All are labeled `ready-for-agent` for specification readiness; `agent:build`
remains a separate human-only authorization. T1 (#32) is the initial frontier.

## Team Execution

### Authorization and workspace isolation

- Start implementation only when a human applies `agent:build` to the relevant
  issue and its blockers have been integrated. Agents do not apply that label.
- Use one branch/worktree or sandbox per active ticket, following the repository's
  `claude/<issue>-description` convention. Base new work on integrated blockers.
- Claim a ticket before work and record the chosen base revision and scope.
- Keep a single integration owner responsible for shared schemas, schedule
  ordering, manifests, lockfile changes, and cross-ticket integration.
- Follow the user's commit and push approval rules. A saved plan or readiness
  label is not authorization to commit, push, or open a PR.
- Publish implementation PRs with `Closes #<issue>` when authorized.

### Parallelism and ownership

1. Complete T1 and freeze the timestamp/observation identity interface.
2. Run T2 (ingest/fusion consumption) and T3 (projection and parity) in parallel.
   Coordinate any shared observation or raw-hint schema changes with the
   integration owner rather than editing each other's implementation.
3. Complete T4, then T5. Their history and transfer contracts are dependencies
   for the renderer, chart, and resilience work.
4. Run T6 (trail geometry/rendering), T7 (inline detail charts), and T8
   (transport scheduling/recovery) in parallel. T5 owns shared selection and
   client-history state first. UI agents consume that interface rather than
   inventing separate selection or buffering models.
5. Integrate those results before T9 validates the whole experience.

### Required handoff for every ticket

- Behavior implemented and acceptance criteria exercised.
- Changed interfaces, invariants, scheduling requirements, and configuration.
- Exact checks run with outcomes, including failures or unavailable environments.
- Any remaining decisions, measured limits, and follow-up requirements.
- Branch/base revision, relevant issue links, and integration instructions.

## T1: Preserve observation time from live feed to current track

**Blocked by:** None

**Deliverable:** A feed pause makes measurements age honestly, and capture replay
uses capture time rather than the time a developer happens to run it.

- Carry observation and receipt times plus time-source quality from decoding
  through the tracker and both ingest paths.
- Define source-aware observation identity, including equal timestamps and
  multiple decoded payloads per frame.
- Establish the BEAST receiver-tick anchor and fallback/reset/rollover rules.
- Move injectable time into shared code and use it in relevant fusion prediction,
  lifecycle, projection, and eviction paths.
- Replace re-timestamped cached-contact replay with new-measurement delivery.

**Acceptance:**

- [ ] A cached position retains its measurement time during repeated polling.
- [ ] A silent feed ages into the configured lifecycle behavior even if its TCP connection stays open.
- [ ] Replay produces the same times and values independent of wall-clock execution speed.
- [ ] Receiver restart/rollover and absent timestamps follow the documented fallback policy.
- [ ] Shared timestamp/identity interfaces and deterministic fixture helpers are available to T2 and T3.

## T2: Handle stationary and telemetry-only updates

**Blocked by:** T1

**Deliverable:** Fresh speed, altitude, and stationary position reports reach
the current display without falsely refreshing other fields or double-fusing.

- Track field freshness independently and retain source identity through ingest.
- Consume observations once using observation identity rather than timestamp-only
  cursors or changed-coordinate tests.
- Handle altitude/velocity-only updates without reusing cached position as a new
  measurement. Preserve late-arrival and acceptance semantics through fusion.

**Acceptance:**

- [ ] Fresh stationary reports are recognized; duplicate polling is ignored.
- [ ] Callsign-only reports do not refresh position or velocity freshness.
- [ ] Altitude- and speed-only updates reach the current display with correct independent ages.
- [ ] Distinct reports sharing a timestamp are processed correctly, including multiple sensors.
- [ ] Late and rejected observations cannot incorrectly refresh unrelated state.

## T3: Align embedded and headless display projection

**Blocked by:** T1

**Deliverable:** Equivalent observations produce equivalent authoritative current
display state in embedded and remote modes.

- Share raw-observation hint construction and freshness-aware projection rules.
- Keep projection independent of model loading and client interpolation.
- Define ground speed, vertical rate, heading/track reference, unknown values,
  and altitude reference consistently in the projected history input.
- Make provenance usable per field, including coasting and stale raw overrides.

**Acceptance:**

- [ ] Both modes project the same normalized display state for the same fixture.
- [ ] Confirmed, coasting, and reacquired cases preserve correct source/freshness semantics.
- [ ] Ground speed uses horizontal velocity; true zero and unknown telemetry are distinct.
- [ ] Projection works with no renderer, model registry, or client connection.
- [ ] The projection interface consumed by T4 is documented and tested.

## T4: Accumulate history and deliver a bounded join preview

**Blocked by:** T2, T3

**Deliverable:** A fresh client receives a pre-connection visible trail while the
agent maintains the configured longer history independently of UI activity.

- Implement the shared history model/recorder, actual first-seen/coverage metadata,
  independent field freshness, provenance, and discontinuity treatment.
- Retain 30 configurable minutes at the initial two-second cadence, with active
  pruning and per-track/global limits.
- Define track lifetime/removal behavior and a bounded, reconstructable correction
  horizon. Separate stable sample sequence from the history operation revision.
- Deliver a bounded preview from canonical history and materialize it in the
  client ECS. Guard live tracks from local recording immediately, with a playback
  adapter retained. Basic trail consumption is part of this slice; T6 completes
  all rendering edge cases.
- Establish safe initial bounds and a coherent wire version. Later resilience
  work tunes limits rather than introducing boundedness for the first time.

**Acceptance:**

- [ ] History accumulates before any client connects and while trails are hidden.
- [ ] A late client sees the available visible preview with original timestamps.
- [ ] No interpolation-derived or client-session-timed samples contaminate live history.
- [ ] Pruning and deletion release history; metadata reports actual available coverage.
- [ ] Corrections preserve sample identity and advance the history revision.
- [ ] Basic embedded/headless history parity is verified with deterministic inputs.

## T5: Load selected-track history and join the live stream

**Blocked by:** T4

**Deliverable:** Selecting an existing track loads its available 30-minute
history, joins live updates cleanly, and allows other tracks to fill in the background.

- Implement bounded snapshot requests, chunks, completion, and append/correction/
  prune operations over the existing connection.
- Make snapshot capture and live-operation handoff consistent at a revision
  watermark, including retention or corrections during download.
- Use server-session, `TrackId`, and request identity throughout selection,
  buffering, and history installation.
- Prioritize current state/preview, then selected-track history, then background
  fill. Selection changes reprioritize valid work; explicit cancellation or
  supersession determines which responses are discarded.
- Expose one stable client-history read interface with coverage and loading state
  for T6 and T7. Include retryable hydration when assets are not ready.

**Acceptance:**

- [ ] Selected-track history includes pre-connection altitude and ground speed.
- [ ] Interleaved chunks and live operations converge without missing or duplicate samples.
- [ ] Corrections to pre-cutoff samples are retained through the revision handoff.
- [ ] Removed tracks, superseded requests, and old sessions cannot restore stale history.
- [ ] Selection changes and background fill use a bounded shared cache.
- [ ] Delayed visual initialization does not lose history or prevent eventual hydration.

## T6: Render authoritative trails in 2D and 3D

**Blocked by:** T5

**Deliverable:** Both trail renderers show accurate pre-connection paths and
consistent gaps, altitude, age, and estimated intervals.

- Consume canonical history and its revisions using the shared server-time reference.
- Honor segment breaks in geometry, including mesh-strip topology, rather than
  merely resetting loop variables while continuing one connected strip.
- Define missing-altitude behavior and preserve estimated styling and altitude coloring.
- Handle selected full history, unselected visible windows, origin recentering,
  zoom/view changes, corrections, and history arriving after visual creation.
- Preserve playback behavior and recording compatibility.

**Acceptance:**

- [ ] Gizmo and mesh-strip trails agree in both view modes for the same history.
- [ ] Gaps and reacquisition jumps are not joined by false path segments.
- [ ] Missing altitude is not presented as an observed sea-level point.
- [ ] Clock skew and delayed transfers do not reset sample ages or fading.
- [ ] Recenter, view changes, corrections, and playback remain visually correct.

## T7: Show altitude and ground-speed history charts

**Blocked by:** T5

**Deliverable:** The active inline selected-track detail view shows historical
altitude and ground speed as soon as retained data arrives.

- Read the same canonical history used by trails through the T5 interface.
- Add 5/15/30-minute windows, time-based axes, unit labels, and timestamp/value hover.
- Show gaps, estimated intervals, transfer progress, partial coverage, and errors.
- Separate actual track age from retained coverage and sample count.
- Preserve gaps and important extrema in any presentation-only downsampling.

**Acceptance:**

- [ ] A newly selected pre-existing track shows pre-connection values.
- [ ] Uneven sample spacing appears correctly on the time axis.
- [ ] Unknown values, zero values, prediction-only intervals, and altitude references remain distinct.
- [ ] Selecting another track updates chart identity and loading state correctly.
- [ ] Complete available coverage does not imply a full 30 minutes when the agent has less.

## T8: Harden reconnect, transfer limits, and multi-client loading

**Blocked by:** T5

**Deliverable:** Reconnecting and slow clients recover retained history reliably
without starving current updates or causing unbounded memory growth.

- Exercise retry/resync, stale sessions, request cancellation, revision gaps,
  retention during transfer, and agent restart invalidation.
- Measure and tune chunk sizing, per-client pacing, active transfers, buffered
  operations, snapshot memory, and background fairness.
- Expose retained samples/bytes, pending transfers, buffered operations, coverage
  truncations, retries, and synchronization latency in diagnostics.
- Record benchmark environment and results, including the target Pi when available.

**Acceptance:**

- [ ] Client reconnect recovers retained history including the disconnected interval.
- [ ] An agent restart clears old-session data and accurately reports newly collected coverage.
- [ ] Repeated selection changes and interrupted transfers release stale work.
- [ ] A slow client cannot block current state or another client's selected-history progress.
- [ ] Declared memory/queue caps hold under 100/500/1,000-track test profiles.
- [ ] Measured budgets and their configuration defaults are documented; missing target-hardware checks are explicit.

## T9: Verify the full late-join experience

**Blocked by:** T6, T7, T8

**Deliverable:** A repeatable acceptance harness demonstrates the complete
user experience, with integrated documentation and deployment guidance.

- Accumulate a moving, timestamped trajectory for 20 simulated minutes before
  connecting a client. Compare against a continuously connected client.
- Continue ingestion through interruption/reconnection and through the retention
  boundary. Verify values, time, provenance, gaps, revisions, and available coverage.
- Run embedded/headless parity, renderer/chart checks, default and thin-client
  build checks, and the relevant existing ingest/fusion regression tests.
- Update runtime configuration/deployment documentation and the architecture/data
  flow descriptions to reflect authoritative history ownership.

**Acceptance:**

- [ ] Early and late clients converge on the same retained history after synchronization.
- [ ] Client connection timing does not affect historical sample values or timestamps.
- [ ] Reconnect, lifecycle, clocks, corrections, charts, and both renderers pass the agreed matrix.
- [ ] Existing playback and relevant ingest/fusion tests remain functional.
- [ ] Configuration, protocol upgrade requirements, memory-first limitations, and measured budgets are documented.

## Navigation and Verification Starting Points

These are baseline navigation aids, not fixed implementation boundaries. Check
the current tree before editing, especially when integrating parallel tickets.

- Time/ingest: `crates/adsb-client/src/protocol/mod.rs`,
  `crates/adsb-client/src/decoder/rs1090_decoder.rs`,
  `crates/adsb-client/src/tracker/mod.rs`, `airjedi-agent/src/live_ingest.rs`,
  `airjedi-agent/src/ingest.rs`, `airjedi-agent/src/main.rs`,
  `src/fusion_integration/adsb_adapter.rs`, `src/fusion_integration/clock.rs`.
- Projection/history: `airjedi-core/src/display.rs`,
  `airjedi-fusion/src/display.rs`, `airjedi-fusion/src/systems.rs`,
  `airjedi-agent/src/replicate_tracks.rs`, `src/fusion_integration/render_bridge.rs`.
- Transport/client: `airjedi-net/src/lib.rs`, `src/thin_client.rs`.
- Presentation/playback: `src/aircraft/trails.rs`, `src/aircraft/trail_renderer.rs`,
  `src/aircraft/mesh_trails.rs`, `src/aircraft/list_panel.rs`,
  `src/aircraft/picking.rs`, `src/recording/player.rs`, `src/config.rs`.
- Test prior art: `crates/adsb-client/tests/ingest_replay.rs`,
  `airjedi-fusion/tests/correlated_mlat.rs`, `airjedi-fusion/tests/tier4_display.rs`,
  `airjedi-net/tests/replication_roundtrip.rs`.

Run the checks relevant to each slice. The integration owner should run the
combined matrix after integration, including:

```bash
cargo test --locked -p adsb-client
cargo test --locked -p airjedi-core
cargo test --locked -p airjedi-fusion
cargo test --locked -p airjedi-net
cargo check --locked -p airjedi-agent
cargo check --locked -p airjedi_bevy
cargo check --locked -p airjedi_bevy --features thin-client
```

Use BRP or an equivalent available visual test harness for both trail renderers
and the active inline charts. Record screenshots and performance evidence under
the ignored `tmp/` directory. A headless test passing is not evidence that visual
checks or target-Pi benchmarks ran.
