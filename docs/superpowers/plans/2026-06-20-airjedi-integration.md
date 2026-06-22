# AirJedi Fusion Integration Implementation Plan (Plan 3 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the `airjedi-fusion` crate into the AirJedi Bevy app. Replace the direct ADS-B-to-Aircraft pipeline with a fusion-mediated flow: ADS-B observations enter the fusion pipeline, fused tracks drive aircraft rendering. Add domain-generic render bridge, uncertainty visualization for coasting tracks, and fusion status in the UI. Existing rendering, camera, tiles, and UI panels remain unchanged.

**Architecture:** A `FusionIntegrationPlugin` sits between `airjedi-fusion` (which knows nothing about rendering) and AirJedi's existing visual systems. An ADS-B adapter converts `adsb_client::Aircraft` into `SensorObservation`s and pushes them into the fusion `ObservationBuffer`. A render bridge reads `TrackerState`/`TrackQuality`/`TargetClassification` from fusion track entities and creates or updates visual `Aircraft` entities (and future domain-specific entities). The fusion pipeline runs in `FixedUpdate` at 10Hz; display interpolation and rendering run per-frame in `Update`.

**Tech Stack:** Rust, Bevy 0.18, airjedi-fusion crate, existing AirJedi modules

**Spec:** `docs/superpowers/specs/2026-06-20-multi-sensor-fusion-pipeline-design.md`

**Depends on:** Plan 1 (core crate) must be complete. Plan 2 (NATS transport) is optional - the integration works without it.

## Global Constraints

- Do not modify existing rendering code (3D models, labels, trails, altitude coloring) - only change what feeds data into them
- The `Aircraft` component remains the rendering source of truth for visual systems - the render bridge writes to it
- Keep backwards compatibility: if fusion is disabled (feature flag off), the app works as before
- FixedUpdate rate: 10Hz (configurable via `FusionConfig`)
- The fusion crate is added as a workspace dependency: `airjedi-fusion = { path = "airjedi-fusion" }`

## File Structure

```
src/
├── main.rs                    (modify: add FusionIntegrationPlugin, remove direct adsb sync)
├── fusion_integration/
│   ├── mod.rs                 FusionIntegrationPlugin
│   ├── adsb_adapter.rs        AdsbAircraftData -> SensorObservation conversion
│   ├── render_bridge.rs       TrackerState -> Aircraft component sync
│   ├── interpolation.rs       Display interpolation between FixedUpdate states
│   ├── uncertainty_viz.rs     Coasting uncertainty ellipse rendering
│   └── fusion_ui.rs           Track status display in detail panel
└── aircraft/
    └── components.rs          (modify: add FusionTrackLink component)
```

---

### Task 1: Add Fusion Crate Dependency and Feature Flag

**Files:**
- Modify: `Cargo.toml` (root - add airjedi-fusion dep, feature flag)
- Test: `cargo check`

**Interfaces:**
- Consumes: nothing
- Produces: `fusion` feature flag, `airjedi-fusion` available as dependency

- [ ] **Step 1: Add dependency and feature to root Cargo.toml**

Add to `[features]`:
```toml
fusion = ["dep:airjedi-fusion"]
```

Update `default` to include fusion:
```toml
default = ["brp", "fusion"]
```

Add to `[dependencies]`:
```toml
airjedi-fusion = { path = "airjedi-fusion", optional = true }
```

- [ ] **Step 2: Verify compilation with and without**

Run:
```bash
cargo check
cargo check --no-default-features
```
Expected: both compile

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "Add airjedi-fusion dependency with fusion feature flag"
```

---

### Task 2: ADS-B Sensor Adapter

**Files:**
- Create: `src/fusion_integration/mod.rs`
- Create: `src/fusion_integration/adsb_adapter.rs`
- Modify: `src/main.rs` (add module declaration)

**Interfaces:**
- Consumes: `AdsbAircraftData` (existing resource from `src/adsb/connection.rs`), `adsb_client::Aircraft` struct
- Produces: `adsb_to_fusion_system` (system that drains ADS-B data into `ObservationBuffer`)

- [ ] **Step 1: Create `fusion_integration/mod.rs`**

```rust
// src/fusion_integration/mod.rs
#[cfg(feature = "fusion")]
mod adsb_adapter;
#[cfg(feature = "fusion")]
mod render_bridge;
#[cfg(feature = "fusion")]
mod interpolation;
#[cfg(feature = "fusion")]
mod uncertainty_viz;
#[cfg(feature = "fusion")]
mod fusion_ui;

use bevy::prelude::*;

pub struct FusionIntegrationPlugin;

#[cfg(feature = "fusion")]
impl Plugin for FusionIntegrationPlugin {
    fn build(&self, app: &mut App) {
        use airjedi_fusion::config::FusionConfig;
        use airjedi_fusion::FusionPlugin;
        use airjedi_fusion::systems::FusionSet;

        // Insert default fusion config if not already present
        if !app.world().contains_resource::<FusionConfig>() {
            app.insert_resource(FusionConfig::default());
        }

        app.add_plugins(FusionPlugin)
            .add_systems(
                FixedUpdate,
                adsb_adapter::adsb_to_fusion_system
                    .before(FusionSet::Drain),
            )
            .add_systems(
                Update,
                (
                    render_bridge::sync_tracks_to_visuals,
                    interpolation::interpolate_display_positions,
                    uncertainty_viz::render_uncertainty_ellipses,
                ),
            );
    }
}

#[cfg(not(feature = "fusion"))]
impl Plugin for FusionIntegrationPlugin {
    fn build(&self, _app: &mut App) {
        // No-op when fusion is disabled - existing ADS-B pipeline stays active
    }
}
```

- [ ] **Step 2: Create `fusion_integration/adsb_adapter.rs`**

```rust
// src/fusion_integration/adsb_adapter.rs
use bevy::prelude::*;
use airjedi_fusion::sensor::*;
use airjedi_fusion::systems::ObservationBuffer;
use airjedi_fusion::types::*;
use airjedi_fusion::coord::CoordinateFrame;
use chrono::Utc;
use nalgebra::DMatrix;
use crate::adsb::connection::AdsbAircraftData;

pub fn adsb_to_fusion_system(
    adsb_data: Res<AdsbAircraftData>,
    mut buffer: ResMut<ObservationBuffer>,
) {
    let aircraft_list = match adsb_data.aircraft.try_lock() {
        Ok(list) => list,
        Err(_) => return, // don't block the main thread
    };

    for ac in aircraft_list.iter() {
        let obs = adsb_aircraft_to_observation(ac);
        buffer.observations.push(obs);
    }
}

fn adsb_aircraft_to_observation(ac: &adsb_client::Aircraft) -> SensorObservation {
    let vel_north = ac.velocity.map(|v| {
        let heading_rad = ac.heading.unwrap_or(0.0).to_radians();
        v * 0.514444 * heading_rad.cos() // knots to m/s, north component
    });
    let vel_east = ac.velocity.map(|v| {
        let heading_rad = ac.heading.unwrap_or(0.0).to_radians();
        v * 0.514444 * heading_rad.sin() // knots to m/s, east component
    });
    let vel_down = ac.vertical_rate.map(|vr| -vr * 0.00508); // fpm to m/s, down positive

    let alt_m = ac.altitude.map(|a| f64::from(a) * 0.3048); // feet to meters

    // Map NACp accuracy category to position covariance (meters^2)
    // NACp 8 = 92.6m, NACp 9 = 30m, NACp 10 = 10m, NACp 11 = 3m
    let pos_variance = 100.0_f64.powi(2); // default 100m if NACp unknown
    let vel_variance = 10.0_f64.powi(2);  // default 10 m/s

    let mut cov = DMatrix::zeros(6, 6);
    for i in 0..3 { cov[(i, i)] = pos_variance; }
    for i in 3..6 { cov[(i, i)] = vel_variance; }

    SensorObservation {
        sensor_id: SensorId {
            id: "adsb-primary".to_string(),
            kind: SensorKind::AdsbReceiver,
            tier: FusionTier::Regional,
            coordinate_frame: CoordinateFrame::Wgs84,
        },
        timestamp: ac.last_seen,
        receipt_time: Utc::now(),
        target_id: Some(TargetId {
            domain: TargetDomain::Air,
            id: ac.icao.clone(),
            id_type: IdentifierType::Icao,
        }),
        measurement: Measurement::PositionVelocity3D {
            lat_deg: ac.latitude,
            lon_deg: ac.longitude,
            alt_m,
            vel_north_mps: vel_north,
            vel_east_mps: vel_east,
            vel_down_mps: vel_down,
            heading_deg: ac.heading,
        },
        covariance: ObservationCovariance { matrix: cov },
        classification_hint: Some(TargetCategory::FixedWing),
        metadata: ObservationMetadata {
            source_label: "ADS-B SBS1".to_string(),
            ..Default::default()
        },
    }
}
```

Note: the exact field names on `adsb_client::Aircraft` (`icao`, `latitude`, `longitude`, `altitude`, `heading`, `velocity`, `vertical_rate`, `last_seen`) should be verified against the actual struct. The adapter may need adjustment based on the actual field types (some may be `Option<>`, some may use different units).

- [ ] **Step 3: Commit**

```bash
git add src/fusion_integration/
git commit -m "Add ADS-B sensor adapter for fusion pipeline"
```

---

### Task 3: Render Bridge (Fusion Tracks to Visuals)

**Files:**
- Create: `src/fusion_integration/render_bridge.rs`
- Modify: `src/aircraft/components.rs` (add `FusionTrackLink` component)

**Interfaces:**
- Consumes: `Track`, `TrackerState`, `TrackQuality`, `TargetClassification` from fusion track entities
- Produces: Creates/updates `Aircraft` entities with position data from fused state. `FusionTrackLink` links visual entity to fusion track entity.

- [ ] **Step 1: Add `FusionTrackLink` to aircraft components**

Add to `src/aircraft/components.rs`:

```rust
#[derive(Component, Debug)]
pub struct FusionTrackLink {
    pub track_entity: Entity,
    pub track_id: airjedi_fusion::TrackId,
}
```

- [ ] **Step 2: Create `render_bridge.rs`**

```rust
// src/fusion_integration/render_bridge.rs
use bevy::prelude::*;
use airjedi_fusion::{Track, TrackerState, TrackQuality, TrackStatus, TargetClassification};
use airjedi_fusion::types::TargetCategory;
use crate::aircraft::components::{Aircraft, FusionTrackLink};

pub fn sync_tracks_to_visuals(
    mut commands: Commands,
    fusion_tracks: Query<
        (Entity, &Track, &TrackerState, &TrackQuality, &TargetClassification),
        Changed<TrackerState>,
    >,
    mut visuals: Query<(&FusionTrackLink, &mut Aircraft)>,
    visual_lookup: Query<(Entity, &FusionTrackLink)>,
) {
    for (track_entity, track, tracker, quality, classification) in &fusion_tracks {
        let (lat, lon, alt) = tracker.position_geodetic();

        // Find existing visual entity for this track
        let existing_visual = visual_lookup
            .iter()
            .find(|(_, link)| link.track_entity == track_entity);

        if let Some((visual_entity, _)) = existing_visual {
            // Update existing Aircraft component
            if let Ok((_, mut aircraft)) = visuals.get_mut(visual_entity) {
                aircraft.latitude = lat;
                aircraft.longitude = lon;
                aircraft.altitude = alt as i32;

                let vel_ecef = tracker.velocity_ecef();
                let speed_mps = (vel_ecef[0].powi(2) + vel_ecef[1].powi(2) + vel_ecef[2].powi(2)).sqrt();
                aircraft.velocity = Some(speed_mps / 0.514444); // m/s to knots

                // Extract heading from velocity components
                // Convert ECEF velocity to local NED for heading
                let (_, _, _) = tracker.position_geodetic();
                // Simplified heading from ENU velocity
                if speed_mps > 1.0 {
                    // Use the existing heading if available, or compute from velocity
                    // This is a simplification - proper NED conversion needed
                }

                aircraft.last_seen = track.last_update;

                // Update callsign from cooperative IDs
                if aircraft.callsign.is_none() || aircraft.callsign.as_deref() == Some("") {
                    for cid in &track.cooperative_ids {
                        if cid.id_type == airjedi_fusion::types::IdentifierType::Callsign {
                            aircraft.callsign = Some(cid.id.clone());
                            break;
                        }
                    }
                }
            }
        } else if quality.status != TrackStatus::Lost {
            // Spawn new visual entity for this track (aircraft only for now)
            if classification.category == TargetCategory::FixedWing
                || classification.category == TargetCategory::RotaryWing
                || classification.category == TargetCategory::Drone
                || classification.category == TargetCategory::Unknown
            {
                let icao = track.cooperative_ids
                    .iter()
                    .find(|id| id.id_type == airjedi_fusion::types::IdentifierType::Icao)
                    .map(|id| id.id.clone())
                    .unwrap_or_else(|| format!("TRK-{}", &track.id.0.to_string()[..8]));

                let callsign = track.cooperative_ids
                    .iter()
                    .find(|id| id.id_type == airjedi_fusion::types::IdentifierType::Callsign)
                    .map(|id| id.id.clone());

                commands.spawn((
                    Aircraft {
                        icao,
                        callsign,
                        latitude: lat,
                        longitude: lon,
                        altitude: alt as i32,
                        heading: None,
                        velocity: None,
                        vertical_rate: None,
                        squawk: None,
                        alert: false,
                        emergency: false,
                        spi: false,
                        last_seen: track.last_update,
                    },
                    FusionTrackLink {
                        track_entity,
                        track_id: track.id.clone(),
                    },
                ));
            }
        }

        // Handle Lost tracks - despawn visual
        if quality.status == TrackStatus::Lost {
            if let Some((visual_entity, _)) = existing_visual {
                commands.entity(visual_entity).despawn_recursive();
            }
        }
    }
}
```

Note: this is a simplified render bridge focused on aircraft. The `Aircraft` struct fields should be verified against the actual `src/aircraft/components.rs` definition. The spawned entity will need additional components (Transform, SceneRoot for 3D model, TrailHistory, etc.) that the existing aircraft sync system currently adds - those should be replicated here or extracted into a shared spawn helper.

- [ ] **Step 3: Commit**

```bash
git add src/fusion_integration/render_bridge.rs src/aircraft/components.rs
git commit -m "Add render bridge mapping fusion tracks to Aircraft visuals"
```

---

### Task 4: Display Interpolation

**Files:**
- Create: `src/fusion_integration/interpolation.rs`

**Interfaces:**
- Consumes: `TrackerState` (updated at FixedUpdate 10Hz), `Aircraft`, `FusionTrackLink`
- Produces: Smooth per-frame position updates between FixedUpdate ticks

- [ ] **Step 1: Create `interpolation.rs`**

```rust
// src/fusion_integration/interpolation.rs
use bevy::prelude::*;
use airjedi_fusion::TrackerState;
use crate::aircraft::components::{Aircraft, FusionTrackLink};

pub fn interpolate_display_positions(
    time: Res<Time>,
    fusion_tracks: Query<&TrackerState>,
    mut visuals: Query<(&FusionTrackLink, &mut Aircraft)>,
) {
    let dt = time.delta_secs_f64();

    for (link, mut aircraft) in &mut visuals {
        if let Ok(tracker) = fusion_tracks.get(link.track_entity) {
            // Simple linear extrapolation from last fused state
            let (lat, lon, alt) = tracker.position_geodetic();
            let vel = tracker.velocity_ecef();
            let speed_mps = (vel[0].powi(2) + vel[1].powi(2) + vel[2].powi(2)).sqrt();

            // Use existing InterpolationState if available, otherwise
            // do simple dead-reckoning from the fused position
            // The fused position already accounts for prediction in FixedUpdate,
            // so we only need to cover the fraction of time since the last FixedUpdate
            let _dt_fraction = dt; // fraction of FixedUpdate period

            // For now, the render bridge already writes the predicted position.
            // Full sub-frame interpolation can be added when needed.
            // The existing AirJedi InterpolationState system handles this
            // if the Aircraft component is being updated at FixedUpdate rate.

            let _ = (lat, lon, alt, speed_mps); // suppress unused warnings
        }
    }
}
```

Note: in practice, the existing `InterpolationState` and `interpolate_aircraft_positions` system in AirJedi already handles smooth frame-rate interpolation. The render bridge writes position at FixedUpdate rate, and the existing interpolation system smooths between updates. This file may reduce to a no-op if the existing system handles it well. Verify by running the app and checking for visual jitter at 10Hz FixedUpdate.

- [ ] **Step 2: Commit**

```bash
git add src/fusion_integration/interpolation.rs
git commit -m "Add display interpolation stub for fusion tracks"
```

---

### Task 5: Uncertainty Visualization

**Files:**
- Create: `src/fusion_integration/uncertainty_viz.rs`

**Interfaces:**
- Consumes: `TrackerState`, `TrackQuality`, `TrackStatus::Coasting`
- Produces: Visual uncertainty ellipse rendered for coasting tracks

- [ ] **Step 1: Create `uncertainty_viz.rs`**

```rust
// src/fusion_integration/uncertainty_viz.rs
use bevy::prelude::*;
use airjedi_fusion::{TrackerState, TrackQuality, TrackStatus};
use crate::aircraft::components::FusionTrackLink;
use crate::geo::CoordinateConverter;
use crate::map::MapState;

pub fn render_uncertainty_ellipses(
    fusion_tracks: Query<(&TrackerState, &TrackQuality)>,
    visuals: Query<(&FusionTrackLink, &Transform)>,
    map_state: Res<MapState>,
    mut gizmos: Gizmos,
) {
    for (link, transform) in &visuals {
        if let Ok((tracker, quality)) = fusion_tracks.get(link.track_entity) {
            if quality.status != TrackStatus::Coasting {
                continue;
            }

            // Extract position uncertainty from covariance diagonal
            // Convert ECEF covariance to local horizontal uncertainty (meters)
            let cov = tracker.variant.covariance_mat();
            if cov.nrows() < 3 {
                continue;
            }

            // Approximate horizontal uncertainty as sqrt of position covariance trace
            let pos_uncertainty_m = (cov[(0, 0)] + cov[(1, 1)]).sqrt();

            // Scale to world units (approximate: 1 degree lat ~ 111km)
            // This is rough - proper conversion through CoordinateConverter needed
            let uncertainty_world = pos_uncertainty_m / 111_000.0
                * 256.0 * (2.0_f64).powi(map_state.zoom_level.to_u8() as i32) as f64;

            // Draw ellipse at track position
            let radius = uncertainty_world as f32;
            if radius > 1.0 && radius < 10000.0 {
                let color = Color::srgba(1.0, 0.8, 0.2, 0.3); // translucent amber
                gizmos.circle_2d(
                    Isometry2d::from_translation(transform.translation.truncate()),
                    radius,
                    color,
                );
            }
        }
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add src/fusion_integration/uncertainty_viz.rs
git commit -m "Add uncertainty ellipse visualization for coasting tracks"
```

---

### Task 6: Fusion UI Panel

**Files:**
- Create: `src/fusion_integration/fusion_ui.rs`

**Interfaces:**
- Consumes: `Track`, `TrackerState`, `TrackQuality`, `TargetClassification` from fusion entities
- Produces: Fusion status info displayed in the existing detail panel or status bar

- [ ] **Step 1: Create `fusion_ui.rs`**

```rust
// src/fusion_integration/fusion_ui.rs
use bevy::prelude::*;
use airjedi_fusion::{Track, TrackerState, TrackQuality, TrackStatus, TargetClassification};

pub fn fusion_status_text(quality: &TrackQuality) -> &'static str {
    match quality.status {
        TrackStatus::Tentative => "TENTATIVE",
        TrackStatus::Confirmed => "CONFIRMED",
        TrackStatus::Coasting => "COASTING",
        TrackStatus::Lost => "LOST",
    }
}

pub fn fusion_status_color(quality: &TrackQuality) -> egui::Color32 {
    match quality.status {
        TrackStatus::Tentative => egui::Color32::from_rgb(180, 180, 100),
        TrackStatus::Confirmed => egui::Color32::from_rgb(100, 200, 100),
        TrackStatus::Coasting => egui::Color32::from_rgb(200, 150, 50),
        TrackStatus::Lost => egui::Color32::from_rgb(200, 80, 80),
    }
}

pub fn render_fusion_info(
    ui: &mut egui::Ui,
    quality: &TrackQuality,
    classification: &TargetClassification,
) {
    ui.horizontal(|ui| {
        ui.label("Track Status:");
        let color = fusion_status_color(quality);
        ui.colored_label(color, fusion_status_text(quality));
    });

    ui.horizontal(|ui| {
        ui.label("Sensors:");
        ui.label(format!("{}", quality.sensor_count));
    });

    ui.horizontal(|ui| {
        ui.label("Confidence:");
        ui.label(format!("{:.0}%", quality.confidence * 100.0));
    });

    ui.horizontal(|ui| {
        ui.label("Category:");
        ui.label(format!("{:?}", classification.category));
    });

    if quality.staleness.as_secs() > 0 {
        ui.horizontal(|ui| {
            ui.label("Stale:");
            ui.label(format!("{}s", quality.staleness.as_secs()));
        });
    }
}
```

This provides helper functions that the existing detail panel can call when displaying a fusion-tracked aircraft. Integration into the actual panel UI depends on the panel's egui layout code in `src/aircraft/detail_panel.rs`.

- [ ] **Step 2: Commit**

```bash
git add src/fusion_integration/fusion_ui.rs
git commit -m "Add fusion status UI helpers for track detail panel"
```

---

### Task 7: Wire Into Main App and Disable Direct ADS-B Sync

**Files:**
- Modify: `src/main.rs` (add FusionIntegrationPlugin, conditionally disable direct sync)

**Interfaces:**
- Consumes: all fusion_integration modules
- Produces: working app with fusion pipeline active when `fusion` feature is enabled

- [ ] **Step 1: Add FusionIntegrationPlugin to main.rs**

In `src/main.rs`, add the module declaration:
```rust
mod fusion_integration;
```

Add the plugin in the app builder, near the other plugin registrations:
```rust
.add_plugins(fusion_integration::FusionIntegrationPlugin)
```

- [ ] **Step 2: Conditionally disable direct ADS-B sync**

The existing `sync_aircraft_from_adsb` system in `src/adsb/sync.rs` directly creates Aircraft entities from ADS-B data. When fusion is active, this must be disabled because the fusion pipeline handles aircraft creation via the render bridge.

In `src/adsb/mod.rs` (or wherever AdsbPlugin registers systems), gate the sync system:

```rust
#[cfg(not(feature = "fusion"))]
app.add_systems(Update, sync_aircraft_from_adsb);

#[cfg(feature = "fusion")]
{
    // ADS-B sync is handled by fusion_integration::adsb_adapter
    // which feeds observations into the fusion pipeline
}
```

This ensures:
- With `fusion` feature: ADS-B -> fusion pipeline -> render bridge -> Aircraft entities
- Without `fusion` feature: ADS-B -> direct sync -> Aircraft entities (existing behavior)

- [ ] **Step 3: Test both feature configurations**

Run without fusion (should work exactly as before):
```bash
cargo run --no-default-features -F brp
```

Run with fusion:
```bash
cargo run
```

Both should display aircraft on the map. With fusion, there may be a brief delay before tracks confirm (Tentative -> Confirmed requires 3 observations).

- [ ] **Step 4: Commit**

```bash
git add src/main.rs src/adsb/
git commit -m "Wire FusionIntegrationPlugin into app, gate direct ADS-B sync on feature flag"
```

---

### Task 8: Visual Verification

**Files:** None (verification only)

**Interfaces:**
- Consumes: running AirJedi app with fusion enabled
- Produces: verified behavior via BRP inspection and visual checks

- [ ] **Step 1: Launch and check track creation**

```bash
cargo run --release
```

Use BRP to verify fusion tracks exist:
```
brp_status(app_name: "airjedi-bevy")
world_query(data: {}, filter: {with: ["airjedi_fusion::track::Track"]})
```

Should show track entities with TrackIds matching incoming aircraft.

- [ ] **Step 2: Verify Aircraft visuals**

```
brp_extras_screenshot(path: "tmp/fusion_test.png")
```

Aircraft should appear on the map as before. Check that:
- Positions match live ADS-B data
- Labels show callsigns
- Trails render correctly

- [ ] **Step 3: Inspect track quality**

```
world_get_components(entity: <track_entity_id>, components: [
    "airjedi_fusion::track::Track",
    "airjedi_fusion::track::TrackQuality",
    "airjedi_fusion::classification::TargetClassification"
])
```

Verify:
- Status is Confirmed (after a few seconds of data)
- sensor_count >= 1
- confidence > 0

- [ ] **Step 4: Test coasting behavior**

Temporarily disconnect the ADS-B source (or pause the adapter). Observe:
- After `coast_timeout` (15s): tracks transition to Coasting
- Uncertainty ellipses appear around coasting aircraft
- After `lost_timeout` (60s): tracks transition to Lost and visuals despawn

Reconnect the source and verify tracks reacquire.

- [ ] **Step 5: Commit any fixes found during verification**

```bash
git add -A
git commit -m "Fix issues found during fusion visual verification"
```

---

## Self-Review

**Spec coverage:**
- ADS-B adapter (AdsbAircraftData -> SensorObservation) - Task 2
- Render bridge (TrackerState -> Aircraft visuals) - Task 3
- Display interpolation (FixedUpdate to frame-rate smoothing) - Task 4
- Uncertainty visualization (coasting ellipse) - Task 5
- Fusion UI (track status, sensor count, confidence) - Task 6
- FixedUpdate/Update scheduling split - Task 2 (FusionPlugin runs in FixedUpdate, render bridge in Update)
- Feature flag for backwards compatibility - Tasks 1, 7
- System ordering (ingest before drain before associate before fuse before render) - Task 2 (plugin wiring)

**Not implemented (deferred for future work):**
- Domain-specific renderers for non-aircraft targets (ships, ground vehicles, people, satellites) - the render bridge currently only spawns Aircraft entities. When new target types are added, the render bridge dispatches to domain-specific renderers via TargetClassification.
- MIL-STD-2525D/APP-6D tactical symbology - symbology.rs is planned but not in scope for initial integration
- Fusion detail panel integration - fusion_ui.rs provides helpers but actual panel modification depends on the existing egui layout code which may need refactoring to accommodate fusion status alongside existing aircraft detail fields

**Type consistency:** `FusionTrackLink` links visual entities to fusion track entities by Bevy Entity reference and TrackId. The render bridge queries fusion tracks and visuals using this link component.

---

## Pre-Execution Notes (Lessons from Plans 1 & 2)

These adjustments should be applied when executing Plan 3, based on what we learned during Plans 1 and 2.

### Dependency Pattern

The fusion crate uses individual bevy sub-crates (`bevy_app`, `bevy_ecs`, etc.), NOT the `bevy` umbrella. When importing from `airjedi-fusion` in the app:
- The app already depends on `bevy = "0.18"` with full features
- `airjedi-fusion` types use `bevy_ecs::prelude::Component`, `bevy_ecs::prelude::Resource`, etc.
- These are the same types as `bevy::prelude::Component` - they're re-exported from the same underlying crates
- No compatibility issues expected since both use bevy 0.18

### Schedule Migration (FixedUpdate)

The fusion crate currently registers all systems in `Update` because `FixedUpdate` doesn't tick in test `App::update()` calls. Plan 3 should:
1. Keep the fusion crate's systems in `Update` as-is (the crate is testable this way)
2. In the AirJedi app's `FusionIntegrationPlugin`, override the schedule placement if needed using `.configure_sets()` to put fusion sets into `FixedUpdate`
3. OR accept `Update` scheduling for now and migrate to `FixedUpdate` as a separate optimization pass once the integration is working

Recommended: get it working in `Update` first, then profile and migrate to `FixedUpdate` if needed.

### Key Imports from airjedi-fusion

```rust
use airjedi_fusion::{
    FusionPlugin, FusionConfig, Track, TrackerState, TrackQuality, TrackStatus,
    TargetClassification, SensorObservation, Measurement, TimelineStore,
    TargetId, TargetDomain, TargetCategory, IdentifierType, Affiliation,
    Timestamp, TrackId,
};
use airjedi_fusion::config::FusionConfig;
use airjedi_fusion::coord::CoordinateFrame;
use airjedi_fusion::filter::ekf::ProcessNoiseConfig;
use airjedi_fusion::sensor::{SensorId, SensorKind, FusionTier, ObservationCovariance, ObservationMetadata};
use airjedi_fusion::systems::{ObservationBuffer, FusionSet};
use airjedi_fusion::transport::NatsTransportConfig;
```

### Feature Flag Pattern

Add to root `Cargo.toml`:
```toml
[features]
fusion = ["dep:airjedi-fusion"]

[dependencies]
airjedi-fusion = { path = "airjedi-fusion", optional = true }
```

Gate all fusion code with `#[cfg(feature = "fusion")]`. The app must compile and work without the fusion feature (existing ADS-B pipeline stays active).

### ADS-B Adapter Notes

The adapter needs to read from `AdsbAircraftData` (existing resource in `src/adsb/connection.rs`). Key fields on `adsb_client::Aircraft` to verify:
- `icao: String` (24-bit hex)
- `latitude: f64`, `longitude: f64` (degrees)
- `altitude: Option<i32>` (feet)
- `heading: Option<f64>` (degrees)
- `velocity: Option<f64>` (knots)
- `vertical_rate: Option<f64>` (feet per minute)
- `callsign: Option<String>`
- `squawk: Option<String>`
- `last_seen: DateTime<Utc>`

Unit conversions needed:
- altitude: feet -> meters (* 0.3048)
- velocity: knots -> m/s (* 0.514444)
- vertical_rate: fpm -> m/s down-positive (* -0.00508)

### What the Render Bridge Needs

The render bridge maps `TrackerState` -> `Aircraft` component. The `Aircraft` component fields (from `src/aircraft/components.rs`) need to be verified before implementation. The bridge must also spawn the visual bundle that the existing sync system creates (Transform, SceneRoot/WorldAssetRoot for 3D model, TrailHistory, InterpolationState, Pickable, etc.).
