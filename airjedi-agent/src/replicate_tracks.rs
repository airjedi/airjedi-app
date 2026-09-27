//! The agent-side projection: fused track -> replicated `DisplayTrack`.
//!
//! This is the server half of the design-b boundary. It reuses the exact same
//! [`derive_display_track`] the fat-mode app uses, then maintains one
//! `(Replicated, DisplayTrack)` entity per fused track. `bevy_replicon` copies
//! those entities to every connected thin client, where the renderers read
//! `DisplayTrack` unchanged.

use std::collections::{HashMap, HashSet};

use airjedi_core::{
    DisplayEstimate, DisplayTrack, PositionSource, PredictedSample, SensorContributions,
    SensorReport, TrackId,
};
use airjedi_fusion::{
    derive_display_track, raw_observation_hint_for, IdentifierType, Measurement, SensorObservation,
    TimelineStore, Track, TrackQuality, TrackStatus, TrackerState,
};
use bevy::prelude::*;
use bevy_replicon::prelude::Replicated;
use nalgebra::DMatrix;

// Forward-prediction cone parameters (agent-side, straight-flight from the
// filter's own model). The client only draws the resulting samples.
const EST_HORIZON_S: f64 = 60.0;
const EST_STEPS: usize = 12;
const EST_SIGMA_MULT: f64 = 2.0;

/// Maps each fused track's stable [`TrackId`] to the replicated display entity.
#[derive(Resource, Default)]
pub struct TrackEntityMap(pub HashMap<TrackId, Entity>);

/// ICAOs readsb tagged `type:"mlat"`, used to tag `DisplayTrack.position_source`.
#[derive(Resource, Default)]
pub struct MlatSet(pub HashSet<u32>);

fn track_icao_u32(track: &Track) -> Option<u32> {
    track
        .cooperative_ids
        .iter()
        .find(|id| id.id_type == IdentifierType::Icao)
        .and_then(|id| adsb_client::Icao::from_hex(&id.id))
        .map(|i| i.0)
}

/// Upsert the replicated display components (`DisplayTrack` + `DisplayEstimate` +
/// `SensorContributions`) per changed fused track, and despawn display entities
/// whose fused track has gone away (or been marked `Lost`).
pub fn sync_replicated_tracks(
    mut commands: Commands,
    changed: Query<(&Track, &TrackerState, &TrackQuality), Changed<TrackerState>>,
    all_tracks: Query<&Track>,
    existing: Query<(), With<DisplayTrack>>,
    timeline_store: Res<TimelineStore>,
    mut map: ResMut<TrackEntityMap>,
    mlat: Res<MlatSet>,
) {
    for (track, tracker, quality) in &changed {
        let track_id = track.id.clone();

        // Lost tracks are gone too long to display; drop the replicated entity.
        if quality.status == TrackStatus::Lost {
            if let Some(entity) = map.0.remove(&track_id) {
                commands.entity(entity).despawn();
            }
            continue;
        }

        let position_source = match track_icao_u32(track) {
            Some(icao) if mlat.0.contains(&icao) => Some(PositionSource::Mlat),
            Some(_) => Some(PositionSource::AdsbIcao),
            None => None,
        };

        // The full render set, all derived agent-side from the fused track:
        //  - DisplayTrack: same projection the fat-mode app uses.
        //  - DisplayEstimate: straight-flight forward-prediction cone.
        //  - SensorContributions: latest per-sensor raw positions from the store.
        let hint = raw_observation_hint_for(&timeline_store, track);
        let dt = derive_display_track(track, tracker, quality, hint.as_ref(), position_source);
        let estimate = sample_estimate(tracker);
        let contributions = contributions_for(&timeline_store, track);

        match map.0.get(&track_id).copied() {
            Some(entity) if existing.contains(entity) => {
                commands
                    .entity(entity)
                    .insert((dt, estimate, contributions));
            }
            _ => {
                let entity = commands
                    .spawn((Replicated, dt, estimate, contributions))
                    .id();
                map.0.insert(track_id, entity);
            }
        }
    }

    // Reap display entities whose fused track no longer exists (track cleanup
    // despawns the fused entity; the changed-query above never sees that).
    let live: HashSet<TrackId> = all_tracks.iter().map(|t| t.id.clone()).collect();
    let stale: Vec<TrackId> = map
        .0
        .keys()
        .filter(|id| !live.contains(*id))
        .cloned()
        .collect();
    for track_id in stale {
        if let Some(entity) = map.0.remove(&track_id) {
            commands.entity(entity).despawn();
        }
    }
}

// --- agent-side projection helpers ---

/// Latest raw per-sensor position for a track, from the fusion timeline store.
/// Mirrors the fat app's `sync_sensor_contributions`.
fn contributions_for(store: &TimelineStore, track: &Track) -> SensorContributions {
    let latest = store.latest_per_sensor(&track.id);
    let mut sources: Vec<SensorReport> = latest
        .iter()
        .filter_map(|(sensor_id, stored)| {
            let (lat, lon) = obs_lat_lon(&stored.observation)?;
            Some(SensorReport {
                sensor_id: sensor_id.clone(),
                lat,
                lon,
                kind: stored.observation.sensor_id.kind,
            })
        })
        .collect();
    // Deterministic order (the source map has no inherent ordering).
    sources.sort_by(|a, b| a.sensor_id.cmp(&b.sensor_id));
    SensorContributions { sources }
}

fn obs_lat_lon(obs: &SensorObservation) -> Option<(f64, f64)> {
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

/// Forward-predict the track along the filter's own model (straight flight),
/// recording lat/lon + growing horizontal uncertainty at each step. This is the
/// headless-friendly core of the fat app's `sample_predicted_track` without the
/// client-only turn-rate / snap-to-visual alignment.
fn sample_estimate(tracker: &TrackerState) -> DisplayEstimate {
    let mut cloned = tracker.clone();
    let dt = EST_HORIZON_S / EST_STEPS as f64;
    let mut samples = Vec::with_capacity(EST_STEPS);

    for i in 0..EST_STEPS {
        cloned.variant.predict(dt);
        let (lat, lon, _alt) = cloned.position_geodetic();
        let vel = cloned.velocity_ecef();
        let cov = cloned.variant.covariance_mat();
        let h_unc = horizontal_uncertainty_m(&cov, lat, lon) * EST_SIGMA_MULT;
        let heading = ecef_vel_to_heading_deg(&vel, lat, lon);
        samples.push(PredictedSample {
            lat,
            lon,
            h_uncertainty_m: h_unc,
            heading_deg: heading as f32,
            time_ahead: (dt * (i + 1) as f64) as f32,
        });
    }

    let maneuver_prob = tracker
        .mode_info()
        .and_then(|m| m.probabilities.get(1).copied())
        .unwrap_or(0.0) as f32;

    DisplayEstimate {
        samples,
        maneuver_prob,
    }
}

fn ecef_vel_to_enu(vel: &[f64; 3], lat_deg: f64, lon_deg: f64) -> (f64, f64, f64) {
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    let (sin_lat, cos_lat) = (lat.sin(), lat.cos());
    let (sin_lon, cos_lon) = (lon.sin(), lon.cos());
    let east = -sin_lon * vel[0] + cos_lon * vel[1];
    let north = -sin_lat * cos_lon * vel[0] - sin_lat * sin_lon * vel[1] + cos_lat * vel[2];
    let up = cos_lat * cos_lon * vel[0] + cos_lat * sin_lon * vel[1] + sin_lat * vel[2];
    (east, north, up)
}

fn ecef_vel_to_heading_deg(vel: &[f64; 3], lat_deg: f64, lon_deg: f64) -> f64 {
    let (east, north, _) = ecef_vel_to_enu(vel, lat_deg, lon_deg);
    east.atan2(north).to_degrees().rem_euclid(360.0)
}

/// 1-sigma horizontal position uncertainty (meters) from the ECEF position
/// covariance, rotated into the local ENU frame. Ported from the fat app.
fn horizontal_uncertainty_m(cov: &DMatrix<f64>, lat_deg: f64, lon_deg: f64) -> f64 {
    if cov.nrows() < 3 {
        return 0.0;
    }
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    let (sin_lat, cos_lat) = (lat.sin(), lat.cos());
    let (sin_lon, cos_lon) = (lon.sin(), lon.cos());
    let p = cov.view((0, 0), (3, 3));

    let var_east = sin_lon * sin_lon * p[(0, 0)] + cos_lon * cos_lon * p[(1, 1)]
        - 2.0 * sin_lon * cos_lon * p[(0, 1)];
    let var_north = (sin_lat * cos_lon).powi(2) * p[(0, 0)]
        + (sin_lat * sin_lon).powi(2) * p[(1, 1)]
        + cos_lat.powi(2) * p[(2, 2)]
        + 2.0 * sin_lat.powi(2) * sin_lon * cos_lon * p[(0, 1)]
        - 2.0 * sin_lat * cos_lat * cos_lon * p[(0, 2)]
        - 2.0 * sin_lat * cos_lat * sin_lon * p[(1, 2)];

    (var_east.abs() + var_north.abs()).sqrt()
}
