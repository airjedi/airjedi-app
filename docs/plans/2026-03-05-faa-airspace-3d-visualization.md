# FAA ADDS 3D Airspace Visualization Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Render FAA Class B/C and special use airspace as semi-transparent extruded wall meshes in 3D mode and filled polygons in 2D mode, with distance-based LOD culling.

**Architecture:** FAA ADDS ArcGIS provider feeds airspace GeoJSON through the existing data ingest pipeline. A consumer system converts canonical records into `Airspace` entities. A rendering system generates wall meshes (3D) and flat polygon meshes (2D) with lazy spawning and distance-based culling.

**Tech Stack:** Bevy 0.18, ArcGIS FeatureServer GeoJSON, existing data ingest pipeline, `StandardMaterial` with `AlphaMode::Blend`.

**Important Bevy 0.18 notes:**
- Messages use `#[derive(Message)]`, `add_message()`, `MessageWriter.write()` (NOT Event/add_event/EventWriter.send())
- `MessageReader` uses `.read()` to iterate received messages
- Run `cargo test -p airjedi_bevy` for tests, `cargo build` for compilation checks

---

### Task 1: Add faa_airspace to DataIngestConfig

**Files:**
- Modify: `src/config.rs:210-262`

**Step 1: Add faa_airspace field to DataIngestConfig**

In `src/config.rs`, add to the `DataIngestConfig` struct (after tfr field, ~line 224):

```rust
    #[serde(default = "DataIngestConfig::default_faa_airspace")]
    pub faa_airspace: ProviderConfig,
```

Add the default function in `impl DataIngestConfig` (after `default_tfr`, ~line 248):

```rust
    fn default_faa_airspace() -> ProviderConfig {
        ProviderConfig { enabled: false, schedule: "0 0 4 * * *".into(), api_key: None, api_secret: None }
    }
```

Add to `Default for DataIngestConfig` (after `tfr`, ~line 261):

```rust
            faa_airspace: Self::default_faa_airspace(),
```

**Step 2: Run build**

Run: `cargo build 2>&1 | tail -10`
Expected: Compiles (non-exhaustive match warnings are OK, we'll fix in Task 2).

**Step 3: Commit**

```
git add src/config.rs
git commit -m "Add faa_airspace to DataIngestConfig"
```

---

### Task 2: Add altitude ref fields to AirspaceInfo canonical type

**Files:**
- Modify: `src/data_ingest/canonical.rs:178-187`

**Step 1: Add fields to AirspaceInfo**

In `src/data_ingest/canonical.rs`, add two fields to `AirspaceInfo` (after `upper_limit_ft`, ~line 184):

```rust
    pub lower_altitude_ref: Option<String>,
    pub upper_altitude_ref: Option<String>,
```

**Step 2: Fix all AirspaceInfo constructors**

Search for all places that construct `AirspaceInfo` (in `openaip.rs` and test files) and add the new fields with `None` values. Use:

```
grep -rn "AirspaceInfo {" src/ tests/
```

For each constructor, add:
```rust
    lower_altitude_ref: None,
    upper_altitude_ref: None,
```

**Step 3: Run tests**

Run: `cargo test -p airjedi_bevy 2>&1 | tail -10`
Expected: All tests pass.

**Step 4: Commit**

```
git add src/data_ingest/canonical.rs src/data_ingest/providers/openaip.rs src/data_ingest/fixture_tests.rs
git commit -m "Add altitude reference fields to AirspaceInfo canonical type"
```

---

### Task 3: Create FAA ADDS Airspace Provider

**Files:**
- Create: `src/data_ingest/providers/faa_adds_airspace.rs`
- Modify: `src/data_ingest/providers/mod.rs`

**Step 1: Create the provider file**

Create `src/data_ingest/providers/faa_adds_airspace.rs`:

```rust
use chrono::Utc;
use serde_json::Value;

use crate::data_ingest::canonical::{AirspaceInfo, CanonicalRecord};
use crate::data_ingest::pipeline::{PipelineData, PipelineError, PipelinePhase, PipelineStage};
use crate::data_ingest::provider::{
    DataProvider, FetchContext, ProviderCategory, ProviderError, ProviderMeta, RawFetchResult,
};

const FAA_ADDS_AIRSPACE_URL: &str = "https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/Class_Airspace/FeatureServer/0/query";

/// Data provider that fetches Class B/C and Special Use Airspace from
/// the FAA ADDS ArcGIS FeatureServer.
pub struct FaaClassAirspaceProvider;

impl DataProvider for FaaClassAirspaceProvider {
    fn name(&self) -> &str {
        "faa_adds_airspace"
    }

    fn schedule(&self) -> &str {
        "0 0 4 * * *"
    }

    fn metadata(&self) -> ProviderMeta {
        ProviderMeta {
            display_name: "FAA Airspace",
            category: ProviderCategory::Navigation,
            description: "Class B/C and special use airspace from FAA ADDS",
            config_key: "faa_airspace",
        }
    }

    fn fetch(&self, _ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
        // Bulk fetch — no bounding box, get all B/C + SUA
        let url = format!(
            "{}?where={}&outFields=*&f=geojson",
            FAA_ADDS_AIRSPACE_URL,
            "CLASS+IN+('B','C')+OR+TYPE+IN+('R','MOA','W','A')"
        );

        let response = reqwest::blocking::get(&url)
            .map_err(|e| ProviderError::Network(format!("FAA ADDS airspace fetch failed: {}", e)))?;

        let bytes = response
            .bytes()
            .map_err(|e| ProviderError::Network(format!("Failed to read response: {}", e)))?;

        Ok(RawFetchResult {
            data: bytes.to_vec(),
            content_type: Some("application/json".to_string()),
            source: url,
        })
    }

    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
        vec![Box::new(FaaAirspaceParseStage)]
    }
}

struct FaaAirspaceParseStage;

impl PipelineStage for FaaAirspaceParseStage {
    fn name(&self) -> &str {
        "faa_adds_airspace_parse"
    }

    fn phase(&self) -> PipelinePhase {
        PipelinePhase::Parse
    }

    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
        let bytes = data
            .raw_bytes
            .as_ref()
            .ok_or_else(|| PipelineError::Parse("No raw data to parse".into()))?;

        let geojson: Value = serde_json::from_slice(bytes)
            .map_err(|e| PipelineError::Parse(format!("Invalid JSON: {}", e)))?;

        let features = geojson["features"]
            .as_array()
            .ok_or_else(|| PipelineError::Parse("No 'features' array in GeoJSON".into()))?;

        let now = Utc::now();
        let mut records = Vec::new();

        for feature in features {
            let props = &feature["properties"];

            let ident = props["IDENT"].as_str().unwrap_or("UNKNOWN");
            let name = props["NAME"].as_str().unwrap_or(ident);
            let class = props["CLASS"].as_str().unwrap_or("");
            let type_code = props["TYPE"].as_str().unwrap_or("");

            // Altitude values are in hundreds of feet
            let upper_val = props["UPPER_VAL"].as_i64().map(|v| (v * 100) as i32);
            let lower_val = props["LOWER_VAL"].as_i64().map(|v| (v * 100) as i32);
            let upper_code = props["UPPER_CODE"].as_str().map(String::from);
            let lower_code = props["LOWER_CODE"].as_str().map(String::from);

            // Determine airspace class string for canonical record
            let airspace_class = match class {
                "B" => "ClassB",
                "C" => "ClassC",
                _ => match type_code {
                    "R" => "Restricted",
                    "MOA" => "MOA",
                    "W" => "Warning",
                    "A" => "Alert",
                    _ => continue, // skip unknown types
                },
            };

            // Extract polygon coordinates from GeoJSON geometry
            let polygon = extract_polygon_coords(&feature["geometry"]);
            if polygon.is_empty() {
                continue;
            }

            records.push(CanonicalRecord::Airspace(AirspaceInfo {
                name: format!("{} {}", name, ident),
                airspace_class: airspace_class.to_string(),
                airspace_type: if class.is_empty() {
                    type_code.to_string()
                } else {
                    format!("Class {}", class)
                },
                lower_limit_ft: lower_val,
                upper_limit_ft: upper_val,
                lower_altitude_ref: lower_code,
                upper_altitude_ref: upper_code,
                polygon,
                fetched_at: now,
            }));
        }

        data.records = records;
        Ok(())
    }
}

/// Extract (lat, lon) pairs from a GeoJSON geometry (Polygon or MultiPolygon).
fn extract_polygon_coords(geometry: &Value) -> Vec<(f64, f64)> {
    let geo_type = geometry["type"].as_str().unwrap_or("");
    let coords = &geometry["coordinates"];

    match geo_type {
        "Polygon" => {
            // coords[0] = outer ring = [[lon, lat], ...]
            extract_ring(&coords[0])
        }
        "MultiPolygon" => {
            // coords[0][0] = first polygon's outer ring
            extract_ring(&coords[0][0])
        }
        _ => Vec::new(),
    }
}

fn extract_ring(ring: &Value) -> Vec<(f64, f64)> {
    ring.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|coord| {
                    let lon = coord[0].as_f64()?;
                    let lat = coord[1].as_f64()?;
                    Some((lat, lon))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_faa_airspace_geojson() {
        let geojson = r#"{
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "properties": {
                    "IDENT": "KICT",
                    "NAME": "WICHITA",
                    "CLASS": "C",
                    "UPPER_VAL": 53,
                    "UPPER_CODE": "MSL",
                    "LOWER_VAL": 0,
                    "LOWER_CODE": "SFC",
                    "TYPE": ""
                },
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[
                        [-97.43, 37.65],
                        [-97.20, 37.65],
                        [-97.20, 37.72],
                        [-97.43, 37.72],
                        [-97.43, 37.65]
                    ]]
                }
            }]
        }"#;

        let mut data = PipelineData {
            raw_bytes: Some(geojson.as_bytes().to_vec()),
            records: Vec::new(),
            metadata: std::collections::HashMap::new(),
        };

        let stage = FaaAirspaceParseStage;
        stage.execute(&mut data).unwrap();

        assert_eq!(data.records.len(), 1);
        if let CanonicalRecord::Airspace(ref info) = data.records[0] {
            assert!(info.name.contains("KICT"));
            assert_eq!(info.airspace_class, "ClassC");
            assert_eq!(info.upper_limit_ft, Some(5300));
            assert_eq!(info.lower_limit_ft, Some(0));
            assert_eq!(info.upper_altitude_ref.as_deref(), Some("MSL"));
            assert_eq!(info.lower_altitude_ref.as_deref(), Some("SFC"));
            assert_eq!(info.polygon.len(), 5);
        } else {
            panic!("Expected Airspace record");
        }
    }

    #[test]
    fn test_parse_restricted_airspace() {
        let geojson = r#"{
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "properties": {
                    "IDENT": "R-2508",
                    "NAME": "CHINA LAKE",
                    "CLASS": "",
                    "UPPER_VAL": 999,
                    "UPPER_CODE": "MSL",
                    "LOWER_VAL": 0,
                    "LOWER_CODE": "SFC",
                    "TYPE": "R"
                },
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[
                        [-117.8, 35.6],
                        [-117.5, 35.6],
                        [-117.5, 35.9],
                        [-117.8, 35.9],
                        [-117.8, 35.6]
                    ]]
                }
            }]
        }"#;

        let mut data = PipelineData {
            raw_bytes: Some(geojson.as_bytes().to_vec()),
            records: Vec::new(),
            metadata: std::collections::HashMap::new(),
        };

        let stage = FaaAirspaceParseStage;
        stage.execute(&mut data).unwrap();

        assert_eq!(data.records.len(), 1);
        if let CanonicalRecord::Airspace(ref info) = data.records[0] {
            assert_eq!(info.airspace_class, "Restricted");
            assert_eq!(info.upper_limit_ft, Some(99900));
        } else {
            panic!("Expected Airspace record");
        }
    }

    #[test]
    fn test_extract_multipolygon() {
        let geo = serde_json::json!({
            "type": "MultiPolygon",
            "coordinates": [[[
                [-97.43, 37.65],
                [-97.20, 37.65],
                [-97.20, 37.72],
                [-97.43, 37.72],
                [-97.43, 37.65]
            ]]]
        });

        let coords = extract_polygon_coords(&geo);
        assert_eq!(coords.len(), 5);
        assert!((coords[0].0 - 37.65).abs() < 0.001); // lat
        assert!((coords[0].1 - (-97.43)).abs() < 0.001); // lon
    }

    #[test]
    fn test_metadata() {
        let provider = FaaClassAirspaceProvider;
        let meta = provider.metadata();
        assert_eq!(meta.config_key, "faa_airspace");
        assert_eq!(meta.category, ProviderCategory::Navigation);
    }
}
```

**Step 2: Add module declaration**

In `src/data_ingest/providers/mod.rs`, add:

```rust
pub mod faa_adds_airspace;
```

**Step 3: Run tests**

Run: `cargo test -p airjedi_bevy -- faa_adds_airspace 2>&1 | tail -15`
Expected: 4 tests pass.

**Step 4: Commit**

```
git add src/data_ingest/providers/faa_adds_airspace.rs src/data_ingest/providers/mod.rs
git commit -m "Add FAA ADDS Class Airspace provider with GeoJSON parser"
```

---

### Task 4: Wire FAA airspace provider into scheduler and Ingest UI

**Files:**
- Modify: `src/data_ingest/mod.rs:148-185` (build_providers)
- Modify: `src/tools_window.rs` (schedule lookup)

**Step 1: Add to build_providers**

In `src/data_ingest/mod.rs`, add after the tfr block (~line 182):

```rust
    if config.faa_airspace.enabled {
        providers.push(Arc::new(providers::faa_adds_airspace::FaaClassAirspaceProvider));
    }
```

**Step 2: Add config_key mapping in tools_window.rs**

Find the schedule/config lookup logic in `tools_window.rs` (search for `"tfr"` in a match arm). Add after the tfr arm:

```rust
        "faa_airspace" => config.data_ingest.faa_airspace.schedule.clone(),
```

Also look for any `enabled` toggle mapping and add:
```rust
        "faa_airspace" => &mut config.data_ingest.faa_airspace,
```

**Step 3: Run build**

Run: `cargo build 2>&1 | tail -10`
Expected: Compiles.

**Step 4: Commit**

```
git add src/data_ingest/mod.rs src/tools_window.rs
git commit -m "Wire FAA airspace provider into scheduler and Ingest UI"
```

---

### Task 5: Add AIRSPACE render layer

**Files:**
- Modify: `src/render_layers.rs`
- Modify: `src/view3d/mod.rs` (camera subscriptions)

**Step 1: Add AIRSPACE constant**

In `src/render_layers.rs`, add after SKY (~line 25):

```rust
    pub const AIRSPACE: usize = 9;  // Airspace volumes (2D and 3D)
```

**Step 2: Add to camera layer subscriptions**

In `src/render_layers.rs`, add `RenderCategory::AIRSPACE` to `layers_2d_map()`:

```rust
pub fn layers_2d_map() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::TILES_2D,
        RenderCategory::GIZMOS,
        RenderCategory::OVERLAYS_2D,
        RenderCategory::LABELS,
        RenderCategory::AIRSPACE,
    ])
}
```

And add to `layers_3d_world()`:

```rust
pub fn layers_3d_world() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::DEFAULT,
        RenderCategory::TILES_3D,
        RenderCategory::GROUND,
        RenderCategory::SKY,
        RenderCategory::AIRSPACE,
    ])
}
```

**Step 3: Run build**

Run: `cargo build 2>&1 | tail -10`
Expected: Compiles.

**Step 4: Commit**

```
git add src/render_layers.rs
git commit -m "Add AIRSPACE render layer for airspace visualization"
```

---

### Task 6: Extend AirspaceDisplayState and add consumer system

**Files:**
- Modify: `src/airspace/mod.rs`

**Step 1: Add new fields to AirspaceDisplayState**

In `src/airspace/mod.rs`, add to `AirspaceDisplayState` (~line 266):

```rust
    pub show_warning: bool,
    pub show_alert: bool,
    pub opacity: f32,
    pub altitude_filter_ft: Option<i32>,
```

Update `Default for AirspaceDisplayState`:

```rust
impl Default for AirspaceDisplayState {
    fn default() -> Self {
        Self {
            enabled: false,
            show_class_b: true,
            show_class_c: true,
            show_class_d: true,
            show_restricted: true,
            show_moa: false,
            show_tfr: true,
            show_warning: true,
            show_alert: true,
            show_labels: true,
            opacity: 0.3,
            altitude_filter_ft: None,
        }
    }
}
```

**Step 2: Add dirty flag to AirspaceData**

Add to `AirspaceData` struct:

```rust
    pub dirty: bool,
```

Initialize as `false` in `Default`. Set `true` in `load_sample_data`.

**Step 3: Add helper to check if class is visible**

Add to `impl AirspaceDisplayState`:

```rust
    pub fn is_class_visible(&self, class: &AirspaceClass) -> bool {
        if !self.enabled {
            return false;
        }
        match class {
            AirspaceClass::ClassB => self.show_class_b,
            AirspaceClass::ClassC => self.show_class_c,
            AirspaceClass::ClassD => self.show_class_d,
            AirspaceClass::Restricted => self.show_restricted,
            AirspaceClass::MOA => self.show_moa,
            AirspaceClass::Warning => self.show_warning,
            AirspaceClass::Alert => self.show_alert,
            AirspaceClass::TFR => self.show_tfr,
            _ => false,
        }
    }

    pub fn passes_altitude_filter(&self, floor_ft: Option<i32>, ceiling_ft: Option<i32>) -> bool {
        let Some(filter) = self.altitude_filter_ft else {
            return true; // no filter = show all
        };
        let floor = floor_ft.unwrap_or(0);
        let ceiling = ceiling_ft.unwrap_or(60000);
        filter >= floor && filter <= ceiling
    }
```

**Step 4: Add consumer system**

Add `consume_airspace_data` system to `src/airspace/mod.rs`:

```rust
use crate::data_ingest::canonical::CanonicalRecord;

/// Consume NavigationDataUpdated messages and load airspace records into AirspaceData.
pub fn consume_airspace_data(
    mut nav_events: MessageReader<crate::data_ingest::NavigationDataUpdated>,
    mut airspace_data: ResMut<AirspaceData>,
) {
    for event in nav_events.read() {
        let airspaces: Vec<Airspace> = event
            .records
            .iter()
            .filter_map(|r| {
                if let CanonicalRecord::Airspace(info) = r {
                    Some(airspace_info_to_airspace(info))
                } else {
                    None
                }
            })
            .collect();

        if !airspaces.is_empty() {
            info!("Loaded {} airspace definitions from data ingest", airspaces.len());
            airspace_data.airspaces = airspaces;
            airspace_data.loaded = true;
            airspace_data.dirty = true;
            airspace_data.source = Some("FAA ADDS".to_string());
        }
    }
}

fn airspace_info_to_airspace(info: &crate::data_ingest::canonical::AirspaceInfo) -> Airspace {
    let class = match info.airspace_class.as_str() {
        "ClassB" => AirspaceClass::ClassB,
        "ClassC" => AirspaceClass::ClassC,
        "ClassD" => AirspaceClass::ClassD,
        "Restricted" => AirspaceClass::Restricted,
        "MOA" => AirspaceClass::MOA,
        "Warning" => AirspaceClass::Warning,
        "Alert" => AirspaceClass::Alert,
        "TFR" => AirspaceClass::TFR,
        _ => AirspaceClass::ClassG,
    };

    let floor = parse_altitude_ref(info.lower_limit_ft, info.lower_altitude_ref.as_deref());
    let ceiling = parse_altitude_ref(info.upper_limit_ft, info.upper_altitude_ref.as_deref());

    Airspace {
        id: info.name.clone(),
        name: info.name.clone(),
        class,
        floor,
        ceiling,
        boundary: info
            .polygon
            .iter()
            .map(|(lat, lon)| AirspacePoint {
                latitude: *lat,
                longitude: *lon,
            })
            .collect(),
        controlling_agency: None,
        frequency: None,
        operating_times: None,
    }
}

fn parse_altitude_ref(ft: Option<i32>, code: Option<&str>) -> AltitudeReference {
    match (ft, code) {
        (_, Some("SFC")) => AltitudeReference::Surface,
        (_, Some("UNL")) => AltitudeReference::Unlimited,
        (Some(v), Some("AGL")) => AltitudeReference::AGL(v),
        (Some(v), Some("FL")) => AltitudeReference::FL(v as u16 / 100),
        (Some(v), _) => AltitudeReference::MSL(v), // default to MSL
        (None, _) => AltitudeReference::Surface,
    }
}
```

**Step 5: Register consumer system in AirspacePlugin**

Add `MessageReader` import and register the system:

```rust
use bevy::ecs::message::MessageReader;

impl Plugin for AirspacePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AirspaceData>()
            .init_resource::<AirspaceDisplayState>()
            .add_systems(Update, (
                toggle_airspace_display,
                consume_airspace_data,
            ));
    }
}
```

**Step 6: Run build and tests**

Run: `cargo build 2>&1 | tail -10`
Expected: Compiles.

Run: `cargo test -p airjedi_bevy 2>&1 | tail -10`
Expected: All tests pass.

**Step 7: Commit**

```
git add src/airspace/mod.rs
git commit -m "Add airspace consumer system and extend display state"
```

---

### Task 7: Implement 3D wall mesh generation

**Files:**
- Modify: `src/airspace/mod.rs`

This is the core rendering task. Add a system that generates extruded wall meshes for visible airspace.

**Step 1: Add mesh-related components and resources**

```rust
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_resource::AsBindGroup;
use crate::render_layers::RenderCategory;
use crate::geo::CoordinateConverter;
use crate::view3d::View3DState;

/// Marker for spawned airspace mesh entities.
#[derive(Component)]
pub struct AirspaceMeshMarker {
    pub airspace_id: String,
}

/// Tracks which airspaces have spawned meshes.
#[derive(Resource, Default)]
pub struct SpawnedAirspaces {
    pub ids: std::collections::HashSet<String>,
}

/// Timer for periodic airspace mesh refresh.
#[derive(Resource)]
pub struct AirspaceRefreshTimer(pub Timer);

impl Default for AirspaceRefreshTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.5, TimerMode::Repeating))
    }
}
```

**Step 2: Add wall mesh builder function**

```rust
/// Build a wall mesh (vertical fence) from a polygon boundary between floor and ceiling.
fn build_wall_mesh(
    boundary: &[AirspacePoint],
    floor_z: f32,
    ceiling_z: f32,
    converter: &CoordinateConverter,
) -> Mesh {
    let n = boundary.len();
    if n < 2 {
        return Mesh::new(PrimitiveTopology::TriangleList, bevy::render::render_asset::RenderAssetUsages::default());
    }

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(n * 4);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(n * 4);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(n * 4);
    let mut indices: Vec<u32> = Vec::with_capacity(n * 6);

    for i in 0..n {
        let j = (i + 1) % n;

        let p0 = converter.latlon_to_world(boundary[i].latitude, boundary[i].longitude);
        let p1 = converter.latlon_to_world(boundary[j].latitude, boundary[j].longitude);

        // Y-up: x=east, y=altitude, z=-north
        let base_idx = positions.len() as u32;

        // Bottom-left, bottom-right, top-right, top-left
        positions.push([p0.x, floor_z, -p0.y]);
        positions.push([p1.x, floor_z, -p1.y]);
        positions.push([p1.x, ceiling_z, -p1.y]);
        positions.push([p0.x, ceiling_z, -p0.y]);

        // Normal pointing outward (cross product of edge x up)
        let edge = bevy::math::Vec3::new(p1.x - p0.x, 0.0, -(p1.y - p0.y));
        let up = bevy::math::Vec3::Y;
        let normal = edge.cross(up).normalize_or_zero();
        for _ in 0..4 {
            normals.push(normal.to_array());
        }

        uvs.push([0.0, 0.0]);
        uvs.push([1.0, 0.0]);
        uvs.push([1.0, 1.0]);
        uvs.push([0.0, 1.0]);

        // Two triangles per quad
        indices.push(base_idx);
        indices.push(base_idx + 1);
        indices.push(base_idx + 2);
        indices.push(base_idx);
        indices.push(base_idx + 2);
        indices.push(base_idx + 3);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, bevy::render::render_asset::RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}
```

**Step 3: Add 2D flat polygon mesh builder**

```rust
/// Build a flat polygon mesh for 2D map rendering.
fn build_flat_polygon_mesh(
    boundary: &[AirspacePoint],
    converter: &CoordinateConverter,
) -> Mesh {
    let n = boundary.len();
    if n < 3 {
        return Mesh::new(PrimitiveTopology::TriangleList, bevy::render::render_asset::RenderAssetUsages::default());
    }

    // Simple fan triangulation from centroid (works for convex and most concave polygons)
    let positions_2d: Vec<bevy::math::Vec2> = boundary
        .iter()
        .map(|p| converter.latlon_to_world(p.latitude, p.longitude))
        .collect();

    let cx: f32 = positions_2d.iter().map(|p| p.x).sum::<f32>() / n as f32;
    let cy: f32 = positions_2d.iter().map(|p| p.y).sum::<f32>() / n as f32;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(n + 1);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(n + 1);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(n + 1);
    let mut indices: Vec<u32> = Vec::with_capacity(n * 3);

    // Center vertex
    positions.push([cx, 0.01, -cy]); // slight Y offset above tiles
    normals.push([0.0, 1.0, 0.0]);
    uvs.push([0.5, 0.5]);

    for p in &positions_2d {
        positions.push([p.x, 0.01, -p.y]);
        normals.push([0.0, 1.0, 0.0]);
        uvs.push([0.0, 0.0]);
    }

    for i in 0..n {
        let j = (i + 1) % n;
        indices.push(0); // center
        indices.push((i + 1) as u32);
        indices.push((j + 1) as u32);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, bevy::render::render_asset::RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}
```

**Step 4: Add spawn/despawn system**

```rust
/// System to spawn/despawn airspace meshes based on visibility and distance.
pub fn update_airspace_meshes(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<AirspaceRefreshTimer>,
    airspace_data: Res<AirspaceData>,
    display_state: Res<AirspaceDisplayState>,
    view3d_state: Option<Res<View3DState>>,
    map_state: Res<crate::map::MapState>,
    tile_settings: Res<bevy_slippy_tiles::SlippyTilesSettings>,
    mut spawned: ResMut<SpawnedAirspaces>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    existing_query: Query<(Entity, &AirspaceMeshMarker)>,
) {
    timer.0.tick(time.delta());
    let needs_refresh = timer.0.just_finished() || airspace_data.dirty;

    if !needs_refresh {
        return;
    }

    if !display_state.enabled || !airspace_data.loaded {
        // Despawn all if disabled
        for (entity, _) in existing_query.iter() {
            commands.entity(entity).despawn();
        }
        spawned.ids.clear();
        return;
    }

    let converter = CoordinateConverter::new(&tile_settings, map_state.zoom_level);
    let is_3d = view3d_state
        .as_ref()
        .is_some_and(|v| v.mode == crate::view3d::ViewMode::Perspective3D);

    let camera_lat = map_state.latitude;
    let camera_lon = map_state.longitude;

    // Despawn meshes for airspaces no longer visible
    for (entity, marker) in existing_query.iter() {
        let should_keep = airspace_data.airspaces.iter().any(|a| {
            a.id == marker.airspace_id
                && display_state.is_class_visible(&a.class)
                && is_in_range(a, camera_lat, camera_lon)
        });
        if !should_keep {
            commands.entity(entity).despawn();
            spawned.ids.remove(&marker.airspace_id);
        }
    }

    // Spawn meshes for newly visible airspaces
    for airspace in &airspace_data.airspaces {
        if spawned.ids.contains(&airspace.id) {
            continue;
        }
        if !display_state.is_class_visible(&airspace.class) {
            continue;
        }
        if !is_in_range(airspace, camera_lat, camera_lon) {
            continue;
        }

        let floor_ft = altitude_ref_to_ft(&airspace.floor);
        let ceiling_ft = altitude_ref_to_ft(&airspace.ceiling);

        if !display_state.passes_altitude_filter(Some(floor_ft), Some(ceiling_ft)) {
            continue;
        }

        let mut color = airspace.class.color();
        color.set_alpha(display_state.opacity);

        let material = materials.add(StandardMaterial {
            base_color: color,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            unlit: true,
            ..default()
        });

        let mesh = if is_3d {
            let v3d = view3d_state.as_ref().unwrap();
            let floor_z = v3d.altitude_to_z(floor_ft);
            let ceiling_z = v3d.altitude_to_z(ceiling_ft);
            build_wall_mesh(&airspace.boundary, floor_z, ceiling_z, &converter)
        } else {
            build_flat_polygon_mesh(&airspace.boundary, &converter)
        };

        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::default(),
            RenderLayers::layer(RenderCategory::AIRSPACE),
            AirspaceMeshMarker {
                airspace_id: airspace.id.clone(),
            },
        ));

        spawned.ids.insert(airspace.id.clone());
    }
}

fn altitude_ref_to_ft(alt: &AltitudeReference) -> i32 {
    match alt {
        AltitudeReference::MSL(ft) => *ft,
        AltitudeReference::AGL(ft) => *ft,
        AltitudeReference::FL(fl) => *fl as i32 * 100,
        AltitudeReference::Surface => 0,
        AltitudeReference::Unlimited => 60000,
    }
}

/// Check if airspace centroid is within rendering range (~250nm).
fn is_in_range(airspace: &Airspace, camera_lat: f64, camera_lon: f64) -> bool {
    let (centroid_lat, centroid_lon) = airspace.centroid();
    let dlat = (centroid_lat - camera_lat).abs();
    let dlon = (centroid_lon - camera_lon).abs();
    // Rough degree-based check (~250nm ≈ 4 degrees at mid-latitudes)
    dlat < 4.0 && dlon < 4.0
}
```

**Step 5: Register resources and system in AirspacePlugin**

Update `AirspacePlugin::build()`:

```rust
impl Plugin for AirspacePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AirspaceData>()
            .init_resource::<AirspaceDisplayState>()
            .init_resource::<SpawnedAirspaces>()
            .init_resource::<AirspaceRefreshTimer>()
            .add_systems(Update, (
                toggle_airspace_display,
                consume_airspace_data,
                update_airspace_meshes,
            ));
    }
}
```

**Step 6: Add necessary imports at top of file**

```rust
use bevy::render::view::RenderLayers;
use bevy::render::mesh::{Indices, PrimitiveTopology};
```

**Step 7: Run build**

Run: `cargo build 2>&1 | tail -20`
Expected: Compiles.

**Step 8: Commit**

```
git add src/airspace/mod.rs
git commit -m "Add 3D wall mesh and 2D polygon mesh generation for airspace"
```

---

### Task 8: Update Airspace panel UI with new controls

**Files:**
- Modify: `src/tools_window.rs` (airspace panel section)

**Step 1: Find and update the airspace rendering in tools_window.rs**

Search for the airspace panel rendering code (look for `show_class_b`, `show_moa`). Add after the existing checkboxes:

```rust
                ui.checkbox(&mut display_state.show_warning, "Warning");
                ui.checkbox(&mut display_state.show_alert, "Alert");

                ui.separator();
                ui.add(egui::Slider::new(&mut display_state.opacity, 0.05..=1.0).text("Opacity"));

                ui.horizontal(|ui| {
                    let mut use_filter = display_state.altitude_filter_ft.is_some();
                    if ui.checkbox(&mut use_filter, "Alt Filter").changed() {
                        display_state.altitude_filter_ft = if use_filter { Some(10000) } else { None };
                    }
                    if let Some(ref mut alt) = display_state.altitude_filter_ft {
                        ui.add(egui::DragValue::new(alt).range(0..=60000).suffix(" ft"));
                    }
                });
```

**Step 2: Run build**

Run: `cargo build 2>&1 | tail -10`
Expected: Compiles.

**Step 3: Commit**

```
git add src/tools_window.rs
git commit -m "Add opacity slider and altitude filter to airspace panel"
```

---

### Task 9: Test with sample data and verify rendering

**Step 1: Run the app**

Run: `cargo run --release`

**Step 2: Verify**

1. Open Tools window, go to Airspace tab
2. Click "Load Sample Data" to verify mesh rendering with the existing sample airspace
3. Check 3D mode (press '3') — should see extruded wall meshes
4. Check 2D mode — should see flat filled polygons
5. Toggle classes on/off, adjust opacity slider
6. Test altitude filter

**Step 3: Enable FAA airspace in config**

Edit `~/.config/airjedi/config.toml` and add:
```toml
[data_ingest.faa_airspace]
enabled = true
schedule = "0 0 4 * * *"
```

Restart app and verify FAA data loads.

**Step 4: Fix any issues and commit**

Only commit if fixes were needed.
