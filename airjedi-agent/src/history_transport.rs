//! Agent-side selected-history transfer over Replicon's existing connection.

use std::collections::{HashMap, VecDeque};

use airjedi_core::{HistoryOperation, HistoryOperationKind, HistoryRequestId};
use airjedi_fusion::{FusionClock, HistoryRecorder};
use airjedi_net::{
    HistoryCancel, HistoryClientMessage, HistoryOperationMessage, HistoryRejectionReason,
    HistoryRequest, HistoryRequestPriority, HistoryRequestRejection, HistoryServerMessage,
    HistorySnapshotChunk, HistorySnapshotComplete, HISTORY_CHUNK_SAMPLES,
    HISTORY_MAX_BUFFERED_OPERATIONS, HISTORY_MAX_SNAPSHOT_SAMPLES,
};
use bevy::prelude::*;
use bevy_replicon::prelude::{ClientId, ConnectedClient, FromClient, SendTargets, ToClients};

const MAX_ACTIVE_REQUESTS_PER_CLIENT: usize = 4;
const MAX_MESSAGES_PER_TICK: usize = 8;

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
}

#[derive(Resource, Debug)]
pub struct HistoryTransferServer {
    transfers: HashMap<TransferKey, Transfer>,
}

impl Default for HistoryTransferServer {
    fn default() -> Self {
        Self {
            transfers: HashMap::new(),
        }
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
        key.is_some_and(|key| self.transfers.remove(&key).is_some())
    }

    fn cancel(&mut self, client_id: ClientId, cancel: &HistoryCancel) {
        let key = TransferKey {
            client_id,
            request_id: cancel.request_id,
        };
        if self
            .transfers
            .get(&key)
            .is_some_and(|transfer| transfer.request.track_id == cancel.track_id)
        {
            self.transfers.remove(&key);
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
            return Err(rejection(
                &request,
                HistoryRejectionReason::InvalidChunk,
                false,
            ));
        }
        if request.session_id != history.session_id() {
            return Err(rejection(
                &request,
                HistoryRejectionReason::StaleSession,
                false,
            ));
        }
        let Some(mut snapshot) = history.snapshot(&request.track_id, server_time) else {
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
            self.transfers.remove(&key);
        }

        if self.active_for(client_id) >= MAX_ACTIVE_REQUESTS_PER_CLIENT
            && !(request.priority == HistoryRequestPriority::Selected
                && self.remove_background_for(client_id))
        {
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
            },
        );
        Ok(())
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
    let live_clients: std::collections::HashSet<ClientId> =
        clients.iter().map(ClientId::from).collect();
    transfers
        .transfers
        .retain(|key, _| live_clients.contains(&key.client_id));

    let mut keys: Vec<TransferKey> = transfers.transfers.keys().copied().collect();
    keys.sort_by(|left, right| {
        let left_priority = transfers
            .transfers
            .get(left)
            .map(|transfer| transfer.request.priority);
        let right_priority = transfers
            .transfers
            .get(right)
            .map(|transfer| transfer.request.priority);
        priority_rank(left_priority)
            .cmp(&priority_rank(right_priority))
            .then_with(|| {
                left.request_id
                    .0
                    .as_bytes()
                    .cmp(right.request_id.0.as_bytes())
            })
    });

    let mut sent = 0;
    let mut remove = Vec::new();
    for key in keys {
        if sent >= MAX_MESSAGES_PER_TICK {
            break;
        }
        let Some(transfer) = transfers.transfers.get_mut(&key) else {
            continue;
        };

        let Some(operations) = history.operations_since(transfer.operation_cursor) else {
            responses.write(ToClients {
                targets: SendTargets::Single(transfer.client_id),
                message: HistoryServerMessage::Rejected(rejection(
                    &transfer.request,
                    HistoryRejectionReason::SnapshotGap,
                    true,
                )),
            });
            remove.push(key);
            sent += 1;
            continue;
        };
        transfer.operation_cursor = history.history_revision();
        transfer.queued_operations.extend(
            operations
                .into_iter()
                .filter(|operation| operation.track_id == transfer.request.track_id),
        );
        if transfer.queued_operations.len() > HISTORY_MAX_BUFFERED_OPERATIONS {
            responses.write(ToClients {
                targets: SendTargets::Single(transfer.client_id),
                message: HistoryServerMessage::Rejected(rejection(
                    &transfer.request,
                    HistoryRejectionReason::SnapshotGap,
                    true,
                )),
            });
            remove.push(key);
            sent += 1;
            continue;
        }

        let message = if transfer.next_chunk < transfer.chunks.len() {
            let chunk_count = transfer.chunks.len() as u16;
            let chunk = HistorySnapshotChunk {
                session_id: transfer.request.session_id,
                track_id: transfer.request.track_id.clone(),
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
            HistoryServerMessage::SnapshotChunk(chunk)
        } else if !transfer.complete_sent {
            transfer.complete_sent = true;
            HistoryServerMessage::SnapshotComplete(HistorySnapshotComplete {
                session_id: transfer.request.session_id,
                track_id: transfer.request.track_id.clone(),
                request_id: transfer.request.request_id,
                server_time: transfer.server_time,
                sample_cutoff: transfer.sample_cutoff,
                snapshot_revision: transfer.snapshot_revision,
                coverage: transfer.coverage.clone(),
                chunk_count: transfer.chunks.len() as u16,
            })
        } else if let Some(operation) = transfer.queued_operations.pop_front() {
            let remove_after = matches!(operation.kind, HistoryOperationKind::Remove);
            let sample_cutoff = operation.sample_cutoff;
            if remove_after {
                remove.push(key);
            }
            HistoryServerMessage::Operation(HistoryOperationMessage {
                session_id: transfer.request.session_id,
                track_id: transfer.request.track_id.clone(),
                request_id: transfer.request.request_id,
                server_time: clock.now_utc(),
                sample_cutoff,
                operation,
            })
        } else {
            continue;
        };

        responses.write(ToClients {
            targets: SendTargets::Single(transfer.client_id),
            message,
        });
        sent += 1;
    }
    for key in remove {
        transfers.transfers.remove(&key);
    }
}

fn priority_rank(priority: Option<HistoryRequestPriority>) -> u8 {
    match priority {
        Some(HistoryRequestPriority::Selected) => 0,
        Some(HistoryRequestPriority::Background) => 1,
        None => 2,
    }
}
