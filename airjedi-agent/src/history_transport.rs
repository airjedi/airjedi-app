//! Agent-side selected-history transfer over Replicon's existing connection.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use airjedi_core::{
    estimate_history_sample_bytes, HistoryOperation, HistoryOperationKind, HistoryRequestId,
    HistorySessionId,
};
use airjedi_fusion::{FusionClock, HistoryRecorder};
use airjedi_net::{
    HistoryCancel, HistoryClientMessage, HistoryOperationMessage, HistoryRejectionReason,
    HistoryRequest, HistoryRequestPriority, HistoryRequestRejection, HistoryServerMessage,
    HistorySnapshotChunk, HistorySnapshotComplete, HISTORY_CHUNK_SAMPLES,
    HISTORY_MAX_BUFFERED_OPERATIONS, HISTORY_MAX_SNAPSHOT_SAMPLES,
};
use bevy::prelude::*;
use bevy_replicon::prelude::{ClientId, ConnectedClient, FromClient, SendTargets, ToClients};

pub const HISTORY_MAX_ACTIVE_REQUESTS_PER_CLIENT: usize = 4;
pub const HISTORY_MAX_ACTIVE_TRANSFERS: usize = 64;
pub const HISTORY_MAX_SNAPSHOT_SAMPLES_IN_FLIGHT: usize = 8 * HISTORY_MAX_SNAPSHOT_SAMPLES;
pub const HISTORY_MAX_SNAPSHOT_BYTES_IN_FLIGHT: usize = 16 * 1024 * 1024;
pub const HISTORY_MAX_MESSAGES_PER_TICK: usize = 8;
pub const HISTORY_BACKGROUND_MESSAGES_PER_TICK: usize = 1;

#[derive(Debug, Clone, Copy)]
pub struct HistoryTransferConfig {
    pub max_active_requests_per_client: usize,
    pub max_active_transfers: usize,
    pub max_snapshot_samples_in_flight: usize,
    pub max_snapshot_bytes_in_flight: usize,
    pub max_buffered_operations: usize,
    pub max_messages_per_tick: usize,
    pub background_messages_per_tick: usize,
}

impl Default for HistoryTransferConfig {
    fn default() -> Self {
        Self {
            max_active_requests_per_client: HISTORY_MAX_ACTIVE_REQUESTS_PER_CLIENT,
            max_active_transfers: HISTORY_MAX_ACTIVE_TRANSFERS,
            max_snapshot_samples_in_flight: HISTORY_MAX_SNAPSHOT_SAMPLES_IN_FLIGHT,
            max_snapshot_bytes_in_flight: HISTORY_MAX_SNAPSHOT_BYTES_IN_FLIGHT,
            max_buffered_operations: HISTORY_MAX_BUFFERED_OPERATIONS,
            max_messages_per_tick: HISTORY_MAX_MESSAGES_PER_TICK,
            background_messages_per_tick: HISTORY_BACKGROUND_MESSAGES_PER_TICK,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TransferKey {
    client_id: ClientId,
    request_id: HistoryRequestId,
}

#[derive(Debug)]
struct Transfer {
    client_id: ClientId,
    request: HistoryRequest,
    server_time: airjedi_core::Timestamp,
    sample_cutoff: Option<u64>,
    snapshot_revision: u64,
    coverage: airjedi_core::HistoryCoverage,
    chunks: Vec<Vec<airjedi_core::DisplayHistorySample>>,
    next_chunk: usize,
    complete_sent: bool,
    operation_cursor: u64,
    queued_operations: VecDeque<HistoryOperation>,
    snapshot_samples: usize,
    snapshot_bytes: usize,
    started_at: Instant,
    last_served: u64,
}

/// Agent-side history transfer accounting. These are allocation and scheduling
/// counters, not wire protocol state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HistoryTransferDiagnostics {
    pub session_id: Option<HistorySessionId>,
    pub pending_transfers: usize,
    pub active_clients: usize,
    pub selected_transfers: usize,
    pub background_transfers: usize,
    pub snapshot_samples: usize,
    pub snapshot_bytes: usize,
    pub buffered_operations: usize,
    pub retries: usize,
    pub cancellations: usize,
    pub rejected_requests: usize,
    pub revision_gaps: usize,
    pub session_invalidations: usize,
    pub snapshot_truncations: usize,
    pub completed_snapshots: usize,
    pub last_sync_latency_ms: Option<u64>,
}

#[derive(Resource, Debug)]
pub struct HistoryTransferServer {
    transfers: HashMap<TransferKey, Transfer>,
    config: HistoryTransferConfig,
    session_id: Option<HistorySessionId>,
    snapshot_samples: usize,
    snapshot_bytes: usize,
    scheduler_cursor: usize,
    scheduler_sequence: u64,
    diagnostics: HistoryTransferDiagnostics,
}

impl Default for HistoryTransferServer {
    fn default() -> Self {
        Self::new(HistoryTransferConfig::default())
    }
}

impl HistoryTransferServer {
    #[must_use]
    pub fn new(config: HistoryTransferConfig) -> Self {
        Self {
            transfers: HashMap::new(),
            config,
            session_id: None,
            snapshot_samples: 0,
            snapshot_bytes: 0,
            scheduler_cursor: 0,
            scheduler_sequence: 0,
            diagnostics: HistoryTransferDiagnostics::default(),
        }
    }

    #[must_use]
    pub fn diagnostics(&self) -> HistoryTransferDiagnostics {
        let mut diagnostics = self.diagnostics;
        diagnostics.session_id = self.session_id;
        diagnostics.pending_transfers = self.transfers.len();
        diagnostics.active_clients = self
            .transfers
            .values()
            .map(|transfer| transfer.client_id)
            .collect::<HashSet<_>>()
            .len();
        diagnostics.selected_transfers = self
            .transfers
            .values()
            .filter(|transfer| transfer.request.priority == HistoryRequestPriority::Selected)
            .count();
        diagnostics.background_transfers = self
            .transfers
            .values()
            .filter(|transfer| transfer.request.priority == HistoryRequestPriority::Background)
            .count();
        diagnostics.snapshot_samples = self.snapshot_samples;
        diagnostics.snapshot_bytes = self.snapshot_bytes;
        diagnostics.buffered_operations = self
            .transfers
            .values()
            .map(|transfer| transfer.queued_operations.len())
            .sum();
        diagnostics
    }

    fn ensure_session(&mut self, session_id: HistorySessionId) {
        if self.session_id == Some(session_id) {
            return;
        }
        if self.session_id.is_some() {
            self.diagnostics.session_invalidations =
                self.diagnostics.session_invalidations.saturating_add(1);
        }
        self.transfers.clear();
        self.snapshot_samples = 0;
        self.snapshot_bytes = 0;
        self.session_id = Some(session_id);
    }

    fn remove_transfer(&mut self, key: &TransferKey) -> Option<Transfer> {
        let transfer = self.transfers.remove(key)?;
        self.snapshot_samples = self
            .snapshot_samples
            .saturating_sub(transfer.snapshot_samples);
        self.snapshot_bytes = self.snapshot_bytes.saturating_sub(transfer.snapshot_bytes);
        Some(transfer)
    }
}

impl HistoryTransferServer {
    fn active_for(&self, client_id: ClientId) -> usize {
        self.transfers
            .values()
            .filter(|transfer| transfer.client_id == client_id)
            .count()
    }

    fn remove_background_for(&mut self, client_id: ClientId) -> bool {
        let key = self
            .transfers
            .iter()
            .filter(|(_, transfer)| {
                transfer.client_id == client_id
                    && transfer.request.priority == HistoryRequestPriority::Background
            })
            .min_by_key(|(_, transfer)| transfer.request.request_id.0.as_bytes().to_vec())
            .map(|(key, _)| *key);
        key.is_some_and(|key| {
            let removed = self.remove_transfer(&key).is_some();
            if removed {
                self.diagnostics.cancellations = self.diagnostics.cancellations.saturating_add(1);
            }
            removed
        })
    }

    fn cancel(&mut self, client_id: ClientId, cancel: &HistoryCancel) {
        let key = TransferKey {
            client_id,
            request_id: cancel.request_id,
        };
        if self.transfers.get(&key).is_some_and(|transfer| {
            transfer.request.track_id == cancel.track_id
                && transfer.request.session_id == cancel.session_id
        }) {
            self.remove_transfer(&key);
            self.diagnostics.cancellations = self.diagnostics.cancellations.saturating_add(1);
        }
    }

    fn start(
        &mut self,
        client_id: ClientId,
        request: HistoryRequest,
        history: &HistoryRecorder,
        server_time: airjedi_core::Timestamp,
    ) -> Result<(), HistoryRequestRejection> {
        if request.max_samples == 0
            || usize::from(request.max_samples) > HISTORY_MAX_SNAPSHOT_SAMPLES
        {
            self.diagnostics.rejected_requests =
                self.diagnostics.rejected_requests.saturating_add(1);
            return Err(rejection(
                &request,
                HistoryRejectionReason::InvalidChunk,
                false,
            ));
        }
        if request.session_id != history.session_id() {
            self.diagnostics.rejected_requests =
                self.diagnostics.rejected_requests.saturating_add(1);
            return Err(rejection(
                &request,
                HistoryRejectionReason::StaleSession,
                false,
            ));
        }
        let Some(mut snapshot) = history.snapshot(&request.track_id, server_time) else {
            self.diagnostics.retries = self.diagnostics.retries.saturating_add(1);
            return Err(rejection(
                &request,
                HistoryRejectionReason::RemovedTrack,
                true,
            ));
        };

        let existing: Vec<TransferKey> = self
            .transfers
            .keys()
            .filter(|key| {
                key.client_id == client_id
                    && self
                        .transfers
                        .get(key)
                        .is_some_and(|transfer| transfer.request.track_id == request.track_id)
            })
            .copied()
            .collect();
        for key in existing {
            self.remove_transfer(&key);
        }

        if self.transfers.len() >= self.config.max_active_transfers
            || (self.active_for(client_id) >= self.config.max_active_requests_per_client
                && !(request.priority == HistoryRequestPriority::Selected
                    && self.remove_background_for(client_id)))
        {
            self.diagnostics.rejected_requests =
                self.diagnostics.rejected_requests.saturating_add(1);
            self.diagnostics.retries = self.diagnostics.retries.saturating_add(1);
            return Err(rejection(
                &request,
                HistoryRejectionReason::TransferBusy,
                true,
            ));
        }

        let max_samples = usize::from(request.max_samples);
        if snapshot.samples.len() > max_samples {
            let start = snapshot.samples.len() - max_samples;
            snapshot.samples.drain(..start);
            snapshot.sample_cutoff = snapshot.samples.last().map(|sample| sample.sample_sequence);
            self.diagnostics.snapshot_truncations =
                self.diagnostics.snapshot_truncations.saturating_add(1);
        }
        let snapshot_samples = snapshot.samples.len();
        let snapshot_bytes = snapshot
            .samples
            .iter()
            .map(estimate_history_sample_bytes)
            .sum::<usize>();
        if self.snapshot_samples.saturating_add(snapshot_samples)
            > self.config.max_snapshot_samples_in_flight
            || self.snapshot_bytes.saturating_add(snapshot_bytes)
                > self.config.max_snapshot_bytes_in_flight
        {
            self.diagnostics.rejected_requests =
                self.diagnostics.rejected_requests.saturating_add(1);
            self.diagnostics.retries = self.diagnostics.retries.saturating_add(1);
            return Err(rejection(
                &request,
                HistoryRejectionReason::TransferBusy,
                true,
            ));
        }
        let chunks: Vec<Vec<airjedi_core::DisplayHistorySample>> = snapshot
            .samples
            .chunks(HISTORY_CHUNK_SAMPLES)
            .map(|chunk| chunk.to_vec())
            .collect();
        self.transfers.insert(
            TransferKey {
                client_id,
                request_id: request.request_id,
            },
            Transfer {
                client_id,
                request,
                server_time: snapshot.server_time,
                sample_cutoff: snapshot.sample_cutoff,
                snapshot_revision: snapshot.revision,
                coverage: snapshot.coverage,
                chunks,
                next_chunk: 0,
                complete_sent: false,
                operation_cursor: snapshot.revision,
                queued_operations: VecDeque::new(),
                snapshot_samples,
                snapshot_bytes,
                started_at: Instant::now(),
                last_served: 0,
            },
        );
        self.snapshot_samples = self.snapshot_samples.saturating_add(snapshot_samples);
        self.snapshot_bytes = self.snapshot_bytes.saturating_add(snapshot_bytes);
        Ok(())
    }

    fn schedule_keys(&mut self) -> Vec<TransferKey> {
        if self.transfers.is_empty() || self.config.max_messages_per_tick == 0 {
            return Vec::new();
        }
        let mut clients: Vec<ClientId> = self
            .transfers
            .values()
            .map(|transfer| transfer.client_id)
            .collect();
        clients.sort_unstable();
        clients.dedup();
        let start = self.scheduler_cursor % clients.len();
        clients.rotate_left(start);
        self.scheduler_cursor = (start + 1) % clients.len();

        let mut selected = Vec::new();
        let mut background = Vec::new();
        for client_id in clients {
            if let Some(key) = self.next_for_client(client_id, HistoryRequestPriority::Selected) {
                selected.push(key);
            }
            if let Some(key) = self.next_for_client(client_id, HistoryRequestPriority::Background) {
                background.push(key);
            }
        }

        let budget = self.config.max_messages_per_tick;
        let reserved_background = self
            .config
            .background_messages_per_tick
            .min(background.len())
            .min(budget);
        let selected_budget = budget.saturating_sub(reserved_background);
        let mut keys = selected
            .into_iter()
            .take(selected_budget)
            .collect::<Vec<_>>();
        let remaining = budget.saturating_sub(keys.len());
        keys.extend(background.into_iter().take(remaining));
        keys
    }

    fn next_for_client(
        &self,
        client_id: ClientId,
        priority: HistoryRequestPriority,
    ) -> Option<TransferKey> {
        self.transfers
            .iter()
            .filter(|(_, transfer)| {
                transfer.client_id == client_id && transfer.request.priority == priority
            })
            .min_by(|(left_key, left), (right_key, right)| {
                left.last_served.cmp(&right.last_served).then_with(|| {
                    left_key
                        .request_id
                        .0
                        .as_bytes()
                        .cmp(right_key.request_id.0.as_bytes())
                })
            })
            .map(|(key, _)| *key)
    }
}

fn rejection(
    request: &HistoryRequest,
    reason: HistoryRejectionReason,
    retryable: bool,
) -> HistoryRequestRejection {
    HistoryRequestRejection {
        session_id: request.session_id,
        track_id: request.track_id.clone(),
        request_id: request.request_id,
        reason,
        retryable,
    }
}

/// Accept client selection/cancellation messages and capture snapshots at the
/// recorder's current revision watermark.
pub fn receive_history_requests(
    mut requests: MessageReader<FromClient<HistoryClientMessage>>,
    mut transfers: ResMut<HistoryTransferServer>,
    history: Res<HistoryRecorder>,
    clock: Res<FusionClock>,
    mut responses: MessageWriter<ToClients<HistoryServerMessage>>,
) {
    transfers.ensure_session(history.session_id());
    for FromClient { client_id, message } in requests.read() {
        match message {
            HistoryClientMessage::Cancel(cancel) => transfers.cancel(*client_id, cancel),
            HistoryClientMessage::Request(request) => {
                if let Err(rejection) =
                    transfers.start(*client_id, request.clone(), &history, clock.now_utc())
                {
                    responses.write(ToClients {
                        targets: SendTargets::Single(*client_id),
                        message: HistoryServerMessage::Rejected(rejection),
                    });
                }
            }
        }
    }
}

/// Pump one bounded chunk/operation per transfer per update. Current display
/// replication remains on its normal path; history cannot starve it.
pub fn pump_history_transfers(
    mut transfers: ResMut<HistoryTransferServer>,
    history: Res<HistoryRecorder>,
    clock: Res<FusionClock>,
    clients: Query<Entity, With<ConnectedClient>>,
    mut responses: MessageWriter<ToClients<HistoryServerMessage>>,
) {
    transfers.ensure_session(history.session_id());
    let live_clients: std::collections::HashSet<ClientId> =
        clients.iter().map(ClientId::from).collect();
    let stale_clients: Vec<TransferKey> = transfers
        .transfers
        .keys()
        .filter(|key| !live_clients.contains(&key.client_id))
        .copied()
        .collect();
    for key in stale_clients {
        transfers.remove_transfer(&key);
    }

    let keys = transfers.schedule_keys();
    let mut remove = Vec::new();
    for key in keys {
        let revision_gap = transfers.transfers.get(&key).is_some_and(|transfer| {
            history
                .operations_since(transfer.operation_cursor)
                .is_none()
        });
        if revision_gap {
            if let Some(transfer) = transfers.transfers.get(&key) {
                responses.write(ToClients {
                    targets: SendTargets::Single(transfer.client_id),
                    message: HistoryServerMessage::Rejected(rejection(
                        &transfer.request,
                        HistoryRejectionReason::SnapshotGap,
                        true,
                    )),
                });
            }
            transfers.diagnostics.revision_gaps =
                transfers.diagnostics.revision_gaps.saturating_add(1);
            transfers.diagnostics.retries = transfers.diagnostics.retries.saturating_add(1);
            transfers.remove_transfer(&key);
            continue;
        }
        let max_buffered_operations = transfers.config.max_buffered_operations;
        let Some((client_id, message, remove_after, completed_latency)) = (|| {
            let transfer = transfers.transfers.get_mut(&key)?;

            let operations = history
                .operations_since(transfer.operation_cursor)
                .expect("revision gap checked before transfer borrow");
            transfer.operation_cursor = history.history_revision();
            let track_id = transfer.request.track_id.clone();
            transfer.queued_operations.extend(
                operations
                    .into_iter()
                    .filter(|operation| operation.track_id == track_id),
            );
            if transfer.queued_operations.len() > max_buffered_operations {
                return None;
            }

            if transfer.next_chunk < transfer.chunks.len() {
                let chunk_count = transfer.chunks.len() as u16;
                let chunk = HistorySnapshotChunk {
                    session_id: transfer.request.session_id,
                    track_id,
                    request_id: transfer.request.request_id,
                    server_time: transfer.server_time,
                    sample_cutoff: transfer.sample_cutoff,
                    snapshot_revision: transfer.snapshot_revision,
                    coverage: transfer.coverage.clone(),
                    chunk_index: transfer.next_chunk as u16,
                    chunk_count,
                    samples: transfer.chunks[transfer.next_chunk].clone(),
                };
                transfer.next_chunk += 1;
                Some((
                    transfer.client_id,
                    HistoryServerMessage::SnapshotChunk(chunk),
                    false,
                    None,
                ))
            } else if !transfer.complete_sent {
                transfer.complete_sent = true;
                let latency = transfer.started_at.elapsed().as_millis() as u64;
                let complete = HistorySnapshotComplete {
                    session_id: transfer.request.session_id,
                    track_id,
                    request_id: transfer.request.request_id,
                    server_time: transfer.server_time,
                    sample_cutoff: transfer.sample_cutoff,
                    snapshot_revision: transfer.snapshot_revision,
                    coverage: transfer.coverage.clone(),
                    chunk_count: transfer.chunks.len() as u16,
                };
                Some((
                    transfer.client_id,
                    HistoryServerMessage::SnapshotComplete(complete),
                    false,
                    Some(latency),
                ))
            } else if let Some(operation) = transfer.queued_operations.pop_front() {
                let remove_after = matches!(operation.kind, HistoryOperationKind::Remove);
                let operation_message = HistoryOperationMessage {
                    session_id: transfer.request.session_id,
                    track_id,
                    request_id: transfer.request.request_id,
                    server_time: clock.now_utc(),
                    sample_cutoff: operation.sample_cutoff,
                    operation,
                };
                Some((
                    transfer.client_id,
                    HistoryServerMessage::Operation(operation_message),
                    remove_after,
                    None,
                ))
            } else {
                None
            }
        })() else {
            let over_buffer_limit = transfers
                .transfers
                .get(&key)
                .is_some_and(|transfer| transfer.queued_operations.len() > max_buffered_operations);
            if over_buffer_limit {
                if let Some(transfer) = transfers.transfers.get(&key) {
                    responses.write(ToClients {
                        targets: SendTargets::Single(transfer.client_id),
                        message: HistoryServerMessage::Rejected(rejection(
                            &transfer.request,
                            HistoryRejectionReason::SnapshotGap,
                            true,
                        )),
                    });
                }
                transfers.diagnostics.revision_gaps =
                    transfers.diagnostics.revision_gaps.saturating_add(1);
                transfers.diagnostics.retries = transfers.diagnostics.retries.saturating_add(1);
                transfers.remove_transfer(&key);
            }
            continue;
        };

        if let Some(latency) = completed_latency {
            transfers.diagnostics.completed_snapshots =
                transfers.diagnostics.completed_snapshots.saturating_add(1);
            transfers.diagnostics.last_sync_latency_ms = Some(latency);
        }
        responses.write(ToClients {
            targets: SendTargets::Single(client_id),
            message,
        });
        transfers.scheduler_sequence = transfers.scheduler_sequence.saturating_add(1);
        let scheduler_sequence = transfers.scheduler_sequence;
        if let Some(transfer) = transfers.transfers.get_mut(&key) {
            transfer.last_served = scheduler_sequence;
        }
        if remove_after {
            remove.push(key);
        }
    }
    for key in remove {
        transfers.remove_transfer(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use airjedi_core::{
        AltitudeReference, DisplayHistoryInput, DisplayProvenance, HeadingReference,
        HistorySessionId, PositionSource, TrackId, TrackStatus, VerticalRateReference,
    };
    use airjedi_fusion::{HistoryConfig, HistoryRecorder};
    use bevy::prelude::Entity;
    use chrono::{TimeZone, Utc};

    fn input(seconds: i64) -> DisplayHistoryInput {
        DisplayHistoryInput {
            state_time: Utc
                .timestamp_opt(1_700_000_000 + seconds, 0)
                .single()
                .unwrap(),
            latitude: 37.0,
            longitude: -97.0,
            altitude_ft: Some(30_000),
            altitude_reference: AltitudeReference::Barometric,
            ground_speed_kts: Some(400.0),
            heading: Some(90.0),
            heading_reference: HeadingReference::GroundTrack,
            vertical_rate: Some(100),
            vertical_rate_reference: VerticalRateReference::FeetPerMinute,
            position_source: Some(PositionSource::AdsbIcao),
            status: TrackStatus::Confirmed,
            estimated: false,
            provenance: DisplayProvenance::default(),
        }
    }

    fn history(track_count: usize) -> (HistoryRecorder, Vec<TrackId>) {
        let session = HistorySessionId::nil();
        let mut recorder = HistoryRecorder::with_session(HistoryConfig::default(), session);
        let tracks: Vec<TrackId> = (0..track_count).map(|_| TrackId::new()).collect();
        for (index, track_id) in tracks.iter().enumerate() {
            recorder.record_input(track_id, input(index as i64));
        }
        (recorder, tracks)
    }

    fn request(
        session_id: HistorySessionId,
        track_id: TrackId,
        request_id: HistoryRequestId,
        priority: HistoryRequestPriority,
    ) -> HistoryRequest {
        HistoryRequest {
            session_id,
            track_id,
            request_id,
            sample_cutoff: None,
            priority,
            max_samples: HISTORY_MAX_SNAPSHOT_SAMPLES as u16,
        }
    }

    fn client(index: u32) -> ClientId {
        ClientId::from(Entity::from_raw_u32(index).expect("valid test entity"))
    }

    #[test]
    fn snapshot_memory_and_active_transfer_limits_are_enforced() {
        let (history, tracks) = history(2);
        let session = history.session_id();
        let mut server = HistoryTransferServer::new(HistoryTransferConfig {
            max_snapshot_samples_in_flight: 1,
            max_snapshot_bytes_in_flight: usize::MAX,
            ..Default::default()
        });
        server.ensure_session(session);

        server
            .start(
                client(1),
                request(
                    session,
                    tracks[0].clone(),
                    HistoryRequestId::new(),
                    HistoryRequestPriority::Selected,
                ),
                &history,
                input(0).state_time,
            )
            .expect("first transfer");
        let rejected = server
            .start(
                client(2),
                request(
                    session,
                    tracks[1].clone(),
                    HistoryRequestId::new(),
                    HistoryRequestPriority::Selected,
                ),
                &history,
                input(0).state_time,
            )
            .expect_err("snapshot budget");
        assert_eq!(rejected.reason, HistoryRejectionReason::TransferBusy);
        assert_eq!(server.diagnostics().snapshot_samples, 1);
        assert_eq!(server.diagnostics().pending_transfers, 1);
    }

    #[test]
    fn selected_request_supersedes_background_and_stale_cancel_is_ignored() {
        let (history, tracks) = history(2);
        let session = history.session_id();
        let mut server = HistoryTransferServer::default();
        server.ensure_session(session);
        let old = HistoryRequestId::new();
        server
            .start(
                client(1),
                request(
                    session,
                    tracks[0].clone(),
                    old,
                    HistoryRequestPriority::Background,
                ),
                &history,
                input(0).state_time,
            )
            .unwrap();
        let current = HistoryRequestId::new();
        server
            .start(
                client(1),
                request(
                    session,
                    tracks[0].clone(),
                    current,
                    HistoryRequestPriority::Selected,
                ),
                &history,
                input(0).state_time,
            )
            .unwrap();
        assert_eq!(server.diagnostics().pending_transfers, 1);
        server.cancel(
            client(1),
            &HistoryCancel {
                session_id: session,
                track_id: tracks[0].clone(),
                request_id: old,
            },
        );
        assert_eq!(server.diagnostics().pending_transfers, 1);
        server.cancel(
            client(1),
            &HistoryCancel {
                session_id: session,
                track_id: tracks[0].clone(),
                request_id: current,
            },
        );
        assert_eq!(server.diagnostics().pending_transfers, 0);
        assert_eq!(server.diagnostics().snapshot_samples, 0);
    }

    #[test]
    fn selected_scheduling_round_robins_clients() {
        let (history, tracks) = history(2);
        let session = history.session_id();
        let mut server = HistoryTransferServer::new(HistoryTransferConfig {
            max_messages_per_tick: 1,
            background_messages_per_tick: 0,
            ..Default::default()
        });
        server.ensure_session(session);
        server
            .start(
                client(1),
                request(
                    session,
                    tracks[0].clone(),
                    HistoryRequestId::new(),
                    HistoryRequestPriority::Selected,
                ),
                &history,
                input(0).state_time,
            )
            .unwrap();
        server
            .start(
                client(2),
                request(
                    session,
                    tracks[1].clone(),
                    HistoryRequestId::new(),
                    HistoryRequestPriority::Selected,
                ),
                &history,
                input(0).state_time,
            )
            .unwrap();

        let first = server.schedule_keys();
        let second = server.schedule_keys();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_ne!(
            server.transfers.get(&first[0]).unwrap().client_id,
            server.transfers.get(&second[0]).unwrap().client_id
        );
    }

    #[test]
    fn a_new_session_invalidates_all_old_transfer_storage() {
        let (history, tracks) = history(1);
        let session = history.session_id();
        let mut server = HistoryTransferServer::default();
        server.ensure_session(session);
        server
            .start(
                client(1),
                request(
                    session,
                    tracks[0].clone(),
                    HistoryRequestId::new(),
                    HistoryRequestPriority::Selected,
                ),
                &history,
                input(0).state_time,
            )
            .unwrap();

        server.ensure_session(HistorySessionId::new());
        let diagnostics = server.diagnostics();
        assert_eq!(diagnostics.pending_transfers, 0);
        assert_eq!(diagnostics.snapshot_samples, 0);
        assert_eq!(diagnostics.session_invalidations, 1);
    }
}
