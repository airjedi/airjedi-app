use bevy::camera::visibility::RenderLayers;

/// Centralized render layer assignments.
///
/// The rendering pipeline uses two cameras:
/// - Camera3d (AircraftCamera, order 0): Primary renderer for tiles, aircraft,
///   sky, ground. Clears with dark background. Uses orthographic in 2D mode,
///   perspective in 3D mode.
/// - Camera2d (MapCamera, order 1): Overlay for gizmos, labels, overlays.
///   Alpha-blends on top of Camera3d output. Never renders tiles.
pub struct RenderCategory;

impl RenderCategory {
    pub const DEFAULT: usize = 0; // Aircraft, lights, SceneRoot children
    pub const TILES: usize = 1; // Tile mesh planes (both 2D and 3D)
    pub const GIZMOS: usize = 2; // Trails, navaids, runways
    pub const OVERLAYS_2D: usize = 4; // Day/night tint, weather overlays
    pub const LABELS: usize = 5; // Text2d labels
    pub const GROUND: usize = 7; // Ground plane (3D only)
    pub const SKY: usize = 8; // Star field (3D only)
    pub const AIRSPACE: usize = 9; // Airspace volumes (2D and 3D)
    pub const UI: usize = 11; // egui (unchanged)

    // Backward compatibility aliases
    pub const TILES_2D: usize = Self::TILES;
    pub const TILES_3D: usize = Self::TILES;
}

/// Layers Camera2d (MapCamera) subscribes to - overlay only, no tiles.
pub fn layers_2d_map() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::GIZMOS,
        RenderCategory::OVERLAYS_2D,
        RenderCategory::LABELS,
    ])
}

/// Layers Camera2d subscribes to in 3D mode - same as 2D (overlay only).
pub fn layers_3d_overlay() -> RenderLayers {
    RenderLayers::from_layers(&[RenderCategory::GIZMOS, RenderCategory::LABELS])
}

/// All layers Camera2d might ever need (overlays only).
pub fn layers_camera2d_all() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::GIZMOS,
        RenderCategory::OVERLAYS_2D,
        RenderCategory::LABELS,
    ])
}

/// Layers Camera3d (AircraftCamera) subscribes to in 2D mode.
/// Tiles + aircraft (default layer 0).
pub fn layers_2d_aircraft() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::DEFAULT,
        RenderCategory::TILES,
    ])
}

/// Layers Camera3d (AircraftCamera) subscribes to in 3D mode.
/// Tiles + aircraft + ground + sky + airspace.
pub fn layers_3d_world() -> RenderLayers {
    RenderLayers::from_layers(&[
        RenderCategory::DEFAULT,
        RenderCategory::TILES,
        RenderCategory::GROUND,
        RenderCategory::SKY,
        RenderCategory::AIRSPACE,
    ])
}
