//! Thin-client mode (design-b Phase 4): render a headless fusion agent's output.
//!
//! Instead of running fusion in-process (fat mode, [`crate::fusion_integration`]),
//! the app connects to an agent over `bevy_replicon` + renet and receives the
//! serializable [`DisplayTrack`] components the agent projects. This module
//! *hydrates* each replicated `DisplayTrack` into the app's existing aircraft
//! visual (adds `Aircraft` + `Transform` + model + `InterpolationState` + trails
//! + picking), so every always-on renderer (camera, view3d, trails, labels,
//! list panel, picking) drives it exactly as in fat mode. The UI cannot tell
//! which mode it is in - that is the whole point of the display boundary.
//!
//! Visuals are attached to the replicated entity itself, so when the agent drops
//! a track and replicon despawns the entity, the visual goes with it - no
//! separate cleanup. Positioning is handled by `AircraftPlugin`'s
//! `interpolate_aircraft_positions` + the camera/view3d transform writers (all
//! always on); we only keep `InterpolationState` fed, since the fat-mode
//! `predicting` sync does not run here.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use bevy::prelude::*;
use bevy_replicon::prelude::{ClientState, RepliconChannels, RepliconPlugins};
use bevy_replicon_renet::{netcode::NetcodeClientTransport, RenetClient, RepliconRenetPlugins};

use airjedi_core::{
    DisplayEstimate, DisplayTrack, DisplayTrail, SensorContributions, TrackId, TrackStatus,
};
use airjedi_net::{
    create_client, register_replicated, ClientHistoryStore, HistoryClientMessage,
    HistoryLoadingState, HistoryRequestPriority, DEFAULT_PORT,
};

use crate::adsb::sync::AircraftModelRegistry;
use crate::aircraft::components::{AuthoritativeHistory, HistoryMaterialized};
use crate::aircraft::interpolation::update_interpolation_on_adsb;
use crate::aircraft::picking::{on_aircraft_click, on_aircraft_hover, on_aircraft_out};
use crate::aircraft::{
    AircraftListState, AircraftTypeDatabase, CameraFollowState, InterpolationState, SessionClock,
    TrailHistory,
};
use crate::config::AppConfig;
use crate::fusion_integration::estimated_track::EstimatedTrackConfig;
use crate::fusion_integration::multi_sensor_debug::{sensor_color, MultiSensorDebugConfig};
use crate::geo::CoordinateConverter;
use crate::statusbar::ThinClientStatus;
use crate::tiles::LocalOrigin;
use crate::Aircraft;

/// The resolved agent address the client connects to.
#[derive(Resource, Clone, Copy)]
struct ThinAgentAddr(SocketAddr);

/// Throttles reconnect attempts while the client is disconnected.
#[derive(Resource)]
struct ReconnectBackoff(Timer);

#[derive(Resource, Default)]
struct SelectedHistoryTrack(Option<TrackId>);

/// Registers the replication client and the `DisplayTrack` -> visual hydrator.
pub struct ThinClientPlugin {
    agent_addr: String,
}

impl ThinClientPlugin {
    #[must_use]
    pub fn new(agent_addr: String) -> Self {
        Self { agent_addr }
    }
}

impl Plugin for ThinClientPlugin {
    fn build(&self, app: &mut App) {
        let addr = parse_agent_addr(&self.agent_addr);

        // Silence local ingest: in thin mode the agent is the sole track source,
        // so AdsbPlugin should open no feeds or enrichment streams (it still
        // provides the model registry the hydrator uses).
        app.insert_resource(crate::adsb::LocalIngestDisabled);

        app.add_plugins((RepliconPlugins, RepliconRenetPlugins));
        register_replicated(app);

        // Overlay configs the fat FusionIntegrationPlugin would normally provide.
        app.insert_resource(EstimatedTrackConfig::default())
            .register_type::<EstimatedTrackConfig>()
            .insert_resource(MultiSensorDebugConfig::default())
            .register_type::<MultiSensorDebugConfig>();

        app.insert_resource(ThinAgentAddr(addr))
            .insert_resource(ThinClientStatus::default())
            .init_resource::<ClientHistoryStore>()
            .init_resource::<SelectedHistoryTrack>()
            .insert_resource(ReconnectBackoff(Timer::from_seconds(
                2.0,
                TimerMode::Repeating,
            )))
            .add_systems(Startup, connect_to_agent)
            .add_systems(
                Update,
                (
                    manage_connection,
                    update_thin_status,
                    ingest_history_previews,
                    receive_history_messages,
                    request_history_transfers,
                    hydrate_new_tracks,
                    update_hydrated_tracks,
                    materialize_client_history,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (draw_estimated_cones, draw_sensor_contributions)
                    .after(crate::aircraft::interpolation::interpolate_aircraft_positions),
            );
    }
}

/// Resolve the agent address from `AIRJEDI_THIN_AGENT`. Accepts `host:port` or
/// `IP:PORT` (DNS names resolved), a bare `IP` or `host` (default port), else
/// falls back to localhost.
fn parse_agent_addr(s: &str) -> SocketAddr {
    use std::net::ToSocketAddrs;

    // "host:port" / "ip:port", resolving DNS names.
    if let Ok(mut addrs) = s.to_socket_addrs() {
        if let Some(addr) = addrs.next() {
            return addr;
        }
    }
    // Bare IP -> default port.
    if let Ok(ip) = s.parse::<IpAddr>() {
        return SocketAddr::new(ip, DEFAULT_PORT);
    }
    // Bare hostname -> resolve with the default port.
    if let Ok(mut addrs) = (s, DEFAULT_PORT).to_socket_addrs() {
        if let Some(addr) = addrs.next() {
            return addr;
        }
    }
    warn!("could not resolve AIRJEDI_THIN_AGENT '{s}'; using 127.0.0.1:{DEFAULT_PORT}");
    SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT))
}

fn connect_to_agent(
    mut commands: Commands,
    channels: Res<RepliconChannels>,
    addr: Res<ThinAgentAddr>,
) {
    match create_client(&channels, addr.0) {
        Ok((client, transport)) => {
            commands.insert_resource(client);
            commands.insert_resource(transport);
            info!("thin client connecting to agent at {}", addr.0);
        }
        Err(e) => error!("thin client failed to connect to {}: {e}", addr.0),
    }
}

/// Re-dial the agent when the connection drops. Two steps, because replicon only
/// registers a *fresh* client connection (netcode tokens are single-use):
///  1. On disconnect, tear down the stale `RenetClient` + transport.
///  2. Once absent, mint a new client + transport after a backoff interval.
fn manage_connection(
    client: Option<Res<RenetClient>>,
    state: Res<State<ClientState>>,
    time: Res<Time>,
    mut backoff: ResMut<ReconnectBackoff>,
    channels: Res<RepliconChannels>,
    addr: Res<ThinAgentAddr>,
    stale: Query<Entity, With<DisplayTrack>>,
    mut commands: Commands,
) {
    // Step 1: disconnected but the stale client is still present - remove it so
    // the next insert reads as a brand-new connection, and despawn the now-stale
    // replicated aircraft so a reconnect repopulates cleanly instead of doubling.
    if *state.get() == ClientState::Disconnected && client.is_some() {
        commands.remove_resource::<RenetClient>();
        commands.remove_resource::<NetcodeClientTransport>();
        for entity in &stale {
            commands.entity(entity).despawn();
        }
        return;
    }

    // Connecting or connected: keep the backoff primed for the next drop.
    if client.is_some() {
        backoff.0.reset();
        return;
    }

    // Step 2: no client - (re)connect once the backoff elapses.
    backoff.0.tick(time.delta());
    if !backoff.0.just_finished() {
        return;
    }
    match create_client(&channels, addr.0) {
        Ok((client, transport)) => {
            commands.insert_resource(client);
            commands.insert_resource(transport);
            info!("thin client reconnecting to agent at {}", addr.0);
        }
        Err(e) => warn!("thin client reconnect failed: {e}"),
    }
}

/// Publish agent-connection status + replicated aircraft count for the status bar.
fn update_thin_status(
    state: Res<State<ClientState>>,
    aircraft: Query<(), With<Aircraft>>,
    mut status: ResMut<ThinClientStatus>,
) {
    status.connected = *state.get() == ClientState::Connected;
    status.aircraft = aircraft.iter().count();
}

/// Build the app's `Aircraft` view from a replicated `DisplayTrack`.
fn aircraft_from_display(dt: &DisplayTrack) -> Aircraft {
    Aircraft {
        icao: dt.icao.clone(),
        callsign: dt.callsign.clone(),
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
    }
}

/// Attach the full aircraft visual to each newly replicated `DisplayTrack`.
fn hydrate_new_tracks(
    mut commands: Commands,
    new_tracks: Query<(Entity, &DisplayTrack, Option<&TrailHistory>)>,
    model_registry: Option<Res<AircraftModelRegistry>>,
    type_db: Option<Res<AircraftTypeDatabase>>,
    local_origin: Res<LocalOrigin>,
    time: Res<Time<Real>>,
) {
    let Some(model_registry) = model_registry else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let converter = CoordinateConverter::new(&local_origin);

    for (entity, dt, trail) in &new_tracks {
        if trail.is_some() {
            continue;
        }
        let type_info = type_db.as_ref().and_then(|db| db.lookup(&dt.icao));
        let type_code = type_info.as_ref().and_then(|i| i.type_code.clone());
        let registration = type_info.as_ref().and_then(|i| i.registration.clone());

        let model_handle = model_registry.get_model(type_code.as_deref());
        let correction = model_registry.get_correction(type_code.as_deref());

        let display_name = dt
            .callsign
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(registration.as_deref())
            .unwrap_or(&dt.icao);

        let pos = converter.latlon_to_world(dt.latitude, dt.longitude);

        let mut entity_commands = commands.entity(entity);
        entity_commands.insert((
            Name::new(format!("Aircraft: {display_name}")),
            WorldAssetRoot(model_handle),
            Transform::from_xyz(pos.x, pos.y, crate::constants::AIRCRAFT_Z_LAYER),
            Pickable::default(),
            aircraft_from_display(dt),
            AuthoritativeHistory,
            HistoryMaterialized::default(),
            TrailHistory::default(),
            InterpolationState::new(
                dt.latitude,
                dt.longitude,
                dt.altitude_ft,
                dt.heading,
                dt.velocity_kts,
                dt.vertical_rate,
                dt.is_on_ground,
                now,
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

fn ingest_history_previews(previews: Query<&DisplayTrail>, mut store: ResMut<ClientHistoryStore>) {
    for preview in &previews {
        store.install_preview(preview);
    }
}

fn receive_history_messages(
    mut messages: MessageReader<airjedi_net::HistoryServerMessage>,
    mut store: ResMut<ClientHistoryStore>,
) {
    for message in messages.read() {
        store.apply(message);
    }
}

fn request_history_transfers(
    list_state: Res<AircraftListState>,
    tracks: Query<(&DisplayTrack, Option<&DisplayTrail>)>,
    mut selected: ResMut<SelectedHistoryTrack>,
    mut store: ResMut<ClientHistoryStore>,
    mut messages: MessageWriter<HistoryClientMessage>,
) {
    let live_tracks: std::collections::HashSet<TrackId> = tracks
        .iter()
        .filter_map(|(track, preview)| preview.map(|_| track.track_id.clone()))
        .collect();
    for cancel in store.retain_tracks(&live_tracks) {
        messages.write(HistoryClientMessage::Cancel(cancel));
    }

    let selected_track = list_state.selected_icao.as_ref().and_then(|icao| {
        tracks
            .iter()
            .find(|(track, _)| &track.icao == icao)
            .map(|(track, _)| track.track_id.clone())
    });

    if selected.0 != selected_track {
        if let Some(previous) = selected.0.as_ref() {
            if let Some(cancel) = store.cancel_request(previous) {
                messages.write(HistoryClientMessage::Cancel(cancel));
            }
        }
        selected.0 = selected_track.clone();
    }

    if let Some(track_id) = selected_track.as_ref() {
        send_request_plan(
            store.prepare_request(track_id, HistoryRequestPriority::Selected),
            &mut messages,
        );
    }

    let mut background_count = store.active_request_count_for(HistoryRequestPriority::Background);
    let mut candidates: Vec<(String, TrackId)> = tracks
        .iter()
        .filter_map(|(track, preview)| {
            if selected_track.as_ref() == Some(&track.track_id) || preview.is_none() {
                return None;
            }
            Some((track.track_id.0.to_string(), track.track_id.clone()))
        })
        .collect();
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    for (_, track_id) in candidates {
        if background_count >= 2 {
            break;
        }
        let plan = store.prepare_request(&track_id, HistoryRequestPriority::Background);
        if plan.request.is_some() {
            background_count += 1;
        }
        send_request_plan(plan, &mut messages);
    }
}

fn send_request_plan(
    plan: airjedi_net::HistoryRequestPlan,
    messages: &mut MessageWriter<HistoryClientMessage>,
) {
    if let Some(cancel) = plan.cancel {
        messages.write(HistoryClientMessage::Cancel(cancel));
    }
    if let Some(request) = plan.request {
        messages.write(HistoryClientMessage::Request(request));
    }
}

/// Materialize either the selected full history or the bounded preview. The
/// component remains present while assets are loading, so hydration retries on
/// a later frame instead of losing the already received samples.
fn materialize_client_history(
    list_state: Res<AircraftListState>,
    store: Res<ClientHistoryStore>,
    clock: Res<SessionClock>,
    mut commands: Commands,
    mut visuals: Query<(
        Entity,
        &DisplayTrack,
        &DisplayTrail,
        &mut TrailHistory,
        Option<&HistoryMaterialized>,
    )>,
) {
    for (entity, track, preview, mut trail, marker) in &mut visuals {
        let selected = list_state.selected_icao.as_ref() == Some(&track.icao);
        let full_history = selected
            .then(|| store.track(&track.track_id))
            .flatten()
            .filter(|history| !matches!(history.loading, HistoryLoadingState::Preview));

        let (session_id, revision, full) = full_history.map_or(
            (preview.session_id, preview.history_revision, false),
            |history| (history.session_id, history.history_revision, true),
        );
        let unchanged = marker.is_some_and(|marker| {
            marker.session_id == Some(session_id)
                && marker.revision == revision
                && marker.full == full
        });
        if unchanged {
            continue;
        }

        if let Some(history) = full_history {
            trail.replace_from_samples(&history.samples, history.server_time, &clock);
        } else {
            trail.replace_from_display(preview, &clock);
        }
        commands.entity(entity).insert(HistoryMaterialized {
            session_id: Some(session_id),
            revision,
            full,
        });
    }
}

/// Push each replicated `DisplayTrack` change into its `Aircraft` view and
/// refresh the interpolation baseline (mirrors the fat-mode render bridge).
fn update_hydrated_tracks(
    mut query: Query<
        (
            &DisplayTrack,
            &mut Aircraft,
            Option<&mut InterpolationState>,
        ),
        Changed<DisplayTrack>,
    >,
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs_f64();
    for (dt, mut aircraft, interp) in &mut query {
        let position_changed = (dt.latitude - aircraft.latitude).abs() > f64::EPSILON
            || (dt.longitude - aircraft.longitude).abs() > f64::EPSILON;

        aircraft.latitude = dt.latitude;
        aircraft.longitude = dt.longitude;
        aircraft.altitude = dt.altitude_ft;
        aircraft.heading = dt.heading;
        aircraft.velocity = dt.velocity_kts;
        aircraft.vertical_rate = dt.vertical_rate;
        aircraft.roll_angle = dt.roll_angle;
        aircraft.track_angle_rate = dt.track_angle_rate;
        if dt.roll_angle.is_some() || dt.track_angle_rate.is_some() {
            aircraft.roll_last_seen = Some(dt.last_seen);
        }
        aircraft.squawk = dt.squawk.clone();
        aircraft.is_on_ground = dt.is_on_ground;
        aircraft.alert = dt.alert;
        aircraft.emergency = dt.emergency;
        aircraft.spi = dt.spi;
        aircraft.last_seen = dt.last_seen;

        if position_changed {
            if let Some(mut interp) = interp {
                update_interpolation_on_adsb(
                    &mut interp,
                    dt.latitude,
                    dt.longitude,
                    dt.altitude_ft,
                    dt.heading,
                    dt.velocity_kts,
                    dt.vertical_rate,
                    dt.is_on_ground,
                    now,
                );
            }
        }
    }
}

// --- thin-mode overlay drawers (replicated DisplayEstimate / SensorContributions) ---
//
// These mirror the fat FusionIntegrationPlugin drawers, but read the replicated
// components straight off the aircraft entity (thin mode puts DisplayTrack,
// DisplayEstimate, SensorContributions, and Aircraft all on one entity - no
// FusionTrackLink join). The forward sampling and per-sensor collection already
// ran agent-side; the client only draws.

fn cone_center_color(maneuver_prob: f32, alpha: f32) -> Color {
    let r = maneuver_prob;
    let g = 0.85 * (1.0 - maneuver_prob) + 0.65 * maneuver_prob;
    let b = 1.0 * (1.0 - maneuver_prob);
    Color::srgba(r, g, b, alpha)
}

fn cone_boundary_color(maneuver_prob: f32, alpha: f32) -> Color {
    let r = 0.3 * (1.0 - maneuver_prob) + 1.0 * maneuver_prob;
    let g = 0.7 * (1.0 - maneuver_prob) + 0.55 * maneuver_prob;
    let b = 1.0 * (1.0 - maneuver_prob) + 0.1 * maneuver_prob;
    Color::srgba(r, g, b, alpha)
}

/// Draw the forward-prediction cone for the selected/followed aircraft from its
/// replicated `DisplayEstimate` (mirrors `draw_estimated_track_cones`).
fn draw_estimated_cones(
    mut gizmos: Gizmos,
    config: Res<EstimatedTrackConfig>,
    app_config: Res<AppConfig>,
    list_state: Res<AircraftListState>,
    follow_state: Res<CameraFollowState>,
    local_origin: Res<LocalOrigin>,
    tracks: Query<(
        &DisplayEstimate,
        &DisplayTrack,
        &Aircraft,
        Option<&InterpolationState>,
    )>,
) {
    if !config.enabled {
        return;
    }
    let Some(target) = follow_state
        .following_icao
        .as_ref()
        .or(list_state.selected_icao.as_ref())
    else {
        return;
    };
    let Some((estimate, track, aircraft, interp)) =
        tracks.iter().find(|(_, _, a, _)| &a.icao == target)
    else {
        return;
    };
    if estimate.samples.is_empty() {
        return;
    }

    let (vis_lat, vis_lon) = if app_config.interpolation_enabled {
        interp
            .map(|i| (i.display_lat, i.display_lon))
            .unwrap_or((aircraft.latitude, aircraft.longitude))
    } else {
        (aircraft.latitude, aircraft.longitude)
    };

    let is_coasting = track.status == TrackStatus::Coasting;
    let maneuver_prob = estimate.maneuver_prob;
    let converter = CoordinateConverter::new(&local_origin);
    let start = converter.latlon_to_world(vis_lat, vis_lon);
    let horizon = estimate
        .samples
        .last()
        .map(|s| s.time_ahead)
        .unwrap_or(1.0)
        .max(1e-3);

    let mut prev_center = start;
    let mut prev_left = start;
    let mut prev_right = start;
    let n = estimate.samples.len();
    for (i, s) in estimate.samples.iter().enumerate() {
        let t_frac = s.time_ahead / horizon;
        let alpha_fade = (1.0 - t_frac * t_frac * 0.7).max(0.1);
        let pos = converter.latlon_to_world(s.lat, s.lon);
        let radius = s.h_uncertainty_m as f32;
        let hr = (s.heading_deg as f64).to_radians();
        let dir = Vec2::new(hr.sin() as f32, hr.cos() as f32);
        if dir == Vec2::ZERO {
            prev_center = pos;
            continue;
        }
        let perp = Vec2::new(-dir.y, dir.x);
        let left = pos + perp * radius;
        let right = pos - perp * radius;

        let ca = if is_coasting { 0.55 } else { 0.75 };
        let center = if is_coasting {
            Color::srgba(1.0, 0.55, 0.1, ca * alpha_fade)
        } else {
            cone_center_color(maneuver_prob, ca * alpha_fade)
        };
        let boundary = if is_coasting {
            Color::srgba(1.0, 0.4, 0.1, 0.35 * alpha_fade)
        } else {
            cone_boundary_color(maneuver_prob, 0.45 * alpha_fade)
        };
        let cross = cone_boundary_color(maneuver_prob, 0.12 * alpha_fade);

        gizmos.line_2d(prev_center, pos, center);
        gizmos.line_2d(prev_left, left, boundary);
        gizmos.line_2d(prev_right, right, boundary);
        gizmos.line_2d(left, right, cross);

        if i == n - 1 {
            gizmos.circle_2d(
                pos,
                radius.max(200.0),
                cone_center_color(maneuver_prob, 0.55),
            );
        }

        prev_center = pos;
        prev_left = left;
        prev_right = right;
    }
}

/// Draw per-sensor markers + lines to the fused position, for tracks with 2+
/// contributing sensors (mirrors `draw_multi_sensor_sources`).
fn draw_sensor_contributions(
    mut gizmos: Gizmos,
    config: Res<MultiSensorDebugConfig>,
    list_state: Res<AircraftListState>,
    follow_state: Res<CameraFollowState>,
    local_origin: Res<LocalOrigin>,
    tracks: Query<(&SensorContributions, &Aircraft)>,
) {
    if !config.enabled {
        return;
    }
    let selected = follow_state
        .following_icao
        .as_ref()
        .or(list_state.selected_icao.as_ref());
    if !config.show_all_aircraft && selected.is_none() {
        return;
    }

    let converter = CoordinateConverter::new(&local_origin);
    for (contrib, aircraft) in &tracks {
        if !config.show_all_aircraft && Some(&aircraft.icao) != selected {
            continue;
        }
        if contrib.sources.len() < 2 {
            continue;
        }
        let fused = converter.latlon_to_world(aircraft.latitude, aircraft.longitude);
        for report in &contrib.sources {
            let pos = converter.latlon_to_world(report.lat, report.lon);
            let color = sensor_color(&report.sensor_id, 0.85);
            gizmos.circle_2d(pos, 30.0, color);
            gizmos.line_2d(pos, fused, color.with_alpha(0.35));
        }
    }
}
