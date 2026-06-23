use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use bevy_slippy_tiles::SlippyTilesSettings;
use airjedi_fusion::{TrackerState, TrackQuality};
use airjedi_fusion::nalgebra::DMatrix;

use crate::aircraft::components::FusionTrackLink;
use crate::aircraft::{AircraftListState, CameraFollowState, Aircraft};
use crate::geo::CoordinateConverter;
use crate::map::MapState;
use crate::view3d::View3DState;

#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct EstimatedTrackConfig {
    pub enabled: bool,
    pub horizon_seconds: f32,
    pub sample_count: usize,
    pub sigma_multiplier: f32,
    pub min_speed_kts: f64,
    pub max_turn_rate_dps: f64,
}

impl Default for EstimatedTrackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            horizon_seconds: 20.0,
            sample_count: 20,
            sigma_multiplier: 2.0,
            min_speed_kts: 30.0,
            max_turn_rate_dps: 6.0,
        }
    }
}

#[derive(Resource, Default)]
pub struct HeadingHistory {
    entries: HashMap<Entity, VecDeque<(f64, f64)>>,
}

const HEADING_HISTORY_WINDOW: f64 = 5.0;
const TURN_RATE_DEAD_ZONE: f64 = 0.1;

struct PredictedSample {
    lat: f64,
    lon: f64,
    h_uncertainty_m: f64,
    heading_deg: f64,
    time_ahead: f32,
}

fn ecef_vel_to_enu(vel_ecef: &[f64; 3], lat_deg: f64, lon_deg: f64) -> (f64, f64, f64) {
    let lat_rad = lat_deg.to_radians();
    let lon_rad = lon_deg.to_radians();
    let sin_lat = lat_rad.sin();
    let cos_lat = lat_rad.cos();
    let sin_lon = lon_rad.sin();
    let cos_lon = lon_rad.cos();

    let east = -sin_lon * vel_ecef[0] + cos_lon * vel_ecef[1];
    let north = -sin_lat * cos_lon * vel_ecef[0]
        - sin_lat * sin_lon * vel_ecef[1]
        + cos_lat * vel_ecef[2];
    let up = cos_lat * cos_lon * vel_ecef[0]
        + cos_lat * sin_lon * vel_ecef[1]
        + sin_lat * vel_ecef[2];
    (east, north, up)
}

fn enu_to_ecef_vel(east: f64, north: f64, up: f64, lat_deg: f64, lon_deg: f64) -> [f64; 3] {
    let lat_rad = lat_deg.to_radians();
    let lon_rad = lon_deg.to_radians();
    let sin_lat = lat_rad.sin();
    let cos_lat = lat_rad.cos();
    let sin_lon = lon_rad.sin();
    let cos_lon = lon_rad.cos();

    let vx = -sin_lon * east - sin_lat * cos_lon * north + cos_lat * cos_lon * up;
    let vy = cos_lon * east - sin_lat * sin_lon * north + cos_lat * sin_lon * up;
    let vz = cos_lat * north + sin_lat * up;
    [vx, vy, vz]
}

fn ecef_vel_to_heading_deg(vel_ecef: &[f64; 3], lat_deg: f64, lon_deg: f64) -> f64 {
    let (east, north, _) = ecef_vel_to_enu(vel_ecef, lat_deg, lon_deg);
    east.atan2(north).to_degrees().rem_euclid(360.0)
}

fn rotate_velocity_ecef(
    vel_ecef: &[f64; 3],
    lat_deg: f64,
    lon_deg: f64,
    angle_deg: f64,
) -> [f64; 3] {
    let (east, north, up) = ecef_vel_to_enu(vel_ecef, lat_deg, lon_deg);
    let angle_rad = angle_deg.to_radians();
    let cos_a = angle_rad.cos();
    let sin_a = angle_rad.sin();
    let rotated_east = east * cos_a + north * sin_a;
    let rotated_north = -east * sin_a + north * cos_a;
    enu_to_ecef_vel(rotated_east, rotated_north, up, lat_deg, lon_deg)
}

fn compute_turn_rate(history: &VecDeque<(f64, f64)>) -> f64 {
    if history.len() < 2 {
        return 0.0;
    }
    let (t_old, h_old) = history.front().unwrap();
    let (t_new, h_new) = history.back().unwrap();
    let dt = t_new - t_old;
    if dt < 0.3 {
        return 0.0;
    }
    let mut dh = h_new - h_old;
    if dh > 180.0 { dh -= 360.0; }
    if dh < -180.0 { dh += 360.0; }
    dh / dt
}

fn horizontal_uncertainty_m(cov: &DMatrix<f64>, lat_deg: f64, lon_deg: f64) -> f64 {
    if cov.nrows() < 3 {
        return 0.0;
    }

    let lat_rad = lat_deg.to_radians();
    let lon_rad = lon_deg.to_radians();

    let sin_lat = lat_rad.sin();
    let cos_lat = lat_rad.cos();
    let sin_lon = lon_rad.sin();
    let cos_lon = lon_rad.cos();

    let pos_cov = cov.view((0, 0), (3, 3));

    let var_east = sin_lon * sin_lon * pos_cov[(0, 0)]
        + cos_lon * cos_lon * pos_cov[(1, 1)]
        - 2.0 * sin_lon * cos_lon * pos_cov[(0, 1)];

    let var_north = (sin_lat * cos_lon).powi(2) * pos_cov[(0, 0)]
        + (sin_lat * sin_lon).powi(2) * pos_cov[(1, 1)]
        + cos_lat.powi(2) * pos_cov[(2, 2)]
        + 2.0 * sin_lat.powi(2) * sin_lon * cos_lon * pos_cov[(0, 1)]
        - 2.0 * sin_lat * cos_lat * cos_lon * pos_cov[(0, 2)]
        - 2.0 * sin_lat * cos_lat * sin_lon * pos_cov[(1, 2)];

    (var_east.abs() + var_north.abs()).sqrt()
}

fn sample_predicted_track(
    tracker: &TrackerState,
    config: &EstimatedTrackConfig,
    turn_rate_dps: f64,
) -> Vec<PredictedSample> {
    let mut cloned = tracker.clone();
    let dt = config.horizon_seconds as f64 / config.sample_count as f64;
    let mut samples = Vec::with_capacity(config.sample_count);

    let applying_turn = turn_rate_dps.abs() > TURN_RATE_DEAD_ZONE;
    let clamped_turn = turn_rate_dps.clamp(-config.max_turn_rate_dps, config.max_turn_rate_dps);

    for i in 0..config.sample_count {
        if applying_turn {
            let (lat, lon, _) = cloned.position_geodetic();
            let vel = cloned.velocity_ecef();
            let rotated = rotate_velocity_ecef(&vel, lat, lon, clamped_turn * dt);

            let state = cloned.variant.state_vec();
            let cov = cloned.variant.covariance_mat();
            let mut new_state = state.clone();
            new_state[3] = rotated[0];
            new_state[4] = rotated[1];
            new_state[5] = rotated[2];
            cloned.variant.initialize_from_state(new_state, cov);
        }

        cloned.variant.predict(dt);

        let (lat, lon, _alt) = cloned.position_geodetic();
        let vel = cloned.velocity_ecef();
        let cov = cloned.variant.covariance_mat();
        let h_unc = horizontal_uncertainty_m(&cov, lat, lon);
        let heading = ecef_vel_to_heading_deg(&vel, lat, lon);

        samples.push(PredictedSample {
            lat,
            lon,
            h_uncertainty_m: h_unc * config.sigma_multiplier as f64,
            heading_deg: heading,
            time_ahead: dt as f32 * (i + 1) as f32,
        });
    }
    samples
}

fn meters_to_world_units(lat_rad: f64, zoom: i32) -> f64 {
    let tiles_around_earth = (1u64 << zoom) as f64;
    let world_units_per_degree = 256.0 * tiles_around_earth / 360.0;
    let meters_per_degree = 111_320.0 * lat_rad.cos();
    world_units_per_degree / meters_per_degree
}

pub fn update_heading_history(
    time: Res<Time>,
    mut history: ResMut<HeadingHistory>,
    trackers: Query<(Entity, &TrackerState, &TrackQuality)>,
) {
    let now = time.elapsed_secs_f64();

    for (entity, tracker, _quality) in trackers.iter() {
        let vel = tracker.velocity_ecef();
        let speed_sq = vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2];
        if speed_sq < 10.0 * 10.0 {
            continue;
        }

        let (lat, lon, _) = tracker.position_geodetic();
        let heading = ecef_vel_to_heading_deg(&vel, lat, lon);

        let ring = history.entries.entry(entity).or_default();
        ring.push_back((now, heading));

        while ring.len() > 2 {
            if let Some(&(t, _)) = ring.front() {
                if now - t > HEADING_HISTORY_WINDOW {
                    ring.pop_front();
                } else {
                    break;
                }
            }
        }
    }

    history.entries.retain(|entity, _| trackers.get(*entity).is_ok());
}

pub fn draw_estimated_track_cones(
    mut gizmos: Gizmos,
    config: Res<EstimatedTrackConfig>,
    list_state: Res<AircraftListState>,
    follow_state: Res<CameraFollowState>,
    tile_settings: Res<SlippyTilesSettings>,
    map_state: Res<MapState>,
    view3d_state: Res<View3DState>,
    heading_history: Res<HeadingHistory>,
    fusion_tracks: Query<(&TrackerState, &TrackQuality)>,
    visuals: Query<(&FusionTrackLink, &Aircraft)>,
) {
    if !config.enabled {
        return;
    }

    let target_icao = follow_state
        .following_icao
        .as_ref()
        .or(list_state.selected_icao.as_ref());

    let Some(target_icao) = target_icao else {
        return;
    };

    let Some((link, aircraft)) = visuals.iter().find(|(_, a)| &a.icao == target_icao) else {
        return;
    };

    let Ok((tracker, _quality)) = fusion_tracks.get(link.track_entity) else {
        return;
    };

    let vel = tracker.velocity_ecef();
    let speed_mps = (vel[0].powi(2) + vel[1].powi(2) + vel[2].powi(2)).sqrt();
    let speed_kts = speed_mps / 0.514444;
    if speed_kts < config.min_speed_kts {
        return;
    }

    let turn_rate = heading_history
        .entries
        .get(&link.track_entity)
        .map(|h| compute_turn_rate(h))
        .unwrap_or(0.0);

    let zoom = view3d_state.effective_zoom(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, zoom);

    let samples = sample_predicted_track(tracker, &config, turn_rate);
    if samples.is_empty() {
        return;
    }

    let aircraft_pos = converter.latlon_to_world(aircraft.latitude, aircraft.longitude);
    let lat_rad = aircraft.latitude.to_radians();
    let zoom_int = i32::from(zoom.to_u8());
    let wu_per_m = meters_to_world_units(lat_rad, zoom_int);

    let center_color_base = Color::srgba(0.0, 0.9, 1.0, 0.7);
    let boundary_color_base = Color::srgba(1.0, 0.6, 0.1, 0.4);
    let crossbar_color_base = Color::srgba(1.0, 0.6, 0.1, 0.15);

    let mut prev_center = aircraft_pos;
    let mut prev_left = aircraft_pos;
    let mut prev_right = aircraft_pos;

    for (i, sample) in samples.iter().enumerate() {
        let t_frac = sample.time_ahead / config.horizon_seconds;
        let alpha_fade = 1.0 - t_frac * 0.6;

        let sample_pos = converter.latlon_to_world(sample.lat, sample.lon);
        let radius_world = (sample.h_uncertainty_m * wu_per_m) as f32;

        let heading_rad = sample.heading_deg.to_radians();
        let heading_dir = Vec2::new(heading_rad.sin() as f32, heading_rad.cos() as f32);

        if heading_dir == Vec2::ZERO {
            prev_center = sample_pos;
            continue;
        }
        let perp = Vec2::new(-heading_dir.y, heading_dir.x);

        let left = sample_pos + perp * radius_world;
        let right = sample_pos - perp * radius_world;

        let center_color = center_color_base.with_alpha(0.7 * alpha_fade);
        gizmos.line_2d(prev_center, sample_pos, center_color);

        let boundary_color = boundary_color_base.with_alpha(0.4 * alpha_fade);
        gizmos.line_2d(prev_left, left, boundary_color);
        gizmos.line_2d(prev_right, right, boundary_color);

        let crossbar_color = crossbar_color_base.with_alpha(0.15 * alpha_fade);
        gizmos.line_2d(left, right, crossbar_color);

        if i == samples.len() - 1 {
            gizmos.circle_2d(sample_pos, 3.0, center_color);
        }

        prev_center = sample_pos;
        prev_left = left;
        prev_right = right;
    }
}
