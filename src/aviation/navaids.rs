use bevy::prelude::*;
use bevy_slippy_tiles::*;

use super::{AviationData, LoadingState, NavaidType};
use crate::MapState;
use crate::constants;
use crate::geo::{CoordinateConverter, haversine_distance_nm};

/// Component marking a navaid entity
#[derive(Component)]
pub struct NavaidMarker {
    pub navaid_id: i64,
}

/// Resource for navaid rendering state
#[derive(Resource)]
pub struct NavaidRenderState {
    pub show_navaids: bool,
}

impl Default for NavaidRenderState {
    fn default() -> Self {
        Self { show_navaids: false } // Off by default
    }
}

/// System to render navaids using Gizmos
pub fn draw_navaids(
    mut gizmos: Gizmos,
    aviation_data: Res<AviationData>,
    render_state: Res<NavaidRenderState>,
    tile_settings: Res<SlippyTilesSettings>,
    map_state: Res<MapState>,
    view3d_state: Res<crate::view3d::View3DState>,
) {
    if aviation_data.loading_state != LoadingState::Ready {
        return;
    }
    if !render_state.show_navaids {
        return;
    }

    // Only show navaids at zoom 7+
    let zoom: u8 = map_state.zoom_level.to_u8();
    if zoom < 7 {
        return;
    }

    let render_zoom = view3d_state.effective_zoom(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, render_zoom);
    let is_3d = view3d_state.is_3d_active();
    let ground_z = view3d_state.altitude_to_z(view3d_state.ground_elevation_ft);

    let center_lat = map_state.latitude;
    let center_lon = map_state.longitude;

    for navaid in &aviation_data.navaids {
        // Distance culling: skip navaids beyond the visibility radius
        if (navaid.latitude_deg - center_lat).abs() > constants::AVIATION_FEATURE_BBOX_DEG
            || (navaid.longitude_deg - center_lon).abs() > constants::AVIATION_FEATURE_BBOX_DEG
        {
            continue;
        }
        if haversine_distance_nm(center_lat, center_lon, navaid.latitude_deg, navaid.longitude_deg)
            > constants::AVIATION_FEATURE_RADIUS_NM
        {
            continue;
        }

        let pos = converter.latlon_to_world(navaid.latitude_deg, navaid.longitude_deg);

        let color = navaid.color();
        let size = 4.0;

        if is_3d {
            let pos3 = Vec3::new(pos.x, pos.y, ground_z);
            // In 3D mode, draw using 3D gizmo primitives at ground elevation
            match navaid.navaid_type {
                NavaidType::Vor | NavaidType::VorDme | NavaidType::Vortac => {
                    gizmos.circle(Isometry3d::new(pos3, Quat::IDENTITY), size, color);
                    for angle in [0.0_f32, 90.0, 180.0, 270.0] {
                        let rad = angle.to_radians();
                        let end = pos3 + Vec3::new(rad.cos(), rad.sin(), 0.0) * (size + 3.0);
                        gizmos.line(pos3, end, color);
                    }
                }
                NavaidType::Ndb | NavaidType::NdbDme => {
                    let points = [
                        pos3 + Vec3::new(0.0, size, 0.0),
                        pos3 + Vec3::new(size, 0.0, 0.0),
                        pos3 + Vec3::new(0.0, -size, 0.0),
                        pos3 + Vec3::new(-size, 0.0, 0.0),
                        pos3 + Vec3::new(0.0, size, 0.0),
                    ];
                    for i in 0..4 {
                        gizmos.line(points[i], points[i + 1], color);
                    }
                }
                NavaidType::Dme | NavaidType::Tacan => {
                    let half = size / 2.0;
                    let corners = [
                        pos3 + Vec3::new(-half, -half, 0.0),
                        pos3 + Vec3::new(half, -half, 0.0),
                        pos3 + Vec3::new(half, half, 0.0),
                        pos3 + Vec3::new(-half, half, 0.0),
                        pos3 + Vec3::new(-half, -half, 0.0),
                    ];
                    for i in 0..4 {
                        gizmos.line(corners[i], corners[i + 1], color);
                    }
                }
                NavaidType::Unknown => {
                    gizmos.circle(Isometry3d::new(pos3, Quat::IDENTITY), 2.0, color);
                }
            }
        } else {
            match navaid.navaid_type {
                NavaidType::Vor | NavaidType::VorDme | NavaidType::Vortac => {
                    gizmos.circle_2d(pos, size, color);
                    for angle in [0.0_f32, 90.0, 180.0, 270.0] {
                        let rad = angle.to_radians();
                        let end = pos + Vec2::new(rad.cos(), rad.sin()) * (size + 3.0);
                        gizmos.line_2d(pos, end, color);
                    }
                }
                NavaidType::Ndb | NavaidType::NdbDme => {
                    let points = [
                        pos + Vec2::new(0.0, size),
                        pos + Vec2::new(size, 0.0),
                        pos + Vec2::new(0.0, -size),
                        pos + Vec2::new(-size, 0.0),
                        pos + Vec2::new(0.0, size),
                    ];
                    for i in 0..4 {
                        gizmos.line_2d(points[i], points[i + 1], color);
                    }
                }
                NavaidType::Dme | NavaidType::Tacan => {
                    let half = size / 2.0;
                    let corners = [
                        pos + Vec2::new(-half, -half),
                        pos + Vec2::new(half, -half),
                        pos + Vec2::new(half, half),
                        pos + Vec2::new(-half, half),
                        pos + Vec2::new(-half, -half),
                    ];
                    for i in 0..4 {
                        gizmos.line_2d(corners[i], corners[i + 1], color);
                    }
                }
                NavaidType::Unknown => {
                    gizmos.circle_2d(pos, 2.0, color);
                }
            }
        }
    }
}
