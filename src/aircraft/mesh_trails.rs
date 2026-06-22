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

    let converter = CoordinateConverter::new(&tile_settings, map_state.zoom_level);

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

        let mut positions: Vec<[f32; 3]> = Vec::with_capacity(trail.points.len() * 4);
        let mut colors: Vec<[f32; 4]> = Vec::with_capacity(trail.points.len() * 4);

        let mut prev_dir: Option<Vec2> = None;
        let mut prev_xy: Option<Vec2> = None;
        let mut prev_z: Option<f32> = None;
        let mut prev_estimated = false;

        for (i, point) in trail.points.iter().enumerate() {
            let opacity = age_opacity(
                clock.age_secs(point.timestamp),
                trail_config.solid_duration_seconds,
                trail_config.fade_duration_seconds,
            );

            if opacity <= 0.0 {
                prev_dir = None;
                prev_xy = None;
                prev_z = None;
                continue;
            }

            let xy = converter.latlon_to_world(point.lat, point.lon);
            let z = if is_3d {
                view3d_state.altitude_to_z(point.altitude.unwrap_or(0))
            } else {
                2.0
            };

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

            let base_color = altitude_color(point.altitude);
            let linear = base_color.to_linear();
            let base_half_width = if is_3d { TRAIL_HALF_WIDTH_3D } else { TRAIL_HALF_WIDTH_2D };
            let segment_estimated = point.estimated || prev_estimated;

            if segment_estimated && prev_xy.is_some() {
                let p_xy = prev_xy.unwrap();
                let p_z = prev_z.unwrap_or(z);
                let half_width = base_half_width * 0.5;

                emit_dashed_segment(
                    p_xy, p_z, xy, z, dir, half_width,
                    opacity, stale_opacity, &linear,
                    is_3d, &mut positions, &mut colors,
                );
            } else {
                let half_width = base_half_width;
                let perp = Vec2::new(-dir.y, dir.x) * half_width;

                let left_zup = Vec3::new(xy.x + perp.x, xy.y + perp.y, z);
                let right_zup = Vec3::new(xy.x - perp.x, xy.y - perp.y, z);

                let (left, right) = if is_3d {
                    (
                        Vec3::new(left_zup.x, left_zup.z, -left_zup.y),
                        Vec3::new(right_zup.x, right_zup.z, -right_zup.y),
                    )
                } else {
                    (left_zup, right_zup)
                };

                let alpha = opacity * stale_opacity;
                let color = [linear.red, linear.green, linear.blue, alpha];

                positions.push(left.into());
                positions.push(right.into());
                colors.push(color);
                colors.push(color);
            }

            prev_dir = Some(dir);
            prev_xy = Some(xy);
            prev_z = Some(z);
            prev_estimated = point.estimated;
        }

        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
}

const DASH_FRACTION: f32 = 0.55;
const DASH_COUNT: usize = 4;

fn emit_dashed_segment(
    from_xy: Vec2, from_z: f32,
    to_xy: Vec2, to_z: f32,
    dir: Vec2, half_width: f32,
    opacity: f32, stale_opacity: f32,
    linear: &bevy::color::LinearRgba,
    is_3d: bool,
    positions: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
) {
    let seg_len = (to_xy - from_xy).length();
    if seg_len < 0.1 {
        return;
    }

    let perp = Vec2::new(-dir.y, dir.x) * half_width;
    let step = 1.0 / DASH_COUNT as f32;
    let dash_alpha = opacity * stale_opacity * 0.5;
    let gap_alpha = 0.0;

    for d in 0..DASH_COUNT {
        let seg_start = d as f32 * step;
        let seg_dash_end = seg_start + step * DASH_FRACTION;
        let seg_end = seg_start + step;

        // Dash portion (visible)
        for &t in &[seg_start, seg_dash_end] {
            let xy = from_xy.lerp(to_xy, t);
            let z = from_z + (to_z - from_z) * t;

            let left_zup = Vec3::new(xy.x + perp.x, xy.y + perp.y, z);
            let right_zup = Vec3::new(xy.x - perp.x, xy.y - perp.y, z);

            let (left, right) = if is_3d {
                (
                    Vec3::new(left_zup.x, left_zup.z, -left_zup.y),
                    Vec3::new(right_zup.x, right_zup.z, -right_zup.y),
                )
            } else {
                (left_zup, right_zup)
            };

            let color = [linear.red, linear.green, linear.blue, dash_alpha];
            positions.push(left.into());
            positions.push(right.into());
            colors.push(color);
            colors.push(color);
        }

        // Gap portion (invisible) - degenerate bridge to next dash
        for &t in &[seg_dash_end, seg_end.min(1.0)] {
            let xy = from_xy.lerp(to_xy, t);
            let z = from_z + (to_z - from_z) * t;

            let left_zup = Vec3::new(xy.x + perp.x, xy.y + perp.y, z);
            let right_zup = Vec3::new(xy.x - perp.x, xy.y - perp.y, z);

            let (left, right) = if is_3d {
                (
                    Vec3::new(left_zup.x, left_zup.z, -left_zup.y),
                    Vec3::new(right_zup.x, right_zup.z, -right_zup.y),
                )
            } else {
                (left_zup, right_zup)
            };

            let color = [linear.red, linear.green, linear.blue, gap_alpha];
            positions.push(left.into());
            positions.push(right.into());
            colors.push(color);
            colors.push(color);
        }
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
