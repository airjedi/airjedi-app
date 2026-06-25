use bevy::{
    camera::{visibility::RenderLayers, CameraOutputMode},
    gizmos::config::{DefaultGizmoConfigGroup, GizmoConfigStore},
    light::SunDisk,
    prelude::*,
};
use bevy_slippy_tiles::*;
use std::io::Write;
use std::sync::{Arc, Mutex};

mod adsb;
mod aircraft;
mod airspace;
mod aviation;
mod bookmarks;
#[cfg(feature = "brp")]
mod brp;
mod build_info;
mod camera;
mod config;
mod coverage;
mod data;
mod data_ingest;
mod data_sources;
mod debug_3d_hud;
mod debug_panel;
mod dock;
mod export;
mod fusion_integration;
mod geo;
mod hud;
mod input;
mod inspector;
mod keyboard;
mod map;
mod paths;
mod recording;
mod render_layers;
mod statusbar;
mod terrain;
pub(crate) mod theme;
mod tile_cache;
mod tiles;
mod toolbar;
mod tools;
mod tools_window;
mod ui_panels;
mod units;
mod view3d;
mod weather;
pub(crate) mod widgets;
mod zoom;

// Re-export core types so crate::Aircraft, crate::MapState, crate::ZoomState
// continue to resolve throughout the codebase.
pub(crate) use aircraft::components::{Aircraft, AircraftLabel};
use bevy_egui::{EguiGlobalSettings, PrimaryEguiContext};
pub(crate) use camera::{AircraftCamera, MapCamera};
use config::ConfigPlugin;
use keyboard::{
    handle_keyboard_shortcuts, sync_panel_manager_to_resources, sync_resources_to_panel_manager,
    toggle_overlays_keyboard, update_help_overlay, HelpOverlayState,
};
pub(crate) use map::{MapState, ZoomState};
pub(crate) use render_layers::RenderCategory;

// ADS-B client types

/// System ordering: all systems that modify `MapState::zoom_level` in 3D mode
/// must be in `ZoomSet::Change`. Position-dependent systems (aircraft, airports,
/// navaids, runways, camera) must run `.after(ZoomSet::Change)` so they always
/// see a consistent zoom level within each frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ZoomSet {
    /// Systems that may change the zoom level (e.g., request_3d_tiles_continuous)
    Change,
}

// =============================================================================
// Constants - All magic numbers centralized here
// =============================================================================

#[allow(dead_code)] // Some constants defined for future use
pub(crate) mod constants {
    // Mercator projection limits
    pub const MERCATOR_LAT_LIMIT: f64 = 85.0511;

    // Zoom thresholds for tile level transitions (use stdlib constants)
    pub const ZOOM_UPGRADE_THRESHOLD: f32 = std::f32::consts::SQRT_2;
    pub const ZOOM_DOWNGRADE_THRESHOLD: f32 = std::f32::consts::FRAC_1_SQRT_2;

    // Camera zoom bounds
    pub const MIN_CAMERA_ZOOM: f32 = 0.1;
    pub const MAX_CAMERA_ZOOM: f32 = 10.0;

    // Tile size: Large = 512px (@2x) for sharper map tiles
    pub const DEFAULT_TILE_SIZE: bevy_slippy_tiles::TileSize = bevy_slippy_tiles::TileSize::Large;
    pub const DEFAULT_TILE_PIXELS: f32 = 512.0;

    // Tile download settings
    pub const TILE_DOWNLOAD_RADIUS: u8 = 3;

    // Zoom sensitivity
    pub const ZOOM_SENSITIVITY_LINE: f32 = 0.1; // Mouse wheel
    pub const ZOOM_SENSITIVITY_PIXEL: f32 = 0.002; // Trackpad

    // Movement threshold for tile requests (degrees, ~100m at equator)
    pub const PAN_TILE_REQUEST_THRESHOLD: f64 = 0.001;

    // UI and rendering
    pub const BASE_FONT_SIZE: f32 = 14.0;
    pub const AIRCRAFT_MARKER_RADIUS: f32 = 8.0;
    pub const LABEL_SCREEN_OFFSET: f32 = 25.0;
    pub const BUTTON_FONT_SIZE: f32 = 16.0;

    // Tile fade/despawn timing
    pub const TILE_FADE_SPEED: f32 = 3.0;

    // Z-layers
    pub const TILE_Z_LAYER: f32 = 0.0;
    pub const AIRCRAFT_Z_LAYER: f32 = 10.0;
    pub const LABEL_Z_LAYER: f32 = 11.0;

    // 3D model scale: model is ~4 units across, target is 32 world units (AIRCRAFT_MARKER_RADIUS * 4)
    pub const AIRCRAFT_MODEL_SCALE: f32 = 8.0;

    // Default map center (Wichita, KS)
    pub const DEFAULT_LATITUDE: f64 = 37.6872;
    pub const DEFAULT_LONGITUDE: f64 = -97.3301;

    // Aviation feature visibility radius (nautical miles)
    pub const AVIATION_FEATURE_RADIUS_NM: f64 = 120.0;
    // Bounding box pre-filter margin in degrees (~3° covers 120 NM at all latitudes)
    pub const AVIATION_FEATURE_BBOX_DEG: f64 = 3.0;

    // ADS-B connection settings
    pub const ADSB_SERVER_ADDRESS: &str = "98.186.33.60:30003";
    pub const ADSB_MAX_DISTANCE_MILES: f64 = 250.0;
    pub const ADSB_AIRCRAFT_TIMEOUT_SECS: i64 = 180;
}

// =============================================================================
// Coordinate Helpers - Centralized coordinate conversion functions
// =============================================================================

/// Clamp latitude to valid Mercator projection range
pub(crate) fn clamp_latitude(lat: f64) -> f64 {
    lat.clamp(
        -constants::MERCATOR_LAT_LIMIT,
        constants::MERCATOR_LAT_LIMIT,
    )
}

/// Clamp longitude to valid range
pub(crate) fn clamp_longitude(lon: f64) -> f64 {
    lon.clamp(-180.0, 180.0)
}

fn main() {
    paths::init_from_args();

    // Raise the open file descriptor limit from the macOS default of 256.
    // 3D mode loads many tile textures across multiple zoom levels and can
    // easily exhaust the soft limit.
    #[cfg(unix)]
    {
        use std::io;
        unsafe {
            let mut rlim = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) == 0 {
                let target = 4096.min(rlim.rlim_max);
                if rlim.rlim_cur < target {
                    rlim.rlim_cur = target;
                    if libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) != 0 {
                        eprintln!(
                            "Warning: failed to raise file descriptor limit: {}",
                            io::Error::last_os_error()
                        );
                    }
                }
            }
        }
    }

    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "AirJedi - Aircraft Map Tracker".to_string(),
                    resolution: (1280, 720).into(),
                    visible: false,
                    ..default()
                }),
                ..default()
            })
            .set(bevy::log::LogPlugin {
                filter: "info,wgpu=warn,naga=warn,bevy_render=info".to_string(),
                ..default()
            }),
        SlippyTilesPlugin,
        ConfigPlugin,
        aviation::AviationPlugin,
        aircraft::AircraftPlugin,
        weather::WeatherPlugin,
        bookmarks::BookmarksPlugin,
        recording::RecordingPlugin,
        tools::ToolsPlugin,
        coverage::CoveragePlugin,
        airspace::AirspacePlugin,
        data_sources::DataSourcesPlugin,
        export::ExportPlugin,
        view3d::View3DPlugin,
        adsb::AdsbPlugin,
    ))
    .add_plugins((
        bevy_obj::ObjPlugin,
        bevy_inspector_egui::DefaultInspectorConfigPlugin,
        data_ingest::DataIngestPlugin,
    ))
    // Full speed when focused; ~4 FPS when unfocused to keep ADS-B data
    // flowing without overwhelming the GPU or triggering macOS throttling.
    .insert_resource(ClearColor(Color::srgb(
        20.0 / 255.0,
        21.0 / 255.0,
        24.0 / 255.0,
    )))
    .insert_resource(bevy::winit::WinitSettings {
        focused_mode: bevy::winit::UpdateMode::Continuous,
        unfocused_mode: bevy::winit::UpdateMode::reactive(std::time::Duration::from_millis(250)),
    })
    .init_resource::<HelpOverlayState>()
    .init_resource::<ui_panels::UiPanelManager>()
    .init_resource::<tools_window::ToolsWindowState>()
    .init_resource::<debug_panel::DebugPanelState>()
    .init_resource::<dock::DockTreeState>()
    .init_resource::<inspector::InspectorState>()
    .init_resource::<statusbar::StatusBarState>()
    .init_resource::<hud::HudState>()
    .register_type::<MapState>()
    .register_type::<ZoomState>()
    .insert_resource(ZoomState::new())
    // SlippyTilesSettings will be updated by setup_slippy_tiles_from_config after config is loaded
    .insert_resource(SlippyTilesSettings {
        endpoint: config::BasemapStyle::default().endpoint_url().to_string(),
        tiles_directory: std::path::PathBuf::from("tiles/"), // Symlinked to centralized cache
        reference_latitude: constants::DEFAULT_LATITUDE, // Wichita, KS (matches MapState default)
        reference_longitude: constants::DEFAULT_LONGITUDE, // Wichita, KS (matches MapState default)
        z_layer: 0.0,                // Render tiles at z=0 (behind aircraft at z=10)
        auto_render: false,          // Disable auto-render, we handle tile display ourselves
        max_concurrent_downloads: 8, // 3D mode generates many parallel requests across zoom levels
        rate_limit_requests: 24, // CartoDB/ESRI CDNs handle this easily; OSM is more restrictive
        ..default()
    })
    .add_plugins((
        zoom::ZoomPlugin,
        tiles::TilesPlugin,
        terrain::TerrainPlugin,
        input::InputPlugin,
        camera::CameraPlugin,
    ))
    .add_systems(
        Startup,
        (setup_debug_logger, setup_map, configure_gizmo_layers),
    )
    .add_systems(
        bevy_egui::EguiPrimaryContextPass,
        (
            theme::apply_egui_theme,
            toolbar::render_toolbar.after(theme::apply_egui_theme),
            statusbar::render_statusbar.after(toolbar::render_toolbar),
            dock::render_dock_tree.after(statusbar::render_statusbar),
        ),
    )
    .add_systems(Update, show_window_after_init)
    .add_systems(Update, handle_keyboard_shortcuts)
    .add_systems(Update, toggle_overlays_keyboard)
    .add_systems(
        Update,
        sync_resources_to_panel_manager.after(handle_keyboard_shortcuts),
    )
    .add_systems(
        Update,
        sync_panel_manager_to_resources.after(sync_resources_to_panel_manager),
    )
    .add_systems(Update, update_help_overlay)
    .add_systems(Update, debug_panel::update_debug_metrics)
    .add_systems(Update, heartbeat_diagnostic);

    app.add_plugins(fusion_integration::FusionIntegrationPlugin);

    #[cfg(feature = "brp")]
    app.add_plugins(brp::BrpPlugin);

    app.run();
}

// Resource to hold debug log file handle
#[derive(Resource, Clone)]
pub(crate) struct ZoomDebugLogger {
    file: Arc<Mutex<std::fs::File>>,
}

impl ZoomDebugLogger {
    pub(crate) fn log(&self, msg: &str) {
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{}", msg);
            let _ = file.flush();
        }
    }
}

/// Show the window after a few frames so the GPU clear color is active before
/// the window becomes visible, avoiding the macOS Metal magenta flash.
fn show_window_after_init(mut windows: Query<&mut Window>, mut frame: Local<u32>) {
    *frame += 1;
    if *frame == 3 {
        if let Ok(mut window) = windows.single_mut() {
            window.visible = true;
        }
    }
}

/// Diagnostic heartbeat: logs wall-clock time, frame delta, and focus state to
/// tmp/heartbeat.log every ~5 seconds.  If the app freezes, gaps in the log reveal
/// whether the process was suspended (App Nap) or the main thread was blocked.
fn heartbeat_diagnostic(
    time: Res<Time>,
    mut last_wall: Local<Option<std::time::Instant>>,
    windows: Query<&Window>,
    tile_query: Query<(), With<MapTile>>,
) {
    // Use wall clock for interval tracking (immune to Bevy virtual time capping)
    let now = std::time::Instant::now();
    if let Some(prev) = *last_wall {
        if now.duration_since(prev).as_secs() < 5 {
            return;
        }
    }
    *last_wall = Some(now);

    let focused = windows.iter().next().is_some_and(|w| w.focused);
    let delta_ms = time.delta_secs() * 1000.0;
    let tile_count = tile_query.iter().count();
    let elapsed = time.elapsed_secs_f64();

    let msg = format!(
        "HEARTBEAT: elapsed={:.1}s delta={:.1}ms focused={} tiles={}",
        elapsed, delta_ms, focused, tile_count
    );
    debug!("{}", msg);

    // Also write to file with explicit flush
    let log_dir = paths::log_dir();
    paths::ensure_dir(&log_dir);
    let log_path = log_dir.join("heartbeat.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "{}", msg);
        let _ = f.flush();
    }
}

// ADS-B integration is in src/adsb/ module

fn setup_debug_logger(mut commands: Commands) {
    use std::fs::OpenOptions;

    // Create or truncate the debug log file in tmp directory (per project conventions)
    let log_dir = paths::log_dir();
    paths::ensure_dir(&log_dir);
    let log_path = log_dir.join("zoom_debug.log");

    match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
    {
        Ok(file) => {
            let logger = ZoomDebugLogger {
                file: Arc::new(Mutex::new(file)),
            };
            logger.log("=== ZOOM DEBUG LOG INITIALIZED ===");
            commands.insert_resource(logger);
            info!("Debug logging enabled to {:?}", log_path);
        }
        Err(e) => {
            warn!("Failed to create debug log file: {}", e);
        }
    }
}

/// Configure gizmos to render on layer 2 so they are only drawn by Camera2d
/// (which includes layer 2) and not by Camera3d (default layer 0 only).
/// This prevents trails, navaids, and runways from being double-rendered
/// when both cameras share the same transform in 3D mode.
fn configure_gizmo_layers(mut config_store: ResMut<GizmoConfigStore>) {
    let (config, _) = config_store.config_mut::<DefaultGizmoConfigGroup>();
    config.render_layers = RenderLayers::layer(RenderCategory::GIZMOS);
}

pub(crate) fn setup_map(
    mut commands: Commands,
    mut download_events: MessageWriter<DownloadSlippyTilesMessage>,
    mut tile_settings: ResMut<SlippyTilesSettings>,
    app_config: Res<config::AppConfig>,
    mut egui_settings: ResMut<EguiGlobalSettings>,
) {
    // Prevent bevy_egui from auto-attaching to Camera2d. We use a dedicated UI
    // camera so egui stays visible when Camera2d switches to perspective in 3D mode.
    egui_settings.auto_create_primary_context = false;

    // Use Visible instead of the default VisibleInView for mesh picking.
    // Our dual-camera architecture (Camera3d overlay with alpha blending,
    // Camera2d base) means ViewVisibility isn't computed correctly for
    // Camera3d's entities. 3D mode picking is handled by a manual
    // MeshRayCast system (pick_aircraft_3d) instead.
    commands.insert_resource(bevy::picking::mesh_picking::MeshPickingSettings {
        require_markers: false,
        ray_cast_visibility: RayCastVisibility::Visible,
    });

    // Set up 2D camera for map tiles and labels.
    // Layer 0 = default content (tiles, sprites, text).
    // Layer 2 = gizmos (trails, navaids, runways) — kept off Camera3d to prevent
    //           double-rendering during 2D↔3D transitions.
    let dark_bg = Color::srgb(20.0 / 255.0, 21.0 / 255.0, 24.0 / 255.0);
    commands.spawn((
        Name::new("Map Camera"),
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(dark_bg),
            ..default()
        },
        MapCamera,
        render_layers::layers_camera2d_all(),
    ));

    // Set up 3D camera for aircraft models (renders on top of 2D).
    // No Atmosphere component — Bevy 0.18's atmosphere triggers multi-camera
    // HDR tonemapping bugs on Metal (#18901, #18902, #17530) that non-
    // deterministically produce a black screen. A gradient sky dome mesh
    // replaces the atmosphere sky visually. See GitHub issue for tech debt.
    //
    // Without Atmosphere/HDR, this camera's output supports alpha compositing
    // over Camera2d, eliminating the need for the separate AircraftCamera2d.
    // Camera2d (order 0) renders first; this camera (order 1) alpha-blends
    // on top via CameraOutputMode::Write.
    commands.spawn((
        Name::new("Aircraft Camera"),
        Camera3d::default(),
        AircraftCamera,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            output_mode: CameraOutputMode::Write {
                blend_state: Some(bevy::render::render_resource::BlendState::ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            ..default()
        },
        Projection::Orthographic(OrthographicProjection::default_2d()),
        Transform::default(),
        bevy::picking::mesh_picking::MeshPickingCamera,
        render_layers::layers_3d_world(),
        bevy::pbr::DistanceFog {
            color: Color::srgba(0.10, 0.10, 0.12, 1.0),
            directional_light_color: Color::NONE,
            directional_light_exponent: 30.0,
            falloff: bevy::pbr::FogFalloff::Linear {
                start: 3500.0,
                end: 5000.0,
            },
        },
    ));

    // Dedicated UI camera for egui. Renders last (order 100) with no clear so it
    // composites the UI on top of everything. Uses an empty render layer (11) to
    // avoid re-rendering any scene content. PrimaryEguiContext tells bevy_egui to
    // attach the egui context here instead of on Camera2d.
    commands.spawn((
        Name::new("UI Camera"),
        Camera2d,
        PrimaryEguiContext,
        Camera {
            order: 100,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        RenderLayers::layer(11),
    ));

    // Lighting for 3D aircraft models
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 300.0,
        affects_lightmapped_meshes: true,
    });
    commands.spawn((
        Name::new("Sun Light"),
        DirectionalLight {
            illuminance: 5000.0,
            shadows_enabled: false,
            ..default()
        },
        SunDisk::EARTH,
        view3d::sky::SunLight,
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -1.0, 0.3, 0.0)),
    ));

    // Moonlight: secondary directional light with cool blue-white color
    commands.spawn((
        Name::new("Moon Light"),
        DirectionalLight {
            illuminance: 0.0,
            shadows_enabled: false,
            color: Color::srgb(0.7, 0.75, 0.9),
            ..default()
        },
        view3d::sky::MoonLight,
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.5, 2.0, 0.0)),
    ));

    // Set up centralized tile cache (creates cache dir + symlink into assets/)
    tile_cache::setup_tile_cache();
    tile_cache::setup_terrain_cache();
    tile_cache::clear_legacy_tiles();
    tile_cache::remove_invalid_tiles();

    // Update SlippyTilesSettings from config
    tile_settings.endpoint = app_config.map.basemap_style.endpoint_url().to_string();
    tile_settings.tile_format = app_config.map.basemap_style.tile_format();
    tile_settings.reverse_axes = app_config.map.basemap_style.reverse_axes();
    tile_settings.reference_latitude = app_config.map.default_latitude;
    tile_settings.reference_longitude = app_config.map.default_longitude;

    // Initialize map state resource from config
    let map_state = MapState {
        latitude: app_config.map.default_latitude,
        longitude: app_config.map.default_longitude,
        zoom_level: ZoomLevel::try_from(app_config.map.default_zoom).unwrap_or(ZoomLevel::L10),
    };

    // Send initial tile download request
    tiles::request_tiles_at_location(
        &mut download_events,
        map_state.latitude,
        map_state.longitude,
        map_state.zoom_level,
        constants::TILE_DOWNLOAD_RADIUS,
        true,
    );

    commands.insert_resource(map_state);
}

pub fn clear_tile_cache() {
    tile_cache::clear_tile_cache();
}
