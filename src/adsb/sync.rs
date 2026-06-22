use bevy::prelude::*;
use std::collections::HashMap;

use bevy::camera::visibility::RenderLayers;
use crate::{constants, Aircraft, AircraftLabel, RenderCategory};
use crate::aircraft::TrailHistory;
use crate::aircraft::picking::{on_aircraft_click, on_aircraft_hover, on_aircraft_out};
use crate::aircraft::interpolation::{InterpolationState, update_interpolation_on_adsb};
use crate::debug_panel::DebugPanelState;
use super::connection::{AdsbAircraftData, ConnectionStatusText};

use crate::theme::AppTheme;

/// Type codes that should use the B737 model
const B737_TYPES: &[&str] = &[
    "B731", "B732", "B733", "B734", "B735", "B736", "B737", "B738", "B739",
    "B37M", "B38M", "B39M",
];

/// Correction transform applied to a model's child mesh entities after scene
/// loading, to re-center and re-orient models whose origin/axes differ from
/// the default GLB convention (nose=+Z, up=+Y, centered at origin).
#[derive(Component, Clone)]
pub struct ModelCorrection {
    pub transform: Transform,
}

/// Marker: correction has been applied to this entity's children.
#[derive(Component)]
pub struct ModelCorrectionApplied;

/// Marker: materials have been set to unlit for this aircraft's mesh children.
#[derive(Component)]
pub struct MaterialsUnlit;

/// Resource holding aircraft 3D model handles keyed by type code
#[derive(Resource)]
pub struct AircraftModelRegistry {
    pub default_model: Handle<Scene>,
    pub type_models: HashMap<String, Handle<Scene>>,
    pub corrections: HashMap<String, ModelCorrection>,
}

impl AircraftModelRegistry {
    /// Get the model handle for a given type code, falling back to the default
    pub fn get_model(&self, type_code: Option<&str>) -> Handle<Scene> {
        if let Some(code) = type_code {
            if let Some(handle) = self.type_models.get(code) {
                return handle.clone();
            }
        }
        self.default_model.clone()
    }

    /// Get the model correction for a given type code, if any
    pub fn get_correction(&self, type_code: Option<&str>) -> Option<ModelCorrection> {
        type_code.and_then(|code| self.corrections.get(code).cloned())
    }
}

/// Load aircraft 3D models and build the registry.
/// The default GLB is loaded with MAIN_WORLD asset usage so mesh data
/// is retained on the CPU for picking raycasts (not just uploaded to GPU).
pub fn setup_aircraft_models(mut commands: Commands, asset_server: Res<AssetServer>) {
    use bevy::asset::RenderAssetUsages;
    use bevy::gltf::GltfLoaderSettings;

    let default_model = asset_server.load_with_settings(
        "airplane.glb#Scene0",
        |settings: &mut GltfLoaderSettings| {
            settings.load_meshes = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        },
    );
    let b737_model: Handle<Scene> = asset_server.load("models/b737/78349.obj");

    let mut type_models = HashMap::new();
    for code in B737_TYPES {
        type_models.insert(code.to_string(), b737_model.clone());
    }

    // B737 OBJ correction: mesh center is at ~(0, 69.5, -47.9) in OBJ space,
    // with nose at -Y direction and height along -Z. The default GLB expects
    // nose=+Z, up=+Y, centered at origin.
    //
    // Axis mapping: R_x(-90°) maps OBJ -Y → GLB +Z (nose forward).
    // Scale: 0.45 matches the GLB model size (~3.9 unit fuselage).
    // Translation: T = -(R * (S * mesh_center)) to re-center after rotation
    // and scale, so the mesh center sits at the entity's transform origin.
    let scale = 0.45_f32;
    let mesh_center = Vec3::new(0.0, 69.5, -47.9);
    let rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    let translation = -(rotation * (scale * mesh_center));
    let b737_correction = ModelCorrection {
        transform: Transform {
            translation,
            rotation,
            scale: Vec3::splat(scale),
        },
    };
    let mut corrections = HashMap::new();
    for code in B737_TYPES {
        corrections.insert(code.to_string(), b737_correction.clone());
    }

    commands.insert_resource(AircraftModelRegistry {
        default_model,
        type_models,
        corrections,
    });
}

/// Sync aircraft entities from the shared ADS-B data.
/// This system runs every frame and updates Bevy entities to match the ADS-B client state.
pub fn sync_aircraft_from_adsb(
    mut commands: Commands,
    model_registry: Option<Res<AircraftModelRegistry>>,
    adsb_data: Option<Res<AdsbAircraftData>>,
    mut aircraft_query: Query<(Entity, &mut Aircraft, &mut Transform, Option<&mut InterpolationState>)>,
    label_query: Query<(Entity, &AircraftLabel)>,
    mut debug: Option<ResMut<DebugPanelState>>,
    theme: Res<AppTheme>,
    type_db: Option<Res<crate::aircraft::AircraftTypeDatabase>>,
    time: Res<Time<Real>>,
) {
    let Some(adsb_data) = adsb_data else {
        return; // ADS-B client not yet initialized
    };
    let Some(model_registry) = model_registry else {
        return; // Aircraft model registry not yet loaded
    };

    // Use try_get to avoid blocking the main thread if the background ADS-B
    // thread currently holds the lock. We'll just skip this frame and retry next frame.
    let Some(adsb_aircraft) = adsb_data.try_get_aircraft() else {
        return;
    };

    // Build a map of existing aircraft entities by ICAO
    let mut existing_aircraft: HashMap<String, Entity> = aircraft_query
        .iter()
        .map(|(entity, aircraft, _, _)| (aircraft.icao.clone(), entity))
        .collect();

    // Update or spawn aircraft
    for adsb_ac in &adsb_aircraft {
        if let Some(ref mut dbg) = debug {
            dbg.messages_processed += 1;
        }

        // Skip aircraft without position data
        let (Some(lat), Some(lon)) = (adsb_ac.latitude, adsb_ac.longitude) else {
            if let Some(ref mut dbg) = debug {
                dbg.positions_rejected += 1;
            }
            continue;
        };

        if let Some(&entity) = existing_aircraft.get(&adsb_ac.icao) {
            // Update existing aircraft
            if let Ok((_, mut aircraft, _, interp_opt)) = aircraft_query.get_mut(entity) {
                let position_changed = (lat - aircraft.latitude).abs() > f64::EPSILON
                    || (lon - aircraft.longitude).abs() > f64::EPSILON;

                aircraft.latitude = lat;
                aircraft.longitude = lon;
                aircraft.altitude = adsb_ac.altitude;
                aircraft.heading = adsb_ac.track.map(|t| t as f32);
                aircraft.velocity = adsb_ac.velocity;
                aircraft.vertical_rate = adsb_ac.vertical_rate;
                aircraft.callsign = adsb_ac.callsign.clone();
                aircraft.squawk = adsb_ac.squawk.clone();
                aircraft.is_on_ground = adsb_ac.is_on_ground;
                aircraft.alert = adsb_ac.alert;
                aircraft.emergency = adsb_ac.emergency;
                aircraft.spi = adsb_ac.spi;
                aircraft.last_seen = adsb_ac.last_seen;

                if position_changed {
                    if let Some(mut interp) = interp_opt {
                        update_interpolation_on_adsb(
                            &mut interp,
                            lat, lon,
                            adsb_ac.altitude,
                            adsb_ac.track.map(|t| t as f32),
                            adsb_ac.velocity,
                            adsb_ac.vertical_rate,
                            adsb_ac.is_on_ground,
                            time.elapsed_secs_f64(),
                        );
                    }
                }
            }
            existing_aircraft.remove(&adsb_ac.icao);
        } else {
            // Log new aircraft
            if let Some(ref mut dbg) = debug {
                let callsign = adsb_ac.callsign.as_deref().unwrap_or("?");
                dbg.push_log(format!("New aircraft: {} ({})", adsb_ac.icao, callsign));
            }
            // Spawn new aircraft with 3D model
            let aircraft_name = adsb_ac.callsign.as_deref().unwrap_or(&adsb_ac.icao);

            // Look up type code for model selection
            let type_code = type_db
                .as_ref()
                .and_then(|db| db.lookup(&adsb_ac.icao))
                .and_then(|info| info.type_code.clone());

            let model_handle = model_registry.get_model(type_code.as_deref());
            let correction = model_registry.get_correction(type_code.as_deref());

            let mut entity_commands = commands.spawn((
                    Name::new(format!("Aircraft: {}", aircraft_name)),
                    SceneRoot(model_handle),
                    Transform::from_xyz(0.0, 0.0, constants::AIRCRAFT_Z_LAYER),
                    Pickable::default(),
                    Aircraft {
                        icao: adsb_ac.icao.clone(),
                        callsign: adsb_ac.callsign.clone(),
                        latitude: lat,
                        longitude: lon,
                        altitude: adsb_ac.altitude,
                        heading: adsb_ac.track.map(|t| t as f32),
                        velocity: adsb_ac.velocity,
                        vertical_rate: adsb_ac.vertical_rate,
                        squawk: adsb_ac.squawk.clone(),
                        is_on_ground: adsb_ac.is_on_ground,
                        alert: adsb_ac.alert,
                        emergency: adsb_ac.emergency,
                        spi: adsb_ac.spi,
                        last_seen: adsb_ac.last_seen,
                    },
                    TrailHistory::default(),
                    InterpolationState::new(
                        lat, lon,
                        adsb_ac.altitude,
                        adsb_ac.track.map(|t| t as f32),
                        adsb_ac.velocity,
                        adsb_ac.vertical_rate,
                        adsb_ac.is_on_ground,
                        time.elapsed_secs_f64(),
                    ),
                ));
            if let Some(corr) = correction {
                entity_commands.insert(corr);
            }
            let aircraft_entity = entity_commands
                .observe(on_aircraft_click)
                .observe(on_aircraft_hover)
                .observe(on_aircraft_out)
                .id();

            // Spawn label for this aircraft
            let callsign_display = adsb_ac.callsign.as_deref().unwrap_or(&adsb_ac.icao);
            let alt_display = adsb_ac
                .altitude
                .map(|a| format!("{} ft", a))
                .unwrap_or_default();
            let label_text = format!("{}\n{}", callsign_display, alt_display);

            commands.spawn((
                Name::new(format!("Label: {}", aircraft_name)),
                Text2d::new(label_text),
                TextFont {
                    font_size: constants::BASE_FONT_SIZE,
                    ..default()
                },
                TextColor(theme.text_primary()),
                Transform::from_xyz(0.0, 0.0, constants::LABEL_Z_LAYER),
                AircraftLabel {
                    aircraft_entity,
                },
                RenderLayers::layer(RenderCategory::LABELS),
            ));
        }
    }

    // Remove aircraft that are no longer in the ADS-B data
    for (icao, _entity) in &existing_aircraft {
        if let Some(ref mut dbg) = debug {
            dbg.push_log(format!("Removed aircraft: {}", icao));
        }
    }
    for (icao, entity) in existing_aircraft {
        // Find and despawn the label first
        for (label_entity, label) in label_query.iter() {
            if label.aircraft_entity == entity {
                commands.entity(label_entity).despawn();
                break;
            }
        }
        commands.entity(entity).despawn();
        info!("Removed aircraft {} from display", icao);
    }
}

/// Apply model corrections to child mesh entities after scene loading.
/// Runs every frame but only processes uncorrected entities (those with
/// ModelCorrection but without ModelCorrectionApplied). Once children
/// are found and corrected, the entity is marked as applied.
pub fn apply_model_corrections(
    mut commands: Commands,
    parent_query: Query<(Entity, &ModelCorrection, &Children), Without<ModelCorrectionApplied>>,
    mut transform_query: Query<&mut Transform>,
) {
    for (entity, correction, children) in parent_query.iter() {
        let mut applied = false;
        for child in children.iter() {
            if let Ok(mut child_transform) = transform_query.get_mut(child) {
                // Apply the correction: re-center, re-orient, and rescale
                child_transform.translation += correction.transform.translation;
                child_transform.rotation = correction.transform.rotation * child_transform.rotation;
                child_transform.scale *= correction.transform.scale;
                applied = true;
            }
        }
        if applied {
            commands.entity(entity).insert(ModelCorrectionApplied);
        }
    }
}

/// Make aircraft model materials self-lit so they aren't affected by the
/// day/night ambient lighting cycle. Runs once per aircraft after scene load.
pub fn make_aircraft_unlit(
    mut commands: Commands,
    aircraft_query: Query<(Entity, &Children), (With<Aircraft>, Without<MaterialsUnlit>)>,
    children_query: Query<&Children>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    material_query: Query<&MeshMaterial3d<StandardMaterial>>,
) {
    for (entity, children) in aircraft_query.iter() {
        let mut found_any = false;
        for child in children.iter() {
            for descendant in std::iter::once(child).chain(children_query.iter_descendants(child)) {
                if let Ok(mat_handle) = material_query.get(descendant) {
                    if let Some(mat) = materials.get_mut(&mat_handle.0) {
                        mat.unlit = true;
                        found_any = true;
                    }
                }
            }
        }
        if found_any {
            commands.entity(entity).insert(MaterialsUnlit);
        }
    }
}

/// Update aircraft labels with current data.
/// Display priority: callsign > tail number (registration) > ICAO hex.
pub fn update_aircraft_label_text(
    aircraft_query: Query<&Aircraft>,
    mut label_query: Query<(&AircraftLabel, &mut Text2d)>,
    type_db: Option<Res<crate::aircraft::AircraftTypeDatabase>>,
) {
    for (label, mut text) in label_query.iter_mut() {
        if let Ok(aircraft) = aircraft_query.get(label.aircraft_entity) {
            let registration = type_db
                .as_ref()
                .and_then(|db| db.lookup(&aircraft.icao))
                .and_then(|info| info.registration);

            let display_name = aircraft
                .callsign
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or(registration.as_deref())
                .unwrap_or(&aircraft.icao);

            let alt_display = aircraft
                .altitude
                .map(|a| format!("{} ft", a))
                .unwrap_or_default();
            **text = format!("{}\n{}", display_name, alt_display);
        }
    }
}
