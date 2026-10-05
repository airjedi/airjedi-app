# ADR-0002: Data Flow Overview

## Status

Accepted — descriptive snapshot, companion to [ADR-0001](0001-architecture-overview.md). Where ADR-0001 shows *what's wired into the ECS* (structure), this ADR shows *how data actually moves through it* (behavior).

## Context

ADR-0001 organizes the desktop app around its plugins. That's the right view for "where do I add a new feature," but it doesn't answer "what happens between an ADS-B packet arriving and an aircraft icon moving on screen?" This ADR traces the three real-time data pipelines the app runs concurrently, every frame, plus the feedback loop that closes them back to user input.

All three pipelines are produced by background threads or async tasks — they never block the main Bevy schedule — and each crosses into the single ECS `World` through a mutex or a channel that gets drained once per frame. That crossing point is the seam between "the outside world" and "the ECS."

## Diagram

![AirJedi data flow: three horizontal lanes (Aircraft Tracking, Map & Tiles, Aviation & Weather Data) each flowing left to right through External Sources, Ingest/Decode, Process/Transform, ECS World, Render Systems, and a shared Screen output, with a dashed feedback loop from user input back into the Map & Tiles lane and the ECS World's shared resources.](assets/0002-data-flow-overview.svg)

## The three pipelines

### Lane 1 — Aircraft Tracking (the hero flow)

This is the most involved pipeline and the reason the fusion engine exists.

1. **External source**: an ADS-B feed — a TCP socket speaking SBS-1 (BaseStation, port 30003) or BEAST binary (port 30005), or a local SDR dongle.
2. **Ingest/decode**: the `adsb-client` crate runs its own background thread per configured feed, pushing bytes through its internal `Transport → Framing → Decoder → Tracker` pipeline. The result lands in `FeedConnectionManager`, a mutex-shared `Vec<Aircraft>` per feed — this is raw, per-feed, un-fused truth.
3. **Process/transform** (runs every frame, in this exact order via `FusionSet`):
   - `adsb_to_fusion_system` converts each feed's `Aircraft` into a `SensorObservation` (position, velocity, covariance) and pushes it to `ObservationBuffer`.
   - **Drain**: `ObservationBuffer` empties into `TimelineStore`.
   - **Associate**: a spatial index + GNN (Global Nearest Neighbor) associator matches unassociated observations to existing `Track` entities.
   - **Fuse**: each `Track`'s Kalman filter (EKF/UKF/IMM, chosen per target category) predicts forward by `dt`, then updates against any newly-associated observations.
   - **Lifecycle**: track status transitions (Confirmed/Coasting/Lost), new-track initiation for unmatched observations (M-of-N gate), stale-track cleanup, and store eviction.
4. **ECS entities**: the fused `Track` + `TrackerState` *is* an ECS entity — the fusion engine has no separate world, it's plugins in the same `App`. `render_bridge::sync_tracks_to_visuals` reads changed tracks, blends in the freshest raw feed data (callsign, squawk, alert flags — raw wins when live, filter-predicted wins while coasting), and creates/updates a linked visual `Aircraft` entity (`Transform`, `TrailHistory`, `InterpolationState`) via `FusionTrackLink`.
5. **Render systems**: interpolation, trail rendering, staleness/altitude coloring, prediction cones, and mouse picking — all reading the `Aircraft` entity each frame.
6. **Screen**: the 3D model renders via `AircraftCamera`; the text label renders via `MapCamera`.

### Lane 2 — Map & Tiles

1. **External source**: a basemap tile server (XYZ/WMTS HTTP endpoint).
2. **Ingest**: the tile download system checks `CachedTileSet`/`DownloadedTiles` first; only a cache miss triggers an HTTP GET.
3. **Process**: successful downloads are saved to the disk `tile_cache` and fire a `TileReady` message; the render system decodes the image into a texture.
4. **ECS entities**: a pooled `Mesh3d` tile entity, positioned using Web Mercator math relative to the floating `LocalOrigin`.
5. **Render**: the tile pool activates a reused mesh slot (no per-tile spawn/despawn churn).
6. **Screen**: rendered as a plane via `AircraftCamera`.

### Lane 3 — Aviation & Weather Data

1. **External sources**: half a dozen APIs — AviationWeather.gov (METAR/TAF/SIGMET/AIRMET/PIREP), OurAirports, FAA NASR, OpenAIP, NOTAMs/TFRs, FAA class airspace.
2. **Ingest**: a single background scheduler thread (`tokio-cron-scheduler`) runs each provider's `fetch()` on its own cron schedule — this is the one pipeline that isn't purely event-driven.
3. **Process**: each provider's `Fetch → Parse → Transform` pipeline stages produce `CanonicalRecord`s, which cross a `crossbeam` channel; `drain_ingest_channel` classifies them each frame into `WeatherDataUpdated` / `NavigationDataUpdated` / `NoticeDataUpdated` Bevy messages. Raw responses are also cached to disk as JSON.
4. **ECS entities/resources**: `AviationPlugin`/`AirspacePlugin`/`WeatherPlugin` consume those messages to spawn or update airport/runway/navaid/airspace entities and weather resources.
5. **Render**: the corresponding plugin's render systems draw the reference layers.
6. **Screen**: airspace meshes via `AircraftCamera`; weather icons and most overlays via `MapCamera`.

## The feedback loop

The map is interactive, so the flow isn't strictly one-directional. `User Input` (mouse drag/scroll, keyboard) does two things directly, both shown as dashed feedback arrows:

- Updates the **shared resources** (`MapState`, `ZoomState`) that both the Aircraft and Tile lanes read for coordinate conversion.
- Directly triggers new `DownloadTilesRequest`s in the **Map & Tiles** lane's ingest stage when the visible area changes.

The Data Sources panel can also queue an on-demand fetch for a specific provider, re-entering Lane 3 at the scheduler stage outside its normal cron schedule (noted on the User Input box, not drawn as a fourth arrow to avoid clutter).

## Shared resources

`MapState`, `ZoomState`, `AppConfig`, and `LocalOrigin` sit inside the ECS World but aren't owned by any single lane — the Aircraft lane uses `LocalOrigin` for lat/lon-to-world conversion, and the Tile lane uses `MapState`/`ZoomState` to know what to fetch. They're drawn once, in the gap between Lane 1 and Lane 2, rather than duplicated per lane.

## Non-goals

- This diagram shows steady-state per-frame flow, not one-time startup sequencing. `config.toml → AppConfig` happens once at boot and seeds the initial feed list, map center, and tile endpoint for all three lanes — it isn't drawn as a fourth pipeline because it doesn't recur.
- System *ordering* within a single `Update` schedule (as opposed to data *flow* across threads/stages) is covered in `CLAUDE.md`'s "System Ordering" section.
- Camera/render-layer assignment detail is covered in `CLAUDE.md`'s "Camera Architecture" section; this ADR only notes which camera each lane's output ultimately reaches.

## Related

- [ADR-0001: Desktop App Architecture Overview](0001-architecture-overview.md) — the structural/component companion to this diagram.
- `CLAUDE.md` — System Ordering, Camera Architecture, Live Data / Supported Protocols sections.
