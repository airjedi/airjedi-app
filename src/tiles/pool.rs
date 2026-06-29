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

/// Grid-space position key for deduplication: (map_x, map_y, zoom).
/// map_x and map_y are in world pixel coordinates rounded to integers.
pub type TileGridKey = (i32, i32, u8);

/// Maps grid positions to their active tile entity.
#[derive(Resource, Default)]
pub struct TileGrid {
    pub occupied: HashMap<TileGridKey, Entity>,
}

/// Pool of pre-allocated tile entities ready for reuse.
#[derive(Resource)]
pub struct TilePool {
    /// Entities in the Idle state, available for assignment.
    pub available: Vec<Entity>,
    /// Total entities ever allocated (pool only grows).
    pub capacity: usize,
}

impl Default for TilePool {
    fn default() -> Self {
        Self {
            available: Vec::with_capacity(256),
            capacity: 0,
        }
    }
}

impl TilePool {
    /// Take an entity from the pool. Returns None if pool is empty.
    pub fn take(&mut self) -> Option<Entity> {
        self.available.pop()
    }

    /// Return an entity to the pool.
    pub fn release(&mut self, entity: Entity) {
        self.available.push(entity);
    }

    /// Number of entities currently available for reuse.
    pub fn available_count(&self) -> usize {
        self.available.len()
    }
}

/// Pre-warm the pool by spawning a batch of hidden tile entities.
/// Called at startup and whenever the pool is exhausted.
pub fn grow_pool(
    commands: &mut Commands,
    pool: &mut TilePool,
    mesh: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
    count: usize,
) {
    for _ in 0..count {
        let entity = commands
            .spawn((
                Name::new("Tile (pooled)"),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Hidden,
                TileState::Idle,
                super::MapTile,
                super::render::TileOriginalImage(Handle::default()),
                super::render::TileFadeState {
                    alpha: 0.0,
                    tile_zoom: 0,
                    spawn_time: 0.0,
                },
                Pickable::IGNORE,
                bevy::camera::visibility::RenderLayers::layer(
                    crate::RenderCategory::TILES,
                ),
            ))
            .id();
        pool.available.push(entity);
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

/// Transition a tile entity from Retiring (or Active) to Idle.
/// Hides it and returns it to the pool.
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
    pool.release(entity);
}

/// Activate a pooled tile entity at a specific position with a material.
pub fn activate_tile(
    commands: &mut Commands,
    entity: Entity,
    grid: &mut TileGrid,
    key: TileGridKey,
    transform: Transform,
    material: Handle<StandardMaterial>,
    original_image: Handle<Image>,
    tile_zoom: u8,
    spawn_time: f64,
) {
    commands.entity(entity).insert((
        TileState::Active,
        Visibility::Hidden, // starts hidden until texture loads
        transform,
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
