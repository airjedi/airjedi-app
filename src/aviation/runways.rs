use bevy::prelude::*;
use bevy_slippy_tiles::*;

use super::{AviationData, LoadingState};
use crate::MapState;
use crate::constants;
use crate::geo::{CoordinateConverter, haversine_distance_nm};

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

const RUNWAY_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.7);

/// System to render runways using Gizmos
pub fn draw_runways(
    mut gizmos: Gizmos,
    aviation_data: Res<AviationData>,
    render_state: Res<RunwayRenderState>,
    tile_settings: Res<SlippyTilesSettings>,
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

    let render_zoom = view3d_state.effective_zoom(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, render_zoom);
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
