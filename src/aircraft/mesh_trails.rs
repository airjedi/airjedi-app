use crate::tiles::LocalOrigin;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;

use super::components::Aircraft;
use super::staleness::{aircraft_age_secs, staleness_opacity};
use super::trails::{
    altitude_color, contiguous_trail_pairs, point_opacity, SessionClock, TrailConfig, TrailHistory,
    TrailRenderer,
};
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

// Trail half-widths are now set via TrailConfig and scaled by meters_per_tile_pixel

pub fn spawn_mesh_trails(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    trail_config: Res<TrailConfig>,
    view3d_state: Res<View3DState>,
    aircraft_query: Query<Entity, (With<Aircraft>, Without<MeshTrailMarker>)>,
) {
    let is_3d = view3d_state.is_3d_active();
    let active_renderer = if is_3d {
        trail_config.renderer_3d
    } else {
        trail_config.renderer_2d
    };
    if !trail_config.enabled || active_renderer != TrailRenderer::MeshStrip {
        return;
    }

    for aircraft_entity in aircraft_query.iter() {
        let mesh = Mesh::new(
            // Each visible segment owns its two triangles. A single triangle
            // strip would connect the first vertex of a new segment to the
            // previous segment even after local iteration state was reset.
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        let mesh_handle = meshes.add(mesh);

        let material = StandardMaterial {
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        };
        let material_handle = materials.add(material);

        let trail_entity = commands
            .spawn((
                Mesh3d(mesh_handle.clone()),
                MeshMaterial3d(material_handle.clone()),
                Transform::default(),
                MeshTrailEffect {
                    aircraft_entity,
                    mesh_handle,
                    material_handle,
                },
            ))
            .id();

        commands.entity(aircraft_entity).insert(MeshTrailMarker);

        let _ = trail_entity;
    }
}

pub fn update_mesh_trails(
    local_origin: Res<LocalOrigin>,
    map_state: Res<MapState>,
    view3d_state: Res<View3DState>,
    trail_config: Res<TrailConfig>,
    clock: Res<SessionClock>,
    mut meshes: ResMut<Assets<Mesh>>,
    aircraft_query: Query<(&TrailHistory, &Aircraft)>,
    effect_query: Query<&MeshTrailEffect>,
    list_state: Res<crate::aircraft::list_panel::AircraftListState>,
) {
    if !trail_config.enabled {
        return;
    }

    let is_3d = view3d_state.is_3d_active();
    let active_renderer = if is_3d {
        trail_config.renderer_3d
    } else {
        trail_config.renderer_2d
    };
    if active_renderer != TrailRenderer::MeshStrip {
        return;
    }

    let converter = CoordinateConverter::new(&local_origin);
    let (min_trail_altitude_3d, width_scale_3d) = if is_3d {
        let ground_y = view3d_state.altitude_to_z(view3d_state.ground_elevation_ft);
        let distance = view3d_state.altitude_to_distance();
        (
            ground_y + (distance * 0.0015).clamp(40.0, 400.0),
            (distance / 120_000.0).clamp(1.0, 1.8),
        )
    } else {
        (0.0, 1.0)
    };

    // Scale trail width by meters_per_tile_pixel so trails have consistent screen size
    let tile_size_meters =
        (2.0 * crate::tiles::WEB_MERCATOR_EXTENT) / (1u64 << map_state.zoom_level.to_u8()) as f64;
    let meters_per_tile_pixel =
        (tile_size_meters / crate::constants::DEFAULT_TILE_PIXELS as f64) as f32;

    for effect in effect_query.iter() {
        let Ok((trail, aircraft)) = aircraft_query.get(effect.aircraft_entity) else {
            if let Some(mut mesh) = meshes.get_mut(&effect.mesh_handle) {
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, Vec::<[f32; 4]>::new());
            }
            continue;
        };

        let Some(mut mesh) = meshes.get_mut(&effect.mesh_handle) else {
            continue;
        };

        let stale_opacity = staleness_opacity(aircraft_age_secs(aircraft));
        let is_selected = list_state.selected_icao.as_ref() == Some(&aircraft.icao);

        let mut positions: Vec<[f32; 3]> = Vec::with_capacity(trail.points.len() * 6);
        let mut colors: Vec<[f32; 4]> = Vec::with_capacity(trail.points.len() * 6);

        for (from, to) in
            contiguous_trail_pairs(&trail.points, &clock, &trail_config, is_selected, is_3d)
        {
            let from_xy = converter.latlon_to_world(from.lat, from.lon);
            let to_xy = converter.latlon_to_world(to.lat, to.lon);
            let direction = to_xy - from_xy;
            let dir = if direction.length_squared() > 0.0001 {
                direction.normalize()
            } else {
                Vec2::Y
            };
            let from_z = if is_3d {
                view3d_state
                    .altitude_to_z(from.altitude.expect("3D pairs have altitude"))
                    .max(min_trail_altitude_3d)
            } else {
                2.0
            };
            let to_z = if is_3d {
                view3d_state
                    .altitude_to_z(to.altitude.expect("3D pairs have altitude"))
                    .max(min_trail_altitude_3d)
            } else {
                2.0
            };
            let opacity = point_opacity(from, &clock, &trail_config, is_selected)
                .min(point_opacity(to, &clock, &trail_config, is_selected));
            let linear = altitude_color(from.altitude).to_linear();
            let base_half_width = if is_3d {
                trail_config.trail_width_3d * width_scale_3d / 2.0
            } else {
                trail_config.trail_width_2d * meters_per_tile_pixel / 2.0
            };

            if from.estimated || to.estimated {
                emit_dashed_segment(
                    from_xy,
                    from_z,
                    to_xy,
                    to_z,
                    dir,
                    base_half_width * 0.5,
                    opacity,
                    stale_opacity,
                    &linear,
                    is_3d,
                    &mut positions,
                    &mut colors,
                );
            } else {
                emit_quad(
                    from_xy,
                    from_z,
                    to_xy,
                    to_z,
                    dir,
                    base_half_width,
                    opacity * stale_opacity,
                    &linear,
                    is_3d,
                    &mut positions,
                    &mut colors,
                );
            }
        }

        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
}

const DASH_FRACTION: f32 = 0.55;
const DASH_COUNT: usize = 4;

fn emit_dashed_segment(
    from_xy: Vec2,
    from_z: f32,
    to_xy: Vec2,
    to_z: f32,
    dir: Vec2,
    half_width: f32,
    opacity: f32,
    stale_opacity: f32,
    linear: &bevy::color::LinearRgba,
    is_3d: bool,
    positions: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
) {
    let seg_len = (to_xy - from_xy).length();
    if seg_len < 0.1 {
        return;
    }

    let step = 1.0 / DASH_COUNT as f32;
    let dash_alpha = opacity * stale_opacity * 0.5;

    for d in 0..DASH_COUNT {
        let seg_start = d as f32 * step;
        let seg_dash_end = seg_start + step * DASH_FRACTION;

        emit_quad(
            from_xy.lerp(to_xy, seg_start),
            from_z + (to_z - from_z) * seg_start,
            from_xy.lerp(to_xy, seg_dash_end.min(1.0)),
            from_z + (to_z - from_z) * seg_dash_end.min(1.0),
            dir,
            half_width,
            dash_alpha,
            linear,
            is_3d,
            positions,
            colors,
        );
    }
}

fn emit_quad(
    from_xy: Vec2,
    from_z: f32,
    to_xy: Vec2,
    to_z: f32,
    dir: Vec2,
    half_width: f32,
    alpha: f32,
    linear: &bevy::color::LinearRgba,
    is_3d: bool,
    positions: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
) {
    let perp = Vec2::new(-dir.y, dir.x) * half_width;
    let left_from: [f32; 3] = trail_vertex(from_xy + perp, from_z, is_3d).into();
    let right_from: [f32; 3] = trail_vertex(from_xy - perp, from_z, is_3d).into();
    let left_to: [f32; 3] = trail_vertex(to_xy + perp, to_z, is_3d).into();
    let right_to: [f32; 3] = trail_vertex(to_xy - perp, to_z, is_3d).into();
    let color = [linear.red, linear.green, linear.blue, alpha];

    // Two independent triangles make each segment topologically separate.
    positions.extend([
        left_from, right_from, left_to, right_from, right_to, left_to,
    ]);
    colors.extend([color; 6]);
}

fn trail_vertex(xy: Vec2, z: f32, is_3d: bool) -> Vec3 {
    let zup = Vec3::new(xy.x, xy.y, z);
    if is_3d {
        Vec3::new(zup.x, zup.z, -zup.y)
    } else {
        zup
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
    let active_renderer = if is_3d {
        trail_config.renderer_3d
    } else {
        trail_config.renderer_2d
    };
    let inactive = active_renderer != TrailRenderer::MeshStrip || !trail_config.enabled;

    for (effect_entity, effect) in effect_query.iter() {
        let aircraft_gone = aircraft_query.get(effect.aircraft_entity).is_err();
        if inactive || aircraft_gone {
            commands.entity(effect_entity).despawn();
            if !aircraft_gone {
                commands
                    .entity(effect.aircraft_entity)
                    .remove::<MeshTrailMarker>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_list_emits_two_triangles_without_bridge_vertices() {
        let mut positions = Vec::new();
        let mut colors = Vec::new();
        let color = Color::WHITE.to_linear();

        emit_quad(
            Vec2::ZERO,
            0.0,
            Vec2::X,
            0.0,
            Vec2::X,
            0.1,
            1.0,
            &color,
            false,
            &mut positions,
            &mut colors,
        );
        emit_quad(
            Vec2::new(10.0, 0.0),
            0.0,
            Vec2::new(11.0, 0.0),
            0.0,
            Vec2::X,
            0.1,
            1.0,
            &color,
            false,
            &mut positions,
            &mut colors,
        );

        assert_eq!(positions.len(), 12);
        assert_eq!(colors.len(), positions.len());
        assert_ne!(positions[5], positions[6]);
    }

    #[test]
    fn dashed_segments_do_not_emit_invisible_bridge_geometry() {
        let mut positions = Vec::new();
        let mut colors = Vec::new();
        let color = Color::WHITE.to_linear();

        emit_dashed_segment(
            Vec2::ZERO,
            0.0,
            Vec2::new(100.0, 0.0),
            0.0,
            Vec2::X,
            1.0,
            1.0,
            1.0,
            &color,
            false,
            &mut positions,
            &mut colors,
        );

        assert_eq!(positions.len(), DASH_COUNT * 6);
        assert!(colors.iter().all(|color| color[3] > 0.0));
    }
}
