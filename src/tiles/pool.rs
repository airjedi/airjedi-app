use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

/// Tile entity lifecycle state.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TileState {
    /// In the pool, hidden, available for reuse.
    Idle,
    /// Visible, displaying a tile texture.
    Active,
    /// Visible but scheduled for replacement. Stays visible until a new tile
    /// covers its grid cell, then transitions to Idle.
    Retiring,
}

/// Grid-space position key for deduplication: (tile_x, tile_y, zoom).
/// Uses tile coordinates directly, which are the natural dedup key in
/// the Mercator meter coordinate system.
pub type TileGridKey = (u32, u32, u8);

/// Maps grid positions to their active tile entity.
#[derive(Resource, Default)]
pub struct TileGrid {
    pub occupied: HashMap<TileGridKey, Entity>,
}

/// Pool of pre-allocated tile entities ready for reuse.
///
/// Uses a dedup set alongside the available stack to prevent the same entity
/// from being returned twice. Multiple systems (cull_offscreen_tiles,
/// animate_tile_fades) can call release_tile on the same entity within a
/// single frame because Bevy's command application is deferred - the MapTile
/// marker removal hasn't happened yet when the second system queries.
#[derive(Resource)]
pub struct TilePool {
    /// Entities in the Idle state, available for assignment.
    pub available: Vec<Entity>,
    /// Tracks which entities are currently in the available stack.
    in_pool: HashSet<Entity>,
    /// Total entities ever allocated (pool only grows).
    pub capacity: usize,
}

impl Default for TilePool {
    fn default() -> Self {
        Self {
            available: Vec::with_capacity(256),
            in_pool: HashSet::with_capacity(256),
            capacity: 0,
        }
    }
}

impl TilePool {
    /// Take an entity from the pool. Returns None if pool is empty.
    pub fn take(&mut self) -> Option<Entity> {
        let entity = self.available.pop()?;
        self.in_pool.remove(&entity);
        Some(entity)
    }

    /// Return an entity to the pool. Idempotent - silently ignores
    /// duplicates caused by deferred command application.
    pub fn release(&mut self, entity: Entity) {
        if self.in_pool.insert(entity) {
            self.available.push(entity);
        }
    }

    /// Number of entities currently available for reuse.
    pub fn available_count(&self) -> usize {
        self.available.len()
    }
}

/// Pre-warm the pool by spawning a batch of hidden tile entities.
/// Entities are spawned with minimal components - the correct mesh and
/// material are set when the entity is activated via `activate_tile`.
pub fn grow_pool(
    commands: &mut Commands,
    pool: &mut TilePool,
    count: usize,
) {
    for _ in 0..count {
        let entity = commands
            .spawn((
                Name::new("Tile (pooled)"),
                Transform::default(),
                Visibility::Hidden,
                TileState::Idle,
                Pickable::IGNORE,
                bevy::camera::visibility::RenderLayers::layer(
                    crate::RenderCategory::TILES,
                ),
                bevy::camera::visibility::NoFrustumCulling,
            ))
            .id();
        pool.available.push(entity);
        pool.in_pool.insert(entity);
        pool.capacity += 1;
    }
    debug!("Grew tile pool by {} (total capacity: {})", count, pool.capacity);
}

/// Transition a tile entity from Active to Retiring.
/// It stays visible but is eligible for replacement.
pub fn retire_tile(
    commands: &mut Commands,
    entity: Entity,
    grid: &mut TileGrid,
    key: &TileGridKey,
) {
    grid.occupied.remove(key);
    commands.entity(entity).insert(TileState::Retiring);
}

/// Transition a tile entity from Active/Retiring to Idle.
/// Removes the MapTile marker so idle entities don't match tile queries
/// (preventing double-release on mode switches). Keeps Mesh3d and
/// MeshMaterial3d on the entity - removing them causes use-after-free in
/// Bevy 0.19's slab allocator when the render world still holds references
/// to the freed GPU data. activate_tile's insert() safely replaces them.
pub fn release_tile(
    commands: &mut Commands,
    entity: Entity,
    pool: &mut TilePool,
) {
    commands.entity(entity).insert((
        TileState::Idle,
        Visibility::Hidden,
        Transform::default(),
    ));
    commands.entity(entity).remove::<(
        super::MapTile,
        super::render::TileOriginalImage,
        super::render::TileFadeState,
    )>();
    pool.release(entity);
}

/// Activate a pooled tile entity at a specific position with mesh and material.
pub fn activate_tile(
    commands: &mut Commands,
    entity: Entity,
    grid: &mut TileGrid,
    key: TileGridKey,
    transform: Transform,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    original_image: Handle<Image>,
    tile_zoom: u8,
    spawn_time: f64,
) {
    commands.entity(entity).insert((
        super::MapTile,
        TileState::Active,
        Visibility::Hidden,
        transform,
        Mesh3d(mesh),
        MeshMaterial3d(material),
        super::render::TileOriginalImage(original_image),
        super::render::TileFadeState {
            alpha: 0.0,
            tile_zoom,
            spawn_time,
        },
    ));
    grid.occupied.insert(key, entity);
}

pub(super) fn setup_pool_systems(app: &mut App) {
    app.init_resource::<TilePool>()
        .init_resource::<TileGrid>();
}
