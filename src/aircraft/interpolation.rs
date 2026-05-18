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
