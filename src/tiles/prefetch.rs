use bevy::prelude::*;
use std::collections::VecDeque;

use super::coords::SlippyTileCoordinates;
use super::download::{DownloadTilesRequest, TileDownloadSettings};
use super::types::{DownloadPriority, Radius};
use crate::map::MapState;
use crate::view3d;

/// Tracks camera velocity for predictive tile prefetching.
#[derive(Resource)]
pub struct CameraVelocityTracker {
    prev_lat: f64,
    prev_lon: f64,
    velocity_lat: f64,
    velocity_lon: f64,
    samples: VecDeque<(f64, f64)>,
}

impl Default for CameraVelocityTracker {
    fn default() -> Self {
        Self {
            prev_lat: 0.0,
            prev_lon: 0.0,
            velocity_lat: 0.0,
            velocity_lon: 0.0,
            samples: VecDeque::with_capacity(8),
        }
    }
}

impl CameraVelocityTracker {
    fn update(&mut self, lat: f64, lon: f64, dt: f64) {
        if dt <= 0.0 {
            return;
        }
        let vlat = (lat - self.prev_lat) / dt;
        let vlon = (lon - self.prev_lon) / dt;
        self.prev_lat = lat;
        self.prev_lon = lon;

        self.samples.push_back((vlat, vlon));
        if self.samples.len() > 6 {
            self.samples.pop_front();
        }

        let n = self.samples.len() as f64;
        self.velocity_lat = self.samples.iter().map(|(v, _)| v).sum::<f64>() / n;
        self.velocity_lon = self.samples.iter().map(|(_, v)| v).sum::<f64>() / n;
    }

    pub fn is_moving(&self) -> bool {
        self.velocity_lat.abs() > 0.0001 || self.velocity_lon.abs() > 0.0001
    }

    pub fn predicted_position(&self, seconds_ahead: f64) -> (f64, f64) {
        (
            self.prev_lat + self.velocity_lat * seconds_ahead,
            self.prev_lon + self.velocity_lon * seconds_ahead,
        )
    }
}

/// Prefetch timer - fires less often than the main tile refresh.
#[derive(Resource)]
struct PrefetchTimer(Timer);

impl Default for PrefetchTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.5, TimerMode::Repeating))
    }
}

fn prefetch_tiles(
    mut timer: ResMut<PrefetchTimer>,
    time: Res<Time>,
    map_state: Res<MapState>,
    view3d_state: Res<view3d::View3DState>,
    mut velocity: ResMut<CameraVelocityTracker>,
    mut download_events: MessageWriter<DownloadTilesRequest>,
    settings: Res<TileDownloadSettings>,
) {
    let dt = time.delta_secs_f64();
    velocity.update(map_state.latitude, map_state.longitude, dt);

    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }

    if !velocity.is_moving() {
        return;
    }

    let zoom = map_state.zoom_level.to_u8();

    // Predict where camera will be in 1-2 seconds and prefetch that area.
    for &lookahead in &[1.0, 2.0] {
        let (pred_lat, pred_lon) = velocity.predicted_position(lookahead);
        download_events.write(DownloadTilesRequest {
            latitude: crate::clamp_latitude(pred_lat),
            longitude: crate::clamp_longitude(pred_lon),
            zoom,
            radius: Radius(2),
            priority: DownloadPriority::Mid,
            use_cache: true,
        });
    }

    // In 3D mode, also prefetch in the camera look direction.
    if view3d_state.is_3d_active() {
        let yaw_rad = view3d_state.camera_yaw.to_radians();
        let deg_per_tile = 360.0 / (1u64 << zoom) as f64;
        let offset = deg_per_tile * 3.0;
        let ahead_lat = map_state.latitude + offset * (yaw_rad.cos() as f64);
        let ahead_lon = map_state.longitude + offset * (yaw_rad.sin() as f64);

        download_events.write(DownloadTilesRequest {
            latitude: crate::clamp_latitude(ahead_lat),
            longitude: crate::clamp_longitude(ahead_lon),
            zoom,
            radius: Radius(3),
            priority: DownloadPriority::Far,
            use_cache: true,
        });
    }
}

pub(super) fn setup_prefetch_systems(app: &mut App) {
    app.init_resource::<CameraVelocityTracker>()
        .init_resource::<PrefetchTimer>()
        .add_systems(Update, prefetch_tiles);
}
