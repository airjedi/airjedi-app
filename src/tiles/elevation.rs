use bevy::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

use super::types::TileFormat;

// ---------------------------------------------------------------------------
// Elevation decoder trait
// ---------------------------------------------------------------------------

pub trait ElevationDecoder: Send + Sync + 'static {
    fn decode_elevation(&self, r: u8, g: u8, b: u8) -> f32;
    fn tile_url(&self, x: u32, y: u32, zoom: u8) -> String;
    fn tile_format(&self) -> TileFormat;
    fn name(&self) -> &'static str;
}

/// Mapbox Terrain-RGB v1:
/// height = -10000 + ((R * 256 * 256 + G * 256 + B) * 0.1)
pub struct MapboxTerrainRgb {
    pub access_token: String,
}

impl ElevationDecoder for MapboxTerrainRgb {
    fn decode_elevation(&self, r: u8, g: u8, b: u8) -> f32 {
        -10000.0 + ((r as f32) * 256.0 * 256.0 + (g as f32) * 256.0 + (b as f32)) * 0.1
    }

    fn tile_url(&self, x: u32, y: u32, zoom: u8) -> String {
        format!(
            "https://api.mapbox.com/v4/mapbox.terrain-rgb/{}/{}/{}.pngraw?access_token={}",
            zoom, x, y, self.access_token
        )
    }

    fn tile_format(&self) -> TileFormat {
        TileFormat::Png
    }

    fn name(&self) -> &'static str {
        "mapbox-terrain-rgb"
    }
}

/// Terrarium (AWS/Mapzen):
/// height = (R * 256 + G + B / 256) - 32768
pub struct TerrariumDecoder {
    pub base_url: String,
}

impl Default for TerrariumDecoder {
    fn default() -> Self {
        Self {
            base_url: "https://s3.amazonaws.com/elevation-tiles-prod/terrarium".into(),
        }
    }
}

impl ElevationDecoder for TerrariumDecoder {
    fn decode_elevation(&self, r: u8, g: u8, b: u8) -> f32 {
        (r as f32) * 256.0 + (g as f32) + (b as f32) / 256.0 - 32768.0
    }

    fn tile_url(&self, x: u32, y: u32, zoom: u8) -> String {
        format!("{}/{}/{}/{}.png", self.base_url, zoom, x, y)
    }

    fn tile_format(&self) -> TileFormat {
        TileFormat::Png
    }

    fn name(&self) -> &'static str {
        "terrarium"
    }
}

// ---------------------------------------------------------------------------
// Heightmap data
// ---------------------------------------------------------------------------

/// Decoded heightmap for a single tile. Grid of elevation values in meters.
#[derive(Clone)]
pub struct Heightmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

impl Heightmap {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0.0; (width * height) as usize],
        }
    }

    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let x = ((u * self.width as f32) as u32).min(self.width - 1);
        let y = ((v * self.height as f32) as u32).min(self.height - 1);
        self.data[(y * self.width + x) as usize]
    }

    pub fn min_max(&self) -> (f32, f32) {
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for &v in &self.data {
            if v < min { min = v; }
            if v > max { max = v; }
        }
        (min, max)
    }

    pub fn decode_from_rgba(
        rgba: &[u8],
        width: u32,
        height: u32,
        decoder: &dyn ElevationDecoder,
    ) -> Self {
        let mut hm = Self::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width + x) * 4) as usize;
                if idx + 2 < rgba.len() {
                    hm.data[(y * width + x) as usize] =
                        decoder.decode_elevation(rgba[idx], rgba[idx + 1], rgba[idx + 2]);
                }
            }
        }
        hm
    }
}

// ---------------------------------------------------------------------------
// Elevation tile key and cache
// ---------------------------------------------------------------------------

#[derive(Eq, PartialEq, Hash, Clone, Debug)]
pub struct ElevationTileKey {
    pub x: u32,
    pub y: u32,
    pub zoom: u8,
}

/// Cached heightmaps, keyed by tile coordinates.
#[derive(Resource, Default)]
pub struct ElevationCache {
    pub tiles: HashMap<ElevationTileKey, Heightmap>,
}

impl ElevationCache {
    pub fn get(&self, x: u32, y: u32, zoom: u8) -> Option<&Heightmap> {
        self.tiles.get(&ElevationTileKey { x, y, zoom })
    }

    pub fn insert(&mut self, x: u32, y: u32, zoom: u8, heightmap: Heightmap) {
        self.tiles.insert(ElevationTileKey { x, y, zoom }, heightmap);
    }

    pub fn contains(&self, x: u32, y: u32, zoom: u8) -> bool {
        self.tiles.contains_key(&ElevationTileKey { x, y, zoom })
    }
}

// ---------------------------------------------------------------------------
// Elevation settings
// ---------------------------------------------------------------------------

/// Configuration for the elevation tile pipeline.
#[derive(Resource)]
pub struct ElevationSettings {
    pub enabled: bool,
    pub decoder_name: String,
    pub vertical_exaggeration: f32,
    pub mesh_subdivisions: u32,
}

impl Default for ElevationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            decoder_name: "terrarium".into(),
            vertical_exaggeration: 1.5,
            mesh_subdivisions: 32,
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin setup
// ---------------------------------------------------------------------------

pub(super) fn setup_elevation_systems(app: &mut App) {
    app.init_resource::<ElevationCache>()
        .init_resource::<ElevationSettings>();
}

/// Save a decoded heightmap to the terrain cache as raw f32 data.
pub fn cache_heightmap_to_disk(key: &ElevationTileKey, heightmap: &Heightmap) {
    let cache_dir = crate::tile_cache::terrain_cache_dir();
    let path = cache_dir.join(format!(
        "{}.{}.{}.heightmap.raw",
        key.zoom, key.x, key.y
    ));
    if let Ok(bytes) = bytemuck_cast_heightmap(heightmap) {
        if let Err(e) = std::fs::write(&path, bytes) {
            warn!("Failed to cache heightmap {:?}: {}", path, e);
        }
    }
}

/// Load a cached heightmap from disk.
pub fn load_cached_heightmap(key: &ElevationTileKey, width: u32, height: u32) -> Option<Heightmap> {
    let cache_dir = crate::tile_cache::terrain_cache_dir();
    let path = cache_dir.join(format!(
        "{}.{}.{}.heightmap.raw",
        key.zoom, key.x, key.y
    ));
    let bytes = std::fs::read(&path).ok()?;
    let expected = (width * height * 4) as usize; // f32 = 4 bytes
    if bytes.len() != expected {
        return None;
    }
    let mut hm = Heightmap::new(width, height);
    for (i, chunk) in bytes.chunks_exact(4).enumerate() {
        hm.data[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    Some(hm)
}

fn bytemuck_cast_heightmap(hm: &Heightmap) -> Result<Vec<u8>, ()> {
    let mut bytes = Vec::with_capacity(hm.data.len() * 4);
    for &val in &hm.data {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    Ok(bytes)
}
