# Fix Tile Resolution Flashing in 3D Mode - Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Eliminate tile resolution flashing in 3D mode by using a single zoom level instead of overlapping multi-resolution bands.

**Architecture:** Replace the multi-resolution band tile request system with single-zoom-level requests using directional coverage. Re-enable zoom-based tile replacement so old-zoom tiles cross-fade out when current-zoom tiles load. Simplify culling since we no longer manage competing zoom levels.

**Tech Stack:** Rust, Bevy 0.18, bevy_slippy_tiles

---

### Task 1: Simplify 3D tile requests to single zoom level

**Files:**
- Modify: `src/tiles.rs:298-417` (request_3d_tiles_continuous and its doc comment)

**Step 1: Replace multi-resolution band requests with single-zoom directional requests**

Replace the entire `request_3d_tiles_continuous` function body (lines 310-417) and its doc comment (lines 298-309) with:

```rust
/// Continuously request tiles in 3D mode at the current adaptive zoom level.
/// Requests are offset in the camera look direction so tiles load ahead of
/// where the user is looking. The radius and forward bias adapt to pitch:
/// low pitch (looking toward horizon) requests more tiles forward;
/// high pitch (looking down) requests tiles centered around the map position.
fn request_3d_tiles_continuous(
    mut timer: ResMut<Tile3DRefreshTimer>,
    time: Res<Time>,
    view3d_state: Res<view3d::View3DState>,
    mut map_state: ResMut<MapState>,
    mut download_events: MessageWriter<DownloadSlippyTilesMessage>,
) {
    if !view3d_state.is_3d_active() {
        return;
    }

    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }

    // Compute zoom level from camera altitude
    let adaptive_zoom = altitude_to_zoom_level(view3d_state.camera_altitude, map_state.zoom_level.to_u8());
    if let Ok(new_zoom) = ZoomLevel::try_from(adaptive_zoom) {
        if map_state.zoom_level != new_zoom {
            debug!("3D adaptive zoom: altitude {:.0} ft -> zoom {}", view3d_state.camera_altitude, adaptive_zoom);
            map_state.zoom_level = new_zoom;
        }
    }

    let lat = map_state.latitude;
    let lon = map_state.longitude;
    let yaw_rad = view3d_state.camera_yaw.to_radians();
    let pitch = view3d_state.camera_pitch;
    let zoom = map_state.zoom_level;
    let z = zoom.to_u8();

    // pitch_factor: 0.0 = low pitch (horizon), 1.0 = high pitch (looking down)
    let pitch_factor = ((pitch - 15.0) / (89.0 - 15.0)).clamp(0.0, 1.0);

    // Base radius adapts to pitch: looking down needs a tight cluster,
    // looking at the horizon needs wider spread
    let base_radius = 4 + (4.0 * pitch_factor) as u8; // 4-8

    // Center request
    download_events.write(DownloadSlippyTilesMessage {
        tile_size: constants::DEFAULT_TILE_SIZE,
        zoom_level: zoom,
        coordinates: Coordinates::from_latitude_longitude(lat, lon),
        radius: Radius(base_radius),
        use_cache: true,
    });

    // Forward-biased requests to fill the perspective view ahead of camera.
    // More forward bias at low pitch (looking at horizon).
    let deg_per_tile_lon = 360.0 / (1u64 << z) as f64;
    let deg_per_tile_lat = deg_per_tile_lon * lat.to_radians().cos();
    let forward_distances = if pitch_factor < 0.5 {
        vec![4.0, 8.0, 12.0] // looking toward horizon: reach further
    } else {
        vec![3.0, 6.0]       // looking down: modest forward bias
    };
    let fwd_radius = 3 + (3.0 * (1.0 - pitch_factor)) as u8; // 3-6

    for fwd in forward_distances {
        let offset_lat = fwd * deg_per_tile_lat * yaw_rad.cos() as f64;
        let offset_lon = fwd * deg_per_tile_lon * yaw_rad.sin() as f64;
        download_events.write(DownloadSlippyTilesMessage {
            tile_size: constants::DEFAULT_TILE_SIZE,
            zoom_level: zoom,
            coordinates: Coordinates::from_latitude_longitude(
                clamp_latitude(lat + offset_lat),
                clamp_longitude(lon + offset_lon),
            ),
            radius: Radius(fwd_radius),
            use_cache: true,
        });

        // Side sweeps at each forward distance for wider coverage
        let spread = fwd * 0.7 + 2.0;
        for &side in &[-spread, spread] {
            let slat = fwd * deg_per_tile_lat * yaw_rad.cos() as f64
                - side * deg_per_tile_lat * yaw_rad.sin() as f64;
            let slon = fwd * deg_per_tile_lon * yaw_rad.sin() as f64
                + side * deg_per_tile_lon * yaw_rad.cos() as f64;
            download_events.write(DownloadSlippyTilesMessage {
                tile_size: constants::DEFAULT_TILE_SIZE,
                zoom_level: zoom,
                coordinates: Coordinates::from_latitude_longitude(
                    clamp_latitude(lat + slat),
                    clamp_longitude(lon + slon),
                ),
                radius: Radius(fwd_radius.saturating_sub(1).max(2)),
                use_cache: true,
            });
        }
    }
}
```

**Step 2: Build and verify no compile errors**

Run: `cargo build 2>&1 | grep -E "^error"`
Expected: No errors (warnings are OK)

**Step 3: Commit**

```
git add src/tiles.rs
git commit -m "Replace multi-resolution tile bands with single-zoom requests in 3D"
```

---

### Task 2: Filter 3D tiles to current zoom level only

**Files:**
- Modify: `src/tiles.rs:449-463` (display_tiles_filtered zoom filter)
- Modify: `src/tiles.rs:492-503` (remove multi-resolution rescaling)
- Modify: `src/tiles.rs:519-522` (remove zoom_diff Z offset)

**Step 1: Simplify the zoom filter to accept only current zoom in both modes**

Replace lines 452-462 (the zoom filter block inside the `for event` loop):

```rust
        // Only accept tiles at the exact current zoom level.
        if event.zoom_level != map_state.zoom_level {
            continue;
        }
```

**Step 2: Remove multi-resolution rescaling**

Replace lines 492-503 (the `zoom_diff` / `rescale` block):

```rust
        let rescale = 1.0;
```

**Step 3: Remove zoom_diff Z offset for 3D tiles**

Replace lines 519-525 (the `tile_z` calculation):

```rust
        let tile_z = if view3d_state.is_3d_active() {
            view3d_state.altitude_to_z(view3d_state.ground_elevation_ft)
        } else {
            tile_settings.z_layer + 0.1
        };
```

**Step 4: Build and verify**

Run: `cargo build 2>&1 | grep -E "^error"`
Expected: No errors. There will likely be an unused variable warning for `zoom_diff` — that's fine, we'll clean it up.

**Step 5: Clean up unused `zoom_diff` variable**

Remove the line that computes `zoom_diff` (line 495):
```rust
        let zoom_diff = current_zoom.saturating_sub(event_zoom) as u32;
```

And also remove the now-unused `event_zoom` variable (line 455):
```rust
        let event_zoom = event.zoom_level.to_u8();
```

**Step 6: Build and verify**

Run: `cargo build 2>&1 | grep -E "^error"`
Expected: No errors

**Step 7: Commit**

```
git add src/tiles.rs
git commit -m "Filter 3D tiles to current zoom level only, remove multi-resolution rescaling"
```

---

### Task 3: Re-enable zoom-based tile replacement in 3D mode

**Files:**
- Modify: `src/tiles.rs:699-707` (animate_tile_fades dominated logic)

**Step 1: Remove the 3D special case that prevented old-tile detection**

Replace lines 700-707 (the `dominated` calculation):

```rust
        let dominated = fade_state.tile_zoom != current_zoom;
```

This makes 3D mode use the same cross-fade-and-replace logic as 2D mode: old-zoom tiles are despawned once current-zoom tiles fully cover their grid cell.

**Step 2: Remove the now-unused `is_3d` variable**

Remove line 692:
```rust
    let is_3d = view3d_state.is_3d_active();
```

And remove the `view3d_state` parameter from the function signature (line 685):
```rust
    view3d_state: Res<view3d::View3DState>,
```

And the import at the top of the function will be cleaned up by the compiler.

**Step 3: Build and verify**

Run: `cargo build 2>&1 | grep -E "^error"`
Expected: No errors

**Step 4: Commit**

```
git add src/tiles.rs
git commit -m "Re-enable zoom-based tile replacement in 3D mode"
```

---

### Task 4: Simplify tile culling

**Files:**
- Modify: `src/tiles.rs:551-559` (max_tile_entities)
- Modify: `src/tiles.rs:599-603` (culling margin)
- Modify: `src/tiles.rs:656-663` (entity budget boost)

**Step 1: Simplify max_tile_entities**

Since we no longer have multi-resolution tiles competing, reduce the 3D budget. Replace the function:

```rust
fn max_tile_entities(view3d_state: Option<&view3d::View3DState>) -> usize {
    if let Some(state) = view3d_state {
        if state.is_3d_active() {
            return 500; // Single zoom level needs fewer tiles than multi-resolution
        }
    }
    400
}
```

**Step 2: Simplify the culling margin**

Replace lines 599-603 (the margin calculation inside the 3D branch of cull_offscreen_tiles):

```rust
        let margin = 2.5;
```

This removes the altitude-change-aware 4.0x margin since we no longer need to protect old-zoom tiles during transitions.

**Step 3: Simplify entity budget**

Replace lines 656-663 (the tile_limit calculation):

```rust
    let tile_limit = max_tile_entities(Some(&view3d_state));
```

This removes the +200 boost during altitude changes.

**Step 4: Consider removing AltitudeChangeTracker**

The `AltitudeChangeTracker` resource, `track_altitude_changes` system, and `alt_tracker` parameter in `cull_offscreen_tiles` are no longer needed. Remove:
- The `AltitudeChangeTracker` struct and its `Default` impl (lines 58-72)
- The `track_altitude_changes` system (lines 419-434)
- The `.init_resource::<AltitudeChangeTracker>()` line in the plugin (line 84)
- The `.add_systems(Update, track_altitude_changes)` line (line 89)
- The `alt_tracker: Res<AltitudeChangeTracker>` parameter from `cull_offscreen_tiles` (line 571)

**Step 5: Build and verify**

Run: `cargo build 2>&1 | grep -E "^error"`
Expected: No errors

**Step 6: Commit**

```
git add src/tiles.rs
git commit -m "Simplify tile culling for single-zoom-level 3D mode"
```

---

### Task 5: Update handle_3d_view_tile_refresh comments

**Files:**
- Modify: `src/tiles.rs:264-268` (comment about multi-resolution tiles)

**Step 1: Update the comment to reflect single-zoom behavior**

Replace lines 264-268:

```rust
    // When returning to 2D mode, clear the spawned tiles tracker
    // and restore the saved 2D zoom level.
    // 3D mode uses altitude-adaptive zoom that may differ from the 2D level;
    // without clearing, the dedup check in display_tiles_filtered would skip
    // re-spawning tiles at the restored zoom level, leaving a blank map.
```

**Step 2: Commit**

```
git add src/tiles.rs
git commit -m "Update tile refresh comment to reflect single-zoom design"
```

---

### Task 6: Manual verification

**Step 1: Build release and run**

Run: `cargo run --release`

**Step 2: Verify 3D mode tiles**

1. Press V to enter 3D mode
2. Pan in all directions — tiles should load without resolution flashing
3. Orbit the camera — no flashing
4. Zoom in and out (scroll wheel) — tiles should cross-fade cleanly between zoom levels
5. Look toward the horizon at low pitch — verify tiles extend far enough
6. Look straight down — verify tight tile coverage
7. Press V to return to 2D — map should display correctly
8. Pan and zoom in 2D — verify no regression

**Step 3: Edge cases to test**

- Rapid altitude changes (scroll wheel quickly)
- Orbit while zooming
- Stationary camera at various altitudes (check no idle flashing)
- Switch between 2D and 3D multiple times
