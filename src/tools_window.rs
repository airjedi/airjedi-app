/// Consolidated "Tools" window with tabs for infrequent-use feature panels.
///
/// Groups Coverage, Airspace, Data Sources, Export, and 3D View into a single
/// tabbed window to reduce floating window clutter.
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::path::Path;

use crate::airspace::{AirspaceData, AirspaceDisplayState};
use crate::coverage::CoverageState;
use crate::data_sources::DataSourceManager;
use crate::export::{ExportFormat, ExportState};
use crate::recording::{PlaybackState, RecordingState};
use crate::terrain::TerrainState;
use crate::theme::{to_egui_color32, to_egui_color32_alpha, AppTheme};
use crate::tiles::GridOverlay;
use crate::view3d::{
    sky::{SunState, TimeState},
    View3DState, ViewMode,
};

/// Which tab is currently active in the tools window.
#[derive(Resource, Default, PartialEq, Eq, Clone, Copy)]
pub enum ToolsTab {
    #[default]
    Coverage,
    Airspace,
    DataSources,
    Export,
    Recording,
    View3D,
    Ingest,
}

/// Whether the tools window is open.
#[derive(Resource, Default)]
pub struct ToolsWindowState {
    pub open: bool,
    pub active_tab: ToolsTab,
}

/// System to render the consolidated tools window.
///
/// The tools window is controlled by `ToolsWindowState.open` and always shows
/// ALL tabs when open. Toolbar buttons and keyboard shortcuts open the window
/// and switch to the relevant tab.
pub fn render_tools_window(
    mut contexts: EguiContexts,
    mut tools_state: ResMut<ToolsWindowState>,
    mut coverage: ResMut<CoverageState>,
    mut airspace_display: ResMut<AirspaceDisplayState>,
    mut airspace_data: ResMut<AirspaceData>,
    mut datasource_mgr: ResMut<DataSourceManager>,
    mut export_state: ResMut<ExportState>,
    mut recording: ResMut<RecordingState>,
    mut playback: ResMut<PlaybackState>,
    mut view3d_state: ResMut<View3DState>,
    mut terrain_state: ResMut<TerrainState>,
    mut time_state: ResMut<TimeState>,
    sun_state: Res<SunState>,
    theme: Res<AppTheme>,
    mut app_config: ResMut<crate::config::AppConfig>,
    ingest_status: Option<Res<crate::data_ingest::IngestStatus>>,
    mut ingest_ui: Option<ResMut<crate::data_ingest::IngestUiState>>,
    mut grid_overlay: Option<ResMut<GridOverlay>>,
) {
    if !tools_state.open {
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let panel_bg = to_egui_color32_alpha(theme.bg_secondary(), 240);
    let border_color = to_egui_color32(theme.bg_contrast());

    let window_frame = egui::Frame::default()
        .fill(panel_bg)
        .stroke(egui::Stroke::new(1.0, border_color))
        .inner_margin(egui::Margin::same(8));

    let mut window_open = true;
    egui::Window::new("Tools")
        .open(&mut window_open)
        .collapsible(true)
        .resizable(true)
        .default_width(320.0)
        .default_height(350.0)
        .frame(window_frame)
        .show(ctx, |ui| {
            // Tab bar - always show all tabs
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(tools_state.active_tab == ToolsTab::Coverage, "Coverage")
                    .clicked()
                {
                    tools_state.active_tab = ToolsTab::Coverage;
                }
                if ui
                    .selectable_label(tools_state.active_tab == ToolsTab::Airspace, "Airspace")
                    .clicked()
                {
                    tools_state.active_tab = ToolsTab::Airspace;
                }
                if ui
                    .selectable_label(tools_state.active_tab == ToolsTab::DataSources, "Sources")
                    .clicked()
                {
                    tools_state.active_tab = ToolsTab::DataSources;
                }
                if ui
                    .selectable_label(tools_state.active_tab == ToolsTab::Export, "Export")
                    .clicked()
                {
                    tools_state.active_tab = ToolsTab::Export;
                }
                if ui
                    .selectable_label(tools_state.active_tab == ToolsTab::Recording, "Recording")
                    .clicked()
                {
                    tools_state.active_tab = ToolsTab::Recording;
                }
                if ui
                    .selectable_label(tools_state.active_tab == ToolsTab::View3D, "3D View")
                    .clicked()
                {
                    tools_state.active_tab = ToolsTab::View3D;
                }
                if ui
                    .selectable_label(tools_state.active_tab == ToolsTab::Ingest, "Ingest")
                    .clicked()
                {
                    tools_state.active_tab = ToolsTab::Ingest;
                }
            });

            ui.separator();

            // Tab content
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match tools_state.active_tab {
                    ToolsTab::Coverage => render_coverage_tab(ui, &mut coverage),
                    ToolsTab::Airspace => {
                        render_airspace_tab(ui, &mut airspace_display, &mut airspace_data)
                    }
                    ToolsTab::DataSources => render_data_sources_tab(ui, &mut datasource_mgr),
                    ToolsTab::Export => render_export_tab(ui, &mut export_state),
                    ToolsTab::Recording => render_recording_tab(ui, &mut recording, &mut playback),
                    ToolsTab::View3D => render_view3d_tab(
                        ui,
                        &mut view3d_state,
                        &mut terrain_state,
                        &mut time_state,
                        &sun_state,
                        grid_overlay.as_deref_mut(),
                    ),
                    ToolsTab::Ingest => render_ingest_tab(
                        ui,
                        ingest_status.as_deref(),
                        &mut app_config,
                        ingest_ui.as_deref_mut(),
                    ),
                });
        });

    // Handle close via egui's window X button
    if !window_open {
        tools_state.open = false;
    }
}

pub fn render_coverage_tab(ui: &mut egui::Ui, coverage: &mut CoverageState) {
    let stats = coverage.get_stats();

    ui.horizontal(|ui| {
        ui.label("Tracking: ");
        if coverage.enabled {
            ui.colored_label(egui::Color32::GREEN, "ACTIVE");
        } else {
            ui.colored_label(egui::Color32::GRAY, "INACTIVE");
        }
    });

    ui.separator();

    egui::Grid::new("coverage_stats_grid_tab")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Max Range:");
            ui.label(format!("{:.1} NM", stats.max_range_nm));
            ui.end_row();

            ui.label("Avg Max Range:");
            ui.label(format!("{:.1} NM", stats.avg_max_range_nm));
            ui.end_row();

            ui.label("Active Sectors:");
            ui.label(format!("{}/{}", stats.active_sectors, stats.total_sectors));
            ui.end_row();

            ui.label("Unique Aircraft:");
            ui.label(format!("{}", stats.unique_aircraft));
            ui.end_row();

            ui.label("Total Observations:");
            ui.label(format!("{}", stats.total_observations));
            ui.end_row();
        });

    ui.separator();

    ui.horizontal(|ui| {
        let enable_text = if coverage.enabled {
            "Disable"
        } else {
            "Enable"
        };
        if ui.button(enable_text).clicked() {
            coverage.enabled = !coverage.enabled;
        }
        if ui.button("Reset").clicked() {
            coverage.reset();
        }
    });

    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Receiver:");
        ui.label(format!(
            "{:.4}, {:.4}",
            coverage.receiver_location.0, coverage.receiver_location.1
        ));
    });
}

pub fn render_airspace_tab(
    ui: &mut egui::Ui,
    display_state: &mut AirspaceDisplayState,
    airspace_data: &mut AirspaceData,
) {
    if !airspace_data.loaded {
        ui.label("No airspace data loaded");
        ui.separator();
        if ui.button("Load Sample Data").clicked() {
            airspace_data.load_sample_data();
        }
        ui.label(
            egui::RichText::new(
                "Note: Full implementation requires\nintegration with FAA/OpenAIP data",
            )
            .size(11.0)
            .color(egui::Color32::GRAY),
        );
    } else {
        if let Some(ref source) = airspace_data.source {
            ui.label(format!("Source: {}", source));
        }
        ui.label(format!(
            "{} airspaces loaded",
            airspace_data.airspaces.len()
        ));
        ui.separator();
        ui.label("Display Options:");
        ui.checkbox(&mut display_state.show_class_b, "Class B");
        ui.checkbox(&mut display_state.show_class_c, "Class C");
        ui.checkbox(&mut display_state.show_class_d, "Class D");
        ui.checkbox(&mut display_state.show_restricted, "Restricted");
        ui.checkbox(&mut display_state.show_moa, "MOA");
        ui.checkbox(&mut display_state.show_tfr, "TFR");
        ui.checkbox(&mut display_state.show_warning, "Warning");
        ui.checkbox(&mut display_state.show_alert, "Alert");

        ui.separator();
        ui.add(egui::Slider::new(&mut display_state.opacity, 0.05..=1.0).text("Opacity"));

        ui.horizontal(|ui| {
            let mut use_filter = display_state.altitude_filter_ft.is_some();
            if ui.checkbox(&mut use_filter, "Alt Filter").changed() {
                display_state.altitude_filter_ft = if use_filter { Some(10000) } else { None };
            }
            if let Some(ref mut alt) = display_state.altitude_filter_ft {
                ui.add(egui::DragValue::new(alt).range(0..=60000).suffix(" ft"));
            }
        });

        ui.separator();
        ui.checkbox(&mut display_state.show_labels, "Show Labels");
    }
}

pub fn render_data_sources_tab(ui: &mut egui::Ui, manager: &mut DataSourceManager) {
    let stats = manager.get_stats();
    ui.label(format!(
        "{}/{} sources connected, {} aircraft",
        stats.connected_sources, stats.total_sources, stats.total_aircraft
    ));

    ui.separator();

    for source in &manager.sources {
        let status = manager.states.get(&source.name);
        let status_text = status
            .map(|s| format!("{:?}", s.status))
            .unwrap_or_else(|| "Unknown".to_string());

        ui.horizontal(|ui| {
            let enabled_icon = if source.enabled {
                "\u{25CF}"
            } else {
                "\u{25CB}"
            };
            ui.label(enabled_icon);
            ui.label(&source.name);
            ui.label(
                egui::RichText::new(&status_text)
                    .size(10.0)
                    .color(egui::Color32::GRAY),
            );
        });

        ui.label(
            egui::RichText::new(format!("  {}", source.endpoint))
                .size(10.0)
                .color(egui::Color32::from_rgb(150, 150, 150)),
        );
    }
}

pub fn render_export_tab(ui: &mut egui::Ui, export_state: &mut ExportState) {
    ui.label("Export recorded flight data to various formats.");
    ui.separator();

    ui.label("Format:");
    egui::ComboBox::from_id_salt("export_format_tab")
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
    ui.label("Available Recordings:");

    let recordings = crate::export::list_available_recordings();
    if recordings.is_empty() {
        ui.label(egui::RichText::new("No recordings found in tmp/").color(egui::Color32::GRAY));
    } else {
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

                    match crate::export::export_recording(
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
                            export_state.status_message = Some(format!("Error: {}", e));
                        }
                    }
                }
            });
        }
    }

    if let Some(ref msg) = export_state.status_message {
        ui.separator();
        let color = if msg.starts_with("Error") {
            egui::Color32::RED
        } else {
            egui::Color32::GREEN
        };
        ui.colored_label(color, msg);
    }
}

pub fn render_view3d_tab(
    ui: &mut egui::Ui,
    state: &mut View3DState,
    terrain: &mut TerrainState,
    time_state: &mut TimeState,
    sun_state: &SunState,
    mut grid_overlay: Option<&mut GridOverlay>,
) {
    ui.colored_label(
        egui::Color32::YELLOW,
        "This feature is in research/prototype stage",
    );
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("View Mode:");
        if ui
            .selectable_label(state.mode == ViewMode::Map2D, "2D Map")
            .clicked()
        {
            state.mode = ViewMode::Map2D;
        }
        if ui
            .selectable_label(state.mode == ViewMode::Perspective3D, "3D View")
            .clicked()
        {
            state.mode = ViewMode::Perspective3D;
        }
    });

    if state.mode == ViewMode::Perspective3D {
        ui.colored_label(egui::Color32::RED, "3D rendering not yet implemented");
    }

    ui.separator();
    ui.label("Camera Settings (for future use):");

    ui.horizontal(|ui| {
        ui.label("Pitch:");
        ui.add(egui::Slider::new(&mut state.camera_pitch, 15.0..=89.0).suffix("\u{00B0}"));
    });
    ui.horizontal(|ui| {
        ui.label("Altitude:");
        ui.add(egui::Slider::new(&mut state.camera_altitude, 1000.0..=60000.0).suffix(" ft"));
    });
    ui.horizontal(|ui| {
        ui.label("Yaw:");
        ui.add(egui::Slider::new(&mut state.camera_yaw, 0.0..=360.0).suffix("\u{00B0}"));
    });
    ui.horizontal(|ui| {
        ui.label("Alt Scale:");
        ui.add(egui::Slider::new(&mut state.altitude_scale, 0.1..=100.0));
    });

    ui.separator();
    ui.label("Ground Elevation:");

    if let Some(ref name) = state.detected_airport_name {
        ui.label(
            egui::RichText::new(format!("Nearest: {}", name))
                .size(11.0)
                .color(egui::Color32::LIGHT_BLUE),
        );
    } else {
        ui.label(
            egui::RichText::new("No nearby airport detected")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
    }

    ui.horizontal(|ui| {
        ui.label("Elevation:");
        let mut elev = state.ground_elevation_ft as f32;
        if ui
            .add(egui::Slider::new(&mut elev, 0.0..=15000.0).suffix(" ft"))
            .changed()
        {
            state.ground_elevation_ft = elev as i32;
        }
    });

    ui.separator();
    ui.label("Atmosphere:");

    ui.checkbox(&mut state.atmosphere_enabled, "Enable atmosphere effects");

    if state.atmosphere_enabled {
        ui.horizontal(|ui| {
            ui.label("Visibility:");
            ui.add(
                egui::Slider::new(&mut state.visibility_range, 1000.0..=20000.0)
                    .suffix(" units")
                    .logarithmic(true),
            );
        });
    }

    ui.separator();
    ui.label("Tiles:");

    if let Some(ref mut grid) = grid_overlay {
        ui.checkbox(&mut grid.enabled, "Show grid overlay");
    }

    ui.separator();
    ui.label("Terrain:");

    ui.checkbox(&mut terrain.enabled, "Enable 3D terrain");

    if terrain.enabled {
        ui.checkbox(&mut terrain.gpu_terrain, "GPU vertex displacement");
        if !terrain.gpu_terrain {
            ui.horizontal(|ui| {
                ui.label("Resolution:");
                egui::ComboBox::from_id_salt("terrain_res")
                    .selected_text(format!(
                        "{}x{}",
                        terrain.mesh_resolution, terrain.mesh_resolution
                    ))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut terrain.mesh_resolution, 16, "16x16");
                        ui.selectable_value(&mut terrain.mesh_resolution, 32, "32x32");
                        ui.selectable_value(&mut terrain.mesh_resolution, 64, "64x64");
                    });
            });
        }
    }

    ui.separator();
    crate::view3d::render_time_of_day_section(ui, time_state, sun_state);
}

pub fn render_recording_tab(
    ui: &mut egui::Ui,
    recording: &mut RecordingState,
    playback: &mut PlaybackState,
) {
    // Record / Stop controls
    ui.horizontal(|ui| {
        if recording.is_recording {
            if ui.button("Stop Recording").clicked() {
                recording.stop();
            }
            ui.label(
                egui::RichText::new(format!(
                    "{} frames | {}s",
                    recording.frame_count,
                    recording.duration_secs()
                ))
                .color(egui::Color32::LIGHT_RED),
            );
        } else {
            if ui.button("Record").clicked() {
                if let Err(e) = recording.start() {
                    error!("Failed to start recording: {}", e);
                }
            }
        }
    });

    if let Some(ref path) = recording.file_path {
        if !recording.is_recording {
            ui.label(
                egui::RichText::new(format!(
                    "Last: {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ))
                .size(10.0)
                .color(egui::Color32::GRAY),
            );
        }
    }

    ui.separator();

    // Playback controls
    ui.label("Playback");

    if playback.is_playing {
        ui.horizontal(|ui| {
            if playback.is_paused {
                if ui.button("Resume").clicked() {
                    playback.resume();
                }
            } else {
                if ui.button("Pause").clicked() {
                    playback.pause();
                }
            }
            if ui.button("Stop").clicked() {
                playback.stop();
            }
        });

        // Speed controls
        ui.horizontal(|ui| {
            ui.label("Speed:");
            for speed in [0.5, 1.0, 2.0, 4.0] {
                let label = format!("{}x", speed);
                if ui
                    .selectable_label((playback.speed - speed).abs() < 0.01, &label)
                    .clicked()
                {
                    playback.speed = speed;
                }
            }
        });

        // Progress bar
        if playback.total_duration_ms > 0 {
            let progress = playback.current_time_ms as f32 / playback.total_duration_ms as f32;
            ui.add(egui::ProgressBar::new(progress).show_percentage());

            let current_secs = playback.current_time_ms / 1000;
            let total_secs = playback.total_duration_ms / 1000;
            ui.label(format!(
                "{}:{:02} / {}:{:02}",
                current_secs / 60,
                current_secs % 60,
                total_secs / 60,
                total_secs % 60
            ));
        }
    } else {
        if ui.button("Load Recording...").clicked() {
            // List available recordings
            let data_dir = crate::paths::data_dir();
            if data_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&data_dir) {
                    let recordings: Vec<_> = entries
                        .filter_map(|e| e.ok())
                        .filter(|e| {
                            e.path()
                                .extension()
                                .map(|ext| ext == "ndjson")
                                .unwrap_or(false)
                        })
                        .collect();

                    if let Some(latest) = recordings.last() {
                        if let Err(e) = playback.load(&latest.path()) {
                            error!("Failed to load recording: {}", e);
                        }
                    }
                }
            }
        }
    }
}

pub fn render_ingest_tab(
    ui: &mut egui::Ui,
    ingest_status: Option<&crate::data_ingest::IngestStatus>,
    app_config: &mut crate::config::AppConfig,
    ingest_ui: Option<&mut crate::data_ingest::IngestUiState>,
) {
    use crate::data_ingest::provider::{ProviderCategory, ProviderStatus, SchedulePreset};

    // Build a list of all configurable providers with their metadata.
    // We iterate provider_entries() which covers all config keys regardless of
    // whether the provider is currently running (so disabled ones still appear).
    // Open data folder button
    let data_dir = crate::paths::data_dir();
    ui.horizontal(|ui| {
        if ui.button("Open Data Folder").clicked() {
            let _ = std::fs::create_dir_all(&data_dir);
            let _ = std::process::Command::new("open").arg(&data_dir).spawn();
        }
        ui.label(
            egui::RichText::new(data_dir.to_string_lossy().to_string())
                .size(10.0)
                .color(egui::Color32::GRAY),
        );
    });
    ui.separator();

    let provider_entries = provider_entries();

    let mut changed = false;
    let mut fetch_keys: Vec<String> = Vec::new();

    for category in ProviderCategory::all() {
        let entries: Vec<_> = provider_entries
            .iter()
            .filter(|e| e.category == *category)
            .collect();

        if entries.is_empty() {
            continue;
        }

        egui::CollapsingHeader::new(category.display_name())
            .default_open(true)
            .show(ui, |ui| {
                for entry in &entries {
                    let provider_config =
                        get_provider_config_mut(&mut app_config.data_ingest, entry.config_key);

                    // Status indicator from live IngestStatus (if available)
                    let status_info = ingest_status.and_then(|s| {
                        s.providers
                            .iter()
                            .find(|p| p.config_key == entry.config_key)
                    });

                    let (status_color, status_text) = match status_info.map(|s| &s.status) {
                        Some(ProviderStatus::Idle) | None => {
                            (egui::Color32::GRAY, "Idle".to_string())
                        }
                        Some(ProviderStatus::Fetching) => {
                            (egui::Color32::YELLOW, "Fetching...".to_string())
                        }
                        Some(ProviderStatus::Ok {
                            last_success,
                            record_count,
                        }) => {
                            let time = last_success.format("%H:%M").to_string();
                            (
                                egui::Color32::GREEN,
                                format!("{} ({} records)", time, record_count),
                            )
                        }
                        Some(ProviderStatus::Error { message, .. }) => {
                            (egui::Color32::RED, format!("Error: {}", message))
                        }
                    };

                    // Header row: enabled checkbox + name + run button + status
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut provider_config.enabled, "").changed() {
                            changed = true;
                        }
                        ui.colored_label(status_color, "\u{25CF}");
                        ui.strong(entry.display_name);

                        // Play button for on-demand fetch
                        if provider_config.enabled {
                            let play_btn = ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("\u{25B6}")
                                            .size(12.0)
                                            .color(egui::Color32::from_rgb(80, 200, 80)),
                                    )
                                    .min_size(egui::vec2(20.0, 16.0)),
                                )
                                .on_hover_text("Run now");

                            if play_btn.clicked() {
                                fetch_keys.push(entry.config_key.to_string());
                            }
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(&status_text)
                                    .size(10.0)
                                    .color(egui::Color32::GRAY),
                            );
                        });
                    });

                    // Expandable settings per provider
                    ui.add_enabled_ui(provider_config.enabled, |ui| {
                        ui.indent(entry.config_key, |ui| {
                            // Description
                            ui.label(
                                egui::RichText::new(entry.description)
                                    .size(11.0)
                                    .color(egui::Color32::from_rgb(160, 160, 160)),
                            );

                            // Schedule selector
                            let current_preset =
                                SchedulePreset::from_cron(&provider_config.schedule);
                            ui.horizontal(|ui| {
                                ui.label("Schedule:");
                                let combo_id = format!("schedule_{}", entry.config_key);
                                egui::ComboBox::from_id_salt(&combo_id)
                                    .selected_text(current_preset.display_name())
                                    .show_ui(ui, |ui| {
                                        for preset in SchedulePreset::all() {
                                            if *preset == SchedulePreset::Custom {
                                                continue; // custom shown via text field below
                                            }
                                            if ui
                                                .selectable_label(
                                                    current_preset == *preset,
                                                    preset.display_name(),
                                                )
                                                .clicked()
                                            {
                                                provider_config.schedule =
                                                    preset.to_cron().to_string();
                                                changed = true;
                                            }
                                        }
                                    });
                            });

                            // Cron expression (editable for Custom or display for presets)
                            if current_preset == SchedulePreset::Custom {
                                ui.horizontal(|ui| {
                                    ui.label("Cron:");
                                    if ui
                                        .text_edit_singleline(&mut provider_config.schedule)
                                        .lost_focus()
                                    {
                                        changed = true;
                                    }
                                });
                            }

                            // API credentials (for providers that need them)
                            if entry.needs_credentials {
                                ui.add_space(4.0);
                                let mut key = provider_config.api_key.clone().unwrap_or_default();
                                let mut secret =
                                    provider_config.api_secret.clone().unwrap_or_default();

                                ui.horizontal(|ui| {
                                    ui.label("API Key:");
                                    if ui.text_edit_singleline(&mut key).lost_focus() {
                                        provider_config.api_key = if key.is_empty() {
                                            None
                                        } else {
                                            Some(key.clone())
                                        };
                                        changed = true;
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.label("API Secret:");
                                    if ui
                                        .add(egui::TextEdit::singleline(&mut secret).password(true))
                                        .lost_focus()
                                    {
                                        provider_config.api_secret = if secret.is_empty() {
                                            None
                                        } else {
                                            Some(secret.clone())
                                        };
                                        changed = true;
                                    }
                                });

                                if key.is_empty() || secret.is_empty() {
                                    ui.label(
                                        egui::RichText::new(entry.credentials_hint)
                                            .size(10.0)
                                            .color(egui::Color32::from_rgb(200, 180, 80)),
                                    );
                                }
                            }
                        });
                    });

                    ui.add_space(4.0);
                }
            });

        ui.add_space(4.0);
    }

    // Note about restart requirement
    ui.separator();
    ui.label(
        egui::RichText::new("Changes to enabled/disabled providers take effect on restart.")
            .size(10.0)
            .color(egui::Color32::from_rgb(180, 180, 100)),
    );

    if let Some(ui_state) = ingest_ui {
        ui_state.pending_fetches.extend(fetch_keys);
    }

    if changed {
        crate::config::save_config(app_config);
    }
}

/// Static metadata for each configurable provider.
struct ProviderEntry {
    config_key: &'static str,
    display_name: &'static str,
    description: &'static str,
    category: crate::data_ingest::provider::ProviderCategory,
    needs_credentials: bool,
    credentials_hint: &'static str,
}

fn provider_entries() -> Vec<ProviderEntry> {
    use crate::data_ingest::provider::ProviderCategory;
    vec![
        ProviderEntry {
            config_key: "metar",
            display_name: "METAR / Weather",
            description: "METARs, TAFs, SIGMETs, AIRMETs, PIREPs from aviationweather.gov",
            category: ProviderCategory::Weather,
            needs_credentials: false,
            credentials_hint: "",
        },
        ProviderEntry {
            config_key: "taf",
            display_name: "TAF (standalone)",
            description: "Terminal Aerodrome Forecasts (when METAR group is disabled)",
            category: ProviderCategory::Weather,
            needs_credentials: false,
            credentials_hint: "",
        },
        ProviderEntry {
            config_key: "ourairports",
            display_name: "OurAirports",
            description: "Airports, runways, and navaids from OurAirports open data",
            category: ProviderCategory::Navigation,
            needs_credentials: false,
            credentials_hint: "",
        },
        ProviderEntry {
            config_key: "faa_nasr",
            display_name: "FAA NASR",
            description: "Airways and frequencies from FAA NASR 28-day subscription (CSV)",
            category: ProviderCategory::Navigation,
            needs_credentials: false,
            credentials_hint: "",
        },
        ProviderEntry {
            config_key: "openaip",
            display_name: "OpenAIP",
            description: "International airport and airspace data from OpenAIP",
            category: ProviderCategory::Navigation,
            needs_credentials: false,
            credentials_hint: "",
        },
        ProviderEntry {
            config_key: "notam",
            display_name: "NOTAMs",
            description: "Notices to Air Missions from FAA NOTAM API v1",
            category: ProviderCategory::Notices,
            needs_credentials: true,
            credentials_hint: "Register free at https://api.faa.gov/s/",
        },
        ProviderEntry {
            config_key: "tfr",
            display_name: "TFRs",
            description: "Temporary Flight Restrictions from FAA GeoServer",
            category: ProviderCategory::Notices,
            needs_credentials: false,
            credentials_hint: "",
        },
        ProviderEntry {
            config_key: "faa_airspace",
            display_name: "FAA Airspace",
            description: "Class B/C and special use airspace from FAA ADDS",
            category: ProviderCategory::Navigation,
            needs_credentials: false,
            credentials_hint: "",
        },
    ]
}

fn get_provider_config_mut<'a>(
    config: &'a mut crate::config::DataIngestConfig,
    key: &str,
) -> &'a mut crate::config::ProviderConfig {
    match key {
        "metar" => &mut config.metar,
        "taf" => &mut config.taf,
        "ourairports" => &mut config.ourairports,
        "faa_nasr" => &mut config.faa_nasr,
        "openaip" => &mut config.openaip,
        "notam" => &mut config.notam,
        "tfr" => &mut config.tfr,
        "faa_airspace" => &mut config.faa_airspace,
        _ => panic!("unknown provider config key: {}", key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::CoverageState;
    use crate::view3d::{View3DState, ViewMode};
    use egui_kittest::{kittest::Queryable, Harness};

    /// Bundled state for view3d tab tests since render_view3d_tab now
    /// requires View3DState, TimeState, and SunState parameters.
    struct View3DTabTestState {
        view3d: View3DState,
        terrain: crate::terrain::TerrainState,
        time: TimeState,
        sun: SunState,
    }

    impl Default for View3DTabTestState {
        fn default() -> Self {
            Self {
                view3d: View3DState::default(),
                terrain: crate::terrain::TerrainState::default(),
                time: TimeState::default(),
                sun: SunState::default(),
            }
        }
    }

    #[test]
    fn test_coverage_tab_shows_inactive() {
        let harness = Harness::new_ui_state(
            |ui, state: &mut CoverageState| {
                render_coverage_tab(ui, state);
            },
            CoverageState::default(),
        );

        harness.get_by_label("INACTIVE");
    }

    #[test]
    fn test_coverage_tab_shows_enable_button() {
        let harness = Harness::new_ui_state(
            |ui, state: &mut CoverageState| {
                render_coverage_tab(ui, state);
            },
            CoverageState::default(),
        );

        harness.get_by_label("Enable");
    }

    #[test]
    fn test_view3d_tab_shows_pitch_label() {
        let harness = Harness::new_ui_state(
            |ui, state: &mut View3DTabTestState| {
                render_view3d_tab(
                    ui,
                    &mut state.view3d,
                    &mut state.terrain,
                    &mut state.time,
                    &state.sun,
                    None,
                );
            },
            View3DTabTestState::default(),
        );

        harness.get_by_label("Pitch:");
    }

    #[test]
    fn test_view3d_tab_shows_altitude_label() {
        let harness = Harness::new_ui_state(
            |ui, state: &mut View3DTabTestState| {
                render_view3d_tab(
                    ui,
                    &mut state.view3d,
                    &mut state.terrain,
                    &mut state.time,
                    &state.sun,
                    None,
                );
            },
            View3DTabTestState::default(),
        );

        harness.get_by_label("Altitude:");
    }

    #[test]
    fn test_view3d_tab_shows_mode_selectable_labels() {
        let harness = Harness::new_ui_state(
            |ui, state: &mut View3DTabTestState| {
                render_view3d_tab(
                    ui,
                    &mut state.view3d,
                    &mut state.terrain,
                    &mut state.time,
                    &state.sun,
                    None,
                );
            },
            View3DTabTestState::default(),
        );

        harness.get_by_label("2D Map");
        harness.get_by_label("3D View");
    }
}
