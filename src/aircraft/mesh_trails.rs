use bevy::prelude::*;
use bevy::mesh::PrimitiveTopology;
use bevy::asset::RenderAssetUsages;
use bevy_slippy_tiles::SlippyTilesSettings;

use super::components::Aircraft;
use super::trails::{altitude_color, age_opacity, TrailConfig, TrailHistory, TrailRenderer, SessionClock};
use super::staleness::{staleness_opacity, aircraft_age_secs};
use crate::geo::CoordinateConverter;
use crate::map::MapState;
use crate::view3d::View3DState;

#[derive(Component)]
pub struct MeshTrailMarker;

#[derive(Component)]
pub struct MeshTrailEffect {
    pub aircraft_entity: Entity,
    pub mesh_handle: Handle<Mesh>,
    pub material_handle: Handle<StandardMaterial>,
}

const TRAIL_HALF_WIDTH_2D: f32 = 1.275;
const TRAIL_HALF_WIDTH_3D: f32 = 3.0;

pub fn spawn_mesh_trails(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    trail_config: Res<TrailConfig>,
    view3d_state: Res<View3DState>,
    aircraft_query: Query<Entity, (With<Aircraft>, Without<MeshTrailMarker>)>,
) {
    let is_3d = view3d_state.is_3d_active();
    let active_renderer = if is_3d { trail_config.renderer_3d } else { trail_config.renderer_2d };
    if !trail_config.enabled || active_renderer != TrailRenderer::MeshStrip {
        return;
    }

    for aircraft_entity in aircraft_query.iter() {
        let mesh = Mesh::new(PrimitiveTopology::TriangleStrip, RenderAssetUsages::default());
        let mesh_handle = meshes.add(mesh);

        let material = StandardMaterial {
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        };
        let material_handle = materials.add(material);

        let trail_entity = commands.spawn((
            Mesh3d(mesh_handle.clone()),
            MeshMaterial3d(material_handle.clone()),
            Transform::default(),
            MeshTrailEffect {
                aircraft_entity,
                mesh_handle,
                material_handle,
            },
        )).id();

        commands.entity(aircraft_entity).insert(MeshTrailMarker);

        let _ = trail_entity;
    }
}

pub fn update_mesh_trails(
    tile_settings: Res<SlippyTilesSettings>,
    map_state: Res<MapState>,
    view3d_state: Res<View3DState>,
    trail_config: Res<TrailConfig>,
    clock: Res<SessionClock>,
    mut meshes: ResMut<Assets<Mesh>>,
    aircraft_query: Query<(&TrailHistory, &Aircraft)>,
    effect_query: Query<&MeshTrailEffect>,
) {
    if !trail_config.enabled {
        return;
    }

    let is_3d = view3d_state.is_3d_active();
    let active_renderer = if is_3d { trail_config.renderer_3d } else { trail_config.renderer_2d };
    if active_renderer != TrailRenderer::MeshStrip {
        return;
    }

    let render_zoom = view3d_state.effective_zoom(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, render_zoom);

    for effect in effect_query.iter() {
        let Ok((trail, aircraft)) = aircraft_query.get(effect.aircraft_entity) else {
            continue;
        };

        let Some(mesh) = meshes.get_mut(&effect.mesh_handle) else {
            continue;
        };

        if trail.points.len() < 2 {
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, Vec::<[f32; 4]>::new());
            continue;
        }

        let stale_opacity = staleness_opacity(aircraft_age_secs(aircraft));

        let mut positions: Vec<[f32; 3]> = Vec::with_capacity(trail.points.len() * 2);
        let mut colors: Vec<[f32; 4]> = Vec::with_capacity(trail.points.len() * 2);

        let mut prev_dir: Option<Vec2> = None;

        for (i, point) in trail.points.iter().enumerate() {
            let opacity = age_opacity(
                clock.age_secs(point.timestamp),
                trail_config.solid_duration_seconds,
                trail_config.fade_duration_seconds,
            );

            if opacity <= 0.0 {
                prev_dir = None;
                continue;
            }

            let xy = converter.latlon_to_world(point.lat, point.lon);
            let z = if is_3d {
                view3d_state.altitude_to_z(point.altitude.unwrap_or(0))
            } else {
                2.0
            };

            // Compute direction to next point (or reuse previous)
            let dir = if i + 1 < trail.points.len() {
                let next = &trail.points[i + 1];
                let next_xy = converter.latlon_to_world(next.lat, next.lon);
                let d = next_xy - xy;
                if d.length_squared() > 0.0001 {
                    d.normalize()
                } else {
                    prev_dir.unwrap_or(Vec2::Y)
                }
            } else {
                prev_dir.unwrap_or(Vec2::Y)
            };

            let half_width = if is_3d { TRAIL_HALF_WIDTH_3D } else { TRAIL_HALF_WIDTH_2D };
            let perp = Vec2::new(-dir.y, dir.x) * half_width;

            // Z-up positions: (x, y, altitude)
            let left_zup = Vec3::new(xy.x + perp.x, xy.y + perp.y, z);
            let right_zup = Vec3::new(xy.x - perp.x, xy.y - perp.y, z);

            // In 3D mode, Camera3d uses Y-up: (x, z, -y)
            let (left, right) = if is_3d {
                (
                    Vec3::new(left_zup.x, left_zup.z, -left_zup.y),
                    Vec3::new(right_zup.x, right_zup.z, -right_zup.y),
                )
            } else {
                (left_zup, right_zup)
            };

            let base_color = altitude_color(point.altitude);
            let linear = base_color.to_linear();
            let alpha = opacity * stale_opacity;
            let color = [linear.red, linear.green, linear.blue, alpha];

            positions.push(left.into());
            positions.push(right.into());
            colors.push(color);
            colors.push(color);

            prev_dir = Some(dir);
        }

        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
}

pub fn cleanup_mesh_trails(
    mut commands: Commands,
    view3d_state: Res<View3DState>,
    trail_config: Res<TrailConfig>,
    aircraft_query: Query<Entity, With<Aircraft>>,
    effect_query: Query<(Entity, &MeshTrailEffect)>,
) {
    let is_3d = view3d_state.is_3d_active();
    let active_renderer = if is_3d { trail_config.renderer_3d } else { trail_config.renderer_2d };
    let inactive = active_renderer != TrailRenderer::MeshStrip || !trail_config.enabled;

    for (effect_entity, effect) in effect_query.iter() {
        let aircraft_gone = aircraft_query.get(effect.aircraft_entity).is_err();
        if inactive || aircraft_gone {
            commands.entity(effect_entity).despawn();
            if !aircraft_gone {
                commands.entity(effect.aircraft_entity).remove::<MeshTrailMarker>();
            }
        }
    }
}
