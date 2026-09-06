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
use bevy_replicon::prelude::{RepliconChannels, RepliconPlugins};
use bevy_replicon_renet::RepliconRenetPlugins;

use airjedi_core::DisplayTrack;
use airjedi_net::{create_client, register_replicated, DEFAULT_PORT};

use crate::adsb::sync::AircraftModelRegistry;
use crate::aircraft::interpolation::update_interpolation_on_adsb;
use crate::aircraft::{AircraftTypeDatabase, InterpolationState, TrailHistory};
use crate::aircraft::picking::{on_aircraft_click, on_aircraft_hover, on_aircraft_out};
use crate::geo::CoordinateConverter;
use crate::tiles::LocalOrigin;
use crate::Aircraft;

/// The resolved agent address the client connects to.
#[derive(Resource, Clone, Copy)]
struct ThinAgentAddr(SocketAddr);

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
        app.insert_resource(ThinAgentAddr(addr))
            .add_systems(Startup, connect_to_agent)
            .add_systems(Update, (hydrate_new_tracks, update_hydrated_tracks).chain());
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
    new_tracks: Query<(Entity, &DisplayTrack), Added<DisplayTrack>>,
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

    for (entity, dt) in &new_tracks {
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

/// Push each replicated `DisplayTrack` change into its `Aircraft` view and
/// refresh the interpolation baseline (mirrors the fat-mode render bridge).
fn update_hydrated_tracks(
    mut query: Query<
        (&DisplayTrack, &mut Aircraft, Option<&mut InterpolationState>),
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
