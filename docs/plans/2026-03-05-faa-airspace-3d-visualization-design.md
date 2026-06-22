# FAA ADDS 3D Airspace Visualization Design

## Summary

Render FAA airspace volumes (Class B, C, Restricted, MOA, Warning, Alert) as semi-transparent extruded wall meshes in 3D mode and filled polygons in 2D mode. Data sourced from FAA ADDS ArcGIS FeatureServer via the existing data ingest system with HTTP caching. Distance-based LOD culls airspace outside the visible range.

## Scope

**Airspace classes:** Class B, Class C, Restricted, MOA, Warning, Alert
**Not included (initial):** Class D, Class E, Prohibited, TFR (TFRs already have their own provider/display)

## Architecture

Three layers:

1. **FAA ADDS Provider** — fetches and parses airspace GeoJSON into canonical records
2. **ECS Consumer** — converts canonical records into `Airspace` entities in the airspace module
3. **Renderer** — generates and manages wall meshes (3D) and flat polygon meshes (2D)

## Layer 1: FAA ADDS Airspace Provider

### Endpoint

ArcGIS FeatureServer query:
```
https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/Class_Airspace/FeatureServer/0/query
  ?where=CLASS IN ('B','C') OR TYPE IN ('R','MOA','W','A')
  &outFields=*
  &f=geojson
```

No bounding box — bulk fetch all matching US airspace. Estimated ~900 features (400 Class B/C tiers + 500 SUA).

### Provider Details

- **File:** `src/data_ingest/providers/faa_adds_airspace.rs`
- **Struct:** `FaaClassAirspaceProvider`
- **Schedule:** Daily (`0 0 4 * * *`)
- **Config key:** `faa_airspace`
- **Category:** Navigation
- **HTTP cache:** Use existing `http_cache` module for ETag/Last-Modified conditional fetches

### AirspaceInfo Enhancement

Add altitude reference fields to `src/data_ingest/canonical.rs`:

```rust
pub struct AirspaceInfo {
    // existing fields...
    pub lower_altitude_ref: Option<String>,  // "MSL", "AGL", "SFC"
    pub upper_altitude_ref: Option<String>,  // "MSL", "AGL", "FL"
}
```

### Key Fields from FAA ADDS Response

| Field | Type | Example | Mapping |
|-------|------|---------|---------|
| IDENT | string | "KICT" | name |
| CLASS | string | "C" | airspace_class |
| UPPER_VAL | int | 53 | upper_limit_ft = val * 100 |
| UPPER_CODE | string | "MSL" | upper_altitude_ref |
| LOWER_VAL | int | 0 | lower_limit_ft = val * 100 |
| LOWER_CODE | string | "SFC" | lower_altitude_ref |
| geometry | GeoJSON | Polygon/MultiPolygon | polygon vec |

### Config Addition

New field in `DataIngestConfig`:
```rust
pub faa_airspace: ProviderConfig,  // default: enabled=false, schedule="0 0 4 * * *"
```

## Layer 2: ECS Consumer

### System: `consume_airspace_data`

In `src/airspace/mod.rs`:

1. Read `NavigationDataUpdated` messages
2. Filter for `CanonicalRecord::Airspace` records
3. Convert `AirspaceInfo` → `Airspace`:
   - Map class strings ("B", "C", "R", "MOA", "W", "A") to `AirspaceClass` enum
   - Map altitude ref strings to `AltitudeReference` enum
   - Convert polygon `Vec<(f64, f64)>` to `Vec<AirspacePoint>`
4. Store in `AirspaceData` resource
5. Set `AirspaceData::dirty = true` to trigger mesh regeneration

### Data Flow

```
FAA ADDS FeatureServer
  → FaaClassAirspaceProvider.fetch()
  → Pipeline (parse GeoJSON → AirspaceInfo records)
  → crossbeam channel → drain_ingest_channel
  → NavigationDataUpdated message
  → consume_airspace_data system
  → AirspaceData resource
  → Mesh generation systems
```

## Layer 3: Rendering

### 3D Mode: Extruded Wall Meshes

For each airspace polygon, generate a "fence" of quads:

- For each edge (vertex[i] → vertex[i+1]):
  - Create a quad from floor altitude to ceiling altitude
  - 4 vertices: bottom-left, bottom-right, top-right, top-left
  - 2 triangles per quad
- Position vertices:
  - XY from `CoordinateConverter::latlon_to_world(lat, lon)`
  - Height from `View3DState::altitude_to_z(altitude_ft)`
  - Convert to Y-up via `zup_to_yup()`
- No top/bottom caps — see through from above/below

### 2D Mode: Flat Polygon Meshes

- Same polygon boundary rendered as a filled mesh at Z=0 (map surface)
- Use ear-clipping triangulation for the polygon fill
- Same material/colors as 3D mode

### Material

```rust
StandardMaterial {
    base_color: airspace_class.color(),  // existing per-class colors with alpha 0.3
    alpha_mode: AlphaMode::Blend,
    double_sided: true,
    cull_mode: None,
    unlit: true,  // consistent regardless of sun/lighting
}
```

### Render Layer

New constant in `render_layers.rs`:
```rust
pub const AIRSPACE: usize = 9;
```

Camera subscriptions:
- **3D mode Camera3d:** Add AIRSPACE layer
- **2D mode Camera2d:** Add AIRSPACE layer

### Components

```rust
#[derive(Component)]
struct AirspaceMesh {
    airspace_id: String,  // matches Airspace.id for lookup
}

#[derive(Component)]
struct AirspaceMesh2d;  // marker for 2D flat meshes

#[derive(Component)]
struct AirspaceMesh3d;  // marker for 3D wall meshes
```

## LOD and Culling

### Distance-Based Culling

- Compute bounding sphere for each airspace: centroid + max vertex distance
- Only generate/show meshes within `View3DState::visibility_range`
- Use Bevy's `VisibilityRange` component for automatic distance-based show/hide

### Refresh Timer

- `AirspaceRefreshTimer` — 500ms periodic check (similar to `Tile3DRefreshTimer`)
- On each tick: recompute which airspaces are in range, spawn/despawn meshes as needed
- Also triggered when `AirspaceData::dirty` flag is set (new data loaded)

### Mesh LOD (two levels)

| Range | Detail | Description |
|-------|--------|-------------|
| 0-80nm | Full | All polygon vertices |
| 80nm+ | Simplified | Douglas-Peucker simplification, ~50% vertex reduction |
| Beyond visibility_range | None | Mesh despawned |

### Lazy Generation

- `AirspaceData` stores all airspace records (hundreds)
- Only generate meshes for airspace passing distance check
- Track spawned airspace meshes to avoid duplicates (like `SpawnedTiles`)
- Despawn meshes when airspace moves out of range

## Settings

### AirspaceDisplayState Extensions

```rust
pub struct AirspaceDisplayState {
    // existing fields...
    pub enabled: bool,
    pub show_class_b: bool,
    pub show_class_c: bool,
    pub show_restricted: bool,
    pub show_moa: bool,
    pub show_labels: bool,
    // new fields:
    pub show_warning: bool,     // Warning areas
    pub show_alert: bool,       // Alert areas
    pub opacity: f32,           // 0.0-1.0, default 0.3
    pub altitude_filter_ft: Option<i32>,  // None = show all
}
```

### UI

Extend existing Airspace panel in Tools window:
- Per-class toggles (already exist, add Warning and Alert)
- Opacity slider (new)
- Altitude filter input with "Show All" option (new)

FAA Airspace provider appears in Ingest tab with enable/schedule controls.

## Coordinate System Notes

- All positions computed via `CoordinateConverter::latlon_to_world()` (same as tiles/aircraft)
- Heights via `View3DState::altitude_to_z()` (feet → world Z units, scaled by altitude_scale)
- 3D meshes converted from Z-up to Y-up via `zup_to_yup()`
- Polygon vertices are lat/lon pairs from GeoJSON
- FAA ADDS altitude values are in hundreds of feet (multiply by 100)
- SFC (surface) floors treated as ground elevation (0 or airport elevation)

## AGL Handling

For airspace with `LOWER_CODE = "AGL"`:
- Approximate as MSL by adding ground elevation from nearest airport
- Or treat as 0 if no reference available
- Full terrain elevation support deferred to future work

## Files Changed

| File | Change |
|------|--------|
| `src/data_ingest/providers/faa_adds_airspace.rs` | New: FAA ADDS provider |
| `src/data_ingest/providers/mod.rs` | Add module declaration |
| `src/data_ingest/canonical.rs` | Add altitude ref fields to AirspaceInfo |
| `src/data_ingest/mod.rs` | Add faa_airspace to build_providers |
| `src/data_ingest/provider.rs` | (none, trait already has metadata) |
| `src/config.rs` | Add faa_airspace to DataIngestConfig |
| `src/airspace/mod.rs` | Consumer system, mesh generation, LOD, components |
| `src/render_layers.rs` | Add AIRSPACE constant |
| `src/view3d/mod.rs` | Add AIRSPACE to Camera3d layers |
| `src/camera.rs` | Add AIRSPACE to Camera2d layers (2D mode) |
| `src/tools_window.rs` | Extend airspace panel UI |
| `tests/fixtures/data_ingest/` | FAA ADDS sample GeoJSON fixture |
