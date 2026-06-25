//! Heightmap tile fetching, decoding, and caching.

use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::provider::TerrainProvider;

const MAX_CONCURRENT_FETCHES: usize = 8;

/// Key for a terrain tile: (zoom, x, y)
pub(crate) type TileKey = (u8, u32, u32);

/// Decoded heightmap data for a single terrain tile.
/// Contains a 256x256 grid of elevation values in meters,
/// plus the raw RGB pixel data for GPU texture upload.
pub(crate) struct HeightmapData {
    width: u32,
    height: u32,
    elevations: Vec<f32>,
    /// Raw RGB pixel data (3 bytes per pixel) for GPU texture creation.
    pub raw_rgb: Vec<u8>,
    pub min_elevation: f32,
    pub max_elevation: f32,
}

impl HeightmapData {
    /// Width of the heightmap grid in pixels.
    pub(crate) fn width(&self) -> usize {
        self.width as usize
    }

    /// Height of the heightmap grid in pixels.
    pub(crate) fn height(&self) -> usize {
        self.height as usize
    }

    /// Create a Bevy `Image` from the raw RGB data for GPU texture upload.
    /// The shader decodes the Terrarium format from the RGB channels.
    pub(crate) fn to_gpu_image(&self) -> bevy::prelude::Image {
        use bevy::asset::RenderAssetUsages;
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

        // Convert RGB → RGBA (GPU textures require 4 channels)
        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);
        for chunk in self.raw_rgb.chunks(3) {
            rgba.push(chunk[0]);
            rgba.push(chunk[1]);
            rgba.push(chunk[2]);
            rgba.push(255);
        }

        Image::new(
            Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            rgba,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        )
    }

    /// Sample the elevation at pixel coordinates (x, y) in meters.
    /// Returns 0.0 for out-of-bounds coordinates.
    pub(crate) fn elevation(&self, x: usize, y: usize) -> f32 {
        if x < self.width as usize && y < self.height as usize {
            self.elevations[y * self.width as usize + x]
        } else {
            0.0
        }
    }
}

/// Resource that caches decoded heightmaps and manages async fetch requests.
#[derive(Resource)]
pub(crate) struct HeightmapCache {
    cache: HashMap<TileKey, HeightmapData>,
    pending: HashSet<TileKey>,
    sender: Sender<(TileKey, HeightmapData)>,
    receiver: Receiver<(TileKey, HeightmapData)>,
    pub provider: TerrainProvider,
    active_fetches: Arc<AtomicUsize>,
}

impl HeightmapCache {
    /// Create a new heightmap cache with the given elevation data provider.
    pub(crate) fn new(provider: TerrainProvider) -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        Self {
            cache: HashMap::new(),
            pending: HashSet::new(),
            sender,
            receiver,
            provider,
            active_fetches: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns a reference to the cached heightmap for the given tile key, if present.
    pub(crate) fn get(&self, key: &TileKey) -> Option<&HeightmapData> {
        self.cache.get(key)
    }

    /// Returns true if the tile is already cached or has a pending fetch request.
    pub(crate) fn contains(&self, key: &TileKey) -> bool {
        self.cache.contains_key(key) || self.pending.contains(key)
    }

    /// Spawn an async fetch for the given tile if it is not already cached or pending.
    /// Skips if the concurrency limit has been reached; the tile will be re-requested next frame.
    pub(crate) fn request(&mut self, key: TileKey) {
        if self.cache.contains_key(&key) || self.pending.contains(&key) {
            return;
        }
        if self.active_fetches.load(Ordering::Relaxed) >= MAX_CONCURRENT_FETCHES {
            return;
        }
        self.pending.insert(key);
        fetch_and_decode(
            self.provider.clone(),
            key,
            self.sender.clone(),
            self.active_fetches.clone(),
        );
    }

    /// Drain the receiver channel, inserting all completed heightmaps into the cache.
    pub(crate) fn poll_completed(&mut self) {
        while let Ok((key, data)) = self.receiver.try_recv() {
            self.pending.remove(&key);
            self.cache.insert(key, data);
        }
    }

    /// Remove a cached heightmap (for tile eviction).
    pub(crate) fn remove(&mut self, key: &TileKey) {
        self.cache.remove(key);
        self.pending.remove(key);
    }

    /// Evict heightmaps outside the active zoom band and enforce a size cap.
    /// Returns the number of entries removed.
    pub(crate) fn evict(&mut self, current_zoom: u8, max_entries: usize) -> usize {
        let mut removed = 0;

        // Phase 1: Remove entries outside the active zoom band [current-4, current]
        let min_zoom = current_zoom.saturating_sub(4);
        self.cache.retain(|&(zoom, _, _), _| {
            let keep = zoom >= min_zoom && zoom <= current_zoom;
            if !keep {
                removed += 1;
            }
            keep
        });
        self.pending
            .retain(|&(zoom, _, _)| zoom >= min_zoom && zoom <= current_zoom);

        // Phase 2: If still over budget, drop oldest (arbitrary) entries
        if self.cache.len() > max_entries {
            let excess = self.cache.len() - max_entries;
            let keys_to_remove: Vec<TileKey> = self.cache.keys().take(excess).copied().collect();
            for key in keys_to_remove {
                self.cache.remove(&key);
                removed += 1;
            }
        }

        removed
    }

    /// Number of cached heightmaps.
    pub(crate) fn len(&self) -> usize {
        self.cache.len()
    }

    /// Sample elevation in meters at a given lat/lon and zoom level.
    /// Returns `None` if the heightmap tile isn't cached yet.
    pub(crate) fn sample_elevation(
        &self,
        lat: f64,
        lon: f64,
        zoom_level: bevy_slippy_tiles::ZoomLevel,
    ) -> Option<f32> {
        let zoom = zoom_level.to_u8();
        let tile_coords =
            bevy_slippy_tiles::SlippyTileCoordinates::from_latitude_longitude(lat, lon, zoom_level);
        let key: TileKey = (zoom, tile_coords.x, tile_coords.y);
        let heightmap = self.cache.get(&key)?;

        // Compute fractional position within the tile (0.0 to 1.0)
        let n = 2.0_f64.powi(zoom as i32);
        let lat_rad = lat.to_radians();

        // Tile boundaries in lat/lon
        let tile_lon_min = (tile_coords.x as f64 / n) * 360.0 - 180.0;
        let tile_lon_max = ((tile_coords.x + 1) as f64 / n) * 360.0 - 180.0;

        let tile_lat_max = (std::f64::consts::PI * (1.0 - 2.0 * tile_coords.y as f64 / n))
            .sinh()
            .atan()
            .to_degrees();
        let tile_lat_min = (std::f64::consts::PI * (1.0 - 2.0 * (tile_coords.y + 1) as f64 / n))
            .sinh()
            .atan()
            .to_degrees();

        // UV within tile: u = east fraction, v = south fraction
        let u = ((lon - tile_lon_min) / (tile_lon_max - tile_lon_min)).clamp(0.0, 1.0) as f32;
        let v = ((tile_lat_max - lat) / (tile_lat_max - tile_lat_min)).clamp(0.0, 1.0) as f32;

        // Bilinear sample
        let w = heightmap.width() as f32;
        let h = heightmap.height() as f32;
        let px = (u * (w - 1.0)).clamp(0.0, w - 1.0);
        let py = (v * (h - 1.0)).clamp(0.0, h - 1.0);

        let x0 = px.floor() as usize;
        let y0 = py.floor() as usize;
        let x1 = (x0 + 1).min(heightmap.width() - 1);
        let y1 = (y0 + 1).min(heightmap.height() - 1);

        let fx = px - px.floor();
        let fy = py - py.floor();

        let e00 = heightmap.elevation(x0, y0);
        let e10 = heightmap.elevation(x1, y0);
        let e01 = heightmap.elevation(x0, y1);
        let e11 = heightmap.elevation(x1, y1);

        let top = e00 * (1.0 - fx) + e10 * fx;
        let bottom = e01 * (1.0 - fx) + e11 * fx;
        Some(top * (1.0 - fy) + bottom * fy)
    }
}

/// Disk cache path for a terrain tile: `{terrain_cache_dir}/{zoom}.{x}.{y}.png`
fn disk_cache_path(zoom: u8, x: u32, y: u32) -> std::path::PathBuf {
    crate::tile_cache::terrain_cache_dir().join(format!("{zoom}.{x}.{y}.png"))
}

/// Spawn a background thread to fetch an elevation tile PNG, decode it, and
/// send the resulting heightmap data back through the channel.
/// Checks disk cache before making an HTTP request, and persists downloaded
/// tiles to disk for future runs.
fn fetch_and_decode(
    provider: TerrainProvider,
    key: TileKey,
    sender: Sender<(TileKey, HeightmapData)>,
    active_fetches: Arc<AtomicUsize>,
) {
    let (zoom, x, y) = key;
    active_fetches.fetch_add(1, Ordering::Relaxed);

    std::thread::spawn(move || {
        let _guard = FetchGuard(active_fetches);

        let cache_path = disk_cache_path(zoom, x, y);
        let bytes = if cache_path.exists() {
            match std::fs::read(&cache_path) {
                Ok(b) => b.into(),
                Err(_) => {
                    let _ = std::fs::remove_file(&cache_path);
                    return;
                }
            }
        } else {
            let url = provider.tile_url(zoom, x, y);
            let downloaded = match reqwest::blocking::get(&url).and_then(|r| r.bytes()) {
                Ok(b) => b,
                Err(_) => return,
            };
            let _ = std::fs::write(&cache_path, &downloaded);
            downloaded
        };

        let img = match image::load_from_memory(&bytes) {
            Ok(i) => i.to_rgb8(),
            Err(_) => {
                let _ = std::fs::remove_file(&cache_path);
                return;
            }
        };
        let (w, h) = (img.width(), img.height());
        let mut elevations = Vec::with_capacity((w * h) as usize);
        let mut min_elev = f32::MAX;
        let mut max_elev = f32::MIN;
        for pixel in img.pixels() {
            let elev = provider.decode_elevation(pixel[0], pixel[1], pixel[2]);
            min_elev = min_elev.min(elev);
            max_elev = max_elev.max(elev);
            elevations.push(elev);
        }
        let raw_rgb = img.as_raw().clone();
        let _ = sender.send((
            key,
            HeightmapData {
                width: w,
                height: h,
                elevations,
                raw_rgb,
                min_elevation: min_elev,
                max_elevation: max_elev,
            },
        ));
    });
}

/// RAII guard that decrements the active fetch counter on drop,
/// ensuring correct accounting even on early returns.
struct FetchGuard(Arc<AtomicUsize>);

impl Drop for FetchGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Bevy systems
// ---------------------------------------------------------------------------

/// System that polls the heightmap completion channel each frame, inserting
/// finished heightmaps into the cache.
pub(crate) fn poll_heightmap_completions(mut cache: ResMut<HeightmapCache>) {
    cache.poll_completed();
}

/// System that requests heightmap fetches for visible map tiles in 3D mode.
///
/// Runs after tile spawning. Iterates over spawned tile entities that have both
/// a `MapTile` marker and a `TileFadeState`. For each tile at the current zoom
/// level, computes slippy tile coordinates from the tile's world-space transform
/// position and requests the corresponding heightmap from the cache.
pub(crate) fn request_heightmaps_for_tiles(
    view_state: Res<crate::view3d::View3DState>,
    terrain_state: Res<super::TerrainState>,
    map_state: Res<crate::MapState>,
    mut cache: ResMut<HeightmapCache>,
    tile_settings: Res<bevy_slippy_tiles::SlippyTilesSettings>,
    tile_query: Query<(&Transform, &crate::tiles::TileFadeState), With<bevy_slippy_tiles::MapTile>>,
) {
    // Only fetch heightmaps when 3D mode is active and terrain is enabled
    if !view_state.is_3d_active() || !terrain_state.enabled {
        return;
    }

    let current_zoom = map_state.zoom_level.to_u8();

    for (transform, fade_state) in tile_query.iter() {
        if fade_state.tile_zoom != current_zoom {
            continue;
        }

        let key = super::tile_key_from_transform(
            transform,
            fade_state,
            &tile_settings,
            map_state.zoom_level,
        );

        if !cache.contains(&key) {
            cache.request(key);
        }
    }
}
