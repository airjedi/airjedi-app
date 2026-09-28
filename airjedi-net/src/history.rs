//! Selected-track history protocol and client-side read model.
//!
//! The protocol uses the existing Replicon/Renet connection. A request captures
//! one server-side snapshot at a revision watermark; later operations carry the
//! same request identity and are buffered until completion. The client read
//! model is deliberately independent of Bevy visuals so trails and charts can
//! consume one stable source in later tickets.

use std::collections::{BTreeMap, HashMap, HashSet};

use airjedi_core::{
    DisplayHistorySample, DisplayTrail, HistoryCoverage, HistoryOperation, HistoryOperationKind,
    HistoryRequestId, HistorySessionId, Timestamp, TrackId,
};
use bevy::prelude::{Message, Resource};
use bevy_replicon::prelude::{Channel, ClientMessageAppExt, ServerMessageAppExt};
use serde::{Deserialize, Serialize};

pub const HISTORY_CHUNK_SAMPLES: usize = 64;
pub const HISTORY_MAX_SNAPSHOT_SAMPLES: usize = 901;
pub const HISTORY_MAX_CLIENT_SAMPLES: usize = 4 * HISTORY_MAX_SNAPSHOT_SAMPLES;
pub const HISTORY_MAX_BUFFERED_OPERATIONS: usize = 256;

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
    pub request_id: Option<HistoryRequestId>,
    pub samples: Vec<DisplayHistorySample>,
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
    sample_count: usize,
}

#[derive(Debug, Clone)]
struct ActiveRequest {
    request: HistoryRequest,
    assembly: Option<SnapshotAssembly>,
    buffered_operations: BTreeMap<u64, HistoryOperationMessage>,
    completed: bool,
    last_applied_revision: u64,
}

/// Client-owned bounded cache and one stable read interface for trails/charts.
#[derive(Resource, Debug)]
pub struct ClientHistoryStore {
    session_id: Option<HistorySessionId>,
    histories: HashMap<TrackId, ClientTrackHistory>,
    active: HashMap<TrackId, ActiveRequest>,
    max_samples: usize,
    max_buffered_operations: usize,
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
        let Some(history) = self.histories.get(track_id) else {
            return HistoryRequestPlan::default();
        };

        if let Some(active) = self.active.get(track_id) {
            if active.request.priority == priority
                && !matches!(history.loading, HistoryLoadingState::RetryableError)
            {
                return HistoryRequestPlan::default();
            }
        }

        let cancel = self.active.remove(track_id).map(|active| HistoryCancel {
            session_id: active.request.session_id,
            track_id: active.request.track_id,
            request_id: active.request.request_id,
        });
        let request = HistoryRequest {
            session_id,
            track_id: track_id.clone(),
            request_id: HistoryRequestId::new(),
            sample_cutoff: history.samples.last().map(|sample| sample.sample_sequence),
            priority,
            max_samples: HISTORY_MAX_SNAPSHOT_SAMPLES as u16,
        };
        self.active.insert(
            track_id.clone(),
            ActiveRequest {
                request: request.clone(),
                assembly: None,
                buffered_operations: BTreeMap::new(),
                completed: false,
                last_applied_revision: 0,
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
        self.prepare_request(track_id, priority)
    }

    pub fn cancel_request(&mut self, track_id: &TrackId) -> Option<HistoryCancel> {
        let active = self.active.remove(track_id)?;
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
        }
        cancellations
    }

    pub fn apply(&mut self, message: &HistoryServerMessage) -> HistoryApplyResult {
        match message {
            HistoryServerMessage::SnapshotChunk(chunk) => self.apply_chunk(chunk),
            HistoryServerMessage::SnapshotComplete(complete) => self.apply_complete(complete),
            HistoryServerMessage::Operation(operation) => self.apply_operation(operation),
            HistoryServerMessage::Rejected(rejection) => self.apply_rejection(rejection),
        }
    }

    fn apply_chunk(&mut self, chunk: &HistorySnapshotChunk) -> HistoryApplyResult {
        if self.session_id != Some(chunk.session_id)
            || chunk.chunk_count > HISTORY_MAX_SNAPSHOT_SAMPLES as u16
            || chunk.chunk_index >= chunk.chunk_count
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
        let new_count = assembly.sample_count.saturating_add(chunk.samples.len());
        if new_count > HISTORY_MAX_SNAPSHOT_SAMPLES {
            return self.reject_active(&chunk.track_id);
        }
        assembly.sample_count = new_count;
        assembly
            .chunks
            .insert(chunk.chunk_index, chunk.samples.clone());
        if let Some(history) = self.histories.get_mut(&chunk.track_id) {
            history.loading = HistoryLoadingState::Loading;
        }
        HistoryApplyResult::Applied
    }

    fn apply_complete(&mut self, complete: &HistorySnapshotComplete) -> HistoryApplyResult {
        if self.session_id != Some(complete.session_id) {
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
        let buffered = std::mem::take(&mut active.buffered_operations);
        active.completed = true;
        active.last_applied_revision = complete.snapshot_revision;
        let request_id = active.request.request_id;
        let track_id = active.request.track_id.clone();
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
        history.request_id = Some(request_id);
        for sample in snapshot_samples {
            upsert_sample(&mut history.samples, sample);
        }

        for (_, operation) in buffered {
            if operation.operation.revision > complete.snapshot_revision {
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
        HistoryApplyResult::Applied
    }

    fn apply_operation(&mut self, message: &HistoryOperationMessage) -> HistoryApplyResult {
        if self.session_id != Some(message.session_id)
            || message.operation.session_id != message.session_id
            || message.operation.track_id != message.track_id
            || message.sample_cutoff != message.operation.sample_cutoff
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
        HistoryApplyResult::Rejected
    }

    fn reject_active(&mut self, track_id: &TrackId) -> HistoryApplyResult {
        self.active.remove(track_id);
        if let Some(history) = self.histories.get_mut(track_id) {
            history.loading = HistoryLoadingState::RetryableError;
            history.request_id = None;
        }
        HistoryApplyResult::Rejected
    }

    fn accept_session(&mut self, session_id: HistorySessionId) -> bool {
        match self.session_id {
            Some(current) if current != session_id => {
                self.session_id = Some(session_id);
                self.histories.clear();
                self.active.clear();
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
}

fn upsert_sample(samples: &mut Vec<DisplayHistorySample>, sample: DisplayHistorySample) {
    match samples.binary_search_by_key(&sample.sample_sequence, |item| item.sample_sequence) {
        Ok(index) => samples[index] = sample,
        Err(index) => samples.insert(index, sample),
    }
}
