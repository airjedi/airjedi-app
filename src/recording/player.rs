use bevy::prelude::*;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::Instant;

use super::recorder::RecordedFrame;
use crate::aircraft::TrailHistory;
use crate::Aircraft;

/// Playback state resource
#[derive(Resource, Default)]
pub struct PlaybackState {
    /// Whether playback is active
    pub is_playing: bool,
    /// Whether playback is paused
    pub is_paused: bool,
    /// Playback speed multiplier
    pub speed: f32,
    /// Current playback time in milliseconds
    pub current_time_ms: u64,
    /// Total duration in milliseconds
    pub total_duration_ms: u64,
    /// Loaded frames
    frames: Vec<RecordedFrame>,
    /// Current frame index
    current_frame_index: usize,
    /// Playback start real time
    playback_start: Option<Instant>,
    /// Time when paused
    pause_time: Option<Instant>,
    /// Accumulated pause duration
    accumulated_pause_ms: u64,
}

impl PlaybackState {
    /// Load a recording file
    pub fn load(&mut self, path: &Path) -> Result<(), String> {
        if self.is_playing {
            self.stop();
        }

        let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;

        let reader = BufReader::new(file);
        let mut frames = Vec::new();

        for line in reader.lines() {
            let line = line.map_err(|e| format!("Failed to read line: {}", e))?;
            if line.trim().is_empty() {
                continue;
            }

            let frame: RecordedFrame =
                serde_json::from_str(&line).map_err(|e| format!("Failed to parse frame: {}", e))?;
            frames.push(frame);
        }

        if frames.is_empty() {
            return Err("Recording file is empty".to_string());
        }

        self.total_duration_ms = frames.last().map(|f| f.timestamp_ms).unwrap_or(0);
        self.frames = frames;
        self.current_frame_index = 0;
        self.current_time_ms = 0;
        self.is_playing = true;
        self.is_paused = false;
        self.speed = 1.0;
        self.playback_start = Some(Instant::now());
        self.pause_time = None;
        self.accumulated_pause_ms = 0;

        info!(
            "Loaded recording with {} frames, duration {} ms",
            self.frames.len(),
            self.total_duration_ms
        );
        Ok(())
    }

    /// Stop playback
    pub fn stop(&mut self) {
        self.is_playing = false;
        self.is_paused = false;
        self.frames.clear();
        self.current_frame_index = 0;
        self.current_time_ms = 0;
        self.playback_start = None;
        self.pause_time = None;
        self.accumulated_pause_ms = 0;
    }

    /// Pause playback
    pub fn pause(&mut self) {
        if self.is_playing && !self.is_paused {
            self.is_paused = true;
            self.pause_time = Some(Instant::now());
        }
    }

    /// Resume playback
    pub fn resume(&mut self) {
        if self.is_playing && self.is_paused {
            if let Some(pause_time) = self.pause_time.take() {
                self.accumulated_pause_ms += pause_time.elapsed().as_millis() as u64;
            }
            self.is_paused = false;
        }
    }

    /// Seek to a specific time in milliseconds
    pub fn seek(&mut self, time_ms: u64) {
        self.current_time_ms = time_ms.min(self.total_duration_ms);

        // Find the frame closest to this time
        self.current_frame_index = self
            .frames
            .iter()
            .position(|f| f.timestamp_ms >= self.current_time_ms)
            .unwrap_or(self.frames.len().saturating_sub(1));
    }

    /// Get the current frame, if any
    pub fn current_frame(&self) -> Option<&RecordedFrame> {
        self.frames.get(self.current_frame_index)
    }

    /// Advance playback and return the current frame if one should be applied
    pub fn advance(&mut self) -> Option<&RecordedFrame> {
        if !self.is_playing || self.is_paused || self.frames.is_empty() {
            return None;
        }

        let Some(start) = self.playback_start else {
            return None;
        };

        // Calculate current playback time
        let real_elapsed_ms = start.elapsed().as_millis() as u64 - self.accumulated_pause_ms;
        self.current_time_ms = (real_elapsed_ms as f32 * self.speed) as u64;

        // Check if we've reached the end
        if self.current_time_ms >= self.total_duration_ms {
            self.stop();
            return None;
        }

        // Find the appropriate frame
        while self.current_frame_index < self.frames.len() {
            let frame = &self.frames[self.current_frame_index];
            if frame.timestamp_ms <= self.current_time_ms {
                self.current_frame_index += 1;
                return Some(frame);
            }
            break;
        }

        None
    }
}

/// System to apply playback frames to aircraft entities
pub fn playback_frame(
    mut commands: Commands,
    mut playback: ResMut<PlaybackState>,
    mut aircraft_query: Query<(Entity, &mut Aircraft, &mut TrailHistory)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    asset_server: Res<AssetServer>,
) {
    // Get the current frame to apply
    let frame = playback.advance();
    let Some(frame) = frame else {
        return;
    };

    // Build a map of current aircraft ICAOs
    let mut existing_aircraft: std::collections::HashMap<String, Entity> = aircraft_query
        .iter()
        .map(|(e, a, _)| (a.icao.clone(), e))
        .collect();

    // Apply frame data
    for state in &frame.aircraft {
        if let Some(entity) = existing_aircraft.remove(&state.icao) {
            // Update existing aircraft
            if let Ok((_, mut aircraft, _)) = aircraft_query.get_mut(entity) {
                aircraft.latitude = state.latitude;
                aircraft.longitude = state.longitude;
                aircraft.altitude = state.altitude;
                aircraft.heading = state.heading;
                aircraft.velocity = state.velocity;
                aircraft.vertical_rate = state.vertical_rate;
                aircraft.callsign = state.callsign.clone();
                aircraft.squawk = state.squawk.clone();
            }
        } else {
            // Spawn new aircraft for playback
            // This is a simplified spawn - in production you'd want to match
            // the main aircraft spawning logic
            commands.spawn((
                Aircraft {
                    icao: state.icao.clone(),
                    callsign: state.callsign.clone(),
                    latitude: state.latitude,
                    longitude: state.longitude,
                    altitude: state.altitude,
                    heading: state.heading,
                    velocity: state.velocity,
                    vertical_rate: state.vertical_rate,
                    squawk: state.squawk.clone(),
                    is_on_ground: None,
                    alert: None,
                    emergency: None,
                    spi: None,
                    last_seen: chrono::Utc::now(),
                },
                TrailHistory {
                    points: VecDeque::new(),
                },
                Transform::default(),
                Visibility::default(),
            ));
        }
    }

    // Remove aircraft that are no longer in the frame
    for (_icao, entity) in existing_aircraft {
        commands.entity(entity).despawn();
    }
}
