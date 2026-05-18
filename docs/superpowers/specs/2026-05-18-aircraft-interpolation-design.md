# Aircraft Position Interpolation - Design Spec

## Problem

Aircraft positions update only when ADS-B messages arrive (typically every 1-15 seconds). Between updates, aircraft freeze in place and then jump to the new position. This creates visible choppiness, especially in 3D mode where smooth motion is expected.

## Solution

Dead reckoning interpolation that advances aircraft positions every frame using last known heading, speed, and vertical rate. When real ADS-B data arrives, the displayed position corrects smoothly (small errors) or snaps (large errors).

## Design Decisions

- **Proportional correction:** Errors < 0.5 nm blend over ~0.3s. Errors >= 0.5 nm snap immediately.
- **Prediction horizon:** Up to 15 seconds. After 15s with no update, hold position (staleness fading handles visibility).
- **Full 3D:** Altitude interpolated using vertical_rate (fpm). Clamped to >= 0 ft.
- **Smooth heading:** Rotation interpolated with shortest-path wraparound (359->1 goes through 0).
- **Ground/no-velocity aircraft:** Prediction disabled. Holds last position, snaps on update.

## New Component: `InterpolationState`

Added to every aircraft entity alongside `Aircraft`.

```rust
#[derive(Component)]
pub struct InterpolationState {
    // Baseline: last confirmed ADS-B position
    pub base_lat: f64,
    pub base_lon: f64,
    pub base_altitude: Option<f32>,      // feet
    pub base_heading: Option<f32>,       // degrees 0-360
    pub base_speed: Option<f64>,         // knots
    pub base_vertical_rate: Option<f32>, // feet per minute
    pub base_time: f64,                  // seconds since startup (Time<Real>)

    // Blend target: set when correction is small enough to smooth
    pub blend_target: Option<BlendTarget>,

    // Display state: what the renderer uses (advanced each frame)
    pub display_lat: f64,
    pub display_lon: f64,
    pub display_altitude: Option<f32>,
    pub display_heading: Option<f32>,

    // Whether dead reckoning is active
    pub predicting: bool,
}

pub struct BlendTarget {
    // Old baseline to dead reckon from (the pre-correction track)
    pub old_base_lat: f64,
    pub old_base_lon: f64,
    pub old_base_altitude: Option<f32>,
    pub old_base_heading: Option<f32>,
    pub old_base_speed: Option<f64>,
    pub old_base_vertical_rate: Option<f32>,
    pub old_base_time: f64,
    // Blend timing
    pub blend_start_time: f64,
    pub blend_duration: f32, // seconds, typically 0.3-0.5
}
```

**Field roles:**
- `base_*`: Snapshot of last confirmed ADS-B truth. Reset on every real update.
- `display_*`: What `update_aircraft_positions` reads for Transform. Advanced by dead reckoning each frame.
- `blend_target`: Active during smooth correction. Stores the old (pre-correction) baseline so we can dead reckon from both old and new tracks simultaneously, lerping between them over blend_duration.
- `predicting`: False when heading/speed unavailable, speed <= 10 kts, on ground, or stale > 15s.

## Update Flow: New ADS-B Data Arrives

In `sync_aircraft_from_adsb`, after updating the `Aircraft` component:

1. Compute prediction error: haversine distance between `display_lat/lon` and new ADS-B lat/lon.
2. If error < 0.5 nm: Set `BlendTarget` with new position, blend_duration ~0.3s. Do NOT snap display.
3. If error >= 0.5 nm: Snap `display_*` directly to new ADS-B position. Clear any active blend.
4. Reset `base_*` fields to new ADS-B values. Reset `base_time` to current time.
5. Set `predicting = true` if heading and speed available and speed > 10 kts and not on ground.

## Frame-by-Frame Interpolation System

New system: `interpolate_aircraft_positions`

Runs every frame, after `sync_aircraft_from_adsb`, before `update_aircraft_positions`.

For each aircraft with `InterpolationState`:

1. Compute `elapsed = current_time - base_time` (seconds).
2. If `elapsed > 15.0`: set `predicting = false`, hold display at current values. Return.
3. If `predicting`:
   - Dead reckon lat/lon: `predict_position(base_lat, base_lon, base_heading, base_speed, elapsed_minutes)`.
   - Dead reckon altitude: `base_altitude + (base_vertical_rate * elapsed / 60.0)`, clamped >= 0.
   - Heading: hold at `base_heading` (no turn rate data in ADS-B).
   - Write to `display_lat/lon/altitude/heading`.
4. If `blend_target` is active:
   - Compute blend progress: `t = (now - blend_start_time) / blend_duration`, clamped 0..1.
   - Compute two positions: (a) dead reckoning from the *old* baseline (pre-correction), and (b) dead reckoning from the *new* baseline (the corrected ADS-B position, stored in `base_*` which was already reset in step 4 of the update flow). Lerp `display_lat/lon` between (a) and (b) using `t`. This way both positions advance forward in time - the blend smoothly migrates from the old track to the corrected track without fighting forward motion.
   - Lerp `display_altitude` between old and new dead-reckoned altitudes.
   - Interpolate `display_heading` via shortest angular path (normalize diff to [-180, +180]).
   - When `t >= 1.0`: clear blend_target, dead reckoning continues naturally from `base_*` (already set to corrected position).

## System Ordering

```
sync_aircraft_from_adsb
    |
interpolate_aircraft_positions    (NEW)
    |
update_aircraft_positions         (MODIFIED: reads InterpolationState.display_*)
    |
update_aircraft_labels            (unchanged)
```

## Integration Points

### Modified: `update_aircraft_positions` (camera.rs)

Change position and heading reads from `Aircraft` to `InterpolationState`:
- `transform.translation.x/y` from `converter.latlon_to_world(interp.display_lat, interp.display_lon)`
- Heading rotation from `interp.display_heading`
- Query changes to `Query<(&Aircraft, &InterpolationState, &mut Transform)>`

### Modified: `sync_aircraft_from_adsb` (adsb/sync.rs)

- On aircraft spawn: insert `InterpolationState` initialized from first ADS-B position.
- On aircraft update: apply snap/blend logic, reset baseline.

### Unchanged systems (continue reading `Aircraft` component):

- `record_trail_points` - Trails reflect real ADS-B positions, not predictions.
- `draw_predictions` - Prediction lines use real ADS-B data.
- `dim_stale_aircraft` - Uses `Aircraft.last_seen` timestamp.
- `update_aircraft_label_text` - Reads callsign/altitude from `Aircraft`.
- Model loading, picking, emergency alerts - all read `Aircraft`.

## New File

`src/aircraft/interpolation.rs`:
- `InterpolationState` component
- `BlendTarget` struct
- `interpolate_aircraft_positions` system
- Helper: `shortest_angle_diff(from: f32, to: f32) -> f32`
- Helper: `lerp_heading(from: f32, to: f32, t: f32) -> f32`

Registered in `AircraftPlugin` with system ordering constraints.

## Edge Cases

| Case | Behavior |
|------|----------|
| No heading or speed | `predicting = false`, hold position, snap on update |
| Speed <= 10 knots | `predicting = false` (likely on ground or stationary) |
| `is_on_ground == true` | `predicting = false` |
| First position after spawn | Initialize display to first position, no blend needed |
| Heading wraparound (350->10) | Shortest path via normalize to [-180, +180] |
| Negative altitude prediction | Clamp to 0 feet |
| Stale > 15 seconds | Stop predicting, hold position, staleness fading applies |
| Aircraft despawned | Component removed with entity, no cleanup needed |

## Configuration

Add to `AppConfig` (persisted in TOML):
- `interpolation_enabled: bool` (default: `true`)

When disabled, `interpolate_aircraft_positions` skips processing and `update_aircraft_positions` falls back to reading from `Aircraft` directly (current behavior).

## Constants

```rust
const BLEND_THRESHOLD_NM: f64 = 0.5;       // snap vs blend decision
const BLEND_DURATION_SECS: f32 = 0.3;       // smooth correction duration
const MAX_PREDICTION_SECS: f64 = 15.0;      // stop predicting after this
const MIN_PREDICTION_SPEED_KTS: f64 = 10.0; // below this, don't predict
```

## Performance

- Per-frame cost: one `predict_position()` (few trig ops) + optional lerp per aircraft.
- At 200 aircraft, 60 FPS: ~12,000 trig calls/second. Negligible.
- Memory: ~120 bytes per aircraft for `InterpolationState`. ~24 KB at 200 aircraft.
- No allocations in the hot path. No new network I/O.
- No impact on tile rendering, atmosphere, or ADS-B sync cost.
