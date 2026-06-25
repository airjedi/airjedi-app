use bevy::prelude::*;
use bevy_slippy_tiles::*;

use super::staleness::{aircraft_age_secs, staleness_opacity};
use super::trails::{age_opacity, altitude_color, TrailRenderer};
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
    tile_settings: Res<SlippyTilesSettings>,
    map_state: Res<MapState>,
    view3d_state: Res<View3DState>,
    trail_query: Query<(&TrailHistory, &Aircraft)>,
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

    let zoom = view3d_state.effective_zoom(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, zoom);

    for (trail, aircraft) in trail_query.iter() {
        let stale_opacity = staleness_opacity(aircraft_age_secs(aircraft));

        if trail.points.len() < 2 {
            continue;
        }

        let mut prev_pos: Option<Vec3> = None;
        let mut prev_color: Option<Color> = None;
        let mut prev_estimated = false;

        for point in trail.points.iter() {
            let opacity = age_opacity(
                clock.age_secs(point.timestamp),
                config.solid_duration_seconds,
                config.fade_duration_seconds,
            );

            if opacity <= 0.0 {
                prev_pos = None;
                continue;
            }

            let xy = converter.latlon_to_world(point.lat, point.lon);
            let z = if is_3d {
                view3d_state.altitude_to_z(point.altitude.unwrap_or(0))
            } else {
                0.0
            };
            let pos = Vec3::new(xy.x, xy.y, z);

            let base_color = altitude_color(point.altitude);
            let segment_estimated = point.estimated || prev_estimated;
            let est_dim = if segment_estimated { 0.4 } else { 1.0 };
            let color = base_color.with_alpha(opacity * stale_opacity * est_dim);

            if let Some(prev) = prev_pos {
                let draw_color = prev_color.unwrap_or(color);
                if segment_estimated {
                    draw_dashed(prev, pos, draw_color, is_3d, &mut gizmos);
                } else if is_3d {
                    gizmos.line(prev, pos, draw_color);
                } else {
                    gizmos.line_2d(prev.truncate(), pos.truncate(), draw_color);
                }
            }

            prev_pos = Some(pos);
            prev_color = Some(color);
            prev_estimated = point.estimated;
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

/// System to prune old trail points
pub fn prune_trails(
    config: Res<TrailConfig>,
    clock: Res<SessionClock>,
    mut trail_query: Query<&mut TrailHistory>,
) {
    for mut trail in trail_query.iter_mut() {
        trail.prune(config.max_age_seconds, &clock);
    }
}
