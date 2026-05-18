# Aircraft Position Interpolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Smooth aircraft motion between ADS-B position updates using dead reckoning interpolation with proportional correction.

**Architecture:** A new `InterpolationState` component on each aircraft stores baseline ADS-B truth and per-frame display state. A new `interpolate_aircraft_positions` system advances display positions each frame via `predict_position()`. When real ADS-B data arrives, small errors blend smoothly (0.3s) while large errors snap. Existing systems that render Transforms read from `InterpolationState.display_*` instead of `Aircraft.*`.

**Tech Stack:** Bevy 0.18 ECS, existing `geo::predict_position()`, `geo::haversine_distance_nm()`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `src/aircraft/interpolation.rs` | Create | `InterpolationState` component, `BlendTarget` struct, `interpolate_aircraft_positions` system, heading helpers, `should_predict()` helper, `update_interpolation_on_adsb()` function |
| `src/aircraft/mod.rs` | Modify | Add `pub mod interpolation;` and re-export `InterpolationState` |
| `src/aircraft/plugin.rs` | Modify | Register `InterpolationState` type, add `interpolate_aircraft_positions` system with ordering |
| `src/adsb/sync.rs` | Modify | Insert `InterpolationState` on spawn, call `update_interpolation_on_adsb()` on update |
| `src/camera.rs` | Modify | `update_aircraft_positions` reads from `InterpolationState.display_*` |
| `src/view3d/mod.rs` | Modify | `update_aircraft_3d_transform` reads altitude/heading from `InterpolationState.display_*` |
| `src/config.rs` | Modify | Add `interpolation_enabled` field to `AppConfig` |

---

### Task 1: Create InterpolationState Component and BlendTarget

**Files:**
- Create: `src/aircraft/interpolation.rs`

- [ ] **Step 1: Create the interpolation module with component, structs, and constants**

```rust
use bevy::prelude::*;

pub const BLEND_THRESHOLD_NM: f64 = 0.5;
pub const BLEND_DURATION_SECS: f32 = 0.3;
pub const MAX_PREDICTION_SECS: f64 = 15.0;
pub const MIN_PREDICTION_SPEED_KTS: f64 = 10.0;

#[derive(Clone)]
pub struct BlendTarget {
    pub old_base_lat: f64,
    pub old_base_lon: f64,
    pub old_base_altitude: Option<f32>,
    pub old_base_heading: Option<f32>,
    pub old_base_speed: Option<f64>,
    pub old_base_vertical_rate: Option<f32>,
    pub old_base_time: f64,
    pub blend_start_time: f64,
    pub blend_duration: f32,
}

#[derive(Component, Clone, Reflect)]
#[reflect(Component)]
pub struct InterpolationState {
    pub base_lat: f64,
    pub base_lon: f64,
    pub base_altitude: Option<f32>,
    pub base_heading: Option<f32>,
    pub base_speed: Option<f64>,
    pub base_vertical_rate: Option<f32>,
    pub base_time: f64,

    #[reflect(ignore)]
    pub blend_target: Option<BlendTarget>,

    pub display_lat: f64,
    pub display_lon: f64,
    pub display_altitude: Option<f32>,
    pub display_heading: Option<f32>,

    pub predicting: bool,
}

impl InterpolationState {
    pub fn new(lat: f64, lon: f64, altitude: Option<i32>, heading: Option<f32>,
               speed: Option<f64>, vertical_rate: Option<i32>,
               is_on_ground: Option<bool>, current_time: f64) -> Self {
        let alt_f32 = altitude.map(|a| a as f32);
        let vrate_f32 = vertical_rate.map(|v| v as f32);
        let predicting = should_predict(heading, speed, is_on_ground);
        Self {
            base_lat: lat,
            base_lon: lon,
            base_altitude: alt_f32,
            base_heading: heading,
            base_speed: speed,
            base_vertical_rate: vrate_f32,
            base_time: current_time,
            blend_target: None,
            display_lat: lat,
            display_lon: lon,
            display_altitude: alt_f32,
            display_heading: heading,
            predicting,
        }
    }
}

pub fn should_predict(heading: Option<f32>, speed: Option<f64>, is_on_ground: Option<bool>) -> bool {
    if is_on_ground == Some(true) {
        return false;
    }
    let Some(_) = heading else { return false };
    let Some(spd) = speed else { return false };
    spd > MIN_PREDICTION_SPEED_KTS
}

pub fn shortest_angle_diff(from: f32, to: f32) -> f32 {
    let mut diff = (to - from) % 360.0;
    if diff > 180.0 {
        diff -= 360.0;
    } else if diff < -180.0 {
        diff += 360.0;
    }
    diff
}

pub fn lerp_heading(from: f32, to: f32, t: f32) -> f32 {
    let diff = shortest_angle_diff(from, to);
    let result = from + diff * t;
    ((result % 360.0) + 360.0) % 360.0
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | head -20`

This won't compile yet because the module isn't registered. That's expected.

- [ ] **Step 3: Commit**

```bash
git add src/aircraft/interpolation.rs
git commit -m "Add InterpolationState component and heading helpers"
```

---

### Task 2: Register the Interpolation Module

**Files:**
- Modify: `src/aircraft/mod.rs:1` (add module declaration and re-export)
- Modify: `src/aircraft/plugin.rs:1-3` (import and register type)

- [ ] **Step 1: Add module and re-export to `src/aircraft/mod.rs`**

Add after line 9 (`pub mod prediction;`):
```rust
pub mod interpolation;
```

Add to the re-export block (after the `pub use prediction::PredictionConfig;` line):
```rust
pub use interpolation::InterpolationState;
```

- [ ] **Step 2: Register the type in `src/aircraft/plugin.rs`**

Add to the imports at the top (after the `use super::` block, around line 8):
```rust
use super::interpolation::InterpolationState;
```

Add after `.register_type::<Aircraft>()` (line 28):
```rust
.register_type::<InterpolationState>()
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1 | head -20`
Expected: Success (warnings about unused code are fine)

- [ ] **Step 4: Commit**

```bash
git add src/aircraft/mod.rs src/aircraft/plugin.rs
git commit -m "Register interpolation module and InterpolationState type"
```

---

### Task 3: Add the `interpolate_aircraft_positions` System

**Files:**
- Modify: `src/aircraft/interpolation.rs` (add system function)
- Modify: `src/aircraft/plugin.rs` (register system with ordering)

- [ ] **Step 1: Add the dead reckoning system to `src/aircraft/interpolation.rs`**

Append to the end of the file:

```rust
use crate::geo;
use crate::config::AppConfig;

fn dead_reckon(lat: f64, lon: f64, heading: Option<f32>, speed: Option<f64>,
               altitude: Option<f32>, vertical_rate: Option<f32>,
               elapsed_secs: f64) -> (f64, f64, Option<f32>) {
    let (pred_lat, pred_lon) = match (heading, speed) {
        (Some(hdg), Some(spd)) if spd > MIN_PREDICTION_SPEED_KTS => {
            let elapsed_minutes = (elapsed_secs / 60.0) as f32;
            geo::predict_position(lat, lon, hdg, spd, elapsed_minutes)
        }
        _ => (lat, lon),
    };

    let pred_alt = match (altitude, vertical_rate) {
        (Some(alt), Some(vrate)) => Some((alt + vrate * elapsed_secs as f32 / 60.0).max(0.0)),
        (Some(alt), None) => Some(alt),
        _ => None,
    };

    (pred_lat, pred_lon, pred_alt)
}

pub fn interpolate_aircraft_positions(
    time: Res<Time<Real>>,
    config: Res<AppConfig>,
    mut query: Query<&mut InterpolationState>,
) {
    if !config.interpolation_enabled {
        return;
    }

    let now = time.elapsed_secs_f64();

    for mut interp in query.iter_mut() {
        let elapsed = now - interp.base_time;

        if elapsed > MAX_PREDICTION_SECS {
            interp.predicting = false;
            // Hold display at current values - don't advance further
            continue;
        }

        if !interp.predicting {
            continue;
        }

        // Dead reckon from current baseline
        let (new_lat, new_lon, new_alt) = dead_reckon(
            interp.base_lat, interp.base_lon,
            interp.base_heading, interp.base_speed,
            interp.base_altitude, interp.base_vertical_rate,
            elapsed,
        );

        if let Some(ref blend) = interp.blend_target {
            let blend_elapsed = now - blend.blend_start_time;
            let t = (blend_elapsed as f32 / blend.blend_duration).clamp(0.0, 1.0);

            // Dead reckon from the OLD baseline too
            let old_elapsed = now - blend.old_base_time;
            let (old_lat, old_lon, old_alt) = dead_reckon(
                blend.old_base_lat, blend.old_base_lon,
                blend.old_base_heading, blend.old_base_speed,
                blend.old_base_altitude, blend.old_base_vertical_rate,
                old_elapsed,
            );

            // Lerp between old and new dead-reckoned tracks
            let t64 = t as f64;
            interp.display_lat = old_lat + (new_lat - old_lat) * t64;
            interp.display_lon = old_lon + (new_lon - old_lon) * t64;

            interp.display_altitude = match (old_alt, new_alt) {
                (Some(o), Some(n)) => Some(o + (n - o) * t),
                (_, n) => n,
            };

            interp.display_heading = match (blend.old_base_heading, interp.base_heading) {
                (Some(old_h), Some(new_h)) => Some(lerp_heading(old_h, new_h, t)),
                (_, h) => h,
            };

            if t >= 1.0 {
                interp.blend_target = None;
            }
        } else {
            // No blend active - straight dead reckoning
            interp.display_lat = new_lat;
            interp.display_lon = new_lon;
            interp.display_altitude = new_alt;
            interp.display_heading = interp.base_heading;
        }
    }
}
```

- [ ] **Step 2: Register the system in `src/aircraft/plugin.rs`**

Add the import for the system (update the existing `use super::interpolation::InterpolationState;` line):
```rust
use super::interpolation::{InterpolationState, interpolate_aircraft_positions};
```

Add the system to the plugin `build` method. Insert after the existing `.add_systems(Update, render_detail_panel)` line (line 59):
```rust
.add_systems(Update, interpolate_aircraft_positions
    .after(crate::adsb::sync_aircraft_from_adsb))
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check 2>&1 | head -20`
Expected: May fail because `AppConfig` doesn't have `interpolation_enabled` yet. That's fine - we add it in a later task. For now, temporarily comment out the config check or add the field. Let's add it now since it's simple (see Task 6), or just verify the structure compiles by checking that the error is only about the missing field.

- [ ] **Step 4: Commit**

```bash
git add src/aircraft/interpolation.rs src/aircraft/plugin.rs
git commit -m "Add interpolate_aircraft_positions dead reckoning system"
```

---

### Task 4: Add `interpolation_enabled` to AppConfig

**Files:**
- Modify: `src/config.rs:78-92` (add field to `AppConfig`)
- Modify: `src/config.rs:281-301` (add default)

- [ ] **Step 1: Add field to `AppConfig` struct**

In `src/config.rs`, add after line 91 (`pub data_ingest: DataIngestConfig,`):
```rust
#[serde(default = "default_interpolation_enabled")]
pub interpolation_enabled: bool,
```

Add the default function near the top of the file (after the `const CONFIG_FILE` line):
```rust
fn default_interpolation_enabled() -> bool { true }
```

- [ ] **Step 2: Add default value in `Default` impl**

In the `Default` impl for `AppConfig` (around line 281), add after `data_ingest: DataIngestConfig::default(),`:
```rust
interpolation_enabled: true,
```

- [ ] **Step 3: Preserve field in settings save**

In `src/config.rs`, in `SettingsUiState::validate_and_build()` (around line 446), the built `AppConfig` needs the field. Since `validate_and_build` doesn't have access to the current `AppConfig`, default to `true`. Add after `data_ingest: self.data_ingest.clone(),`:
```rust
interpolation_enabled: default_interpolation_enabled(),
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check 2>&1 | head -20`
Expected: Success

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "Add interpolation_enabled config field"
```

---

### Task 5: Add `update_interpolation_on_adsb` and Wire Into Sync

**Files:**
- Modify: `src/aircraft/interpolation.rs` (add update function)
- Modify: `src/adsb/sync.rs:109-262` (insert component on spawn, call update on existing)

- [ ] **Step 1: Add `update_interpolation_on_adsb` to `src/aircraft/interpolation.rs`**

Append to the file:

```rust
pub fn update_interpolation_on_adsb(
    interp: &mut InterpolationState,
    new_lat: f64,
    new_lon: f64,
    new_altitude: Option<i32>,
    new_heading: Option<f32>,
    new_speed: Option<f64>,
    new_vertical_rate: Option<i32>,
    is_on_ground: Option<bool>,
    current_time: f64,
) {
    let error_nm = geo::haversine_distance_nm(
        interp.display_lat, interp.display_lon,
        new_lat, new_lon,
    );

    let new_alt_f32 = new_altitude.map(|a| a as f32);
    let new_vrate_f32 = new_vertical_rate.map(|v| v as f32);

    if error_nm < BLEND_THRESHOLD_NM {
        // Small error: set up a blend from old track to new track
        interp.blend_target = Some(BlendTarget {
            old_base_lat: interp.base_lat,
            old_base_lon: interp.base_lon,
            old_base_altitude: interp.base_altitude,
            old_base_heading: interp.base_heading,
            old_base_speed: interp.base_speed,
            old_base_vertical_rate: interp.base_vertical_rate,
            old_base_time: interp.base_time,
            blend_start_time: current_time,
            blend_duration: BLEND_DURATION_SECS,
        });
    } else {
        // Large error: snap display to new position immediately
        interp.display_lat = new_lat;
        interp.display_lon = new_lon;
        interp.display_altitude = new_alt_f32;
        interp.display_heading = new_heading;
        interp.blend_target = None;
    }

    // Reset baseline to new ADS-B truth
    interp.base_lat = new_lat;
    interp.base_lon = new_lon;
    interp.base_altitude = new_alt_f32;
    interp.base_heading = new_heading;
    interp.base_speed = new_speed;
    interp.base_vertical_rate = new_vrate_f32;
    interp.base_time = current_time;
    interp.predicting = should_predict(new_heading, new_speed, is_on_ground);
}
```

- [ ] **Step 2: Modify `sync_aircraft_from_adsb` in `src/adsb/sync.rs` to insert `InterpolationState` on spawn**

Add to the imports at the top of `src/adsb/sync.rs` (after existing `use` statements):
```rust
use crate::aircraft::interpolation::{InterpolationState, update_interpolation_on_adsb};
```

Add `time: Res<Time<Real>>` parameter to the `sync_aircraft_from_adsb` function signature. The new signature becomes:
```rust
pub fn sync_aircraft_from_adsb(
    mut commands: Commands,
    model_registry: Option<Res<AircraftModelRegistry>>,
    adsb_data: Option<Res<AdsbAircraftData>>,
    mut aircraft_query: Query<(Entity, &mut Aircraft, &mut Transform, Option<&mut InterpolationState>)>,
    label_query: Query<(Entity, &AircraftLabel)>,
    mut debug: Option<ResMut<DebugPanelState>>,
    theme: Res<AppTheme>,
    type_db: Option<Res<crate::aircraft::AircraftTypeDatabase>>,
    time: Res<Time<Real>>,
) {
```

In the spawn block (around line 188, the `commands.spawn((` call), add `InterpolationState` to the component bundle. After `TrailHistory::default(),` add:
```rust
InterpolationState::new(
    lat, lon,
    adsb_ac.altitude,
    adsb_ac.track.map(|t| t as f32),
    adsb_ac.velocity,
    adsb_ac.vertical_rate,
    adsb_ac.is_on_ground,
    time.elapsed_secs_f64(),
),
```

- [ ] **Step 3: Call `update_interpolation_on_adsb` when updating existing aircraft**

In the existing aircraft update block (around line 152-169), after updating `aircraft.last_seen`, add the interpolation update. The existing block updates the `Aircraft` component. After that block, add:

```rust
// Update interpolation state for smooth motion
if let Ok((_, _, _, Some(mut interp))) = aircraft_query.get_mut(entity) {
    update_interpolation_on_adsb(
        &mut interp,
        lat, lon,
        adsb_ac.altitude,
        adsb_ac.track.map(|t| t as f32),
        adsb_ac.velocity,
        adsb_ac.vertical_rate,
        adsb_ac.is_on_ground,
        time.elapsed_secs_f64(),
    );
}
```

Note: This second `get_mut` call requires restructuring the existing update block slightly. The current code does `if let Ok((_, mut aircraft, _)) = aircraft_query.get_mut(entity)` which borrows the query. We need to update both `Aircraft` and `InterpolationState` in the same `get_mut`. Restructure the existing update block to:

```rust
if let Ok((_, mut aircraft, _, interp_opt)) = aircraft_query.get_mut(entity) {
    aircraft.latitude = lat;
    aircraft.longitude = lon;
    aircraft.altitude = adsb_ac.altitude;
    aircraft.heading = adsb_ac.track.map(|t| t as f32);
    aircraft.velocity = adsb_ac.velocity;
    aircraft.vertical_rate = adsb_ac.vertical_rate;
    aircraft.callsign = adsb_ac.callsign.clone();
    aircraft.squawk = adsb_ac.squawk.clone();
    aircraft.is_on_ground = adsb_ac.is_on_ground;
    aircraft.alert = adsb_ac.alert;
    aircraft.emergency = adsb_ac.emergency;
    aircraft.spi = adsb_ac.spi;
    aircraft.last_seen = adsb_ac.last_seen;

    if let Some(mut interp) = interp_opt {
        update_interpolation_on_adsb(
            &mut interp,
            lat, lon,
            adsb_ac.altitude,
            adsb_ac.track.map(|t| t as f32),
            adsb_ac.velocity,
            adsb_ac.vertical_rate,
            adsb_ac.is_on_ground,
            time.elapsed_secs_f64(),
        );
    }
}
existing_aircraft.remove(&adsb_ac.icao);
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check 2>&1 | head -20`
Expected: Success

- [ ] **Step 5: Commit**

```bash
git add src/aircraft/interpolation.rs src/adsb/sync.rs
git commit -m "Wire InterpolationState into ADS-B sync: spawn and update"
```

---

### Task 6: Modify `update_aircraft_positions` to Read from InterpolationState

**Files:**
- Modify: `src/camera.rs:235-261` (`update_aircraft_positions`)

- [ ] **Step 1: Update `update_aircraft_positions` to read from `InterpolationState`**

Replace the `update_aircraft_positions` function in `src/camera.rs` (lines 235-261) with:

```rust
pub(crate) fn update_aircraft_positions(
    map_state: Res<MapState>,
    tile_settings: Res<SlippyTilesSettings>,
    config: Res<crate::config::AppConfig>,
    mut aircraft_query: Query<(&Aircraft, Option<&crate::aircraft::InterpolationState>, &mut Transform)>,
) {
    let converter = geo::CoordinateConverter::new(&tile_settings, map_state.zoom_level);

    for (aircraft, interp_opt, mut transform) in aircraft_query.iter_mut() {
        // Use interpolated display position if available and enabled, otherwise raw ADS-B
        let (lat, lon, heading) = if config.interpolation_enabled {
            if let Some(interp) = interp_opt {
                (interp.display_lat, interp.display_lon, interp.display_heading)
            } else {
                (aircraft.latitude, aircraft.longitude, aircraft.heading)
            }
        } else {
            (aircraft.latitude, aircraft.longitude, aircraft.heading)
        };

        let pos = converter.latlon_to_world(lat, lon);

        transform.translation.x = pos.x;
        transform.translation.y = pos.y;

        let base_rot = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
            * Quat::from_rotation_z(std::f32::consts::PI);
        if let Some(heading) = heading {
            transform.rotation = Quat::from_rotation_z((-heading).to_radians()) * base_rot;
        } else {
            transform.rotation = base_rot;
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | head -20`
Expected: Success

- [ ] **Step 3: Commit**

```bash
git add src/camera.rs
git commit -m "update_aircraft_positions reads from InterpolationState display values"
```

---

### Task 7: Modify `update_aircraft_3d_transform` to Read from InterpolationState

**Files:**
- Modify: `src/view3d/mod.rs:814-860` (`update_aircraft_3d_transform`)

- [ ] **Step 1: Update `update_aircraft_3d_transform` to read altitude and heading from `InterpolationState`**

This system runs after `update_aircraft_positions` and remaps the 2D pixel-space positions to Y-up 3D space. It currently reads `aircraft.altitude` and `aircraft.heading` directly. Change it to prefer `InterpolationState.display_*` values.

Replace the function signature and the 3D-active branch (lines 814-845):

```rust
pub fn update_aircraft_3d_transform(
    state: Res<View3DState>,
    config: Res<crate::config::AppConfig>,
    mut aircraft_query: Query<(&crate::Aircraft, Option<&crate::aircraft::InterpolationState>, &mut Transform), Without<crate::AircraftLabel>>,
    mut label_query: Query<(&crate::AircraftLabel, &mut Visibility)>,
) {
    if state.is_3d_active() {
        let ground_y = state.altitude_to_z(state.ground_elevation_ft);
        let min_aircraft_y = ground_y + 10.0;

        for (aircraft, interp_opt, mut transform) in aircraft_query.iter_mut() {
            let px = transform.translation.x;
            let py = transform.translation.y;

            // Use interpolated altitude/heading if available and enabled
            let (alt, heading) = if config.interpolation_enabled {
                if let Some(interp) = interp_opt {
                    (
                        interp.display_altitude.map(|a| a as i32).unwrap_or(0),
                        interp.display_heading,
                    )
                } else {
                    (aircraft.altitude.unwrap_or(0), aircraft.heading)
                }
            } else {
                (aircraft.altitude.unwrap_or(0), aircraft.heading)
            };

            let alt_y = state.altitude_to_z(alt).max(min_aircraft_y);
            transform.translation = Vec3::new(px, alt_y, -py);

            let base_rot = crate::camera::BASE_ROT_YUP;
            if let Some(heading) = heading {
                transform.rotation =
                    Quat::from_rotation_y((-heading).to_radians()) * base_rot;
            } else {
                transform.rotation = base_rot;
            }
        }
        for (_label, mut vis) in label_query.iter_mut() {
            *vis = Visibility::Hidden;
        }
    } else if !state.is_transitioning() {
        for (_aircraft, _interp, mut transform) in aircraft_query.iter_mut() {
            transform.translation.z = crate::constants::AIRCRAFT_Z_LAYER;
        }
        for (_label, mut vis) in label_query.iter_mut() {
            if *vis == Visibility::Hidden {
                *vis = Visibility::Inherited;
            }
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | head -20`
Expected: Success

- [ ] **Step 3: Commit**

```bash
git add src/view3d/mod.rs
git commit -m "update_aircraft_3d_transform reads altitude/heading from InterpolationState"
```

---

### Task 8: Add System Ordering for `interpolate_aircraft_positions`

**Files:**
- Modify: `src/aircraft/plugin.rs` (refine system ordering)
- Modify: `src/camera.rs:44-78` (add `.after(interpolate_aircraft_positions)`)

- [ ] **Step 1: Update ordering in `src/camera.rs`**

In `CameraPlugin::build`, change the `update_aircraft_positions` system registration (around line 65) to also run after the interpolation system:

```rust
.add_systems(
    Update,
    update_aircraft_positions
        .after(update_camera_position)
        .after(crate::adsb::sync_aircraft_from_adsb)
        .after(crate::aircraft::interpolation::interpolate_aircraft_positions)
        .after(ZoomSet::Change),
)
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | head -20`
Expected: Success

- [ ] **Step 3: Verify the full system chain**

Run: `cargo check 2>&1 | tail -5`
Expected: Success with no errors. The ordering is now:
1. `sync_aircraft_from_adsb` (updates Aircraft, triggers InterpolationState blend/snap)
2. `interpolate_aircraft_positions` (advances display_* via dead reckoning each frame)
3. `update_aircraft_positions` (converts display lat/lon to world coords, sets Transform)
4. `update_aircraft_3d_transform` (remaps to Y-up in 3D mode)
5. `update_aircraft_labels` (follows Transform)

- [ ] **Step 4: Commit**

```bash
git add src/camera.rs
git commit -m "Enforce system ordering: interpolation runs between sync and position update"
```

---

### Task 9: Build and Smoke Test

**Files:** None (verification only)

- [ ] **Step 1: Full build**

Run: `cargo build 2>&1 | tail -10`
Expected: Successful build

- [ ] **Step 2: Release build**

Run: `cargo build --release 2>&1 | tail -10`
Expected: Successful build

- [ ] **Step 3: Run the app and verify aircraft appear and move smoothly**

Run: `cargo run --release`

Verification checklist:
- Aircraft appear on the map at correct positions
- Aircraft move continuously between ADS-B updates (no freezing)
- Aircraft don't jump abruptly on small corrections
- Aircraft snap correctly when large corrections arrive (e.g., new aircraft or turns)
- 3D mode: aircraft altitude changes smoothly during climbs/descents
- 3D mode: aircraft heading rotation is smooth
- Trails still record from real ADS-B positions (not interpolated)
- Labels follow aircraft smoothly
- Staleness fading still works

- [ ] **Step 4: Test with interpolation disabled**

Set `interpolation_enabled = false` in `config.toml` and restart. Verify behavior is identical to pre-change behavior (aircraft jump on updates).

- [ ] **Step 5: Commit any fixes from testing**

```bash
git add -A
git commit -m "Fix issues found during smoke testing"
```

(Only if fixes were needed)
