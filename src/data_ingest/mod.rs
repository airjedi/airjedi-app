pub mod canonical;
pub mod http_cache;
pub mod pipeline;
pub mod provider;
pub mod providers;
pub mod scheduler;

use std::sync::Arc;

use bevy::prelude::*;
use crossbeam_channel::Receiver;

use canonical::CanonicalRecord;
use provider::FetchContext;

/// Save canonical records to a JSON file in the data directory.
/// Files are named `{provider_name}.json` and overwritten on each fetch.
pub fn save_records_to_file(provider_name: &str, records: &[CanonicalRecord]) {
    let data_dir = crate::paths::data_dir();
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        warn!("Failed to create data dir: {}", e);
        return;
    }

    let path = data_dir.join(format!("{}.json", provider_name));
    match serde_json::to_string_pretty(records) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!("Failed to write {}: {}", path.display(), e);
            } else {
                debug!("Saved {} records to {}", records.len(), path.display());
            }
        }
        Err(e) => {
            warn!("Failed to serialize records for '{}': {}", provider_name, e);
        }
    }
}

/// Message sent from the background scheduler thread to Bevy ECS via crossbeam.
pub struct IngestMessage {
    pub provider_name: String,
    pub records: Vec<CanonicalRecord>,
}

/// Batch of processed records ready for ECS consumption after draining the channel.
#[derive(Debug)]
pub struct ProcessedBatch {
    pub provider_name: String,
    pub records: Vec<CanonicalRecord>,
}

/// Resource holding the receiving end of the crossbeam channel.
/// The sending end lives in the background scheduler thread.
#[derive(Resource)]
pub struct IngestReceiver {
    pub rx: Receiver<IngestMessage>,
}

/// Resource for queuing on-demand fetch requests from the UI.
#[derive(Resource, Default)]
pub struct IngestUiState {
    /// Config keys queued for on-demand fetch by the UI.
    pub pending_fetches: Vec<String>,
}

/// Resource tracking per-provider status for UI display.
#[derive(Resource, Default)]
pub struct IngestStatus {
    pub providers: Vec<ProviderStatusEntry>,
}

/// Status entry for a single provider.
pub struct ProviderStatusEntry {
    pub name: String,
    pub display_name: String,
    pub category: provider::ProviderCategory,
    pub description: String,
    pub config_key: String,
    pub status: provider::ProviderStatus,
}

impl IngestStatus {
    pub fn from_providers(providers: &[Arc<dyn provider::DataProvider>]) -> Self {
        let entries = providers
            .iter()
            .map(|p| {
                let meta = p.metadata();
                ProviderStatusEntry {
                    name: p.name().to_string(),
                    display_name: meta.display_name.to_string(),
                    category: meta.category,
                    description: meta.description.to_string(),
                    config_key: meta.config_key.to_string(),
                    status: provider::ProviderStatus::Idle,
                }
            })
            .collect();
        Self { providers: entries }
    }
}

/// Bevy message fired when weather-related data (METAR, TAF, SIGMET, etc.) arrives.
#[derive(Message)]
pub struct WeatherDataUpdated {
    pub records: Vec<CanonicalRecord>,
}

/// Bevy message fired when navigation data (airports, runways, navaids, etc.) arrives.
#[derive(Message)]
pub struct NavigationDataUpdated {
    pub records: Vec<CanonicalRecord>,
}

/// Bevy message fired when notice data (NOTAMs, TFRs) arrives.
#[derive(Message)]
pub struct NoticeDataUpdated {
    pub records: Vec<CanonicalRecord>,
}

/// Resource holding the sending end of the crossbeam channel so other
/// systems (e.g. on-demand fetch) can also push messages.
#[derive(Resource)]
pub struct IngestSender {
    pub tx: crossbeam_channel::Sender<IngestMessage>,
}

/// Message requesting an on-demand fetch for a specific provider config key.
#[derive(Message)]
pub struct OnDemandFetchRequest {
    pub config_key: String,
}

pub struct DataIngestPlugin;

impl Plugin for DataIngestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<IngestUiState>()
            .add_message::<OnDemandFetchRequest>()
            .add_message::<WeatherDataUpdated>()
            .add_message::<NavigationDataUpdated>()
            .add_message::<NoticeDataUpdated>()
            .add_systems(Startup, start_ingest_scheduler)
            .add_systems(Update, (drain_ingest_channel, handle_on_demand_fetch));
    }
}

/// Build the list of data providers based on the current DataIngestConfig.
fn build_providers(
    config: &crate::config::DataIngestConfig,
) -> Vec<Arc<dyn provider::DataProvider>> {
    let mut providers: Vec<Arc<dyn provider::DataProvider>> = vec![];

    if config.metar.enabled {
        providers.push(Arc::new(providers::aviation_weather::MetarProvider));
        providers.push(Arc::new(providers::aviation_weather::TafProvider));
        providers.push(Arc::new(providers::aviation_weather::SigmetProvider));
        providers.push(Arc::new(providers::aviation_weather::AirmetProvider));
        providers.push(Arc::new(providers::aviation_weather::PirepProvider));
    } else if config.taf.enabled {
        providers.push(Arc::new(providers::aviation_weather::TafProvider));
    }

    if config.ourairports.enabled {
        providers.push(Arc::new(providers::our_airports::AirportsProvider));
        providers.push(Arc::new(providers::our_airports::RunwaysProvider));
        providers.push(Arc::new(providers::our_airports::NavaidsProvider));
    }

    if config.faa_nasr.enabled {
        providers.push(Arc::new(providers::faa_nasr::FaaNasrProvider::new()));
    }

    if config.openaip.enabled {
        providers.push(Arc::new(providers::openaip::OpenAipProvider::new()));
    }

    if config.notam.enabled {
        providers.push(Arc::new(providers::notams::NotamProvider));
    }

    if config.tfr.enabled {
        providers.push(Arc::new(providers::tfrs::TfrProvider));
    }

    if config.faa_airspace.enabled {
        providers.push(Arc::new(
            providers::faa_adds_airspace::FaaClassAirspaceProvider,
        ));
    }

    providers
}

/// Startup system: creates the crossbeam channel, builds the initial
/// FetchContext from AppConfig, and spawns the background scheduler thread.
fn start_ingest_scheduler(mut commands: Commands, app_config: Res<crate::config::AppConfig>) {
    let (tx, rx) = crossbeam_channel::unbounded();

    // Use AppConfig for initial context since MapState may not be inserted yet
    // (startup system ordering is not guaranteed).
    let context = FetchContext {
        center_latitude: app_config.map.default_latitude,
        center_longitude: app_config.map.default_longitude,
        radius_nm: crate::constants::AVIATION_FEATURE_RADIUS_NM,
    };

    let providers = build_providers(&app_config.data_ingest);

    commands.insert_resource(IngestStatus::from_providers(&providers));

    scheduler::spawn_scheduler(providers, context, tx.clone());

    commands.insert_resource(IngestReceiver { rx });
    commands.insert_resource(IngestSender { tx });
}

/// System that drains the crossbeam channel each frame and dispatches
/// domain-specific messages so other systems can react to new data.
fn drain_ingest_channel(
    receiver: Option<Res<IngestReceiver>>,
    mut weather_events: MessageWriter<WeatherDataUpdated>,
    mut nav_events: MessageWriter<NavigationDataUpdated>,
    mut notice_events: MessageWriter<NoticeDataUpdated>,
) {
    let Some(receiver) = receiver else { return };

    while let Ok(msg) = receiver.rx.try_recv() {
        let mut weather = Vec::new();
        let mut nav = Vec::new();
        let mut notice = Vec::new();

        for record in msg.records {
            match &record {
                CanonicalRecord::Metar(_)
                | CanonicalRecord::Taf(_)
                | CanonicalRecord::Sigmet(_)
                | CanonicalRecord::Airmet(_)
                | CanonicalRecord::Pirep(_) => weather.push(record),

                CanonicalRecord::Airport(_)
                | CanonicalRecord::Runway(_)
                | CanonicalRecord::Navaid(_)
                | CanonicalRecord::Airway(_)
                | CanonicalRecord::Airspace(_)
                | CanonicalRecord::Frequency(_) => nav.push(record),

                CanonicalRecord::Notam(_) | CanonicalRecord::Tfr(_) => notice.push(record),
            }
        }

        if !weather.is_empty() {
            weather_events.write(WeatherDataUpdated { records: weather });
        }
        if !nav.is_empty() {
            nav_events.write(NavigationDataUpdated { records: nav });
        }
        if !notice.is_empty() {
            notice_events.write(NoticeDataUpdated { records: notice });
        }
    }
}

/// Build providers for a specific config key (for on-demand fetch).
fn build_providers_for_key(key: &str) -> Vec<Arc<dyn provider::DataProvider>> {
    match key {
        "metar" => vec![
            Arc::new(providers::aviation_weather::MetarProvider),
            Arc::new(providers::aviation_weather::TafProvider),
            Arc::new(providers::aviation_weather::SigmetProvider),
            Arc::new(providers::aviation_weather::AirmetProvider),
            Arc::new(providers::aviation_weather::PirepProvider),
        ],
        "taf" => vec![Arc::new(providers::aviation_weather::TafProvider)],
        "ourairports" => vec![
            Arc::new(providers::our_airports::AirportsProvider),
            Arc::new(providers::our_airports::RunwaysProvider),
            Arc::new(providers::our_airports::NavaidsProvider),
        ],
        "faa_nasr" => vec![Arc::new(providers::faa_nasr::FaaNasrProvider::new())],
        "openaip" => vec![Arc::new(providers::openaip::OpenAipProvider::new())],
        "notam" => vec![Arc::new(providers::notams::NotamProvider)],
        "tfr" => vec![Arc::new(providers::tfrs::TfrProvider)],
        "faa_airspace" => vec![Arc::new(
            providers::faa_adds_airspace::FaaClassAirspaceProvider,
        )],
        _ => vec![],
    }
}

/// System that handles on-demand fetch requests by spawning background threads.
fn handle_on_demand_fetch(
    mut ui_state: ResMut<IngestUiState>,
    sender: Option<Res<IngestSender>>,
    app_config: Res<crate::config::AppConfig>,
) {
    if ui_state.pending_fetches.is_empty() {
        return;
    }
    let Some(sender) = sender else { return };

    let pending = std::mem::take(&mut ui_state.pending_fetches);
    for config_key in pending {
        let providers = build_providers_for_key(&config_key);
        if providers.is_empty() {
            warn!("On-demand fetch: unknown config key '{}'", config_key);
            continue;
        }

        let tx = sender.tx.clone();
        let context = provider::FetchContext {
            center_latitude: app_config.map.default_latitude,
            center_longitude: app_config.map.default_longitude,
            radius_nm: crate::constants::AVIATION_FEATURE_RADIUS_NM,
        };

        let key = config_key;
        std::thread::Builder::new()
            .name(format!("ingest-ondemand-{}", key))
            .spawn(move || {
                info!("On-demand fetch started for '{}'", key);
                for provider in &providers {
                    let name = provider.name().to_string();
                    let raw = match provider.fetch(&context) {
                        Ok(r) => r,
                        Err(e) => {
                            warn!("On-demand fetch '{}' failed: {}", name, e);
                            continue;
                        }
                    };

                    let pipeline_data = pipeline::PipelineData {
                        raw_bytes: Some(raw.data),
                        records: Vec::new(),
                        metadata: std::collections::HashMap::new(),
                    };

                    let stages = provider.pipeline_stages();
                    let result = match pipeline::run_pipeline(&stages, pipeline_data) {
                        Ok(data) => data,
                        Err(e) => {
                            warn!("On-demand pipeline '{}' failed: {}", name, e);
                            continue;
                        }
                    };

                    if !result.records.is_empty() {
                        info!(
                            "On-demand '{}': produced {} records",
                            name,
                            result.records.len()
                        );
                        save_records_to_file(&name, &result.records);
                        let _ = tx.send(IngestMessage {
                            provider_name: name,
                            records: result.records,
                        });
                    }
                }
                info!("On-demand fetch completed for '{}'", key);
            })
            .ok();
    }
}

#[cfg(test)]
mod fixture_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_ingest::canonical::*;
    use crate::data_ingest::pipeline::*;
    use crate::data_ingest::provider::*;

    /// Mock provider that returns canned METAR + airport data.
    struct MockProvider;

    impl DataProvider for MockProvider {
        fn name(&self) -> &str {
            "mock_provider"
        }
        fn schedule(&self) -> &str {
            "0 */5 * * * *"
        }

        fn fetch(&self, _ctx: &FetchContext) -> Result<RawFetchResult, ProviderError> {
            Ok(RawFetchResult {
                data: b"mock data".to_vec(),
                content_type: Some("text/plain".into()),
                source: "mock".into(),
            })
        }

        fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>> {
            vec![Box::new(MockParseStage)]
        }

        fn metadata(&self) -> ProviderMeta {
            ProviderMeta {
                display_name: "Mock",
                category: ProviderCategory::Weather,
                description: "Mock provider for testing",
                config_key: "metar",
            }
        }
    }

    struct MockParseStage;

    impl PipelineStage for MockParseStage {
        fn name(&self) -> &str {
            "mock_parse"
        }
        fn phase(&self) -> PipelinePhase {
            PipelinePhase::Parse
        }

        fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
            data.records.push(CanonicalRecord::Metar(MetarReport {
                icao: "KICT".into(),
                raw_text: "KICT 031200Z 27010KT 10SM FEW050 15/10 A2992".into(),
                observation_time: chrono::Utc::now(),
                wind_direction_deg: Some(270),
                wind_speed_kt: Some(10),
                wind_gust_kt: None,
                visibility_sm: Some(10.0),
                ceiling_ft: None,
                temperature_c: Some(15.0),
                dewpoint_c: Some(10.0),
                altimeter_inhg: Some(29.92),
                flight_category: "VFR".into(),
                fetched_at: chrono::Utc::now(),
            }));
            data.records.push(CanonicalRecord::Airport(AirportInfo {
                ident: "KICT".into(),
                name: "Wichita Dwight D Eisenhower National".into(),
                airport_type: "large_airport".into(),
                latitude: 37.6499,
                longitude: -97.4331,
                elevation_ft: Some(1333),
                iso_country: "US".into(),
                iso_region: "US-KS".into(),
                municipality: Some("Wichita".into()),
                scheduled_service: true,
                iata_code: Some("ICT".into()),
                fetched_at: chrono::Utc::now(),
            }));
            Ok(())
        }
    }

    #[test]
    fn end_to_end_mock_provider_through_channel() {
        let (tx, rx) = crossbeam_channel::unbounded::<IngestMessage>();

        let provider = std::sync::Arc::new(MockProvider);
        let ctx = FetchContext {
            center_latitude: 37.6872,
            center_longitude: -97.3301,
            radius_nm: 250.0,
        };

        // Simulate what the scheduler does: fetch → pipeline → send
        let raw = provider.fetch(&ctx).expect("fetch should succeed");
        let pipeline_data = PipelineData {
            raw_bytes: Some(raw.data),
            ..Default::default()
        };
        let stages = provider.pipeline_stages();
        let result = run_pipeline(&stages, pipeline_data).expect("pipeline should succeed");

        assert_eq!(result.records.len(), 2, "pipeline should produce 2 records");

        tx.send(IngestMessage {
            provider_name: provider.name().to_string(),
            records: result.records,
        })
        .expect("send should succeed");

        // Receive and classify (simulates drain_ingest_channel)
        let msg = rx.try_recv().expect("should receive message");
        assert_eq!(msg.provider_name, "mock_provider");
        assert_eq!(msg.records.len(), 2);

        let mut weather_count = 0;
        let mut nav_count = 0;
        for record in &msg.records {
            match record {
                CanonicalRecord::Metar(_) => weather_count += 1,
                CanonicalRecord::Airport(_) => nav_count += 1,
                _ => {}
            }
        }
        assert_eq!(weather_count, 1, "should have 1 weather record");
        assert_eq!(nav_count, 1, "should have 1 navigation record");
    }

    #[test]
    fn channel_empty_when_no_messages() {
        let (_tx, rx) = crossbeam_channel::unbounded::<IngestMessage>();
        assert!(rx.try_recv().is_err(), "empty channel should return error");
    }
}
