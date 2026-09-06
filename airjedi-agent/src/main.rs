//! Headless AirJedi fusion agent (design-b Phase 4).
//!
//! Runs the full ingest -> fusion -> display-projection pipeline with no
//! renderer (`MinimalPlugins`), then replicates the resulting `DisplayTrack`s to
//! thin clients over `bevy_replicon` + renet. The projection is the *same*
//! [`airjedi_fusion::derive_display_track`] the desktop app uses in fat mode, so
//! a thin client sees identical display state.
//!
//! Modes:
//! - `airjedi-agent`                    - server, fixture replay (default port 5599).
//! - `airjedi-agent --feed host:port`   - server, live BEAST feed instead of fixture.
//! - `airjedi-agent --port 6000`        - choose the listen port.
//! - `airjedi-agent --selftest`         - run the pipeline headless, print the
//!                                        projected DisplayTracks, and exit 0.
//! - `airjedi-agent --probe [--connect host:port] [--secs N]`
//!                                      - verification client: print replicated
//!                                        DisplayTracks from a running agent.
//! - `--fixture <dir>`                  - override the capture directory.

mod ingest;
mod live_ingest;
mod replicate_tracks;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use airjedi_core::{DisplayTrack, PositionSource};
use airjedi_fusion::sensor::SensorKind;
use airjedi_fusion::systems::{FusionSet, ObservationBuffer};
use airjedi_fusion::{FusionConfig, FusionPlugin};
use airjedi_net::{create_client, create_server, register_replicated, DEFAULT_PORT};
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy_replicon::prelude::{RepliconChannels, RepliconPlugins};
use bevy_replicon_renet::RepliconRenetPlugins;
use chrono::Utc;

use crate::ingest::{default_fixture_dir, load_scene, make_observation, Contact, Scene};
use crate::replicate_tracks::{sync_replicated_tracks, MlatSet, TrackEntityMap};

/// Where the agent gets observations from.
enum Ingest {
    /// Deterministic replay of the correlated capture (demo / offline).
    Fixture(Scene),
    /// A live BEAST feed at `host:port`.
    Live(String),
}

/// Chosen listen port, so the `Startup` server-setup system can read it.
#[derive(Resource)]
struct ServerPort(u16);

/// Throttle for draining the live-feed snapshot into fusion observations.
#[derive(Resource)]
struct LiveFeedTimer(Timer);

/// The replay scene as a resource, re-emitted on a timer to keep tracks alive.
#[derive(Resource)]
struct ReplayFeed {
    adsb: Vec<Contact>,
    mlat: Vec<Contact>,
    timer: Timer,
    primed: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // `--probe` is a standalone verification client: connect to a running agent
    // and print the DisplayTracks it replicates. No fixture needed.
    if args.iter().any(|a| a == "--probe") {
        let connect = flag_value(&args, "--connect")
            .and_then(|s| s.parse::<SocketAddr>().ok())
            .unwrap_or_else(|| SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT)));
        let secs = flag_value(&args, "--secs")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(10);
        run_probe(connect, secs);
    }

    let selftest = args.iter().any(|a| a == "--selftest");
    let port = flag_value(&args, "--port")
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);

    // A live BEAST feed replaces fixture replay and needs no capture files.
    if let Some(feed_addr) = flag_value(&args, "--feed") {
        eprintln!("[agent] live BEAST ingest from {feed_addr}");
        run_server(Ingest::Live(feed_addr), port);
    }

    let dir = flag_value(&args, "--fixture")
        .map(PathBuf::from)
        .unwrap_or_else(default_fixture_dir);

    let scene = match load_scene(&dir) {
        Ok(scene) => scene,
        Err(e) => {
            eprintln!("[agent] failed to load fixture scene from {}: {e}", dir.display());
            std::process::exit(1);
        }
    };
    eprintln!(
        "[agent] scene loaded: {} adsb, {} mlat contacts ({} mlat-tagged icaos)",
        scene.adsb.len(),
        scene.mlat.len(),
        scene.mlat_set.len()
    );

    if selftest {
        run_selftest(scene);
    } else {
        run_server(Ingest::Fixture(scene), port);
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Add the fusion engine + the agent-side projection to `app`.
fn add_fusion_and_projection(app: &mut App, mlat_set: std::collections::HashSet<u32>) {
    if !app.world().contains_resource::<FusionConfig>() {
        app.insert_resource(FusionConfig::default());
    }
    app.add_plugins(FusionPlugin)
        .init_resource::<TrackEntityMap>()
        .insert_resource(MlatSet(mlat_set))
        .add_systems(Update, sync_replicated_tracks.after(FusionSet::Lifecycle));
}

/// Push the whole scene into the observation buffer at the current instant.
fn push_scene(app: &mut App, scene: &Scene, include_adsb: bool) {
    let now = Utc::now();
    let mut buffer = app.world_mut().resource_mut::<ObservationBuffer>();
    if include_adsb {
        for c in &scene.adsb {
            buffer
                .observations
                .push(make_observation(c, SensorKind::AdsbReceiver, now));
        }
    }
    for c in &scene.mlat {
        buffer
            .observations
            .push(make_observation(c, SensorKind::MlatNetwork, now));
    }
}

/// Headless pipeline check: feed the scene, project, print the DisplayTracks,
/// assert the MLAT source tag survives, and exit.
fn run_selftest(scene: Scene) -> ! {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    add_fusion_and_projection(&mut app, scene.mlat_set.clone());

    // Confirm passes (ADS-B + MLAT), then a few MLAT-only passes, mirroring the
    // tier-4 test's warm-up so tracks reach a stable status.
    for _ in 0..3 {
        push_scene(&mut app, &scene, true);
        app.update();
    }
    for _ in 0..4 {
        push_scene(&mut app, &scene, false);
        app.update();
    }
    for _ in 0..3 {
        app.update();
    }

    let world = app.world_mut();
    let mut q = world.query::<&DisplayTrack>();
    let tracks: Vec<DisplayTrack> = q.iter(world).cloned().collect();
    let total = tracks.len();
    let mlat: Vec<&DisplayTrack> = tracks
        .iter()
        .filter(|t| t.position_source == Some(PositionSource::Mlat))
        .collect();

    println!("[selftest] projected DisplayTracks: {total}");
    println!("[selftest] mlat-tagged: {}", mlat.len());
    for t in &mlat {
        println!(
            "  mlat icao={} status={:?} filter={} lat={:.3} lon={:.3} alt_ft={:?}",
            t.icao, t.status, t.filter_type, t.latitude, t.longitude, t.altitude_ft
        );
    }

    assert!(total >= 20, "expected many display tracks, got {total}");
    assert!(
        !mlat.is_empty(),
        "expected at least one MLAT-tagged display track"
    );
    for t in &tracks {
        assert!(t.latitude.is_finite() && t.longitude.is_finite());
        assert!(!t.icao.is_empty());
    }
    println!("[selftest] OK");
    std::process::exit(0);
}

/// Run the replicating server: fusion agent + renet transport, ticked ~60 Hz.
fn run_server(ingest: Ingest, port: u16) -> ! {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(16))),
        StatesPlugin,
        bevy::log::LogPlugin::default(),
        RepliconPlugins,
        RepliconRenetPlugins,
    ));

    register_replicated(&mut app);

    // MLAT source tagging is a property of the correlated fixture; a raw BEAST
    // feed carries no MLAT type, so live tracks tag as ADS-B.
    let mlat_set = match &ingest {
        Ingest::Fixture(scene) => scene.mlat_set.clone(),
        Ingest::Live(_) => Default::default(),
    };
    add_fusion_and_projection(&mut app, mlat_set);

    match ingest {
        Ingest::Fixture(scene) => {
            app.insert_resource(ReplayFeed {
                adsb: scene.adsb,
                mlat: scene.mlat,
                timer: Timer::from_seconds(1.0, TimerMode::Repeating),
                primed: false,
            })
            .add_systems(Update, feed_observations.before(FusionSet::Drain));
            info!("airjedi-agent starting on udp/{port} (fixture replay)");
        }
        Ingest::Live(addr) => {
            let shared = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            live_ingest::spawn_beast_reader(addr.clone(), shared.clone());
            app.insert_resource(live_ingest::LiveAircraft(shared))
                .insert_resource(LiveFeedTimer(Timer::from_seconds(
                    0.5,
                    TimerMode::Repeating,
                )))
                .add_systems(Update, feed_live_observations.before(FusionSet::Drain));
            info!("airjedi-agent starting on udp/{port} (live BEAST feed {addr})");
        }
    }

    app.insert_resource(ServerPort(port))
        .add_systems(Startup, setup_server);

    app.run();
    unreachable!("ScheduleRunnerPlugin loop never returns")
}

/// Drain the live-feed snapshot into fusion observations on a timer.
fn feed_live_observations(
    time: Res<Time>,
    mut timer: ResMut<LiveFeedTimer>,
    live: Res<live_ingest::LiveAircraft>,
    mut buffer: ResMut<ObservationBuffer>,
) {
    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }
    let now = Utc::now();
    if let Ok(contacts) = live.0.lock() {
        for c in contacts.iter() {
            buffer
                .observations
                .push(make_observation(c, SensorKind::AdsbReceiver, now));
        }
    }
}

/// The agent address the probe client connects to.
#[derive(Resource)]
struct ProbeTarget(SocketAddr);

/// Run a thin verification client: connect to a running agent, then print the
/// replicated DisplayTracks once per second for `secs` seconds, and exit.
fn run_probe(connect: SocketAddr, secs: u64) -> ! {
    eprintln!("[probe] connecting to agent at {connect} for {secs}s");
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        bevy::log::LogPlugin::default(),
        RepliconPlugins,
        RepliconRenetPlugins,
    ));
    register_replicated(&mut app);
    app.insert_resource(ProbeTarget(connect));
    app.add_systems(Startup, setup_probe_client);
    // Manual update loop: do run()'s implicit finish/cleanup ourselves.
    app.finish();
    app.cleanup();

    let start = Instant::now();
    let mut next_report = Duration::from_secs(1);
    let deadline = Duration::from_secs(secs);
    loop {
        app.update();
        std::thread::sleep(Duration::from_millis(16));
        if start.elapsed() >= next_report {
            report_probe(&mut app);
            next_report += Duration::from_secs(1);
        }
        if start.elapsed() >= deadline {
            break;
        }
    }
    println!("[probe] final snapshot:");
    report_probe(&mut app);
    std::process::exit(0);
}

fn setup_probe_client(
    mut commands: Commands,
    channels: Res<RepliconChannels>,
    target: Res<ProbeTarget>,
) {
    match create_client(&channels, target.0) {
        Ok((client, transport)) => {
            commands.insert_resource(client);
            commands.insert_resource(transport);
            info!("probe client connecting to {}", target.0);
        }
        Err(e) => error!("failed to start probe client: {e}"),
    }
}

fn report_probe(app: &mut App) {
    let world = app.world_mut();
    let mut q = world.query::<&DisplayTrack>();
    let tracks: Vec<DisplayTrack> = q.iter(world).cloned().collect();
    let mlat = tracks
        .iter()
        .filter(|t| t.position_source == Some(PositionSource::Mlat))
        .count();
    println!(
        "[probe] received DisplayTracks: {} (mlat-tagged: {mlat})",
        tracks.len()
    );
    for t in tracks
        .iter()
        .filter(|t| t.position_source == Some(PositionSource::Mlat))
    {
        println!(
            "  mlat icao={} status={:?} filter={} lat={:.3} lon={:.3}",
            t.icao, t.status, t.filter_type, t.latitude, t.longitude
        );
    }
}

fn setup_server(mut commands: Commands, channels: Res<RepliconChannels>, port: Res<ServerPort>) {
    match create_server(&channels, port.0) {
        Ok((server, transport)) => {
            commands.insert_resource(server);
            commands.insert_resource(transport);
            info!("airjedi-agent listening for thin clients on udp/{}", port.0);
        }
        Err(e) => {
            error!("failed to start server transport on udp/{}: {e}", port.0);
        }
    }
}

fn feed_observations(
    time: Res<Time>,
    mut feed: ResMut<ReplayFeed>,
    mut buffer: ResMut<ObservationBuffer>,
) {
    feed.timer.tick(time.delta());
    let should_feed = !feed.primed || feed.timer.just_finished();
    if !should_feed {
        return;
    }
    feed.primed = true;

    let now = Utc::now();
    for c in &feed.adsb {
        buffer
            .observations
            .push(make_observation(c, SensorKind::AdsbReceiver, now));
    }
    for c in &feed.mlat {
        buffer
            .observations
            .push(make_observation(c, SensorKind::MlatNetwork, now));
    }
}
