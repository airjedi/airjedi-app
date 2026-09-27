# Track History on Connect: Design

**Date:** 2026-09-27
**Status:** Approved design; implementation issues published
**Baseline:** `1850be4` plus the existing local working tree
**Tracking issue:** [#31 - Restore authoritative track history for late-joining clients](https://github.com/airjedi/airjedi-app/issues/31)
**Execution plan:** [Track History on Connect: Agent Implementation Plan](2026-09-27-track-history-on-connect-plan.md)
**Related design:** [Design B, Phase 1: Display Components](2026-09-03-design-b-phase1-display-components.md)

## Problem Statement

A client connecting to an already-running agent receives current track state,
but its local trail starts empty. Position, altitude, and ground-speed history
collected before client startup should already be available. Reconnecting should
restore the agent's retained history rather than start another independent trail.

The original display-component design included a bounded `DisplayTrail` for
mid-flight connection. The current transport excludes that component, the agent
does not populate it, and its sample schema does not include historical speed.
Local trail recording also uses client-session time and can sample interpolated
positions. Ingest currently converts cached contacts into fresh observations,
so simply sending more historical points would preserve inaccurate freshness.

## Solution

The agent owns a bounded, timestamped record of authoritative display state.
Embedded and headless modes use the same recording and projection behavior.
Clients receive that record into their ECS and render it as trails and charts.

The agreed experience is:

- Retain up to **30 configurable minutes per active track**, in memory.
- Collect history regardless of connected clients or trail visibility.
- Display current tracks and the configured visible trail window first. The
  default visible window remains **five minutes**.
- Prioritize the selected track's full history, then fill other tracks in the
  background using bounded transfer capacity.
- Show altitude and **ground-speed** charts with 5-, 15-, and 30-minute windows.
- Recover retained history on client reconnect. Collection starts when the
  running agent acquires observations; restart persistence is a later feature.

## User Stories

1. As a late-joining user, I want to see paths collected before I opened the client, so I can understand a track's recent movement.
2. As a user, I want current positions to appear promptly while longer histories load, so I can use the map immediately.
3. As a user selecting a track, I want its full retained history prioritized, so the detail view becomes useful quickly.
4. As a user, I want other tracks' histories to fill in the background, so switching selection can reuse downloaded data.
5. As a user reconnecting after an interruption, I want the retained path restored without duplicated points or a snapshot-to-live gap.
6. As a user, I want altitude and ground-speed charts to show pre-connection samples with their original timestamps.
7. As a user, I want missing or stale telemetry distinguished from zero, so charts do not invent measurements.
8. As a user, I want estimated intervals visibly distinguished from observed or fused intervals, so I can assess the evidence behind a path.
9. As a user, I want trails to break across discontinuities and reacquisition jumps, so a line does not imply an observed path through a coverage gap.
10. As a user, I want history age and fading to remain correct when my computer's clock differs from the agent's clock.
11. As a user, I want track age, retained coverage, and download progress distinguished, so a short history has a clear explanation.
12. As a user, I want identical historical values in embedded and remote modes when both process the same observations.
13. As an operator, I want collection to continue when clients disconnect or hide trails, so presentation settings do not alter retained data.
14. As an operator, I want configurable retention and bounded memory and queues, so a busy feed or slow client cannot cause unbounded growth.
15. As a user, I want a new track lifetime to remain separate from an older lifetime with the same ICAO address.
16. As a user, I want both trail renderers and both view modes to represent the same historical samples and gaps.
17. As a playback user, I want existing recordings to remain usable after live history ownership changes.
18. As a maintainer, I want a deterministic late-join test using a moving trajectory, so correctness does not depend on a live receiver or real-time waiting.

## Implementation Decisions

### 1. Measurement time and freshness precede history

Carry observation time, receipt time, time-source quality, and observation identity
through decoding, tracking, ingest, and fusion. Preserve freshness independently
for position, altitude, and velocity. Metadata-only messages may update contact
liveness without refreshing unrelated measurements.

BEAST receiver ticks are not UTC. Establish an explicit per-source clock anchor
for live reception and use capture-time metadata for replay. Handle receiver
restart, rollover, unavailable timestamps, and feed reconnection explicitly.
When only receipt time is available, record that limitation rather than claiming
an exact measurement time. Keep UTC labels distinct from monotonic scheduling.

Replace periodic re-ingestion of cached contacts with genuinely new observations.
Identity must distinguish separate reports with equal timestamps and separate
payloads decoded from one frame. Repeated polling of the same report is not a new
measurement. Fresh reports from a stationary target are still new measurements.
Deduplication is source-aware so independent sensor reports are not suppressed.

Fusion consumes each accepted observation once, including defined handling for
late observations. Shared injectable time drives prediction, lifecycle, pruning,
and recording in live and deterministic-test modes.

### 2. One projection policy for embedded and headless modes

Both modes must supply the same raw-observation hints and use the same
freshness-aware raw-versus-fused projection policy. Today embedded mode supplies
raw hints and the headless agent supplies none. Resolve this difference before
recording history.

Historical samples describe the authoritative state at their state time, before
client interpolation. Client frame rate, model loading, selection, and trail
visibility must not affect their values or collection. Ground speed means
horizontal speed; vertical speed and airspeed retain separate meanings. Preserve
altitude reference where known and distinguish true zero from missing telemetry.

### 3. Shared historical data model and recorder

Define the wire-safe historical vocabulary in `airjedi-core`. Put the recorder
and projection integration in shared agent-side library code usable by embedded
and headless applications. Keep the history interface independent of networking
and rendering. Existing short ADS-B position history, fusion observation storage,
and filter rollback history keep their separate responsibilities.

The conceptual sample and metadata contract includes:

| Area | Required information |
| --- | --- |
| Identity | Server session, stable `TrackId`, sample sequence |
| Time | State time; relevant observation times and their time-source quality |
| Position | Latitude, longitude, optional altitude and known altitude reference |
| Telemetry | Optional ground speed, track/heading with defined reference, vertical rate; roll and turn rate when supplied |
| Provenance | Source and observed/fused/prediction-only treatment, including per-field freshness |
| Continuity | Segment identity or an explicit break reason |
| Coverage | Track first-seen time, actual retained range, sampling interval, truncation reason |
| Synchronization | Current history revision and retained sequence range |

Start with a configurable **two-second sampling cadence**, approximately 900
regular samples for 30 minutes. Sampling is independent of display frames and
network ticks. A scheduling pause must not fabricate missed observations.
Represent important continuity changes even when they occur between samples.

Specify both per-track and global memory bounds. Pruning is an active system,
not just an available helper, and runs even without connected clients. Report
shortened coverage if a resource limit evicts data before the requested window.
Client retention also stays bounded independently of rendering filters.

Use stable sample identity for bounded retrospective corrections. A sample
sequence orders samples; a separate monotonically increasing history revision
orders append, correction, and prune operations. A snapshot records both its
sample cutoff and its revision. This lets an update to an older sample survive
the snapshot-to-live handoff.

Correction scope must fit the state that can actually be reconstructed by the
fusion pipeline. T4 must define and test that horizon before publication of the
wire contract. Outside it, preserve the historical record and apply accepted
late information to subsequent state; do not silently invent older estimates.

### 4. Explicit lifecycle and track identity

Key all history and in-flight requests by server session and `TrackId`, never
by an ECS entity number or ICAO alone. Use the same identity for selection and
the associated historical detail view.

Coasting samples are explicitly estimated. A substantial coverage gap, filter
reinitialization, or reacquisition discontinuity starts a new segment. Reacquiring
the same surviving track can retain earlier segments; a new `TrackId` is a new
lifetime. Removed tracks release retained history and pending transfers. Define
the relationship between `Lost`, display removal, and final track cleanup in T4
so a delayed response cannot restore a removed entity.

### 5. Visible-first synchronization on the existing connection

Use the existing Replicon/Renet connection. Maintain a small bounded
`DisplayTrail`-style preview for initial visibility, backed by the same canonical
history as full transfer. The preview has a sequence range, revision, and actual
coverage metadata. Full history is sent in bounded chunks; steady-state updates
carry small append/correction/prune operations. Measure preview update costs as
well as full-history costs; do not make the preview an unbounded hot vector.

Synchronization proceeds per track:

1. Receive current display state, server time reference, and history availability.
2. Make the visible preview available and schedule full-history transfer, with
   selection priority and bounded background work.
3. Capture a consistent snapshot and revision watermark atomically with the
   live-update handoff. Register/buffer later operations from that watermark.
4. Transfer chunks tagged with session, track, request, cutoff, revision, and
   completeness metadata. Bound server snapshot storage and client assembly.
5. Install the completed snapshot and apply buffered operations newer than its
   revision. Merge the preview by identity without duplicate samples.
6. Continue live updates, pruning, and explicit resynchronization on gaps or
   exhausted buffers.

Replicon's ordered messages do not make component value mutations and history
messages one atomic stream. Its entity-lifecycle ordering is useful, but the
history contract still needs its own tested watermark and revision handling.

Selection changes reprioritize work. Valid data for an active track may populate
the background cache; discard explicitly cancelled or superseded requests, old
server sessions, and responses for removed track lifetimes. Changing selection
alone must not contradict the agreed background-fill behavior.

Use bounded concurrency, per-client pacing, and fair scheduling. Current display
updates take priority over history downloads. Snapshot copies, queued operations,
and slow-client recovery all count toward resource limits. Bump the wire protocol
version and make mismatched client/server versions fail clearly.

### 6. Rendering, charts, and playback

Materialize received history into the client ECS. Both trail renderers and charts
consume the same historical record. Remove client-authoritative recording for
live agent-owned tracks; keep an explicit playback adapter with existing recording
format compatibility.

Apply received history regardless of whether visual/model initialization has
finished. Hydration must be retryable and must not overwrite an already loaded
history with a default empty value.

Use the agent's time reference, extrapolated with a monotonic client clock, for
history age and fading. Preserve original timestamps and expose UTC/local labels
only as presentation. Test clock skew, wall-clock adjustments, and suspension.

Both gizmo and mesh-strip renderers must honor actual segment breaks, estimated
intervals, and missing altitude. Rebuild geometry correctly after corrections,
origin recentering, and 2D/3D changes. Keep geographic history independent of the
floating rendering origin. Selected tracks may display their longer available
history; unselected tracks use the configured visible window.

Add charts in the active inline detail UI. Show a true time axis, 5/15/30-minute
windows, units, timestamp/value hover details, gaps, and estimated styling. Any
display downsampling preserves gaps and important extrema. Distinguish loading,
partial transfer, complete available coverage, empty history, and retryable error.
Track age and retained coverage are separate metrics.

## Testing Decisions

The primary test interface is a timestamped trajectory entering the agent and
its history appearing in a fresh client ECS, then being consumed by trail/chart
views. Existing ingest replay, fusion-to-display snapshots, and replication
round-trip tests provide the starting points. Test externally visible values,
timestamps, sequence continuity, coverage, and resource bounds.

The main acceptance scenario uses an injectable clock: collect a moving track
for 20 simulated minutes, connect a fresh client, verify pre-connection history,
continue live updates, interrupt the client, and reconnect. A continuously
connected client and a late client converge to the same retained history once
their downloads and revisions have caught up.

Additional coverage includes stationary and telemetry-only updates; identical
timestamps from distinct sources; duplicates; late data; coasting; reacquisition;
track deletion during loading; cancelled/superseded requests; selection changes;
retention expiry during transfer; revision gaps; clock skew; initial asset-load
delay; embedded/headless parity; playback; and both renderers in both view modes.

Benchmark with declared load profiles, including 100, 500, and 1,000 active
tracks and multiple clients. Record serialized bytes per sample, memory including
in-flight snapshots, first-state/preview latency, selected-history latency, live
update latency during transfer, CPU, and rendering frame time. Confirm budgets
on the target Pi before finalizing chunk size and throughput defaults. These
track counts are test profiles, not claims about current operational traffic.

## Out of Scope

- Persistence across agent restarts or recovery of data from before collection.
- An archive/query UI for removed track lifetimes.
- Replacing the transport with NATS, gRPC, or a new web architecture.
- A full raw-observation archive or unrestricted retrospective smoothing.
- A general playback/recording redesign or unrelated rendering refactor.

## Further Notes

This completes the history portion of Design B. The historical display model
remains keyed by generic track identity, with ADS-B fixtures providing the first
acceptance path. No new vehicle-specific adapters are required by this feature.

The existing architecture ADRs describe the earlier embedded pipeline. The
shared recorder and client read-only treatment are deliberate changes to that
flow; refresh those descriptions as part of final integration.

Implementation authorization follows the repository's human-only `agent:build`
gate. Readiness labels and dependency links describe planning status, not build
authorization. Implementation tickets, their dependencies, and team handoffs
are maintained in the companion plan.
