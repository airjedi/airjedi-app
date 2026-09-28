//! Authoritative, bounded display history shared by embedded and headless modes.
//!
//! This module deliberately records projected display values rather than raw
//! observations or client interpolation output. The recorder is independent of
//! clients, trail visibility, and renderer readiness. `DisplayTrail` is only a
//! bounded preview of this canonical record; T5 owns selected-track transfer.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use airjedi_core::{
    estimate_history_operation_bytes, estimate_history_sample_bytes, DisplayHistoryInput,
    DisplayHistorySample, DisplayTrail, HistoryBreakReason, HistoryCoverage, HistoryOperation,
    HistoryOperationKind, HistorySessionId, HistorySnapshot, HistoryTruncationReason, TrackId,
    TrackStatus,
};
use bevy_ecs::prelude::{Query, Res, ResMut, Resource};
use chrono::{DateTime, Utc};

use crate::clock::FusionClock;
use crate::display::{derive_display_track, raw_observation_hint_for};
use crate::filter::TrackerState;
use crate::store::TimelineStore;
use crate::track::{Track, TrackQuality};

/// Initial bounds for authoritative history. The cadence and retention are
/// configuration, not assumptions made by the client or renderer.
#[derive(Debug, Clone)]
pub struct HistoryConfig {
    pub retention: Duration,
    pub sampling_interval: Duration,
    pub preview_window: Duration,
    pub max_samples_per_track: usize,
    pub max_samples_global: usize,
    pub max_preview_samples: usize,
    pub max_operation_log: usize,
    /// Matches the fusion OOSM default. Older samples are retained as history,
    /// but the pipeline cannot reconstruct them safely as corrections.
    pub correction_horizon: Duration,
    pub discontinuity_gap: Duration,
}

/// Bounded recorder accounting exposed to the agent and diagnostics surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryRecorderDiagnostics {
    pub session_id: HistorySessionId,
    pub history_revision: u64,
    pub track_count: usize,
    pub retained_samples: usize,
    pub retained_sample_bytes: usize,
    pub operation_count: usize,
    pub operation_bytes: usize,
    pub truncation_events: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            retention: Duration::from_secs(30 * 60),
            sampling_interval: Duration::from_secs(2),
            preview_window: Duration::from_secs(5 * 60),
            max_samples_per_track: 901,
            max_samples_global: 100_000,
            max_preview_samples: 256,
            max_operation_log: 4_096,
            correction_horizon: Duration::from_secs(30),
            discontinuity_gap: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone)]
struct TrackHistory {
    first_seen: DateTime<Utc>,
    samples: VecDeque<DisplayHistorySample>,
    next_sequence: u64,
    truncation_reason: Option<HistoryTruncationReason>,
}

impl TrackHistory {
    fn new(first_seen: DateTime<Utc>) -> Self {
        Self {
            first_seen,
            samples: VecDeque::new(),
            next_sequence: 1,
            truncation_reason: None,
        }
    }
}

/// Shared in-memory history owned by the agent/fusion process.
#[derive(Debug, Resource)]
pub struct HistoryRecorder {
    session_id: HistorySessionId,
    config: HistoryConfig,
    history_revision: u64,
    tracks: HashMap<TrackId, TrackHistory>,
    operations: VecDeque<HistoryOperation>,
    truncation_events: usize,
}

impl HistoryRecorder {
    #[must_use]
    pub fn new(config: HistoryConfig) -> Self {
        Self {
            session_id: HistorySessionId::new(),
            config,
            history_revision: 0,
            tracks: HashMap::new(),
            operations: VecDeque::new(),
            truncation_events: 0,
        }
    }

    #[must_use]
    pub fn with_session(config: HistoryConfig, session_id: HistorySessionId) -> Self {
        Self {
            session_id,
            ..Self::new(config)
        }
    }

    #[must_use]
    pub fn config(&self) -> &HistoryConfig {
        &self.config
    }

    #[must_use]
    pub fn session_id(&self) -> HistorySessionId {
        self.session_id
    }

    #[must_use]
    pub fn history_revision(&self) -> u64 {
        self.history_revision
    }

    #[must_use]
    pub fn track_sample_count(&self, track_id: &TrackId) -> usize {
        self.tracks
            .get(track_id)
            .map_or(0, |history| history.samples.len())
    }

    #[must_use]
    pub fn total_sample_count(&self) -> usize {
        self.tracks
            .values()
            .map(|history| history.samples.len())
            .sum()
    }

    #[must_use]
    pub fn diagnostics(&self) -> HistoryRecorderDiagnostics {
        HistoryRecorderDiagnostics {
            session_id: self.session_id,
            history_revision: self.history_revision,
            track_count: self.tracks.len(),
            retained_samples: self.total_sample_count(),
            retained_sample_bytes: self
                .tracks
                .values()
                .flat_map(|history| history.samples.iter())
                .map(estimate_history_sample_bytes)
                .sum(),
            operation_count: self.operations.len(),
            operation_bytes: self
                .operations
                .iter()
                .map(estimate_history_operation_bytes)
                .sum(),
            truncation_events: self.truncation_events,
        }
    }

    #[must_use]
    pub fn contains_track(&self, track_id: &TrackId) -> bool {
        self.tracks.contains_key(track_id)
    }

    /// Capture a bounded, internally consistent copy for a selected-track
    /// transfer. The caller supplies the current fusion/server time reference;
    /// it is not used to change any sample timestamps.
    #[must_use]
    pub fn snapshot(
        &self,
        track_id: &TrackId,
        server_time: DateTime<Utc>,
    ) -> Option<HistorySnapshot> {
        let history = self.tracks.get(track_id)?;
        Some(HistorySnapshot {
            session_id: self.session_id,
            track_id: track_id.clone(),
            server_time,
            sample_cutoff: history.samples.back().map(|sample| sample.sample_sequence),
            revision: self.history_revision,
            coverage: coverage_for(history, self.config.sampling_interval),
            samples: history.samples.iter().cloned().collect(),
        })
    }

    /// Return all retained operations newer than `revision`. `None` means the
    /// bounded operation log no longer covers the requested watermark and the
    /// client must retry with a fresh snapshot.
    #[must_use]
    pub fn operations_since(&self, revision: u64) -> Option<Vec<HistoryOperation>> {
        if revision == self.history_revision {
            return Some(Vec::new());
        }
        let first_revision = self.operations.front()?.revision;
        if revision.saturating_add(1) < first_revision {
            return None;
        }
        Some(
            self.operations
                .iter()
                .filter(|operation| operation.revision > revision)
                .cloned()
                .collect(),
        )
    }

    /// Record a projected state when the configured cadence is due. The caller
    /// supplies `now` only for pruning; sample timestamps come from the input.
    pub fn record_display_track(
        &mut self,
        display: &airjedi_core::DisplayTrack,
        now: DateTime<Utc>,
    ) -> bool {
        let input = DisplayHistoryInput::from(display);
        let recorded = self.record_input(&display.track_id, input);
        self.prune(now);
        recorded
    }

    /// Public recorder seam used by deterministic replay and parity tests.
    pub fn record_input(&mut self, track_id: &TrackId, input: DisplayHistoryInput) -> bool {
        let history = self
            .tracks
            .entry(track_id.clone())
            .or_insert_with(|| TrackHistory::new(input.state_time));

        if input.state_time < history.first_seen {
            history.first_seen = input.state_time;
        }

        let should_sample = match history.samples.back() {
            None => true,
            Some(previous) => {
                let elapsed = input.state_time.signed_duration_since(previous.state_time);
                let gap = chrono::Duration::from_std(self.config.discontinuity_gap)
                    .unwrap_or_else(|_| chrono::Duration::seconds(10));
                elapsed
                    >= chrono::Duration::from_std(self.config.sampling_interval)
                        .unwrap_or_else(|_| chrono::Duration::seconds(2))
                    || elapsed > gap
                    || previous.status != input.status
                    || previous.estimated != input.estimated
            }
        };

        if !should_sample {
            return false;
        }

        let (segment_id, break_reason) = history.samples.back().map_or((0, None), |previous| {
            let elapsed = input.state_time.signed_duration_since(previous.state_time);
            let gap = chrono::Duration::from_std(self.config.discontinuity_gap)
                .unwrap_or_else(|_| chrono::Duration::seconds(10));
            let reason = if elapsed > gap {
                Some(HistoryBreakReason::SamplingGap)
            } else if previous.status != TrackStatus::Coasting
                && input.status == TrackStatus::Coasting
            {
                Some(HistoryBreakReason::Coasting)
            } else if previous.status == TrackStatus::Coasting
                && input.status != TrackStatus::Coasting
            {
                Some(HistoryBreakReason::Reacquired)
            } else if previous.estimated != input.estimated {
                Some(HistoryBreakReason::EstimatedBoundary)
            } else {
                None
            };
            (previous.segment_id + u32::from(reason.is_some()), reason)
        });

        let sequence = history.next_sequence;
        history.next_sequence = history.next_sequence.saturating_add(1);
        let sample = DisplayHistorySample::from_input(sequence, &input, segment_id, break_reason);
        history.samples.push_back(sample.clone());
        let coverage = coverage_for(history, self.config.sampling_interval);
        self.history_revision = self.history_revision.saturating_add(1);
        self.record_operation(track_id, coverage, HistoryOperationKind::Append(sample));
        true
    }

    /// Correct an existing sample only inside the same bounded horizon that the
    /// fusion pipeline can reconstruct. The sample sequence is intentionally
    /// preserved and the operation revision advances.
    pub fn correct_sample(
        &mut self,
        track_id: &TrackId,
        sample_sequence: u64,
        input: DisplayHistoryInput,
        now: DateTime<Utc>,
    ) -> bool {
        let Some(history) = self.tracks.get_mut(track_id) else {
            return false;
        };
        let age = now.signed_duration_since(
            history
                .samples
                .iter()
                .find(|sample| sample.sample_sequence == sample_sequence)
                .map_or(input.state_time, |sample| sample.state_time),
        );
        let horizon = chrono::Duration::from_std(self.config.correction_horizon)
            .unwrap_or_else(|_| chrono::Duration::seconds(30));
        if age < chrono::Duration::zero() || age > horizon {
            return false;
        }

        let Some(sample) = history
            .samples
            .iter_mut()
            .find(|sample| sample.sample_sequence == sample_sequence)
        else {
            return false;
        };
        let segment_id = sample.segment_id;
        let break_reason = sample.break_reason;
        *sample =
            DisplayHistorySample::from_input(sample_sequence, &input, segment_id, break_reason);
        let corrected = sample.clone();
        let coverage = coverage_for(history, self.config.sampling_interval);
        self.history_revision = self.history_revision.saturating_add(1);
        self.record_operation(
            track_id,
            coverage,
            HistoryOperationKind::Correction(corrected),
        );
        true
    }

    /// Prune retention and both memory bounds without consulting any client.
    pub fn prune(&mut self, now: DateTime<Utc>) {
        let retention = chrono::Duration::from_std(self.config.retention)
            .unwrap_or_else(|_| chrono::Duration::minutes(30));
        let cutoff = now - retention;
        let per_track_limit = self.config.max_samples_per_track.max(1);
        let mut pruned = Vec::new();

        for (track_id, history) in &mut self.tracks {
            while history
                .samples
                .front()
                .is_some_and(|sample| sample.state_time < cutoff)
            {
                let removed = history.samples.pop_front().expect("front exists");
                history.truncation_reason = Some(HistoryTruncationReason::Retention);
                pruned.push((track_id.clone(), removed.sample_sequence));
            }
            while history.samples.len() > per_track_limit {
                let removed = history.samples.pop_front().expect("limit exceeded");
                history.truncation_reason = Some(HistoryTruncationReason::PerTrackLimit);
                pruned.push((track_id.clone(), removed.sample_sequence));
            }
        }

        while self.total_sample_count() > self.config.max_samples_global.max(1) {
            let oldest =
                self.tracks
                    .iter()
                    .filter_map(|(track_id, history)| {
                        history.samples.front().map(|sample| {
                            (track_id.clone(), sample.state_time, sample.sample_sequence)
                        })
                    })
                    .min_by_key(|(_, state_time, sequence)| (*state_time, *sequence));
            let Some((track_id, _, _)) = oldest else {
                break;
            };
            if let Some(history) = self.tracks.get_mut(&track_id) {
                let removed = history.samples.pop_front().expect("front exists");
                history.truncation_reason = Some(HistoryTruncationReason::GlobalLimit);
                pruned.push((track_id, removed.sample_sequence));
            }
        }

        let mut highest_removed: HashMap<TrackId, u64> = HashMap::new();
        for (track_id, sequence) in pruned {
            highest_removed
                .entry(track_id)
                .and_modify(|highest| *highest = (*highest).max(sequence))
                .or_insert(sequence);
        }
        let mut pruned_tracks: Vec<(TrackId, u64)> = highest_removed.into_iter().collect();
        pruned_tracks.sort_by(|(left, _), (right, _)| left.0.as_bytes().cmp(right.0.as_bytes()));
        for (track_id, sequence) in pruned_tracks {
            let Some(history) = self.tracks.get(&track_id) else {
                continue;
            };
            let coverage = coverage_for(history, self.config.sampling_interval);
            self.history_revision = self.history_revision.saturating_add(1);
            self.record_operation(
                &track_id,
                coverage,
                HistoryOperationKind::Prune {
                    through_sequence: sequence,
                },
            );
            self.truncation_events = self.truncation_events.saturating_add(1);
        }
    }

    /// Remove a completed track lifetime and all associated metadata.
    pub fn remove_track(&mut self, track_id: &TrackId) -> bool {
        let removed = self.tracks.remove(track_id).is_some();
        if removed {
            self.history_revision = self.history_revision.saturating_add(1);
            self.record_operation(
                track_id,
                HistoryCoverage::default(),
                HistoryOperationKind::Remove,
            );
        }
        removed
    }

    /// Remove histories whose TrackId is no longer present in the fusion world.
    pub fn retain_tracks(&mut self, live_tracks: &HashSet<TrackId>) {
        let stale: Vec<TrackId> = self
            .tracks
            .keys()
            .filter(|track_id| !live_tracks.contains(*track_id))
            .cloned()
            .collect();
        for track_id in stale {
            self.remove_track(&track_id);
        }
    }

    /// Produce the bounded visible-first preview for the current server time.
    /// The coverage describes the complete retained range, not only the values
    /// included in this preview.
    #[must_use]
    pub fn preview(&self, track_id: &TrackId, server_time: DateTime<Utc>) -> DisplayTrail {
        let window = chrono::Duration::from_std(self.config.preview_window)
            .unwrap_or_else(|_| chrono::Duration::minutes(5));
        let max_samples = self.config.max_preview_samples.max(1);
        let Some(history) = self.tracks.get(track_id) else {
            return DisplayTrail {
                session_id: self.session_id,
                track_id: track_id.clone(),
                server_time,
                history_revision: self.history_revision,
                preview_window: window,
                ..Default::default()
            };
        };

        let coverage = coverage_for(history, self.config.sampling_interval);
        let preview_cutoff = server_time - window;
        let mut samples: Vec<DisplayHistorySample> = history
            .samples
            .iter()
            .filter(|sample| sample.state_time >= preview_cutoff)
            .cloned()
            .collect();
        if samples.len() > max_samples {
            let start = samples.len() - max_samples;
            samples.drain(..start);
        }
        let preview_truncated = samples.len() < history.samples.len();
        let sample_sequence_start = samples.first().map(|sample| sample.sample_sequence);
        let sample_sequence_end = samples.last().map(|sample| sample.sample_sequence);

        DisplayTrail {
            session_id: self.session_id,
            track_id: track_id.clone(),
            server_time,
            history_revision: self.history_revision,
            coverage,
            samples,
            preview_window: window,
            preview_truncated,
            sample_sequence_start,
            sample_sequence_end,
        }
    }

    fn record_operation(
        &mut self,
        track_id: &TrackId,
        coverage: HistoryCoverage,
        kind: HistoryOperationKind,
    ) {
        let sample_cutoff = if matches!(&kind, HistoryOperationKind::Remove) {
            None
        } else {
            self.tracks
                .get(track_id)
                .and_then(|history| history.samples.back())
                .map(|sample| sample.sample_sequence)
        };
        self.operations.push_back(HistoryOperation {
            session_id: self.session_id,
            track_id: track_id.clone(),
            sample_cutoff,
            revision: self.history_revision,
            coverage,
            kind,
        });
        while self.operations.len() > self.config.max_operation_log.max(1) {
            self.operations.pop_front();
        }
    }
}

fn coverage_for(history: &TrackHistory, sampling_interval: Duration) -> HistoryCoverage {
    let retained_from = history.samples.front().map(|sample| sample.state_time);
    let retained_to = history.samples.back().map(|sample| sample.state_time);
    let retained_duration = retained_from
        .zip(retained_to)
        .map_or_else(chrono::Duration::zero, |(from, to)| to - from);
    HistoryCoverage {
        first_seen: Some(history.first_seen),
        retained_from,
        retained_to,
        retained_sample_count: history.samples.len(),
        retained_duration,
        sampling_interval: chrono::Duration::from_std(sampling_interval)
            .unwrap_or_else(|_| chrono::Duration::seconds(2)),
        truncation_reason: history.truncation_reason,
    }
}

/// Shared recorder integration. It runs after fusion lifecycle and before any
/// client or renderer projection, so collection continues with zero clients.
pub fn record_history_system(
    mut recorder: ResMut<HistoryRecorder>,
    clock: Res<FusionClock>,
    store: Res<TimelineStore>,
    tracks: Query<(&Track, &TrackerState, &TrackQuality)>,
) {
    let now = clock.now_utc();
    let live_tracks: HashSet<TrackId> = tracks
        .iter()
        .map(|(track, _, _)| track.id.clone())
        .collect();

    for (track, tracker, quality) in &tracks {
        if quality.status == TrackStatus::Lost {
            continue;
        }
        let hint = raw_observation_hint_for(&store, track);
        let display = derive_display_track(track, tracker, quality, hint.as_ref(), None);
        recorder.record_display_track(&display, now);
    }
    recorder.prune(now);
    recorder.retain_tracks(&live_tracks);
}
