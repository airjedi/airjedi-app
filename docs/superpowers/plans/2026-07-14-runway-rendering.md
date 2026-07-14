# Runway Rendering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single-line runway gizmo with properly-sized gray rectangular runway bodies, white dashed centerlines, and rotated white runway number labels.

**Architecture:** Three rendering layers per runway - a `Mesh2d(Rectangle)` entity for the gray concrete body, a `draw_dashed_2d` gizmo for the white centerline (zoom 10+), and two `Text2d` entities per runway for the rotated runway number labels (zoom 11+). Entities are spawned once when aviation data loads (filtered to ~15k scheduled-service airport runways) and updated via visibility and position systems, mirroring the existing airport marker pattern.

**Tech Stack:** Bevy 0.19, `Mesh2d` + `ColorMaterial` for runway bodies, `Text2d` + `TextFont` for labels, Bevy Gizmos for centerline, existing `CoordinateConverter` for lat/lon-to-world math.

## Global Constraints

- Bevy 0.19 API throughout - use `Mesh2d`, `MeshMaterial2d`, `Text2d`, `TextFont`, `TextColor`, `FontSize::Px(f32)`
- All world positions are Web Mercator meters relative to `LocalOrigin` (floating origin)
- Runway body on render layer 0 (DEFAULT, rendered by `AircraftCamera`)
- Runway labels on `RenderLayers::layer(RenderCategory::LABELS)` (layer 5, rendered by `MapCamera`)
- Centerline drawn via Gizmos (layer 2, rendered by `MapCamera`) - same as current `draw_runways`
- 3D mode: all runway entities hidden (3D rendering is out of scope)
- Zoom thresholds: body at 8+, centerline at 10+, labels at 11+
- Filter runway spawning to airports passing `AirportFilter::FrequentlyUsed`
- Runway body Z = 4.5 (above tiles ~2-3, below airport markers at 5.0)
- Runway label Z = 4.6
- Body color open: `Color::srgba(0.55, 0.55, 0.55, 1.0)`
- Body color closed: `Color::srgba(0.35, 0.35, 0.35, 0.7)`
- Label + centerline color: `Color::WHITE` / `Color::srgba(1.0, 1.0, 1.0, 0.85)`
- Label font size: `FontSize::Px(10.0)`

---

## File Map

| File | Change |
|------|--------|
| `src/aviation/runways.rs` | Add `RunwayBody`, `RunwayLabel` components; add math helpers; add `spawn_runway_entities`, `update_runway_positions`, `update_runway_visibility` systems; upgrade `draw_runways` to dashed centerline |
| `src/aviation/plugin.rs` | Register three new systems |
| `src/aviation/types.rs` | No changes |

---

## Task 1: Math Helpers and New Components

**Files:**
- Modify: `src/aviation/runways.rs`

**Interfaces:**
- Produces:
  - `RunwayBody` component (stores cached coords for fast position updates)
  - `RunwayLabel` component (stores coords + end flag)
  - `heading_to_rotation(heading_deg: f64) -> f32` - pure function
  - `le_label_pos(le: Vec2, he: Vec2) -> Vec2` - pure function
  - `he_label_pos(le: Vec2, he: Vec2) -> Vec2` - pure function
  - `const RUNWAY_BODY_Z: f32 = 4.5`
  - `const RUNWAY_LABEL_Z: f32 = 4.6`
  - `const FEET_TO_METERS_F32: f32 = 0.3048`

- [ ] **Step 1: Add constants and component definitions to `src/aviation/runways.rs`**

Add the following directly below the existing `use` statements and before the existing `RunwayMarker` definition. Replace `RunwayMarker` and `RunwayRenderState` (keep `RunwayRenderState`) - `RunwayMarker` becomes `RunwayBody` with richer data:

```rust
// Conversion constants
const FEET_TO_METERS_F32: f32 = 0.3048;
const RUNWAY_BODY_Z: f32 = 4.5;
const RUNWAY_LABEL_Z: f32 = 4.6;

/// Marks a runway body mesh entity. Caches the geographic data needed
/// for position updates without re-querying AviationData.
#[derive(Component)]
pub struct RunwayBody {
    pub runway_id: i64,
    pub le_lat: f64,
    pub le_lon: f64,
    pub he_lat: f64,
    pub he_lon: f64,
    pub heading_deg: f64,
    pub width_m: f32,
    pub midpoint_lat: f64,
    pub midpoint_lon: f64,
}

/// Marks a runway number text entity.
#[derive(Component)]
pub struct RunwayLabel {
    pub runway_id: i64,
    pub le_lat: f64,
    pub le_lon: f64,
    pub he_lat: f64,
    pub he_lon: f64,
    pub heading_deg: f64,
    pub is_he_end: bool,
    pub midpoint_lat: f64,
    pub midpoint_lon: f64,
}
```

- [ ] **Step 2: Add math helper functions after the component definitions**

```rust
/// Convert a runway true heading (degrees, CW from north) to a Bevy 2D
/// rotation angle (radians, CCW from +Y). Heading 0 = north = no rotation.
pub fn heading_to_rotation(heading_deg: f64) -> f32 {
    -(heading_deg as f32).to_radians()
}

/// Position of the LE runway number label: 12% of runway length inward
/// from the LE threshold.
pub fn le_label_pos(le: Vec2, he: Vec2) -> Vec2 {
    le + (he - le) * 0.12
}

/// Position of the HE runway number label: 12% of runway length inward
/// from the HE threshold.
pub fn he_label_pos(le: Vec2, he: Vec2) -> Vec2 {
    he + (le - he) * 0.12
}
```

- [ ] **Step 3: Add unit tests at the bottom of `src/aviation/runways.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_to_rotation_north_is_zero() {
        assert!((heading_to_rotation(0.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn heading_to_rotation_east_is_neg_half_pi() {
        let expected = -std::f32::consts::FRAC_PI_2;
        assert!((heading_to_rotation(90.0) - expected).abs() < 1e-5);
    }

    #[test]
    fn heading_to_rotation_south_is_neg_pi() {
        let expected = -std::f32::consts::PI;
        assert!((heading_to_rotation(180.0) - expected).abs() < 1e-5);
    }

    #[test]
    fn le_label_pos_is_12_percent_inset() {
        let le = Vec2::new(0.0, 0.0);
        let he = Vec2::new(0.0, 1000.0);
        let pos = le_label_pos(le, he);
        assert!((pos.x).abs() < 1e-4);
        assert!((pos.y - 120.0).abs() < 1e-3);
    }

    #[test]
    fn he_label_pos_is_12_percent_inset_from_he() {
        let le = Vec2::new(0.0, 0.0);
        let he = Vec2::new(0.0, 1000.0);
        let pos = he_label_pos(le, he);
        assert!((pos.x).abs() < 1e-4);
        assert!((pos.y - 880.0).abs() < 1e-3);
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p airjedi-bevy aviation::runways 2>&1 | tail -20
```

Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/aviation/runways.rs
git commit -m "Add RunwayBody, RunwayLabel components and runway math helpers"
```

---

## Task 2: spawn_runway_entities System

**Files:**
- Modify: `src/aviation/runways.rs`
- Modify: `src/aviation/plugin.rs`

**Interfaces:**
- Consumes: `RunwayBody`, `RunwayLabel`, `heading_to_rotation`, `le_label_pos`, `he_label_pos` (Task 1); `AviationData`, `LoadingState`, `AirportFilter` (existing)
- Produces: ECS entities tagged with `RunwayBody` and `RunwayLabel`, all initially `Visibility::Hidden`

- [ ] **Step 1: Add required imports to `src/aviation/runways.rs`**

Replace the existing import block at the top of the file with:

```rust
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use std::collections::HashSet;

use crate::render_layers::RenderCategory;
use crate::tiles::*;
use crate::geo::{haversine_distance_nm, CoordinateConverter};
use crate::constants;
use crate::{MapState, view3d::View3DState};
use super::{Airport, AirportFilter, AviationData, LoadingState};
```

- [ ] **Step 2: Add `spawn_runway_entities` function to `src/aviation/runways.rs`**

Add this function after the helper functions from Task 1:

```rust
/// Spawns Mesh2d body entities and Text2d label entities for all runways
/// belonging to airports that have scheduled service. Runs once when
/// AviationData becomes ready.
pub fn spawn_runway_entities(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    aviation_data: Res<AviationData>,
    render_state: Res<RunwayRenderState>,
    local_origin: Res<LocalOrigin>,
    existing: Query<(), With<RunwayBody>>,
) {
    if aviation_data.loading_state != LoadingState::Ready {
        return;
    }
    if !existing.is_empty() {
        return;
    }
    if !render_state.show_runways {
        return;
    }

    let scheduled_airports: HashSet<i64> = aviation_data
        .airports
        .iter()
        .filter(|a| a.passes_filter(AirportFilter::FrequentlyUsed))
        .map(|a| a.id)
        .collect();

    let open_mat = materials.add(ColorMaterial::from_color(
        Color::srgba(0.55, 0.55, 0.55, 1.0),
    ));
    let closed_mat = materials.add(ColorMaterial::from_color(
        Color::srgba(0.35, 0.35, 0.35, 0.7),
    ));

    let converter = CoordinateConverter::new(&local_origin);
    let mut count = 0;

    for runway in &aviation_data.runways {
        if !runway.has_valid_coords() {
            continue;
        }
        if !scheduled_airports.contains(&runway.airport_ref) {
            continue;
        }
        let Some(width_ft) = runway.width_ft else {
            continue;
        };
        let Some(heading) = runway.le_heading_deg_t else {
            continue;
        };

        let le_lat = runway.le_latitude_deg.unwrap();
        let le_lon = runway.le_longitude_deg.unwrap();
        let he_lat = runway.he_latitude_deg.unwrap();
        let he_lon = runway.he_longitude_deg.unwrap();
        let mid_lat = (le_lat + he_lat) / 2.0;
        let mid_lon = (le_lon + he_lon) / 2.0;

        let le_world = converter.latlon_to_world(le_lat, le_lon);
        let he_world = converter.latlon_to_world(he_lat, he_lon);
        let center = (le_world + he_world) / 2.0;
        let length_m = le_world.distance(he_world);

        if length_m < 1.0 {
            continue;
        }

        let width_m = (width_ft as f32) * FEET_TO_METERS_F32;
        let angle = heading_to_rotation(heading);
        let rotation = Quat::from_rotation_z(angle);
        let material = if runway.is_closed() {
            closed_mat.clone()
        } else {
            open_mat.clone()
        };
        let mesh = meshes.add(Rectangle::new(width_m, length_m));

        commands.spawn((
            RunwayBody {
                runway_id: runway.id,
                le_lat,
                le_lon,
                he_lat,
                he_lon,
                heading_deg: heading,
                width_m,
                midpoint_lat: mid_lat,
                midpoint_lon: mid_lon,
            },
            Mesh2d(mesh),
            MeshMaterial2d(material),
            Transform {
                translation: Vec3::new(center.x, center.y, RUNWAY_BODY_Z),
                rotation,
                ..default()
            },
            Visibility::Hidden,
        ));

        // LE label
        if let Some(le_ident) = &runway.le_ident {
            let lp = le_label_pos(le_world, he_world);
            commands.spawn((
                RunwayLabel {
                    runway_id: runway.id,
                    le_lat,
                    le_lon,
                    he_lat,
                    he_lon,
                    heading_deg: heading,
                    is_he_end: false,
                    midpoint_lat: mid_lat,
                    midpoint_lon: mid_lon,
                },
                Text2d::new(le_ident.clone()),
                TextFont {
                    font_size: FontSize::Px(10.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                Transform {
                    translation: Vec3::new(lp.x, lp.y, RUNWAY_LABEL_Z),
                    rotation,
                    ..default()
                },
                Visibility::Hidden,
                RenderLayers::layer(RenderCategory::LABELS),
            ));
        }

        // HE label
        if let Some(he_ident) = &runway.he_ident {
            let hp = he_label_pos(le_world, he_world);
            let he_rotation = Quat::from_rotation_z(angle + std::f32::consts::PI);
            commands.spawn((
                RunwayLabel {
                    runway_id: runway.id,
                    le_lat,
                    le_lon,
                    he_lat,
                    he_lon,
                    heading_deg: heading,
                    is_he_end: true,
                    midpoint_lat: mid_lat,
                    midpoint_lon: mid_lon,
                },
                Text2d::new(he_ident.clone()),
                TextFont {
                    font_size: FontSize::Px(10.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                Transform {
                    translation: Vec3::new(hp.x, hp.y, RUNWAY_LABEL_Z),
                    rotation: he_rotation,
                    ..default()
                },
                Visibility::Hidden,
                RenderLayers::layer(RenderCategory::LABELS),
            ));
        }

        count += 1;
    }

    info!("Spawned {} runway body entities", count);
}
```

- [ ] **Step 3: Register `spawn_runway_entities` in `src/aviation/plugin.rs`**

Add to the imports at the top:
```rust
use super::{
    draw_navaids, draw_runways, poll_aviation_data_loading, spawn_airports,
    spawn_runway_entities, start_aviation_data_loading, update_airport_positions,
    update_airport_visibility, AirportRenderState, AviationData, NavaidRenderState,
    RunwayRenderState,
};
```

Add to the `Update` systems list:
```rust
.add_systems(
    Update,
    (
        poll_aviation_data_loading,
        spawn_airports,
        spawn_runway_entities,
        update_airport_positions.after(ZoomSet::Change),
        update_airport_visibility,
        draw_runways.after(ZoomSet::Change),
        draw_navaids.after(ZoomSet::Change),
    ),
);
```

- [ ] **Step 4: Build to verify no compile errors**

```bash
cargo build 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 5: Run app and verify runway entities exist via BRP**

```bash
cargo run 2>/dev/null &
sleep 10
```

Then query for runway body entities (in a separate terminal or via BRP tool):
```bash
# Wait for aviation data to load (watch the log for "Spawned N runway body entities")
# Then verify via BRP:
curl -s -X POST http://localhost:15702 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.query","id":1,"params":{"data":{},"filter":{"with":["airjedi_bevy::aviation::runways::RunwayBody"]}}}' \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print(f\"Found {len(d.get('result',[]))} RunwayBody entities\")"
```

Expected: `Found N RunwayBody entities` where N > 0 (typically 10,000-20,000).

- [ ] **Step 6: Commit**

```bash
git add src/aviation/runways.rs src/aviation/plugin.rs
git commit -m "Spawn runway body and label entities on aviation data load"
```

---

## Task 3: update_runway_positions and update_runway_visibility Systems

**Files:**
- Modify: `src/aviation/runways.rs`
- Modify: `src/aviation/plugin.rs`

**Interfaces:**
- Consumes: `RunwayBody`, `RunwayLabel` components (Task 1); `LocalOrigin`, `MapState`, `View3DState` (existing resources)
- Produces: correct `Transform` positions on map pan; correct `Visibility` based on zoom + distance + 2D mode

- [ ] **Step 1: Add `update_runway_positions` to `src/aviation/runways.rs`**

Add after `spawn_runway_entities`:

```rust
/// Recomputes runway body and label transforms when the floating origin shifts.
/// Early-returns on unchanged origin to avoid per-frame work.
pub fn update_runway_positions(
    local_origin: Res<LocalOrigin>,
    mut body_query: Query<(&RunwayBody, &mut Transform)>,
    mut label_query: Query<(&RunwayLabel, &mut Transform), Without<RunwayBody>>,
) {
    if !local_origin.is_changed() {
        return;
    }

    let converter = CoordinateConverter::new(&local_origin);

    for (body, mut transform) in body_query.iter_mut() {
        let le = converter.latlon_to_world(body.le_lat, body.le_lon);
        let he = converter.latlon_to_world(body.he_lat, body.he_lon);
        let center = (le + he) / 2.0;
        let angle = heading_to_rotation(body.heading_deg);
        transform.translation.x = center.x;
        transform.translation.y = center.y;
        transform.rotation = Quat::from_rotation_z(angle);
    }

    for (label, mut transform) in label_query.iter_mut() {
        let le = converter.latlon_to_world(label.le_lat, label.le_lon);
        let he = converter.latlon_to_world(label.he_lat, label.he_lon);
        let angle = heading_to_rotation(label.heading_deg);
        let pos = if label.is_he_end {
            he_label_pos(le, he)
        } else {
            le_label_pos(le, he)
        };
        let rotation = if label.is_he_end {
            Quat::from_rotation_z(angle + std::f32::consts::PI)
        } else {
            Quat::from_rotation_z(angle)
        };
        transform.translation.x = pos.x;
        transform.translation.y = pos.y;
        transform.rotation = rotation;
    }
}
```

- [ ] **Step 2: Add `update_runway_visibility` to `src/aviation/runways.rs`**

Add after `update_runway_positions`:

```rust
/// Toggles runway body and label entity visibility based on zoom level,
/// distance from map center, and 2D/3D mode (hidden in 3D mode).
pub fn update_runway_visibility(
    map_state: Res<MapState>,
    render_state: Res<RunwayRenderState>,
    view3d_state: Res<View3DState>,
    mut body_query: Query<(&RunwayBody, &mut Visibility)>,
    mut label_query: Query<(&RunwayLabel, &mut Visibility), Without<RunwayBody>>,
) {
    let zoom: u8 = map_state.zoom_level.to_u8();
    let show_bodies = render_state.show_runways
        && zoom >= 8
        && !view3d_state.is_3d_active();
    let show_labels = show_bodies && zoom >= 11;

    let center_lat = map_state.latitude;
    let center_lon = map_state.longitude;

    if !show_bodies {
        for (_, mut vis) in body_query.iter_mut() {
            *vis = Visibility::Hidden;
        }
        for (_, mut vis) in label_query.iter_mut() {
            *vis = Visibility::Hidden;
        }
        return;
    }

    for (body, mut vis) in body_query.iter_mut() {
        let in_range = (body.midpoint_lat - center_lat).abs()
            <= constants::AVIATION_FEATURE_BBOX_DEG
            && (body.midpoint_lon - center_lon).abs()
                <= constants::AVIATION_FEATURE_BBOX_DEG
            && haversine_distance_nm(
                center_lat,
                center_lon,
                body.midpoint_lat,
                body.midpoint_lon,
            ) <= constants::AVIATION_FEATURE_RADIUS_NM;
        *vis = if in_range {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }

    for (label, mut vis) in label_query.iter_mut() {
        if !show_labels {
            *vis = Visibility::Hidden;
            continue;
        }
        let in_range = (label.midpoint_lat - center_lat).abs()
            <= constants::AVIATION_FEATURE_BBOX_DEG
            && (label.midpoint_lon - center_lon).abs()
                <= constants::AVIATION_FEATURE_BBOX_DEG
            && haversine_distance_nm(
                center_lat,
                center_lon,
                label.midpoint_lat,
                label.midpoint_lon,
            ) <= constants::AVIATION_FEATURE_RADIUS_NM;
        *vis = if in_range {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}
```

- [ ] **Step 3: Register both new systems in `src/aviation/plugin.rs`**

Update the import:
```rust
use super::{
    draw_navaids, draw_runways, poll_aviation_data_loading, spawn_airports,
    spawn_runway_entities, start_aviation_data_loading, update_airport_positions,
    update_airport_visibility, update_runway_positions, update_runway_visibility,
    AirportRenderState, AviationData, NavaidRenderState, RunwayRenderState,
};
```

Update the systems list:
```rust
.add_systems(
    Update,
    (
        poll_aviation_data_loading,
        spawn_airports,
        spawn_runway_entities,
        update_airport_positions.after(ZoomSet::Change),
        update_airport_visibility,
        update_runway_positions.after(ZoomSet::Change),
        update_runway_visibility.after(ZoomSet::Change),
        draw_runways.after(ZoomSet::Change),
        draw_navaids.after(ZoomSet::Change),
    ),
);
```

- [ ] **Step 4: Build to verify no compile errors**

```bash
cargo build 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 5: Run app and visually verify runway bodies appear at zoom 8+**

Launch with debug logging:
```bash
RUST_LOG=airjedi_bevy::aviation=debug cargo run
```

Pan to an airport area (e.g., KICT at 37.65, -97.43) and zoom in to level 8. Expected: gray runway rectangles appear aligned with the true heading of each runway. Zoom in to level 11 and confirm white runway number labels appear, correctly rotated and readable.

- [ ] **Step 6: Commit**

```bash
git add src/aviation/runways.rs src/aviation/plugin.rs
git commit -m "Add runway visibility and position update systems"
```

---

## Task 4: Dashed Centerline in draw_runways

**Files:**
- Modify: `src/aviation/runways.rs`

**Interfaces:**
- Consumes: existing `draw_runways` system signature; `RunwayBody` approach is now the primary visual; gizmo centerline is additive at zoom 10+
- Produces: white dashed centerline gizmo drawn over the body mesh at zoom 10+; original solid line removed

- [ ] **Step 1: Add `draw_dashed_2d` helper to `src/aviation/runways.rs`**

Add before `draw_runways`:

```rust
/// Draw a dashed 2D line between two world-space points.
/// dash_m and gap_m are in world-space meters.
fn draw_dashed_2d(from: Vec2, to: Vec2, color: Color, dash_m: f32, gap_m: f32, gizmos: &mut Gizmos) {
    let delta = to - from;
    let total = delta.length();
    if total < 1.0 {
        return;
    }
    let dir = delta / total;
    let step = dash_m + gap_m;
    let mut t = 0.0f32;
    while t < total {
        let seg_end = (t + dash_m).min(total);
        gizmos.line_2d(from + dir * t, from + dir * seg_end, color);
        t += step;
    }
}
```

- [ ] **Step 2: Replace the body of `draw_runways` with centerline-only logic**

Replace the entire `draw_runways` function:

```rust
/// Draws dashed white centerlines over runway body meshes at zoom 10+.
/// The gray body rectangles are handled by RunwayBody mesh entities.
pub fn draw_runways(
    mut gizmos: Gizmos,
    aviation_data: Res<AviationData>,
    render_state: Res<RunwayRenderState>,
    local_origin: Res<LocalOrigin>,
    map_state: Res<MapState>,
    view3d_state: Res<crate::view3d::View3DState>,
) {
    if aviation_data.loading_state != LoadingState::Ready {
        return;
    }
    if !render_state.show_runways {
        return;
    }
    if view3d_state.is_3d_active() {
        return;
    }

    let zoom: u8 = map_state.zoom_level.to_u8();
    if zoom < 10 {
        return;
    }

    let converter = CoordinateConverter::new(&local_origin);
    let center_lat = map_state.latitude;
    let center_lon = map_state.longitude;
    let centerline_color = Color::srgba(1.0, 1.0, 1.0, 0.85);

    for runway in &aviation_data.runways {
        if !runway.has_valid_coords() || runway.is_closed() {
            continue;
        }

        let le_lat = runway.le_latitude_deg.unwrap();
        let le_lon = runway.le_longitude_deg.unwrap();
        let he_lat = runway.he_latitude_deg.unwrap();
        let he_lon = runway.he_longitude_deg.unwrap();
        let mid_lat = (le_lat + he_lat) / 2.0;
        let mid_lon = (le_lon + he_lon) / 2.0;

        if (mid_lat - center_lat).abs() > constants::AVIATION_FEATURE_BBOX_DEG
            || (mid_lon - center_lon).abs() > constants::AVIATION_FEATURE_BBOX_DEG
        {
            continue;
        }
        if haversine_distance_nm(center_lat, center_lon, mid_lat, mid_lon)
            > constants::AVIATION_FEATURE_RADIUS_NM
        {
            continue;
        }

        let le = converter.latlon_to_world(le_lat, le_lon);
        let he = converter.latlon_to_world(he_lat, he_lon);
        // 5% inset from each end so centerline doesn't overrun the threshold
        let inset = (he - le) * 0.05;
        draw_dashed_2d(le + inset, he - inset, centerline_color, 15.0, 10.0, &mut gizmos);
    }
}
```

- [ ] **Step 3: Build to verify no compile errors**

```bash
cargo build 2>&1 | grep -E "^error" | head -20
```

Expected: no errors.

- [ ] **Step 4: Run app and visually verify all three layers**

```bash
cargo run
```

Pan to KICT (Wichita, KS) at 37.65N, 97.43W. Expected behavior:
- **Zoom 8-9:** Gray rectangular runway bodies visible, correct heading, correct width. No centerlines, no numbers.
- **Zoom 10:** Centerline dashes appear in white over the gray body.
- **Zoom 11+:** White runway number labels appear at each end (e.g., "01L" / "19R"), rotated to align with the runway direction and readable from the threshold inward.
- **Switch to 3D mode:** All runway bodies and labels disappear.
- **Closed runways:** Appear darker/faded (closed_mat color).

- [ ] **Step 5: Commit**

```bash
git add src/aviation/runways.rs
git commit -m "Draw dashed runway centerlines via gizmos at zoom 10+"
```

---

## Self-Review

**Spec coverage:**
- Runway body (gray Mesh2d) - Task 2 ✓
- Centerline (white dashed gizmo, zoom 10+) - Task 4 ✓
- Runway numbers (Text2d, zoom 11+) - Task 2 ✓
- Filter to ~15k scheduled-service airports - Task 2 ✓
- position update on LocalOrigin change - Task 3 ✓
- visibility by zoom + distance - Task 3 ✓
- 3D mode hidden - Task 3 (`update_runway_visibility`) + Task 4 (`draw_runways`) ✓
- Colors: concrete gray open/closed, white centerline, white labels - Task 2 + 4 ✓
- Z-layers: body 4.5, labels 4.6 - Task 1 constants ✓
- Zoom thresholds: 8/10/11 - Tasks 2, 3, 4 ✓

**Placeholder scan:** No TBDs or TODOs. All code blocks complete. ✓

**Type consistency:**
- `RunwayBody` fields used in Task 2 spawn = same fields read in Task 3 position/visibility ✓
- `heading_to_rotation` defined Task 1, used Tasks 2 and 3 ✓
- `le_label_pos` / `he_label_pos` defined Task 1, used Tasks 2 and 3 ✓
- `RUNWAY_BODY_Z`, `RUNWAY_LABEL_Z`, `FEET_TO_METERS_F32` defined Task 1, used Task 2 ✓
