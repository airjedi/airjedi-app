//! Proves the design-b thin-mode wire end to end: a server `App` spawns a
//! `Replicated` `DisplayTrack`, a client `App` connects over renet loopback, and
//! the exact component (with its `PositionSource`) arrives in the client `World`.
//!
//! This isolates the transport from fusion - fusion timing is covered by the
//! tier-4 test; here we verify replication + `DisplayTrack` serialization only.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use airjedi_core::{DisplayTrack, PositionSource, TrackId, TrackStatus};
use airjedi_net::{create_client, create_server, register_replicated};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::RepliconRenetPlugins;
use chrono::Utc;

// A distinct port so the test never collides with a running agent (DEFAULT_PORT).
const TEST_PORT: u16 = 5601;

fn sample_track() -> DisplayTrack {
    DisplayTrack {
        track_id: TrackId::new(),
        icao: "ae5e13".to_string(),
        callsign: Some("N123AB".to_string()),
        latitude: 38.1234,
        longitude: -97.5678,
        altitude_ft: Some(30_000),
        position_freshness: None,
        altitude_freshness: None,
        velocity_freshness: None,
        altitude_reference: airjedi_core::AltitudeReference::Barometric,
        heading: Some(270.0),
        heading_reference: airjedi_core::HeadingReference::GroundTrack,
        velocity_kts: Some(420.0),
        airspeed_kts: None,
        vertical_rate: Some(0),
        vertical_rate_reference: airjedi_core::VerticalRateReference::FeetPerMinute,
        roll_angle: None,
        track_angle_rate: None,
        squawk: Some("1200".to_string()),
        is_on_ground: Some(false),
        alert: Some(false),
        emergency: Some(false),
        spi: Some(false),
        last_seen: Utc::now(),
        status: TrackStatus::Confirmed,
        position_source: Some(PositionSource::Mlat),
        h_uncertainty_m: Some(250.0),
        predicting: true,
        filter_type: "IMM".to_string(),
        mode_probabilities: Some(vec![0.7, 0.3]),
        dominant_mode: Some(0),
        observation_count: 12,
        provenance: airjedi_core::DisplayProvenance::default(),
    }
}

fn base_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins,
        RepliconRenetPlugins,
    ));
    register_replicated(&mut app);
    app
}

fn setup_server(mut commands: Commands, channels: Res<RepliconChannels>) {
    let (server, transport) = create_server(
        &channels,
        TEST_PORT,
        SocketAddr::from((Ipv4Addr::LOCALHOST, TEST_PORT)),
    )
    .expect("server transport should start");
    commands.insert_resource(server);
    commands.insert_resource(transport);
    commands.spawn((Replicated, sample_track()));
}

fn setup_client(mut commands: Commands, channels: Res<RepliconChannels>) {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, TEST_PORT));
    let (client, transport) =
        create_client(&channels, addr).expect("client transport should start");
    commands.insert_resource(client);
    commands.insert_resource(transport);
}

#[test]
fn display_track_replicates_agent_to_client() {
    let mut server = base_app();
    server.add_systems(Startup, setup_server);
    // Driving an App with update() by hand skips run()'s implicit finish/cleanup,
    // which is where plugins (incl. replicon channel setup) complete. Do it here
    // so the RenetServer is built with the full channel set.
    server.finish();
    server.cleanup();

    let mut client = base_app();
    client.add_systems(Startup, setup_client);
    client.finish();
    client.cleanup();

    // Pump both apps over real localhost UDP until the client receives the track
    // (netcode handshake + replication), or time out.
    let mut received = 0usize;
    for _ in 0..800 {
        server.update();
        client.update();
        std::thread::sleep(Duration::from_millis(8));
        received = client
            .world_mut()
            .query::<&DisplayTrack>()
            .iter(client.world())
            .count();
        if received >= 1 {
            break;
        }
    }

    assert!(
        received >= 1,
        "client never received the replicated DisplayTrack"
    );

    let world = client.world_mut();
    let mut q = world.query::<&DisplayTrack>();
    let dt = q
        .iter(world)
        .next()
        .expect("replicated DisplayTrack present");
    assert_eq!(dt.icao, "ae5e13", "icao should round-trip");
    assert_eq!(
        dt.position_source,
        Some(PositionSource::Mlat),
        "position source should round-trip across the wire"
    );
    assert_eq!(dt.status, TrackStatus::Confirmed);
    assert_eq!(dt.filter_type, "IMM");
    assert!((dt.latitude - 38.1234).abs() < 1e-9);
}
