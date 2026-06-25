use bevy::prelude::*;
use bevy_egui::EguiContexts;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

use crate::Aircraft;

/// Recorded aircraft state for a single frame
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedAircraftState {
    pub icao: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callsign: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altitude: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub velocity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_rate: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub squawk: Option<String>,
}

impl From<&Aircraft> for RecordedAircraftState {
    fn from(aircraft: &Aircraft) -> Self {
        Self {
            icao: aircraft.icao.clone(),
            callsign: aircraft.callsign.clone(),
            latitude: aircraft.latitude,
            longitude: aircraft.longitude,
            altitude: aircraft.altitude,
            heading: aircraft.heading,
            velocity: aircraft.velocity,
            vertical_rate: aircraft.vertical_rate,
            squawk: aircraft.squawk.clone(),
        }
    }
}

/// A single frame of recorded data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedFrame {
    /// Timestamp in milliseconds since recording start
    pub timestamp_ms: u64,
    /// All aircraft states at this timestamp
    pub aircraft: Vec<RecordedAircraftState>,
}

/// Recording state resource
#[derive(Resource, Default)]
pub struct RecordingState {
    /// Whether recording is active
    pub is_recording: bool,
    /// Recording start time
    pub start_time: Option<Instant>,
    /// File writer for the current recording
    writer: Option<BufWriter<File>>,
    /// Path to current recording file
    pub file_path: Option<PathBuf>,
    /// Number of frames recorded
    pub frame_count: u64,
    /// Last frame time for throttling
    last_frame_time: Option<Instant>,
}

impl RecordingState {
    /// Recording interval in milliseconds (approximately 1 FPS for efficient storage)
    const FRAME_INTERVAL_MS: u64 = 1000;

    /// Start a new recording
    pub fn start(&mut self) -> Result<(), String> {
        if self.is_recording {
            return Err("Already recording".to_string());
        }

        // Create data directory if it doesn't exist
        let tmp_dir = crate::paths::data_dir();
        std::fs::create_dir_all(&tmp_dir)
            .map_err(|e| format!("Failed to create data directory: {}", e))?;

        // Generate filename with timestamp
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let filename = format!("recording_{}.ndjson", timestamp);
        let file_path = tmp_dir.join(&filename);

        // Open file for writing
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&file_path)
            .map_err(|e| format!("Failed to create recording file: {}", e))?;

        self.writer = Some(BufWriter::new(file));
        self.file_path = Some(file_path.clone());
        self.start_time = Some(Instant::now());
        self.frame_count = 0;
        self.last_frame_time = None;
        self.is_recording = true;

        info!("Started recording to {:?}", file_path);
        Ok(())
    }

    /// Stop the current recording
    pub fn stop(&mut self) {
        if !self.is_recording {
            return;
        }

        if let Some(ref mut writer) = self.writer {
            let _ = writer.flush();
        }

        self.is_recording = false;
        self.writer = None;

        if let Some(ref path) = self.file_path {
            info!(
                "Stopped recording. {} frames saved to {:?}",
                self.frame_count, path
            );
        }
    }

    /// Record a frame of aircraft data
    pub fn record_frame(&mut self, aircraft: &[RecordedAircraftState]) {
        if !self.is_recording {
            return;
        }

        let Some(start_time) = self.start_time else {
            return;
        };

        // Throttle frame rate
        if let Some(last) = self.last_frame_time {
            if last.elapsed().as_millis() < Self::FRAME_INTERVAL_MS as u128 {
                return;
            }
        }

        let frame = RecordedFrame {
            timestamp_ms: start_time.elapsed().as_millis() as u64,
            aircraft: aircraft.to_vec(),
        };

        if let Some(ref mut writer) = self.writer {
            match serde_json::to_string(&frame) {
                Ok(json) => {
                    if writeln!(writer, "{}", json).is_ok() {
                        self.frame_count += 1;
                        self.last_frame_time = Some(Instant::now());
                    }
                }
                Err(e) => {
                    warn!("Failed to serialize frame: {}", e);
                }
            }
        }
    }

    /// Get recording duration in seconds
    pub fn duration_secs(&self) -> u64 {
        self.start_time.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }
}

/// System to record aircraft positions each frame
pub fn record_frame(mut recording: ResMut<RecordingState>, aircraft_query: Query<&Aircraft>) {
    if !recording.is_recording {
        return;
    }

    let states: Vec<RecordedAircraftState> = aircraft_query
        .iter()
        .map(RecordedAircraftState::from)
        .collect();

    recording.record_frame(&states);
}

/// System to toggle recording with a keyboard shortcut
pub fn toggle_recording(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut recording: ResMut<RecordingState>,
    mut contexts: EguiContexts,
) {
    // Don't toggle if egui wants input
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    // Ctrl+R to toggle recording
    if keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight) {
        if keyboard.just_pressed(KeyCode::KeyR) {
            if recording.is_recording {
                recording.stop();
            } else {
                if let Err(e) = recording.start() {
                    error!("Failed to start recording: {}", e);
                }
            }
        }
    }
}
