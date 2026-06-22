use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crossbeam_channel::{Receiver, Sender, TrySendError};
use futures::StreamExt;
use crate::prelude_imports::*;
use crate::classification::TargetClassification;
use crate::config::FusionConfig;
use crate::filter::TrackerState;
use crate::sensor::{FusionTier, SensorObservation};
use crate::systems::ObservationBuffer;
use crate::track::{Track, TrackQuality, TrackStatus};
use super::messages::{self, FusedTrackMessage};
use super::NatsTransportConfig;

#[derive(Resource)]
pub struct NatsTransport {
    publish_tx: Sender<Vec<u8>>,
    subscribe_rx: Receiver<SensorObservation>,
    connected: Arc<AtomicBool>,
    config: NatsTransportConfig,
}

impl NatsTransport {
    pub fn start(config: NatsTransportConfig) -> Self {
        let (pub_tx, pub_rx) = crossbeam_channel::bounded::<Vec<u8>>(1000);
        let (sub_tx, sub_rx) = crossbeam_channel::bounded::<SensorObservation>(1000);
        let connected = Arc::new(AtomicBool::new(false));
        let connected_clone = connected.clone();

        let server_url = config.server_url.clone();
        let node_id = config.node_id.clone();
        let tier = config.tier;
        let publish_subject = format!(
            "fusion.{}.{}.tracks",
            tier_str(tier),
            node_id,
        );
        let subscriptions = config.subscriptions.clone();
        let js_config = config.jetstream.clone();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    bevy_log::warn!("Failed to create tokio runtime for NATS: {e}");
                    return;
                }
            };

            rt.block_on(async move {
                let client = match async_nats::connect(&server_url).await {
                    Ok(c) => c,
                    Err(e) => {
                        bevy_log::warn!(
                            "NATS connection failed: {e}. Running in offline mode."
                        );
                        return;
                    }
                };

                connected_clone.store(true, Ordering::Release);
                bevy_log::info!("NATS connected to {server_url}");

                let jetstream = async_nats::jetstream::new(client.clone());

                // Create or get stream (idempotent)
                if let Err(e) = jetstream
                    .get_or_create_stream(async_nats::jetstream::stream::Config {
                        name: js_config.stream_name.clone(),
                        subjects: vec!["fusion.>".to_string()],
                        max_age: js_config.max_age,
                        max_bytes: js_config.max_bytes as i64,
                        ..Default::default()
                    })
                    .await
                {
                    bevy_log::warn!("JetStream stream creation failed: {e}");
                }

                // Publisher task
                let pub_client = client.clone();
                let pub_subj = publish_subject.clone();
                let pub_connected = connected_clone.clone();
                tokio::spawn(async move {
                    while let Ok(bytes) = pub_rx.recv() {
                        if let Err(e) = pub_client
                            .publish(pub_subj.clone(), bytes.into())
                            .await
                        {
                            bevy_log::warn!("NATS publish error: {e}");
                            pub_connected.store(false, Ordering::Release);
                        }
                    }
                });

                // Subscriber tasks
                for sub_config in &subscriptions {
                    let sub_tx = sub_tx.clone();
                    let mut subscriber =
                        match client.subscribe(sub_config.subject.clone()).await {
                            Ok(s) => s,
                            Err(e) => {
                                bevy_log::warn!(
                                    "NATS subscribe error for {}: {e}",
                                    sub_config.subject
                                );
                                continue;
                            }
                        };

                    tokio::spawn(async move {
                        while let Some(msg) = subscriber.next().await {
                            match bincode::deserialize::<FusedTrackMessage>(
                                msg.payload.as_ref(),
                            ) {
                                Ok(update) => {
                                    let obs = messages::message_to_observation(
                                        &update,
                                        chrono::Utc::now(),
                                    );
                                    if let Err(TrySendError::Full(_)) =
                                        sub_tx.try_send(obs)
                                    {
                                        bevy_log::warn!(
                                            "NATS subscribe buffer full, dropping message"
                                        );
                                    }
                                }
                                Err(e) => {
                                    bevy_log::warn!(
                                        "Failed to decode NATS message: {e}"
                                    );
                                }
                            }
                        }
                    });
                }

                // Keep alive
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                }
            });
        });

        Self {
            publish_tx: pub_tx,
            subscribe_rx: sub_rx,
            connected,
            config,
        }
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }
}

fn tier_str(tier: FusionTier) -> &'static str {
    match tier {
        FusionTier::Edge => "edge",
        FusionTier::Regional => "regional",
        FusionTier::Global => "global",
    }
}

// --- Bevy Systems ---

pub fn nats_publish_system(
    transport: Option<Res<NatsTransport>>,
    config: Res<FusionConfig>,
    tracks: Query<(&Track, &TrackerState, &TrackQuality, &TargetClassification)>,
) {
    let transport = match transport {
        Some(ref t) if t.is_connected() => t,
        _ => return, // no transport or not connected - silently skip
    };

    for (track, tracker, quality, classification) in &tracks {
        if quality.status == TrackStatus::Lost {
            continue;
        }
        let msg = messages::track_to_message(
            track,
            tracker,
            quality,
            classification,
            &config.node_id,
            config.tier,
        );
        if let Ok(bytes) = bincode::serialize(&msg) {
            let _ = transport.publish_tx.try_send(bytes);
        }
    }
}

pub fn nats_subscribe_drain_system(
    transport: Option<Res<NatsTransport>>,
    mut buffer: ResMut<ObservationBuffer>,
) {
    let transport = match transport {
        Some(ref t) => t,
        None => return, // no transport configured - silently skip
    };

    // Drain all available messages without blocking
    while let Ok(obs) = transport.subscribe_rx.try_recv() {
        buffer.observations.push(obs);
    }
}
