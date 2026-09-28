//! Authoritative, bounded display history shared by embedded and headless modes.
//!
//! This module deliberately records projected display values rather than raw
//! observations or client interpolation output. The recorder is independent of
//! clients, trail visibility, and renderer readiness. `DisplayTrail` is only a
//! bounded preview of this canonical record; T5 owns selected-track transfer.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use airjedi_core::{
    DisplayHistoryInput, DisplayHistorySample, DisplayTrail, HistoryBreakReason, HistoryCoverage,
    HistorySessionId, HistoryTruncationReason, TrackId, TrackStatus,
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
    /// Matches the fusion OOSM default. Older samples are retained as history,
    /// but the pipeline cannot reconstruct them safely as corrections.
    pub correction_horizon: Duration,
    pub discontinuity_gap: Duration,
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
}

impl HistoryRecorder {
    #[must_use]
    pub fn new(config: HistoryConfig) -> Self {
        Self {
            session_id: HistorySessionId::new(),
            config,
            history_revision: 0,
            tracks: HashMap::new(),
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
    pub fn contains_track(&self, track_id: &TrackId) -> bool {
        self.tracks.contains_key(track_id)
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
        history.samples.push_back(DisplayHistorySample::from_input(
            sequence,
            &input,
            segment_id,
            break_reason,
        ));
        self.history_revision = self.history_revision.saturating_add(1);
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
        self.history_revision = self.history_revision.saturating_add(1);
        true
    }

    /// Prune retention and both memory bounds without consulting any client.
    pub fn prune(&mut self, now: DateTime<Utc>) {
        let retention = chrono::Duration::from_std(self.config.retention)
            .unwrap_or_else(|_| chrono::Duration::minutes(30));
        let cutoff = now - retention;
        let per_track_limit = self.config.max_samples_per_track.max(1);
        let mut changed = false;

        for history in self.tracks.values_mut() {
            while history
                .samples
                .front()
                .is_some_and(|sample| sample.state_time < cutoff)
            {
                history.samples.pop_front();
                history.truncation_reason = Some(HistoryTruncationReason::Retention);
                changed = true;
            }
            while history.samples.len() > per_track_limit {
                history.samples.pop_front();
                history.truncation_reason = Some(HistoryTruncationReason::PerTrackLimit);
                changed = true;
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
                history.samples.pop_front();
                history.truncation_reason = Some(HistoryTruncationReason::GlobalLimit);
                changed = true;
            }
        }

        if changed {
            self.history_revision = self.history_revision.saturating_add(1);
        }
    }

    /// Remove a completed track lifetime and all associated metadata.
    pub fn remove_track(&mut self, track_id: &TrackId) -> bool {
        let removed = self.tracks.remove(track_id).is_some();
        if removed {
            self.history_revision = self.history_revision.saturating_add(1);
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
