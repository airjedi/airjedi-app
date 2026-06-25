use bevy::prelude::*;
use crossbeam_channel::Sender;
use std::sync::{Arc, Mutex};
use tokio_cron_scheduler::{Job, JobScheduler};

use super::canonical::CanonicalRecord;
use super::pipeline::{run_pipeline, PipelineData};
use super::provider::{DataProvider, FetchContext};
use super::IngestMessage;

/// A registered provider with its shared state.
struct RegisteredProvider {
    provider: Arc<dyn DataProvider>,
    context: Arc<Mutex<FetchContext>>,
}

/// Spawn a background thread running a tokio multi-thread runtime with
/// cron-scheduled provider execution. Results are sent to Bevy ECS via
/// the crossbeam `tx` channel.
///
/// IMPORTANT: Must use `new_multi_thread()` — `new_current_thread()` deadlocks
/// with tokio-cron-scheduler.
pub fn spawn_scheduler(
    providers: Vec<Arc<dyn DataProvider>>,
    context: FetchContext,
    tx: Sender<IngestMessage>,
) {
    if providers.is_empty() {
        info!("Data ingest scheduler: no providers registered, skipping");
        return;
    }

    let shared_context = Arc::new(Mutex::new(context));

    std::thread::Builder::new()
        .name("data-ingest-scheduler".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("failed to create tokio runtime for data ingest scheduler");

            // Run all providers immediately BEFORE entering the async runtime.
            // Providers use reqwest::blocking which creates its own internal
            // tokio runtime — calling it inside rt.block_on() causes a panic
            // ("Cannot drop a runtime in a context where blocking is not allowed").
            for provider in &providers {
                run_provider(provider, &shared_context, &tx);
            }

            rt.block_on(async move {
                let sched = JobScheduler::new()
                    .await
                    .expect("failed to create job scheduler");

                for provider in &providers {
                    let p = Arc::clone(provider);
                    let ctx = Arc::clone(&shared_context);
                    let sender = tx.clone();
                    let name = provider.name().to_string();
                    let schedule = provider.schedule().to_string();

                    // Then schedule for periodic execution
                    let job = Job::new_async(schedule.as_str(), move |_uuid, _lock| {
                        let p = Arc::clone(&p);
                        let ctx = Arc::clone(&ctx);
                        let sender = sender.clone();
                        Box::pin(async move {
                            // Run fetch + pipeline on a blocking thread to avoid
                            // starving the scheduler's async executor
                            tokio::task::spawn_blocking(move || {
                                run_provider(&p, &ctx, &sender);
                            })
                            .await
                            .ok();
                        })
                    });

                    match job {
                        Ok(job) => {
                            if let Err(e) = sched.add(job).await {
                                error!("Failed to schedule provider '{}': {}", name, e);
                            } else {
                                info!("Scheduled provider '{}' with cron '{}'", name, schedule);
                            }
                        }
                        Err(e) => {
                            error!("Failed to create job for provider '{}': {}", name, e);
                        }
                    }
                }

                if let Err(e) = sched.start().await {
                    error!("Failed to start data ingest scheduler: {}", e);
                    return;
                }

                info!(
                    "Data ingest scheduler started with {} providers",
                    providers.len()
                );

                // Keep the scheduler alive
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
                }
            });
        })
        .expect("failed to spawn data ingest scheduler thread");
}

/// Execute a single provider: fetch → pipeline → send results.
fn run_provider(
    provider: &Arc<dyn DataProvider>,
    context: &Arc<Mutex<FetchContext>>,
    tx: &Sender<IngestMessage>,
) {
    let name = provider.name().to_string();

    let ctx = match context.lock() {
        Ok(c) => c.clone(),
        Err(e) => {
            error!("Provider '{}': failed to lock context: {}", name, e);
            return;
        }
    };

    // Fetch raw data
    let raw_result = match provider.fetch(&ctx) {
        Ok(r) => r,
        Err(e) => {
            warn!("Provider '{}' fetch failed: {}", name, e);
            return;
        }
    };

    // Build pipeline data from raw fetch result
    let pipeline_data = PipelineData {
        raw_bytes: Some(raw_result.data),
        records: Vec::new(),
        metadata: std::collections::HashMap::new(),
    };

    // Run pipeline stages
    let stages = provider.pipeline_stages();
    let result = match run_pipeline(&stages, pipeline_data) {
        Ok(data) => data,
        Err(e) => {
            warn!("Provider '{}' pipeline failed: {}", name, e);
            return;
        }
    };

    if result.records.is_empty() {
        debug!("Provider '{}': pipeline produced no records", name);
        return;
    }

    info!(
        "Provider '{}': produced {} records",
        name,
        result.records.len()
    );

    // Cache to disk
    super::save_records_to_file(&name, &result.records);

    // Send to Bevy ECS
    let _ = tx.send(IngestMessage {
        provider_name: name,
        records: result.records,
    });
}
