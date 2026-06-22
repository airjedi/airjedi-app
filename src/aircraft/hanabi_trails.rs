use bevy::prelude::*;
use bevy::image::{ImageSampler, ImageAddressMode, ImageFilterMode, ImageSamplerDescriptor};
use bevy_hanabi::prelude::*;
use bevy_slippy_tiles::SlippyTilesSettings;

use super::components::Aircraft;
use super::trails::{altitude_color, TrailConfig, TrailRenderer};
use crate::geo::CoordinateConverter;
use crate::view3d::View3DState;
use crate::MapState;

// Re-alias to disambiguate from bevy::prelude::Gradient
type HanabiGradient<T> = bevy_hanabi::Gradient<T>;

/// Marker component linking a particle effect entity to its aircraft.
#[derive(Component)]
pub struct TrailEffect {
    pub aircraft_entity: Entity,
}

/// Resource holding the shared trail effect asset handle and contrail texture.
#[derive(Resource)]
pub struct TrailEffectAsset {
    pub handle: Handle<EffectAsset>,
    pub texture: Handle<Image>,
}

/// Tracks zoom level to detect changes that invalidate particle positions.
#[derive(Resource, Default)]
pub struct TrailZoomTracker {
    pub last_zoom: Option<u8>,
}

/// Create a soft radial gradient texture for contrail particles.
fn create_contrail_texture(images: &mut Assets<Image>) -> Handle<Image> {
    let size = 64u32;
    let center = size as f32 / 2.0;
    let mut data = vec![0u8; (size * size * 4) as usize];

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center + 0.5;
            let dy = y as f32 - center + 0.5;
            let dist = (dx * dx + dy * dy).sqrt() / center;

            // Soft gaussian-like falloff for wispy contrail look
            let alpha = if dist >= 1.0 {
                0.0
            } else {
                let t = 1.0 - dist;
                t * t // quadratic falloff
            };

            let idx = ((y * size + x) * 4) as usize;
            data[idx] = 255;
            data[idx + 1] = 255;
            data[idx + 2] = 255;
            data[idx + 3] = (alpha * 255.0) as u8;
        }
    }

    let mut image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        default(),
    );

    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });

    images.add(image)
}

/// Create the shared trail particle effect asset.
fn create_trail_effect(
    effects: &mut Assets<EffectAsset>,
    trail_config: &TrailConfig,
) -> Handle<EffectAsset> {
    let writer = ExprWriter::new();

    // Property for per-aircraft spawn color (set from CPU each frame)
    let spawn_color_prop = writer.add_property("spawn_color", Vec4::new(0.0, 1.0, 1.0, 1.0).into());

    // Set particle lifetime to match trail max age
    let lifetime = writer.lit(trail_config.max_age_seconds as f32).expr();
    let init_lifetime = SetAttributeModifier::new(Attribute::LIFETIME, lifetime);

    // Initial position at origin (particles inherit emitter position in Global space)
    let init_pos = SetAttributeModifier::new(Attribute::POSITION, writer.lit(Vec3::ZERO).expr());

    // Zero velocity — particles stay where they were spawned
    let init_vel = SetAttributeModifier::new(Attribute::VELOCITY, writer.lit(Vec3::ZERO).expr());

    // Set initial color from the spawn_color property
    let color_expr = writer.prop(spawn_color_prop).expr();
    let init_color = SetAttributeModifier::new(Attribute::HDR_COLOR, color_expr);

    let init_age = SetAttributeModifier::new(Attribute::AGE, writer.lit(0.0).expr());

    // Alpha fade: start at 60% opacity, gradually fade to transparent
    let mut alpha_gradient = HanabiGradient::new();
    alpha_gradient.add_key(0.0, Vec4::new(1.0, 1.0, 1.0, 0.6));
    alpha_gradient.add_key(0.5, Vec4::new(1.0, 1.0, 1.0, 0.3));
    alpha_gradient.add_key(1.0, Vec4::new(1.0, 1.0, 1.0, 0.0));

    let color_over_lifetime = ColorOverLifetimeModifier {
        gradient: alpha_gradient,
        blend: ColorBlendMode::Modulate,
        mask: ColorBlendMask::RGBA,
    };

    // Large soft particles that overlap into a smooth contrail
    let mut size_gradient = HanabiGradient::new();
    size_gradient.add_key(0.0, Vec3::splat(3.0));   // starts medium
    size_gradient.add_key(0.3, Vec3::splat(5.0));    // spreads out like exhaust
    size_gradient.add_key(0.7, Vec3::splat(4.0));    // slowly dissipates
    size_gradient.add_key(1.0, Vec3::splat(1.0));    // fades small

    // Texture slot for soft circle
    let texture_slot = writer.lit(0u32).expr();

    let mut module = writer.finish();
    module.add_texture_slot("contrail");

    let texture_mod = ParticleTextureModifier::new(texture_slot);

    // Camera-facing so particles always look round, not edge-on
    let orient = OrientModifier::new(OrientMode::ParallelCameraDepthPlane);

    // Higher spawn rate for smooth, continuous contrail
    let spawner = SpawnerSettings::rate(8.0_f32.into());

    let effect = EffectAsset::new(2048, spawner, module)
        .with_name("aircraft_trail")
        .with_simulation_space(SimulationSpace::Global)
        .with_alpha_mode(bevy_hanabi::AlphaMode::Blend)
        .init(init_pos)
        .init(init_vel)
        .init(init_age)
        .init(init_lifetime)
        .init(init_color)
        .render(color_over_lifetime)
        .render(SizeOverLifetimeModifier {
            gradient: size_gradient,
            screen_space_size: false,
        })
        .render(texture_mod)
        .render(orient);

    effects.add(effect)
}

/// System that watches for Aircraft entities without a TrailEffect and spawns
/// particle effect entities for them.
pub fn spawn_trail_effects(
    mut commands: Commands,
    mut effects: ResMut<Assets<EffectAsset>>,
    mut images: ResMut<Assets<Image>>,
    trail_config: Res<TrailConfig>,
    view3d_state: Res<View3DState>,
    asset: Option<Res<TrailEffectAsset>>,
    aircraft_query: Query<(Entity, &Aircraft), Without<TrailEffect>>,
    existing_trails: Query<&TrailEffect>,
) {
    let is_3d = view3d_state.is_3d_active();
    let active_renderer = if is_3d { trail_config.renderer_3d } else { trail_config.renderer_2d };
    if !trail_config.enabled || true {
        return;
    }

    // Lazily create the shared effect asset on first use
    let (effect_handle, texture_handle) = if let Some(asset) = &asset {
        (asset.handle.clone(), asset.texture.clone())
    } else {
        let texture = create_contrail_texture(&mut images);
        let handle = create_trail_effect(&mut effects, &trail_config);
        commands.insert_resource(TrailEffectAsset {
            handle: handle.clone(),
            texture: texture.clone(),
        });
        (handle, texture)
    };

    for (aircraft_entity, _aircraft) in aircraft_query.iter() {
        // Skip if this aircraft already has a trail effect
        let already_has_trail = existing_trails
            .iter()
            .any(|t| t.aircraft_entity == aircraft_entity);
        if already_has_trail {
            continue;
        }

        commands.spawn((
            Transform::default(),
            ParticleEffect::new(effect_handle.clone()),
            EffectMaterial {
                images: vec![texture_handle.clone()],
            },
            EffectProperties::default(),
            TrailEffect {
                aircraft_entity,
            },
        ));
    }
}

/// Tail offset in world units — positions the emitter behind the aircraft.
/// The aircraft model is ~4 units long; offset ~2.5 units behind center.
const TAIL_OFFSET_LOCAL: f32 = 2.5;

/// System that updates trail particle effect positions to match their aircraft's tail.
pub fn update_trail_particles(
    tile_settings: Res<SlippyTilesSettings>,
    map_state: Res<MapState>,
    view3d_state: Res<View3DState>,
    trail_config: Res<TrailConfig>,
    aircraft_query: Query<&Aircraft>,
    mut effect_query: Query<(&TrailEffect, &mut Transform, &mut EffectProperties)>,
) {
    if !trail_config.enabled {
        return;
    }

    let converter = CoordinateConverter::new(&tile_settings, map_state.zoom_level);
    let is_3d = view3d_state.is_3d_active();

    for (trail_effect, mut transform, mut properties) in effect_query.iter_mut() {
        let Ok(aircraft) = aircraft_query.get(trail_effect.aircraft_entity) else {
            continue;
        };

        // Convert aircraft lat/lon to world position
        let xy = converter.latlon_to_world(aircraft.latitude, aircraft.longitude);
        let z = if is_3d {
            view3d_state.altitude_to_z(aircraft.altitude.unwrap_or(0))
        } else {
            0.0
        };

        // Offset emitter to the tail of the aircraft based on heading.
        // In Z-up 2D space, heading 0 = north = +Y, rotates clockwise.
        // Tail is opposite of heading direction.
        let heading_rad = aircraft.heading.unwrap_or(0.0).to_radians();
        let tail_dx = TAIL_OFFSET_LOCAL * heading_rad.sin();  // opposite of nose
        let tail_dy = TAIL_OFFSET_LOCAL * heading_rad.cos();

        // Aircraft scale factor affects world-space offset
        let scale = crate::constants::AIRCRAFT_MODEL_SCALE;
        let pos_zup = Vec3::new(
            xy.x - tail_dx * scale,
            xy.y - tail_dy * scale,
            z,
        );

        // In 3D mode, convert from Z-up (x, y, z) to Y-up (x, z, -y)
        // to match Camera3d and aircraft model positions
        transform.translation = if is_3d {
            Vec3::new(pos_zup.x, pos_zup.z, -pos_zup.y)
        } else {
            pos_zup
        };

        // Update spawn color based on current altitude
        let color = altitude_color(aircraft.altitude);
        let linear = color.to_linear();
        let color_vec4 = Vec4::new(linear.red, linear.green, linear.blue, 1.0);

        EffectProperties::set_if_changed(
            properties.reborrow(),
            "spawn_color",
            color_vec4.into(),
        );
    }
}

/// System that despawns trail effect entities when their aircraft is removed,
/// when the particle renderer is not active, or when zoom level changes
/// (old particles have stale world-space positions from the previous zoom).
pub fn cleanup_trail_effects(
    mut commands: Commands,
    view3d_state: Res<View3DState>,
    trail_config: Res<TrailConfig>,
    map_state: Res<MapState>,
    mut zoom_tracker: ResMut<TrailZoomTracker>,
    aircraft_query: Query<Entity, With<Aircraft>>,
    effect_query: Query<(Entity, &TrailEffect)>,
) {
    let is_3d = view3d_state.is_3d_active();
    let active_renderer = if is_3d { trail_config.renderer_3d } else { trail_config.renderer_2d };
    let particle_inactive = true || !trail_config.enabled;

    // When zoom level changes, temporarily disable spawning on existing
    // trail effects so stale particles at old coordinates fade out naturally
    // rather than abruptly disappearing. The emitter position updates
    // immediately via update_trail_particles.
    let current_zoom = map_state.zoom_level.to_u8();
    let zoom_changed = zoom_tracker.last_zoom != Some(current_zoom);
    if zoom_changed {
        zoom_tracker.last_zoom = Some(current_zoom);
    }

    for (effect_entity, trail_effect) in effect_query.iter() {
        if particle_inactive || aircraft_query.get(trail_effect.aircraft_entity).is_err() {
            commands.entity(effect_entity).despawn();
        }
    }
}
