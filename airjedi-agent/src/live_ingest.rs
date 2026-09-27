//! Live BEAST ingest for the agent.
//!
//! A background thread connects to a dump1090/readsb BEAST feed (port 30005),
//! decodes it with the same adsb-client path the fixture replay uses, and keeps
//! a shared snapshot of positioned contacts. A Bevy system (in `main`) drains
//! that snapshot into fusion observations. Reconnects automatically on error so
//! the agent survives feed hiccups.

use std::io::Read;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adsb_client::{BeastFramer, Decoder, Framer, Rs1090Decoder, TrackerConfig};
use bevy::prelude::*;
use chrono::Utc;

use crate::ingest::{Contact, RECEIVER_LAT, RECEIVER_LON};

/// Latest positioned contacts from the live feed, updated by the reader thread.
#[derive(Resource, Clone, Default)]
pub struct LiveAircraft(pub Arc<Mutex<Vec<Contact>>>);

/// Spawn the reader thread. `addr` is `host:port` for a BEAST feed.
pub fn spawn_beast_reader(addr: String, shared: Arc<Mutex<Vec<Contact>>>) {
    std::thread::spawn(move || {
        loop {
            match TcpStream::connect(&addr) {
                Ok(stream) => {
                    info!("live ingest connected to {addr}");
                    run_reader(stream, &shared);
                    warn!("live ingest disconnected from {addr}; reconnecting in 3s");
                }
                Err(e) => {
                    warn!("live ingest connect to {addr} failed: {e}; retrying in 3s");
                }
            }
            // Drop stale contacts while disconnected so the client doesn't show ghosts.
            if let Ok(mut guard) = shared.lock() {
                guard.clear();
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

/// Read+decode until the stream ends or errors, refreshing `shared` ~2 Hz.
fn run_reader(mut stream: TcpStream, shared: &Arc<Mutex<Vec<Contact>>>) {
    let mut framer = BeastFramer::new();
    let mut decoder = Rs1090Decoder::new();
    decoder.set_reference_position(RECEIVER_LAT, RECEIVER_LON);
    let mut tracker = adsb_client::AircraftTracker::new(TrackerConfig {
        center: Some((RECEIVER_LAT, RECEIVER_LON)),
        max_distance_miles: 450.0,
        aircraft_timeout_secs: 120,
        position_history_secs: 120,
        event_channel_capacity: 1024,
    });

    let mut buf = [0u8; 8192];
    let mut last_snapshot = Instant::now();
    let mut last_log = Instant::now();
    let mut last_cleanup = Instant::now();
    let mut frames: u64 = 0;
    let mut msgs: u64 = 0;
    let mut bytes: u64 = 0;
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => return, // clean EOF
            Ok(n) => n,
            Err(_) => return, // treat any error as disconnect -> reconnect
        };
        bytes += n as u64;
        framer.feed(&buf[..n]);
        while let Some(frame) = framer.next_frame() {
            frames += 1;
            let receipt_time = Utc::now();
            for msg in decoder.decode_at(&frame, receipt_time) {
                msgs += 1;
                tracker.process_decoded_message(msg);
            }
        }
        if last_cleanup.elapsed() >= Duration::from_secs(5) {
            tracker.cleanup_stale();
            last_cleanup = Instant::now();
        }
        if last_log.elapsed() >= Duration::from_secs(30) {
            info!(
                "live ingest diag: bytes={bytes} frames={frames} msgs={msgs} aircraft={} positioned={} position_history={}",
                tracker.len(),
                tracker.positioned_len(),
                tracker.position_history_len(),
            );
            last_log = Instant::now();
        }
        if last_snapshot.elapsed() >= Duration::from_millis(500) {
            let contacts: Vec<Contact> = tracker
                .get_aircraft()
                .iter()
                .filter_map(|a| {
                    let (lat, lon) = (a.latitude?, a.longitude?);
                    Some(Contact {
                        icao: a.icao.0,
                        lat,
                        lon,
                        alt_ft: a.altitude,
                        track: a.track,
                        vel_kts: a.velocity,
                        observation_time: a
                            .position_observation_time
                            .unwrap_or(a.last_observation_time),
                        receipt_time: a.last_seen,
                        time_source: a.position_time_source.unwrap_or(a.last_time_source),
                        observation_id: a.position_observation_id.unwrap_or(a.last_observation_id),
                    })
                })
                .collect();
            if let Ok(mut guard) = shared.lock() {
                *guard = contacts;
            }
            last_snapshot = Instant::now();
        }
    }
}
