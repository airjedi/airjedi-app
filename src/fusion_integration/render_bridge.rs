use crate::adsb::connection::FeedConnectionManager;
use crate::adsb::sync::AircraftModelRegistry;
use crate::aircraft::components::{Aircraft, AircraftLabel, FusionDiagnostics, FusionTrackLink};
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
use crate::tiles::LocalOrigin;

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
        Option<&mut FusionDiagnostics>,
    )>,
    visual_lookup: Query<(Entity, &FusionTrackLink)>,
    label_query: Query<(Entity, &AircraftLabel)>,
    model_registry: Option<Res<AircraftModelRegistry>>,
    type_db: Option<Res<crate::aircraft::AircraftTypeDatabase>>,
    feed_mgr: Option<Res<FeedConnectionManager>>,
    theme: Res<AppTheme>,
    time: Res<Time<Real>>,
    map_state: Res<MapState>,
    local_origin: Res<LocalOrigin>,
    view3d_state: Res<view3d::View3DState>,
) {
    let Some(model_registry) = model_registry else {
        return;
    };

    let raw_aircraft: Option<Vec<adsb_client::Aircraft>> = feed_mgr.as_ref().map(|mgr| {
        mgr.all_aircraft().into_iter().map(|(_, ac)| ac).collect()
    });

    for (track_entity, track, tracker, quality, classification) in &fusion_tracks {
        let (lat, lon, alt_m) = tracker.position_geodetic();
        let filter_alt_ft = (alt_m / 0.3048) as i32;

        let vel_ecef = tracker.velocity_ecef();
        let speed_mps = (vel_ecef[0].powi(2) + vel_ecef[1].powi(2) + vel_ecef[2].powi(2)).sqrt();
        let speed_kts = speed_mps / 0.514444;

        let heading = compute_heading_from_ecef(lat, lon, &vel_ecef, speed_mps);

        let track_icao = track
            .cooperative_ids
            .iter()
            .find(|id| id.id_type == IdentifierType::Icao)
            .and_then(|id| adsb_client::Icao::from_hex(&id.id));
        let raw_ac = track_icao.and_then(|icao| {
            raw_aircraft
                .as_ref()
                .and_then(|list| list.iter().find(|ac| ac.icao == icao))
        });
        let alt_ft = raw_ac
            .and_then(|ac| ac.altitude)
            .unwrap_or(filter_alt_ft);
        let vrate = raw_ac
            .and_then(|ac| ac.vertical_rate)
            .or_else(|| compute_vertical_rate(&vel_ecef, lat, lon));
        let heading = raw_ac
            .and_then(|ac| ac.track)
            .or(heading);
        let speed_kts = raw_ac
            .and_then(|ac| ac.velocity)
            .unwrap_or(speed_kts);
        let is_coasting = quality.status == TrackStatus::Coasting;

        // During coasting, prefer the filter's predicted position (which propagates forward
        // each frame) over the stale raw position to show smooth dead reckoning. When
        // confirmed, raw data takes priority as it is the freshest observed position.
        let lat = if is_coasting {
            lat
        } else {
            raw_ac.and_then(|ac| ac.latitude).unwrap_or(lat)
        };
        let lon = if is_coasting {
            lon
        } else {
            raw_ac.and_then(|ac| ac.longitude).unwrap_or(lon)
        };
        let squawk = raw_ac.and_then(|ac| ac.squawk.clone());
        let is_on_ground = raw_ac
            .and_then(|ac| ac.is_on_ground)
            .or(Some(track.is_on_ground));
        let alert = raw_ac.and_then(|ac| ac.alert);
        let emergency = raw_ac.and_then(|ac| ac.emergency);
        let spi = raw_ac.and_then(|ac| ac.spi);
        let roll_angle = raw_ac.and_then(|ac| ac.roll_angle.map(|v| v as f32));
        let track_angle_rate = raw_ac.and_then(|ac| ac.track_angle_rate.map(|v| v as f32));

        let existing_visual = visual_lookup
            .iter()
            .find(|(_, link)| link.track_entity == track_entity);

        // Lost tracks have been gone too long to be meaningfully displayed; coasting tracks
        // still have a valid predicted position and should continue to update.
        if matches!(quality.status, TrackStatus::Lost) {
            continue;
        }

        if let Some((visual_entity, _)) = existing_visual {
            if let Ok((_, mut aircraft, interp_opt, diag_opt)) = visuals.get_mut(visual_entity) {
                let position_changed = (lat - aircraft.latitude).abs() > f64::EPSILON
                    || (lon - aircraft.longitude).abs() > f64::EPSILON;

                aircraft.latitude = lat;
                aircraft.longitude = lon;
                aircraft.altitude = Some(alt_ft);
                aircraft.heading = heading.map(|h| h as f32);
                aircraft.velocity = Some(speed_kts);
                aircraft.vertical_rate = vrate;
                aircraft.is_on_ground = is_on_ground;
                aircraft.alert = alert;
                aircraft.emergency = emergency;
                aircraft.spi = spi;
                aircraft.roll_angle = roll_angle;
                aircraft.track_angle_rate = track_angle_rate;
                aircraft.last_seen = track.last_update;
                if squawk.is_some() {
                    aircraft.squawk = squawk.clone();
                }

                if let Some(ac) = raw_ac {
                    if let Some(ref cs) = ac.callsign {
                        if !cs.trim().is_empty() {
                            aircraft.callsign = Some(cs.clone());
                        }
                    }
                } else if aircraft.callsign.is_none() {
                    for cid in &track.cooperative_ids {
                        if cid.id_type == IdentifierType::Callsign {
                            aircraft.callsign = Some(cid.id.clone());
                            break;
                        }
                    }
                }

                if let Some(mut diag) = diag_opt {
                    update_diagnostics(&mut diag, tracker, quality);
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
                            vrate,
                            None,
                            time.elapsed_secs_f64(),
                        );
                    }
                }
            }
        } else if is_air_target(classification.category) && !is_coasting {
            // Don't spawn new visual entities for coasting tracks. A coasting track
            // with no existing visual means its visual was cleaned up because
            // aircraft.last_seen exceeded the timeout. Spawning a new one would set
            // aircraft.last_seen = track.last_update (old), causing cleanup_orphaned_visuals
            // to immediately despawn it next frame, creating a continuous spawn-despawn cycle.
            // When the track reacquires (signals return), it will be re-confirmed and a
            // fresh visual will be spawned then.

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

            let converter = geo::CoordinateConverter::new(&local_origin);
            let pos = converter.latlon_to_world(lat, lon);

            let mut entity_commands = commands.spawn((
                Name::new(format!("Aircraft: {}", aircraft_name)),
                WorldAssetRoot(model_handle),
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
                    vertical_rate: vrate,
                    roll_angle,
                    track_angle_rate,
                    squawk,
                    is_on_ground,
                    alert,
                    emergency,
                    spi,
                    last_seen: track.last_update,
                },
                FusionTrackLink {
                    track_entity,
                    track_id: track.id.clone(),
                },
                make_diagnostics(tracker, quality),
                TrailHistory::default(),
                InterpolationState::new(
                    lat,
                    lon,
                    Some(alt_ft),
                    heading.map(|h| h as f32),
                    Some(speed_kts),
                    vrate,
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
                    font_size: FontSize::Px(constants::BASE_FONT_SIZE),
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

fn filter_type_label(tracker: &TrackerState) -> &'static str {
    if tracker.mode_info().is_some() {
        "IMM"
    } else {
        match tracker.state_type {
            airjedi_fusion::StateVectorType::Surface4Dof => "Surface",
            _ => "EKF",
        }
    }
}

fn make_diagnostics(tracker: &TrackerState, quality: &TrackQuality) -> FusionDiagnostics {
    let mode = tracker.mode_info();
    FusionDiagnostics {
        filter_type: filter_type_label(tracker),
        mode_probabilities: mode.as_ref().map(|m| m.probabilities.clone()),
        dominant_mode: mode.as_ref().map(|m| m.dominant_mode),
        track_status: Some(quality.status),
        observation_count: quality.observation_count,
    }
}

fn update_diagnostics(
    diag: &mut FusionDiagnostics,
    tracker: &TrackerState,
    quality: &TrackQuality,
) {
    let mode = tracker.mode_info();
    diag.filter_type = filter_type_label(tracker);
    diag.mode_probabilities = mode.as_ref().map(|m| m.probabilities.clone());
    diag.dominant_mode = mode.as_ref().map(|m| m.dominant_mode);
    diag.track_status = Some(quality.status);
    diag.observation_count = quality.observation_count;
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

fn compute_vertical_rate(vel_ecef: &[f64; 3], lat_deg: f64, lon_deg: f64) -> Option<i32> {
    let lat_rad = lat_deg.to_radians();
    let lon_rad = lon_deg.to_radians();

    let sin_lat = lat_rad.sin();
    let cos_lat = lat_rad.cos();
    let sin_lon = lon_rad.sin();
    let cos_lon = lon_rad.cos();

    let vu = cos_lat * cos_lon * vel_ecef[0]
        + cos_lat * sin_lon * vel_ecef[1]
        + sin_lat * vel_ecef[2];

    let vr_fpm = vu / 0.00508;
    if vr_fpm.abs() > 0.1 {
        Some(vr_fpm as i32)
    } else {
        None
    }
}

/// Refresh visual aircraft last_seen directly from the feed tracker data.
/// This keeps the visual entity "alive" (undimmed, not timed out) as long
/// as the adsb-client tracker is still receiving messages for the aircraft,
/// even when the fusion pipeline hasn't pushed a state change.
pub fn refresh_aircraft_last_seen(
    feed_mgr: Option<Res<FeedConnectionManager>>,
    mut visuals: Query<&mut Aircraft>,
) {
    let Some(mgr) = feed_mgr else {
        return;
    };

    let now = chrono::Utc::now();

    for conn in mgr.connections.values() {
        let aircraft_list = match conn.data.aircraft.try_lock() {
            Ok(list) => list,
            Err(_) => continue,
        };

        for raw_ac in aircraft_list.iter() {
            for mut visual in visuals.iter_mut() {
                if visual.icao == raw_ac.icao.to_string() && raw_ac.last_seen > visual.last_seen {
                    visual.last_seen = raw_ac.last_seen;
                }
            }
        }
    }
}

/// Despawn visual entities whose fusion track entity no longer exists,
/// or whose last_seen age exceeds the staleness timeout.
pub fn cleanup_orphaned_visuals(
    mut commands: Commands,
    visuals: Query<(Entity, &FusionTrackLink, &Aircraft)>,
    fusion_tracks: Query<Entity, With<Track>>,
    label_query: Query<(Entity, &AircraftLabel)>,
) {
    let now = chrono::Utc::now();

    for (visual_entity, link, aircraft) in &visuals {
        let orphaned = fusion_tracks.get(link.track_entity).is_err();
        let age_secs = (now - aircraft.last_seen).num_seconds();
        let timed_out = age_secs > crate::constants::ADSB_AIRCRAFT_TIMEOUT_SECS;

        if orphaned || timed_out {
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
