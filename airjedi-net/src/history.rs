//! Selected-track history protocol and client-side read model.
//!
//! The protocol uses the existing Replicon/Renet connection. A request captures
//! one server-side snapshot at a revision watermark; later operations carry the
//! same request identity and are buffered until completion. The client read
//! model is deliberately independent of Bevy visuals so trails and charts can
//! consume one stable source in later tickets.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use airjedi_core::{
    estimate_history_operation_bytes, estimate_history_sample_bytes, DisplayHistorySample,
    DisplayTrail, HistoryCoverage, HistoryOperation, HistoryOperationKind, HistoryRequestId,
    HistorySessionId, Timestamp, TrackId,
};
use bevy::prelude::{Message, Resource};
use bevy_replicon::prelude::{Channel, ClientMessageAppExt, ServerMessageAppExt};
use serde::{Deserialize, Serialize};

pub const HISTORY_CHUNK_SAMPLES: usize = 64;
pub const HISTORY_MAX_SNAPSHOT_SAMPLES: usize = 901;
pub const HISTORY_MAX_CLIENT_SAMPLES: usize = 4 * HISTORY_MAX_SNAPSHOT_SAMPLES;
pub const HISTORY_MAX_CLIENT_TRACKS: usize = 1_000;
pub const HISTORY_MAX_CLIENT_REQUESTS: usize = 4;
pub const HISTORY_MAX_BUFFERED_OPERATIONS: usize = 256;
pub const HISTORY_MAX_CHUNKS: u16 =
    ((HISTORY_MAX_SNAPSHOT_SAMPLES + HISTORY_CHUNK_SAMPLES - 1) / HISTORY_CHUNK_SAMPLES) as u16;

/// Selected history is always serviced before background fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryRequestPriority {
    Selected,
    Background,
}

/// Client-to-server history control message.
#[derive(Message, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HistoryClientMessage {
    Request(HistoryRequest),
    Cancel(HistoryCancel),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRequest {
    pub session_id: HistorySessionId,
    pub track_id: TrackId,
    pub request_id: HistoryRequestId,
    /// The latest sequence already known by the client. The server still sends
    /// a complete bounded snapshot; the cutoff makes resume/debug semantics
    /// explicit and the client merges by stable sample identity.
    pub sample_cutoff: Option<u64>,
    pub priority: HistoryRequestPriority,
    pub max_samples: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryCancel {
    pub session_id: HistorySessionId,
    pub track_id: TrackId,
    pub request_id: HistoryRequestId,
}

/// Server-to-client history stream. All variants carry session, track, and
/// request identity so stale or superseded responses cannot hydrate a new
/// lifetime.
#[derive(Message, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HistoryServerMessage {
    SnapshotChunk(HistorySnapshotChunk),
    SnapshotComplete(HistorySnapshotComplete),
    Operation(HistoryOperationMessage),
    Rejected(HistoryRequestRejection),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistorySnapshotChunk {
    pub session_id: HistorySessionId,
    pub track_id: TrackId,
    pub request_id: HistoryRequestId,
    pub server_time: Timestamp,
    pub sample_cutoff: Option<u64>,
    pub snapshot_revision: u64,
    pub coverage: HistoryCoverage,
    pub chunk_index: u16,
    pub chunk_count: u16,
    pub samples: Vec<DisplayHistorySample>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistorySnapshotComplete {
    pub session_id: HistorySessionId,
    pub track_id: TrackId,
    pub request_id: HistoryRequestId,
    pub server_time: Timestamp,
    pub sample_cutoff: Option<u64>,
    pub snapshot_revision: u64,
    pub coverage: HistoryCoverage,
    pub chunk_count: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryOperationMessage {
    pub session_id: HistorySessionId,
    pub track_id: TrackId,
    pub request_id: HistoryRequestId,
    pub server_time: Timestamp,
    pub sample_cutoff: Option<u64>,
    pub operation: HistoryOperation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRequestRejection {
    pub session_id: HistorySessionId,
    pub track_id: TrackId,
    pub request_id: HistoryRequestId,
    pub reason: HistoryRejectionReason,
    pub retryable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryRejectionReason {
    StaleSession,
    RemovedTrack,
    SupersededRequest,
    CancelledRequest,
    SnapshotGap,
    TransferBusy,
    InvalidChunk,
}

/// Register the two typed messages on Replicon's existing ordered channels.
pub fn register_history_messages(app: &mut bevy::prelude::App) {
    app.add_client_message::<HistoryClientMessage>(Channel::Ordered)
        .add_server_message::<HistoryServerMessage>(Channel::Ordered)
        .make_message_independent::<HistoryServerMessage>();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryLoadingState {
    Preview,
    Loading,
    Partial,
    Complete,
    RetryableError,
}

/// Stable read model for one track. The vector is sorted by stable sample
/// sequence and contains no duplicate identities.
#[derive(Debug, Clone)]
pub struct ClientTrackHistory {
    pub session_id: HistorySessionId,
    pub track_id: TrackId,
    pub server_time: Timestamp,
    pub history_revision: u64,
    pub coverage: HistoryCoverage,
    pub loading: HistoryLoadingState,
    pub transfer: Option<HistoryTransferProgress>,
    pub request_id: Option<HistoryRequestId>,
    pub samples: Vec<DisplayHistorySample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HistoryTransferProgress {
    pub received_chunks: u16,
    pub total_chunks: u16,
    pub received_samples: usize,
    pub expected_samples: Option<usize>,
}

impl ClientTrackHistory {
    pub fn sample(&self, sequence: u64) -> Option<&DisplayHistorySample> {
        self.samples
            .binary_search_by_key(&sequence, |sample| sample.sample_sequence)
            .ok()
            .map(|index| &self.samples[index])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryApplyResult {
    Applied,
    IgnoredStale,
    Rejected,
}

#[derive(Debug, Clone, Default)]
pub struct HistoryRequestPlan {
    pub cancel: Option<HistoryCancel>,
    pub request: Option<HistoryRequest>,
}

/// Client-side synchronization accounting for diagnostics and load tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClientHistoryDiagnostics {
    pub session_id: Option<HistorySessionId>,
    pub tracks: usize,
    pub retained_samples: usize,
    pub retained_bytes: usize,
    pub active_requests: usize,
    pub buffered_operations: usize,
    pub buffered_operation_bytes: usize,
    pub retries: usize,
    pub cancellations: usize,
    pub stale_responses: usize,
    pub rejected_responses: usize,
    pub coverage_truncations: usize,
    pub last_sync_latency_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct SnapshotAssembly {
    session_id: HistorySessionId,
    track_id: TrackId,
    request_id: HistoryRequestId,
    server_time: Timestamp,
    sample_cutoff: Option<u64>,
    snapshot_revision: u64,
    coverage: HistoryCoverage,
    chunk_count: u16,
    chunks: BTreeMap<u16, Vec<DisplayHistorySample>>,
    sample_sequences: HashSet<u64>,
    sample_count: usize,
}

#[derive(Debug, Clone)]
struct ActiveRequest {
    request: HistoryRequest,
    assembly: Option<SnapshotAssembly>,
    buffered_operations: BTreeMap<u64, HistoryOperationMessage>,
    completed: bool,
    last_applied_revision: u64,
    started_at: Instant,
}

/// Client-owned bounded cache and one stable read interface for trails/charts.
#[derive(Resource, Debug)]
pub struct ClientHistoryStore {
    session_id: Option<HistorySessionId>,
    histories: HashMap<TrackId, ClientTrackHistory>,
    active: HashMap<TrackId, ActiveRequest>,
    max_samples: usize,
    max_buffered_operations: usize,
    retries: usize,
    cancellations: usize,
    stale_responses: usize,
    rejected_responses: usize,
    last_sync_latency_ms: Option<u64>,
    retry_after: HashMap<TrackId, Instant>,
    retry_attempts: HashMap<TrackId, u32>,
    permanent_errors: HashSet<TrackId>,
}

impl Default for ClientHistoryStore {
    fn default() -> Self {
        Self::new(HISTORY_MAX_CLIENT_SAMPLES, HISTORY_MAX_BUFFERED_OPERATIONS)
    }
}

impl ClientHistoryStore {
    #[must_use]
    pub fn new(max_samples: usize, max_buffered_operations: usize) -> Self {
        Self {
            session_id: None,
            histories: HashMap::new(),
            active: HashMap::new(),
            max_samples: max_samples.max(1),
            max_buffered_operations: max_buffered_operations.max(1),
            retries: 0,
            cancellations: 0,
            stale_responses: 0,
            rejected_responses: 0,
            last_sync_latency_ms: None,
            retry_after: HashMap::new(),
            retry_attempts: HashMap::new(),
            permanent_errors: HashSet::new(),
        }
    }

    #[must_use]
    pub fn session_id(&self) -> Option<HistorySessionId> {
        self.session_id
    }

    #[must_use]
    pub fn track(&self, track_id: &TrackId) -> Option<&ClientTrackHistory> {
        self.histories.get(track_id)
    }

    #[must_use]
    pub fn track_count(&self) -> usize {
        self.histories.len()
    }

    #[must_use]
    pub fn total_sample_count(&self) -> usize {
        self.histories
            .values()
            .map(|history| history.samples.len())
            .sum()
    }

    #[must_use]
    pub fn active_request_count(&self) -> usize {
        self.active.len()
    }

    #[must_use]
    pub fn active_request_count_for(&self, priority: HistoryRequestPriority) -> usize {
        self.active
            .values()
            .filter(|active| active.request.priority == priority)
            .count()
    }

    #[must_use]
    pub fn active_request_priority(&self, track_id: &TrackId) -> Option<HistoryRequestPriority> {
        self.active
            .get(track_id)
            .map(|active| active.request.priority)
    }

    #[must_use]
    pub fn diagnostics(&self) -> ClientHistoryDiagnostics {
        ClientHistoryDiagnostics {
            session_id: self.session_id,
            tracks: self.histories.len(),
            retained_samples: self.total_sample_count(),
            retained_bytes: self
                .histories
                .values()
                .flat_map(|history| history.samples.iter())
                .map(estimate_history_sample_bytes)
                .sum(),
            active_requests: self.active.len(),
            buffered_operations: self
                .active
                .values()
                .map(|active| active.buffered_operations.len())
                .sum(),
            buffered_operation_bytes: self
                .active
                .values()
                .flat_map(|active| active.buffered_operations.values())
                .map(|message| estimate_history_operation_bytes(&message.operation))
                .sum(),
            retries: self.retries,
            cancellations: self.cancellations,
            stale_responses: self.stale_responses,
            rejected_responses: self.rejected_responses,
            coverage_truncations: self
                .histories
                .values()
                .filter(|history| history.coverage.truncation_reason.is_some())
                .count(),
            last_sync_latency_ms: self.last_sync_latency_ms,
        }
    }

    pub fn install_preview(&mut self, preview: &DisplayTrail) -> HistoryApplyResult {
        if !self.accept_session(preview.session_id) {
            return HistoryApplyResult::Rejected;
        }

        let has_active = self.active.contains_key(&preview.track_id);
        let history = self.history_entry(
            preview.session_id,
            preview.track_id.clone(),
            preview.server_time,
            preview.coverage.clone(),
        );
        if preview.history_revision < history.history_revision {
            return HistoryApplyResult::IgnoredStale;
        }

        history.server_time = preview.server_time;
        history.history_revision = preview.history_revision;
        history.coverage = preview.coverage.clone();
        for sample in &preview.samples {
            upsert_sample(&mut history.samples, sample.clone());
        }
        if !has_active {
            history.loading = HistoryLoadingState::Preview;
            history.request_id = None;
        }
        self.enforce_sample_bound();
        self.enforce_track_bound();
        HistoryApplyResult::Applied
    }

    /// Start or reprioritize one request. A background request is superseded
    /// when the same track becomes selected; the caller sends the returned
    /// cancellation before the new request.
    pub fn prepare_request(
        &mut self,
        track_id: &TrackId,
        priority: HistoryRequestPriority,
    ) -> HistoryRequestPlan {
        let Some(session_id) = self.session_id else {
            return HistoryRequestPlan::default();
        };
        let Some((history_loading, sample_cutoff)) = self.histories.get(track_id).map(|history| {
            (
                history.loading,
                history.samples.last().map(|sample| sample.sample_sequence),
            )
        }) else {
            return HistoryRequestPlan::default();
        };

        if self.permanent_errors.contains(track_id)
            || self
                .retry_after
                .get(track_id)
                .is_some_and(|retry_after| Instant::now() < *retry_after)
        {
            return HistoryRequestPlan::default();
        }

        if let Some(active) = self.active.get(track_id) {
            if active.request.priority == priority
                && !matches!(history_loading, HistoryLoadingState::RetryableError)
            {
                return HistoryRequestPlan::default();
            }
        }

        let mut cancel = self.active.remove(track_id).map(|active| HistoryCancel {
            session_id: active.request.session_id,
            track_id: active.request.track_id,
            request_id: active.request.request_id,
        });
        if cancel.is_none() && self.active.len() >= HISTORY_MAX_CLIENT_REQUESTS {
            if priority == HistoryRequestPriority::Selected {
                let background_track = self
                    .active
                    .iter()
                    .filter(|(_, active)| {
                        active.request.priority == HistoryRequestPriority::Background
                    })
                    .min_by_key(|(_, active)| active.started_at)
                    .map(|(track_id, _)| track_id.clone());
                if let Some(background_track) = background_track {
                    cancel = self.cancel_request(&background_track);
                }
            }
            if cancel.is_none() && self.active.len() >= HISTORY_MAX_CLIENT_REQUESTS {
                return HistoryRequestPlan::default();
            }
        }
        if matches!(history_loading, HistoryLoadingState::RetryableError) {
            self.retries = self.retries.saturating_add(1);
        }
        let request = HistoryRequest {
            session_id,
            track_id: track_id.clone(),
            request_id: HistoryRequestId::new(),
            sample_cutoff,
            priority,
            max_samples: HISTORY_MAX_SNAPSHOT_SAMPLES as u16,
        };
        self.retry_after.remove(track_id);
        self.active.insert(
            track_id.clone(),
            ActiveRequest {
                request: request.clone(),
                assembly: None,
                buffered_operations: BTreeMap::new(),
                completed: false,
                last_applied_revision: 0,
                started_at: Instant::now(),
            },
        );
        let history = self
            .histories
            .get_mut(track_id)
            .expect("history still exists");
        history.loading = HistoryLoadingState::Loading;
        history.request_id = Some(request.request_id);
        HistoryRequestPlan {
            cancel,
            request: Some(request),
        }
    }

    /// Build a fresh request after a retryable transfer or hydration error.
    /// Existing valid samples remain in the cache while the new request gets a
    /// new identity and is checked against the current server session.
    pub fn retry(
        &mut self,
        track_id: &TrackId,
        priority: HistoryRequestPriority,
    ) -> HistoryRequestPlan {
        self.retry_after.remove(track_id);
        self.permanent_errors.remove(track_id);
        self.prepare_request(track_id, priority)
    }

    pub fn cancel_request(&mut self, track_id: &TrackId) -> Option<HistoryCancel> {
        let active = self.active.remove(track_id)?;
        self.cancellations = self.cancellations.saturating_add(1);
        if let Some(history) = self.histories.get_mut(track_id) {
            history.request_id = None;
            if !active.completed {
                history.loading = HistoryLoadingState::Preview;
            }
        }
        Some(HistoryCancel {
            session_id: active.request.session_id,
            track_id: active.request.track_id,
            request_id: active.request.request_id,
        })
    }

    /// Drop in-flight requests after a transport disconnect while retaining
    /// already received samples. A reconnect can merge a fresh preview and
    /// issue a new request without losing the disconnected interval.
    pub fn invalidate_active_requests(&mut self) {
        let active = std::mem::take(&mut self.active);
        for (track_id, request) in active {
            self.retry_after.remove(&track_id);
            if let Some(history) = self.histories.get_mut(&track_id) {
                history.request_id = None;
                if !request.completed {
                    history.loading = HistoryLoadingState::Preview;
                }
            }
        }
    }

    /// Background hydration is a cache fill, not a permanent live subscription.
    /// Release completed background requests so the bounded scheduler can move
    /// on to the next visible track. A selected request remains subscribed for
    /// correction and prune operations.
    pub fn release_completed_background_requests(&mut self) -> Vec<HistoryCancel> {
        let completed: Vec<TrackId> = self
            .active
            .iter()
            .filter(|(_, active)| {
                active.completed && active.request.priority == HistoryRequestPriority::Background
            })
            .map(|(track_id, _)| track_id.clone())
            .collect();
        let mut cancellations = Vec::with_capacity(completed.len());
        for track_id in completed {
            if let Some(active) = self.active.remove(&track_id) {
                self.cancellations = self.cancellations.saturating_add(1);
                self.retry_after.remove(&track_id);
                if let Some(history) = self.histories.get_mut(&track_id) {
                    history.request_id = None;
                }
                cancellations.push(HistoryCancel {
                    session_id: active.request.session_id,
                    track_id: active.request.track_id,
                    request_id: active.request.request_id,
                });
            }
        }
        cancellations
    }

    /// Remove histories and return cancellations for tracks no longer present
    /// in the current display set.
    pub fn retain_tracks(&mut self, live_tracks: &HashSet<TrackId>) -> Vec<HistoryCancel> {
        let stale: Vec<TrackId> = self
            .histories
            .keys()
            .filter(|track_id| !live_tracks.contains(*track_id))
            .cloned()
            .collect();
        let mut cancellations = Vec::new();
        for track_id in stale {
            if let Some(cancel) = self.cancel_request(&track_id) {
                cancellations.push(cancel);
            }
            self.histories.remove(&track_id);
            self.retry_after.remove(&track_id);
            self.retry_attempts.remove(&track_id);
            self.permanent_errors.remove(&track_id);
        }
        cancellations
    }

    pub fn apply(&mut self, message: &HistoryServerMessage) -> HistoryApplyResult {
        let result = match message {
            HistoryServerMessage::SnapshotChunk(chunk) => self.apply_chunk(chunk),
            HistoryServerMessage::SnapshotComplete(complete) => self.apply_complete(complete),
            HistoryServerMessage::Operation(operation) => self.apply_operation(operation),
            HistoryServerMessage::Rejected(rejection) => self.apply_rejection(rejection),
        };
        match result {
            HistoryApplyResult::IgnoredStale => {
                self.stale_responses = self.stale_responses.saturating_add(1);
            }
            HistoryApplyResult::Rejected => {
                self.rejected_responses = self.rejected_responses.saturating_add(1);
            }
            HistoryApplyResult::Applied => {}
        }
        result
    }

    fn apply_chunk(&mut self, chunk: &HistorySnapshotChunk) -> HistoryApplyResult {
        if self.session_id != Some(chunk.session_id)
            || chunk.chunk_count == 0
            || chunk.chunk_count > HISTORY_MAX_CHUNKS
            || chunk.chunk_index >= chunk.chunk_count
            || chunk.samples.len() > HISTORY_CHUNK_SAMPLES
        {
            return HistoryApplyResult::Rejected;
        }
        let Some(active) = self.active.get_mut(&chunk.track_id) else {
            return HistoryApplyResult::Rejected;
        };
        if active.request.request_id != chunk.request_id
            || active.request.session_id != chunk.session_id
            || active.completed
        {
            return HistoryApplyResult::Rejected;
        }
        if let Some(cutoff) = chunk.sample_cutoff {
            if chunk
                .samples
                .iter()
                .any(|sample| sample.sample_sequence > cutoff)
            {
                return self.reject_active(&chunk.track_id);
            }
        }

        let assembly = active.assembly.get_or_insert_with(|| SnapshotAssembly {
            session_id: chunk.session_id,
            track_id: chunk.track_id.clone(),
            request_id: chunk.request_id,
            server_time: chunk.server_time,
            sample_cutoff: chunk.sample_cutoff,
            snapshot_revision: chunk.snapshot_revision,
            coverage: chunk.coverage.clone(),
            chunk_count: chunk.chunk_count,
            chunks: BTreeMap::new(),
            sample_sequences: HashSet::new(),
            sample_count: 0,
        });
        if assembly.session_id != chunk.session_id
            || assembly.track_id != chunk.track_id
            || assembly.request_id != chunk.request_id
            || assembly.server_time != chunk.server_time
            || assembly.sample_cutoff != chunk.sample_cutoff
            || assembly.snapshot_revision != chunk.snapshot_revision
            || assembly.chunk_count != chunk.chunk_count
        {
            return self.reject_active(&chunk.track_id);
        }
        if assembly.chunks.contains_key(&chunk.chunk_index) {
            return HistoryApplyResult::IgnoredStale;
        }
        let mut chunk_sequences = HashSet::with_capacity(chunk.samples.len());
        if chunk
            .samples
            .iter()
            .any(|sample| !chunk_sequences.insert(sample.sample_sequence))
            || chunk
                .samples
                .iter()
                .any(|sample| assembly.sample_sequences.contains(&sample.sample_sequence))
        {
            return self.reject_active(&chunk.track_id);
        }
        let new_count = assembly.sample_count.saturating_add(chunk.samples.len());
        if new_count > usize::from(active.request.max_samples) {
            return self.reject_active(&chunk.track_id);
        }
        assembly.sample_count = new_count;
        assembly.sample_sequences.extend(chunk_sequences);
        assembly
            .chunks
            .insert(chunk.chunk_index, chunk.samples.clone());
        let transfer = HistoryTransferProgress {
            received_chunks: assembly.chunks.len() as u16,
            total_chunks: assembly.chunk_count,
            received_samples: assembly.sample_count,
            expected_samples: None,
        };
        if let Some(history) = self.histories.get_mut(&chunk.track_id) {
            history.loading = HistoryLoadingState::Loading;
            history.transfer = Some(transfer);
        }
        HistoryApplyResult::Applied
    }

    fn apply_complete(&mut self, complete: &HistorySnapshotComplete) -> HistoryApplyResult {
        if self.session_id != Some(complete.session_id) || complete.chunk_count > HISTORY_MAX_CHUNKS
        {
            return HistoryApplyResult::Rejected;
        }
        let Some(active) = self.active.get_mut(&complete.track_id) else {
            return HistoryApplyResult::Rejected;
        };
        if active.request.request_id != complete.request_id
            || active.request.session_id != complete.session_id
            || active.completed
        {
            return HistoryApplyResult::Rejected;
        }
        let assembly = match active.assembly.take() {
            Some(assembly) => assembly,
            None => {
                if complete.chunk_count != 0 {
                    return self.reject_active(&complete.track_id);
                }
                SnapshotAssembly {
                    session_id: complete.session_id,
                    track_id: complete.track_id.clone(),
                    request_id: complete.request_id,
                    server_time: complete.server_time,
                    sample_cutoff: complete.sample_cutoff,
                    snapshot_revision: complete.snapshot_revision,
                    coverage: complete.coverage.clone(),
                    chunk_count: 0,
                    chunks: BTreeMap::new(),
                    sample_sequences: HashSet::new(),
                    sample_count: 0,
                }
            }
        };
        if assembly.chunk_count != complete.chunk_count
            || assembly.chunks.len() != usize::from(complete.chunk_count)
            || assembly.server_time != complete.server_time
            || assembly.sample_cutoff != complete.sample_cutoff
            || assembly.snapshot_revision != complete.snapshot_revision
            || assembly.coverage != complete.coverage
        {
            return self.reject_active(&complete.track_id);
        }

        let mut snapshot_samples = Vec::with_capacity(assembly.sample_count);
        for samples in assembly.chunks.into_values() {
            snapshot_samples.extend(samples);
        }
        let snapshot_sample_count = snapshot_samples.len();
        let buffered = std::mem::take(&mut active.buffered_operations);
        active.completed = true;
        active.last_applied_revision = complete.snapshot_revision;
        let request_id = active.request.request_id;
        let track_id = active.request.track_id.clone();
        let sync_latency_ms = active.started_at.elapsed().as_millis() as u64;
        let _ = active;

        let history = self
            .histories
            .get_mut(&track_id)
            .expect("preview must precede history request");
        let preview_revision = history.history_revision;
        if complete.snapshot_revision >= preview_revision {
            history.server_time = complete.server_time;
            history.coverage = complete.coverage.clone();
        }
        history.history_revision = preview_revision.max(complete.snapshot_revision);
        history.loading = HistoryLoadingState::Complete;
        history.transfer = Some(HistoryTransferProgress {
            received_chunks: complete.chunk_count,
            total_chunks: complete.chunk_count,
            received_samples: snapshot_sample_count,
            expected_samples: Some(snapshot_sample_count),
        });
        history.request_id = Some(request_id);
        self.last_sync_latency_ms = Some(sync_latency_ms);

        let baseline_revision = preview_revision.max(complete.snapshot_revision);
        if let Some(active) = self.active.get_mut(&track_id) {
            active.last_applied_revision = baseline_revision;
        }
        for sample in snapshot_samples {
            upsert_sample(&mut history.samples, sample);
        }

        for (_, operation) in buffered {
            if operation.operation.revision > baseline_revision {
                let revision = operation.operation.revision;
                let removed = matches!(operation.operation.kind, HistoryOperationKind::Remove);
                self.apply_live_operation(operation);
                if !removed {
                    if let Some(active) = self.active.get_mut(&track_id) {
                        active.last_applied_revision = active.last_applied_revision.max(revision);
                    }
                }
            }
        }
        self.enforce_sample_bound();
        self.enforce_track_bound();
        HistoryApplyResult::Applied
    }

    fn apply_operation(&mut self, message: &HistoryOperationMessage) -> HistoryApplyResult {
        if self.session_id != Some(message.session_id)
            || message.operation.session_id != message.session_id
            || message.operation.track_id != message.track_id
            || message.sample_cutoff != message.operation.sample_cutoff
            || message.operation.revision == 0
        {
            return HistoryApplyResult::Rejected;
        }
        let Some(active) = self.active.get_mut(&message.track_id) else {
            return HistoryApplyResult::Rejected;
        };
        if active.request.request_id != message.request_id
            || active.request.session_id != message.session_id
        {
            return HistoryApplyResult::Rejected;
        }
        if !active.completed {
            if active
                .buffered_operations
                .contains_key(&message.operation.revision)
            {
                return HistoryApplyResult::IgnoredStale;
            }
            if active.buffered_operations.len() >= self.max_buffered_operations {
                return self.reject_active(&message.track_id);
            }
            active
                .buffered_operations
                .entry(message.operation.revision)
                .or_insert_with(|| message.clone());
            return HistoryApplyResult::Applied;
        }
        if message.operation.revision <= active.last_applied_revision {
            return HistoryApplyResult::IgnoredStale;
        }
        active.last_applied_revision = message.operation.revision;
        let operation = message.clone();
        let _ = active;
        self.apply_live_operation(operation);
        self.enforce_sample_bound();
        HistoryApplyResult::Applied
    }

    fn apply_live_operation(&mut self, message: HistoryOperationMessage) {
        let track_id = message.track_id.clone();
        if matches!(message.operation.kind, HistoryOperationKind::Remove) {
            self.histories.remove(&track_id);
            self.active.remove(&track_id);
            self.retry_after.remove(&track_id);
            self.retry_attempts.remove(&track_id);
            self.permanent_errors.remove(&track_id);
            return;
        }
        let Some(history) = self.histories.get_mut(&track_id) else {
            return;
        };
        if message.operation.revision >= history.history_revision {
            history.server_time = message.server_time;
            history.history_revision = message.operation.revision;
            history.coverage = message.operation.coverage.clone();
        }
        match message.operation.kind {
            HistoryOperationKind::Append(sample) | HistoryOperationKind::Correction(sample) => {
                upsert_sample(&mut history.samples, sample)
            }
            HistoryOperationKind::Prune { through_sequence } => {
                history
                    .samples
                    .retain(|sample| sample.sample_sequence > through_sequence);
            }
            HistoryOperationKind::Remove => unreachable!("handled above"),
        }
    }

    fn apply_rejection(&mut self, rejection: &HistoryRequestRejection) -> HistoryApplyResult {
        if self.session_id != Some(rejection.session_id) {
            return HistoryApplyResult::Rejected;
        }
        let Some(active) = self.active.get(&rejection.track_id) else {
            return HistoryApplyResult::Rejected;
        };
        if active.request.request_id != rejection.request_id {
            return HistoryApplyResult::IgnoredStale;
        }
        self.active.remove(&rejection.track_id);
        if let Some(history) = self.histories.get_mut(&rejection.track_id) {
            history.loading = HistoryLoadingState::RetryableError;
            history.request_id = None;
        }
        if rejection.retryable {
            self.schedule_retry(&rejection.track_id);
        } else {
            self.permanent_errors.insert(rejection.track_id.clone());
        }
        HistoryApplyResult::Rejected
    }

    fn reject_active(&mut self, track_id: &TrackId) -> HistoryApplyResult {
        self.active.remove(track_id);
        if let Some(history) = self.histories.get_mut(track_id) {
            history.loading = HistoryLoadingState::RetryableError;
            history.request_id = None;
        }
        self.schedule_retry(track_id);
        HistoryApplyResult::Rejected
    }

    fn schedule_retry(&mut self, track_id: &TrackId) {
        let attempts = self.retry_attempts.entry(track_id.clone()).or_default();
        *attempts = attempts.saturating_add(1);
        let shift = attempts.saturating_sub(1).min(5);
        let delay_ms = 50_u64.saturating_mul(1_u64 << shift);
        self.retry_after.insert(
            track_id.clone(),
            Instant::now() + std::time::Duration::from_millis(delay_ms.min(2_000)),
        );
    }

    fn accept_session(&mut self, session_id: HistorySessionId) -> bool {
        match self.session_id {
            Some(current) if current != session_id => {
                self.session_id = Some(session_id);
                self.histories.clear();
                self.active.clear();
                self.retry_after.clear();
                self.retry_attempts.clear();
                self.permanent_errors.clear();
                true
            }
            None => {
                self.session_id = Some(session_id);
                true
            }
            _ => true,
        }
    }

    fn history_entry(
        &mut self,
        session_id: HistorySessionId,
        track_id: TrackId,
        server_time: Timestamp,
        coverage: HistoryCoverage,
    ) -> &mut ClientTrackHistory {
        self.histories
            .entry(track_id.clone())
            .or_insert_with(|| ClientTrackHistory {
                session_id,
                track_id,
                server_time,
                history_revision: 0,
                coverage,
                loading: HistoryLoadingState::Preview,
                transfer: None,
                request_id: None,
                samples: Vec::new(),
            })
    }

    fn enforce_sample_bound(&mut self) {
        while self.total_sample_count() > self.max_samples {
            let Some(track_id) = self
                .histories
                .iter()
                .filter(|(_, history)| !matches!(history.loading, HistoryLoadingState::Complete))
                .min_by_key(|(_, history)| history.samples.first().map(|s| s.state_time))
                .map(|(track_id, _)| track_id.clone())
                .or_else(|| {
                    self.histories
                        .iter()
                        .min_by_key(|(_, history)| history.samples.first().map(|s| s.state_time))
                        .map(|(track_id, _)| track_id.clone())
                })
            else {
                break;
            };
            let Some(history) = self.histories.get_mut(&track_id) else {
                break;
            };
            if history.samples.is_empty() {
                break;
            }
            history.samples.remove(0);
            if matches!(history.loading, HistoryLoadingState::Complete) {
                history.loading = HistoryLoadingState::Partial;
            }
        }
    }

    fn enforce_track_bound(&mut self) {
        while self.histories.len() > HISTORY_MAX_CLIENT_TRACKS {
            let Some(track_id) = self
                .histories
                .iter()
                .filter(|(track_id, _)| !self.active.contains_key(*track_id))
                .min_by_key(|(_, history)| history.server_time)
                .map(|(track_id, _)| track_id.clone())
            else {
                break;
            };
            self.histories.remove(&track_id);
            self.retry_after.remove(&track_id);
            self.retry_attempts.remove(&track_id);
            self.permanent_errors.remove(&track_id);
        }
    }
}

fn upsert_sample(samples: &mut Vec<DisplayHistorySample>, sample: DisplayHistorySample) {
    match samples.binary_search_by_key(&sample.sample_sequence, |item| item.sample_sequence) {
        Ok(index) => samples[index] = sample,
        Err(index) => samples.insert(index, sample),
    }
}
