//! `airjedi-net` - the design-b thin-mode transport.
//!
//! This is the layer Phase 1 deferred as "the Phase 4 decision": it copies the
//! serializable display components (`DisplayTrack` et al.) from the fusion agent
//! `World` into a thin client `World` so the UI reads the *same* components and
//! cannot tell which mode it is in.
//!
//! Transport is [`bevy_replicon`] (server-authoritative ECS replication) over a
//! [`bevy_replicon_renet`]/renet UDP backend. This crate does two jobs:
//!
//! 1. [`register_replicated`] - the single source of truth for *which* display
//!    components replicate, in a fixed order. `bevy_replicon` matches components
//!    between server and client by registration order, so both sides MUST call
//!    this one function.
//! 2. [`create_server`] / [`create_client`] - the renet + netcode boilerplate,
//!    kept in one place so the agent and the app set up the wire identically.

use std::error::Error;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::SystemTime;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::netcode::{
    ClientAuthentication, NetcodeClientTransport, NetcodeServerTransport, ServerAuthentication,
    ServerConfig,
};
use bevy_replicon_renet::renet::ConnectionConfig;
use bevy_replicon_renet::{RenetChannelsExt, RenetClient, RenetServer};

use airjedi_core::{DisplayEstimate, DisplayTrack, SensorContributions};

/// Default UDP port the fusion agent listens on for thin clients.
pub const DEFAULT_PORT: u16 = 5599;

/// netcode protocol id. Server and client must agree; bump this if the
/// replicated component set changes shape in a wire-incompatible way.
pub const PROTOCOL_ID: u64 = 0xA17E_D100_0001;

/// Register the display components that replicate agent -> client, in a fixed
/// order. Both the agent (server) and the thin client MUST call this - identical
/// registration order is what lets `bevy_replicon` line the component sets up.
///
/// Only the render-facing components are here. `DisplayTrail` is intentionally
/// omitted (Phase 1 leaves it unpopulated).
pub fn register_replicated(app: &mut App) {
    app.replicate::<DisplayTrack>();
    app.replicate::<DisplayEstimate>();
    app.replicate::<SensorContributions>();
}

/// Build the renet server resources (the [`RenetServer`] and its
/// [`NetcodeServerTransport`]) bound to `0.0.0.0:port`. Insert both into the
/// agent `World`; `RepliconRenetPlugins` drives them from there.
///
/// # Errors
/// Fails if the UDP socket cannot bind or the netcode transport cannot start.
pub fn create_server(
    channels: &RepliconChannels,
    port: u16,
    public_addr: SocketAddr,
) -> Result<(RenetServer, NetcodeServerTransport), Box<dyn Error>> {
    let server = RenetServer::new(ConnectionConfig {
        server_channels_config: channels.server_configs(),
        client_channels_config: channels.client_configs(),
        ..Default::default()
    });

    let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))?;
    let server_config = ServerConfig {
        current_time,
        max_clients: 64,
        protocol_id: PROTOCOL_ID,
        authentication: ServerAuthentication::Unsecure,
        public_addresses: vec![public_addr],
    };
    let transport = NetcodeServerTransport::new(server_config, socket)?;

    Ok((server, transport))
}

/// Build the renet client resources ([`RenetClient`] + [`NetcodeClientTransport`])
/// pointed at `server_addr`. Insert both into the client `World`.
///
/// # Errors
/// Fails if the UDP socket cannot bind or the netcode transport cannot start.
pub fn create_client(
    channels: &RepliconChannels,
    server_addr: SocketAddr,
) -> Result<(RenetClient, NetcodeClientTransport), Box<dyn Error>> {
    let client = RenetClient::new(ConnectionConfig {
        server_channels_config: channels.server_configs(),
        client_channels_config: channels.client_configs(),
        ..Default::default()
    });

    let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
    let client_id = current_time.as_millis() as u64;
    // Bind on all local interfaces so replies from a remote agent can reach the
    // client. Binding to loopback only works for the in-process round-trip test.
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let authentication = ClientAuthentication::Unsecure {
        client_id,
        protocol_id: PROTOCOL_ID,
        server_addr,
        user_data: None,
    };
    let transport = NetcodeClientTransport::new(current_time, authentication, socket)?;

    Ok((client, transport))
}
