# ADR-0001: Desktop App Architecture Overview

## Status

Accepted — descriptive snapshot, not a design decision to be revisited. Kept as ADR-0001 so it's the first thing an agent or new contributor reads before touching `src/`.

## Context

`airjedi-app` is a Cargo workspace with four members, but only one of them — `airjedi_bevy` (the code in `src/`) — is the desktop map tracker itself:

| Workspace member | Role |
|---|---|
| `airjedi_bevy` (`src/`) | The Bevy desktop app. **In scope for this diagram.** |
| `airjedi-fusion` | Sensor-fusion library (EKF/UKF/IMM filters, track association). Plugged into the desktop app via `FusionIntegrationPlugin`, but also has its own NATS transport and can run headless. |
| `crates/adsb-client` | ADS-B protocol decoder/tracker (SBS-1 + BEAST framing). Used by the desktop app's `adsb`/`data_ingest` modules. |
| `airjedi-web` | A separate Rust web server (`/api/*`, `/ws`) + TypeScript globe client, fed via NATS. A browser-based companion, not part of the desktop app. |

This ADR diagrams only `airjedi_bevy`'s internals. `airjedi-fusion` and `adsb-client` appear as external dependency boxes because the desktop app genuinely depends on them at compile time. `airjedi-web` is omitted entirely — it doesn't share a process, a `World`, or a build target with the desktop app.

The app is built on Bevy's ECS: a single `App` owns one `World`, and roughly two dozen `Plugin`s (registered in `src/main.rs`) add resources, components, and systems into that shared `World`. There is no other central coordinator — the ECS *is* the architecture. Everything in this diagram is a satellite of it.

## Diagram

![AirJedi desktop app architecture: a central "Bevy App / ECS World" hub connected to seven surrounding plugin clusters (Rendering & Camera, Map & Tiles, Aircraft & Live Tracking, Aviation Reference Data, Tools & Session, UI Shell, Support/Infra), with two external crates (adsb-client, airjedi-fusion) shown as dashed dependency boxes off the Aircraft & Live Tracking cluster.](assets/0001-architecture-overview.svg)

## Cluster reference

| Cluster | Plugins / modules | Purpose |
|---|---|---|
| **ECS Core** | `App`, `World`, Schedules (`Startup` → `Update` → `EguiPrimaryContextPass`), core resources (`MapState`, `ZoomState`, `AppConfig`, `LocalOrigin`, `Tile*Settings`), `ZoomSet::Change` ordering | The single source of truth every plugin reads from and writes into. |
| **Rendering & Camera** | `CameraPlugin` (3-camera rig: `AircraftCamera` 3D, `MapCamera` 2D overlay, dedicated UI camera), `View3DPlugin` (2D/3D dolly-zoom transitions, orbit camera, sky/fog), `InputPlugin` (pan/drag, Mercator conversion), `render_layers` | Owns the three-camera compositing pipeline and view-mode transitions. |
| **Map & Tiles** | `TilesPlugin` (download/render/pool/prefetch), `ZoomPlugin` (two-tier zoom: continuous camera zoom + discrete tile zoom), `tile_cache` (disk cache, symlinked into `assets/`), `TerrainPlugin` (elevation/heightmaps), `LocationPlugin`, `geo` (`CoordinateConverter`, haversine) | The slippy-map tile system and Web Mercator coordinate math. |
| **Aircraft & Live Tracking** | `AircraftPlugin` (components, trails, picking, prediction, staleness), `AdsbPlugin` (live ADS-B feeds via `adsb-client`), `FusionIntegrationPlugin` (bridges the `airjedi-fusion` crate's EKF/UKF/IMM filters into ECS entities), `DataIngestPlugin` (provider pipeline), `DataSourcesPlugin` (feed configuration UI) | The core domain: turning raw ADS-B/BEAST frames into tracked, rendered aircraft. |
| **Aviation Reference Data** | `AviationPlugin` (airports/navaids/runways), `AirspacePlugin` (airspace boundaries), `WeatherPlugin` (METAR), `CoveragePlugin` (ADS-B coverage viz) | Static/slow-changing reference layers drawn on the map. |
| **Tools & Session** | `ToolsPlugin` (measurement tools), `RecordingPlugin` (record/playback), `BookmarksPlugin` (saved locations), `ExportPlugin` (data export) | User-facing utilities that operate on a session but aren't core tracking. |
| **UI Shell (egui)** | `dock`, `toolbar`, `statusbar`, `theme`, `widgets`, `tools_window`, `ui_panels`, `keyboard`, `inspector`, `debug_panel`, `hud`, `debug_3d_hud` | The `egui_tiles`-based dock, toolbar/statusbar, Catppuccin theming, and debug overlays. Plain modules, not Bevy plugins in the strict sense, but wired into the same `EguiPrimaryContextPass` schedule. |
| **Support / Infra** | `ConfigPlugin` (TOML persistence), `paths` (bundle-aware path resolution), `build_info` (shadow-rs build metadata), `brp` (optional Bevy Remote Protocol, feature-gated — dashed box) | Cross-cutting infrastructure with no domain logic of its own. |
| **External crates** *(dashed, outside the app boundary)* | `adsb-client` (SBS-1 + BEAST protocol decode/tracker), `airjedi-fusion` (EKF/UKF/IMM filters, track association) | Separate workspace crates the desktop app depends on but doesn't own. |

## Non-goals

- This is a **component** diagram, not a data-flow or sequence diagram. It shows *what registers into the ECS*, not the frame-by-frame order systems run in (see `CLAUDE.md`'s "System Ordering" section for that).
- `airjedi-web` and its NATS-based integration with `airjedi-fusion` are intentionally out of scope.
- Individual system-level detail (e.g. the tile pooling/culling logic, the 3-camera render-layer assignments) is documented in `CLAUDE.md` under "3D Tile Rendering — Known Pitfalls" and "Camera Architecture"; this ADR stays at the plugin/module level.

## Related

- `CLAUDE.md` — Module Map, Domain Modules, and Camera Architecture tables (source of truth this diagram was derived from).
- `docs/agents/domain.md` — describes how agents should consume ADRs and `CONTEXT.md` in this repo.
