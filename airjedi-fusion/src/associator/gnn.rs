use crate::associator::spatial_index::SpatialIndex;
use crate::associator::{Assignment, AssociationResult, AssociatorConfig};
use crate::classification::TargetClassification;
use crate::coord;
use crate::filter::TrackerState;
use crate::sensor::{Measurement, SensorObservation};
use crate::store::StoredObservation;
use crate::track::Track;
use crate::types::TrackId;
use std::collections::{HashMap, HashSet};

pub struct GnnAssociator;

impl GnnAssociator {
    pub fn associate(
        observations: &[&StoredObservation],
        tracks: &[(&Track, &TrackerState, &TargetClassification)],
        spatial_index: &SpatialIndex,
        config: &AssociatorConfig,
    ) -> AssociationResult {
        if observations.is_empty() || tracks.is_empty() {
            return AssociationResult {
                assignments: Vec::new(),
                unassigned_observations: (0..observations.len()).collect(),
                unassigned_tracks: (0..tracks.len()).collect(),
            };
        }

        let track_id_to_idx: HashMap<&TrackId, usize> = tracks
            .iter()
            .enumerate()
            .map(|(i, (t, _, _))| (&t.id, i))
            .collect();

        // Build cost matrix in two passes:
        // 1. Cooperative ID matches (ICAO, MMSI, etc.) - forced association, no gating
        // 2. Spatial pre-filter + Mahalanobis gating for non-cooperative targets
        let mut costs: Vec<(usize, usize, f64)> = Vec::new();
        let mut id_matched_obs: HashSet<usize> = HashSet::new();
        let mut id_matched_tracks: HashSet<usize> = HashSet::new();

        // Pass 1: cooperative ID matches are deterministic - same ID = same target
        for (obs_idx, obs) in observations.iter().enumerate() {
            if let Some(ref target_id) = obs.observation.target_id {
                for (track_idx, (track, _, _)) in tracks.iter().enumerate() {
                    if track
                        .cooperative_ids
                        .iter()
                        .any(|cid| cid.id == target_id.id)
                    {
                        costs.push((obs_idx, track_idx, 0.0));
                        id_matched_obs.insert(obs_idx);
                        id_matched_tracks.insert(track_idx);
                        break;
                    }
                }
            }
        }

        // Pass 2: spatial + statistical gating for observations without ID matches
        for (obs_idx, obs) in observations.iter().enumerate() {
            if id_matched_obs.contains(&obs_idx) {
                continue;
            }

            let obs_pos = observation_geodetic_position(&obs.observation);
            let (obs_lat, obs_lon) = match obs_pos {
                Some(pos) => pos,
                None => continue,
            };

            let nearby = spatial_index.nearby_tracks(obs_lat, obs_lon);

            for nearby_track_id in &nearby {
                let track_idx = match track_id_to_idx.get(nearby_track_id) {
                    Some(&idx) => idx,
                    None => continue,
                };

                if id_matched_tracks.contains(&track_idx) {
                    continue;
                }

                let (_, tracker, classification) = &tracks[track_idx];
                let gate = config.gate_for(&classification.category);

                if let Some(innov) = tracker.variant.innovation(&obs.observation) {
                    let distance = innov.mahalanobis_distance;
                    if distance * distance <= gate.chi_squared_threshold {
                        costs.push((obs_idx, track_idx, distance));
                    }
                }
            }
        }

        // Greedy assignment sorted by cost (upgrade to JV for optimality later)
        costs.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

        let mut assigned_obs = vec![false; observations.len()];
        let mut assigned_tracks = vec![false; tracks.len()];
        let mut assignments = Vec::new();

        for (obs_idx, track_idx, distance) in &costs {
            if !assigned_obs[*obs_idx] && !assigned_tracks[*track_idx] {
                #[allow(clippy::cast_possible_truncation)]
                assignments.push(Assignment {
                    observation_idx: *obs_idx,
                    track_idx: *track_idx,
                    distance: *distance,
                    confidence: (1.0 - distance / 10.0).clamp(0.0, 1.0) as f32,
                });
                assigned_obs[*obs_idx] = true;
                assigned_tracks[*track_idx] = true;
            }
        }

        let unassigned_observations: Vec<usize> = (0..observations.len())
            .filter(|i| !assigned_obs[*i])
            .collect();
        let unassigned_tracks: Vec<usize> =
            (0..tracks.len()).filter(|i| !assigned_tracks[*i]).collect();

        AssociationResult {
            assignments,
            unassigned_observations,
            unassigned_tracks,
        }
    }
}

fn observation_geodetic_position(obs: &SensorObservation) -> Option<(f64, f64)> {
    match &obs.measurement {
        Measurement::PositionVelocity3D {
            lat_deg, lon_deg, ..
        }
        | Measurement::PositionVelocity2D {
            lat_deg, lon_deg, ..
        } => Some((*lat_deg, *lon_deg)),
        Measurement::Spherical {
            range_m,
            azimuth_rad,
            elevation_rad,
            ..
        } => {
            if let coord::CoordinateFrame::SensorSpherical {
                sensor_lat_deg,
                sensor_lon_deg,
                sensor_alt_m,
            } = &obs.sensor_id.coordinate_frame
            {
                let sensor_ecef =
                    coord::geodetic_to_ecef(*sensor_lat_deg, *sensor_lon_deg, *sensor_alt_m);
                let el = elevation_rad.unwrap_or(0.0);
                let target_ecef = coord::spherical_to_ecef(
                    *range_m,
                    *azimuth_rad,
                    el,
                    &sensor_ecef,
                    *sensor_lat_deg,
                    *sensor_lon_deg,
                );
                let (lat, lon, _) = coord::ecef_to_geodetic(&target_ecef);
                Some((lat, lon))
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::CoordinateFrame;
    use crate::filter::ekf::ProcessNoiseConfig;
    use crate::sensor::*;
    use crate::types::*;
    use chrono::Utc;
    use nalgebra::DMatrix;

    fn make_obs_at(lat: f64, lon: f64, icao: Option<&str>) -> StoredObservation {
        StoredObservation {
            observation: SensorObservation {
                sensor_id: SensorId {
                    id: "test".to_string(),
                    kind: SensorKind::AdsbReceiver,
                    tier: FusionTier::Regional,
                    coordinate_frame: CoordinateFrame::Wgs84,
                },
                timestamp: Utc::now(),
                receipt_time: Utc::now(),
                target_id: icao.map(|id| TargetId {
                    domain: TargetDomain::Air,
                    id: id.to_string(),
                    id_type: IdentifierType::Icao,
                }),
                measurement: Measurement::PositionVelocity3D {
                    lat_deg: lat,
                    lon_deg: lon,
                    alt_m: Some(10000.0),
                    vel_north_mps: Some(100.0),
                    vel_east_mps: Some(0.0),
                    vel_down_mps: Some(0.0),
                    heading_deg: None,
                },
                covariance: ObservationCovariance {
                    matrix: DMatrix::identity(6, 6) * 100.0,
                },
                classification_hint: Some(TargetCategory::FixedWing),
                metadata: ObservationMetadata::default(),
            },
            associated_track: None,
            store_index: 0,
        }
    }

    fn make_track_at(
        lat: f64,
        lon: f64,
        icao: Option<&str>,
    ) -> (Track, TrackerState, TargetClassification) {
        let mut tracker = TrackerState::new_6dof(ProcessNoiseConfig::default());
        let obs = make_obs_at(lat, lon, icao);
        tracker.variant.initialize(&obs.observation);

        let mut track = Track {
            id: TrackId::new(),
            cooperative_ids: Vec::new(),
            created_at: Utc::now(),
            last_update: Utc::now(),
            is_on_ground: false,
        };
        if let Some(id) = icao {
            track.cooperative_ids.push(TargetId {
                domain: TargetDomain::Air,
                id: id.to_string(),
                id_type: IdentifierType::Icao,
            });
        }

        let classification = TargetClassification::default();
        (track, tracker, classification)
    }

    #[test]
    fn associate_nearby_observation_to_track() {
        let obs = make_obs_at(37.0, -97.0, None);
        let (track, tracker, class) = make_track_at(37.0, -97.0, None);

        let mut spatial = SpatialIndex::new(0.5);
        spatial.update_track(&track.id, 37.0, -97.0);

        let config = AssociatorConfig::default();
        let result =
            GnnAssociator::associate(&[&obs], &[(&track, &tracker, &class)], &spatial, &config);
        assert_eq!(result.assignments.len(), 1);
        assert!(result.unassigned_observations.is_empty());
    }

    #[test]
    fn distant_observation_not_associated() {
        let obs = make_obs_at(50.0, -50.0, None);
        let (track, tracker, class) = make_track_at(37.0, -97.0, None);

        let mut spatial = SpatialIndex::new(0.5);
        spatial.update_track(&track.id, 37.0, -97.0);

        let config = AssociatorConfig::default();
        let result =
            GnnAssociator::associate(&[&obs], &[(&track, &tracker, &class)], &spatial, &config);
        assert!(result.assignments.is_empty());
        assert_eq!(result.unassigned_observations.len(), 1);
    }

    #[test]
    fn empty_inputs() {
        let spatial = SpatialIndex::new(0.5);
        let config = AssociatorConfig::default();
        let result = GnnAssociator::associate(&[], &[], &spatial, &config);
        assert!(result.assignments.is_empty());
    }

    #[test]
    fn cooperative_id_preferred() {
        let obs = make_obs_at(37.01, -97.01, Some("ABC123"));
        let (track1, tracker1, class1) = make_track_at(37.0, -97.0, Some("ABC123"));
        let (track2, tracker2, class2) = make_track_at(37.005, -97.005, None);

        let mut spatial = SpatialIndex::new(0.5);
        spatial.update_track(&track1.id, 37.0, -97.0);
        spatial.update_track(&track2.id, 37.005, -97.005);

        let config = AssociatorConfig::default();
        let result = GnnAssociator::associate(
            &[&obs],
            &[(&track1, &tracker1, &class1), (&track2, &tracker2, &class2)],
            &spatial,
            &config,
        );
        assert_eq!(result.assignments.len(), 1);
        assert_eq!(result.assignments[0].track_idx, 0);
    }
}
