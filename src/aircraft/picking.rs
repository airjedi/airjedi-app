use bevy::prelude::*;
use bevy::picking::mesh_picking::ray_cast::MeshRayCast;

use super::detail_panel::CameraFollowState;
use super::list_panel::AircraftListState;
use crate::Aircraft;

/// Marker component added to aircraft entities when selected via click.
#[derive(Component)]
pub struct SelectionOutline;

/// Marker component added to aircraft entities when hovered.
#[derive(Component)]
pub struct HoverOutline;

/// Pre-built materials for selection and hover outlines.
#[derive(Resource)]
pub struct OutlineMaterials {
    /// Bright cyan material for selected aircraft.
    pub selected: Handle<StandardMaterial>,
    /// Dimmer cyan material for hovered aircraft.
    pub hover: Handle<StandardMaterial>,
    /// Original materials stashed before outline swap, keyed by entity.
    pub originals: Vec<(Entity, Handle<StandardMaterial>)>,
}

/// Startup system that creates the outline materials.
pub fn setup_outline_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let selected = materials.add(StandardMaterial {
        base_color: Color::srgb(0.0, 1.0, 1.0),
        emissive: LinearRgba::new(0.0, 4.0, 4.0, 1.0),
        unlit: false,
        alpha_mode: AlphaMode::Opaque,
        ..default()
    });

    let hover = materials.add(StandardMaterial {
        base_color: Color::srgb(0.0, 0.6, 0.6),
        emissive: LinearRgba::new(0.0, 1.5, 1.5, 1.0),
        unlit: false,
        alpha_mode: AlphaMode::Opaque,
        ..default()
    });

    commands.insert_resource(OutlineMaterials {
        selected,
        hover,
        originals: Vec::new(),
    });
}

/// Observer triggered when an aircraft entity is clicked.
/// Since Pointer events auto-propagate up the hierarchy, clicks on child
/// mesh entities bubble up to the aircraft entity where this observer lives.
pub fn on_aircraft_click(
    event: On<Pointer<Click>>,
    aircraft_query: Query<&Aircraft>,
    mut list_state: ResMut<AircraftListState>,
    mut follow_state: ResMut<CameraFollowState>,
) {
    let aircraft_entity = event.observer();

    if let Ok(aircraft) = aircraft_query.get(aircraft_entity) {
        info!("Aircraft clicked: {}", aircraft.icao);
        list_state.selected_icao = Some(aircraft.icao.clone());
        follow_state.following_icao = Some(aircraft.icao.clone());
    }
}

/// Observer triggered when the pointer enters an aircraft entity.
pub fn on_aircraft_hover(
    event: On<Pointer<Over>>,
    mut commands: Commands,
    hover_query: Query<(), With<HoverOutline>>,
) {
    let aircraft_entity = event.observer();

    if hover_query.get(aircraft_entity).is_err() {
        if let Ok(mut ec) = commands.get_entity(aircraft_entity) {
            ec.try_insert(HoverOutline);
        }
    }
}

/// Observer triggered when the pointer leaves an aircraft entity.
pub fn on_aircraft_out(
    event: On<Pointer<Out>>,
    mut commands: Commands,
    hover_query: Query<(), With<HoverOutline>>,
) {
    let aircraft_entity = event.observer();

    if hover_query.get(aircraft_entity).is_ok() {
        if let Ok(mut ec) = commands.get_entity(aircraft_entity) {
            ec.try_remove::<HoverOutline>();
        }
    }
}

/// System that clears selection when ESC is pressed.
pub fn deselect_on_escape(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut list_state: ResMut<AircraftListState>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        if list_state.selected_icao.is_some() {
            list_state.selected_icao = None;
        }
    }
}

/// System that lerps the 3D camera orbit center toward the followed aircraft.
/// When chase is active, also lerps orbit params toward chase targets.
pub fn follow_aircraft_3d(
    mut view3d_state: ResMut<crate::view3d::View3DState>,
    follow_state: Res<CameraFollowState>,
    aircraft_query: Query<&Aircraft>,
    time: Res<Time>,
    tile_settings: Res<bevy_slippy_tiles::SlippyTilesSettings>,
    map_state: Res<crate::MapState>,
) {
    use crate::view3d::{ViewMode, TransitionState};

    // Only follow in steady-state 3D (not during transitions)
    if !matches!(view3d_state.mode, ViewMode::Perspective3D)
        || !matches!(view3d_state.transition, TransitionState::Idle)
    {
        return;
    }

    let Some(ref following_icao) = follow_state.following_icao else {
        // Just stopped following — deactivate chase and restore orbit params
        if view3d_state.chase_active {
            if !view3d_state.chase_orbit_override {
                // Restore pre-chase orbit params only if user hasn't orbited
                view3d_state.camera_pitch = view3d_state.pre_chase_pitch;
                view3d_state.camera_yaw = view3d_state.pre_chase_yaw;
                view3d_state.camera_altitude = view3d_state.pre_chase_altitude;
            }
            view3d_state.chase_active = false;
            view3d_state.chase_transition = 0.0;
            view3d_state.chase_orbit_override = false;
        }
        view3d_state.follow_altitude_ft = None;
        return;
    };

    let Some(aircraft) = aircraft_query.iter().find(|a| a.icao == *following_icao) else {
        view3d_state.follow_altitude_ft = None;
        return;
    };

    // Activate chase on first frame of following
    if !view3d_state.chase_active {
        view3d_state.pre_chase_pitch = view3d_state.camera_pitch;
        view3d_state.pre_chase_yaw = view3d_state.camera_yaw;
        view3d_state.pre_chase_altitude = view3d_state.camera_altitude;
        view3d_state.chase_active = true;
        view3d_state.chase_transition = 0.0;
    }

    let render_zoom = view3d_state.effective_zoom(map_state.zoom_level);
    let converter = crate::geo::CoordinateConverter::new(&tile_settings, render_zoom);
    let target_pos = converter.latlon_to_world(aircraft.latitude, aircraft.longitude);

    // Lerp map center toward aircraft position
    let pos_lerp_speed = 3.0;
    let t_pos = (pos_lerp_speed * time.delta_secs()).min(1.0);
    view3d_state.saved_2d_center.x += (target_pos.x - view3d_state.saved_2d_center.x) * t_pos;
    view3d_state.saved_2d_center.y += (target_pos.y - view3d_state.saved_2d_center.y) * t_pos;

    // Track the followed aircraft's altitude for the orbit center
    view3d_state.follow_altitude_ft = aircraft.altitude;

    // Advance chase transition progress
    view3d_state.chase_transition = (view3d_state.chase_transition
        + time.delta_secs() / crate::view3d::CHASE_TRANSITION_DURATION)
        .min(1.0);

    // Only lerp yaw/pitch/altitude when user hasn't overridden with orbit input
    if !view3d_state.chase_orbit_override {
        let chase_lerp_speed = 2.0;
        let t_chase = (chase_lerp_speed * time.delta_secs()).min(1.0);

        // Target yaw: behind the aircraft.
        // Orbit yaw=0 places camera south (+Z) looking north (-Z).
        // Aircraft heading=0 means flying north, so camera should be south = yaw 0.
        // Therefore chase yaw = aircraft heading directly.
        let target_yaw = if let Some(heading) = aircraft.heading {
            ((heading % 360.0) + 360.0) % 360.0
        } else {
            view3d_state.camera_yaw
        };

        // Shortest-path yaw lerp (handle 0/360 wrap)
        let mut yaw_diff = target_yaw - view3d_state.camera_yaw;
        if yaw_diff > 180.0 { yaw_diff -= 360.0; }
        if yaw_diff < -180.0 { yaw_diff += 360.0; }
        view3d_state.camera_yaw += yaw_diff * t_chase;
        if view3d_state.camera_yaw < 0.0 { view3d_state.camera_yaw += 360.0; }
        if view3d_state.camera_yaw >= 360.0 { view3d_state.camera_yaw -= 360.0; }

        // Target pitch
        let target_pitch = crate::view3d::CHASE_PITCH;
        view3d_state.camera_pitch += (target_pitch - view3d_state.camera_pitch) * t_chase;

        // Target altitude: aircraft altitude + offset above
        let target_altitude = aircraft.altitude.unwrap_or(0) as f32
            + crate::view3d::CHASE_OFFSET_ABOVE_FT;
        view3d_state.camera_altitude += (target_altitude - view3d_state.camera_altitude) * t_chase;
    }
}

/// System that clears selection when the selected aircraft no longer exists.
pub fn clear_stale_selection(
    mut list_state: ResMut<AircraftListState>,
    mut follow_state: ResMut<CameraFollowState>,
    aircraft_query: Query<&Aircraft>,
) {
    let Some(ref selected_icao) = list_state.selected_icao else {
        return;
    };

    let still_exists = aircraft_query.iter().any(|a| a.icao == *selected_icao);
    if !still_exists {
        info!("Selected aircraft {} no longer exists, clearing selection", selected_icao);
        list_state.selected_icao = None;
        follow_state.following_icao = None;
    }
}

/// System that keeps SelectionOutline marker in sync with AircraftListState.
/// Runs every frame but only does work when selected_icao changes.
pub fn manage_selection_outline(
    mut commands: Commands,
    list_state: Res<AircraftListState>,
    aircraft_query: Query<(Entity, &Aircraft)>,
    selected_query: Query<Entity, With<SelectionOutline>>,
) {
    if !list_state.is_changed() {
        return;
    }

    // Remove SelectionOutline from all currently selected entities
    for entity in selected_query.iter() {
        if let Ok(mut ec) = commands.get_entity(entity) {
            ec.try_remove::<SelectionOutline>();
        }
    }

    // Add SelectionOutline to the newly selected aircraft
    if let Some(ref selected_icao) = list_state.selected_icao {
        for (entity, aircraft) in aircraft_query.iter() {
            if aircraft.icao == *selected_icao {
                if let Ok(mut ec) = commands.get_entity(entity) {
                    ec.try_insert(SelectionOutline);
                }
                break;
            }
        }
    }
}

/// System that swaps materials on child meshes of aircraft with SelectionOutline or HoverOutline.
/// SelectionOutline takes priority over HoverOutline.
pub fn swap_outline_materials(
    mut outline_mats: ResMut<OutlineMaterials>,
    selected_query: Query<&Children, With<SelectionOutline>>,
    hover_query: Query<&Children, (With<HoverOutline>, Without<SelectionOutline>)>,
    normal_query: Query<
        (Entity, &Children),
        (With<Aircraft>, Without<SelectionOutline>, Without<HoverOutline>),
    >,
    children_query: Query<&Children>,
    mesh_query: Query<&MeshMaterial3d<StandardMaterial>>,
    mut commands: Commands,
) {
    let selected_mat = outline_mats.selected.clone();
    let hover_mat = outline_mats.hover.clone();

    // Apply selected material to selected aircraft children
    for children in selected_query.iter() {
        apply_material_to_hierarchy(
            children,
            &children_query,
            &mesh_query,
            &mut outline_mats.originals,
            &selected_mat,
            &mut commands,
        );
    }

    // Apply hover material to hovered (non-selected) aircraft children
    for children in hover_query.iter() {
        apply_material_to_hierarchy(
            children,
            &children_query,
            &mesh_query,
            &mut outline_mats.originals,
            &hover_mat,
            &mut commands,
        );
    }

    // Restore original materials for normal (non-selected, non-hovered) aircraft
    for (entity, children) in normal_query.iter() {
        restore_materials_in_hierarchy(
            entity,
            children,
            &children_query,
            &mesh_query,
            &mut outline_mats.originals,
            &mut commands,
        );
    }
}

/// Recursively apply outline material to all meshes in the hierarchy.
fn apply_material_to_hierarchy(
    children: &Children,
    children_query: &Query<&Children>,
    mesh_query: &Query<&MeshMaterial3d<StandardMaterial>>,
    originals: &mut Vec<(Entity, Handle<StandardMaterial>)>,
    outline_mat: &Handle<StandardMaterial>,
    commands: &mut Commands,
) {
    for child in children.iter() {
        if let Ok(mat_handle) = mesh_query.get(child) {
            // Stash the original material if not already stashed
            if !originals.iter().any(|(e, _)| *e == child) {
                originals.push((child, mat_handle.0.clone()));
            }
            // Replace with outline material
            if mat_handle.0 != *outline_mat {
                commands
                    .entity(child)
                    .insert(MeshMaterial3d(outline_mat.clone()));
            }
        }
        if let Ok(grandchildren) = children_query.get(child) {
            apply_material_to_hierarchy(
                grandchildren,
                children_query,
                mesh_query,
                originals,
                outline_mat,
                commands,
            );
        }
    }
}

// =============================================================================
// Manual 3D Picking (bypasses broken mesh picking backend)
// =============================================================================

/// Find the aircraft ancestor of an entity by walking up the ChildOf hierarchy.
fn find_aircraft_ancestor(
    entity: Entity,
    aircraft_query: &Query<(Entity, &Aircraft)>,
    parent_query: &Query<&ChildOf>,
) -> Option<Entity> {
    // Check the entity itself
    if aircraft_query.get(entity).is_ok() {
        return Some(entity);
    }
    let mut current = entity;
    for _ in 0..10 {
        if let Ok(parent) = parent_query.get(current) {
            let pe = parent.parent();
            if aircraft_query.get(pe).is_ok() {
                return Some(pe);
            }
            current = pe;
        } else {
            break;
        }
    }
    None
}

/// Raycast picking for 3D mode. The standard mesh picking backend uses
/// ViewVisibility to filter entities, which doesn't work with our dual-camera
/// architecture (Camera3d with Atmosphere post-processing + Camera2d overlay).
/// This system uses MeshRayCast directly from Camera3d, giving us full control
/// over the ray source and entity filtering.
pub fn pick_aircraft_3d(
    mouse_button: Res<ButtonInput<MouseButton>>,
    window_query: Query<&Window>,
    camera_3d: Query<(&Camera, &GlobalTransform), With<crate::AircraftCamera>>,
    mut raycast: MeshRayCast,
    aircraft_query: Query<(Entity, &Aircraft)>,
    parent_query: Query<&ChildOf>,
    mut list_state: ResMut<AircraftListState>,
    mut follow_state: ResMut<CameraFollowState>,
    view3d_state: Res<crate::view3d::View3DState>,
    mut commands: Commands,
    hover_query: Query<Entity, With<HoverOutline>>,
) {
    if !view3d_state.is_3d_active() || view3d_state.is_transitioning() {
        return;
    }

    let Ok(window) = window_query.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((camera, cam_gtf)) = camera_3d.single() else { return };
    let Ok(ray) = camera.viewport_to_world(cam_gtf, cursor_pos) else { return };

    let hits = raycast.cast_ray(ray, &default());

    // Find the closest aircraft hit
    let aircraft_hit = hits.iter().find_map(|(entity, _hit)| {
        find_aircraft_ancestor(*entity, &aircraft_query, &parent_query)
    });

    // Handle hover: add/remove HoverOutline based on what's under cursor
    if let Some(ac_entity) = aircraft_hit {
        if hover_query.get(ac_entity).is_err() {
            // Remove hover from all others first
            for entity in hover_query.iter() {
                if let Ok(mut ec) = commands.get_entity(entity) {
                    ec.try_remove::<HoverOutline>();
                }
            }
            if let Ok(mut ec) = commands.get_entity(ac_entity) {
                ec.try_insert(HoverOutline);
            }
        }
    } else {
        // No aircraft under cursor — remove all hovers
        for entity in hover_query.iter() {
            if let Ok(mut ec) = commands.get_entity(entity) {
                ec.try_remove::<HoverOutline>();
            }
        }
    }

    // Handle click
    if !mouse_button.just_pressed(MouseButton::Left) {
        return;
    }

    // Don't select if this was a drag (handled by drag dead zone in handle_3d_camera_controls)
    if view3d_state.drag_active {
        return;
    }

    if let Some(ac_entity) = aircraft_hit {
        if let Ok((_, aircraft)) = aircraft_query.get(ac_entity) {
            info!("3D pick: Aircraft clicked: {}", aircraft.icao);
            list_state.selected_icao = Some(aircraft.icao.clone());
            follow_state.following_icao = Some(aircraft.icao.clone());
        }
    } else {
        // Clicked empty space — deselect
        if list_state.selected_icao.is_some() {
            info!("3D pick: Ground clicked, clearing selection");
            list_state.selected_icao = None;
            follow_state.following_icao = None;
        }
    }
}

/// Recursively restore original materials for all meshes in the hierarchy.
fn restore_materials_in_hierarchy(
    aircraft_entity: Entity,
    children: &Children,
    children_query: &Query<&Children>,
    mesh_query: &Query<&MeshMaterial3d<StandardMaterial>>,
    originals: &mut Vec<(Entity, Handle<StandardMaterial>)>,
    commands: &mut Commands,
) {
    for child in children.iter() {
        if mesh_query.get(child).is_ok() {
            // Find and restore the original material
            if let Some(pos) = originals.iter().position(|(e, _)| *e == child) {
                let (_, original_mat) = originals.remove(pos);
                commands
                    .entity(child)
                    .insert(MeshMaterial3d(original_mat));
            }
        }
        if let Ok(grandchildren) = children_query.get(child) {
            restore_materials_in_hierarchy(
                aircraft_entity,
                grandchildren,
                children_query,
                mesh_query,
                originals,
                commands,
            );
        }
    }
}
