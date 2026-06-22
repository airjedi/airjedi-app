use bevy::prelude::*;
use chrono::{DateTime, Utc};
use airjedi_fusion::{Track, TrackerState, TrackQuality, TrackStatus};
use crate::aircraft::components::{AircraftLabel, FusionTrackLink};
use crate::aviation::loader::{AviationData, LoadingState};
use crate::geo::haversine_distance_nm;

const LANDING_ALTITUDE_FT: i32 = 1500;
const LANDING_SPEED_KTS: f64 = 80.0;
const RUNWAY_PROXIMITY_NM: f64 = 1.5;
const LANDED_CLEANUP_SECS: i64 = 30;

#[derive(Component)]
pub struct LandedAircraft {
    pub landed_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub near_airport: Option<String>,
}

pub fn detect_landings(
    mut commands: Commands,
    aviation_data: Option<Res<AviationData>>,
    mut fusion_tracks: Query<(Entity, &mut Track, &mut TrackerState, &TrackQuality)>,
    visual_lookup: Query<(Entity, &FusionTrackLink), Without<LandedAircraft>>,
) {
    let Some(aviation_data) = aviation_data else {
        return;
    };
    if aviation_data.loading_state != LoadingState::Ready {
        return;
    }

    for (track_entity, mut track, mut tracker, quality) in &mut fusion_tracks {
        if track.is_on_ground {
            continue;
        }
        if quality.status == TrackStatus::Lost {
            continue;
        }

        let (lat, lon, alt_m) = tracker.position_geodetic();
        let alt_ft = (alt_m / 0.3048) as i32;

        if alt_ft > LANDING_ALTITUDE_FT {
            continue;
        }

        let vel = tracker.velocity_ecef();
        let speed_mps = (vel[0].powi(2) + vel[1].powi(2) + vel[2].powi(2)).sqrt();
        let speed_kts = speed_mps / 0.514444;

        if speed_kts > LANDING_SPEED_KTS {
            continue;
        }

        let mut nearest_airport: Option<String> = None;
        let mut found_runway = false;

        for runway in &aviation_data.runways {
            if !runway.has_valid_coords() {
                continue;
            }

            if let (Some(le_lat), Some(le_lon)) =
                (runway.le_latitude_deg, runway.le_longitude_deg)
            {
                if haversine_distance_nm(lat, lon, le_lat, le_lon) < RUNWAY_PROXIMITY_NM {
                    nearest_airport = Some(runway.airport_ident.clone());
                    found_runway = true;
                    break;
                }
            }

            if !found_runway {
                if let (Some(he_lat), Some(he_lon)) =
                    (runway.he_latitude_deg, runway.he_longitude_deg)
                {
                    if haversine_distance_nm(lat, lon, he_lat, he_lon) < RUNWAY_PROXIMITY_NM {
                        nearest_airport = Some(runway.airport_ident.clone());
                        found_runway = true;
                        break;
                    }
                }
            }
        }

        if found_runway {
            track.is_on_ground = true;
            tracker.zero_velocity();

            for (visual_entity, link) in &visual_lookup {
                if link.track_entity == track_entity {
                    commands.entity(visual_entity).insert(LandedAircraft {
                        landed_at: Utc::now(),
                        near_airport: nearest_airport.clone(),
                    });
                }
            }
        }
    }
}

pub fn cleanup_landed_aircraft(
    mut commands: Commands,
    landed_query: Query<(Entity, &LandedAircraft, &FusionTrackLink)>,
    fusion_tracks: Query<&TrackQuality>,
    label_query: Query<(Entity, &AircraftLabel)>,
) {
    let now = Utc::now();

    for (entity, landed, link) in &landed_query {
        let age = now.signed_duration_since(landed.landed_at).num_seconds();

        let is_stale = if let Ok(quality) = fusion_tracks.get(link.track_entity) {
            quality.status == TrackStatus::Coasting || quality.status == TrackStatus::Lost
        } else {
            true
        };

        if age > LANDED_CLEANUP_SECS && is_stale {
            for (label_entity, label) in label_query.iter() {
                if label.aircraft_entity == entity {
                    commands.entity(label_entity).despawn();
                    break;
                }
            }
            commands.entity(entity).despawn();
        }
    }
}
