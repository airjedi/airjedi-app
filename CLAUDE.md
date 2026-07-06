# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Agent Workflow

Agent skills in `.claude/skills/` handle issue triage, implementation,
and PR review. Run them manually from Claude Code in this repo.

**Usage:**
- Triage an issue: invoke triage-issue skill with an issue number
- Build from issue: apply `agent:build` label, then invoke build-from-issue
- Review a PR: invoke review-pr skill with a PR number

**Labels:**
- `agent:build` - human-only gate that authorizes agent implementation
- `agent:skip` - tells agents to ignore an issue
- `type:*` and `priority:*` - set by triage agent
- `state:*` - tracks agent workflow progress

**Branch convention:** Agent branches use `claude/<issue>-description`.

**PR convention:** Agent PRs include `Closes #<issue-number>` in the body.

**Commit style:** Plain imperative (e.g., "Fix altimeter QNH transition").
No conventional commit prefixes, no emojis.

## Project Overview

AirJedi is an aircraft map tracker built with the Bevy 0.19 game engine and an integrated tile rendering system. The application displays aircraft positions on an interactive slippy map (OpenStreetMap-based) with pan, zoom, 2D/3D view modes, and real-time position tracking capabilities.

## Build and Run Commands

```bash
# Build the project
cargo build

# Run the application
cargo run

# Build with release optimizations
cargo build --release

# Run with release optimizations
cargo run --release

# Run with debug logging for airjedi
RUST_LOG=airjedi_bevy=debug cargo run

# Generate and open documentation
cargo doc --open
```

Note: The project uses custom optimization profiles in Cargo.toml - dev mode has opt-level 1 for the main crate but opt-level 3 for dependencies to improve compile-time performance.

## macOS Application Bundle

```bash
# Generate app icon from SVG (requires: brew install librsvg)
cd macos && make icons

# Build AirJedi.app bundle (release mode)
cd macos && make app

# Build and launch
cd macos && make run

# Clean build artifacts
cd macos && make clean
```

The `macos/` directory contains all macOS-specific build files. Assets are copied into `Contents/MacOS/assets/` inside the bundle (where Bevy's AssetPlugin looks). Tile cache uses `~/Library/Caches/airjedi/tiles/` in both development and bundle modes. The `src/paths.rs` module handles bundle-aware path resolution.

## Architecture

The application uses Bevy's ECS architecture with a plugin-based modular design. `src/main.rs` wires plugins together and defines shared constants, coordinate helpers, and the `setup_map` startup system.

### Module Map

| Module | Purpose |
|--------|---------|
| `src/main.rs` | App setup, plugin registration, constants, `setup_map` |
| `src/camera.rs` | Dual-camera system (AircraftCamera 3D order 0 + MapCamera 2D overlay order 1), aircraft position/label updates |
| `src/zoom.rs` | Two-tier zoom: continuous camera zoom + discrete tile zoom levels |
| `src/input.rs` | Pan/drag handling with Mercator projection conversion |
| `src/tiles/` | Integrated tile system: download, render, pool, prefetch, coords, elevation |
| `src/tile_cache.rs` | Centralized tile cache in `~/Library/Caches/airjedi/tiles/`, symlinked into assets |
| `src/geo.rs` | `CoordinateConverter`, haversine distance, lat/lon-to-world helpers |
| `src/config.rs` | `AppConfig` persistence (TOML), basemap style, settings UI state |
| `src/theme.rs` | Catppuccin-based theming, egui style application |
| `src/dock.rs` | `egui_tiles`-based dock layout with tabbed panels |
| `src/toolbar.rs` | Top toolbar rendering |
| `src/statusbar.rs` | Bottom status bar (connection, zoom, coordinates) |
| `src/keyboard.rs` | Keyboard shortcuts and help overlay |
| `src/inspector.rs` | `bevy-inspector-egui` integration |
| `src/debug_panel.rs` | FPS, frame time, entity count diagnostics |
| `src/render_layers.rs` | `RenderCategory` constants for layer separation |
| `src/paths.rs` | Bundle-aware path resolution (dev vs macOS .app) |
| `src/map.rs` | `MapState` and `ZoomState` resource definitions |
| `src/units.rs` | Unit conversion helpers |
| `src/ui_panels.rs` | `UiPanelManager` and `PanelId` for panel visibility |
| `src/tools_window.rs` | Consolidated tools/settings window |

### Domain Modules (each has its own Plugin)

| Module | Plugin | Purpose |
|--------|--------|---------|
| `src/aircraft/` | `AircraftPlugin` | Components, trails, picking, prediction, staleness, altitude coloring, list/detail/stats panels, emergency alerts, type database, hanabi particle effects (optional feature) |
| `src/adsb/` | `AdsbPlugin` | Live ADS-B data via `adsb-client` crate, aircraft sync, 3D model loading |
| `src/aviation/` | `AviationPlugin` | Airport, navaid, and runway data loading and rendering |
| `src/view3d/` | `View3DPlugin` | 2D/3D mode transitions, orbit camera, atmosphere/sky/fog, sun/moon positioning, day/night cycle |
| `src/weather/` | `WeatherPlugin` | METAR fetching and weather indicators |
| `src/recording/` | `RecordingPlugin` | Flight recording and playback |
| `src/bookmarks/` | `BookmarksPlugin` | Map location bookmarks |
| `src/tools/` | `ToolsPlugin` | Measurement tools |
| `src/coverage/` | `CoveragePlugin` | ADS-B coverage visualization |
| `src/airspace/` | `AirspacePlugin` | Airspace boundary display |
| `src/data_sources/` | `DataSourcesPlugin` | Data source management |
| `src/export/` | `ExportPlugin` | Data export functionality |
| `src/data/` | — | Background data downloading |

### Camera Architecture

The app uses three cameras:

1. **AircraftCamera** (`Camera3d`, order 0): Primary renderer for tiles (as Mesh3d planes), aircraft, sky dome, ground plane. Uses orthographic in 2D mode, perspective in 3D mode. Clears with dark background. Layers: DEFAULT(0), TILES(1), GROUND(7), SKY(8), AIRSPACE(9).
2. **MapCamera** (`Camera2d`, order 1): Overlay for gizmos, labels, overlays. Alpha-blends on top of AircraftCamera output. Never renders tiles. Layers: GIZMOS(2), OVERLAYS_2D(4), LABELS(5).
3. **UI Camera** (`Camera2d`, order 100): Dedicated egui camera, layer 11 only.

In 3D mode, AircraftCamera uses perspective projection. In 2D mode, both cameras use orthographic. `manage_camera_mode` in `src/view3d/sky.rs` handles fog and ground plane toggling on mode transitions.

### Key Resources

- `MapState`: Current map center (lat/lon) and discrete zoom level
- `ZoomState`: Continuous camera zoom (0.1x–10x)
- `View3DState`: 3D view mode, camera orbit (pitch/yaw/altitude), transition state
- `AppConfig`: Persisted settings (basemap, default location, zoom)
- `DockTreeState`: `egui_tiles` dock layout state

### Coordinate System

- All world positions use zoom-independent Web Mercator meters (EPSG:3857)
- `LocalOrigin` (floating origin) keeps f32 values near zero; recenters when camera moves >25km
- `CoordinateConverter` in `src/geo.rs` converts lat/lon to local meters via `LocalOrigin`
- 2D mode: X=east, Y=north, Z=layer depth (Z-up)
- 3D mode: X=east, Y=altitude, Z=-north (Y-up, via `zup_to_yup` in view3d)
- `WEB_MERCATOR_EXTENT = 20037508.342789244` meters (half Earth circumference)
- Latitude clamped to +/-85.0511 (Mercator limit), longitude to +/-180
- Default center: Wichita, KS (37.6872, -97.3301)
- Altitude uses 50x exaggeration: `altitude_to_z(feet) = feet * 0.3048 * altitude_scale`

### System Ordering

Systems that modify `MapState::zoom_level` in 3D mode run in `ZoomSet::Change`. Position-dependent systems (aircraft, airports, navaids, camera) run `.after(ZoomSet::Change)`.

## Dependencies

Key dependencies (see `Cargo.toml` for full list):

- `bevy = "0.19"` with `jpeg` feature: Game engine
- `bevy_egui = "0.40"` + `bevy-inspector-egui = "0.37"`: egui UI integration and entity inspector
- `bevy_obj = "0.19"`: OBJ model loading
- `bevy_brp_extras = "0.20"` (optional `brp` feature, default on): BRP extras for remote inspection, screenshots, input simulation
- `adsb-client`: Local crate for ADS-B SBS1 protocol parsing
- `tokio`: Async runtime for ADS-B network connections
- `reqwest`: HTTP client (blocking + json) for METAR and data downloads
- `serde` + `serde_json` + `toml`: Serialization for config and data
- `chrono`: Date/time handling for sun position and recordings
- `solar-positioning`: Sun elevation/azimuth calculations
- `csv`: Airport/navaid data parsing
- `dirs`: Platform-specific directory paths

## Live Data

Aircraft data comes from a live ADS-B feed via the `adsb-client` crate connecting to an SBS1 server (default: `98.186.33.60:30003`). Aircraft are synced in real-time with a 180-second staleness timeout and 250-mile max distance filter. Connection settings are in `src/main.rs::constants`.

## Map Tile Caching

Tiles are cached in `~/Library/Caches/airjedi/tiles/` (centralized), symlinked into `assets/tiles/` for Bevy's AssetPlugin. Naming format: `{zoom}.{x}.{y}.{tile_size}.tile.png`
- Clear cache via in-app button or `tile_cache::clear_tile_cache()`
- Cache can grow large with extensive panning/zooming across multiple zoom levels
- `tile_cache::remove_invalid_tiles()` runs at startup to clean corrupted files

## Bevy Remote Protocol (BRP)

The `brp` feature flag (default-on) enables runtime inspection and control of the app via the Bevy Remote Protocol. The `bevy_brp_mcp` MCP server exposes all BRP and brp_extras capabilities as tools.

- **Feature flag:** `brp` (disable with `--no-default-features -F hanabi`)
- **HTTP endpoint:** `localhost:15702` (default BRP port)
- **MCP server:** Configured in `.mcp.json`, uses `bevy_brp_mcp` binary (stdio)
- **macOS bundle:** BRP is excluded from release app bundles
- **Implementation:** `src/brp.rs` adds `BrpExtrasPlugin` which includes both `RemotePlugin` and `RemoteHttpPlugin`

### App Lifecycle

| Tool | Description |
|------|-------------|
| `brp_status` | Check if app is running with BRP. Returns `running_with_brp`, `running_no_brp`, or `not_running` |
| `brp_launch_bevy_app` | Launch a Bevy app in detached mode with auto-build and logging. Supports debug/release profiles, multi-instance on sequential ports |
| `brp_shutdown` | Graceful shutdown via brp_extras, falls back to process kill |
| `brp_list_bevy_apps` | Discover all Bevy apps in workspace via cargo metadata |
| `brp_list_brp_apps` | Discover BRP-enabled apps specifically |
| `brp_list_bevy_examples` | List all Bevy examples in workspace |
| `brp_launch_bevy_example` | Launch a Bevy example in detached mode |

### Entity & Component Operations

| Tool | Description | Example |
|------|-------------|---------|
| `world_query` | Query entities by component filters | `data: {}, filter: {with: ["bevy_transform::components::transform::Transform"]}` |
| `world_get_components` | Get component data from a specific entity | `entity: 123, components: ["bevy_transform::components::transform::Transform"]` |
| `world_insert_components` | Insert/replace components on an entity | `entity: 123, components: {"bevy_sprite::sprite::Sprite": {color: ...}}` |
| `world_mutate_components` | Update specific fields without replacing the whole component | `entity: 123, component: "...Transform", path: ".translation.y", value: 10.5` |
| `world_remove_components` | Remove components from an entity | `entity: 123, components: ["bevy_sprite::sprite::Sprite"]` |
| `world_list_components` | List all registered component types, or components on a specific entity | `entity: 123` (optional) |
| `world_spawn_entity` | Spawn a new entity with components, returns entity ID | `components: {"...Transform": {translation: {x:0,y:0,z:0}, ...}}` |
| `world_despawn_entity` | Permanently remove an entity | `entity: 123` |
| `world_reparent_entities` | Change parent of entities (or remove parent) | `entities: [123, 124], parent: 100` |

**Mutation path syntax:** `.field.nested`, `.points[2]` (array), `.0` (tuple). Leading dot required.

### Resource Operations

| Tool | Description | Example |
|------|-------------|---------|
| `world_get_resources` | Get resource data | `resource: "bevy_time::time::Time"` |
| `world_insert_resources` | Insert or replace a resource | `resource: "my::Config", value: {difficulty: "hard"}` |
| `world_mutate_resources` | Update specific fields in a resource | `resource: "my::Config", path: ".volume", value: 0.5` |
| `world_remove_resources` | Remove a resource (WARNING: may break systems) | `resource: "my::TempCache"` |
| `world_list_resources` | List all registered resources | (no params) |

### Events

| Tool | Description | Example |
|------|-------------|---------|
| `world_trigger_event` | Trigger a registered event globally | `event: "my_game::PauseGame"` or with payload: `event: "my_game::SpawnEnemy", value: {enemy_type: "goblin"}` |

Events must be registered with `#[derive(Event, Reflect)]` and `#[reflect(Event)]`.

### Type Introspection

| Tool | Description | Notes |
|------|-------------|-------|
| `brp_type_guide` | Get spawn/insert/mutate examples for specific types | Best for targeted type lookup |
| `brp_all_type_guides` | Get guides for all registered Components and Resources | Can be large |
| `registry_schema` | Get full type schemas with properties | **WARNING: 200k+ tokens unfiltered.** Always use `with_crates` or `with_types` filters |
| `rpc_discover` | Discover all available BRP methods (OpenRPC spec) | Useful for debugging connectivity |
| `brp_execute` | Execute any arbitrary BRP method | `method: "world.query", params: {...}` |

**Schema filter examples:**
- `with_crates: ["airjedi_bevy"]` — app-specific types only
- `with_types: ["Component"]` — components only
- `with_crates: ["bevy_transform"], with_types: ["Component"]` — Transform components

### Input Simulation (brp_extras)

#### Keyboard

| Tool | Description | Example |
|------|-------------|---------|
| `brp_extras_send_keys` | Send key press/release (simultaneous — for shortcuts) | `keys: ["ShiftLeft", "KeyA"]` for Shift+A; `keys: ["Space"], duration_ms: 2000` to hold |
| `brp_extras_type_text` | Type text sequentially (one char per frame — for text input) | `text: "hello world"` |

**Key names:** `KeyA`-`KeyZ`, `Digit0`-`Digit9`, `F1`-`F24`, `ShiftLeft/Right`, `ControlLeft/Right`, `AltLeft/Right`, `SuperLeft/Right` (Cmd on macOS), `Enter`, `Tab`, `Space`, `Backspace`, `Delete`, `Escape`, `ArrowUp/Down/Left/Right`, `Home`, `End`, `PageUp`, `PageDown`.

#### Mouse

| Tool | Description | Example |
|------|-------------|---------|
| `brp_extras_move_mouse` | Move cursor (absolute or relative) | `position: [200, 150]` or `delta: [50, 30]` |
| `brp_extras_click_mouse` | Click a button (press + release, 100ms) | `button: "Left"` |
| `brp_extras_double_click_mouse` | Double click | `button: "Left", delay_ms: 250` |
| `brp_extras_send_mouse_button` | Press-hold-release a button | `button: "Left", duration_ms: 500` |
| `brp_extras_drag_mouse` | Smooth drag from start to end | `button: "Left", start: [100,100], end: [300,200], frames: 30` |
| `brp_extras_scroll_mouse` | Mouse wheel scroll | `x: 0, y: 5, unit: "Line"` or `unit: "Pixel"` |

**Buttons:** `Left`, `Right`, `Middle`, `Back`, `Forward`.

#### Trackpad Gestures (macOS)

| Tool | Description | Example |
|------|-------------|---------|
| `brp_extras_pinch_gesture` | Pinch to zoom | `delta: 2.5` (zoom in), `delta: -1.5` (zoom out) |
| `brp_extras_rotation_gesture` | Rotation gesture (radians) | `delta: 0.5` (clockwise) |
| `brp_extras_double_tap_gesture` | Double tap gesture | (no params) |

### Visual & Diagnostics (brp_extras)

| Tool | Description | Example |
|------|-------------|---------|
| `brp_extras_screenshot` | Capture screenshot to file | `path: "tmp/screenshot.png"` |
| `brp_extras_get_diagnostics` | Get FPS, frame time, frame count | Returns current/average/smoothed values |
| `brp_extras_set_window_title` | Change the window title | `title: "AirJedi - Debug"` |

### Watch/Monitoring

| Tool | Description |
|------|-------------|
| `world_get_components_watch` | Watch entity component value changes, logs to file | `entity: 123, types: ["...Transform"]` |
| `world_list_components_watch` | Watch component additions/removals on an entity | `entity: 123` |
| `brp_list_active_watches` | List all active watch subscriptions with log paths |
| `brp_stop_watch` | Stop a watch by ID | `watch_id: 1` |

Watch logs are written to `/tmp/bevy_brp_mcp_watch_*.log`. Always stop watches when done to free resources.

### Log Management

| Tool | Description |
|------|-------------|
| `brp_list_logs` | List BRP log files (newest first). Use `verbose: true` for full details |
| `brp_read_log` | Read log contents. Filter with `keyword` or `tail_lines` |
| `brp_delete_logs` | Delete logs. Filter by `app_name` or `older_than_seconds` |

### Common Debugging Workflows

**Check app status and take a screenshot:**
```
brp_status(app_name: "airjedi-bevy") → brp_extras_screenshot(path: "tmp/debug.png")
```

**Find and inspect a specific entity:**
```
world_query(data: {}, filter: {with: ["airjedi_bevy::camera::MapCamera"]})
→ world_get_components(entity: <id>, components: ["bevy_transform::components::transform::Transform"])
```

**Inspect app-specific resources:**
```
registry_schema(with_crates: ["airjedi_bevy"], with_types: ["Resource"])
→ world_get_resources(resource: "airjedi_bevy::map::MapState")
```

**Simulate user interaction (pan the map):**
```
brp_extras_move_mouse(position: [400, 300])
→ brp_extras_drag_mouse(button: "Left", start: [400,300], end: [200,300], frames: 20)
```

**Monitor an entity's transform over time:**
```
world_get_components_watch(entity: 123, types: ["bevy_transform::components::transform::Transform"])
→ brp_read_log(filename: <log_file>)
→ brp_stop_watch(watch_id: <id>)
```

**Get performance diagnostics:**
```
brp_extras_get_diagnostics() → returns FPS current/avg/smoothed, frame_time_ms, frame_count
```

**Component types use fully-qualified paths.** Use `world_list_components()` or `registry_schema(with_crates: ["airjedi_bevy"])` to discover available types. Common Bevy types:
- `bevy_transform::components::transform::Transform`
- `bevy_render::camera::camera::Camera`
- `bevy_sprite::sprite::Sprite`
- `bevy_core::name::Name`

### Reference Test View — 3D Wichita at FL300

A saved camera state for reproducible 3D testing. Restore this view directly via BRP resource mutations — no keyboard or scroll simulation needed.

- **Mode:** Perspective3D
- **Map center:** 37.6872, -97.3301 (Wichita, KS)
- **Camera altitude:** 30,000 ft (FL300)
- **Camera pitch:** 25°, yaw: 0°

**To restore via BRP:**
```
world_mutate_resources(resource: "airjedi_bevy::view3d::View3DState", path: ".mode", value: "Perspective3D")
world_mutate_resources(resource: "airjedi_bevy::view3d::View3DState", path: ".camera_altitude", value: 30000)
world_mutate_resources(resource: "airjedi_bevy::view3d::View3DState", path: ".camera_pitch", value: 25)
world_mutate_resources(resource: "airjedi_bevy::map::MapState", path: ".latitude", value: 37.6872)
world_mutate_resources(resource: "airjedi_bevy::map::MapState", path: ".longitude", value: -97.3301)
```

**BRP-accessible view resources:**
- `airjedi_bevy::view3d::View3DState` — mode, camera_altitude, camera_pitch, camera_yaw, altitude_scale, visibility_range, atmosphere_enabled
- `airjedi_bevy::map::MapState` — latitude, longitude
- `airjedi_bevy::map::ZoomState` — camera_zoom, min_zoom, max_zoom

## 3D Tile Rendering -- Known Pitfalls

The integrated tile system in `src/tiles/` has several interacting subsystems. Read this before changing tile display, zoom transitions, or culling logic.

### Architecture

- **Integrated tile system**: Replaced `bevy_slippy_tiles` with `src/tiles/` module containing coords, download, render, pool, prefetch, and elevation submodules.
- **Zoom-independent positioning**: Tiles use Web Mercator meters via `tile_to_mercator_aabb()`. Position is fixed regardless of zoom level - zoom only affects which tiles exist and their mesh size.
- **SSE-driven LOD**: In 3D mode, `zoom_for_distance()` computes the ideal zoom level for each tile based on its screen-space error (distance from camera vs tile size in pixels). Near tiles get high zoom, distant tiles get low zoom.
- **Entity pooling**: `TilePool` reuses entity slots via `activate_tile`/`release_tile` instead of spawning/despawning. `CachedTileSet` provides an in-memory index of disk-cached tiles to avoid filesystem stat calls.
- **Adaptive zoom**: `update_3d_adaptive_zoom` adjusts `map_state.zoom_level` based on camera altitude every 300ms with hysteresis (0.7 upgrade / 0.6 downgrade).
- **Dedup via TileGrid**: `tile_grid.occupied` (HashMap keyed by `(tx, ty, zoom)`) prevents spawning duplicate entities.

### Critical: Mesh Types

- **3D tiles**: `Plane3d` (natively in XZ plane, faces +Y). No rotation needed. Do NOT use `Rectangle` + rotation - produces degenerate AABB in Bevy 0.19.
- **2D tiles**: `Rectangle` (natively in XY plane, faces -Z). No rotation needed.
- Per-zoom mesh handles are pre-built at startup with 0.1% overlap to prevent seams.
- All tile entities have `NoFrustumCulling` (Plane3d has zero Y extent in its AABB).

### Tile Elevation (Y-up space)

`update_tile_elevation` sets tile Y position using absolute zoom for depth ordering: higher zoom tiles sit closer to ground_y (render on top), lower zoom tiles sit below. The offset is `(19 - tile_zoom) * 0.1` units. This prevents Z-fighting regardless of which zoom level is current.

### 2D-to-3D Transition

The transition uses a dolly-zoom technique: starts at 1 deg FOV (nearly orthographic) and widens to 60 deg over 0.8 seconds, adjusting camera height to maintain the same visible ground area. Fog fades in with transition progress. Aircraft scale blends between 2D and 3D formulas.

### Common Causes of Tile Flashing

1. **Spawn-cull-respawn cycle**: Entity budget (`max_tile_entities = 5000` in 3D) must exceed steady-state tile count.
2. **Zoom oscillation**: Asymmetric hysteresis (0.7 upgrade, 0.6 downgrade) with 1-second cooldown prevents rapid switching.
3. **Z-fighting between zoom levels**: Fixed by absolute zoom depth ordering in `update_tile_elevation`.
4. **Mode switch**: `orient_tiles_for_view_mode` releases all tiles on 2D/3D switch (coordinate systems differ).

### Debugging Tile Issues

Run with `RUST_LOG=airjedi_bevy::tiles=info cargo run --release 2>tmp/tile_debug.log`
