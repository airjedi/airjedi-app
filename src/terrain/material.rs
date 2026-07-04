//! GPU terrain material with vertex displacement from heightmap textures.
//!
//! Replaces CPU-generated per-tile meshes with a single shared grid mesh
//! and a per-tile material that performs vertex displacement in the shader.
//! The heightmap is uploaded as a GPU texture and sampled in the vertex shader.

use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// Shader asset path relative to `assets/`.
const TERRAIN_SHADER: &str = "shaders/terrain.wgsl";

/// Uniform parameters passed to the terrain vertex/fragment shader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect, ShaderType)]
pub(crate) struct TerrainParams {
    /// Combined elevation scale: `elevation_m * 0.001 * elevation_scale` → world units.
    /// Typically 50.0 (altitude exaggeration factor).
    pub elevation_scale: f32,
    /// Size of one heightmap texel in UV space: `1.0 / heightmap_width`.
    /// Used for central-difference normal computation.
    pub texel_size: f32,
    /// Emissive brightness multiplier (bypasses HDR exposure). Matches `TILE_EMISSIVE_BOOST`.
    pub emissive_boost: f32,
    /// Transition factor: 0.0 = flat (2D), 1.0 = full displacement (3D).
    /// Animated during 2D↔3D transitions.
    pub transition_factor: f32,
}

/// Custom material for GPU-displaced terrain tiles.
///
/// Each tile gets its own `TerrainMaterial` instance with the satellite texture
/// and heightmap texture specific to that tile. The vertex shader samples the
/// heightmap and displaces vertices, while the fragment shader renders the
/// satellite texture as emissive with subtle directional shading.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct TerrainMaterial {
    #[uniform(0)]
    pub params: TerrainParams,

    /// Satellite imagery texture (the visible map tile).
    #[texture(1)]
    #[sampler(2)]
    pub satellite_texture: Option<Handle<Image>>,

    /// Heightmap texture (Terrarium-encoded PNG, RGB → elevation).
    /// Visibility set to all stages since it's sampled in the vertex shader.
    #[texture(3, visibility(all))]
    #[sampler(4, visibility(all))]
    pub heightmap_texture: Option<Handle<Image>>,
}

impl Material for TerrainMaterial {
    fn vertex_shader() -> ShaderRef {
        TERRAIN_SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        TERRAIN_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Opaque
    }
}
