# Dual Camera Architecture: Hybrid A-then-C

## Context

Every time a new visual element is added to AirJedi (overlays, gizmos, navaids), z-ordering and camera compositing break. The root cause is that two cameras share the same world space with order-swapping, clear-color-swapping, and HDR/LDR mode-swapping in `manage_atmosphere_camera` (sky.rs). This makes the rendering pipeline fragile and hard to extend.

This design addresses the problem in two phases:
- **Phase 1 (A):** Formalize render layers, fix camera order, eliminate z-translation ordering
- **Phase 2 (C):** Mode-exclusive cameras with `is_active` toggling and 0.3s crossfade

## Decisions

- **Mode toggle**: Quick crossfade (0.3s opacity blend), not geometric interpolation
- **Tiles in 3D**: Mesh-only on dedicated layer. Sprites on separate layer. Never both active.
- **Layer isolation**: Full — no layer shared between Camera2d and Camera3d
- **Aircraft representation**: Single entity per aircraft, swap visual children (sprite vs 3D model) on mode change

## Phase 1: Layer-Per-Pass Architecture

### Goal

Replace ad-hoc z-ordering with formalized render layer categories. Fix camera order so it never swaps. Make "add a new entity type" a mechanical recipe.

### 1.1 Define RenderCategory constants

Create `src/render_layers.rs`:

```rust
pub struct RenderCategory;
impl RenderCategory {
    pub const TILES_2D: u8 = 1;       // Tile sprites (2D rendering)
    pub const GIZMOS: u8 = 2;         // Trails, navaids, runways (already on layer 2)
    pub const AIRCRAFT: u8 = 3;       // 3D GLB models (both modes)
    pub const OVERLAYS_2D: u8 = 4;    // Day/night tint, weather overlays
    pub const LABELS: u8 = 5;         // Text2d labels
    pub const TILES_3D: u8 = 6;       // Tile mesh quads (3D rendering)
    pub const GROUND: u8 = 7;         // Ground plane (3D only)
    pub const SKY: u8 = 8;            // Star field (3D only)
    pub const UI: u8 = 11;            // egui (unchanged)
}
```

### 1.2 Fix camera order -- never swap

Current problem: `manage_atmosphere_camera` swaps camera order (0/1) and clear color on every mode change, causing HDR/LDR mismatch bugs and compositing artifacts.

New rule: Camera order is constant. Mode changes only update which layers each camera subscribes to.

| Camera | Order | 2D Layers | 3D Layers | Clear |
|--------|-------|-----------|-----------|-------|
| Camera3d (AircraftCamera) | 0 | [3] (aircraft only) | [3,6,7,8] + atmosphere | Default in 3D, NONE in 2D |
| Camera2d (MapCamera) | 1 | [1,2,4,5] | [2,5] alpha-blend over 3D | Default in 2D, NONE in 3D |
| UI Camera | 100 | [11] | [11] | None |

### 1.3 Assign RenderLayers to entities at spawn

- **Tile sprites** (tiles.rs): `RenderLayers::layer(TILES_2D)`
- **Tile mesh quads** (tiles.rs): `RenderLayers::layer(TILES_3D)`
- **Aircraft** (adsb/sync.rs): `RenderLayers::layer(AIRCRAFT)`
- **Labels** (adsb/sync.rs): `RenderLayers::layer(LABELS)`
- **Day/night tint** (view3d/sky.rs): `RenderLayers::layer(OVERLAYS_2D)`
- **Ground plane** (view3d/sky.rs): `RenderLayers::layer(GROUND)`
- **Star field** (view3d/sky.rs): `RenderLayers::layer(SKY)`
- **Gizmos**: Already on layer 2, no change needed

### 1.4 Refactor manage_atmosphere_camera

Replace order/clear-color swapping with layer subscription updates. Camera order constants never change. Mode transitions only update `RenderLayers` and clear color on each camera.

### 1.5 Eliminate z-translation constants

Remove `TILE_Z_LAYER`, `AIRCRAFT_Z_LAYER`, `LABEL_Z_LAYER`. Inter-category ordering is handled by render layers and camera order. Within a category, z-translation can still differentiate if needed.

### 1.6 Recipe for adding new entity types

1. Pick or create a `RenderCategory` constant
2. Add the layer to the appropriate camera's layer set
3. Spawn entity with `RenderLayers::layer(RenderCategory::YOUR_TYPE)`
4. If mode-specific visibility, toggle `Visibility` in mode change handlers

### Files to modify (Phase 1)

- `src/render_layers.rs` -- new file
- `src/main.rs` -- camera spawning, remove z-layer constants
- `src/view3d/sky.rs` -- refactor `manage_atmosphere_camera`
- `src/tiles.rs` -- add RenderLayers to tile sprites and mesh quads
- `src/adsb/sync.rs` -- add RenderLayers to aircraft and labels
- `src/camera.rs` -- remove z-layer references

## Phase 2: Mode-Exclusive Cameras

### Goal

Evolve from "two cameras always rendering with layer switching" to "only the active mode's cameras render." Eliminates compositing artifacts entirely.

### 2.1 Mode-exclusive camera toggling

Toggle `camera.is_active` instead of swapping layers:

```
2D Mode:
  Map2DCamera (is_active=true)  -> renders layers [1,2,3,4,5]
  Map3DCamera (is_active=false) -> zero GPU cost
  UI Camera (is_active=true)    -> renders layer [11]

3D Mode:
  Map2DCamera (is_active=false) -> zero GPU cost
  Map3DCamera (is_active=true)  -> renders layers [3,6,7,8] + atmosphere
  Overlay3DCamera (is_active=true) -> renders layers [2,5] (gizmos, labels)
  UI Camera (is_active=true)    -> renders layer [11]
```

### 2.2 Transition crossfade

During 0.3s transition, both cameras active. Output alpha fades departing camera 1.0->0.0 while incoming fades in. After transition, departing camera deactivated.

### 2.3 Dedicated Overlay3DCamera

Lightweight Camera2d with `clear_color: NONE` and alpha-blend output. Active only in 3D mode. Renders gizmos and labels over the 3D scene. Replaces Camera2d's dual role.

### 2.4 Benefits over Phase 1 alone

- Inactive cameras = zero GPU cost
- No compositing between cameras in steady state
- Adding entities to 2D mode cannot affect 3D rendering
- HDR/LDR mismatch impossible (Map3DCamera always HDR, Map2DCamera always LDR)

### Files to modify (Phase 2, in addition to Phase 1)

- `src/main.rs` -- spawn Overlay3DCamera
- `src/view3d/sky.rs` -- simplify to is_active toggling
- `src/view3d/mod.rs` -- update transition for crossfade
- `src/camera.rs` -- add mode guards

## Verification

### Phase 1
1. `cargo build` compiles without errors
2. 2D mode: tiles render, aircraft visible, gizmos draw
3. Press '3': transition to 3D, no flashing, tiles become mesh quads
4. Press '3': return to 2D, clean transition
5. Pan and zoom in both modes
6. Day/night tint in 2D only, atmosphere in 3D only
7. Add a test entity with new RenderCategory -- appears without breaking anything

### Phase 2
1. All Phase 1 tests pass
2. Inactive cameras have zero GPU cost
3. 0.3s crossfade transition is smooth
4. Gizmos render correctly in both modes
