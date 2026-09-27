use crate::adsb::connection::FeedConnectionManager;
use crate::adsb::enrichment::{EnrichmentConnectionManager, PositionSource};
use crate::adsb::sync::AircraftModelRegistry;
use crate::aircraft::components::{Aircraft, FusionDiagnostics, FusionTrackLink};
use crate::aircraft::picking::{on_aircraft_click, on_aircraft_hover, on_aircraft_out};
use crate::aircraft::{InterpolationState, TrailHistory};
use crate::constants;
use crate::geo;
use crate::map::MapState;
use crate::tiles::LocalOrigin;
use crate::view3d;
use airjedi_fusion::types::{IdentifierType, TargetCategory};
use airjedi_fusion::{
    derive_display_track, filter_type_label, raw_observation_hint_for, TargetClassification,
    TimelineStore, Track, TrackQuality, TrackStatus, TrackerState,
};
use bevy::prelude::*;

pub fn sync_tracks_to_visuals(
    mut commands: Commands,
    fusion_tracks: Query<(
        Entity,
        &Track,
        &TrackerState,
        &TrackQuality,
        &TargetClassification,
    )>,
    mut visuals: Query<(
        &FusionTrackLink,
        &mut Aircraft,
        Option<&mut InterpolationState>,
        Option<&mut FusionDiagnostics>,
    )>,
    visual_lookup: Query<(Entity, &FusionTrackLink)>,
    model_registry: Option<Res<AircraftModelRegistry>>,
    type_db: Option<Res<crate::aircraft::AircraftTypeDatabase>>,
    enrichment_mgr: Option<Res<EnrichmentConnectionManager>>,
    timeline_store: Res<TimelineStore>,
    clock: Res<super::clock::SimClock>,
    map_state: Res<MapState>,
    local_origin: Res<LocalOrigin>,
    view3d_state: Res<view3d::View3DState>,
) {
    let Some(model_registry) = model_registry else {
        return;
    };

    for (track_entity, track, tracker, quality, classification) in &fusion_tracks {
        let track_icao = track
            .cooperative_ids
            .iter()
            .find(|id| id.id_type == IdentifierType::Icao)
            .and_then(|id| adsb_client::Icao::from_hex(&id.id));
        let position_source = track_icao
            .and_then(|icao| enrichment_mgr.as_ref().and_then(|mgr| mgr.lookup(icao)))
            .map(|info| info.source);

        // Single source of truth: derive the render-ready DisplayTrack once. The
        // prefer-raw/prefer-filter merge + ECEF->geodetic derivation live in
        // airjedi_fusion::derive_display_track (reachable from headless tests);
        // the raw ADS-B overrides cross into it via a sensor-agnostic hint. The
        // visual `Aircraft` written below is a client-side view built from this
        // same `dt`; the serializable DisplayTrack is stored on the track entity
        // as the projection boundary. Both embedded and headless paths build
        // the hint from the same timestamped TimelineStore observations.
        let hint = raw_observation_hint_for(&timeline_store, track);
        let dt = derive_display_track(track, tracker, quality, hint.as_ref(), position_source);
        let is_coasting = dt.status == TrackStatus::Coasting;
        commands.entity(track_entity).insert(dt.clone());

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
                let position_changed = (dt.latitude - aircraft.latitude).abs() > f64::EPSILON
                    || (dt.longitude - aircraft.longitude).abs() > f64::EPSILON;

                aircraft.latitude = dt.latitude;
                aircraft.longitude = dt.longitude;
                aircraft.altitude = dt.altitude_ft;
                aircraft.heading = dt.heading;
                aircraft.velocity = dt.velocity_kts;
                aircraft.vertical_rate = dt.vertical_rate;
                aircraft.is_on_ground = dt.is_on_ground;
                aircraft.alert = dt.alert;
                aircraft.emergency = dt.emergency;
                aircraft.spi = dt.spi;
                aircraft.roll_angle = dt.roll_angle;
                aircraft.track_angle_rate = dt.track_angle_rate;
                if dt.roll_angle.is_some() || dt.track_angle_rate.is_some() {
                    aircraft.roll_last_seen = Some(dt.last_seen);
                }
                aircraft.last_seen = dt.last_seen;
                if dt.squawk.is_some() {
                    aircraft.squawk = dt.squawk.clone();
                }

                if aircraft.callsign.is_none() {
                    for cid in &track.cooperative_ids {
                        if cid.id_type == IdentifierType::Callsign {
                            aircraft.callsign = Some(cid.id.clone());
                            break;
                        }
                    }
                }

                if let Some(mut diag) = diag_opt {
                    update_diagnostics(&mut diag, tracker, quality, position_source);
                }

                if position_changed {
                    if let Some(mut interp) = interp_opt {
                        crate::aircraft::interpolation::update_interpolation_on_adsb(
                            &mut interp,
                            dt.latitude,
                            dt.longitude,
                            dt.altitude_ft,
                            dt.heading,
                            dt.velocity_kts,
                            dt.vertical_rate,
                            None,
                            clock.elapsed_secs_f64(),
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
            let pos = converter.latlon_to_world(dt.latitude, dt.longitude);

            let mut entity_commands = commands.spawn((
                Name::new(format!("Aircraft: {}", aircraft_name)),
                WorldAssetRoot(model_handle),
                Transform::from_xyz(pos.x, pos.y, constants::AIRCRAFT_Z_LAYER),
                Pickable::default(),
                Aircraft {
                    icao: icao.clone(),
                    callsign: callsign.clone(),
                    latitude: dt.latitude,
                    longitude: dt.longitude,
                    altitude: dt.altitude_ft,
                    heading: dt.heading,
                    velocity: dt.velocity_kts,
                    vertical_rate: dt.vertical_rate,
                    roll_angle: dt.roll_angle,
                    track_angle_rate: dt.track_angle_rate,
                    roll_last_seen: if dt.roll_angle.is_some() || dt.track_angle_rate.is_some() {
                        Some(dt.last_seen)
                    } else {
                        None
                    },
                    squawk: dt.squawk.clone(),
                    is_on_ground: dt.is_on_ground,
                    alert: dt.alert,
                    emergency: dt.emergency,
                    spi: dt.spi,
                    last_seen: dt.last_seen,
                },
                FusionTrackLink {
                    track_entity,
                    track_id: track.id.clone(),
                },
                make_diagnostics(tracker, quality, position_source),
                TrailHistory::default(),
                InterpolationState::new(
                    dt.latitude,
                    dt.longitude,
                    dt.altitude_ft,
                    dt.heading,
                    dt.velocity_kts,
                    dt.vertical_rate,
                    None,
                    clock.elapsed_secs_f64(),
                ),
            ));
            if let Some(corr) = correction {
                entity_commands.insert(corr);
            }
            entity_commands
                .observe(on_aircraft_click)
                .observe(on_aircraft_hover)
                .observe(on_aircraft_out);
        }
    }
}

fn make_diagnostics(
    tracker: &TrackerState,
    quality: &TrackQuality,
    position_source: Option<PositionSource>,
) -> FusionDiagnostics {
    let mode = tracker.mode_info();
    FusionDiagnostics {
        filter_type: filter_type_label(tracker),
        mode_probabilities: mode.as_ref().map(|m| m.probabilities.clone()),
        dominant_mode: mode.as_ref().map(|m| m.dominant_mode),
        track_status: Some(quality.status),
        observation_count: quality.observation_count,
        last_position_source: position_source,
    }
}

fn update_diagnostics(
    diag: &mut FusionDiagnostics,
    tracker: &TrackerState,
    quality: &TrackQuality,
    position_source: Option<PositionSource>,
) {
    let mode = tracker.mode_info();
    diag.filter_type = filter_type_label(tracker);
    diag.mode_probabilities = mode.as_ref().map(|m| m.probabilities.clone());
    diag.dominant_mode = mode.as_ref().map(|m| m.dominant_mode);
    diag.track_status = Some(quality.status);
    diag.observation_count = quality.observation_count;
    if position_source.is_some() {
        diag.last_position_source = position_source;
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
    clock: Res<super::clock::SimClock>,
) {
    let now = clock.now_utc();

    for (visual_entity, link, aircraft) in &visuals {
        let orphaned = fusion_tracks.get(link.track_entity).is_err();
        let age_secs = (now - aircraft.last_seen).num_seconds();
        let timed_out = age_secs > crate::constants::ADSB_AIRCRAFT_TIMEOUT_SECS;

        if orphaned || timed_out {
            commands.entity(visual_entity).despawn();
        }
    }
}
