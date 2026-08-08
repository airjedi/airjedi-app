use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bytes::Bytes;
use log::{error, info, warn};

use super::{Transport, TransportEvent};

const RETRY_INITIAL_SECS: u64 = 5;
const RETRY_MAX_SECS: u64 = 30;

/// Configuration for an SDR data source.
#[derive(Debug, Clone)]
pub struct SdrConfig {
    pub device_index: usize,
    pub center_freq: u32,
    pub sample_rate: u32,
    pub gain: SdrGain,
    pub bias_tee: bool,
}

/// Gain setting for an SDR device.
#[derive(Debug, Clone, Copy)]
pub enum SdrGain {
    Auto,
    /// Manual gain in dB (e.g., 49.6).
    Manual(f64),
}

impl Default for SdrGain {
    fn default() -> Self {
        Self::Auto
    }
}

impl Default for SdrConfig {
    fn default() -> Self {
        Self {
            device_index: 0,
            center_freq: 1_090_000_000,
            sample_rate: 2_400_000,
            gain: SdrGain::Auto,
            bias_tee: false,
        }
    }
}

/// Transport that receives Mode-S frames from an SDR device via rs1090 demodulation.
///
/// Internally creates a desperado `IqAsyncSource`, feeds it through rs1090's
/// 1090 MHz demodulator, and emits individual Mode-S frames as `TransportEvent::Data`.
///
/// Each `Data` event uses a minimal internal encoding:
/// `[4 bytes f32 LE signal_level][7 or 14 bytes Mode-S frame]`
/// Paired with `SdrFramer` which decodes this format.
///
/// Automatically retries device open on failure or USB disconnect, with
/// capped exponential backoff (5s, 10s, 20s, 30s, 30s, ...).
pub struct SdrTransport {
    rx: tokio::sync::mpsc::Receiver<TransportEvent>,
    shutdown: Arc<AtomicBool>,
}

impl std::fmt::Debug for SdrTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SdrTransport").finish_non_exhaustive()
    }
}

impl SdrTransport {
    pub fn new(config: SdrConfig) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(512);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let gain_param = match config.gain {
            SdrGain::Auto => None,
            SdrGain::Manual(db) => Some((db * 10.0) as i32),
        };
        let device_index = config.device_index;
        let center_freq = config.center_freq;
        let sample_rate = config.sample_rate;

        tokio::spawn(async move {
            let mut retry_secs = RETRY_INITIAL_SECS;

            loop {
                if shutdown_clone.load(Ordering::Acquire) {
                    return;
                }

                info!(
                    "SDR: opening device {} at {} MHz, rate {} MHz",
                    device_index,
                    center_freq as f64 / 1e6,
                    sample_rate as f64 / 1e6,
                );

                let source = match desperado::IqAsyncSource::from_rtlsdr(
                    device_index,
                    center_freq,
                    sample_rate,
                    gain_param,
                )
                .await
                {
                    Ok(s) => {
                        retry_secs = RETRY_INITIAL_SECS;
                        s
                    }
                    Err(e) => {
                        error!("SDR: failed to open device: {}", e);
                        let _ = tx
                            .send(TransportEvent::Error(format!(
                                "SDR device error: {}",
                                e
                            )))
                            .await;

                        warn!(
                            "SDR: retrying in {} seconds",
                            retry_secs
                        );
                        tokio::time::sleep(Duration::from_secs(retry_secs)).await;
                        retry_secs = (retry_secs * 2).min(RETRY_MAX_SECS);
                        continue;
                    }
                };

                info!("SDR: device opened, starting demodulation");
                if tx.send(TransportEvent::Connected).await.is_err() {
                    return;
                }

                let (frame_tx, mut frame_rx) =
                    tokio::sync::mpsc::channel::<rs1090::decode::TimedMessage>(512);

                let rate = f64::from(sample_rate);
                tokio::spawn(async move {
                    rs1090::source::iqread::receiver(
                        frame_tx, source, 0, rate, None,
                    )
                    .await;
                });

                while let Some(msg) = frame_rx.recv().await {
                    if shutdown_clone.load(Ordering::Acquire) {
                        return;
                    }
                    let rssi = msg.metadata.first().and_then(|m| m.rssi);
                    let mut buf = Vec::with_capacity(4 + msg.frame.len());
                    buf.extend_from_slice(
                        &rssi.unwrap_or(f32::NAN).to_le_bytes(),
                    );
                    buf.extend_from_slice(&msg.frame);
                    if tx
                        .send(TransportEvent::Data(Bytes::from(buf)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }

                info!("SDR: device stream ended, will retry");
                if tx.send(TransportEvent::Disconnected).await.is_err() {
                    return;
                }

                warn!("SDR: retrying in {} seconds", retry_secs);
                tokio::time::sleep(Duration::from_secs(retry_secs)).await;
                retry_secs = (retry_secs * 2).min(RETRY_MAX_SECS);
            }
        });

        Self { rx, shutdown }
    }
}

#[async_trait::async_trait]
impl Transport for SdrTransport {
    async fn recv(&mut self) -> Option<TransportEvent> {
        self.rx.recv().await
    }

    fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    fn current_address(&self) -> String {
        "rtlsdr://local".to_string()
    }
}
