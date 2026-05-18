use bevy::prelude::*;

pub const BLEND_THRESHOLD_NM: f64 = 0.5;
pub const BLEND_DURATION_SECS: f32 = 0.3;
pub const MAX_PREDICTION_SECS: f64 = 15.0;
pub const MIN_PREDICTION_SPEED_KTS: f64 = 10.0;

#[derive(Clone)]
pub struct BlendTarget {
    pub old_base_lat: f64,
    pub old_base_lon: f64,
    pub old_base_altitude: Option<f32>,
    pub old_base_heading: Option<f32>,
    pub old_base_speed: Option<f64>,
    pub old_base_vertical_rate: Option<f32>,
    pub old_base_time: f64,
    pub blend_start_time: f64,
    pub blend_duration: f32,
}

#[derive(Component, Clone, Reflect)]
#[reflect(Component)]
pub struct InterpolationState {
    pub base_lat: f64,
    pub base_lon: f64,
    pub base_altitude: Option<f32>,
    pub base_heading: Option<f32>,
    pub base_speed: Option<f64>,
    pub base_vertical_rate: Option<f32>,
    pub base_time: f64,

    #[reflect(ignore)]
    pub blend_target: Option<BlendTarget>,

    pub display_lat: f64,
    pub display_lon: f64,
    pub display_altitude: Option<f32>,
    pub display_heading: Option<f32>,

    pub predicting: bool,
}

impl InterpolationState {
    pub fn new(lat: f64, lon: f64, altitude: Option<i32>, heading: Option<f32>,
               speed: Option<f64>, vertical_rate: Option<i32>,
               is_on_ground: Option<bool>, current_time: f64) -> Self {
        let alt_f32 = altitude.map(|a| a as f32);
        let vrate_f32 = vertical_rate.map(|v| v as f32);
        let predicting = should_predict(heading, speed, is_on_ground);
        Self {
            base_lat: lat,
            base_lon: lon,
            base_altitude: alt_f32,
            base_heading: heading,
            base_speed: speed,
            base_vertical_rate: vrate_f32,
            base_time: current_time,
            blend_target: None,
            display_lat: lat,
            display_lon: lon,
            display_altitude: alt_f32,
            display_heading: heading,
            predicting,
        }
    }
}

pub fn should_predict(heading: Option<f32>, speed: Option<f64>, is_on_ground: Option<bool>) -> bool {
    if is_on_ground == Some(true) {
        return false;
    }
    let Some(_) = heading else { return false };
    let Some(spd) = speed else { return false };
    spd > MIN_PREDICTION_SPEED_KTS
}

pub fn shortest_angle_diff(from: f32, to: f32) -> f32 {
    let mut diff = (to - from) % 360.0;
    if diff > 180.0 {
        diff -= 360.0;
    } else if diff < -180.0 {
        diff += 360.0;
    }
    diff
}

pub fn lerp_heading(from: f32, to: f32, t: f32) -> f32 {
    let diff = shortest_angle_diff(from, to);
    let result = from + diff * t;
    ((result % 360.0) + 360.0) % 360.0
}

use crate::geo;
use crate::config::AppConfig;

fn dead_reckon(lat: f64, lon: f64, heading: Option<f32>, speed: Option<f64>,
               altitude: Option<f32>, vertical_rate: Option<f32>,
               elapsed_secs: f64) -> (f64, f64, Option<f32>) {
    let (pred_lat, pred_lon) = match (heading, speed) {
        (Some(hdg), Some(spd)) if spd > MIN_PREDICTION_SPEED_KTS => {
            let elapsed_minutes = (elapsed_secs / 60.0) as f32;
            geo::predict_position(lat, lon, hdg, spd, elapsed_minutes)
        }
        _ => (lat, lon),
    };

    let pred_alt = match (altitude, vertical_rate) {
        (Some(alt), Some(vrate)) => Some((alt + vrate * elapsed_secs as f32 / 60.0).max(0.0)),
        (Some(alt), None) => Some(alt),
        _ => None,
    };

    (pred_lat, pred_lon, pred_alt)
}

pub fn interpolate_aircraft_positions(
    time: Res<Time<Real>>,
    config: Res<AppConfig>,
    mut query: Query<&mut InterpolationState>,
) {
    if !config.interpolation_enabled {
        return;
    }

    let now = time.elapsed_secs_f64();

    for mut interp in query.iter_mut() {
        let elapsed = now - interp.base_time;

        if elapsed > MAX_PREDICTION_SECS {
            interp.predicting = false;
            continue;
        }

        if !interp.predicting {
            continue;
        }

        // Dead reckon from current baseline
        let (new_lat, new_lon, new_alt) = dead_reckon(
            interp.base_lat, interp.base_lon,
            interp.base_heading, interp.base_speed,
            interp.base_altitude, interp.base_vertical_rate,
            elapsed,
        );

        if let Some(blend) = interp.blend_target.clone() {
            let blend_elapsed = now - blend.blend_start_time;
            let t = (blend_elapsed as f32 / blend.blend_duration).clamp(0.0, 1.0);

            // Dead reckon from the OLD baseline too
            let old_elapsed = now - blend.old_base_time;
            let (old_lat, old_lon, old_alt) = dead_reckon(
                blend.old_base_lat, blend.old_base_lon,
                blend.old_base_heading, blend.old_base_speed,
                blend.old_base_altitude, blend.old_base_vertical_rate,
                old_elapsed,
            );

            // Lerp between old and new dead-reckoned tracks
            let t64 = t as f64;
            interp.display_lat = old_lat + (new_lat - old_lat) * t64;
            interp.display_lon = old_lon + (new_lon - old_lon) * t64;

            interp.display_altitude = match (old_alt, new_alt) {
                (Some(o), Some(n)) => Some(o + (n - o) * t),
                (_, n) => n,
            };

            interp.display_heading = match (blend.old_base_heading, interp.base_heading) {
                (Some(old_h), Some(new_h)) => Some(lerp_heading(old_h, new_h, t)),
                (_, h) => h,
            };

            if t >= 1.0 {
                interp.blend_target = None;
            }
        } else {
            // No blend active - straight dead reckoning
            interp.display_lat = new_lat;
            interp.display_lon = new_lon;
            interp.display_altitude = new_alt;
            interp.display_heading = interp.base_heading;
        }
    }
}

pub fn update_interpolation_on_adsb(
    interp: &mut InterpolationState,
    new_lat: f64,
    new_lon: f64,
    new_altitude: Option<i32>,
    new_heading: Option<f32>,
    new_speed: Option<f64>,
    new_vertical_rate: Option<i32>,
    is_on_ground: Option<bool>,
    current_time: f64,
) {
    let error_nm = geo::haversine_distance_nm(
        interp.display_lat, interp.display_lon,
        new_lat, new_lon,
    );

    let new_alt_f32 = new_altitude.map(|a| a as f32);
    let new_vrate_f32 = new_vertical_rate.map(|v| v as f32);

    if error_nm < BLEND_THRESHOLD_NM {
        interp.blend_target = Some(BlendTarget {
            old_base_lat: interp.base_lat,
            old_base_lon: interp.base_lon,
            old_base_altitude: interp.base_altitude,
            old_base_heading: interp.base_heading,
            old_base_speed: interp.base_speed,
            old_base_vertical_rate: interp.base_vertical_rate,
            old_base_time: interp.base_time,
            blend_start_time: current_time,
            blend_duration: BLEND_DURATION_SECS,
        });
    } else {
        interp.display_lat = new_lat;
        interp.display_lon = new_lon;
        interp.display_altitude = new_alt_f32;
        interp.display_heading = new_heading;
        interp.blend_target = None;
    }

    interp.base_lat = new_lat;
    interp.base_lon = new_lon;
    interp.base_altitude = new_alt_f32;
    interp.base_heading = new_heading;
    interp.base_speed = new_speed;
    interp.base_vertical_rate = new_vrate_f32;
    interp.base_time = current_time;
    interp.predicting = should_predict(new_heading, new_speed, is_on_ground);
}
