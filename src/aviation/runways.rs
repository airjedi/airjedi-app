use bevy::prelude::*;
use crate::tiles::*;

use super::{AviationData, LoadingState};
use crate::constants;
use crate::geo::{haversine_distance_nm, CoordinateConverter};
use crate::MapState;

/// Component marking a runway entity
#[derive(Component)]
pub struct RunwayMarker {
    pub runway_id: i64,
    pub airport_ref: i64,
}

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
    pub runway_id: i64,
    pub le_lat: f64,
    pub le_lon: f64,
    pub he_lat: f64,
    pub he_lon: f64,
    pub heading_deg: f64,
    pub width_m: f32,
    pub midpoint_lat: f64,
    pub midpoint_lon: f64,
}

#[derive(Component)]
pub struct RunwayLabel {
    pub runway_id: i64,
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

const RUNWAY_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.7);

/// System to render runways using Gizmos
pub fn draw_runways(
    mut gizmos: Gizmos,
    aviation_data: Res<AviationData>,
    render_state: Res<RunwayRenderState>,
    local_origin: Res<LocalOrigin>,
    map_state: Res<MapState>,
    view3d_state: Res<crate::view3d::View3DState>,
) {
    if aviation_data.loading_state != LoadingState::Ready {
        return;
    }
    if !render_state.show_runways {
        return;
    }

    // Only show runways at zoom 8+
    let zoom: u8 = map_state.zoom_level.to_u8();
    if zoom < 8 {
        return;
    }

    let converter = CoordinateConverter::new(&local_origin);
    let is_3d = view3d_state.is_3d_active();
    let ground_z = view3d_state.altitude_to_z(view3d_state.ground_elevation_ft);

    let center_lat = map_state.latitude;
    let center_lon = map_state.longitude;

    for runway in &aviation_data.runways {
        if !runway.has_valid_coords() || runway.is_closed() {
            continue;
        }

        let le_lat = runway.le_latitude_deg.unwrap();
        let le_lon = runway.le_longitude_deg.unwrap();
        let he_lat = runway.he_latitude_deg.unwrap();
        let he_lon = runway.he_longitude_deg.unwrap();

        // Distance culling: skip runways beyond the visibility radius
        if (le_lat - center_lat).abs() > constants::AVIATION_FEATURE_BBOX_DEG
            || (le_lon - center_lon).abs() > constants::AVIATION_FEATURE_BBOX_DEG
        {
            continue;
        }
        if haversine_distance_nm(center_lat, center_lon, le_lat, le_lon)
            > constants::AVIATION_FEATURE_RADIUS_NM
        {
            continue;
        }

        let start = converter.latlon_to_world(le_lat, le_lon);
        let end = converter.latlon_to_world(he_lat, he_lon);

        if is_3d {
            gizmos.line(
                Vec3::new(start.x, start.y, ground_z),
                Vec3::new(end.x, end.y, ground_z),
                RUNWAY_COLOR,
            );
        } else {
            gizmos.line_2d(start, end, RUNWAY_COLOR);
        }
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
