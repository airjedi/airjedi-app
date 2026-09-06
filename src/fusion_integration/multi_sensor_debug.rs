use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use airjedi_core::{SensorContributions, SensorReport};
use airjedi_fusion::{Measurement, SensorObservation, TimelineStore, Track};
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

/// Agent-side projection: write each fused track's latest per-sensor raw
/// positions into the serializable [`SensorContributions`] component on the
/// track entity. This mirrors exactly what `draw_multi_sensor_sources` reads
/// from `TimelineStore` today; the drawer migrates to read the component (and
/// this becomes the sole `TimelineStore` reader) during the reader cutover.
pub fn sync_sensor_contributions(
    mut commands: Commands,
    timeline_store: Res<TimelineStore>,
    tracks: Query<(Entity, &Track)>,
) {
    for (entity, track) in &tracks {
        let latest = timeline_store.latest_per_sensor(&track.id);

        let mut sources: Vec<SensorReport> = latest
            .iter()
            .filter_map(|(sensor_id, stored)| {
                let (lat, lon) = observation_lat_lon(&stored.observation)?;
                Some(SensorReport {
                    sensor_id: sensor_id.clone(),
                    lat,
                    lon,
                    kind: stored.observation.sensor_id.kind,
                })
            })
            .collect();
        // Deterministic order so the component is stable frame to frame (and
        // for snapshot tests), since the source map has no inherent ordering.
        sources.sort_by(|a, b| a.sensor_id.cmp(&b.sensor_id));

        commands
            .entity(entity)
            .insert(SensorContributions { sources });
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
    contributions: Query<&SensorContributions>,
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

        let Ok(contrib) = contributions.get(link.track_entity) else {
            continue;
        };
        // A single contributing sensor has nothing to disagree with - skip it
        // rather than drawing a redundant marker on top of the aircraft icon.
        if contrib.sources.len() < 2 {
            continue;
        }

        let fused_pos = converter.latlon_to_world(aircraft.latitude, aircraft.longitude);

        // `sources` is already sorted by sensor id in the writer.
        for report in &contrib.sources {
            let pos = converter.latlon_to_world(report.lat, report.lon);
            let color = sensor_color(&report.sensor_id, 0.85);

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
