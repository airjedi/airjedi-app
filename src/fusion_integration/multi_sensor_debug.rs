use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use airjedi_fusion::{Measurement, SensorObservation, TimelineStore};
use bevy::prelude::*;

use crate::aircraft::components::FusionTrackLink;
use crate::aircraft::{Aircraft, AircraftListState, CameraFollowState};
use crate::geo::CoordinateConverter;
use crate::tiles::LocalOrigin;

/// Debug overlay showing the raw, per-sensor position reports that feed a
/// fused track, so fusion behavior (association, filter convergence,
/// disagreement between overlapping feeds) can be inspected visually.
///
/// This is purely a visualization of data that already exists in
/// `TimelineStore` - it does not affect fusion itself.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct MultiSensorDebugConfig {
    pub enabled: bool,
    /// When false (default), only the selected/followed aircraft is shown, to
    /// avoid cluttering the map. When true, every aircraft with 2+ contributing
    /// sensors is shown.
    pub show_all_aircraft: bool,
}

impl Default for MultiSensorDebugConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            show_all_aircraft: false,
        }
    }
}

/// Deterministic per-sensor color derived from a hash of the sensor id, so the
/// same feed (e.g. "adsb-north") always renders the same hue across frames and
/// app restarts without a shared color registry.
pub fn sensor_color(sensor_id: &str, alpha: f32) -> Color {
    let mut hasher = DefaultHasher::new();
    sensor_id.hash(&mut hasher);
    let hue = (hasher.finish() % 360) as f32;
    Color::hsl(hue, 0.75, 0.55).with_alpha(alpha)
}

fn observation_lat_lon(obs: &SensorObservation) -> Option<(f64, f64)> {
    match &obs.measurement {
        Measurement::PositionVelocity3D {
            lat_deg, lon_deg, ..
        }
        | Measurement::PositionVelocity2D {
            lat_deg, lon_deg, ..
        } => Some((*lat_deg, *lon_deg)),
        _ => None,
    }
}

/// Draw a marker at each contributing sensor's latest raw (pre-fusion) position,
/// with a line to the current fused aircraft position, so the spread between
/// independent sensor reports and the fused estimate is visible at a glance.
pub fn draw_multi_sensor_sources(
    mut gizmos: Gizmos,
    config: Res<MultiSensorDebugConfig>,
    list_state: Res<AircraftListState>,
    follow_state: Res<CameraFollowState>,
    local_origin: Res<LocalOrigin>,
    timeline_store: Res<TimelineStore>,
    visuals: Query<(&FusionTrackLink, &Aircraft)>,
) {
    if !config.enabled {
        return;
    }

    let selected_icao = follow_state
        .following_icao
        .as_ref()
        .or(list_state.selected_icao.as_ref());

    if !config.show_all_aircraft && selected_icao.is_none() {
        return;
    }

    let converter = CoordinateConverter::new(&local_origin);

    for (link, aircraft) in visuals.iter() {
        if !config.show_all_aircraft && Some(&aircraft.icao) != selected_icao {
            continue;
        }

        let sources = timeline_store.latest_per_sensor(&link.track_id);
        // A single contributing sensor has nothing to disagree with - skip it
        // rather than drawing a redundant marker on top of the aircraft icon.
        if sources.len() < 2 {
            continue;
        }

        let fused_pos = converter.latlon_to_world(aircraft.latitude, aircraft.longitude);

        let mut sensor_ids: Vec<&String> = sources.keys().collect();
        sensor_ids.sort();

        for sensor_id in sensor_ids {
            let stored = sources[sensor_id];
            let Some((lat, lon)) = observation_lat_lon(&stored.observation) else {
                continue;
            };
            let pos = converter.latlon_to_world(lat, lon);
            let color = sensor_color(sensor_id, 0.85);

            gizmos.circle_2d(pos, 30.0, color);
            gizmos.line_2d(pos, fused_pos, color.with_alpha(0.35));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensor_color_is_deterministic() {
        let a = sensor_color("adsb-north", 1.0);
        let b = sensor_color("adsb-north", 1.0);
        assert_eq!(a.to_srgba(), b.to_srgba());
    }

    #[test]
    fn sensor_color_differs_across_sensors() {
        let a = sensor_color("adsb-north", 1.0);
        let b = sensor_color("adsb-south", 1.0);
        assert_ne!(a.to_srgba(), b.to_srgba());
    }
}
