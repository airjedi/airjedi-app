//! Coverage Map Module
//!
//! Visualizes receiver coverage area based on tracked aircraft positions.
//! Uses a sector-based approach: divides the area around the receiver into
//! 36 sectors (10 degrees each) and tracks the maximum range observed in each.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use std::collections::HashMap;

use crate::geo::{haversine_distance_nm, initial_bearing};

/// Number of sectors to divide the coverage area into
const NUM_SECTORS: usize = 36;
/// Degrees per sector
const DEGREES_PER_SECTOR: f64 = 360.0 / NUM_SECTORS as f64;

/// A single coverage sector tracking max range
#[derive(Clone, Debug, Default)]
pub struct CoverageSector {
    /// Maximum range observed in nautical miles
    pub max_range_nm: f64,
    /// Number of aircraft tracked in this sector
    pub aircraft_count: u32,
    /// Average range in nautical miles
    pub avg_range_nm: f64,
    /// Running sum for average calculation
    sum_range: f64,
}

impl CoverageSector {
    /// Update sector with new aircraft observation
    pub fn observe(&mut self, range_nm: f64) {
        if range_nm > self.max_range_nm {
            self.max_range_nm = range_nm;
        }
        self.aircraft_count += 1;
        self.sum_range += range_nm;
        self.avg_range_nm = self.sum_range / self.aircraft_count as f64;
    }

    /// Reset sector statistics
    pub fn reset(&mut self) {
        self.max_range_nm = 0.0;
        self.aircraft_count = 0;
        self.avg_range_nm = 0.0;
        self.sum_range = 0.0;
    }
}

/// Resource tracking receiver coverage
#[derive(Resource)]
pub struct CoverageState {
    /// Whether coverage visualization is enabled
    pub enabled: bool,
    /// Receiver location (latitude, longitude)
    pub receiver_location: (f64, f64),
    /// Coverage sectors indexed by sector number (0-35)
    pub sectors: [CoverageSector; NUM_SECTORS],
    /// Track unique aircraft by ICAO to avoid double counting
    observed_aircraft: HashMap<String, Vec<usize>>,
    /// Whether to show coverage polygon on map
    pub show_polygon: bool,
    /// Whether to show coverage statistics panel
    pub show_stats: bool,
    /// Overall maximum range observed
    pub overall_max_range_nm: f64,
}

impl Default for CoverageState {
    fn default() -> Self {
        Self {
            enabled: false,
            receiver_location: (37.6872, -97.3301), // Default: Wichita, KS
            sectors: std::array::from_fn(|_| CoverageSector::default()),
            observed_aircraft: HashMap::new(),
            show_polygon: true,
            show_stats: false,
            overall_max_range_nm: 0.0,
        }
    }
}

impl CoverageState {
    /// Get the sector index for a given bearing (0-360 degrees)
    fn bearing_to_sector(bearing: f64) -> usize {
        let normalized = bearing.rem_euclid(360.0);
        ((normalized / DEGREES_PER_SECTOR) as usize).min(NUM_SECTORS - 1)
    }

    /// Calculate bearing from receiver to aircraft
    fn calculate_bearing(&self, aircraft_lat: f64, aircraft_lon: f64) -> f64 {
        initial_bearing(
            self.receiver_location.0,
            self.receiver_location.1,
            aircraft_lat,
            aircraft_lon,
        )
    }

    /// Calculate distance from receiver to aircraft in nautical miles
    fn calculate_range_nm(&self, aircraft_lat: f64, aircraft_lon: f64) -> f64 {
        haversine_distance_nm(
            self.receiver_location.0,
            self.receiver_location.1,
            aircraft_lat,
            aircraft_lon,
        )
    }

    /// Record an aircraft observation
    pub fn observe_aircraft(&mut self, icao: &str, latitude: f64, longitude: f64) {
        if !self.enabled {
            return;
        }

        let bearing = self.calculate_bearing(latitude, longitude);
        let range = self.calculate_range_nm(latitude, longitude);
        let sector = Self::bearing_to_sector(bearing);

        // Track which sectors this aircraft has been observed in
        let sectors_seen = self.observed_aircraft.entry(icao.to_string()).or_default();
        if !sectors_seen.contains(&sector) {
            sectors_seen.push(sector);
            self.sectors[sector].observe(range);
        } else {
            // Update max range even if already seen in this sector
            if range > self.sectors[sector].max_range_nm {
                self.sectors[sector].max_range_nm = range;
            }
        }

        if range > self.overall_max_range_nm {
            self.overall_max_range_nm = range;
        }
    }

    /// Reset all coverage data
    pub fn reset(&mut self) {
        for sector in &mut self.sectors {
            sector.reset();
        }
        self.observed_aircraft.clear();
        self.overall_max_range_nm = 0.0;
    }

    /// Get polygon points for rendering coverage area
    pub fn get_polygon_points(&self) -> Vec<(f64, f64)> {
        let mut points = Vec::with_capacity(NUM_SECTORS);
        for (i, sector) in self.sectors.iter().enumerate() {
            let bearing = (i as f64 * DEGREES_PER_SECTOR + DEGREES_PER_SECTOR / 2.0).to_radians();
            let range = sector.max_range_nm;

            if range > 0.0 {
                // Convert polar (bearing, range) to lat/lon offset
                // Approximate conversion: 1 NM = 1/60 degree
                let lat_offset = range * bearing.cos() / 60.0;
                let lon_offset =
                    range * bearing.sin() / (60.0 * self.receiver_location.0.to_radians().cos());

                points.push((
                    self.receiver_location.0 + lat_offset,
                    self.receiver_location.1 + lon_offset,
                ));
            } else {
                // No data for this sector, use receiver location
                points.push(self.receiver_location);
            }
        }
        points
    }

    /// Get coverage statistics
    pub fn get_stats(&self) -> CoverageStats {
        let active_sectors = self.sectors.iter().filter(|s| s.max_range_nm > 0.0).count();
        let total_aircraft: u32 = self.sectors.iter().map(|s| s.aircraft_count).sum();
        let avg_max_range = if active_sectors > 0 {
            self.sectors.iter().map(|s| s.max_range_nm).sum::<f64>() / active_sectors as f64
        } else {
            0.0
        };

        CoverageStats {
            max_range_nm: self.overall_max_range_nm,
            avg_max_range_nm: avg_max_range,
            active_sectors,
            total_sectors: NUM_SECTORS,
            unique_aircraft: self.observed_aircraft.len(),
            total_observations: total_aircraft as usize,
        }
    }
}

/// Summary statistics for coverage
#[derive(Debug, Clone)]
pub struct CoverageStats {
    pub max_range_nm: f64,
    pub avg_max_range_nm: f64,
    pub active_sectors: usize,
    pub total_sectors: usize,
    pub unique_aircraft: usize,
    pub total_observations: usize,
}

/// Component for coverage polygon entities
#[derive(Component)]
pub struct CoveragePolygon;

/// System to toggle coverage mode with keyboard
pub fn toggle_coverage_mode(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut coverage: ResMut<CoverageState>,
    mut contexts: EguiContexts,
) {
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    // V - Toggle coverage visualization
    if keyboard.just_pressed(KeyCode::KeyV) {
        coverage.enabled = !coverage.enabled;
        if coverage.enabled {
            info!("Coverage tracking enabled");
        } else {
            info!("Coverage tracking disabled");
        }
    }

    // Shift+V - Toggle coverage stats panel
    if keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight) {
        if keyboard.just_pressed(KeyCode::KeyV) {
            coverage.show_stats = !coverage.show_stats;
        }
    }
}

/// System to update coverage from aircraft positions
pub fn update_coverage_from_aircraft(
    mut coverage: ResMut<CoverageState>,
    aircraft_query: Query<&crate::Aircraft>,
) {
    if !coverage.enabled {
        return;
    }

    for aircraft in aircraft_query.iter() {
        coverage.observe_aircraft(&aircraft.icao, aircraft.latitude, aircraft.longitude);
    }
}

/// System to render coverage statistics panel
pub fn render_coverage_stats_panel(
    mut contexts: EguiContexts,
    mut coverage: ResMut<CoverageState>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    if !coverage.show_stats {
        return;
    }

    let stats = coverage.get_stats();

    egui::Window::new("Coverage Statistics")
        .collapsible(true)
        .resizable(false)
        .default_width(250.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Tracking: ");
                if coverage.enabled {
                    ui.colored_label(egui::Color32::GREEN, "ACTIVE");
                } else {
                    ui.colored_label(egui::Color32::GRAY, "INACTIVE");
                }
            });

            ui.separator();

            egui::Grid::new("coverage_stats_grid")
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

            // Receiver location
            ui.horizontal(|ui| {
                ui.label("Receiver:");
                ui.label(format!(
                    "{:.4}, {:.4}",
                    coverage.receiver_location.0, coverage.receiver_location.1
                ));
            });
        });
}

/// Plugin for coverage functionality
pub struct CoveragePlugin;

impl Plugin for CoveragePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CoverageState>().add_systems(
            Update,
            (toggle_coverage_mode, update_coverage_from_aircraft),
        );
        // Coverage stats panel is rendered via the consolidated Tools window (tools_window.rs)
    }
}
