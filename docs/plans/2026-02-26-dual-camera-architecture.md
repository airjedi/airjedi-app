# Dual Camera Architecture Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace ad-hoc z-ordering with formalized render layer categories and fixed camera order to eliminate z-fighting and compositing bugs when adding new visual elements.

**Architecture:** Phase 1 of Hybrid A-then-C. Create a `RenderCategory` constants module, re-tag all entities onto non-overlapping render layers, fix camera order so it never swaps, and refactor `manage_atmosphere_camera` to only change layer subscriptions. Phase 2 (mode-exclusive `is_active` toggling) is a separate future plan.

**Tech Stack:** Bevy 0.18, RenderLayers, Camera2d/Camera3d, StandardMaterial

**Design doc:** `docs/plans/2026-02-26-dual-camera-architecture-design.md`

---

### Task 1: Create RenderCategory Constants Module

**Files:**
- Create: `src/render_layers.rs`
- Modify: `src/main.rs:1` (add `mod render_layers;` declaration)
- Modify: `src/main.rs:39` (add `pub(crate) use render_layers::RenderCategory;`)

**Step 1: Create `src/render_layers.rs`**

```rust
use bevy::camera::visibility::RenderLayers;

/// Centralized render layer assignments.
/// Each visual category gets its own layer so cameras can subscribe
/// to exactly the layers they need per mode.
///
/// Recipe for adding a new entity type:
/// 1. Add a constant here
/// 2. Add the layer to the appropriate camera in manage_atmosphere_camera
/// 3. Spawn entity with RenderLayers::layer(RenderCategory::YOUR_TYPE)
pub struct RenderCategory;

impl RenderCategory {
    pub const TILES_2D: u8 = 1;    // Tile sprites (2D rendering)
    pub const GIZMOS: u8 = 2;      // Trails, navaids, runways
    pub const AIRCRAFT: u8 = 3;    // 3D GLB models
    pub const OVERLAYS_2D: u8 = 4; // Day/night tint, weather overlays
    pub const LABELS: u8 = 5;      // Text2d labels
    pub const TILES_3D: u8 = 6;    // Tile mesh quads (3D rendering)
    pub const GROUND: u8 = 7;      // Ground plane (3D only)
    pub const SKY: u8 = 8;         // Star field (3D only)
    pub const UI: u8 = 11;         // egui (unchanged)
}

/// Layers the Map Camera (Camera2d) subscribes to in 2D mode.
pub fn layers_2d_map() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::TILES_2D,
        RenderCategory::GIZMOS,
        RenderCategory::OVERLAYS_2D,
        RenderCategory::LABELS,
    ])
}

/// Layers the Map Camera (Camera2d) subscribes to in 3D mode.
/// Only gizmos and labels — tiles are mesh quads on Camera3d.
pub fn layers_3d_overlay() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::GIZMOS,
        RenderCategory::LABELS,
    ])
}

/// Layers the Aircraft Camera (Camera3d) subscribes to in 2D mode.
/// Only aircraft models.
pub fn layers_2d_aircraft() -> RenderLayers {
    RenderLayers::layer(RenderCategory::AIRCRAFT)
}

/// Layers the Aircraft Camera (Camera3d) subscribes to in 3D mode.
/// Aircraft, tile meshes, ground plane, sky.
pub fn layers_3d_world() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::AIRCRAFT,
        RenderCategory::TILES_3D,
        RenderCategory::GROUND,
        RenderCategory::SKY,
    ])
}
```

**Step 2: Add module declaration and re-export in `src/main.rs`**

At line 33 (after `mod tiles;`), add:
```rust
mod render_layers;
```

At line 39 (after `pub(crate) use camera::{MapCamera, AircraftCamera};`), add:
```rust
pub(crate) use render_layers::RenderCategory;
```

**Step 3: Build to verify**

Run: `cargo build 2>&1 | head -20`
Expected: Compiles with possible unused warnings (that's fine, we'll use everything in later tasks)

**Step 4: Commit**

```
feat: add RenderCategory constants module
```

---

### Task 2: Update Camera Spawning to Use RenderCategory Layers

**Files:**
- Modify: `src/main.rs:342-385` (camera spawn code)

**Step 1: Update Map Camera (line 346)**

Change:
```rust
commands.spawn((Name::new("Map Camera"), Camera2d, MapCamera, RenderLayers::from_layers(&[0, 2])));
```
To:
```rust
commands.spawn((
    Name::new("Map Camera"),
    Camera2d,
    MapCamera,
    render_layers::layers_2d_map(),
));
```

**Step 2: Update Aircraft Camera (lines 354-369)**

Change the spawn to include explicit RenderLayers. The Camera3d currently has no RenderLayers (defaults to layer 0). Add the 2D-mode aircraft layer:
```rust
commands.spawn((
    Name::new("Aircraft Camera"),
    Camera3d::default(),
    AircraftCamera,
    Camera {
        order: 1,
        clear_color: ClearColorConfig::Custom(Color::NONE),
        ..default()
    },
    Projection::Orthographic(OrthographicProjection::default_2d()),
    Transform::default(),
    bevy::picking::mesh_picking::MeshPickingCamera,
    render_layers::layers_2d_aircraft(),
));
```

**Step 3: UI Camera stays unchanged (lines 375-385)**

Already uses `RenderLayers::layer(11)` which matches `RenderCategory::UI`. No change needed.

**Step 4: Update gizmo layer config (lines 315-318)**

Already uses `RenderLayers::layer(2)` which matches `RenderCategory::GIZMOS`. Update to use the constant for clarity:
```rust
fn configure_gizmo_layers(mut config_store: ResMut<GizmoConfigStore>) {
    let (config, _) = config_store.config_mut::<DefaultGizmoConfigGroup>();
    config.render_layers = RenderLayers::layer(RenderCategory::GIZMOS);
}
```

This requires importing `RenderCategory` — add `use crate::RenderCategory;` at the top of the function or use `crate::RenderCategory::GIZMOS` inline. Since `RenderCategory` is re-exported at crate root, `use crate::RenderCategory;` should already be available.

**Step 5: Build to verify**

Run: `cargo build 2>&1 | head -20`
Expected: Compiles. Note: the app will look broken at this point because entities are still on layer 0 but cameras no longer subscribe to layer 0. This is expected and fixed in Task 3.

**Step 6: Commit**

```
refactor: update camera spawning to use RenderCategory layers
```

---

### Task 3: Tag Tile Entities with RenderLayers

**Files:**
- Modify: `src/tiles.rs:550-564` (tile sprite spawning)
- Modify: `src/tiles.rs:822-829` (tile mesh quad spawning)

**Step 1: Add import at top of `src/tiles.rs`**

Add near the top imports:
```rust
use crate::RenderCategory;
use bevy::camera::visibility::RenderLayers;
```

Check if `RenderLayers` is already imported — if so, only add `RenderCategory`.

**Step 2: Add RenderLayers to tile sprite spawn (line 550)**

Add `RenderLayers::layer(RenderCategory::TILES_2D)` to the spawn tuple:
```rust
commands.spawn((
    Name::new(format!("Map Tile z{}", event_zoom)),
    Sprite {
        image: tile_handle,
        color: Color::srgba(1.0, 1.0, 1.0, 0.0),
        ..default()
    },
    Transform::from_xyz(transform_x, transform_y, tile_z)
        .with_scale(Vec3::splat(rescale)),
    MapTile,
    TileFadeState {
        alpha: 0.0,
        tile_zoom: event_zoom,
    },
    RenderLayers::layer(RenderCategory::TILES_2D),
));
```

**Step 3: Add RenderLayers to tile mesh quad spawn (line 822)**

Add `RenderLayers::layer(RenderCategory::TILES_3D)` to the mesh quad spawn:
```rust
let mesh_entity = commands.spawn((
    TileQuad3d,
    Mesh3d(quad_mesh.0.clone()),
    MeshMaterial3d(material),
    Transform::from_translation(pos_yup)
        .with_scale(Vec3::new(transform.scale.x, 1.0, transform.scale.x)),
    Pickable::IGNORE,
    RenderLayers::layer(RenderCategory::TILES_3D),
)).id();
```

**Step 4: Build to verify**

Run: `cargo build 2>&1 | head -20`
Expected: Compiles. Tiles should now render in 2D mode (Camera2d subscribes to TILES_2D layer). 3D mesh quads should render when Camera3d subscribes to TILES_3D (handled in Task 6).

**Step 5: Commit**

```
refactor: tag tile sprites and mesh quads with RenderCategory layers
```

---

### Task 4: Tag Aircraft and Label Entities with RenderLayers

**Files:**
- Modify: `src/adsb/sync.rs:187-239` (aircraft and label spawning)

**Step 1: Add imports at top of `src/adsb/sync.rs`**

```rust
use crate::RenderCategory;
use bevy::camera::visibility::RenderLayers;
```

**Step 2: Add RenderLayers to aircraft spawn (line 187)**

Add `RenderLayers::layer(RenderCategory::AIRCRAFT)` to the aircraft spawn:
```rust
let mut entity_commands = commands.spawn((
    Name::new(format!("Aircraft: {}", aircraft_name)),
    SceneRoot(model_handle),
    Transform::from_xyz(0.0, 0.0, constants::AIRCRAFT_Z_LAYER),
    Pickable::default(),
    Aircraft { /* fields unchanged */ },
    TrailHistory::default(),
    RenderLayers::layer(RenderCategory::AIRCRAFT),
));
```

**Important note:** `SceneRoot` spawns child mesh entities. In Bevy 0.18, child entities inherit the parent's `RenderLayers` if they don't have their own. Verify this works by running the app after Task 6. If child meshes don't render, we may need to propagate `RenderLayers` to children — but this should work by default.

**Step 3: Add RenderLayers to label spawn (line 227)**

Add `RenderLayers::layer(RenderCategory::LABELS)` to the label spawn:
```rust
commands.spawn((
    Name::new(format!("Label: {}", aircraft_name)),
    Text2d::new(label_text),
    TextFont {
        font_size: constants::BASE_FONT_SIZE,
        ..default()
    },
    TextColor(theme.text_primary()),
    Transform::from_xyz(0.0, 0.0, constants::LABEL_Z_LAYER),
    AircraftLabel {
        aircraft_entity,
    },
    RenderLayers::layer(RenderCategory::LABELS),
));
```

**Step 4: Build to verify**

Run: `cargo build 2>&1 | head -20`
Expected: Compiles.

**Step 5: Commit**

```
refactor: tag aircraft and labels with RenderCategory layers
```

---

### Task 5: Tag Ground Plane and Sky Entities with RenderLayers

**Files:**
- Modify: `src/view3d/sky.rs` (ground plane and star field spawning)

**Step 1: Find ground plane and star field spawn locations**

Search `src/view3d/sky.rs` for `GroundPlane` spawn and any star field / sky sphere spawn. Add:
```rust
use crate::RenderCategory;
use bevy::camera::visibility::RenderLayers;
```

**Step 2: Add RenderLayers to ground plane spawn**

Add `RenderLayers::layer(RenderCategory::GROUND)` to the `GroundPlane` entity spawn.

**Step 3: Add RenderLayers to star field / sky entity spawn (if exists)**

Add `RenderLayers::layer(RenderCategory::SKY)` to any star field or sky background entity.

**Step 4: Check for day/night overlay entities**

Search for any day/night tint or weather overlay entities. Tag them with `RenderLayers::layer(RenderCategory::OVERLAYS_2D)`.

**Step 5: Build to verify**

Run: `cargo build 2>&1 | head -20`
Expected: Compiles.

**Step 6: Commit**

```
refactor: tag ground plane and sky entities with RenderCategory layers
```

---

### Task 6: Refactor manage_atmosphere_camera — Fixed Order, Layer Swapping

**Files:**
- Modify: `src/view3d/sky.rs:551-668` (`manage_atmosphere_camera` function)

This is the critical task. Replace order/clear-color swapping with layer subscription updates.

**Step 1: Add imports**

```rust
use crate::render_layers;
```

**Step 2: Rewrite the function**

Replace the current `manage_atmosphere_camera` function (lines 551-668) with:

```rust
pub fn manage_atmosphere_camera(
    mut commands: Commands,
    state: Res<View3DState>,
    sun_state: Res<SunState>,
    medium_handle: Option<Res<crate::AtmosphereMediumHandle>>,
    mut camera_3d: Query<(Entity, &mut Camera, Option<&Atmosphere>), With<Camera3d>>,
    mut camera_2d: Query<(Entity, &mut Camera), (With<crate::MapCamera>, Without<Camera3d>)>,
    mut ground_query: Query<(&mut Transform, &mut Visibility), With<GroundPlane>>,
) {
    let Ok((cam3d_entity, mut cam3d, has_atmo)) = camera_3d.single_mut() else {
        return;
    };
    let Ok((cam2d_entity, mut cam2d)) = camera_2d.single_mut() else {
        return;
    };

    // FIXED ORDER — never changes
    cam3d.order = 0;
    cam2d.order = 1;

    if state.is_3d_active() {
        let show_atmo = state.atmosphere_enabled;

        if show_atmo {
            cam3d.clear_color = ClearColorConfig::Default;
        } else {
            cam3d.clear_color = ClearColorConfig::Custom(Color::BLACK);
        }

        // Add atmosphere components once when entering 3D
        if show_atmo && has_atmo.is_none() {
            let scene_units_to_m = 0.3 * 1000.0 / (super::PIXEL_SCALE * state.altitude_scale);
            if let Some(ref medium_handle) = medium_handle {
                let mut atmo = Atmosphere::earthlike(medium_handle.0.clone());
                atmo.ground_albedo = Vec3::ZERO;
                commands.entity(cam3d_entity).insert((
                    atmo,
                    AtmosphereSettings {
                        scene_units_to_m,
                        aerial_view_lut_max_distance: 500.0,
                        ..default()
                    },
                    AtmosphereEnvironmentMapLight::default(),
                    Exposure { ev100: 9.0 },
                    DistanceFog {
                        color: Color::srgba(0.10, 0.10, 0.12, 1.0),
                        directional_light_color: Color::NONE,
                        directional_light_exponent: 30.0,
                        falloff: FogFalloff::Linear {
                            start: state.visibility_range * 0.4,
                            end: state.visibility_range,
                        },
                    },
                ));
            }
        } else if !show_atmo && has_atmo.is_some() {
            commands.entity(cam3d_entity)
                .remove::<Atmosphere>()
                .remove::<AtmosphereSettings>()
                .remove::<AtmosphereEnvironmentMapLight>()
                .remove::<Exposure>()
                .remove::<DistanceFog>();
        }

        // Camera3d: full 3D world
        commands.entity(cam3d_entity).insert(render_layers::layers_3d_world());
        // Camera2d: overlay (gizmos + labels) with alpha blending
        cam2d.clear_color = ClearColorConfig::Custom(Color::NONE);
        cam2d.output_mode = CameraOutputMode::Write {
            blend_state: Some(BlendState::ALPHA_BLENDING),
            clear_color: ClearColorConfig::None,
        };
        commands.entity(cam2d_entity).insert(render_layers::layers_3d_overlay());

        // Ground plane visible
        if let Ok((_, mut gp_vis)) = ground_query.single_mut() {
            *gp_vis = Visibility::Inherited;
        }
    } else {
        if has_atmo.is_some() {
            commands.entity(cam3d_entity)
                .remove::<Atmosphere>()
                .remove::<AtmosphereSettings>()
                .remove::<AtmosphereEnvironmentMapLight>()
                .remove::<Exposure>()
                .remove::<DistanceFog>()
                .remove::<Hdr>();
        }

        // Camera3d: aircraft only, transparent background
        cam3d.clear_color = ClearColorConfig::Custom(Color::NONE);
        cam3d.output_mode = CameraOutputMode::default();
        commands.entity(cam3d_entity).insert(render_layers::layers_2d_aircraft());

        // Camera2d: primary 2D renderer
        cam2d.clear_color = ClearColorConfig::Default;
        cam2d.output_mode = CameraOutputMode::default();
        commands.entity(cam2d_entity).insert(render_layers::layers_2d_map());

        // Ground plane hidden
        if let Ok((_, mut gp_vis)) = ground_query.single_mut() {
            *gp_vis = Visibility::Hidden;
        }
    }
}
```

**Key changes from current code:**
1. Camera order is fixed (`cam3d.order = 0`, `cam2d.order = 1`) — never swaps
2. Query now returns `(Entity, &mut Camera)` for Camera2d (needs entity for `commands.entity().insert()`)
3. Layer subscriptions updated via `commands.entity().insert(RenderLayers)` based on mode
4. No more order swapping in 2D branch (`cam2d.order = 0` / `cam3d.order = 1` removed)

**Step 3: Build to verify**

Run: `cargo build 2>&1 | head -20`
Expected: Compiles.

**Step 4: Commit**

```
refactor: fixed camera order, layer-only mode switching in manage_atmosphere_camera
```

---

### Task 7: Run and Visually Verify 2D Mode

**Step 1: Run the app**

Run: `cargo run 2>&1 | head -5`

**Step 2: Verify 2D mode**

Check:
- Map tiles render correctly
- Aircraft models visible
- Labels visible with correct positioning
- Gizmos (trails, navaids) render correctly
- Pan and zoom work

**Step 3: If aircraft models don't render**

SceneRoot children may not inherit RenderLayers. If so, add a system that propagates RenderLayers to children:

```rust
fn propagate_render_layers_to_children(
    parents: Query<(&RenderLayers, &Children), With<Aircraft>>,
    mut commands: Commands,
    children_query: Query<Entity, Without<RenderLayers>>,
) {
    for (layers, children) in parents.iter() {
        for &child in children.iter() {
            if children_query.get(child).is_ok() {
                commands.entity(child).insert(layers.clone());
            }
        }
    }
}
```

Only add this if needed. Test first without it.

**Step 4: If tiles don't render**

Check that Camera2d's RenderLayers includes `TILES_2D` (layer 1). The initial spawn in Task 2 uses `layers_2d_map()` which includes it, and `manage_atmosphere_camera` in Task 6 sets it in the 2D branch.

**Step 5: Commit any fixes**

```
fix: resolve rendering issues after layer migration
```

---

### Task 8: Visually Verify 3D Mode

**Step 1: Run the app and press '3' to enter 3D mode**

**Step 2: Verify 3D mode**

Check:
- Tile mesh quads render on the ground plane
- Tile sprites are NOT visible (hidden via `hide_tile_sprites_in_3d`)
- Aircraft models visible in 3D perspective
- Gizmos render as overlay
- Labels render as overlay
- Atmosphere/sky renders (if enabled)
- Ground plane visible
- No z-fighting between tiles and ground

**Step 3: Press '3' again to return to 2D**

Check:
- Clean transition back to 2D
- All 2D elements restored

**Step 4: Pan and zoom in both modes**

Verify tile loading, culling, and rendering work in both modes.

**Step 5: Commit any fixes**

```
fix: resolve 3D mode rendering issues after layer migration
```

---

### Task 9: Remove Z-Layer Constants (Optional Cleanup)

**Files:**
- Modify: `src/main.rs:90-93` (z-layer constant definitions)
- Modify: `src/adsb/sync.rs:190,235` (z-layer references in spawns)
- Modify: `src/view3d/mod.rs:682,722` (z-layer references in position updates)

**Important:** Z-translation is still useful for ordering within the same render layer (e.g., multiple tile sprites on TILES_2D). Do NOT remove z-translation from tile sprites. Only remove the cross-category z-layer constants if they are truly no longer needed for inter-category ordering.

**Step 1: Evaluate whether z-layer constants are still needed**

With render layers isolating categories, `AIRCRAFT_Z_LAYER` (10.0) and `LABEL_Z_LAYER` (11.0) no longer need to be above `TILE_Z_LAYER` (0.0) for ordering purposes. However, the z values still affect transform positioning. Consider:

- Aircraft z=10.0 in 2D mode: only matters for Camera3d which only renders aircraft. Could be z=0.0.
- Labels z=11.0 in 2D mode: only matters for Camera2d. Labels are on their own layer, so z doesn't matter for ordering vs tiles.

**Step 2: If safe to simplify, update spawns**

Change aircraft spawn z from `constants::AIRCRAFT_Z_LAYER` to `0.0`.
Change label spawn z from `constants::LABEL_Z_LAYER` to `0.0`.
Remove the constants from `src/main.rs:91-93`.
Update references in `src/view3d/mod.rs:682,722`.

**Step 3: Build and verify**

Run: `cargo build && cargo run`
Verify both modes still work.

**Step 4: Commit**

```
cleanup: remove z-layer constants, ordering now handled by render layers
```

---

### Task 10: Update CLAUDE.md Documentation

**Files:**
- Modify: `CLAUDE.md`

**Step 1: Update the Architecture section**

Add a new subsection documenting the render layer system and the recipe for adding new entity types. Remove or update references to z-layer ordering.

Add after the "Key Components" subsection:

```markdown
### Render Layers (src/render_layers.rs)

Visual categories are isolated by render layer to prevent z-fighting:

| Layer | Constant | Content | 2D Camera | 3D Camera |
|-------|----------|---------|-----------|-----------|
| 1 | TILES_2D | Tile sprites | Yes | No |
| 2 | GIZMOS | Trails, navaids, runways | Yes | No |
| 3 | AIRCRAFT | 3D GLB models | No | Yes |
| 4 | OVERLAYS_2D | Day/night tint, weather | Yes | No |
| 5 | LABELS | Text2d labels | Yes (2D+3D) | No |
| 6 | TILES_3D | Tile mesh quads | No | Yes (3D only) |
| 7 | GROUND | Ground plane | No | Yes (3D only) |
| 8 | SKY | Star field | No | Yes (3D only) |
| 11 | UI | egui | UI Camera | No |

Camera order is fixed: Camera3d=0, Camera2d=1, UI=100. Mode changes update layer subscriptions, never camera order.

**Adding a new entity type:**
1. Add a constant to `RenderCategory` in `src/render_layers.rs`
2. Add the layer to the appropriate camera function (`layers_2d_map`, `layers_3d_world`, etc.)
3. Spawn entity with `RenderLayers::layer(RenderCategory::YOUR_TYPE)`
4. If mode-specific, toggle `Visibility` in mode change handlers
```

**Step 2: Remove or update the "3D Tile Rendering — Known Pitfalls" section**

Many of the documented pitfalls around z-fighting between zoom levels should be resolved. Update this section to reflect the new layer-based architecture. Keep the debugging section but note the new layer system.

**Step 3: Commit**

```
docs: update CLAUDE.md with render layer architecture
```

---

## Verification Checklist (Run After All Tasks)

1. `cargo build` — compiles without errors or warnings
2. 2D mode: tiles, aircraft, labels, gizmos all render correctly
3. Press '3': smooth transition to 3D
4. 3D mode: tile mesh quads, aircraft, gizmos overlay, atmosphere all render
5. Press '3': clean transition back to 2D
6. Pan and zoom in both modes
7. Day/night tint only in 2D, atmosphere only in 3D
8. No z-fighting between any visual categories
9. Aircraft picking works in both modes
