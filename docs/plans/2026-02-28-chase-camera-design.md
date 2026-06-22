# Chase Camera Design

## Summary

Add a chase camera mode that activates when following an aircraft in 3D mode. The camera smoothly transitions to a position 100 ft behind, 25 ft above, with 5 degrees down pitch, aligned to the aircraft's heading. The camera continuously tracks heading changes. User can exit chase by pressing Escape or clicking empty space, which clears the follow state.

## Behavior

1. **Activation:** Setting `following_icao` in 3D mode triggers chase. The system lerps orbit parameters toward chase targets over ~2 seconds.
2. **Continuous heading tracking:** Target yaw = aircraft heading + 180 degrees. Camera lerps smoothly to follow turns.
3. **Scroll/orbit during chase:** Scroll (altitude) and shift+drag (orbit) work normally. Chase keeps overriding yaw to track heading; adjusted altitude/pitch persist.
4. **Exit chase:** Escape or clicking empty space clears `following_icao`, exiting chase mode. Orbit parameters restore to pre-chase values.

## New State in `View3DState`

```rust
pub chase_active: bool,
pub chase_transition: f32,       // 0.0 to 1.0 entry progress
pub pre_chase_pitch: f32,
pub pre_chase_yaw: f32,
pub pre_chase_altitude: f32,
```

## Constants

- `CHASE_OFFSET_BEHIND`: 100 ft
- `CHASE_OFFSET_ABOVE`: 25 ft
- `CHASE_PITCH`: 5.0 degrees
- `CHASE_TRANSITION_SPEED`: ~2.0 seconds

## System Changes

- `follow_aircraft_3d` (picking.rs): When chase active, continuously lerp yaw toward aircraft heading + 180, set camera altitude to aircraft alt + offset, set pitch to chase pitch.
- `update_3d_camera` (view3d/mod.rs): No changes needed — already reads pitch/yaw/altitude from View3DState.
- `handle_3d_camera_controls` (view3d/mod.rs): Escape already clears follow state.
- Chase activation logic: When `following_icao` transitions from None to Some in 3D mode, save current orbit params and set `chase_active = true`.

## Coordinate Details

100 ft behind and 25 ft above are converted to pixel-space via `altitude_to_z()`. The chase position is computed as: aircraft world position + offset vector rotated by aircraft heading, matching the existing orbit camera coordinate system.
