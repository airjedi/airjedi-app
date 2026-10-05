# Plan: Unify 2D/3D into Single Always-3D Rendering Architecture

## Context

AirJedi currently maintains two parallel rendering paths - a 2D orthographic map view and a 3D perspective view - with ~65 mode-branching sites across 16 files. Every feature (tiles, aircraft, airports, trails, input, zoom, picking, culling) has dual implementations for each mode, plus a dolly-zoom transition system to animate between them. This imposes significant maintenance cost and blocks features like terrain elevation from working cleanly across modes.

Following the Cesium Columbus View model (flat map surface, perspective camera, objects at altitude), this plan unifies everything into a single always-3D rendering path where camera pitch is a continuous parameter from 0 (horizon) to 90 (top-down map). The "2D map view" becomes a preset: pitch=90 with orthographic projection. The "3D view" is any other pitch with perspective projection.

The user also wants to support satellite-altitude viewing (LEO, ~250 miles) and view presets with continuous pitch/altitude control.

## Architecture Overview

### Single Coordinate System: Y-up Always
- X = east, Y = up (altitude), Z = -north (south)
- All entities positioned in Y-up at all times
- Eliminate all `zup_to_yup` / `yup_to_zup` conversions
- Tiles always use `Plane3d` (XZ plane, faces +Y)
- `Rectangle` mesh path removed entirely

### Camera Model
- One primary camera (AircraftCamera/Camera3d) with continuous pitch/yaw/altitude
- At pitch=90: orthographic projection, scale derived from altitude
- At pitch<90: perspective projection, 60-degree FOV
- Camera2d overlay persists for gizmos/labels, always derives transform from Camera3d
- No mode enum, no transition state machine

### View Presets
Named (pitch, altitude, yaw) tuples with smooth animated transitions:

| Preset | Pitch | Altitude | Notes |
|--------|-------|----------|-------|
| Map | 90 | from current zoom | Top-down orthographic, classic map view |
| Tilted | 70 | 30,000 ft | Default 3D perspective |
| Low | 25 | 5,000 ft | Near-ground perspective |
| Chase | follows aircraft | offset behind/above | Existing chase system |
| Satellite | 90 | 1,000,000 ft | LEO overview, orthographic |

The `3` key cycles through presets. Shift+drag adjusts pitch continuously. Scroll adjusts altitude.

### Altitude Range
- MIN_CAMERA_ALTITUDE: 1,000 ft (unchanged)
- MAX_CAMERA_ALTITUDE: 1,500,000 ft (~284 miles, covers LEO)
- Adaptive zoom formula extended to support zoom levels 1-18
- Sky dome, ground plane, far plane scale dynamically with altitude

## Phased Implementation

### Phase 1: Always Y-up Tiles

Remove the coordinate system fork in tile rendering. This is the foundation.

**Changes:**
- `src/tiles/render.rs`:
  - Remove `meshes_2d` from `TileQuadMesh` (delete `Rectangle` creation in `setup_tile_quad_mesh`)
  - `load_visible_tiles`: always use Y-up positioning `(local_x, tile_z, -local_y)` and `meshes_3d`
  - Remove `orient_tiles_for_view_mode` entirely (no more tile teardown/respawn on mode switch)
  - `cull_offscreen_tiles`: always use XZ ground-plane distance (remove the 2D XY branch)
  - `compute_tile_radius`: always use the 3D perspective calculation
  - Remove the 2D branch in `load_visible_tiles` (single-band loading becomes the low-pitch case of SSE-driven LOD)
- `src/tiles/render.rs`: `animate_tile_fades` - remove 2D-only "keep only current zoom" branch; always use the 3D multi-zoom-band logic

**Verification:** App launches, tiles render correctly in current 3D mode. Switch to "2D mode" still works (tiles are Plane3d but camera is orthographic looking down at them - same visual result). No tile flashing on mode switch since tiles are no longer torn down.

### Phase 2: Unified Camera System

Replace the dual camera mode with a single continuous camera.

**Changes:**
- `src/view3d/mod.rs`:
  - Remove `ViewMode` enum. Replace with continuous `camera_pitch: f32` (0-90)
  - Remove `TransitionState` enum and `animate_view_transition` system
  - Remove `is_3d_active()` method (replaced by pitch checks where needed, e.g., `pitch < 90.0` for perspective features)
  - Add `ViewPreset` enum and `animate_to_preset` system with smooth easing
  - Merge `update_3d_camera` logic to work at all pitches:
    - At pitch=90: set `Projection::Orthographic` with scale derived from altitude
    - At pitch<90: set `Projection::Perspective` with 60-degree FOV
    - Orbit calculation works continuously (at pitch=90 it's directly above the center)
  - Remove `toggle_3d_view` (replaced by preset cycling)
  - Remove `saved_2d_center` / `saved_2d_zoom_level` (no mode to save/restore for)
- `src/camera.rs`:
  - Remove `update_camera_position` (2D-only system) - absorbed into unified camera update
  - Remove `sync_aircraft_camera` - Camera2d always derives from Camera3d
  - Keep Camera2d overlay with derived transform (needed for gizmos/labels)
- `src/main.rs`:
  - Camera spawns with perspective projection (current default for 3D)
  - Initial preset: "Map" (pitch=90, orthographic, at the configured default zoom altitude)

**Verification:** Camera works continuously. Scroll changes altitude. At pitch=90, map looks identical to old 2D mode. Tilting pitch smoothly transitions to perspective. No mode toggle needed.

### Phase 3: Unified Input

Merge the dual input paths into one system.

**Changes:**
- `src/input.rs`:
  - Rewrite `handle_pan_drag` to always work on the XZ ground plane:
    - At pitch=90 (orthographic): screen delta / ortho_scale = world XZ delta (same math as current 2D)
    - At pitch<90 (perspective): screen delta projected onto ground plane via yaw-rotated basis vectors (same math as current 3D pan)
    - One system, pitch determines which projection math to use
- `src/zoom.rs`:
  - Rewrite `handle_zoom` / `handle_pinch_zoom`:
    - Scroll always changes camera altitude (no more discrete zoom level + continuous camera_zoom two-tier)
    - Discrete tile zoom level derived automatically from altitude via `altitude_to_zoom_level` (already exists)
    - Zoom-to-cursor: at pitch=90 (ortho) use current ortho math; at pitch<90 use ground-plane ray intersection
  - Remove `apply_camera_zoom` (ortho scale now set by camera system from altitude)
  - Remove `ZoomState.camera_zoom` - replaced by altitude
  - `ZoomState` retains only `min_zoom` / `max_zoom` for tile zoom level clamping
  - Add shift+scroll for pitch adjustment, shift+drag for orbit (yaw+pitch)
- `src/view3d/mod.rs`:
  - Remove `handle_3d_camera_controls` - absorbed into unified input

**Verification:** Pan, zoom, orbit all work at any pitch. Zoom-to-cursor works correctly in both orthographic (top-down) and perspective (tilted).

### Phase 4: Unified Entity Positioning

Remove all per-entity 2D/3D branching.

**Changes:**
- `src/camera.rs`:
  - `update_aircraft_positions`: position directly in Y-up with altitude: `(pos.x, altitude_y, -pos.y)`
  - Remove the Z-up intermediate step
  - Aircraft rotation: always use `BASE_ROT_YUP` with heading around Y axis
  - `scale_aircraft_and_labels`: one scale formula (perspective-aware), no transition blend
  - Remove `cull_offscreen_aircraft` - Bevy's frustum culling handles perspective
- `src/view3d/mod.rs`:
  - Remove `update_aircraft_3d_transform` entirely (aircraft positioned correctly in phase 4)
  - Remove `fade_distant_sprites` (fog handles distance fading)
  - Remove `fix_aircraft_model_materials` unlit toggle (always lit, or user preference)
- `src/aircraft/picking.rs`:
  - Merge `pick_aircraft_2d` and `pick_aircraft_3d` into one perspective raycast system (works for ortho too since Bevy's raycast handles both projection types)
- `src/aircraft/mesh_trails.rs`:
  - Always use 3D renderer (`MeshTrailRenderer::Mesh3d`)
  - Always position vertices in Y-up with altitude
  - Remove all `if is_3d` coordinate conversion branches
- `src/aircraft/trail_renderer.rs`:
  - Always use `gizmos.line()` (3D), remove `gizmos.line_2d()` path
- `src/aviation/airports.rs`:
  - Always position at `(x, altitude_to_z(elevation), -y)`, remove Z-layer constant
  - Convert from `Mesh2d(Rectangle)` to `Mesh3d(Plane3d)` or keep as gizmo overlay
- `src/aviation/navaids.rs`:
  - Always use 3D gizmo positions, remove Z-layer branch
- `src/aviation/runways.rs`:
  - Always visible (remove the "hidden in 3D" gate)
  - Position at ground elevation in Y-up
- `src/view3d/sky.rs`:
  - Sky dome, ground plane, fog always active
  - At pitch=90 (top-down): fog pushed to max distance, sky dome below camera (not visible)
  - Ground plane always positioned at ground_elevation_ft
  - Remove all `is_3d_active()` visibility toggles - use pitch-based decisions instead

**Verification:** Aircraft, airports, navaids, runways, trails all render correctly at any pitch. No visual difference from current 2D mode when at pitch=90. No visual difference from current 3D mode when tilted.

### Phase 5: Altitude Extension and Presets

Extend camera range and add the preset system.

**Changes:**
- `src/view3d/mod.rs`:
  - `MAX_CAMERA_ALTITUDE`: 120,000 -> 1,500,000 ft
  - Implement `ViewPreset` enum with named presets (Map, Tilted, Low, Chase, Satellite)
  - `animate_to_preset`: smooth pitch/altitude/yaw animation using hermite interpolation
  - Keyboard shortcut `3` cycles through presets
  - Each preset stores target pitch, altitude, and optional yaw
- `src/tiles/render.rs`:
  - Extend `altitude_to_zoom_level` formula to map satellite altitudes to zoom 1-4:
    - Current: `(16 - (alt/5000).log2() * 1.5).clamp(8, 18)`
    - New: `.clamp(1, 18)` plus adjusted reference values for extreme altitudes
  - `MIN_SSE_ZOOM`: 4 -> 1
  - `compute_tile_radius`: handle satellite-altitude viewport (very large tile radius at low zoom)
- `src/view3d/sky.rs`:
  - Sky dome radius: scale with altitude, hide above ~200km (pitch-dependent)
  - Ground plane extent: scale with altitude up to ~2000km at satellite altitude
  - Far plane: already dynamic, just remove the 500km minimum floor at satellite altitude
  - Fog: at satellite altitude, push fog to extreme distance or disable
- `src/toolbar.rs` / `src/tools_window.rs`:
  - Replace 2D/3D toggle with preset selector dropdown
  - Add pitch/altitude sliders
  - Remove "3D View" panel concept (all settings are always relevant)

**Verification:** Camera smoothly zooms from street level to satellite altitude. Tile zoom levels transition smoothly. Preset cycling works via keyboard and UI. Sky/fog/ground plane scale appropriately.

### Phase 6: Terrain Integration

With unified architecture, reconnect the terrain module (from the original terrain plan).

**Changes:**
- `src/tiles/pool.rs`: Add `TileCoord` component, insert during `activate_tile`
- `src/terrain/mod.rs`: Update all queries to use `TileCoord` and unified tile entities (no `TileMeshQuad`, no `Sprite`)
- `src/terrain/mesh.rs`: Fix elevation scaling to match altitude convention (meters -> feet -> world units + relief multiplier)
- `src/terrain/heightmap.rs`: Update `request_heightmaps_for_tiles` to use `TileCoord`
- Terrain is active whenever enabled, at any camera pitch. At top-down it's invisible (no Y displacement visible from directly above). At any tilt, relief appears naturally.

## Code Removal Summary

Systems and code paths eliminated by this unification:

| Removed | Location |
|---------|----------|
| `ViewMode` enum | `src/view3d/mod.rs` |
| `TransitionState` enum + dolly-zoom | `src/view3d/mod.rs` |
| `toggle_3d_view` system | `src/view3d/mod.rs` |
| `animate_view_transition` system | `src/view3d/mod.rs` |
| `update_aircraft_3d_transform` system | `src/view3d/mod.rs` |
| `fade_distant_sprites` system | `src/view3d/mod.rs` |
| `fix_aircraft_model_materials` system | `src/view3d/mod.rs` |
| `update_camera_position` (2D) system | `src/camera.rs` |
| `sync_aircraft_camera` system | `src/camera.rs` |
| `cull_offscreen_aircraft` system | `src/camera.rs` |
| `handle_pan_drag` (2D path) | `src/input.rs` |
| `handle_zoom` (2D path) | `src/zoom.rs` |
| `handle_pinch_zoom` (2D path) | `src/zoom.rs` |
| `apply_camera_zoom` system | `src/zoom.rs` |
| `pick_aircraft_2d` system | `src/aircraft/picking.rs` |
| `orient_tiles_for_view_mode` system | `src/tiles/render.rs` |
| `meshes_2d` / `Rectangle` tile meshes | `src/tiles/render.rs` |
| `zup_to_yup` / `yup_to_zup` helpers | `src/view3d/mod.rs` |
| `saved_2d_center` / `saved_2d_zoom_level` | `src/view3d/mod.rs` |
| `ZoomState.camera_zoom` | `src/map.rs` |
| ~65 `if is_3d_active()` branch points | 16 files |

## Files Changed by Phase

| Phase | Files |
|-------|-------|
| 1: Tiles | `src/tiles/render.rs` |
| 2: Camera | `src/view3d/mod.rs`, `src/camera.rs`, `src/main.rs` |
| 3: Input | `src/input.rs`, `src/zoom.rs`, `src/view3d/mod.rs` |
| 4: Entities | `src/camera.rs`, `src/view3d/mod.rs`, `src/view3d/sky.rs`, `src/aircraft/picking.rs`, `src/aircraft/mesh_trails.rs`, `src/aircraft/trail_renderer.rs`, `src/aviation/airports.rs`, `src/aviation/navaids.rs`, `src/aviation/runways.rs`, `src/tools/measurement.rs` |
| 5: Altitude | `src/view3d/mod.rs`, `src/view3d/sky.rs`, `src/tiles/render.rs`, `src/toolbar.rs`, `src/tools_window.rs`, `src/keyboard.rs` |
| 6: Terrain | `src/tiles/pool.rs`, `src/terrain/mod.rs`, `src/terrain/mesh.rs`, `src/terrain/heightmap.rs`, `src/tools_window.rs` |

## Key Risks and Mitigations

1. **"2D feel" regression**: At pitch=90 with orthographic projection, the view should be pixel-identical to the current 2D mode. Test by comparing screenshots.

2. **Performance at top-down**: SSE-driven LOD at pitch=90 loads similar tile counts to current 2D. The main tile band covers the viewport; lower-zoom bands are irrelevant at top-down (all tiles are equidistant). May need to skip lower bands when pitch > 80.

3. **f32 precision at satellite altitude**: LocalOrigin floating-origin system already handles this - camera stays near origin in local coordinates. Ground plane extent at 2000km is within f32 range. Far plane at ~5000km is fine.

4. **Zoom-to-cursor at projection boundary**: When pitch crosses 90 degrees during a zoom operation, the projection changes. The zoom math should use the current projection type, not assume one or the other. This is a localized concern in the zoom handler.

5. **Label readability at tilted views**: Labels currently hidden in 3D. With unified rendering, labels need billboard behavior (face camera). This is a Bevy feature (`Billboard` component or manual rotation toward camera).

## Verification Strategy

Each phase has a verification step. End-to-end validation after all phases:

1. Launch app - opens in Map preset (top-down, orthographic). Visually identical to current 2D mode.
2. Press `3` - smoothly transitions to Tilted preset. Tiles, aircraft, terrain visible.
3. Press `3` again - transitions to Low preset.
4. Scroll to zoom in/out at each preset - tile zoom adapts, zoom-to-cursor works.
5. Shift+drag to adjust pitch continuously - smooth projection switch at 90.
6. Pan at various pitches - consistent ground-plane panning.
7. Enable terrain - relief visible at tilted views, invisible at top-down.
8. Zoom out to satellite altitude (via Satellite preset) - tiles at zoom 1-3, Earth visible.
9. Select aircraft - picking works at all pitches.
10. Trails render correctly at all pitches.
11. Airports, navaids, runways visible and correctly positioned at all pitches.
