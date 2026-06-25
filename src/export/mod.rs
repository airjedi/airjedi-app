//! Export/Import Module
//!
//! Provides functionality to export flight data to various formats (KML, CSV)
//! and import previously recorded sessions.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::geo::FEET_TO_METERS;
use crate::recording::RecordedFrame;

/// Export format options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportFormat {
    #[default]
    KML,
    CSV,
    GeoJSON,
}

impl ExportFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::KML => "kml",
            ExportFormat::CSV => "csv",
            ExportFormat::GeoJSON => "geojson",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            ExportFormat::KML => "KML (Google Earth)",
            ExportFormat::CSV => "CSV (Spreadsheet)",
            ExportFormat::GeoJSON => "GeoJSON (Map Data)",
        }
    }
}

/// Resource for export state
#[derive(Resource, Default)]
pub struct ExportState {
    /// Whether the export panel is open
    pub panel_open: bool,
    /// Selected export format
    pub format: ExportFormat,
    /// Last export path
    pub last_export_path: Option<PathBuf>,
    /// Status message
    pub status_message: Option<String>,
    /// Include trail data
    pub include_trails: bool,
}

/// Export flight data to KML format (for Google Earth)
pub fn export_to_kml(frames: &[RecordedFrame], output_path: &Path) -> Result<(), String> {
    let mut file =
        File::create(output_path).map_err(|e| format!("Failed to create file: {}", e))?;

    // KML header
    writeln!(file, r#"<?xml version="1.0" encoding="UTF-8"?>"#)
        .map_err(|e| format!("Write error: {}", e))?;
    writeln!(file, r#"<kml xmlns="http://www.opengis.net/kml/2.2">"#)
        .map_err(|e| format!("Write error: {}", e))?;
    writeln!(file, "<Document>").map_err(|e| format!("Write error: {}", e))?;
    writeln!(file, "  <name>AirJedi Flight Export</name>")
        .map_err(|e| format!("Write error: {}", e))?;

    // Define styles
    writeln!(file, r#"  <Style id="aircraftStyle">"#).map_err(|e| format!("Write error: {}", e))?;
    writeln!(
        file,
        r#"    <LineStyle><color>ff0000ff</color><width>2</width></LineStyle>"#
    )
    .map_err(|e| format!("Write error: {}", e))?;
    writeln!(file, "  </Style>").map_err(|e| format!("Write error: {}", e))?;

    // Group positions by aircraft ICAO
    let mut aircraft_tracks: std::collections::HashMap<String, Vec<(f64, f64, i32, u64)>> =
        std::collections::HashMap::new();

    for frame in frames {
        for aircraft in &frame.aircraft {
            aircraft_tracks
                .entry(aircraft.icao.clone())
                .or_default()
                .push((
                    aircraft.longitude,
                    aircraft.latitude,
                    aircraft.altitude.unwrap_or(0),
                    frame.timestamp_ms,
                ));
        }
    }

    // Write each aircraft as a placemark with line string
    for (icao, positions) in &aircraft_tracks {
        if positions.len() < 2 {
            continue;
        }

        writeln!(file, "  <Placemark>").map_err(|e| format!("Write error: {}", e))?;
        writeln!(file, "    <name>{}</name>", icao).map_err(|e| format!("Write error: {}", e))?;
        writeln!(file, "    <styleUrl>#aircraftStyle</styleUrl>")
            .map_err(|e| format!("Write error: {}", e))?;
        writeln!(file, "    <LineString>").map_err(|e| format!("Write error: {}", e))?;
        writeln!(file, "      <altitudeMode>absolute</altitudeMode>")
            .map_err(|e| format!("Write error: {}", e))?;
        writeln!(file, "      <coordinates>").map_err(|e| format!("Write error: {}", e))?;

        for (lon, lat, alt, _ts) in positions {
            // KML uses meters for altitude, convert from feet
            let alt_meters = (*alt as f64) * FEET_TO_METERS;
            writeln!(file, "        {},{},{}", lon, lat, alt_meters)
                .map_err(|e| format!("Write error: {}", e))?;
        }

        writeln!(file, "      </coordinates>").map_err(|e| format!("Write error: {}", e))?;
        writeln!(file, "    </LineString>").map_err(|e| format!("Write error: {}", e))?;
        writeln!(file, "  </Placemark>").map_err(|e| format!("Write error: {}", e))?;
    }

    writeln!(file, "</Document>").map_err(|e| format!("Write error: {}", e))?;
    writeln!(file, "</kml>").map_err(|e| format!("Write error: {}", e))?;

    info!("Exported {} aircraft tracks to KML", aircraft_tracks.len());
    Ok(())
}

/// Export flight data to CSV format
pub fn export_to_csv(frames: &[RecordedFrame], output_path: &Path) -> Result<(), String> {
    let mut file =
        File::create(output_path).map_err(|e| format!("Failed to create file: {}", e))?;

    // CSV header
    writeln!(
        file,
        "timestamp_ms,icao,callsign,latitude,longitude,altitude_ft,heading,velocity_kts,vertical_rate,squawk"
    )
    .map_err(|e| format!("Write error: {}", e))?;

    let mut row_count = 0;
    for frame in frames {
        for aircraft in &frame.aircraft {
            writeln!(
                file,
                "{},{},{},{},{},{},{},{},{},{}",
                frame.timestamp_ms,
                aircraft.icao,
                aircraft.callsign.as_deref().unwrap_or(""),
                aircraft.latitude,
                aircraft.longitude,
                aircraft.altitude.map(|a| a.to_string()).unwrap_or_default(),
                aircraft
                    .heading
                    .map(|h| format!("{:.1}", h))
                    .unwrap_or_default(),
                aircraft
                    .velocity
                    .map(|v| format!("{:.1}", v))
                    .unwrap_or_default(),
                aircraft
                    .vertical_rate
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                aircraft.squawk.as_deref().unwrap_or(""),
            )
            .map_err(|e| format!("Write error: {}", e))?;
            row_count += 1;
        }
    }

    info!("Exported {} rows to CSV", row_count);
    Ok(())
}

/// Export flight data to GeoJSON format
pub fn export_to_geojson(frames: &[RecordedFrame], output_path: &Path) -> Result<(), String> {
    use std::collections::HashMap;

    let mut file =
        File::create(output_path).map_err(|e| format!("Failed to create file: {}", e))?;

    // Group by aircraft
    let mut aircraft_tracks: HashMap<String, Vec<(f64, f64, Option<i32>)>> = HashMap::new();
    let mut aircraft_callsigns: HashMap<String, Option<String>> = HashMap::new();

    for frame in frames {
        for aircraft in &frame.aircraft {
            aircraft_tracks
                .entry(aircraft.icao.clone())
                .or_default()
                .push((aircraft.longitude, aircraft.latitude, aircraft.altitude));

            if aircraft.callsign.is_some() {
                aircraft_callsigns.insert(aircraft.icao.clone(), aircraft.callsign.clone());
            }
        }
    }

    // Build GeoJSON
    writeln!(file, r#"{{"type": "FeatureCollection", "features": ["#)
        .map_err(|e| format!("Write error: {}", e))?;

    let mut first = true;
    for (icao, positions) in &aircraft_tracks {
        if positions.len() < 2 {
            continue;
        }

        if !first {
            writeln!(file, ",").map_err(|e| format!("Write error: {}", e))?;
        }
        first = false;

        let callsign = aircraft_callsigns.get(icao).and_then(|c| c.clone());

        writeln!(file, r#"  {{"type": "Feature","#).map_err(|e| format!("Write error: {}", e))?;
        writeln!(
            file,
            r#"   "properties": {{"icao": "{}", "callsign": {}}},"#,
            icao,
            callsign
                .map(|c| format!(r#""{}""#, c))
                .unwrap_or("null".to_string())
        )
        .map_err(|e| format!("Write error: {}", e))?;
        writeln!(
            file,
            r#"   "geometry": {{"type": "LineString", "coordinates": ["#
        )
        .map_err(|e| format!("Write error: {}", e))?;

        for (i, (lon, lat, _alt)) in positions.iter().enumerate() {
            let comma = if i < positions.len() - 1 { "," } else { "" };
            writeln!(file, "     [{}, {}]{}", lon, lat, comma)
                .map_err(|e| format!("Write error: {}", e))?;
        }

        writeln!(file, r#"   ]}}}}"#).map_err(|e| format!("Write error: {}", e))?;
    }

    writeln!(file, "]}}").map_err(|e| format!("Write error: {}", e))?;

    info!(
        "Exported {} aircraft tracks to GeoJSON",
        aircraft_tracks.len()
    );
    Ok(())
}

/// Load recorded frames from NDJSON file
pub fn load_recording(path: &Path) -> Result<Vec<RecordedFrame>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
    let reader = BufReader::new(file);

    let mut frames = Vec::new();
    for (line_num, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("Read error at line {}: {}", line_num + 1, e))?;
        if line.trim().is_empty() {
            continue;
        }

        let frame: RecordedFrame = serde_json::from_str(&line)
            .map_err(|e| format!("Parse error at line {}: {}", line_num + 1, e))?;
        frames.push(frame);
    }

    info!("Loaded {} frames from recording", frames.len());
    Ok(frames)
}

/// Export recording to specified format
pub fn export_recording(
    recording_path: &Path,
    output_path: &Path,
    format: ExportFormat,
) -> Result<(), String> {
    let frames = load_recording(recording_path)?;

    match format {
        ExportFormat::KML => export_to_kml(&frames, output_path),
        ExportFormat::CSV => export_to_csv(&frames, output_path),
        ExportFormat::GeoJSON => export_to_geojson(&frames, output_path),
    }
}

/// System to render export panel
pub fn render_export_panel(mut contexts: EguiContexts, mut export_state: ResMut<ExportState>) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    if !export_state.panel_open {
        return;
    }

    egui::Window::new("Export Data")
        .collapsible(true)
        .resizable(false)
        .default_width(300.0)
        .show(ctx, |ui| {
            ui.label("Export recorded flight data to various formats.");
            ui.separator();

            // Format selection
            ui.label("Format:");
            egui::ComboBox::from_id_salt("export_format")
                .selected_text(export_state.format.display_name())
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut export_state.format,
                        ExportFormat::KML,
                        ExportFormat::KML.display_name(),
                    );
                    ui.selectable_value(
                        &mut export_state.format,
                        ExportFormat::CSV,
                        ExportFormat::CSV.display_name(),
                    );
                    ui.selectable_value(
                        &mut export_state.format,
                        ExportFormat::GeoJSON,
                        ExportFormat::GeoJSON.display_name(),
                    );
                });

            ui.add_space(8.0);

            // List available recordings
            ui.label("Available Recordings:");

            let recordings = list_available_recordings();
            if recordings.is_empty() {
                ui.label(
                    egui::RichText::new("No recordings found in tmp/").color(egui::Color32::GRAY),
                );
            } else {
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for recording in &recordings {
                            let name = recording.file_name().unwrap_or_default().to_string_lossy();

                            ui.horizontal(|ui| {
                                ui.label(&*name);
                                if ui.button("Export").clicked() {
                                    let output_name = format!(
                                        "{}.{}",
                                        name.trim_end_matches(".ndjson"),
                                        export_state.format.extension()
                                    );
                                    let output_path = recording
                                        .parent()
                                        .unwrap_or(Path::new("."))
                                        .join(&output_name);

                                    match export_recording(
                                        recording,
                                        &output_path,
                                        export_state.format,
                                    ) {
                                        Ok(()) => {
                                            export_state.status_message =
                                                Some(format!("Exported to {}", output_name));
                                            export_state.last_export_path = Some(output_path);
                                        }
                                        Err(e) => {
                                            export_state.status_message =
                                                Some(format!("Error: {}", e));
                                        }
                                    }
                                }
                            });
                        }
                    });
            }

            ui.separator();

            // Status message
            if let Some(ref msg) = export_state.status_message {
                let color = if msg.starts_with("Error") {
                    egui::Color32::RED
                } else {
                    egui::Color32::GREEN
                };
                ui.colored_label(color, msg);
            }

            // Close button
            ui.separator();
            if ui.button("Close").clicked() {
                export_state.panel_open = false;
            }
        });
}

/// List available recording files
pub fn list_available_recordings() -> Vec<PathBuf> {
    let tmp_dir = crate::paths::data_dir();

    if !tmp_dir.exists() {
        return Vec::new();
    }

    std::fs::read_dir(&tmp_dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "ndjson")
                        .unwrap_or(false)
                })
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

/// System to toggle export panel
pub fn toggle_export_panel(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut export_state: ResMut<ExportState>,
    mut contexts: EguiContexts,
) {
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    // E - Toggle export panel
    if keyboard.just_pressed(KeyCode::KeyE) {
        export_state.panel_open = !export_state.panel_open;
    }
}

/// Plugin for export functionality
pub struct ExportPlugin;

impl Plugin for ExportPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ExportState>()
            .add_systems(Update, toggle_export_panel);
        // Export panel is rendered via the consolidated Tools window (tools_window.rs)
    }
}
