use crate::tiles::*;
use bevy::prelude::*;

use super::list_panel::AircraftListState;
use super::staleness::{aircraft_age_secs, staleness_opacity};
use super::trails::{altitude_color, contiguous_trail_pairs, point_opacity, TrailRenderer};
use super::{SessionClock, TrailConfig, TrailHistory};
use crate::geo::CoordinateConverter;
use crate::view3d::View3DState;
use crate::{Aircraft, MapState};

/// System to draw flight trails using Gizmos.
/// In 2D mode, draws flat trails. In 3D mode, draws trails at altitude using Vec3 positions.
/// Skips drawing when the active renderer for the current mode is not Gizmo.
pub fn draw_trails(
    mut gizmos: Gizmos,
    config: Res<TrailConfig>,
    clock: Res<SessionClock>,
    local_origin: Res<LocalOrigin>,
    map_state: Res<MapState>,
    view3d_state: Res<View3DState>,
    trail_query: Query<(&TrailHistory, &Aircraft)>,
    list_state: Res<AircraftListState>,
) {
    if !config.enabled {
        return;
    }

    let is_3d = view3d_state.is_3d_active();
    let active_renderer = if is_3d {
        config.renderer_3d
    } else {
        config.renderer_2d
    };
    if active_renderer != TrailRenderer::Gizmo {
        return;
    }

    // Gizmo trails draw in both 2D and 3D modes. In 3D, they render as
    // an overlay through Camera2d on the GIZMOS layer.

    let converter = CoordinateConverter::new(&local_origin);

    for (trail, aircraft) in trail_query.iter() {
        let stale_opacity = staleness_opacity(aircraft_age_secs(aircraft));
        let is_selected = list_state.selected_icao.as_ref() == Some(&aircraft.icao);

        for (previous, point) in
            contiguous_trail_pairs(&trail.points, &clock, &config, is_selected, is_3d)
        {
            let previous_xy = converter.latlon_to_world(previous.lat, previous.lon);
            let point_xy = converter.latlon_to_world(point.lat, point.lon);
            let previous_z = if is_3d {
                view3d_state.altitude_to_z(previous.altitude.expect("3D pairs have altitude"))
            } else {
                0.0
            };
            let point_z = if is_3d {
                view3d_state.altitude_to_z(point.altitude.expect("3D pairs have altitude"))
            } else {
                0.0
            };
            let previous_pos = Vec3::new(previous_xy.x, previous_xy.y, previous_z);
            let point_pos = Vec3::new(point_xy.x, point_xy.y, point_z);

            let opacity = point_opacity(previous, &clock, &config, is_selected).min(point_opacity(
                point,
                &clock,
                &config,
                is_selected,
            ));
            let estimated = previous.estimated || point.estimated;
            let estimate_dim = if estimated { 0.4 } else { 1.0 };
            let color = altitude_color(previous.altitude)
                .with_alpha(opacity * stale_opacity * estimate_dim);

            if estimated {
                draw_dashed(previous_pos, point_pos, color, is_3d, &mut gizmos);
            } else if is_3d {
                gizmos.line(previous_pos, point_pos, color);
            } else {
                gizmos.line_2d(previous_pos.truncate(), point_pos.truncate(), color);
            }
        }
    }
}

/// Draw a dashed line segment between two points.
/// Alternates between visible (60%) and gap (40%) along the segment.
fn draw_dashed(from: Vec3, to: Vec3, color: Color, is_3d: bool, gizmos: &mut Gizmos) {
    let dir = to - from;
    let length = dir.length();
    if length < 0.1 {
        return;
    }

    let dash_len = 8.0_f32.min(length * 0.3);
    let gap_len = dash_len * 0.65;
    let step = dash_len + gap_len;
    let norm = dir / length;

    let mut t = 0.0;
    while t < length {
        let seg_start = from + norm * t;
        let seg_end = from + norm * (t + dash_len).min(length);
        if is_3d {
            gizmos.line(seg_start, seg_end, color);
        } else {
            gizmos.line_2d(seg_start.truncate(), seg_end.truncate(), color);
        }
        t += step;
    }
}
