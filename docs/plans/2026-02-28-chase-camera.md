# Chase Camera Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** When an aircraft is followed in 3D mode, smoothly transition the camera to a chase position 100 ft behind, 25 ft above, with 5 deg down pitch tracking the aircraft's heading.

**Architecture:** Extend `View3DState` with chase mode fields. The existing `follow_aircraft_3d` system gains chase logic that continuously lerps orbit params (yaw, pitch, altitude) toward chase targets. Escape or clicking empty space clears `following_icao`, which deactivates chase and restores pre-chase orbit params.

**Tech Stack:** Bevy 0.18 ECS, existing View3DState/CameraFollowState resources

---

### Task 1: Add chase state fields to View3DState

**Files:**
- Modify: `src/view3d/mod.rs:66-93` (View3DState struct)
- Modify: `src/view3d/mod.rs:99-119` (Default impl)

**Step 1: Add chase fields to `View3DState`**

Add after `saved_2d_zoom_level` (line 92):

```rust
    /// Whether the camera is in chase mode (tracking aircraft heading)
    pub chase_active: bool,
    /// Progress of the initial transition into chase position (0.0 to 1.0)
    pub chase_transition: f32,
    /// Saved orbit parameters from before chase started
    pub pre_chase_pitch: f32,
    pub pre_chase_yaw: f32,
    pub pre_chase_altitude: f32,
```

**Step 2: Add defaults**

Add to the Default impl after `saved_2d_zoom_level: None,` (line 116):

```rust
            chase_active: false,
            chase_transition: 0.0,
            pre_chase_pitch: DEFAULT_PITCH,
            pre_chase_yaw: 0.0,
            pre_chase_altitude: DEFAULT_CAMERA_ALTITUDE,
```

**Step 3: Add chase constants**

Add after `const ALTITUDE_EXAGGERATION: f32 = 1.0;` (line 41):

```rust
const CHASE_OFFSET_BEHIND_FT: f32 = 100.0;
const CHASE_OFFSET_ABOVE_FT: f32 = 25.0;
const CHASE_PITCH: f32 = 5.0;
const CHASE_TRANSITION_DURATION: f32 = 2.0;
```

**Step 4: Build and verify**

Run: `cargo build 2>&1 | grep "^error"`
Expected: No errors (warnings OK)

**Step 5: Commit**

```
feat: add chase camera state fields to View3DState
```

---

### Task 2: Add chase activation logic to `follow_aircraft_3d`

**Files:**
- Modify: `src/aircraft/picking.rs:117-154` (follow_aircraft_3d system)

**Step 1: Update `follow_aircraft_3d` to activate chase and track heading**

Replace the entire `follow_aircraft_3d` function body. The key changes:
- When `following_icao` transitions from None to Some, save current orbit params and set `chase_active = true`
- When chase is active, lerp yaw toward aircraft heading + 180, pitch toward chase pitch, altitude toward aircraft alt + offset
- When following is cleared, restore pre-chase params

```rust
pub fn follow_aircraft_3d(
    mut view3d_state: ResMut<crate::view3d::View3DState>,
    follow_state: Res<CameraFollowState>,
    aircraft_query: Query<&Aircraft>,
    time: Res<Time>,
    tile_settings: Res<bevy_slippy_tiles::SlippyTilesSettings>,
    map_state: Res<crate::MapState>,
) {
    use crate::view3d::{ViewMode, TransitionState};

    // Only follow in steady-state 3D (not during transitions)
    if !matches!(view3d_state.mode, ViewMode::Perspective3D)
        || !matches!(view3d_state.transition, TransitionState::Idle)
    {
        return;
    }

    let Some(ref following_icao) = follow_state.following_icao else {
        // Just stopped following — deactivate chase and restore orbit params
        if view3d_state.chase_active {
            view3d_state.camera_pitch = view3d_state.pre_chase_pitch;
            view3d_state.camera_yaw = view3d_state.pre_chase_yaw;
            view3d_state.camera_altitude = view3d_state.pre_chase_altitude;
            view3d_state.chase_active = false;
            view3d_state.chase_transition = 0.0;
        }
        view3d_state.follow_altitude_ft = None;
        return;
    };

    let Some(aircraft) = aircraft_query.iter().find(|a| a.icao == *following_icao) else {
        view3d_state.follow_altitude_ft = None;
        return;
    };

    // Activate chase on first frame of following
    if !view3d_state.chase_active {
        view3d_state.pre_chase_pitch = view3d_state.camera_pitch;
        view3d_state.pre_chase_yaw = view3d_state.camera_yaw;
        view3d_state.pre_chase_altitude = view3d_state.camera_altitude;
        view3d_state.chase_active = true;
        view3d_state.chase_transition = 0.0;
    }

    let converter = crate::geo::CoordinateConverter::new(&tile_settings, map_state.zoom_level);
    let target_pos = converter.latlon_to_world(aircraft.latitude, aircraft.longitude);

    // Lerp map center toward aircraft position
    let pos_lerp_speed = 3.0;
    let t_pos = (pos_lerp_speed * time.delta_secs()).min(1.0);
    view3d_state.saved_2d_center.x += (target_pos.x - view3d_state.saved_2d_center.x) * t_pos;
    view3d_state.saved_2d_center.y += (target_pos.y - view3d_state.saved_2d_center.y) * t_pos;

    // Track the followed aircraft's altitude for the orbit center
    view3d_state.follow_altitude_ft = aircraft.altitude;

    // Chase camera: lerp orbit params toward chase targets
    // Advance chase transition progress
    view3d_state.chase_transition = (view3d_state.chase_transition
        + time.delta_secs() / crate::view3d::CHASE_TRANSITION_DURATION)
        .min(1.0);

    let chase_lerp_speed = 2.0;
    let t_chase = (chase_lerp_speed * time.delta_secs()).min(1.0);

    // Target yaw: behind the aircraft (heading + 180)
    let target_yaw = if let Some(heading) = aircraft.heading {
        (heading + 180.0) % 360.0
    } else {
        view3d_state.camera_yaw
    };

    // Shortest-path yaw lerp (handle 0/360 wrap)
    let mut yaw_diff = target_yaw - view3d_state.camera_yaw;
    if yaw_diff > 180.0 { yaw_diff -= 360.0; }
    if yaw_diff < -180.0 { yaw_diff += 360.0; }
    view3d_state.camera_yaw += yaw_diff * t_chase;
    if view3d_state.camera_yaw < 0.0 { view3d_state.camera_yaw += 360.0; }
    if view3d_state.camera_yaw >= 360.0 { view3d_state.camera_yaw -= 360.0; }

    // Target pitch and altitude
    let target_pitch = crate::view3d::CHASE_PITCH;
    view3d_state.camera_pitch += (target_pitch - view3d_state.camera_pitch) * t_chase;

    let target_altitude = aircraft.altitude.unwrap_or(0) as f32
        + crate::view3d::CHASE_OFFSET_ABOVE_FT
        + crate::view3d::CHASE_OFFSET_BEHIND_FT;
    view3d_state.camera_altitude += (target_altitude - view3d_state.camera_altitude) * t_chase;
}
```

Note: `CHASE_OFFSET_BEHIND_FT` is added to altitude because the orbit camera computes horizontal distance from altitude and pitch. At a 5-degree pitch, the camera needs to be at an altitude where the orbit geometry places it approximately 100 ft behind horizontally. The orbit distance formula is `altitude / sin(pitch)`, so at 5 deg pitch with 125 ft altitude, the orbit distance is ~1434 ft and horizontal distance is ~1429 ft. For a true 100 ft behind, we set the total camera altitude to `aircraft_alt + CHASE_OFFSET_ABOVE_FT + CHASE_OFFSET_BEHIND_FT * tan(5 deg)` which is approximately `aircraft_alt + 25 + 8.7 ≈ aircraft_alt + 34`. But this puts the camera very close. Instead, since we want 100 ft behind at 5 deg down, the correct altitude above aircraft = `100 * tan(5 deg) + 25 ≈ 33.7`. Let's use `CHASE_OFFSET_ABOVE_FT` = 25 and compute the target altitude as `aircraft_alt + 25`. The 100 ft behind is handled by the low pitch angle naturally. We'll tune in Task 4 after visual testing.

Actually, let me simplify: the orbit camera places the camera at `altitude / sin(pitch)` orbit distance, with vertical component = altitude and horizontal component = `altitude / tan(pitch)`. So for a 5 deg pitch and the camera to be ~100ft horizontal behind, we need altitude ≈ `100 * tan(5 deg)` ≈ 8.7 ft above the orbit center. The orbit center is already at aircraft altitude. So total camera altitude = aircraft altitude + 8.7 ft. But that's only 8.7 ft above the aircraft — the user wants 25 ft above. With 25 ft above and 5 deg pitch: horizontal distance = `25 / tan(5 deg)` ≈ 286 ft behind. That's reasonable for a chase view, just farther back than 100 ft. For exactly 100 ft behind and 25 ft above, the pitch should be `atan(25/100)` ≈ 14 degrees, not 5.

The user specified all three values (100 ft behind, 25 ft above, 5 deg down). These are geometrically inconsistent for the orbit camera model. The simplest approach is to bypass the orbit calculation in chase mode and directly compute the camera position from the offset vector.

**Revised approach in Task 3.**

**Step 2: Make chase constants pub(crate)**

The constants need to be accessible from `picking.rs`. Change in `src/view3d/mod.rs`:

```rust
pub(crate) const CHASE_OFFSET_BEHIND_FT: f32 = 100.0;
pub(crate) const CHASE_OFFSET_ABOVE_FT: f32 = 25.0;
pub(crate) const CHASE_PITCH: f32 = 5.0;
pub(crate) const CHASE_TRANSITION_DURATION: f32 = 2.0;
```

**Step 3: Build and verify**

Run: `cargo build 2>&1 | grep "^error"`
Expected: No errors

**Step 4: Commit**

```
feat: add chase activation and heading tracking to follow_aircraft_3d
```

---

### Task 3: Override camera transform in chase mode

**Files:**
- Modify: `src/view3d/mod.rs:384-503` (update_3d_camera system)

Since 100 ft behind, 25 ft above, and 5 deg pitch are geometrically independent (not derivable from the orbit model), we need to directly compute the chase camera position when chase is active, bypassing the orbit calculation.

**Step 1: Add a chase camera transform method to `View3DState`**

Add after `calculate_camera_transform_yup` (after line 174):

```rust
    /// Calculate chase camera transform in Y-up space.
    /// Places camera at a fixed offset behind and above the orbit center,
    /// rotated by the chase yaw, with a fixed downward pitch.
    fn calculate_chase_transform_yup(&self, center: Vec3) -> Transform {
        let yaw_rad = self.camera_yaw.to_radians();

        let behind_dist = CHASE_OFFSET_BEHIND_FT * 0.3048 / 1000.0 * PIXEL_SCALE * self.altitude_scale;
        let above_dist = CHASE_OFFSET_ABOVE_FT * 0.3048 / 1000.0 * PIXEL_SCALE * self.altitude_scale;

        // Camera position: behind along yaw direction, above center
        // At yaw=0, camera is south (+Z in Y-up), looking north (-Z)
        let camera_pos = Vec3::new(
            center.x - behind_dist * yaw_rad.sin(),
            center.y + above_dist,
            center.z + behind_dist * yaw_rad.cos(),
        );

        // Look at a point slightly below center for the downward pitch
        let pitch_rad = CHASE_PITCH.to_radians();
        let look_ahead_dist = behind_dist * 2.0;
        let look_target = Vec3::new(
            center.x + look_ahead_dist * yaw_rad.sin(),
            center.y - look_ahead_dist * pitch_rad.tan(),
            center.z - look_ahead_dist * yaw_rad.cos(),
        );

        Transform::from_translation(camera_pos).looking_at(look_target, Vec3::Y)
    }
```

**Step 2: Use chase transform in `update_3d_camera`**

In `update_3d_camera`, replace the line that computes `orbit_yup` (line 435):

```rust
    let orbit_yup = state.calculate_camera_transform_yup(center_yup);
```

With:

```rust
    let orbit_yup = if state.chase_active {
        let t = smooth_step(state.chase_transition);
        let orbit = state.calculate_camera_transform_yup(center_yup);
        let chase = state.calculate_chase_transform_yup(center_yup);
        // Blend from orbit to chase during transition
        Transform {
            translation: orbit.translation.lerp(chase.translation, t),
            rotation: orbit.rotation.slerp(chase.rotation, t),
            scale: Vec3::ONE,
        }
    } else {
        state.calculate_camera_transform_yup(center_yup)
    };
```

**Step 3: Build and verify**

Run: `cargo build 2>&1 | grep "^error"`
Expected: No errors

**Step 4: Commit**

```
feat: add chase camera transform with smooth transition from orbit
```

---

### Task 4: Visual testing and tuning

**Step 1: Run the app**

Run: `cargo run`

**Step 2: Test chase camera**

1. Press `3` to enter 3D mode
2. Click an aircraft to follow it
3. Verify camera smoothly transitions to behind the aircraft
4. Verify camera tracks as aircraft turns
5. Press Escape — verify camera restores pre-chase orbit params
6. Click empty space — verify chase deactivates

**Step 3: Tune offsets if needed**

Adjust `CHASE_OFFSET_BEHIND_FT`, `CHASE_OFFSET_ABOVE_FT`, and `CHASE_PITCH` constants based on visual results. The pixel-space conversion may need scaling adjustments.

**Step 4: Commit any tuning changes**

```
fix: tune chase camera offset and pitch values
```
