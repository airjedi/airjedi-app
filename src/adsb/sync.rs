use bevy::prelude::*;
use std::collections::HashMap;

use crate::{Aircraft, AircraftLabel};

/// Type codes that should use the B737 model
const B737_TYPES: &[&str] = &[
    "B731", "B732", "B733", "B734", "B735", "B736", "B737", "B738", "B739", "B37M", "B38M", "B39M",
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
