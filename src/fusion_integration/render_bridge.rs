use crate::adsb::sync::AircraftModelRegistry;
use crate::aircraft::components::{Aircraft, AircraftLabel, FusionTrackLink};
use crate::aircraft::picking::{on_aircraft_click, on_aircraft_hover, on_aircraft_out};
use crate::aircraft::{InterpolationState, TrailHistory};
use crate::constants;
use crate::geo;
use crate::map::MapState;
use crate::theme::AppTheme;
use crate::view3d;
use crate::RenderCategory;
use airjedi_fusion::types::{IdentifierType, TargetCategory};
use airjedi_fusion::{TargetClassification, Track, TrackQuality, TrackStatus, TrackerState};
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy_slippy_tiles::SlippyTilesSettings;

pub fn sync_tracks_to_visuals(
    mut commands: Commands,
    fusion_tracks: Query<
        (
            Entity,
            &Track,
            &TrackerState,
            &TrackQuality,
            &TargetClassification,
        ),
        Changed<TrackerState>,
    >,
    mut visuals: Query<(
        &FusionTrackLink,
        &mut Aircraft,
        Option<&mut InterpolationState>,
    )>,
    visual_lookup: Query<(Entity, &FusionTrackLink)>,
    label_query: Query<(Entity, &AircraftLabel)>,
    model_registry: Option<Res<AircraftModelRegistry>>,
    type_db: Option<Res<crate::aircraft::AircraftTypeDatabase>>,
    theme: Res<AppTheme>,
    time: Res<Time<Real>>,
    map_state: Res<MapState>,
    tile_settings: Res<SlippyTilesSettings>,
    view3d_state: Res<view3d::View3DState>,
) {
    let Some(model_registry) = model_registry else {
        return;
    };

    for (track_entity, track, tracker, quality, classification) in &fusion_tracks {
        let (lat, lon, alt_m) = tracker.position_geodetic();
        let alt_ft = (alt_m / 0.3048) as i32;

        let vel_ecef = tracker.velocity_ecef();
        let speed_mps = (vel_ecef[0].powi(2) + vel_ecef[1].powi(2) + vel_ecef[2].powi(2)).sqrt();
        let speed_kts = speed_mps / 0.514444;

        let heading = compute_heading_from_ecef(lat, lon, &vel_ecef, speed_mps);

        let existing_visual = visual_lookup
            .iter()
            .find(|(_, link)| link.track_entity == track_entity);

        if quality.status == TrackStatus::Lost {
            if let Some((visual_entity, _)) = existing_visual {
                for (label_entity, label) in label_query.iter() {
                    if label.aircraft_entity == visual_entity {
                        commands.entity(label_entity).despawn();
                        break;
                    }
                }
                commands.entity(visual_entity).despawn();
            }
            continue;
        }

        if let Some((visual_entity, _)) = existing_visual {
            if let Ok((_, mut aircraft, interp_opt)) = visuals.get_mut(visual_entity) {
                let position_changed = (lat - aircraft.latitude).abs() > f64::EPSILON
                    || (lon - aircraft.longitude).abs() > f64::EPSILON;

                aircraft.latitude = lat;
                aircraft.longitude = lon;
                aircraft.altitude = Some(alt_ft);
                aircraft.heading = heading.map(|h| h as f32);
                aircraft.velocity = Some(speed_kts);
                aircraft.vertical_rate = compute_vertical_rate(&vel_ecef);
                aircraft.is_on_ground = Some(track.is_on_ground);
                aircraft.last_seen = track.last_update;

                if aircraft.callsign.is_none() {
                    for cid in &track.cooperative_ids {
                        if cid.id_type == IdentifierType::Callsign {
                            aircraft.callsign = Some(cid.id.clone());
                            break;
                        }
                    }
                }

                if position_changed {
                    if let Some(mut interp) = interp_opt {
                        crate::aircraft::interpolation::update_interpolation_on_adsb(
                            &mut interp,
                            lat,
                            lon,
                            Some(alt_ft),
                            heading.map(|h| h as f32),
                            Some(speed_kts),
                            compute_vertical_rate(&vel_ecef),
                            None,
                            time.elapsed_secs_f64(),
                        );
                    }
                }
            }
        } else if is_air_target(classification.category) {
            let icao = track
                .cooperative_ids
                .iter()
                .find(|id| id.id_type == IdentifierType::Icao)
                .map(|id| id.id.clone())
                .unwrap_or_else(|| format!("TRK-{}", &track.id.0.to_string()[..8]));

            let callsign = track
                .cooperative_ids
                .iter()
                .find(|id| id.id_type == IdentifierType::Callsign)
                .map(|id| id.id.clone());

            let type_info = type_db.as_ref().and_then(|db| db.lookup(&icao));

            let type_code = type_info.as_ref().and_then(|i| i.type_code.clone());
            let registration = type_info.as_ref().and_then(|i| i.registration.clone());

            let model_handle = model_registry.get_model(type_code.as_deref());
            let correction = model_registry.get_correction(type_code.as_deref());

            let display_name = callsign
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or(registration.as_deref())
                .unwrap_or(&icao);
            let aircraft_name = display_name;

            let zoom = view3d_state.effective_zoom(map_state.zoom_level);
            let converter = geo::CoordinateConverter::new(&tile_settings, zoom);
            let pos = converter.latlon_to_world(lat, lon);

            let mut entity_commands = commands.spawn((
                Name::new(format!("Aircraft: {}", aircraft_name)),
                SceneRoot(model_handle),
                Transform::from_xyz(pos.x, pos.y, constants::AIRCRAFT_Z_LAYER),
                Pickable::default(),
                Aircraft {
                    icao: icao.clone(),
                    callsign: callsign.clone(),
                    latitude: lat,
                    longitude: lon,
                    altitude: Some(alt_ft),
                    heading: heading.map(|h| h as f32),
                    velocity: Some(speed_kts),
                    vertical_rate: compute_vertical_rate(&vel_ecef),
                    squawk: None,
                    is_on_ground: Some(track.is_on_ground),
                    alert: None,
                    emergency: None,
                    spi: None,
                    last_seen: track.last_update,
                },
                FusionTrackLink {
                    track_entity,
                    track_id: track.id.clone(),
                },
                TrailHistory::default(),
                InterpolationState::new(
                    lat,
                    lon,
                    Some(alt_ft),
                    heading.map(|h| h as f32),
                    Some(speed_kts),
                    compute_vertical_rate(&vel_ecef),
                    None,
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

            let label_text = format!("{}\n{}", display_name, format!("{} ft", alt_ft),);

            commands.spawn((
                Name::new(format!("Label: {}", aircraft_name)),
                Text2d::new(label_text),
                TextFont {
                    font_size: constants::BASE_FONT_SIZE,
                    ..default()
                },
                TextColor(theme.text_primary()),
                Transform::from_xyz(pos.x, pos.y, constants::LABEL_Z_LAYER),
                Visibility::Hidden,
                AircraftLabel { aircraft_entity },
                RenderLayers::layer(RenderCategory::LABELS),
            ));
        }
    }
}

fn is_air_target(category: TargetCategory) -> bool {
    matches!(
        category,
        TargetCategory::FixedWing
            | TargetCategory::RotaryWing
            | TargetCategory::Drone
            | TargetCategory::Balloon
            | TargetCategory::Unknown
    )
}

fn compute_heading_from_ecef(
    lat_deg: f64,
    lon_deg: f64,
    vel_ecef: &[f64; 3],
    speed_mps: f64,
) -> Option<f64> {
    if speed_mps < 1.0 {
        return None;
    }

    let lat_rad = lat_deg.to_radians();
    let lon_rad = lon_deg.to_radians();

    let sin_lat = lat_rad.sin();
    let cos_lat = lat_rad.cos();
    let sin_lon = lon_rad.sin();
    let cos_lon = lon_rad.cos();

    // ECEF to ENU rotation
    let ve = -sin_lon * vel_ecef[0] + cos_lon * vel_ecef[1];
    let vn =
        -sin_lat * cos_lon * vel_ecef[0] - sin_lat * sin_lon * vel_ecef[1] + cos_lat * vel_ecef[2];

    let heading = ve.atan2(vn).to_degrees();
    Some(((heading % 360.0) + 360.0) % 360.0)
}

fn compute_vertical_rate(vel_ecef: &[f64; 3]) -> Option<i32> {
    let vu_approx = vel_ecef[2];
    let vr_fpm = vu_approx / 0.00508;
    if vr_fpm.abs() > 0.1 {
        Some(vr_fpm as i32)
    } else {
        None
    }
}

/// Despawn visual entities whose fusion track entity no longer exists.
/// This catches cases where the track was cleaned up (Lost -> despawned)
/// but the render bridge didn't process it because TrackerState didn't change.
pub fn cleanup_orphaned_visuals(
    mut commands: Commands,
    visuals: Query<(Entity, &FusionTrackLink)>,
    fusion_tracks: Query<Entity, With<Track>>,
    label_query: Query<(Entity, &AircraftLabel)>,
) {
    for (visual_entity, link) in &visuals {
        if fusion_tracks.get(link.track_entity).is_err() {
            for (label_entity, label) in label_query.iter() {
                if label.aircraft_entity == visual_entity {
                    commands.entity(label_entity).despawn();
                    break;
                }
            }
            commands.entity(visual_entity).despawn();
        }
    }
}
