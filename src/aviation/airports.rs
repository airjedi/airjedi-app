use bevy::prelude::*;
use bevy_slippy_tiles::*;

use super::{Airport, AirportFilter, AviationData, LoadingState};
use crate::MapState;
use crate::constants;
use crate::geo::{CoordinateConverter, haversine_distance_nm};

/// Component marking an airport entity
#[derive(Component)]
pub struct AirportMarker {
    pub airport_id: i64,
}

/// Component for airport labels
#[derive(Component)]
pub struct AirportLabel {
    pub airport_entity: Entity,
}

/// Resource for airport rendering state
#[derive(Resource)]
pub struct AirportRenderState {
    pub show_airports: bool,
    pub filter: AirportFilter,
    /// Viewport bounds for culling (min_lat, max_lat, min_lon, max_lon)
    pub viewport_bounds: Option<(f64, f64, f64, f64)>,
}

impl Default for AirportRenderState {
    fn default() -> Self {
        Self {
            show_airports: true,
            filter: AirportFilter::FrequentlyUsed,
            viewport_bounds: None,
        }
    }
}

/// Z-layer for airports (below aircraft, above tiles)
const AIRPORT_Z_LAYER: f32 = 5.0;

/// System to spawn airport entities when data is ready
pub fn spawn_airports(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    aviation_data: Res<AviationData>,
    render_state: Res<AirportRenderState>,
    tile_settings: Res<SlippyTilesSettings>,
    map_state: Res<MapState>,
    view3d_state: Res<crate::view3d::View3DState>,
    existing_airports: Query<Entity, With<AirportMarker>>,
) {
    // Only run when data is ready and no airports exist yet
    if aviation_data.loading_state != LoadingState::Ready {
        return;
    }
    if !existing_airports.is_empty() {
        return;
    }
    if !render_state.show_airports {
        return;
    }

    info!("Spawning airport markers...");

    let zoom = view3d_state.effective_zoom(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, zoom);

    let mut count = 0;
    for airport in &aviation_data.airports {
        if !airport.passes_filter(render_state.filter) {
            continue;
        }

        let pos = converter.latlon_to_world(airport.latitude_deg, airport.longitude_deg);
        let x = pos.x;
        let y = pos.y;

        // Create airport marker (small square)
        let mesh = meshes.add(Rectangle::new(6.0, 6.0));
        let material = materials.add(ColorMaterial::from_color(airport.color()));

        commands.spawn((
            AirportMarker {
                airport_id: airport.id,
            },
            Mesh2d(mesh),
            MeshMaterial2d(material),
            Transform::from_xyz(x, y, AIRPORT_Z_LAYER),
            Visibility::Inherited,
        ));

        count += 1;
    }

    info!("Spawned {} airport markers", count);
}

/// System to update airport positions when map moves
pub fn update_airport_positions(
    tile_settings: Res<SlippyTilesSettings>,
    map_state: Res<MapState>,
    aviation_data: Res<AviationData>,
    view3d_state: Res<crate::view3d::View3DState>,
    mut airport_query: Query<(&AirportMarker, &mut Transform)>,
) {
    if aviation_data.loading_state != LoadingState::Ready {
        return;
    }

    let zoom = view3d_state.effective_zoom(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, zoom);

    // In 3D mode, raise airport markers to ground elevation
    let z = if view3d_state.is_3d_active() {
        view3d_state.altitude_to_z(view3d_state.ground_elevation_ft)
    } else {
        AIRPORT_Z_LAYER
    };

    // Build a lookup map for airports
    let airport_map: std::collections::HashMap<i64, &Airport> = aviation_data
        .airports
        .iter()
        .map(|a| (a.id, a))
        .collect();

    for (marker, mut transform) in airport_query.iter_mut() {
        if let Some(airport) = airport_map.get(&marker.airport_id) {
            let pos = converter.latlon_to_world(airport.latitude_deg, airport.longitude_deg);
            transform.translation.x = pos.x;
            transform.translation.y = pos.y;
            transform.translation.z = z;
        }
    }
}

/// System to toggle airport visibility based on zoom level and distance from camera
pub fn update_airport_visibility(
    map_state: Res<MapState>,
    render_state: Res<AirportRenderState>,
    aviation_data: Res<AviationData>,
    mut airport_query: Query<(&AirportMarker, &mut Visibility)>,
) {
    let zoom: u8 = map_state.zoom_level.to_u8();
    let zoom_ok = render_state.show_airports && zoom >= 6;

    if !zoom_ok {
        for (_, mut visibility) in airport_query.iter_mut() {
            *visibility = Visibility::Hidden;
        }
        return;
    }

    let center_lat = map_state.latitude;
    let center_lon = map_state.longitude;

    // Build a lookup map for airport coordinates
    let airport_map: std::collections::HashMap<i64, &Airport> = aviation_data
        .airports
        .iter()
        .map(|a| (a.id, a))
        .collect();

    for (marker, mut visibility) in airport_query.iter_mut() {
        let in_range = if let Some(airport) = airport_map.get(&marker.airport_id) {
            (airport.latitude_deg - center_lat).abs() <= constants::AVIATION_FEATURE_BBOX_DEG
                && (airport.longitude_deg - center_lon).abs()
                    <= constants::AVIATION_FEATURE_BBOX_DEG
                && haversine_distance_nm(
                    center_lat,
                    center_lon,
                    airport.latitude_deg,
                    airport.longitude_deg,
                ) <= constants::AVIATION_FEATURE_RADIUS_NM
        } else {
            false
        };

        *visibility = if in_range {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}
