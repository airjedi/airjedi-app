# Fix Tile Resolution Flashing in 3D Mode

**Date:** 2026-02-26
**Issue:** #10

## Problem

Map tiles flash between different resolutions in 3D perspective mode during pan, zoom, orbit, and sometimes when idle. The flashing manifests as visible snapping between blurry (low-zoom) and sharp (high-zoom) tile versions.

## Root Cause

Multiple zoom-level tiles occupy the same screen area simultaneously. Their 3D mesh quads use `AlphaMode::Opaque` at nearly identical Y elevations (separated by only 0.05 units per zoom level). GPU depth testing causes Z-fighting, and the winning tile switches unpredictably frame-to-frame.

Previous fixes (hysteresis zoom, disabled zoom-based despawning, altitude change tracking) addressed symptoms but not the fundamental problem: overlapping opaque geometry at competing zoom levels.

## Solution: Single Zoom Level in 3D

Stop rendering multiple zoom levels simultaneously. Use the altitude-adaptive zoom level everywhere. Lower-zoom tiles serve as temporary fallback only until current-zoom tiles load, then get despawned (same pattern 2D mode uses).

### Changes

1. **`request_3d_tiles_continuous()`** -- Replace multi-resolution band requests (31 requests across 5 zoom levels) with requests at the single current zoom level. Use larger radius and directional offsets based on camera yaw/pitch to fill the 3D perspective footprint.

2. **`display_tiles_filtered()`** -- In 3D mode, only accept tiles at the exact current zoom level. Remove multi-resolution acceptance window and position/scale rescaling for lower-zoom tiles.

3. **`animate_tile_fades()`** -- Re-enable zoom-based "dominated" detection in 3D mode. Old-zoom tiles despawn once covered by new tiles at the current zoom level.

4. **`cull_offscreen_tiles()`** -- Simplify entity budget since we no longer juggle multiple zoom levels.

### Unchanged

- Altitude-adaptive zoom with hysteresis
- Tile fade-in animation
- 3D mesh quad system (sync, alpha, transforms)
- 2D mode (completely untouched)
- Tile caching

### Risk

At low pitch angles (looking toward horizon), distant tiles at the current zoom level may not extend far enough. Mitigated by generous forward-biased request radius scaled by pitch.
