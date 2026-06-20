use bevy::camera::visibility::RenderLayers;

/// Centralized render layer assignments.
/// Each visual category gets its own layer so cameras can subscribe
/// to exactly the layers they need per mode.
///
/// Aircraft, lights, and SceneRoot children stay on layer 0 (default)
/// to avoid propagation complexity. Only visual categories that caused
/// z-fighting (tiles) or need separate ordering are on dedicated layers.
///
/// Recipe for adding a new entity type:
/// 1. Add a constant here (or use layer 0 for 3D meshes with SceneRoot)
/// 2. Add the layer to the appropriate camera helper function
/// 3. Spawn entity with RenderLayers::layer(RenderCategory::YOUR_TYPE)
pub struct RenderCategory;

impl RenderCategory {
    pub const DEFAULT: usize = 0;      // Aircraft, lights, SceneRoot children
    pub const TILES_2D: usize = 1;     // Tile sprites (2D rendering)
    pub const GIZMOS: usize = 2;       // Trails, navaids, runways
    pub const OVERLAYS_2D: usize = 4;  // Day/night tint, weather overlays
    pub const LABELS: usize = 5;       // Text2d labels
    pub const TILES_3D: usize = 6;     // Tile mesh quads (3D rendering)
    pub const GROUND: usize = 7;       // Ground plane (3D only)
    pub const SKY: usize = 8;          // Star field (3D only)
    pub const AIRSPACE: usize = 9;     // Airspace volumes (2D and 3D)
    pub const UI: usize = 11;          // egui (unchanged)
}

/// Layers the Map Camera (Camera2d) subscribes to in 2D mode.
pub fn layers_2d_map() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::TILES_2D,
        RenderCategory::GIZMOS,
        RenderCategory::OVERLAYS_2D,
        RenderCategory::LABELS,
    ])
}

/// Layers the Map Camera (Camera2d) subscribes to in 3D mode.
/// Only gizmos and labels -- tiles are mesh quads on Camera3d.
pub fn layers_3d_overlay() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::GIZMOS,
        RenderCategory::LABELS,
    ])
}

/// All layers Camera2d might ever need (union of 2D map + 3D overlay).
/// Used at startup to avoid deferred render-layer commands during mode switches.
/// Extra layers in 3D mode are harmless since tile sprites are hidden.
pub fn layers_camera2d_all() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::TILES_2D,
        RenderCategory::GIZMOS,
        RenderCategory::OVERLAYS_2D,
        RenderCategory::LABELS,
    ])
}

/// Layers the Aircraft Camera (Camera3d) subscribes to in 2D mode.
/// Layer 0 covers aircraft meshes, lights, and SceneRoot children.
pub fn layers_2d_aircraft() -> RenderLayers {
    RenderLayers::layer(RenderCategory::DEFAULT)
}

/// Layers the Aircraft Camera (Camera3d) subscribes to in 3D mode.
/// Layer 0 for aircraft/lights, plus tile meshes, ground plane, sky.
pub fn layers_3d_world() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::DEFAULT,
        RenderCategory::TILES_3D,
        RenderCategory::GROUND,
        RenderCategory::SKY,
        RenderCategory::AIRSPACE,
    ])
}
