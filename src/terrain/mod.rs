//! 3D Terrain rendering module.
//!
//! Provides real-time terrain mesh generation from elevation tile data,
//! replacing flat tile quads with heightmap-displaced geometry in 3D mode.
//! Uses AWS Terrain Tiles (Terrarium PNG format) as the elevation data source.

pub(crate) mod heightmap;
pub(crate) mod material;
pub(crate) mod mesh;
pub(crate) mod provider;

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_slippy_tiles::MapTile;

use crate::constants;
use crate::map::MapState;
use crate::tiles::{TileFadeState, TileMeshQuad};
use crate::view3d::{self, TransitionState, View3DState};

use heightmap::{HeightmapCache, TileKey};
use mesh::{generate_terrain_mesh, resolution_for_zoom_offset, NeighborLod};
use provider::TerrainProvider;

// ---------------------------------------------------------------------------
// Resources and components
// ---------------------------------------------------------------------------

/// Controls whether terrain rendering is active and its parameters.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub(crate) struct TerrainState {
    /// Whether terrain mesh generation is enabled (vs flat tile quads).
    pub enabled: bool,
    /// Use GPU vertex displacement (Phase 2) instead of CPU meshes (Phase 1).
    pub gpu_terrain: bool,
    /// Base mesh resolution for nearest tiles (vertices per side: 32 or 64).
    pub mesh_resolution: u32,
}

impl Default for TerrainState {
    fn default() -> Self {
        Self {
            enabled: false,
            gpu_terrain: false,
            mesh_resolution: 32,
        }
    }
}

/// Component on entities that have terrain mesh geometry.
/// Stores the LOD resolution and tile key used to generate the mesh,
/// enabling detection of when regeneration is needed.
#[derive(Component)]
pub(crate) struct TerrainTile {
    /// Grid resolution used when this mesh was generated.
    pub resolution: u32,
    /// Tile key (zoom, x, y) for this terrain tile.
    pub tile_key: TileKey,
}

/// Cached terrain mesh entry with its generation parameters.
struct CachedTerrainMesh {
    handle: Handle<Mesh>,
    resolution: u32,
}

/// Cache of generated terrain mesh handles, keyed by tile coordinates.
#[derive(Resource, Default)]
pub(crate) struct TerrainMeshCache {
    meshes: HashMap<TileKey, CachedTerrainMesh>,
}

impl TerrainMeshCache {
    /// Look up the resolution used for a cached tile (for neighbor LOD queries).
    fn resolution(&self, key: &TileKey) -> Option<u32> {
        self.meshes.get(key).map(|e| e.resolution)
    }

    /// Evict mesh entries outside the active zoom band and enforce a size cap.
    fn evict(&mut self, current_zoom: u8, max_entries: usize) -> usize {
        let mut removed = 0;
        let min_zoom = current_zoom.saturating_sub(4);

        self.meshes.retain(|&(zoom, _, _), _| {
            let keep = zoom >= min_zoom && zoom <= current_zoom;
            if !keep {
                removed += 1;
            }
            keep
        });

        if self.meshes.len() > max_entries {
            let excess = self.meshes.len() - max_entries;
            let keys_to_remove: Vec<TileKey> = self.meshes.keys().take(excess).copied().collect();
            for key in keys_to_remove {
                self.meshes.remove(&key);
                removed += 1;
            }
        }

        removed
    }
}

/// Cache of GPU heightmap texture handles, keyed by tile coordinates.
#[derive(Resource, Default)]
pub(crate) struct HeightmapTextureCache {
    textures: HashMap<TileKey, Handle<Image>>,
}

/// Shared grid mesh for GPU terrain tiles.
/// All GPU terrain tiles use the same mesh — displacement happens in the shader.
#[derive(Resource)]
pub(crate) struct TerrainGridMesh(pub Handle<Mesh>);

/// Timer that controls how often cache eviction runs.
#[derive(Resource)]
struct EvictionTimer(Timer);

impl Default for EvictionTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(5.0, TimerMode::Repeating))
    }
}

/// Max cached heightmaps (256×256×f32 ≈ 256KB each → 400 = ~100MB)
const MAX_HEIGHTMAP_ENTRIES: usize = 400;
/// Max cached terrain meshes
const MAX_MESH_ENTRIES: usize = 400;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute the slippy tile key for a tile entity from its world-space transform.
///
/// Converts the tile's transform position back to world-pixel coordinates,
/// then to lat/lon, then to slippy tile coordinates.
pub(crate) fn tile_key_from_transform(
    transform: &Transform,
    fade_state: &TileFadeState,
    tile_settings: &bevy_slippy_tiles::SlippyTilesSettings,
    zoom_level: bevy_slippy_tiles::ZoomLevel,
) -> TileKey {
    let reference_ll = bevy_slippy_tiles::LatitudeLongitudeCoordinates {
        latitude: tile_settings.reference_latitude,
        longitude: tile_settings.reference_longitude,
    };
    let reference_pixel = bevy_slippy_tiles::world_coords_to_world_pixel(
        &reference_ll,
        constants::DEFAULT_TILE_SIZE,
        zoom_level,
    );

    let world_px_x = transform.translation.x as f64 + reference_pixel.0;
    let world_px_y = transform.translation.y as f64 + reference_pixel.1;

    let ll = bevy_slippy_tiles::world_pixel_to_world_coords(
        world_px_x,
        world_px_y,
        constants::DEFAULT_TILE_SIZE,
        zoom_level,
    );
    let tile_coords = bevy_slippy_tiles::SlippyTileCoordinates::from_latitude_longitude(
        ll.latitude,
        ll.longitude,
        zoom_level,
    );

    (fade_state.tile_zoom, tile_coords.x, tile_coords.y)
}

/// Build neighbor LOD info for edge stitching.
/// Checks the terrain mesh cache for the four adjacent tiles and returns
/// their resolutions when they are strictly lower than this tile's resolution.
fn build_neighbor_lod(
    tile_key: &TileKey,
    resolution: u32,
    cache: &TerrainMeshCache,
) -> NeighborLod {
    let (zoom, x, y) = *tile_key;

    let check = |nx: u32, ny: u32| -> Option<u32> {
        let neighbor_key: TileKey = (zoom, nx, ny);
        cache.resolution(&neighbor_key).filter(|&r| r < resolution)
    };

    NeighborLod {
        top: if y > 0 { check(x, y - 1) } else { None },
        bottom: check(x, y + 1),
        left: if x > 0 { check(x - 1, y) } else { None },
        right: check(x + 1, y),
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

pub(crate) struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<material::TerrainMaterial>::default())
            .init_resource::<TerrainState>()
            .init_resource::<TerrainMeshCache>()
            .init_resource::<HeightmapTextureCache>()
            .init_resource::<EvictionTimer>()
            .insert_resource(HeightmapCache::new(TerrainProvider::default()))
            .register_type::<TerrainState>()
            .add_systems(Startup, setup_terrain_grid_mesh)
            .add_systems(Update, heightmap::poll_heightmap_completions)
            .add_systems(
                Update,
                heightmap::request_heightmaps_for_tiles
                    .after(heightmap::poll_heightmap_completions),
            )
            .add_systems(
                Update,
                create_terrain_meshes.after(heightmap::request_heightmaps_for_tiles),
            )
            .add_systems(Update, evict_terrain_caches.after(create_terrain_meshes))
            .add_systems(
                Update,
                update_ground_elevation.after(heightmap::poll_heightmap_completions),
            )
            .add_systems(
                Update,
                animate_terrain_displacement.after(crate::tiles::sync_tile_mesh_transforms),
            )
            .add_systems(
                Update,
                create_gpu_terrain_tiles.after(heightmap::request_heightmaps_for_tiles),
            )
            .add_systems(
                Update,
                (sync_gpu_terrain_transforms, cleanup_gpu_terrain_companions)
                    .after(crate::tiles::sync_tile_mesh_transforms),
            );
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Upgrade flat tile mesh quads to terrain meshes when heightmap data is available.
///
/// This system checks tiles that already have a flat `TileMeshQuad` companion,
/// computes their slippy tile coordinates from their world-space position,
/// looks up their heightmap data, and if available, replaces the flat mesh
/// with a terrain-displaced mesh.
fn create_terrain_meshes(
    mut commands: Commands,
    terrain_state: Res<TerrainState>,
    view3d_state: Res<View3DState>,
    map_state: Res<MapState>,
    tile_settings: Res<bevy_slippy_tiles::SlippyTilesSettings>,
    heightmap_cache: Res<HeightmapCache>,
    mut terrain_mesh_cache: ResMut<TerrainMeshCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    // Tiles that have a mesh quad companion but no terrain marker yet
    tiles_to_upgrade: Query<
        (Entity, &Transform, &TileFadeState, &TileMeshQuad),
        (With<MapTile>, Without<TerrainTile>),
    >,
    mut mesh_query: Query<&mut Mesh3d>,
    // Tiles that already have terrain — used for cleanup when terrain is disabled
    terrain_tiles: Query<(Entity, &TileMeshQuad), (With<MapTile>, With<TerrainTile>)>,
    quad_mesh: Option<Res<crate::tiles::TileQuadMesh>>,
) {
    // Skip CPU path when GPU terrain is active
    if terrain_state.gpu_terrain && terrain_state.enabled && view3d_state.is_3d_active() {
        return;
    }

    // When terrain is disabled or we are in 2D mode, restore flat meshes.
    // The existing emissive material is kept — no material swap needed.
    if !terrain_state.enabled || !view3d_state.is_3d_active() {
        if let Some(ref quad_mesh) = quad_mesh {
            for (entity, mesh_quad) in terrain_tiles.iter() {
                if let Ok(mut mesh3d) = mesh_query.get_mut(mesh_quad.0) {
                    mesh3d.0 = quad_mesh.0.clone();
                }
                commands.entity(entity).try_remove::<TerrainTile>();
            }
        }
        return;
    }

    let current_zoom = map_state.zoom_level.to_u8();
    let altitude_scale = view3d::PIXEL_SCALE * view3d_state.altitude_scale;

    for (tile_entity, transform, fade_state, mesh_quad) in tiles_to_upgrade.iter() {
        let tile_key =
            tile_key_from_transform(transform, fade_state, &tile_settings, map_state.zoom_level);

        let zoom_offset = current_zoom.saturating_sub(fade_state.tile_zoom);
        let resolution = resolution_for_zoom_offset(zoom_offset as u32);

        // Check if we already have a cached terrain mesh for this tile at this resolution
        let mesh_handle = if let Some(entry) = terrain_mesh_cache.meshes.get(&tile_key) {
            if entry.resolution == resolution {
                entry.handle.clone()
            } else {
                // Resolution changed (zoom transition) — regenerate
                let Some(heightmap) = heightmap_cache.get(&tile_key) else {
                    continue;
                };
                let neighbor_lod = build_neighbor_lod(&tile_key, resolution, &terrain_mesh_cache);
                let terrain_mesh = generate_terrain_mesh(
                    heightmap,
                    constants::DEFAULT_TILE_PIXELS,
                    resolution,
                    altitude_scale,
                    true,
                    &neighbor_lod,
                );
                let handle = meshes.add(terrain_mesh);
                terrain_mesh_cache.meshes.insert(
                    tile_key,
                    CachedTerrainMesh {
                        handle: handle.clone(),
                        resolution,
                    },
                );
                handle
            }
        } else {
            // Generate terrain mesh from heightmap data (skip if not yet downloaded)
            let Some(heightmap) = heightmap_cache.get(&tile_key) else {
                continue;
            };

            let neighbor_lod = build_neighbor_lod(&tile_key, resolution, &terrain_mesh_cache);
            let terrain_mesh = generate_terrain_mesh(
                heightmap,
                constants::DEFAULT_TILE_PIXELS,
                resolution,
                altitude_scale,
                true,
                &neighbor_lod,
            );

            let handle = meshes.add(terrain_mesh);
            terrain_mesh_cache.meshes.insert(
                tile_key,
                CachedTerrainMesh {
                    handle: handle.clone(),
                    resolution,
                },
            );
            handle
        };

        // Replace the flat mesh with the terrain mesh.
        if let Ok(mut mesh3d) = mesh_query.get_mut(mesh_quad.0) {
            mesh3d.0 = mesh_handle;
        }

        // Mark with LOD info so we can detect resolution changes
        commands.entity(tile_entity).try_insert(TerrainTile {
            resolution,
            tile_key,
        });
    }
}

/// Periodically evict stale heightmap and mesh cache entries to bound memory usage.
/// Runs every 5 seconds. Removes entries outside the active zoom band and enforces
/// size caps on both caches.
fn evict_terrain_caches(
    time: Res<Time>,
    map_state: Res<MapState>,
    mut timer: ResMut<EvictionTimer>,
    mut heightmap_cache: ResMut<HeightmapCache>,
    mut mesh_cache: ResMut<TerrainMeshCache>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let current_zoom = map_state.zoom_level.to_u8();
    let hm_removed = heightmap_cache.evict(current_zoom, MAX_HEIGHTMAP_ENTRIES);
    let mesh_removed = mesh_cache.evict(current_zoom, MAX_MESH_ENTRIES);

    if hm_removed > 0 || mesh_removed > 0 {
        debug!(
            "Terrain cache eviction: {} heightmaps, {} meshes removed (remaining: {} hm, {} mesh)",
            hm_removed,
            mesh_removed,
            heightmap_cache.len(),
            mesh_cache.meshes.len()
        );
    }
}

/// Sample the heightmap at the camera's map center position and update
/// `View3DState::ground_elevation_ft`. Only runs when terrain is enabled
/// and 3D mode is active. Falls back to the existing airport-based detection
/// when no heightmap data is available at the current position.
fn update_ground_elevation(
    terrain_state: Res<TerrainState>,
    mut view3d_state: ResMut<View3DState>,
    map_state: Res<MapState>,
    mut heightmap_cache: ResMut<HeightmapCache>,
) {
    if !terrain_state.enabled || !view3d_state.is_3d_active() {
        return;
    }

    let zoom = map_state.zoom_level.to_u8();
    let zoom_level = map_state.zoom_level;

    // Ensure the heightmap for the camera center tile is requested
    let tile_coords = bevy_slippy_tiles::SlippyTileCoordinates::from_latitude_longitude(
        map_state.latitude,
        map_state.longitude,
        zoom_level,
    );
    let center_key: heightmap::TileKey = (zoom, tile_coords.x, tile_coords.y);
    if !heightmap_cache.contains(&center_key) {
        heightmap_cache.request(center_key);
    }

    if let Some(elevation_m) =
        heightmap_cache.sample_elevation(map_state.latitude, map_state.longitude, zoom_level)
    {
        let elevation_ft = (elevation_m * 3.28084) as i32;
        view3d_state.ground_elevation_ft = elevation_ft;
    }
}

/// Animate terrain displacement during 2D↔3D transitions.
///
/// During transitions, scales the Y component of terrain mesh entities
/// from 0.0 (flat, 2D) to 1.0 (full displacement, 3D) using the same
/// smooth-step curve as the camera transition. This prevents terrain
/// from popping in/out instantly when toggling view modes.
fn animate_terrain_displacement(
    view3d_state: Res<View3DState>,
    terrain_state: Res<TerrainState>,
    terrain_tiles: Query<&TileMeshQuad, (With<MapTile>, With<TerrainTile>)>,
    mut mesh_transforms: Query<&mut Transform, Without<MapTile>>,
) {
    if !terrain_state.enabled {
        return;
    }

    // Compute transition factor: 0.0 = fully 2D (flat), 1.0 = fully 3D.
    // Must run every frame (not just during transitions) because
    // sync_tile_mesh_transforms resets scale.y = 1.0 each frame.
    let t = match view3d_state.transition {
        TransitionState::Idle => match view3d_state.mode {
            crate::view3d::ViewMode::Perspective3D => 1.0,
            crate::view3d::ViewMode::Map2D => 0.0,
        },
        TransitionState::TransitioningTo3D { progress } => {
            let s = progress;
            s * s * (3.0 - 2.0 * s)
        }
        TransitionState::TransitioningTo2D { progress } => {
            let s = 1.0 - progress;
            s * s * (3.0 - 2.0 * s)
        }
    };

    // When fully 3D and idle, no need to override (sync_tile_mesh_transforms sets 1.0)
    if t >= 1.0 {
        return;
    }

    for mesh_quad in terrain_tiles.iter() {
        if let Ok(mut mesh_tf) = mesh_transforms.get_mut(mesh_quad.0) {
            // Scale Y by transition factor. X and Z stay as set by sync_tile_mesh_transforms.
            mesh_tf.scale.y = t;
        }
    }
}

/// Marker on tile entities that have been upgraded to GPU terrain.
#[derive(Component)]
struct GpuTerrainTile;

/// Component on GPU terrain companion entities linking back to their source tile.
#[derive(Component)]
struct GpuTerrainCompanion {
    source_tile: Entity,
}

/// Create the shared grid mesh used by all GPU terrain tiles.
fn setup_terrain_grid_mesh(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    // 64x64 grid — same resolution as the highest CPU LOD tier.
    // Vertex displacement happens in the shader, so one mesh fits all tiles.
    let mesh = Mesh::from(Plane3d::new(
        Vec3::Y,
        Vec2::splat(constants::DEFAULT_TILE_PIXELS / 2.0),
    ))
    .with_generated_tangents()
    .expect("Plane3d tangent generation");
    commands.insert_resource(TerrainGridMesh(meshes.add(mesh)));
}

/// GPU terrain path: for tiles with available heightmaps, replace the flat
/// `TileQuad3d` companion with a `TerrainMaterial` entity that performs
/// vertex displacement in the shader.
///
/// Only runs when `terrain_state.gpu_terrain` is true.
fn create_gpu_terrain_tiles(
    mut commands: Commands,
    terrain_state: Res<TerrainState>,
    view3d_state: Res<View3DState>,
    map_state: Res<MapState>,
    tile_settings: Res<bevy_slippy_tiles::SlippyTilesSettings>,
    heightmap_cache: Res<HeightmapCache>,
    grid_mesh: Option<Res<TerrainGridMesh>>,
    mut heightmap_textures: ResMut<HeightmapTextureCache>,
    mut images: ResMut<Assets<Image>>,
    mut terrain_materials: ResMut<Assets<material::TerrainMaterial>>,
    // Tiles with mesh quads but no GPU terrain yet
    tiles_to_upgrade: Query<
        (Entity, &Sprite, &Transform, &TileFadeState, &TileMeshQuad),
        (With<MapTile>, Without<GpuTerrainTile>),
    >,
    mut mesh_query: Query<(
        &mut Mesh3d,
        &mut MeshMaterial3d<bevy::pbr::StandardMaterial>,
    )>,
    // Existing GPU terrain tiles — for cleanup (has Sprite via MapTile)
    mut gpu_tiles: Query<
        (Entity, &TileMeshQuad, &mut TileFadeState, &mut Sprite),
        (With<MapTile>, With<GpuTerrainTile>),
    >,
    // GPU terrain companion entities to despawn on cleanup
    gpu_companions: Query<Entity, With<GpuTerrainCompanion>>,
    quad_mesh: Option<Res<crate::tiles::TileQuadMesh>>,
) {
    // Only active when GPU terrain is enabled
    if !terrain_state.enabled || !terrain_state.gpu_terrain || !view3d_state.is_3d_active() {
        // Cleanup: remove markers, reset sprite fade so tiles re-appear in 2D,
        // and despawn GPU companion entities
        for (entity, _mesh_quad, mut fade_state, mut sprite) in gpu_tiles.iter_mut() {
            // Reset alpha so animate_tile_fades will re-fade the sprite in
            fade_state.alpha = 0.0;
            sprite.color = Color::srgba(1.0, 1.0, 1.0, 0.0);
            commands.entity(entity).try_remove::<GpuTerrainTile>();
        }
        for companion in gpu_companions.iter() {
            commands.entity(companion).despawn();
        }
        return;
    }

    let Some(grid_mesh) = grid_mesh else { return };

    let altitude_scale = view3d::PIXEL_SCALE * view3d_state.altitude_scale;
    let transition_factor = match view3d_state.transition {
        TransitionState::Idle => 1.0,
        TransitionState::TransitioningTo3D { progress } => {
            progress * progress * (3.0 - 2.0 * progress)
        }
        TransitionState::TransitioningTo2D { progress } => {
            let s = 1.0 - progress;
            s * s * (3.0 - 2.0 * s)
        }
    };

    let current_zoom = map_state.zoom_level.to_u8();

    for (tile_entity, sprite, transform, fade_state, mesh_quad) in tiles_to_upgrade.iter() {
        if fade_state.tile_zoom != current_zoom {
            continue;
        }

        let tile_key =
            tile_key_from_transform(transform, fade_state, &tile_settings, map_state.zoom_level);

        // Need heightmap data for this tile
        let Some(heightmap) = heightmap_cache.get(&tile_key) else {
            continue;
        };

        // Get or create GPU texture for this heightmap
        let heightmap_handle = heightmap_textures
            .textures
            .entry(tile_key)
            .or_insert_with(|| images.add(heightmap.to_gpu_image()))
            .clone();

        // Create TerrainMaterial for this tile
        let terrain_mat = terrain_materials.add(material::TerrainMaterial {
            params: material::TerrainParams {
                elevation_scale: altitude_scale,
                texel_size: 1.0 / heightmap.width() as f32,
                emissive_boost: 8.0, // Match TILE_EMISSIVE_BOOST
                transition_factor,
            },
            satellite_texture: Some(sprite.image.clone()),
            heightmap_texture: Some(heightmap_handle),
        });

        // The original StandardMaterial companion stays visible (tiles.rs manages its
        // lifecycle). The TerrainMaterial companion renders on the same layer and
        // covers it via depth testing — both are opaque at the same position.

        let pos_yup = view3d::zup_to_yup(transform.translation);

        commands.spawn((
            GpuTerrainCompanion {
                source_tile: tile_entity,
            },
            Mesh3d(grid_mesh.0.clone()),
            MeshMaterial3d(terrain_mat),
            Transform::from_translation(pos_yup).with_scale(Vec3::new(
                transform.scale.x,
                1.0,
                transform.scale.x,
            )),
            Pickable::IGNORE,
            bevy::camera::visibility::RenderLayers::layer(crate::RenderCategory::TILES_3D),
        ));

        commands.entity(tile_entity).try_insert(GpuTerrainTile);
    }
}

/// Sync GPU terrain companion transforms with their source tile's
/// StandardMaterial companion (which tiles.rs keeps positioned correctly).
fn sync_gpu_terrain_transforms(
    companions: Query<(&GpuTerrainCompanion, Entity)>,
    tile_query: Query<(&Transform, &TileMeshQuad), With<MapTile>>,
    mut transforms: Query<&mut Transform, Without<MapTile>>,
) {
    // Collect source positions first to avoid borrow conflicts
    let updates: Vec<(Entity, Vec3, Vec3)> = companions
        .iter()
        .filter_map(|(comp, comp_entity)| {
            let (tile_tf, mesh_quad) = tile_query.get(comp.source_tile).ok()?;
            let source_tf = transforms.get(mesh_quad.0).ok()?;
            Some((comp_entity, source_tf.translation, source_tf.scale))
        })
        .collect();

    for (comp_entity, translation, scale) in updates {
        if let Ok(mut tf) = transforms.get_mut(comp_entity) {
            tf.translation = translation;
            tf.scale = scale;
        }
    }
}

/// Despawn GPU terrain companion entities whose source tile no longer exists.
fn cleanup_gpu_terrain_companions(
    mut commands: Commands,
    companions: Query<(Entity, &GpuTerrainCompanion)>,
    tiles: Query<Entity, With<MapTile>>,
) {
    for (companion_entity, companion) in companions.iter() {
        if tiles.get(companion.source_tile).is_err() {
            commands.entity(companion_entity).despawn();
        }
    }
}
