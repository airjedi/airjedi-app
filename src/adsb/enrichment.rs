//! Side-channel enrichment lookup for position source (ADS-B vs MLAT).
//!
//! readsb/ultrafeeder's `--net-json-port` serves a persistent NDJSON socket
//! (one JSON object per aircraft per line) carrying a `type` field
//! (`adsb_icao`, `mlat`, etc.) that distinguishes real ADS-B positions from
//! MLAT-derived ones. That distinction cannot be recovered from the Beast
//! wire format readsb re-broadcasts, since it's internal bookkeeping tied to
//! which connector delivered the data, not something re-encoded into the
//! outgoing frames. This module maintains a small ICAO-keyed lookup table
//! from that NDJSON feed, joined against Beast-decoded aircraft elsewhere.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adsb_client::Icao;
use bevy::prelude::*;

/// Position source as classified by readsb's `type` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionSource {
    AdsbIcao,
    AdsbIcaoNt,
    AdsrIcao,
    TisbIcao,
    Adsc,
    Mlat,
    Other,
    Unknown,
}

impl PositionSource {
    fn parse(type_str: &str) -> Self {
        match type_str {
            "adsb_icao" => Self::AdsbIcao,
            "adsb_icao_nt" => Self::AdsbIcaoNt,
            "adsr_icao" => Self::AdsrIcao,
            "tisb_icao" => Self::TisbIcao,
            "adsc" => Self::Adsc,
            "mlat" => Self::Mlat,
            _ => Self::Other,
        }
    }
}

/// Enrichment info for a single aircraft, as of its last NDJSON update.
#[derive(Debug, Clone, Copy)]
pub struct EnrichmentInfo {
    pub source: PositionSource,
    pub nic: Option<u8>,
    pub updated_at: Instant,
}

/// Enrichment entries older than this are treated as stale (aircraft likely
/// gone) rather than returned as current — matches the app's existing
/// aircraft staleness timeout (`constants::ADSB_AIRCRAFT_TIMEOUT_SECS`).
const STALE_AFTER: Duration = Duration::from_secs(180);

/// Shared, thread-safe enrichment lookup table for a single feed.
#[derive(Clone)]
pub struct EnrichmentData {
    map: Arc<Mutex<HashMap<Icao, EnrichmentInfo>>>,
    shutdown: Arc<AtomicBool>,
}

impl EnrichmentData {
    fn new() -> Self {
        Self {
            map: Arc::new(Mutex::new(HashMap::new())),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn try_get(&self, icao: Icao) -> Option<EnrichmentInfo> {
        let info = self.map.try_lock().ok().and_then(|m| m.get(&icao).copied())?;
        (info.updated_at.elapsed() < STALE_AFTER).then_some(info)
    }

    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

/// Spawn a background thread reading readsb's NDJSON `--net-json-port`
/// stream for `feed_name`, keeping `EnrichmentData` up to date by ICAO.
pub fn spawn_enrichment_reader(host: &str, port: u16, feed_name: String) -> EnrichmentData {
    let data = EnrichmentData::new();
    let map = Arc::clone(&data.map);
    let shutdown = Arc::clone(&data.shutdown);
    let address = format!("{host}:{port}");

    std::thread::spawn(move || loop {
        if shutdown.load(Ordering::Acquire) {
            return;
        }

        match TcpStream::connect(&address) {
            Ok(stream) => {
                info!("[{feed_name}] Enrichment stream connected to {address}");
                let reader = BufReader::new(stream);
                for line in reader.lines() {
                    if shutdown.load(Ordering::Acquire) {
                        return;
                    }
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Some((icao, info)) = parse_enrichment_line(&line) {
                        if let Ok(mut map) = map.lock() {
                            map.insert(icao, info);
                        }
                    }
                }
                warn!("[{feed_name}] Enrichment stream disconnected, reconnecting...");
            }
            Err(e) => {
                warn!("[{feed_name}] Enrichment connect to {address} failed: {e}");
            }
        }

        if shutdown.load(Ordering::Acquire) {
            return;
        }
        std::thread::sleep(Duration::from_secs(5));
    });

    data
}

/// A single live enrichment source connection.
struct EnrichmentSourceConnection {
    config: crate::config::EnrichmentSourceConfig,
    data: EnrichmentData,
}

/// Resource managing all configured enrichment sources, independent of the
/// ADS-B feed connections in `crate::adsb::connection`. Lookups are global
/// by ICAO across every configured source, since position-source provenance
/// isn't tied to any one Beast connection.
#[derive(Resource, Default)]
pub struct EnrichmentConnectionManager {
    connections: HashMap<String, EnrichmentSourceConnection>,
    prev_snapshot: Vec<crate::config::EnrichmentSourceConfig>,
}

impl EnrichmentConnectionManager {
    /// Look up the most recent position-source info for an ICAO, across
    /// all configured enrichment sources.
    pub fn lookup(&self, icao: Icao) -> Option<EnrichmentInfo> {
        self.connections
            .values()
            .find_map(|conn| conn.data.try_get(icao))
    }
}

/// Startup system: spawn readers for all enabled enrichment sources.
pub fn setup_enrichment_connections(
    mut commands: Commands,
    app_config: Res<crate::config::AppConfig>,
) {
    let mut manager = EnrichmentConnectionManager::default();

    for source in &app_config.enrichment_sources {
        if source.enabled {
            if let Some(data) = spawn_from_config(source) {
                manager.connections.insert(
                    source.id.clone(),
                    EnrichmentSourceConnection {
                        config: source.clone(),
                        data,
                    },
                );
            }
        }
    }

    manager.prev_snapshot = app_config.enrichment_sources.clone();
    commands.insert_resource(manager);
}

/// Detect enrichment source config changes and spawn/shutdown as needed.
pub fn reconnect_on_enrichment_changes(
    app_config: Res<crate::config::AppConfig>,
    mut manager: ResMut<EnrichmentConnectionManager>,
) {
    if !app_config.is_changed() {
        return;
    }
    if app_config.enrichment_sources == manager.prev_snapshot {
        return;
    }

    let new_sources: HashMap<String, &crate::config::EnrichmentSourceConfig> = app_config
        .enrichment_sources
        .iter()
        .map(|s| (s.id.clone(), s))
        .collect();

    let mut to_remove = Vec::new();
    for (id, conn) in &manager.connections {
        match new_sources.get(id) {
            None => {
                conn.data.request_shutdown();
                to_remove.push(id.clone());
            }
            Some(new_config) => {
                if !new_config.enabled || new_config.endpoint != conn.config.endpoint {
                    conn.data.request_shutdown();
                    to_remove.push(id.clone());
                }
            }
        }
    }
    for id in &to_remove {
        manager.connections.remove(id);
    }

    for source in &app_config.enrichment_sources {
        if source.enabled && !manager.connections.contains_key(&source.id) {
            if let Some(data) = spawn_from_config(source) {
                manager.connections.insert(
                    source.id.clone(),
                    EnrichmentSourceConnection {
                        config: source.clone(),
                        data,
                    },
                );
            }
        }
    }

    manager.prev_snapshot = app_config.enrichment_sources.clone();
}

fn spawn_from_config(source: &crate::config::EnrichmentSourceConfig) -> Option<EnrichmentData> {
    let (host, port_str) = source.endpoint.split_once(':')?;
    let port: u16 = port_str.parse().ok()?;
    if host.is_empty() {
        return None;
    }
    Some(spawn_enrichment_reader(host, port, source.name.clone()))
}

fn parse_enrichment_line(line: &str) -> Option<(Icao, EnrichmentInfo)> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let hex = value.get("hex")?.as_str()?;
    let icao = Icao::from_hex(hex)?;
    let source = value
        .get("type")
        .and_then(|v| v.as_str())
        .map(PositionSource::parse)
        .unwrap_or(PositionSource::Unknown);
    let nic = value.get("nic").and_then(|v| v.as_u64()).map(|n| n as u8);

    Some((
        icao,
        EnrichmentInfo {
            source,
            nic,
            updated_at: Instant::now(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_adsb_line() {
        let line = r#"{"hex":"a37efc","type":"adsb_icao","nic":8,"rc":186}"#;
        let (icao, info) = parse_enrichment_line(line).expect("should parse");
        assert_eq!(icao, Icao::from_hex("a37efc").unwrap());
        assert_eq!(info.source, PositionSource::AdsbIcao);
        assert_eq!(info.nic, Some(8));
    }

    #[test]
    fn parses_mlat_line() {
        let line = r#"{"hex":"ae11f0","type":"mlat"}"#;
        let (icao, info) = parse_enrichment_line(line).expect("should parse");
        assert_eq!(icao, Icao::from_hex("ae11f0").unwrap());
        assert_eq!(info.source, PositionSource::Mlat);
        assert_eq!(info.nic, None);
    }

    #[test]
    fn unknown_type_becomes_other() {
        let line = r#"{"hex":"a00001","type":"some_future_type"}"#;
        let (_, info) = parse_enrichment_line(line).expect("should parse");
        assert_eq!(info.source, PositionSource::Other);
    }

    #[test]
    fn missing_hex_field_fails_to_parse() {
        let line = r#"{"type":"adsb_icao"}"#;
        assert!(parse_enrichment_line(line).is_none());
    }

    #[test]
    fn malformed_json_fails_to_parse() {
        let line = r#"{"hex":"a3"#;
        assert!(parse_enrichment_line(line).is_none());
    }

    #[test]
    fn missing_type_field_becomes_unknown() {
        let line = r#"{"hex":"a00002"}"#;
        let (_, info) = parse_enrichment_line(line).expect("should parse");
        assert_eq!(info.source, PositionSource::Unknown);
    }
}
