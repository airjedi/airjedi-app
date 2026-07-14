use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use std::collections::HashSet;

use crate::render_layers::RenderCategory;
use crate::tiles::*;
use crate::geo::{haversine_distance_nm, CoordinateConverter};
use crate::constants;
use crate::MapState;
use crate::view3d::View3DState;
use super::{AirportFilter, AviationData, LoadingState};

/// Resource for runway rendering state
#[derive(Resource)]
pub struct RunwayRenderState {
    pub show_runways: bool,
}

impl Default for RunwayRenderState {
    fn default() -> Self {
        Self { show_runways: true }
    }
}

const FEET_TO_METERS_F32: f32 = 0.3048;
const RUNWAY_BODY_Z: f32 = 4.5;
const RUNWAY_LABEL_Z: f32 = 4.6;

#[derive(Component)]
pub struct RunwayBody {
    pub le_lat: f64,
    pub le_lon: f64,
    pub he_lat: f64,
    pub he_lon: f64,
    pub heading_deg: f64,
    pub midpoint_lat: f64,
    pub midpoint_lon: f64,
}

#[derive(Component)]
pub struct RunwayLabel {
    pub le_lat: f64,
    pub le_lon: f64,
    pub he_lat: f64,
    pub he_lon: f64,
    pub heading_deg: f64,
    pub is_he_end: bool,
    pub midpoint_lat: f64,
    pub midpoint_lon: f64,
}

pub fn heading_to_rotation(heading_deg: f64) -> f32 {
    -(heading_deg as f32).to_radians()
}

pub fn le_label_pos(le: Vec2, he: Vec2) -> Vec2 {
    le + (he - le) * 0.12
}

pub fn he_label_pos(le: Vec2, he: Vec2) -> Vec2 {
    he + (le - he) * 0.12
}

pub fn spawn_runway_entities(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    aviation_data: Res<AviationData>,
    render_state: Res<RunwayRenderState>,
    local_origin: Res<LocalOrigin>,
    existing: Query<(), With<RunwayBody>>,
) {
    if aviation_data.loading_state != LoadingState::Ready {
        return;
    }
    if !existing.is_empty() {
        return;
    }
    if !render_state.show_runways {
        return;
    }

    let scheduled_airports: HashSet<i64> = aviation_data
        .airports
        .iter()
        .filter(|a| a.passes_filter(AirportFilter::FrequentlyUsed))
        .map(|a| a.id)
        .collect();

    let open_mat = materials.add(ColorMaterial::from_color(
        Color::srgba(0.55, 0.55, 0.55, 1.0),
    ));
    let closed_mat = materials.add(ColorMaterial::from_color(
        Color::srgba(0.35, 0.35, 0.35, 0.7),
    ));

    let converter = CoordinateConverter::new(&local_origin);
    let mut count = 0;

    for runway in &aviation_data.runways {
        if !runway.has_valid_coords() {
            continue;
        }
        if !scheduled_airports.contains(&runway.airport_ref) {
            continue;
        }
        let Some(width_ft) = runway.width_ft else {
            continue;
        };
        let Some(heading) = runway.le_heading_deg_t else {
            continue;
        };

        let le_lat = runway.le_latitude_deg.unwrap();
        let le_lon = runway.le_longitude_deg.unwrap();
        let he_lat = runway.he_latitude_deg.unwrap();
        let he_lon = runway.he_longitude_deg.unwrap();
        let mid_lat = (le_lat + he_lat) / 2.0;
        let mid_lon = (le_lon + he_lon) / 2.0;

        let le_world = converter.latlon_to_world(le_lat, le_lon);
        let he_world = converter.latlon_to_world(he_lat, he_lon);
        let center = (le_world + he_world) / 2.0;
        let length_m = le_world.distance(he_world);

        if length_m < 1.0 {
            continue;
        }

        let width_m = (width_ft as f32) * FEET_TO_METERS_F32;
        let angle = heading_to_rotation(heading);
        let rotation = Quat::from_rotation_z(angle);
        let material = if runway.is_closed() {
            closed_mat.clone()
        } else {
            open_mat.clone()
        };
        let mesh = meshes.add(Rectangle::new(width_m, length_m));

        commands.spawn((
            RunwayBody {
                le_lat,
                le_lon,
                he_lat,
                he_lon,
                heading_deg: heading,
                midpoint_lat: mid_lat,
                midpoint_lon: mid_lon,
            },
            Mesh2d(mesh),
            MeshMaterial2d(material),
            Transform {
                translation: Vec3::new(center.x, center.y, RUNWAY_BODY_Z),
                rotation,
                ..default()
            },
            Visibility::Hidden,
            RenderLayers::layer(RenderCategory::OVERLAYS_2D),
        ));

        if let Some(le_ident) = &runway.le_ident {
            let lp = le_label_pos(le_world, he_world);
            commands.spawn((
                RunwayLabel {
                    le_lat,
                    le_lon,
                    he_lat,
                    he_lon,
                    heading_deg: heading,
                    is_he_end: false,
                    midpoint_lat: mid_lat,
                    midpoint_lon: mid_lon,
                },
                Text2d::new(le_ident.clone()),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                Transform {
                    translation: Vec3::new(lp.x, lp.y, RUNWAY_LABEL_Z),
                    rotation,
                    ..default()
                },
                Visibility::Hidden,
                RenderLayers::layer(RenderCategory::LABELS),
            ));
        }

        if let Some(he_ident) = &runway.he_ident {
            let hp = he_label_pos(le_world, he_world);
            let he_rotation = Quat::from_rotation_z(angle + std::f32::consts::PI);
            commands.spawn((
                RunwayLabel {
                    le_lat,
                    le_lon,
                    he_lat,
                    he_lon,
                    heading_deg: heading,
                    is_he_end: true,
                    midpoint_lat: mid_lat,
                    midpoint_lon: mid_lon,
                },
                Text2d::new(he_ident.clone()),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                Transform {
                    translation: Vec3::new(hp.x, hp.y, RUNWAY_LABEL_Z),
                    rotation: he_rotation,
                    ..default()
                },
                Visibility::Hidden,
                RenderLayers::layer(RenderCategory::LABELS),
            ));
        }

        count += 1;
    }

    info!("Spawned {} runway body entities", count);
}

pub fn update_runway_positions(
    local_origin: Res<LocalOrigin>,
    mut body_query: Query<(&RunwayBody, &mut Transform)>,
    mut label_query: Query<(&RunwayLabel, &mut Transform), Without<RunwayBody>>,
) {
    if !local_origin.is_changed() {
        return;
    }

    let converter = CoordinateConverter::new(&local_origin);

    for (body, mut transform) in body_query.iter_mut() {
        let le = converter.latlon_to_world(body.le_lat, body.le_lon);
        let he = converter.latlon_to_world(body.he_lat, body.he_lon);
        let center = (le + he) / 2.0;
        let angle = heading_to_rotation(body.heading_deg);
        transform.translation.x = center.x;
        transform.translation.y = center.y;
        transform.rotation = Quat::from_rotation_z(angle);
    }

    for (label, mut transform) in label_query.iter_mut() {
        let le = converter.latlon_to_world(label.le_lat, label.le_lon);
        let he = converter.latlon_to_world(label.he_lat, label.he_lon);
        let angle = heading_to_rotation(label.heading_deg);
        let pos = if label.is_he_end {
            he_label_pos(le, he)
        } else {
            le_label_pos(le, he)
        };
        let rotation = if label.is_he_end {
            Quat::from_rotation_z(angle + std::f32::consts::PI)
        } else {
            Quat::from_rotation_z(angle)
        };
        transform.translation.x = pos.x;
        transform.translation.y = pos.y;
        transform.rotation = rotation;
    }
}

pub fn update_runway_visibility(
    map_state: Res<MapState>,
    render_state: Res<RunwayRenderState>,
    view3d_state: Res<View3DState>,
    mut body_query: Query<(&RunwayBody, &mut Visibility)>,
    mut label_query: Query<(&RunwayLabel, &mut Visibility), Without<RunwayBody>>,
) {
    let zoom: u8 = map_state.zoom_level.to_u8();
    let show_bodies = render_state.show_runways
        && zoom >= 8
        && !view3d_state.is_3d_active();
    let show_labels = show_bodies && zoom >= 11;

    let center_lat = map_state.latitude;
    let center_lon = map_state.longitude;

    if !show_bodies {
        for (_, mut vis) in body_query.iter_mut() {
            *vis = Visibility::Hidden;
        }
        for (_, mut vis) in label_query.iter_mut() {
            *vis = Visibility::Hidden;
        }
        return;
    }

    for (body, mut vis) in body_query.iter_mut() {
        let in_range = (body.midpoint_lat - center_lat).abs()
            <= constants::AVIATION_FEATURE_BBOX_DEG
            && (body.midpoint_lon - center_lon).abs()
                <= constants::AVIATION_FEATURE_BBOX_DEG
            && haversine_distance_nm(
                center_lat,
                center_lon,
                body.midpoint_lat,
                body.midpoint_lon,
            ) <= constants::AVIATION_FEATURE_RADIUS_NM;
        *vis = if in_range {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }

    for (label, mut vis) in label_query.iter_mut() {
        if !show_labels {
            *vis = Visibility::Hidden;
            continue;
        }
        let in_range = (label.midpoint_lat - center_lat).abs()
            <= constants::AVIATION_FEATURE_BBOX_DEG
            && (label.midpoint_lon - center_lon).abs()
                <= constants::AVIATION_FEATURE_BBOX_DEG
            && haversine_distance_nm(
                center_lat,
                center_lon,
                label.midpoint_lat,
                label.midpoint_lon,
            ) <= constants::AVIATION_FEATURE_RADIUS_NM;
        *vis = if in_range {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

fn draw_dashed_2d(from: Vec2, to: Vec2, color: Color, dash_m: f32, gap_m: f32, gizmos: &mut Gizmos) {
    let delta = to - from;
    let total = delta.length();
    if total < 1.0 {
        return;
    }
    let dir = delta / total;
    let step = dash_m + gap_m;
    let mut t = 0.0f32;
    while t < total {
        let seg_end = (t + dash_m).min(total);
        gizmos.line_2d(from + dir * t, from + dir * seg_end, color);
        t += step;
    }
}

/// Draws dashed white centerlines over runway body meshes at zoom 10+.
/// The gray body rectangles are handled by RunwayBody mesh entities.
pub fn draw_runways(
    mut gizmos: Gizmos,
    aviation_data: Res<AviationData>,
    render_state: Res<RunwayRenderState>,
    local_origin: Res<LocalOrigin>,
    map_state: Res<MapState>,
    view3d_state: Res<View3DState>,
) {
    if aviation_data.loading_state != LoadingState::Ready {
        return;
    }
    if !render_state.show_runways {
        return;
    }
    if view3d_state.is_3d_active() {
        return;
    }

    let zoom: u8 = map_state.zoom_level.to_u8();
    if zoom < 10 {
        return;
    }

    let converter = CoordinateConverter::new(&local_origin);
    let center_lat = map_state.latitude;
    let center_lon = map_state.longitude;
    let centerline_color = Color::srgba(1.0, 1.0, 1.0, 0.85);

    for runway in &aviation_data.runways {
        if !runway.has_valid_coords() || runway.is_closed() {
            continue;
        }

        let le_lat = runway.le_latitude_deg.unwrap();
        let le_lon = runway.le_longitude_deg.unwrap();
        let he_lat = runway.he_latitude_deg.unwrap();
        let he_lon = runway.he_longitude_deg.unwrap();
        let mid_lat = (le_lat + he_lat) / 2.0;
        let mid_lon = (le_lon + he_lon) / 2.0;

        if (mid_lat - center_lat).abs() > constants::AVIATION_FEATURE_BBOX_DEG
            || (mid_lon - center_lon).abs() > constants::AVIATION_FEATURE_BBOX_DEG
        {
            continue;
        }
        if haversine_distance_nm(center_lat, center_lon, mid_lat, mid_lon)
            > constants::AVIATION_FEATURE_RADIUS_NM
        {
            continue;
        }

        let le = converter.latlon_to_world(le_lat, le_lon);
        let he = converter.latlon_to_world(he_lat, he_lon);
        let inset = (he - le) * 0.05;
        draw_dashed_2d(le + inset, he - inset, centerline_color, 15.0, 10.0, &mut gizmos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_to_rotation_north_is_zero() {
        assert!((heading_to_rotation(0.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn heading_to_rotation_east_is_neg_half_pi() {
        let expected = -std::f32::consts::FRAC_PI_2;
        assert!((heading_to_rotation(90.0) - expected).abs() < 1e-5);
    }

    #[test]
    fn heading_to_rotation_south_is_neg_pi() {
        let expected = -std::f32::consts::PI;
        assert!((heading_to_rotation(180.0) - expected).abs() < 1e-5);
    }

    #[test]
    fn le_label_pos_is_12_percent_inset() {
        let le = Vec2::new(0.0, 0.0);
        let he = Vec2::new(0.0, 1000.0);
        let pos = le_label_pos(le, he);
        assert!((pos.x).abs() < 1e-4);
        assert!((pos.y - 120.0).abs() < 1e-3);
    }

    #[test]
    fn he_label_pos_is_12_percent_inset_from_he() {
        let le = Vec2::new(0.0, 0.0);
        let he = Vec2::new(0.0, 1000.0);
        let pos = he_label_pos(le, he);
        assert!((pos.x).abs() < 1e-4);
        assert!((pos.y - 880.0).abs() < 1e-3);
    }
}
