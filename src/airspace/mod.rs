//! Airspace Boundaries Module
//!
//! Displays Class B/C/D airspace and restricted areas.
//! This module provides data structures and stubs for airspace visualization.
//!
//! ## Data Sources (for future implementation)
//! - FAA NASR (National Airspace System Resources)
//! - OpenAIP (open aviation database)
//! - FAA SUA (Special Use Airspace)
//! - VATSIM data (for virtual airspace)
//!
//! ## Implementation Notes
//! Airspace data is typically distributed in various formats:
//! - OpenAIP uses XML format
//! - FAA NASR uses various fixed-width and CSV formats
//! - GeoJSON is common for web applications
//!
//! The approach here defines the data structures needed, with stubs
//! for loading and rendering that can be filled in when data integration
//! is implemented.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::message::MessageReader;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use serde::{Deserialize, Serialize};

use crate::data_ingest::canonical::CanonicalRecord;
use crate::geo::CoordinateConverter;
use crate::render_layers::RenderCategory;
use crate::view3d::View3DState;

/// Classification of airspace
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AirspaceClass {
    /// Class A airspace (18,000 MSL to FL600, requires IFR)
    ClassA,
    /// Class B airspace (around major airports)
    ClassB,
    /// Class C airspace (around medium airports with radar)
    ClassC,
    /// Class D airspace (around airports with control tower)
    ClassD,
    /// Class E airspace (controlled airspace, various altitudes)
    ClassE,
    /// Class G airspace (uncontrolled)
    #[default]
    ClassG,
    /// Restricted area (R-xxxx)
    Restricted,
    /// Prohibited area (P-xxxx)
    Prohibited,
    /// Warning area (W-xxxx)
    Warning,
    /// Military Operations Area (MOA)
    MOA,
    /// Alert area (A-xxxx)
    Alert,
    /// Temporary Flight Restriction
    TFR,
}

impl AirspaceClass {
    /// Get display name for this class
    pub fn display_name(&self) -> &'static str {
        match self {
            AirspaceClass::ClassA => "Class A",
            AirspaceClass::ClassB => "Class B",
            AirspaceClass::ClassC => "Class C",
            AirspaceClass::ClassD => "Class D",
            AirspaceClass::ClassE => "Class E",
            AirspaceClass::ClassG => "Class G",
            AirspaceClass::Restricted => "Restricted",
            AirspaceClass::Prohibited => "Prohibited",
            AirspaceClass::Warning => "Warning",
            AirspaceClass::MOA => "MOA",
            AirspaceClass::Alert => "Alert",
            AirspaceClass::TFR => "TFR",
        }
    }

    /// Get color for rendering this airspace type
    pub fn color(&self) -> Color {
        match self {
            AirspaceClass::ClassA => Color::srgba(1.0, 1.0, 1.0, 0.1), // White, subtle
            AirspaceClass::ClassB => Color::srgba(0.0, 0.4, 1.0, 0.3), // Blue
            AirspaceClass::ClassC => Color::srgba(0.5, 0.0, 0.5, 0.3), // Magenta
            AirspaceClass::ClassD => Color::srgba(0.0, 0.0, 1.0, 0.3), // Blue (dashed in real charts)
            AirspaceClass::ClassE => Color::srgba(0.5, 0.0, 0.5, 0.2), // Light magenta
            AirspaceClass::ClassG => Color::srgba(0.5, 0.5, 0.5, 0.1), // Gray
            AirspaceClass::Restricted => Color::srgba(1.0, 0.0, 0.0, 0.3), // Red
            AirspaceClass::Prohibited => Color::srgba(0.8, 0.0, 0.0, 0.4), // Dark red
            AirspaceClass::Warning => Color::srgba(1.0, 0.5, 0.0, 0.3), // Orange
            AirspaceClass::MOA => Color::srgba(0.6, 0.3, 0.0, 0.2), // Brown
            AirspaceClass::Alert => Color::srgba(1.0, 1.0, 0.0, 0.2), // Yellow
            AirspaceClass::TFR => Color::srgba(1.0, 0.0, 0.0, 0.4), // Red, prominent
        }
    }
}

/// A point defining part of an airspace boundary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirspacePoint {
    pub latitude: f64,
    pub longitude: f64,
}

/// Altitude specification for airspace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AltitudeReference {
    /// Mean Sea Level (feet)
    MSL(i32),
    /// Above Ground Level (feet)
    AGL(i32),
    /// Flight Level (hundreds of feet)
    FL(u16),
    /// Surface (ground level)
    Surface,
    /// Unlimited (no upper limit)
    Unlimited,
}

impl AltitudeReference {
    /// Get displayable string
    pub fn display(&self) -> String {
        match self {
            AltitudeReference::MSL(ft) => format!("{} MSL", ft),
            AltitudeReference::AGL(ft) => format!("{} AGL", ft),
            AltitudeReference::FL(fl) => format!("FL{}", fl),
            AltitudeReference::Surface => "SFC".to_string(),
            AltitudeReference::Unlimited => "UNL".to_string(),
        }
    }
}

/// Complete airspace definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Airspace {
    /// Unique identifier (e.g., "KSFO_B", "R-2508")
    pub id: String,
    /// Display name
    pub name: String,
    /// Airspace classification
    pub class: AirspaceClass,
    /// Lower altitude limit
    pub floor: AltitudeReference,
    /// Upper altitude limit
    pub ceiling: AltitudeReference,
    /// Polygon points defining the boundary
    pub boundary: Vec<AirspacePoint>,
    /// Optional: controlling agency/frequency
    pub controlling_agency: Option<String>,
    /// Optional: frequency for communications
    pub frequency: Option<String>,
    /// Optional: times of operation (e.g., "0600-2200 local")
    pub operating_times: Option<String>,
}

impl Airspace {
    /// Check if a point is inside this airspace (horizontally)
    pub fn contains_point(&self, lat: f64, lon: f64) -> bool {
        // Ray casting algorithm for point-in-polygon
        if self.boundary.len() < 3 {
            return false;
        }

        let mut inside = false;
        let n = self.boundary.len();
        let mut j = n - 1;

        for i in 0..n {
            let yi = self.boundary[i].latitude;
            let xi = self.boundary[i].longitude;
            let yj = self.boundary[j].latitude;
            let xj = self.boundary[j].longitude;

            if ((yi > lat) != (yj > lat)) && (lon < (xj - xi) * (lat - yi) / (yj - yi) + xi) {
                inside = !inside;
            }
            j = i;
        }

        inside
    }

    /// Get the centroid of the airspace for labeling
    pub fn centroid(&self) -> (f64, f64) {
        if self.boundary.is_empty() {
            return (0.0, 0.0);
        }
        let sum_lat: f64 = self.boundary.iter().map(|p| p.latitude).sum();
        let sum_lon: f64 = self.boundary.iter().map(|p| p.longitude).sum();
        let n = self.boundary.len() as f64;
        (sum_lat / n, sum_lon / n)
    }
}

/// Generate a circular polygon approximation from a center point and radius in nm.
fn generate_circle(center_lat: f64, center_lon: f64, radius_nm: f64, segments: usize) -> Vec<AirspacePoint> {
    let mut points = Vec::with_capacity(segments + 1);
    let deg_per_nm_lat = 1.0 / 60.0;
    let deg_per_nm_lon = 1.0 / (60.0 * center_lat.to_radians().cos());
    for i in 0..=segments {
        let angle = 2.0 * std::f64::consts::PI * (i as f64) / (segments as f64);
        let lat = center_lat + radius_nm * deg_per_nm_lat * angle.cos();
        let lon = center_lon + radius_nm * deg_per_nm_lon * angle.sin();
        points.push(AirspacePoint { latitude: lat, longitude: lon });
    }
    points
}

/// Resource storing loaded airspace data
#[derive(Resource, Default)]
pub struct AirspaceData {
    /// All loaded airspace definitions
    pub airspaces: Vec<Airspace>,
    /// Data source (for display)
    pub source: Option<String>,
    /// Whether data has been loaded
    pub loaded: bool,
    /// Whether data has changed and meshes need regeneration
    pub dirty: bool,
}

impl AirspaceData {
    /// Load airspace data from a file
    ///
    /// TODO: Implement actual file parsing for:
    /// - OpenAIP XML format
    /// - GeoJSON format
    /// - FAA NASR format
    pub fn load_from_file(&mut self, _path: &std::path::Path) -> Result<(), String> {
        // Stub implementation
        warn!("Airspace loading not yet implemented");
        Err("Airspace loading not yet implemented. Data sources to consider: OpenAIP, FAA NASR".to_string())
    }

    /// Load sample airspace data for testing - realistic KICT Class C tiers.
    pub fn load_sample_data(&mut self) {
        // KICT Class C inner ring: SFC to 5300 MSL (~5nm radius)
        self.airspaces.push(Airspace {
            id: "KICT_C_INNER".to_string(),
            name: "KICT Class C Inner".to_string(),
            class: AirspaceClass::ClassC,
            floor: AltitudeReference::Surface,
            ceiling: AltitudeReference::MSL(5300),
            boundary: generate_circle(37.6499, -97.4331, 5.0, 36),
            controlling_agency: Some("Wichita Approach".to_string()),
            frequency: Some("118.2".to_string()),
            operating_times: None,
        });

        // KICT Class C outer ring: 2700 MSL to 5300 MSL (~10nm radius)
        self.airspaces.push(Airspace {
            id: "KICT_C_OUTER".to_string(),
            name: "KICT Class C Outer".to_string(),
            class: AirspaceClass::ClassC,
            floor: AltitudeReference::MSL(2700),
            ceiling: AltitudeReference::MSL(5300),
            boundary: generate_circle(37.6499, -97.4331, 10.0, 48),
            controlling_agency: Some("Wichita Approach".to_string()),
            frequency: Some("118.2".to_string()),
            operating_times: None,
        });

        self.loaded = true;
        self.dirty = true;
        self.source = Some("Sample Data (KICT)".to_string());
        info!("Loaded {} sample airspace definitions", self.airspaces.len());
    }

    /// Find airspaces containing a point
    pub fn find_at_point(&self, lat: f64, lon: f64) -> Vec<&Airspace> {
        self.airspaces.iter().filter(|a| a.contains_point(lat, lon)).collect()
    }
}

/// Resource for airspace display settings
#[derive(Resource)]
pub struct AirspaceDisplayState {
    /// Whether to show airspace boundaries
    pub enabled: bool,
    /// Which classes to display
    pub show_class_b: bool,
    pub show_class_c: bool,
    pub show_class_d: bool,
    pub show_restricted: bool,
    pub show_moa: bool,
    pub show_tfr: bool,
    pub show_warning: bool,
    pub show_alert: bool,
    /// Whether to show labels
    pub show_labels: bool,
    /// Opacity for airspace meshes (0.0 - 1.0)
    pub opacity: f32,
    /// Optional altitude filter in feet (only show airspaces at this altitude)
    pub altitude_filter_ft: Option<i32>,
}

impl Default for AirspaceDisplayState {
    fn default() -> Self {
        Self {
            enabled: true,
            show_class_b: true,
            show_class_c: true,
            show_class_d: true,
            show_restricted: true,
            show_moa: false,
            show_tfr: true,
            show_warning: true,
            show_alert: true,
            show_labels: true,
            opacity: 0.10,
            altitude_filter_ft: None,
        }
    }
}

impl AirspaceDisplayState {
    pub fn is_class_visible(&self, class: &AirspaceClass) -> bool {
        if !self.enabled {
            return false;
        }
        match class {
            AirspaceClass::ClassB => self.show_class_b,
            AirspaceClass::ClassC => self.show_class_c,
            AirspaceClass::ClassD => self.show_class_d,
            AirspaceClass::Restricted => self.show_restricted,
            AirspaceClass::MOA => self.show_moa,
            AirspaceClass::Warning => self.show_warning,
            AirspaceClass::Alert => self.show_alert,
            AirspaceClass::TFR => self.show_tfr,
            _ => false,
        }
    }

    pub fn passes_altitude_filter(&self, floor_ft: Option<i32>, ceiling_ft: Option<i32>) -> bool {
        let Some(filter) = self.altitude_filter_ft else {
            return true;
        };
        let floor = floor_ft.unwrap_or(0);
        let ceiling = ceiling_ft.unwrap_or(60000);
        filter >= floor && filter <= ceiling
    }
}

/// Component for airspace boundary entities
#[derive(Component)]
pub struct AirspaceBoundary {
    pub airspace_id: String,
}

/// Marker for spawned airspace mesh entities.
#[derive(Component)]
pub struct AirspaceMeshMarker {
    pub airspace_id: String,
}

/// Tracks which airspaces have spawned meshes.
#[derive(Resource, Default)]
pub struct SpawnedAirspaces {
    pub ids: std::collections::HashSet<String>,
    pub was_3d: bool,
    pub last_zoom: Option<u8>,
}

/// Timer for periodic airspace mesh refresh.
#[derive(Resource)]
pub struct AirspaceRefreshTimer(pub Timer);

impl Default for AirspaceRefreshTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.5, TimerMode::Repeating))
    }
}

/// System to toggle airspace display
pub fn toggle_airspace_display(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut display_state: ResMut<AirspaceDisplayState>,
    mut contexts: EguiContexts,
) {
    if let Ok(ctx) = contexts.ctx_mut() {
        if ctx.wants_keyboard_input() {
            return;
        }
    }

    // Shift+A - Toggle airspace boundaries
    if keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight) {
        if keyboard.just_pressed(KeyCode::KeyA) {
            display_state.enabled = !display_state.enabled;
            info!("Airspace display: {}", if display_state.enabled { "enabled" } else { "disabled" });
        }
    }
}

/// System to render airspace settings panel
pub fn render_airspace_panel(
    mut contexts: EguiContexts,
    mut display_state: ResMut<AirspaceDisplayState>,
    mut airspace_data: ResMut<AirspaceData>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    // Only show panel when airspace is enabled
    if !display_state.enabled {
        return;
    }

    egui::Window::new("Airspace")
        .collapsible(true)
        .resizable(false)
        .default_width(200.0)
        .show(ctx, |ui| {
            if !airspace_data.loaded {
                ui.label("No airspace data loaded");
                ui.separator();

                if ui.button("Load Sample Data").clicked() {
                    airspace_data.load_sample_data();
                }

                ui.label(
                    egui::RichText::new("Note: Full implementation requires\nintegration with FAA/OpenAIP data")
                        .size(11.0)
                        .color(egui::Color32::GRAY)
                );
            } else {
                if let Some(ref source) = airspace_data.source {
                    ui.label(format!("Source: {}", source));
                }
                ui.label(format!("{} airspaces loaded", airspace_data.airspaces.len()));

                ui.separator();
                ui.label("Display Options:");

                ui.checkbox(&mut display_state.show_class_b, "Class B");
                ui.checkbox(&mut display_state.show_class_c, "Class C");
                ui.checkbox(&mut display_state.show_class_d, "Class D");
                ui.checkbox(&mut display_state.show_restricted, "Restricted");
                ui.checkbox(&mut display_state.show_moa, "MOA");
                ui.checkbox(&mut display_state.show_tfr, "TFR");

                ui.separator();
                ui.checkbox(&mut display_state.show_labels, "Show Labels");
            }
        });
}

/// Consume NavigationDataUpdated messages and load airspace records into AirspaceData.
pub fn consume_airspace_data(
    mut nav_events: MessageReader<crate::data_ingest::NavigationDataUpdated>,
    mut airspace_data: ResMut<AirspaceData>,
) {
    for event in nav_events.read() {
        let airspaces: Vec<Airspace> = event
            .records
            .iter()
            .filter_map(|r| {
                if let CanonicalRecord::Airspace(info) = r {
                    Some(airspace_info_to_airspace(info))
                } else {
                    None
                }
            })
            .collect();

        if !airspaces.is_empty() {
            info!("Loaded {} airspace definitions from data ingest", airspaces.len());
            airspace_data.airspaces = airspaces;
            airspace_data.loaded = true;
            airspace_data.dirty = true;
            airspace_data.source = Some("FAA ADDS".to_string());
        }
    }
}

fn airspace_info_to_airspace(info: &crate::data_ingest::canonical::AirspaceInfo) -> Airspace {
    let class = match info.airspace_class.as_str() {
        "ClassB" => AirspaceClass::ClassB,
        "ClassC" => AirspaceClass::ClassC,
        "ClassD" => AirspaceClass::ClassD,
        "Restricted" => AirspaceClass::Restricted,
        "MOA" => AirspaceClass::MOA,
        "Warning" => AirspaceClass::Warning,
        "Alert" => AirspaceClass::Alert,
        "TFR" => AirspaceClass::TFR,
        _ => AirspaceClass::ClassG,
    };

    let floor = parse_altitude_ref(info.lower_limit_ft, info.lower_altitude_ref.as_deref());
    let ceiling = parse_altitude_ref(info.upper_limit_ft, info.upper_altitude_ref.as_deref());

    Airspace {
        id: info.name.clone(),
        name: info.name.clone(),
        class,
        floor,
        ceiling,
        boundary: info
            .polygon
            .iter()
            .map(|(lat, lon)| AirspacePoint {
                latitude: *lat,
                longitude: *lon,
            })
            .collect(),
        controlling_agency: None,
        frequency: None,
        operating_times: None,
    }
}

fn parse_altitude_ref(ft: Option<i32>, code: Option<&str>) -> AltitudeReference {
    match (ft, code) {
        (_, Some("SFC")) => AltitudeReference::Surface,
        (_, Some("UNL")) => AltitudeReference::Unlimited,
        (Some(v), Some("AGL")) => AltitudeReference::AGL(v),
        (Some(v), Some("FL")) => AltitudeReference::FL(v as u16 / 100),
        (Some(v), _) => AltitudeReference::MSL(v),
        (None, _) => AltitudeReference::Surface,
    }
}

/// Build a wall mesh (vertical fence) from a polygon boundary between floor and ceiling.
fn build_wall_mesh(
    boundary: &[AirspacePoint],
    floor_z: f32,
    ceiling_z: f32,
    converter: &CoordinateConverter,
) -> Mesh {
    let n = boundary.len();
    if n < 3 {
        return Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    }

    // Pre-compute 2D positions for all boundary points
    let world_pts: Vec<bevy::math::Vec2> = boundary
        .iter()
        .map(|p| converter.latlon_to_world(p.latitude, p.longitude))
        .collect();

    // Capacity: walls (n*4 verts, n*6 idx) + 2 caps (n+1 verts each, n*3 idx each)
    let wall_verts = n * 4;
    let cap_verts = (n + 1) * 2;
    let total_verts = wall_verts + cap_verts;
    let wall_idx = n * 6;
    let cap_idx = n * 3 * 2;
    let total_idx = wall_idx + cap_idx;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(total_verts);
    let mut indices: Vec<u32> = Vec::with_capacity(total_idx);

    // --- Wall quads ---
    for i in 0..n {
        let j = (i + 1) % n;
        let p0 = &world_pts[i];
        let p1 = &world_pts[j];

        let base_idx = positions.len() as u32;

        // Y-up: x=east, y=altitude, z=-north
        positions.push([p0.x, floor_z, -p0.y]);
        positions.push([p1.x, floor_z, -p1.y]);
        positions.push([p1.x, ceiling_z, -p1.y]);
        positions.push([p0.x, ceiling_z, -p0.y]);

        let edge = Vec3::new(p1.x - p0.x, 0.0, -(p1.y - p0.y));
        let normal = edge.cross(Vec3::Y).normalize_or_zero();
        for _ in 0..4 {
            normals.push(normal.to_array());
        }

        uvs.push([0.0, 0.0]);
        uvs.push([1.0, 0.0]);
        uvs.push([1.0, 1.0]);
        uvs.push([0.0, 1.0]);

        indices.push(base_idx);
        indices.push(base_idx + 1);
        indices.push(base_idx + 2);
        indices.push(base_idx);
        indices.push(base_idx + 2);
        indices.push(base_idx + 3);
    }

    // --- Top cap (ceiling) --- fan triangulation from centroid
    let cx: f32 = world_pts.iter().map(|p| p.x).sum::<f32>() / n as f32;
    let cy: f32 = world_pts.iter().map(|p| p.y).sum::<f32>() / n as f32;

    let top_center_idx = positions.len() as u32;
    positions.push([cx, ceiling_z, -cy]);
    normals.push([0.0, 1.0, 0.0]);
    uvs.push([0.5, 0.5]);

    for p in &world_pts {
        positions.push([p.x, ceiling_z, -p.y]);
        normals.push([0.0, 1.0, 0.0]);
        uvs.push([0.0, 0.0]);
    }

    for i in 0..n {
        let j = (i + 1) % n;
        indices.push(top_center_idx);
        indices.push(top_center_idx + 1 + i as u32);
        indices.push(top_center_idx + 1 + j as u32);
    }

    // --- Bottom cap (floor) --- fan triangulation, winding reversed
    let bot_center_idx = positions.len() as u32;
    positions.push([cx, floor_z, -cy]);
    normals.push([0.0, -1.0, 0.0]);
    uvs.push([0.5, 0.5]);

    for p in &world_pts {
        positions.push([p.x, floor_z, -p.y]);
        normals.push([0.0, -1.0, 0.0]);
        uvs.push([0.0, 0.0]);
    }

    for i in 0..n {
        let j = (i + 1) % n;
        indices.push(bot_center_idx);
        indices.push(bot_center_idx + 1 + j as u32); // reversed winding
        indices.push(bot_center_idx + 1 + i as u32);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Build a flat polygon mesh for 2D map rendering.
fn build_flat_polygon_mesh(
    boundary: &[AirspacePoint],
    converter: &CoordinateConverter,
) -> Mesh {
    let n = boundary.len();
    if n < 3 {
        return Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    }

    // Simple fan triangulation from centroid
    let positions_2d: Vec<Vec2> = boundary
        .iter()
        .map(|p| converter.latlon_to_world(p.latitude, p.longitude))
        .collect();

    let cx: f32 = positions_2d.iter().map(|p| p.x).sum::<f32>() / n as f32;
    let cy: f32 = positions_2d.iter().map(|p| p.y).sum::<f32>() / n as f32;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(n + 1);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(n + 1);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(n + 1);
    let mut indices: Vec<u32> = Vec::with_capacity(n * 3);

    // Center vertex
    positions.push([cx, 0.01, -cy]); // slight Y offset above tiles
    normals.push([0.0, 1.0, 0.0]);
    uvs.push([0.5, 0.5]);

    for p in &positions_2d {
        positions.push([p.x, 0.01, -p.y]);
        normals.push([0.0, 1.0, 0.0]);
        uvs.push([0.0, 0.0]);
    }

    for i in 0..n {
        let j = (i + 1) % n;
        indices.push(0); // center
        indices.push((i + 1) as u32);
        indices.push((j + 1) as u32);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Build a line-loop mesh for outlining the top and bottom edges of an airspace volume.
fn build_outline_mesh(
    boundary: &[AirspacePoint],
    floor_z: f32,
    ceiling_z: f32,
    converter: &CoordinateConverter,
) -> Mesh {
    let n = boundary.len();
    if n < 3 {
        return Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
    }

    let world_pts: Vec<Vec2> = boundary
        .iter()
        .map(|p| converter.latlon_to_world(p.latitude, p.longitude))
        .collect();

    // 2 edges (top + bottom) * n segments * 2 endpoints per line
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(n * 4);

    for i in 0..n {
        let j = (i + 1) % n;
        let p0 = &world_pts[i];
        let p1 = &world_pts[j];

        // Top edge segment
        positions.push([p0.x, ceiling_z, -p0.y]);
        positions.push([p1.x, ceiling_z, -p1.y]);

        // Bottom edge segment
        positions.push([p0.x, floor_z, -p0.y]);
        positions.push([p1.x, floor_z, -p1.y]);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh
}

/// Marker for spawned airspace outline entities.
#[derive(Component)]
pub struct AirspaceOutlineMarker {
    pub airspace_id: String,
}

fn altitude_ref_to_ft(alt: &AltitudeReference) -> i32 {
    match alt {
        AltitudeReference::MSL(ft) => *ft,
        AltitudeReference::AGL(ft) => *ft,
        AltitudeReference::FL(fl) => *fl as i32 * 100,
        AltitudeReference::Surface => 0,
        AltitudeReference::Unlimited => 60000,
    }
}

/// Check if airspace centroid is within rendering range (~250nm).
fn is_in_range(airspace: &Airspace, camera_lat: f64, camera_lon: f64) -> bool {
    let (centroid_lat, centroid_lon) = airspace.centroid();
    let dlat = (centroid_lat - camera_lat).abs();
    let dlon = (centroid_lon - camera_lon).abs();
    // Rough degree-based check (~250nm = ~4 degrees at mid-latitudes)
    dlat < 4.0 && dlon < 4.0
}

/// System to spawn/despawn airspace meshes based on visibility and distance.
pub fn update_airspace_meshes(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<AirspaceRefreshTimer>,
    mut airspace_data: ResMut<AirspaceData>,
    display_state: Res<AirspaceDisplayState>,
    view3d_state: Option<Res<View3DState>>,
    map_state: Res<crate::map::MapState>,
    tile_settings: Res<bevy_slippy_tiles::SlippyTilesSettings>,
    mut spawned: ResMut<SpawnedAirspaces>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    existing_query: Query<(Entity, &AirspaceMeshMarker)>,
    outline_query: Query<(Entity, &AirspaceOutlineMarker)>,
) {
    timer.0.tick(time.delta());

    // Auto-load sample data when airspace is enabled but no data exists
    if display_state.enabled && !airspace_data.loaded {
        airspace_data.load_sample_data();
    }

    let needs_refresh = timer.0.just_finished()
        || airspace_data.dirty
        || (display_state.enabled && airspace_data.loaded && spawned.ids.is_empty());

    if !needs_refresh {
        return;
    }

    if airspace_data.dirty {
        airspace_data.dirty = false;
    }

    let despawn_all = |commands: &mut Commands,
                       existing_query: &Query<(Entity, &AirspaceMeshMarker)>,
                       outline_query: &Query<(Entity, &AirspaceOutlineMarker)>| {
        for (entity, _) in existing_query.iter() {
            commands.entity(entity).despawn();
        }
        for (entity, _) in outline_query.iter() {
            commands.entity(entity).despawn();
        }
    };

    if !display_state.enabled {
        despawn_all(&mut commands, &existing_query, &outline_query);
        spawned.ids.clear();
        return;
    }

    let render_zoom = view3d_state
        .as_ref()
        .map(|v| v.effective_zoom(map_state.zoom_level))
        .unwrap_or(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, render_zoom);
    let is_3d = view3d_state
        .as_ref()
        .is_some_and(|v| v.mode == crate::view3d::ViewMode::Perspective3D);

    // Despawn all meshes when switching between 2D and 3D modes
    if is_3d != spawned.was_3d {
        despawn_all(&mut commands, &existing_query, &outline_query);
        spawned.ids.clear();
        spawned.was_3d = is_3d;
    }

    // Despawn all meshes when rendering zoom changes (positions are zoom-dependent)
    if Some(render_zoom.to_u8()) != spawned.last_zoom {
        despawn_all(&mut commands, &existing_query, &outline_query);
        spawned.ids.clear();
        spawned.last_zoom = Some(render_zoom.to_u8());
    }

    let camera_lat = map_state.latitude;
    let camera_lon = map_state.longitude;

    // Build a set of airspace IDs that should be visible for O(1) lookup
    let visible_ids: std::collections::HashSet<&str> = airspace_data
        .airspaces
        .iter()
        .filter(|a| display_state.is_class_visible(&a.class) && is_in_range(a, camera_lat, camera_lon))
        .map(|a| a.id.as_str())
        .collect();

    // Despawn meshes for airspaces no longer visible
    for (entity, marker) in existing_query.iter() {
        if !visible_ids.contains(marker.airspace_id.as_str()) {
            commands.entity(entity).despawn();
            spawned.ids.remove(&marker.airspace_id);
        }
    }
    for (entity, marker) in outline_query.iter() {
        if !visible_ids.contains(marker.airspace_id.as_str()) {
            commands.entity(entity).despawn();
        }
    }

    // Spawn meshes for newly visible airspaces
    trace!(
        "Airspace refresh: {} airspaces, camera ({:.4}, {:.4}), is_3d={}, spawned={}",
        airspace_data.airspaces.len(), camera_lat, camera_lon, is_3d, spawned.ids.len()
    );
    for airspace in &airspace_data.airspaces {
        if spawned.ids.contains(&airspace.id) {
            continue;
        }
        if !display_state.is_class_visible(&airspace.class) {
            continue;
        }
        if !is_in_range(airspace, camera_lat, camera_lon) {
            continue;
        }

        let floor_ft = altitude_ref_to_ft(&airspace.floor);
        let ceiling_ft = altitude_ref_to_ft(&airspace.ceiling);

        if !display_state.passes_altitude_filter(Some(floor_ft), Some(ceiling_ft)) {
            continue;
        }

        let mut color = airspace.class.color();
        color.set_alpha(display_state.opacity);

        let material = materials.add(StandardMaterial {
            base_color: color,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            unlit: true,
            ..default()
        });

        if is_3d {
            let v3d = view3d_state.as_ref().unwrap();
            let ground_z = v3d.altitude_to_z(v3d.ground_elevation_ft);
            // Floor/ceiling use altitude_to_z which includes View3DState::altitude_scale
            // This ensures airspace volumes scale consistently with aircraft, camera, etc.
            // Offset floor slightly above ground so airspace walls aren't
            // occluded by opaque tile mesh quads at the same depth.
            let floor_z = if floor_ft == 0 {
                ground_z + 5.0
            } else {
                v3d.altitude_to_z(floor_ft).max(ground_z + 5.0)
            };
            let ceiling_z = v3d.altitude_to_z(ceiling_ft);
            let mesh = build_wall_mesh(&airspace.boundary, floor_z, ceiling_z, &converter);

            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(material),
                Transform::IDENTITY,
                Visibility::default(),
                RenderLayers::layer(RenderCategory::AIRSPACE),
                AirspaceMeshMarker {
                    airspace_id: airspace.id.clone(),
                },
            ));

            // Spawn outline as a line mesh on the same render layer
            let outline_mesh = build_outline_mesh(&airspace.boundary, floor_z, ceiling_z, &converter);
            let mut outline_color = airspace.class.color();
            outline_color.set_alpha((display_state.opacity * 3.0).min(0.6));
            let outline_material = materials.add(StandardMaterial {
                base_color: outline_color,
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            });
            commands.spawn((
                Mesh3d(meshes.add(outline_mesh)),
                MeshMaterial3d(outline_material),
                Transform::IDENTITY,
                Visibility::default(),
                RenderLayers::layer(RenderCategory::AIRSPACE),
                AirspaceOutlineMarker {
                    airspace_id: airspace.id.clone(),
                },
            ));
        } else {
            // In 2D mode, just mark as spawned - gizmos handle the rendering
            commands.spawn(AirspaceMeshMarker {
                airspace_id: airspace.id.clone(),
            });
        }

        spawned.ids.insert(airspace.id.clone());
        debug!("  Spawned airspace mesh: {} (floor={} ceil={} is_3d={})", airspace.id, floor_ft, ceiling_ft, is_3d);
    }
}


/// Draw airspace boundaries as gizmo lines in 2D mode.
pub fn draw_airspace_gizmos(
    mut gizmos: Gizmos,
    airspace_data: Res<AirspaceData>,
    display_state: Res<AirspaceDisplayState>,
    view3d_state: Option<Res<View3DState>>,
    map_state: Res<crate::map::MapState>,
    tile_settings: Res<bevy_slippy_tiles::SlippyTilesSettings>,
) {
    if !display_state.enabled || !airspace_data.loaded {
        return;
    }

    let is_3d = view3d_state
        .as_ref()
        .is_some_and(|v| v.mode == crate::view3d::ViewMode::Perspective3D);

    // In 3D mode, outlines are spawned as line meshes on the AIRSPACE render layer
    if is_3d {
        return;
    }

    let render_zoom = view3d_state
        .as_ref()
        .map(|v| v.effective_zoom(map_state.zoom_level))
        .unwrap_or(map_state.zoom_level);
    let converter = CoordinateConverter::new(&tile_settings, render_zoom);
    let camera_lat = map_state.latitude;
    let camera_lon = map_state.longitude;

    for airspace in &airspace_data.airspaces {
        if !display_state.is_class_visible(&airspace.class) {
            continue;
        }
        if !is_in_range(airspace, camera_lat, camera_lon) {
            continue;
        }

        let floor_ft = altitude_ref_to_ft(&airspace.floor);
        let ceiling_ft = altitude_ref_to_ft(&airspace.ceiling);
        if !display_state.passes_altitude_filter(Some(floor_ft), Some(ceiling_ft)) {
            continue;
        }

        let mut outline_color = airspace.class.color();
        outline_color.set_alpha((display_state.opacity * 3.0).min(0.6));
        let n = airspace.boundary.len();
        if n < 3 {
            continue;
        }

        for i in 0..n {
            let j = (i + 1) % n;
            let p0 = converter.latlon_to_world(
                airspace.boundary[i].latitude,
                airspace.boundary[i].longitude,
            );
            let p1 = converter.latlon_to_world(
                airspace.boundary[j].latitude,
                airspace.boundary[j].longitude,
            );
            gizmos.line_2d(p0, p1, outline_color);
        }
    }
}

/// Plugin for airspace functionality
pub struct AirspacePlugin;

impl Plugin for AirspacePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AirspaceData>()
            .init_resource::<AirspaceDisplayState>()
            .init_resource::<SpawnedAirspaces>()
            .init_resource::<AirspaceRefreshTimer>()
            .add_systems(Update, (
                toggle_airspace_display,
                consume_airspace_data,
                update_airspace_meshes,
                draw_airspace_gizmos,
            ));
        // Airspace panel is rendered via the consolidated Tools window (tools_window.rs)
    }
}
