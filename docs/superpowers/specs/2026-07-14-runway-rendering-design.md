# Runway Rendering Design

**Date:** 2026-07-14
**Status:** Approved

## Overview

Replace the current single-line runway gizmo with a three-layer rendering system that shows runways as properly-sized gray rectangles with white centerlines and rotated runway number labels. All data required (width, length, endpoint coordinates, headings, identifiers) is available from the existing OurAirports dataset loaded in `AviationData`.

## Data Available

From `Runway` in `src/aviation/types.rs`:
- `width_ft: Option<i32>` - runway width in feet
- `length_ft: Option<i32>` - runway length in feet (also derivable from endpoints)
- `le_lat/lon`, `he_lat/lon` - precise coordinates for both ends
- `le_heading_deg_t`, `he_heading_deg_t` - true headings for each end
- `le_ident`, `he_ident` - runway numbers e.g. "09L", "27R"
- `surface: Option<String>` - surface type
- `closed: Option<i32>` - closed flag

## Architecture

Three rendering layers per runway:

1. **Body** - `Mesh2d(Rectangle)` entity, solid gray, rotated to runway heading
2. **Numbers** - Two `Text2d` entities per runway (one per end), rotated to reading direction
3. **Centerline** - Dashed white gizmo line, drawn in the existing `draw_runways` system

## Zoom Thresholds

| Zoom | Features shown |
|------|---------------|
| < 8  | Nothing |
| 8+   | Runway body rectangles |
| 10+  | Centerline dashes |
| 11+  | Runway number labels |

## Entity Lifecycle

### Spawn (`spawn_runway_entities`)

Runs once when `AviationData` transitions to `LoadingState::Ready` and no runway entities exist yet. Filters to runways whose `airport_ref` maps to an airport that passes `AirportFilter::FrequentlyUsed` (has scheduled service) - the same filter used by the airport marker spawning system. This reduces the ~48k runway dataset to approximately 15,000 entity sets.

Per runway, spawns:
- 1 `RunwayBody` mesh entity (initially `Visibility::Hidden`)
- 2 `RunwayLabel` text entities, one per end (initially `Visibility::Hidden`)

### Visibility (`update_runway_visibility`)

Runs each frame. For each runway entity, checks zoom level and whether the runway midpoint falls within `AVIATION_FEATURE_BBOX_DEG` and `AVIATION_FEATURE_RADIUS_NM` of the map center. Sets `Visibility::Inherited` or `Visibility::Hidden` accordingly. Also hides number text entities when zoom < 11.

### Position update (`update_runway_positions`)

Runs each frame but early-returns unless `local_origin.is_changed()` (floating origin recentered). Recomputes world-space transforms for all runway body and label entities using `CoordinateConverter`.

The existing `draw_runways` gizmo system is kept for the centerline, upgraded to draw a dashed line instead of the current solid line.

## Coordinate Math

**Midpoint:**
```
mid = (le_world + he_world) / 2
```

**Rectangle size:**
```
width_m  = width_ft * 0.3048
length_m = distance(le_world, he_world)  // world-space meters
```

**Rotation:**
```
angle_rad = -le_heading_deg_t.to_radians()
rotation  = Quat::from_rotation_z(angle_rad)
```
(Heading 0° = north maps to Y-up Rectangle with no rotation; heading 90° = east rotates CW by 90°.)

**Number positions:**
- Inset 12% of runway length from each threshold toward center
- LE end: same rotation as body (`angle_rad`)
- HE end: rotated 180° (`angle_rad + PI`)
- Font size: `10.0` px (smaller than `BASE_FONT_SIZE = 14.0` to fit within narrow runways)

**Centerline:**
- Drawn from LE to HE world positions, slightly inset from thresholds (5% inset each end)
- Uses a local `draw_dashed_2d` helper in `runways.rs` (the existing `draw_dashed` in `trail_renderer` is private)

## Colors

| Element | Color | Value |
|---------|-------|-------|
| Body (open) | Medium concrete gray | `Color::srgba(0.55, 0.55, 0.55, 1.0)` |
| Body (closed) | Faded dark gray | `Color::srgba(0.35, 0.35, 0.35, 0.7)` |
| Centerline | White | `Color::srgba(1.0, 1.0, 1.0, 0.85)` |
| Numbers | White | `Color::WHITE` |

## Z-Layers

| Entity | Z / Layer |
|--------|-----------|
| Runway body | Z = 4.5 (above tiles at ~2-3, below airport markers at 5.0) |
| Centerline | Gizmo layer (MapCamera, `RenderCategory::GIZMOS`) |
| Numbers | `RenderLayers::layer(RenderCategory::LABELS)` |

## New Components and Resources

```rust
// Marks a runway body mesh entity
#[derive(Component)]
pub struct RunwayBody {
    pub runway_id: i64,
}

// Marks a runway number text entity
#[derive(Component)]
pub struct RunwayLabel {
    pub runway_id: i64,
    pub is_he_end: bool,   // false = LE end, true = HE end
}
```

`RunwayRenderState` (already exists) gains no new fields - `show_runways` drives all three layers.

## Files Changed

| File | Change |
|------|--------|
| `src/aviation/runways.rs` | Add `spawn_runway_entities`, `update_runway_visibility`, `update_runway_positions`; upgrade `draw_runways` gizmos to draw dashed centerline + outline at zoom 8+; add `RunwayBody` and `RunwayLabel` components |
| `src/aviation/plugin.rs` | Register three new systems in `Update` schedule |
| `src/aviation/types.rs` | No changes needed |

## Out of Scope

- 3D mode runway rendering (elevated mesh on ground plane)
- Displaced threshold markings
- Touchdown zone markings
- Taxiway rendering
- Surface-type color differentiation (grass vs. asphalt)
