//! Sky rendering with star field and day/night cycle.
//!
//! The star field is rendered as a large 2D sprite on Camera2d at a low
//! z-depth (behind map tiles). This avoids multi-camera compositing
//! issues while ensuring stars never bleed through opaque tiles.
//! Sun position is computed using the NREL Solar Position Algorithm (SPA)
//! via the `solar-positioning` crate (~0.0003 degree accuracy).
//! Supports both real-time wall clock and manual time override via TimeState.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CameraOutputMode, Exposure};
use bevy::pbr::{DistanceFog, FogFalloff, StandardMaterial};
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;

use super::View3DState;
use crate::map::MapState;
use crate::RenderCategory;

/// Z-depth for the star field sprite (behind tiles at z=0.1)
const STAR_Z: f32 = -1.0;

/// Marker component for the star field sprite
#[derive(Component)]
pub struct StarField;

/// Marker component for the ground plane mesh
#[derive(Component)]
pub struct GroundPlane;

/// Marker component for the gradient sky dome mesh
#[derive(Component)]
pub struct SkyDome;

/// Number of latitude rings in the sky dome hemisphere
const SKY_DOME_RINGS: u32 = 16;
/// Number of segments around each ring
const SKY_DOME_SEGMENTS: u32 = 32;
/// Radius of the sky dome (large enough to surround the camera)
const SKY_DOME_RADIUS: f32 = 100_000.0;

/// Resource tracking current sun position
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct SunState {
    /// Sun elevation in degrees (-90 to 90, negative = below horizon)
    pub elevation: f32,
    /// Sun azimuth in degrees (0 = north, 90 = east)
    pub azimuth: f32,
}

impl Default for SunState {
    fn default() -> Self {
        Self {
            elevation: 45.0,
            azimuth: 180.0,
        }
    }
}

/// Controls whether the app uses real wall-clock time or a manual override.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct TimeState {
    #[reflect(ignore)]
    pub override_time: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub utc_offset_hours: f32,
    /// Set via BRP to override the local hour (0.0-24.0). Calls set_hour()
    /// internally. Resets to -1 after applying.
    pub override_hour: f32,
}

impl Default for TimeState {
    fn default() -> Self {
        Self {
            override_time: None,
            utc_offset_hours: 0.0,
            override_hour: -1.0,
        }
    }
}

impl TimeState {
    pub fn current_datetime(&self) -> chrono::DateTime<chrono::FixedOffset> {
        self.override_time
            .unwrap_or_else(|| chrono::Utc::now().fixed_offset())
    }

    pub fn is_manual(&self) -> bool {
        self.override_time.is_some()
    }

    pub fn set_hour(&mut self, hour: f32) {
        use chrono::Timelike;
        let now = chrono::Utc::now();
        let offset_secs = (self.utc_offset_hours * 3600.0) as i32;
        let offset = chrono::FixedOffset::east_opt(offset_secs)
            .unwrap_or(chrono::FixedOffset::east_opt(0).unwrap());
        let local_today = now.with_timezone(&offset);

        let h = hour.floor() as u32;
        let m = ((hour.fract()) * 60.0).floor() as u32;
        if let Some(dt) = local_today
            .with_hour(h.min(23))
            .and_then(|d| d.with_minute(m.min(59)))
            .and_then(|d| d.with_second(0))
        {
            self.override_time = Some(dt.fixed_offset());
        }
    }

    pub fn reset_to_live(&mut self) {
        self.override_time = None;
    }
}

/// Compute sun elevation and azimuth using the NREL Solar Position Algorithm.
/// Accuracy: ~0.0003 degrees. Handles polar day/night edge cases.
pub fn compute_sun_position(latitude: f64, longitude: f64) -> (f32, f32) {
    let now = chrono::Utc::now().fixed_offset();
    compute_sun_position_at(latitude, longitude, &now)
}

/// Compute sun position at a specific time (for time slider support).
pub fn compute_sun_position_at(
    latitude: f64,
    longitude: f64,
    datetime: &chrono::DateTime<chrono::FixedOffset>,
) -> (f32, f32) {
    use solar_positioning::{spa, time::DeltaT, RefractionCorrection};

    let delta_t = DeltaT::estimate_from_date_like(*datetime).unwrap_or(69.184);

    match spa::solar_position(
        *datetime,
        latitude,
        longitude,
        0.0,
        delta_t,
        Some(RefractionCorrection::standard()),
    ) {
        Ok(position) => {
            let elevation = position.elevation_angle() as f32;
            let azimuth = position.azimuth() as f32;
            (elevation, azimuth)
        }
        Err(_) => (45.0, 180.0),
    }
}

/// Marker for the directional light used as the sun
#[derive(Component)]
pub struct SunLight;

/// Marker for the directional light used as moonlight.
#[derive(Component)]
pub struct MoonLight;

/// Resource tracking current moon position and phase.
#[derive(Resource)]
pub struct MoonState {
    pub elevation: f32,
    pub azimuth: f32,
    /// 0.0 = new moon, 0.5 = full moon, 1.0 = new moon again
    pub phase: f32,
}

impl Default for MoonState {
    fn default() -> Self {
        Self {
            elevation: -10.0,
            azimuth: 0.0,
            phase: 0.5,
        }
    }
}

/// Simplified moon position using J2000.0 epoch (~2-5 degree accuracy).
fn compute_moon_position(
    latitude: f64,
    longitude: f64,
    datetime: &chrono::DateTime<chrono::FixedOffset>,
) -> (f32, f32, f32) {
    let timestamp = datetime.timestamp() as f64;
    let j2000_epoch = 946728000.0_f64;
    let days = (timestamp - j2000_epoch) / 86400.0;

    let l = (218.316 + 13.176396 * days) % 360.0;
    let m = (134.963 + 13.064993 * days) % 360.0;
    let f = (93.272 + 13.229350 * days) % 360.0;

    let m_rad = m.to_radians();
    let f_rad = f.to_radians();

    let ecl_lon = (l + 6.289 * m_rad.sin()).to_radians();
    let ecl_lat = (5.128 * f_rad.sin()).to_radians();

    let obliquity = 23.439_f64.to_radians();

    let sin_ra = ecl_lon.sin() * obliquity.cos() - ecl_lat.tan() * obliquity.sin();
    let cos_ra = ecl_lon.cos();
    let declination =
        (ecl_lat.cos() * obliquity.sin() * ecl_lon.sin() + ecl_lat.sin() * obliquity.cos()).asin();

    let gmst = (280.46061837 + 360.98564736629 * days) % 360.0;
    let ra = sin_ra.atan2(cos_ra).to_degrees();
    let local_sidereal = (gmst + longitude) % 360.0;
    let hour_angle = (local_sidereal - ra).to_radians();

    let lat_rad = latitude.to_radians();

    let sin_alt =
        lat_rad.sin() * declination.sin() + lat_rad.cos() * declination.cos() * hour_angle.cos();
    let elevation = sin_alt.asin();

    let cos_az = (declination.sin() - lat_rad.sin() * sin_alt) / (lat_rad.cos() * elevation.cos());
    let mut azimuth = cos_az.clamp(-1.0, 1.0).acos();
    if hour_angle.sin() > 0.0 {
        azimuth = std::f64::consts::TAU - azimuth;
    }

    // Lunar phase: synodic month = 29.530588853 days
    // Known new moon: 2000-01-06 18:14 UTC (J2000 + 5.76 days)
    let synodic_month = 29.530588853;
    let phase = ((days - 5.76) % synodic_month) / synodic_month;
    let phase = if phase < 0.0 { phase + 1.0 } else { phase };

    (
        elevation.to_degrees() as f32,
        azimuth.to_degrees() as f32,
        phase as f32,
    )
}

/// Toggle sky dome visibility based on 3D mode.
pub fn update_sky_visibility(
    state: Res<View3DState>,
    mut sky_query: Query<&mut Visibility, With<SkyDome>>,
) {
    let Ok(mut vis) = sky_query.single_mut() else {
        return;
    };
    *vis = if state.is_3d_active() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
}

/// Keep star field sprite centered on Camera2d and scaled to fill the viewport.
pub fn sync_sky_camera(
    state: Res<View3DState>,
    main_camera: Query<(&Transform, &Projection), (With<crate::MapCamera>, Without<StarField>)>,
    window_query: Query<&Window>,
    zoom_state: Res<crate::ZoomState>,
    mut star_query: Query<&mut Transform, (With<StarField>, Without<crate::MapCamera>)>,
) {
    if !state.is_3d_active() {
        return;
    }

    let Ok((main_tf, _main_proj)) = main_camera.single() else {
        return;
    };

    let Ok(mut star_tf) = star_query.single_mut() else {
        return;
    };

    // Position star field at camera XY but behind tiles
    star_tf.translation.x = main_tf.translation.x;
    star_tf.translation.y = main_tf.translation.y;
    star_tf.translation.z = STAR_Z;

    // Scale to fill viewport (account for camera zoom)
    if let Ok(window) = window_query.single() {
        let scale_factor = 1.0 / zoom_state.camera_zoom;
        // Scale sprite to cover viewport with some margin for panning
        let sx = (window.width() * scale_factor * 2.0) / 4096.0;
        let sy = (window.height() * scale_factor * 2.0) / 4096.0;
        let s = sx.max(sy);
        star_tf.scale = Vec3::new(s, s, 1.0);
    }
}

/// Spawn the star field as a 2D sprite and the ground plane mesh.
pub fn setup_sky(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Generate procedural star texture
    let star_image = generate_star_texture(4096);
    let star_texture = images.add(star_image);

    commands.spawn((
        Name::new("Star Field"),
        StarField,
        Sprite {
            image: star_texture,
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, STAR_Z),
        Visibility::Hidden,
        RenderLayers::layer(RenderCategory::SKY),
    ));

    // Spawn ground plane mesh (hidden until 3D mode activates).
    // Color matches dark CartoDB basemap tiles so the transition
    // from tiles to ground plane is seamless at distance.
    let ground_mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(250_000.0)));
    let ground_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.15, 0.15, 0.18),
        unlit: true,
        perceptual_roughness: 1.0,
        reflectance: 0.0,
        ..default()
    });
    commands.spawn((
        Name::new("Ground Plane"),
        GroundPlane,
        Mesh3d(ground_mesh),
        MeshMaterial3d(ground_material),
        // Exclude from picking — its huge mesh blocks aircraft raycasts from
        // Camera2d's rotated transform in 3D mode.
        Pickable::IGNORE,
        Transform::from_xyz(0.0, 0.0, 0.0),
        Visibility::Hidden,
        RenderLayers::layer(RenderCategory::GROUND),
    ));

    // Spawn gradient sky dome — replaces Bevy's Atmosphere sky rendering.
    // An inverted hemisphere with vertex colors that update based on sun position.
    let sky_mesh = meshes.add(generate_sky_dome_mesh());
    let sky_material = materials.add(StandardMaterial {
        unlit: true,
        cull_mode: None, // Render inside faces
        ..default()
    });
    commands.spawn((
        Name::new("Sky Dome"),
        SkyDome,
        Mesh3d(sky_mesh),
        MeshMaterial3d(sky_material),
        Pickable::IGNORE,
        Transform::from_xyz(0.0, 0.0, 0.0),
        Visibility::Hidden,
        RenderLayers::layer(RenderCategory::SKY),
    ));
}

/// Generate a hemisphere mesh for the sky dome with vertex colors.
/// The mesh is an inverted dome (normals pointing inward) centered at origin.
/// Vertices go from horizon (ring 0) to zenith (top vertex).
fn generate_sky_dome_mesh() -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();

    // Generate rings from horizon (0°) to zenith (90°)
    for ring in 0..=SKY_DOME_RINGS {
        let elevation = (ring as f32 / SKY_DOME_RINGS as f32) * std::f32::consts::FRAC_PI_2;
        let y = SKY_DOME_RADIUS * elevation.sin();
        let ring_radius = SKY_DOME_RADIUS * elevation.cos();

        for seg in 0..=SKY_DOME_SEGMENTS {
            let angle = (seg as f32 / SKY_DOME_SEGMENTS as f32) * std::f32::consts::TAU;
            let x = ring_radius * angle.cos();
            let z = ring_radius * angle.sin();

            positions.push([x, y, z]);
            // Inward-facing normals
            let n = Vec3::new(-x, -y, -z).normalize();
            normals.push([n.x, n.y, n.z]);
            // Default: daytime blue gradient (will be updated by system)
            let t = ring as f32 / SKY_DOME_RINGS as f32;
            let r = 0.4 * (1.0 - t) + 0.15 * t;
            let g = 0.5 * (1.0 - t) + 0.3 * t;
            let b = 0.7 * (1.0 - t) + 0.8 * t;
            colors.push([r, g, b, 1.0]);
        }
    }

    // Generate triangle indices
    for ring in 0..SKY_DOME_RINGS {
        for seg in 0..SKY_DOME_SEGMENTS {
            let stride = SKY_DOME_SEGMENTS + 1;
            let a = ring * stride + seg;
            let b = a + 1;
            let c = (ring + 1) * stride + seg;
            let d = c + 1;
            // Wind triangles so inward-facing side is front
            indices.push(a);
            indices.push(c);
            indices.push(b);
            indices.push(b);
            indices.push(c);
            indices.push(d);
        }
    }

    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(bevy::mesh::Indices::U32(indices));
    mesh
}

/// Update sky dome vertex colors based on sun elevation.
/// Horizon is warmer/lighter, zenith is deeper blue. At night, everything darkens.
pub fn update_sky_dome_colors(
    sun_state: Res<SunState>,
    state: Res<View3DState>,
    sky_query: Query<&Mesh3d, With<SkyDome>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if !state.is_3d_active() {
        return;
    }
    let Ok(mesh_handle) = sky_query.single() else {
        return;
    };
    let Some(mesh) = meshes.get_mut(&mesh_handle.0) else {
        return;
    };

    let elev = sun_state.elevation;

    // Day factor: 1.0 at full day (sun > 10°), 0.0 at deep night (< -12°)
    let day = if elev > 10.0 {
        1.0
    } else if elev > -12.0 {
        (elev + 12.0) / 22.0
    } else {
        0.0
    };

    // Sunset/sunrise warmth factor: peaks when sun is near horizon
    let warmth = if elev > -2.0 && elev < 8.0 {
        1.0 - ((elev - 3.0) / 5.0).abs().min(1.0)
    } else {
        0.0
    };

    let mut colors = Vec::new();
    for ring in 0..=SKY_DOME_RINGS {
        let t = ring as f32 / SKY_DOME_RINGS as f32; // 0=horizon, 1=zenith

        // Day colors: horizon (hazy light blue) → zenith (deep blue)
        let day_r = 0.70 * (1.0 - t) + 0.25 * t;
        let day_g = 0.80 * (1.0 - t) + 0.50 * t;
        let day_b = 0.95 * (1.0 - t) + 1.00 * t;

        // Night colors: horizon (dark blue-gray) → zenith (near black)
        let night_r = 0.04 * (1.0 - t) + 0.01 * t;
        let night_g = 0.04 * (1.0 - t) + 0.02 * t;
        let night_b = 0.10 * (1.0 - t) + 0.06 * t;

        // Sunset/sunrise warmth (strongest at horizon, fades by 30° elevation)
        let warm_r = 1.0 * (1.0 - t).powi(3) * warmth;
        let warm_g = 0.4 * (1.0 - t).powi(3) * warmth;

        let r = (night_r + (day_r - night_r) * day + warm_r).clamp(0.0, 1.0);
        let g = (night_g + (day_g - night_g) * day + warm_g).clamp(0.0, 1.0);
        let b = (night_b + (day_b - night_b) * day).clamp(0.0, 1.0);

        for _seg in 0..=SKY_DOME_SEGMENTS {
            colors.push([r, g, b, 1.0]);
        }
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
}

/// Generate a procedural star texture as an Image.
/// ~3000 main stars with magnitude-based brightness and color variation,
/// plus a ~2000-star Milky Way band along a diagonal gaussian belt.
fn generate_star_texture(size: u32) -> Image {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    let mut data = vec![0u8; (size * size * 4) as usize];

    // Main star field: ~3000 stars with magnitude-based brightness
    let num_stars = 3000u32;
    for i in 0..num_stars {
        let hash = pseudo_hash(i);
        let x = (hash % size) as usize;
        let y = ((hash / size) % size) as usize;

        let mag_hash = pseudo_hash(i + num_stars) % 1000;
        let mag_factor = mag_hash as f32 / 1000.0;
        let brightness = (40.0 + 215.0 * mag_factor * mag_factor * mag_factor) as u8;

        let idx = (y * size as usize + x) * 4;
        if idx + 3 < data.len() {
            let color_hash = pseudo_hash(i + num_stars * 2) % 100;
            let (r, g, b) = if color_hash < 15 {
                // Blue-white hot stars
                (
                    brightness.saturating_sub(20),
                    brightness.saturating_sub(10),
                    brightness,
                )
            } else if color_hash < 25 {
                // Warm yellow stars
                (
                    brightness,
                    brightness.saturating_sub(15),
                    brightness.saturating_sub(40),
                )
            } else {
                (brightness, brightness, brightness)
            };
            data[idx] = r;
            data[idx + 1] = g;
            data[idx + 2] = b;
            data[idx + 3] = 255;
        }
    }

    // Milky Way band: extra-dim stars concentrated along a diagonal gaussian belt
    let milky_way_stars = 2000u32;
    for i in 0..milky_way_stars {
        let hash = pseudo_hash(i + num_stars * 3);
        let along = (hash % 10000) as f32 / 10000.0;
        let offset_hash = pseudo_hash(i + num_stars * 4);
        let gauss = ((offset_hash % 100) as f32 / 100.0 - 0.5)
            + ((pseudo_hash(i + num_stars * 5) % 100) as f32 / 100.0 - 0.5);
        let band_width = 0.08;
        let perpendicular = gauss * band_width;

        let x = ((along + perpendicular * 0.7) * size as f32) as usize % size as usize;
        let y = ((along * 0.8 + 0.1 + perpendicular) * size as f32) as usize % size as usize;

        let brightness = 25 + (pseudo_hash(i + num_stars * 6) % 35) as u8;

        let idx = (y * size as usize + x) * 4;
        if idx + 3 < data.len() {
            // Additive blending: don't overwrite brighter stars with dimmer Milky Way pixels
            let existing = data[idx];
            if brightness > existing {
                data[idx] = brightness;
                data[idx + 1] = brightness;
                data[idx + 2] = brightness + 5;
                data[idx + 3] = 255;
            }
        }
    }

    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        default(),
    )
}

/// Simple deterministic hash for star placement.
fn pseudo_hash(seed: u32) -> u32 {
    let mut h = seed.wrapping_mul(2654435761);
    h ^= h >> 16;
    h = h.wrapping_mul(2246822507);
    h ^= h >> 13;
    h
}

/// Hide star field sprite. The star field is a Camera2d sprite; in 3D mode,
/// Camera2d composites on top of Camera3d via alpha blending, so visible stars
/// bleed through onto the mesh quad tiles. The Atmosphere component handles
/// sky rendering. Night stars would require a Camera3d skybox mesh.
pub fn update_star_visibility(
    _state: Res<View3DState>,
    _sun_state: Res<SunState>,
    _time: Res<Time>,
    mut star_query: Query<&mut Visibility, With<StarField>>,
) {
    let Ok(mut vis) = star_query.single_mut() else {
        return;
    };
    *vis = Visibility::Hidden;
}

/// Update sun direction from time state and map coordinates.
pub fn update_sun_position(
    map_state: Res<MapState>,
    time_state: Res<TimeState>,
    mut sun_state: ResMut<SunState>,
    mut sun_query: Query<(&mut DirectionalLight, &mut Transform), With<SunLight>>,
    mut ambient: ResMut<GlobalAmbientLight>,
) {
    let datetime = time_state.current_datetime();
    let (elevation, azimuth) =
        compute_sun_position_at(map_state.latitude, map_state.longitude, &datetime);

    // Only update when position changes meaningfully (0.05 degrees ≈ 12 seconds of time).
    // Avoiding per-frame writes prevents Bevy change detection from triggering
    // atmosphere/fog recalculation every frame, which causes tile flashing.
    let elev_changed = (sun_state.elevation - elevation).abs() > 0.05;
    let azim_changed = (sun_state.azimuth - azimuth).abs() > 0.05;
    if !elev_changed && !azim_changed {
        return;
    }

    sun_state.elevation = elevation;
    sun_state.azimuth = azimuth;

    let Ok((mut light, mut transform)) = sun_query.single_mut() else {
        return;
    };

    // Convert sun elevation and azimuth to directional light rotation.
    let elev_rad = elevation.to_radians();
    let azim_rad = azimuth.to_radians();
    *transform =
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, -azim_rad, -elev_rad, 0.0));

    // Scale illuminance with sun elevation (128,000 lux = raw sunlight pre-scattering)
    if elevation > 0.0 {
        let factor = (elevation / 90.0).clamp(0.0, 1.0);
        light.illuminance = 128_000.0 * factor.sqrt();
    } else {
        light.illuminance = 0.0;
    }

    // Ambient light with civil/nautical/astronomical twilight zones
    let ambient_factor = if elevation > 0.0 {
        1.0
    } else if elevation > -6.0 {
        // Civil twilight: -6 to 0 degrees
        ((elevation + 6.0) / 6.0).clamp(0.0, 1.0) * 0.8 + 0.2
    } else if elevation > -12.0 {
        // Nautical twilight: -12 to -6 degrees
        ((elevation + 12.0) / 6.0).clamp(0.0, 1.0) * 0.15 + 0.05
    } else if elevation > -18.0 {
        // Astronomical twilight: -18 to -12 degrees
        ((elevation + 18.0) / 6.0).clamp(0.0, 1.0) * 0.04 + 0.01
    } else {
        // Full night
        0.01
    };

    ambient.brightness = 300.0 * ambient_factor;
}

/// Update moon position and moonlight from time and map coordinates.
pub fn update_moon_position(
    map_state: Res<MapState>,
    time_state: Res<TimeState>,
    mut moon_state: ResMut<MoonState>,
    mut moon_query: Query<(&mut DirectionalLight, &mut Transform), With<MoonLight>>,
) {
    let datetime = time_state.current_datetime();
    let (elevation, azimuth, phase) =
        compute_moon_position(map_state.latitude, map_state.longitude, &datetime);

    // Only update when position changes meaningfully.
    let elev_changed = (moon_state.elevation - elevation).abs() > 0.05;
    let azim_changed = (moon_state.azimuth - azimuth).abs() > 0.05;
    if !elev_changed && !azim_changed {
        return;
    }

    moon_state.elevation = elevation;
    moon_state.azimuth = azimuth;
    moon_state.phase = phase;

    let Ok((mut light, mut transform)) = moon_query.single_mut() else {
        return;
    };

    let elev_rad = elevation.to_radians();
    let azim_rad = azimuth.to_radians();
    *transform =
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, -azim_rad, -elev_rad, 0.0));

    // Full moon ~0.25 lux, scaled by phase (sine curve peaks at phase=0.5)
    let phase_illuminance = (std::f32::consts::PI * phase).sin();
    if elevation > 0.0 {
        let elev_factor = (elevation / 90.0).clamp(0.0, 1.0).sqrt();
        light.illuminance = 0.25 * phase_illuminance * elev_factor;
    } else {
        light.illuminance = 0.0;
    }
}

/// Keep time offset in sync with map longitude, and apply BRP override_hour.
pub fn sync_time_offset(map_state: Res<MapState>, mut time_state: ResMut<TimeState>) {
    let new_offset = (map_state.longitude / 15.0) as f32;
    if (time_state.utc_offset_hours - new_offset).abs() > 0.01 {
        time_state.utc_offset_hours = new_offset;
    }
    // Apply BRP-triggered hour override
    if time_state.override_hour >= 0.0 {
        let hour = time_state.override_hour;
        time_state.set_hour(hour);
        time_state.override_hour = -1.0;
    }
}

/// Manage camera settings based on 2D/3D mode.
///
/// Camera2d (order 0) renders map tiles, gizmos, labels as the base layer.
/// Camera3d (order 1) alpha-blends 3D content on top via CameraOutputMode::Write.
/// Without Atmosphere/HDR, Camera3d's output supports alpha compositing natively.
///
/// NOTE: Bevy's Atmosphere component was removed due to multi-camera HDR
/// tonemapping bugs on Metal (#18901, #18902, #17530) that non-deterministically
/// produce a black screen. A gradient sky dome mesh replaces the atmosphere sky.
/// See the tech debt GitHub issue for restoration plan.
pub fn manage_camera_mode(
    state: Res<View3DState>,
    mut camera_3d: Query<
        (&mut Camera, &mut DistanceFog),
        (With<crate::AircraftCamera>, Without<crate::MapCamera>),
    >,
    mut camera_2d: Query<&mut Camera, (With<crate::MapCamera>, Without<Camera3d>)>,
    mut ground_query: Query<(&mut Transform, &mut Visibility), With<GroundPlane>>,
    mut last_3d: Local<Option<bool>>,
) {
    let Ok((mut cam3d, mut fog)) = camera_3d.single_mut() else {
        return;
    };
    let Ok(mut cam2d) = camera_2d.single_mut() else {
        return;
    };

    if state.is_3d_active() {
        fog.falloff = FogFalloff::Linear {
            start: state.visibility_range * 0.4,
            end: state.visibility_range,
        };

        // Camera ordering: Camera2d base (order 0), Camera3d overlay (order 1)
        cam2d.order = 0;
        cam3d.order = 1;

        if *last_3d != Some(true) {
            *last_3d = Some(true);
            if let Ok((_, mut gp_vis)) = ground_query.single_mut() {
                *gp_vis = Visibility::Inherited;
            }
        }
    } else {
        // 2D mode: disable fog, Camera2d is primary
        fog.falloff = FogFalloff::Linear {
            start: 999999.0,
            end: 999999.0,
        };

        cam2d.order = 0;
        cam3d.order = 1;

        if *last_3d != Some(false) {
            *last_3d = Some(false);
            cam2d.clear_color = ClearColorConfig::Default;
            cam2d.output_mode = CameraOutputMode::default();
            if let Ok((_, mut gp_vis)) = ground_query.single_mut() {
                *gp_vis = Visibility::Hidden;
            }
        }
    }
}

/// Blend DistanceFog color between daytime blue-gray and nighttime dark
/// based on sun elevation, so the fog matches the scene lighting.
pub fn update_fog_color_for_time(
    sun_state: Res<SunState>,
    state: Res<View3DState>,
    mut fog_query: Query<&mut DistanceFog, With<Camera3d>>,
) {
    if !state.is_3d_active() {
        return;
    }
    let Ok(mut fog) = fog_query.single_mut() else {
        return;
    };

    let elev = sun_state.elevation;

    // Blend factor: 1.0 at full day (sun > 10°), 0.0 at night (sun < -6°)
    let t = if elev > 10.0 {
        1.0
    } else if elev > -6.0 {
        (elev + 6.0) / 16.0
    } else {
        0.0
    };

    // Day: hazy blue matching sky horizon. Night: dark matching tiles.
    // Fog should blend tiles into the sky dome seamlessly at distance.
    let r = 0.08 + 0.55 * t;
    let g = 0.08 + 0.65 * t;
    let b = 0.10 + 0.80 * t;
    let a = 0.85 - 0.55 * t; // Night: 0.85, Day: 0.3
    fog.color = Color::srgba(r, g, b, a);
}

/// Adapt camera exposure to time of day.
/// At night, lower the EV100 so faint atmospheric scattering isn't amplified
/// into a bright horizon glow.
///
/// Uses a large dead zone (0.5 EV) to avoid frequent updates that can cause
/// the atmosphere post-process to re-render and flash tiles near the horizon.
pub fn update_exposure_for_time(
    sun_state: Res<SunState>,
    state: Res<View3DState>,
    mut camera_query: Query<&mut Exposure, With<Camera3d>>,
) {
    if !state.is_3d_active() {
        return;
    }
    let Ok(mut exposure) = camera_query.single_mut() else {
        return;
    };

    let elev = sun_state.elevation;

    // EV100: 13.0 at full day, ramp down through twilight to 2.0 at night.
    let ev = if elev > 10.0 {
        13.0
    } else if elev > 0.0 {
        // Low sun: 5..13
        5.0 + 8.0 * (elev / 10.0)
    } else if elev > -6.0 {
        // Civil twilight: 2..5
        2.0 + 3.0 * ((elev + 6.0) / 6.0)
    } else {
        // Night
        2.0
    };

    // Only write when crossing a significant threshold to minimize
    // atmosphere post-process recalculations that can flash tiles.
    if (exposure.ev100 - ev).abs() > 0.5 {
        exposure.ev100 = ev;
    }
}

/// Keep the ground plane centered on the camera target in Y-up space.
pub fn sync_ground_plane(
    state: Res<View3DState>,
    mut ground_query: Query<(&mut Transform, &mut Visibility), With<GroundPlane>>,
) {
    let Ok((mut gp_transform, mut gp_vis)) = ground_query.single_mut() else {
        return;
    };

    if state.is_3d_active() {
        *gp_vis = Visibility::Inherited;
        // Place the ground plane slightly below the tile mesh quads so tiles
        // always render on top. Lower-zoom tiles can be up to 0.2 units below
        // the base ground level (zoom_diff * 0.05), so offset by 1.0 to clear all.
        let ground_alt = state.altitude_to_z(state.ground_elevation_ft) - 1.0;
        let pos_yup = super::zup_to_yup(Vec3::new(
            state.saved_2d_center.x,
            state.saved_2d_center.y,
            ground_alt,
        ));
        gp_transform.translation = pos_yup;
    } else {
        *gp_vis = Visibility::Hidden;
    }
}

/// Keep sky dome centered on Camera3d so it always surrounds the viewer.
pub fn sync_sky_dome(
    state: Res<View3DState>,
    camera_query: Query<&Transform, (With<crate::AircraftCamera>, Without<SkyDome>)>,
    mut sky_query: Query<&mut Transform, (With<SkyDome>, Without<crate::AircraftCamera>)>,
) {
    if !state.is_3d_active() {
        return;
    }
    let Ok(cam_tf) = camera_query.single() else {
        return;
    };
    let Ok(mut sky_tf) = sky_query.single_mut() else {
        return;
    };
    sky_tf.translation = cam_tf.translation;
}

/// Blend ground plane color between day (hazy blue-gray matching sky horizon)
/// and night (dark matching CartoDB tiles) based on sun elevation.
pub fn update_ground_plane_color(
    sun_state: Res<SunState>,
    state: Res<View3DState>,
    ground_query: Query<&MeshMaterial3d<StandardMaterial>, With<GroundPlane>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !state.is_3d_active() {
        return;
    }
    let Ok(mat_handle) = ground_query.single() else {
        return;
    };
    let Some(material) = materials.get_mut(mat_handle.id()) else {
        return;
    };

    let elev = sun_state.elevation;
    let day = if elev > 10.0 {
        1.0
    } else if elev > -6.0 {
        (elev + 6.0) / 16.0
    } else {
        0.0
    };

    // Day: hazy blue-gray matching sky horizon. Night: dark CartoDB tile color.
    let r = 0.078 + (0.45 - 0.078) * day;
    let g = 0.082 + (0.52 - 0.082) * day;
    let b = 0.094 + (0.60 - 0.094) * day;
    material.base_color = Color::srgb(r, g, b);
}
