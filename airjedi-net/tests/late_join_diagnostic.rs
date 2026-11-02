//! Opt-in diagnostic: late join with mature history must not freeze live state.
//! Uses production transport and wire components with field-level provenance and
//! a five-minute preview, then updates the same entity rather than respawning it.
//! Run the local test explicitly with `--ignored --nocapture`. Environment knobs
//! change one variable at a time: DIAG_PREVIEW_SAMPLES, DIAG_SEND_BUDGET,
//! DIAG_SEPARATE_HISTORY, or DIAG_HISTORY_CHUNKS. DIAG_LOG enables backend logs.
//! The separate remote comparison requires an explicit DIAG_AGENT=host:port.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use airjedi_core::{
    DisplayHistorySample, DisplayProvenance, DisplayTrack, DisplayTrail, DisplayValueSource,
    FieldFreshness, FieldProvenance, RawOverride,
};
use airjedi_net::{create_client, create_server, register_replicated};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::renet::ConnectionConfig;
use bevy_replicon_renet::{RenetChannelsExt, RenetServer, RepliconRenetPlugins};
use chrono::{Duration as ChronoDuration, Utc};

use airjedi_net::{HistoryServerMessage, HistorySnapshotChunk, HISTORY_CHUNK_SAMPLES};

#[derive(Resource, Default)]
struct ReceivedChunks(usize);

fn count_chunks(
    mut messages: MessageReader<HistoryServerMessage>,
    mut count: ResMut<ReceivedChunks>,
) {
    for message in messages.read() {
        if matches!(message, HistoryServerMessage::SnapshotChunk(_)) {
            count.0 += 1;
        }
    }
}

mod fixtures {
    use super::*;
    use airjedi_core::{
        AltitudeReference, DisplayHistoryInput, HeadingReference, TrackId, TrackStatus,
        VerticalRateReference,
    };

    pub fn track() -> DisplayTrack {
        DisplayTrack {
            track_id: TrackId::new(),
            icao: "ae5e13".into(),
            callsign: Some("N123AB".into()),
            latitude: 38.1234,
            longitude: -97.5678,
            altitude_ft: Some(30_000),
            position_freshness: None,
            altitude_freshness: None,
            velocity_freshness: None,
            altitude_reference: AltitudeReference::Barometric,
            heading: Some(270.0),
            heading_reference: HeadingReference::GroundTrack,
            velocity_kts: Some(420.0),
            airspeed_kts: None,
            vertical_rate: Some(0),
            vertical_rate_reference: VerticalRateReference::FeetPerMinute,
            roll_angle: None,
            track_angle_rate: None,
            squawk: Some("1200".into()),
            is_on_ground: Some(false),
            alert: Some(false),
            emergency: Some(false),
            spi: Some(false),
            last_seen: Utc::now(),
            status: TrackStatus::Confirmed,
            position_source: None,
            h_uncertainty_m: Some(250.0),
            predicting: false,
            filter_type: "IMM".into(),
            mode_probabilities: Some(vec![0.7, 0.3]),
            dominant_mode: Some(0),
            observation_count: 12,
            provenance: DisplayProvenance::default(),
        }
    }

    pub fn trail() -> DisplayTrail {
        let value = track();
        DisplayTrail {
            track_id: value.track_id.clone(),
            server_time: value.last_seen,
            history_revision: 1,
            samples: vec![DisplayHistorySample::from_input(
                1,
                &DisplayHistoryInput::from(&value),
                0,
                None,
            )],
            preview_window: ChronoDuration::minutes(5),
            ..Default::default()
        }
    }
}

fn base_app(logging: bool) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        RepliconPlugins,
        RepliconRenetPlugins,
    ));
    register_replicated(&mut app);
    if logging {
        app.add_plugins(bevy::log::LogPlugin::default());
    }
    app.finish();
    app.cleanup();
    app
}

fn pump(server: &mut App, client: &mut App, duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        server.update();
        client.update();
        std::thread::sleep(Duration::from_millis(8));
    }
}

fn populate_provenance(sample: &mut DisplayHistorySample) {
    let field = FieldProvenance {
        source: DisplayValueSource::RawObservation,
        freshness: FieldFreshness::Fresh,
        raw_override: RawOverride::Applied,
        observation_time: Some(sample.state_time),
        receipt_time: Some(sample.state_time),
        sensor_id: Some("adsb:live".to_string()),
        ..FieldProvenance::unknown()
    };
    sample.provenance = DisplayProvenance {
        position: field.clone(),
        altitude: field.clone(),
        ground_speed: field.clone(),
        heading: field.clone(),
        vertical_rate: field,
        ..DisplayProvenance::default()
    };
}

#[test]
#[ignore = "Opt-in late-join diagnosis; exercises the currently reported freeze"]
fn mature_history_must_not_freeze_existing_track_updates() {
    let count = std::env::var("DIAG_PREVIEW_SAMPLES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(150);
    let separate_history = std::env::var_os("DIAG_SEPARATE_HISTORY").is_some();
    let chunk_history = std::env::var_os("DIAG_HISTORY_CHUNKS").is_some();
    let mut server = base_app(std::env::var_os("DIAG_LOG").is_some());
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 15603));
    let (network, transport) = create_server(
        server.world().resource::<RepliconChannels>(),
        addr.port(),
        addr,
    )
    .unwrap();
    server.insert_resource(network).insert_resource(transport);
    if let Some(budget) = std::env::var("DIAG_SEND_BUDGET")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        let channels = server.world().resource::<RepliconChannels>();
        let network = RenetServer::new(ConnectionConfig {
            available_bytes_per_tick: budget,
            server_channels_config: channels.server_configs(),
            client_channels_config: channels.client_configs(),
        });
        server.insert_resource(network);
    }

    let mut track = fixtures::track();
    let mut preview = fixtures::trail();
    preview.track_id = track.track_id.clone();
    preview.samples = (0..count)
        .map(|index| {
            let mut sample = preview.samples[0].clone();
            sample.sample_sequence = index as u64;
            sample.state_time =
                track.last_seen - ChronoDuration::seconds(2 * (count - index) as i64);
            populate_provenance(&mut sample);
            sample
        })
        .collect();
    preview.coverage.retained_sample_count = count;
    let entity = server.world_mut().spawn((Replicated, track.clone())).id();
    let history_entity = if separate_history {
        server.world_mut().spawn((Replicated, preview.clone())).id()
    } else {
        server
            .world_mut()
            .entity_mut(entity)
            .insert(preview.clone());
        entity
    };

    let mut client = base_app(false);
    client
        .init_resource::<ReceivedChunks>()
        .add_systems(Update, count_chunks);
    let (network, transport) =
        create_client(client.world().resource::<RepliconChannels>(), addr).unwrap();
    client.insert_resource(network).insert_resource(transport);
    pump(&mut server, &mut client, Duration::from_secs(2));
    let client_entity = client
        .world_mut()
        .query::<(Entity, &DisplayTrack)>()
        .iter(client.world())
        .find(|(_, value)| value.track_id == track.track_id)
        .map(|(entity, _)| entity)
        .expect("initial mature track must arrive");
    let samples_received = client
        .world_mut()
        .query::<&DisplayTrail>()
        .iter(client.world())
        .find(|trail| trail.track_id == track.track_id)
        .unwrap()
        .samples
        .len();
    assert_eq!(samples_received, count);
    let initial_seen = track.last_seen;
    let mut live_updates_received = 0;

    for step in 1..=4 {
        track.latitude += 0.01;
        track.last_seen = initial_seen + ChronoDuration::seconds(step);
        preview.history_revision += 1;
        preview.server_time = track.last_seen;
        server.world_mut().entity_mut(entity).insert(track.clone());
        if chunk_history {
            let request_id = airjedi_core::HistoryRequestId::new();
            for (index, samples) in preview.samples.chunks(HISTORY_CHUNK_SAMPLES).enumerate() {
                server.world_mut().write_message(ToClients {
                    targets: SendTargets::All,
                    message: HistoryServerMessage::SnapshotChunk(HistorySnapshotChunk {
                        session_id: preview.session_id,
                        track_id: preview.track_id.clone(),
                        request_id,
                        server_time: preview.server_time,
                        sample_cutoff: preview.samples.last().map(|sample| sample.sample_sequence),
                        snapshot_revision: preview.history_revision,
                        coverage: preview.coverage.clone(),
                        chunk_index: index as u16,
                        chunk_count: count.div_ceil(HISTORY_CHUNK_SAMPLES) as u16,
                        samples: samples.to_vec(),
                    }),
                });
            }
        } else {
            server
                .world_mut()
                .entity_mut(history_entity)
                .insert(preview.clone());
        }
        pump(&mut server, &mut client, Duration::from_millis(300));
        let received = client.world().get::<DisplayTrack>(client_entity).unwrap();
        if received.last_seen == track.last_seen
            && (received.latitude - track.latitude).abs() < 1e-9
        {
            live_updates_received += 1;
        }
    }

    let mut newcomer = fixtures::track();
    newcomer.icao = "NEW001".into();
    let mut new_preview = fixtures::trail();
    new_preview.track_id = newcomer.track_id.clone();
    server
        .world_mut()
        .spawn((Replicated, newcomer.clone(), new_preview));
    pump(&mut server, &mut client, Duration::from_secs(1));

    let received = client.world().get::<DisplayTrack>(client_entity).unwrap();
    let seen_advance = (received.last_seen - initial_seen).num_milliseconds();
    let moved = (received.latitude - track.latitude).abs() < 1e-9;
    let newcomer_received = client
        .world_mut()
        .query::<&DisplayTrack>()
        .iter(client.world())
        .any(|value| value.track_id == newcomer.track_id);
    let chunks = client.world().resource::<ReceivedChunks>().0;
    eprintln!("[late-join-diag] preview_samples={count} separate_history={separate_history} chunk_history={chunk_history} initial_history_received=true last_seen_advance_ms={seen_advance} latest_position_received={moved} newcomer_received={newcomer_received} history_chunks_received={chunks} live_updates_received={live_updates_received}/4");
    if chunk_history {
        assert_eq!(chunks, 4 * count.div_ceil(HISTORY_CHUNK_SAMPLES));
    }
    assert!(newcomer_received, "new entity should still arrive");
    assert!(
        moved && seen_advance == 4000 && live_updates_received == 4,
        "existing track froze after initial sync while new aircraft still arrived"
    );
}

/// A fresh second client is a read-only view of the agent's newer state. Compare
/// matching TrackIds so aircraft churn cannot be mistaken for existing updates.
#[test]
#[ignore = "Requires DIAG_AGENT=host:port; opens two read-only agent connections"]
fn compare_existing_client_with_fresh_agent_snapshot() {
    use airjedi_core::TrackId;
    use std::collections::HashMap;
    use std::net::ToSocketAddrs;

    fn snapshot(app: &mut App) -> HashMap<TrackId, (DisplayTrack, usize)> {
        app.world_mut()
            .query::<(&DisplayTrack, Option<&DisplayTrail>)>()
            .iter(app.world())
            .map(|(track, trail)| {
                (
                    track.track_id.clone(),
                    (track.clone(), trail.map_or(0, |t| t.samples.len())),
                )
            })
            .collect()
    }
    fn connect(addr: SocketAddr) -> App {
        let mut app = base_app(false);
        let (client, transport) =
            create_client(app.world().resource::<RepliconChannels>(), addr).unwrap();
        app.insert_resource(client).insert_resource(transport);
        app
    }
    let addr = std::env::var("DIAG_AGENT")
        .expect("set DIAG_AGENT explicitly")
        .to_socket_addrs()
        .unwrap()
        .find(SocketAddr::is_ipv4)
        .unwrap();
    eprintln!("[remote-diag] connecting to {addr}");
    let mut first = connect(addr);
    let warmup = Instant::now();
    while warmup.elapsed() < Duration::from_secs(20) {
        first.update();
        std::thread::sleep(Duration::from_millis(16));
        if warmup.elapsed() > Duration::from_secs(5) && !snapshot(&mut first).is_empty() {
            break;
        }
    }
    let initial = snapshot(&mut first);
    eprintln!(
        "[remote-diag] state={:?} received_tracks={}",
        first.world().resource::<State<ClientState>>().get(),
        initial.len()
    );
    assert!(!initial.is_empty(), "agent supplied no tracks");
    let watch = Instant::now();
    while watch.elapsed() < Duration::from_secs(10) {
        first.update();
        std::thread::sleep(Duration::from_millis(16));
    }
    let mut second = connect(addr);
    let second_warmup = Instant::now();
    while second_warmup.elapsed() < Duration::from_secs(20) {
        first.update();
        second.update();
        std::thread::sleep(Duration::from_millis(16));
        if second_warmup.elapsed() > Duration::from_secs(5) && !snapshot(&mut second).is_empty() {
            break;
        }
    }
    let existing = snapshot(&mut first);
    let fresh = snapshot(&mut second);
    let mut blocked = 0;
    let mut same_tracks = 0;
    for (id, (original, _)) in &initial {
        let (Some((older, count)), Some((newer, _))) = (existing.get(id), fresh.get(id)) else {
            continue;
        };
        same_tracks += 1;
        if older.last_seen == original.last_seen && newer.last_seen > older.last_seen {
            blocked += 1;
            eprintln!("[remote-diag] icao={} callsign={:?} track_id={} preview_samples={} first_client_stuck=true fresh_snapshot_newer_ms={} position_diff={}",
                older.icao, older.callsign, id.0, count,
                (newer.last_seen - older.last_seen).num_milliseconds(),
                (newer.latitude - older.latitude).abs() + (newer.longitude - older.longitude).abs());
        }
    }
    eprintln!("[remote-diag] endpoint={addr} initial_tracks={} existing_tracks={} fresh_tracks={} existing_state={:?} fresh_state={:?} comparable_track_ids={same_tracks} stale_only_on_existing_client={blocked}",
        initial.len(), existing.len(), fresh.len(),
        first.world().resource::<State<ClientState>>().get(),
        second.world().resource::<State<ClientState>>().get());
    assert!(
        same_tracks > 0,
        "no matching track lifetimes across both clients"
    );
    assert_eq!(
        blocked, 0,
        "agent has newer same-track state that existing client never receives"
    );
}
