use std::mem::size_of;

use bevy_ecs::prelude::Component;
use chrono::Duration;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::display::{
    AltitudeReference, DisplayProvenance, DisplayTrack, FieldProvenance, HeadingReference,
    VerticalRateReference,
};
use crate::{PositionSource, Timestamp, TrackId, TrackStatus};

/// Identifies one in-memory agent lifetime. History is never valid across a
/// process restart, so clients must include this value in any future handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HistorySessionId(pub Uuid);

impl HistorySessionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub fn nil() -> Self {
        Self(Uuid::nil())
    }
}

impl Default for HistorySessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// The render-ready value sampled by the authoritative history recorder.
/// `state_time` is an observation-derived time, never a client frame time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayHistoryInput {
    pub state_time: Timestamp,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude_ft: Option<i32>,
    pub altitude_reference: AltitudeReference,
    pub ground_speed_kts: Option<f64>,
    pub heading: Option<f32>,
    pub heading_reference: HeadingReference,
    pub vertical_rate: Option<i32>,
    pub vertical_rate_reference: VerticalRateReference,
    pub position_source: Option<PositionSource>,
    pub status: TrackStatus,
    pub estimated: bool,
    pub provenance: DisplayProvenance,
}

impl From<&DisplayTrack> for DisplayHistoryInput {
    fn from(track: &DisplayTrack) -> Self {
        Self {
            state_time: track.last_seen,
            latitude: track.latitude,
            longitude: track.longitude,
            altitude_ft: track.altitude_ft,
            altitude_reference: track.altitude_reference,
            ground_speed_kts: track.velocity_kts,
            heading: track.heading,
            heading_reference: track.heading_reference,
            vertical_rate: track.vertical_rate,
            vertical_rate_reference: track.vertical_rate_reference,
            position_source: track.position_source,
            status: track.status,
            estimated: track.predicting || track.status == TrackStatus::Coasting,
            provenance: track.provenance.clone(),
        }
    }
}

/// Why a sample starts a new continuity segment. The break is represented on
/// the first sample of the new segment so renderers do not infer a line across
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryBreakReason {
    SamplingGap,
    Coasting,
    Reacquired,
    EstimatedBoundary,
}

/// A stable historical value. `sample_sequence` remains stable when a value is
/// corrected and is never reused after pruning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayHistorySample {
    pub sample_sequence: u64,
    pub state_time: Timestamp,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude_ft: Option<i32>,
    pub altitude_reference: AltitudeReference,
    pub ground_speed_kts: Option<f64>,
    pub heading: Option<f32>,
    pub heading_reference: HeadingReference,
    pub vertical_rate: Option<i32>,
    pub vertical_rate_reference: VerticalRateReference,
    pub position_source: Option<PositionSource>,
    pub status: TrackStatus,
    pub estimated: bool,
    pub provenance: DisplayProvenance,
    pub segment_id: u32,
    pub break_reason: Option<HistoryBreakReason>,
}

/// Estimate the heap footprint of one retained history sample.
///
/// The fixed-size part includes the vector-owned value and the variable part
/// accounts for provenance sensor identifiers. This is intentionally an
/// allocation budget estimate, not a serialized wire-size calculation.
#[must_use]
pub fn estimate_history_sample_bytes(sample: &DisplayHistorySample) -> usize {
    size_of::<DisplayHistorySample>()
        + estimate_field_provenance_bytes(&sample.provenance.position)
        + estimate_field_provenance_bytes(&sample.provenance.altitude)
        + estimate_field_provenance_bytes(&sample.provenance.ground_speed)
        + estimate_field_provenance_bytes(&sample.provenance.airspeed)
        + estimate_field_provenance_bytes(&sample.provenance.vertical_rate)
        + estimate_field_provenance_bytes(&sample.provenance.heading)
}

fn estimate_field_provenance_bytes(provenance: &FieldProvenance) -> usize {
    size_of::<FieldProvenance>() + provenance.sensor_id.as_ref().map_or(0, String::capacity)
}

impl DisplayHistorySample {
    #[must_use]
    pub fn from_input(
        sample_sequence: u64,
        input: &DisplayHistoryInput,
        segment_id: u32,
        break_reason: Option<HistoryBreakReason>,
    ) -> Self {
        Self {
            sample_sequence,
            state_time: input.state_time,
            latitude: input.latitude,
            longitude: input.longitude,
            altitude_ft: input.altitude_ft,
            altitude_reference: input.altitude_reference,
            ground_speed_kts: input.ground_speed_kts,
            heading: input.heading,
            heading_reference: input.heading_reference,
            vertical_rate: input.vertical_rate,
            vertical_rate_reference: input.vertical_rate_reference,
            position_source: input.position_source,
            status: input.status,
            estimated: input.estimated,
            provenance: input.provenance.clone(),
            segment_id,
            break_reason,
        }
    }
}

/// The portion of a track retained by the agent, independent of the preview
/// window sent to a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryCoverage {
    pub first_seen: Option<Timestamp>,
    pub retained_from: Option<Timestamp>,
    pub retained_to: Option<Timestamp>,
    pub retained_sample_count: usize,
    pub retained_duration: Duration,
    pub sampling_interval: Duration,
    pub truncation_reason: Option<HistoryTruncationReason>,
}

impl Default for HistoryCoverage {
    fn default() -> Self {
        Self {
            first_seen: None,
            retained_from: None,
            retained_to: None,
            retained_sample_count: 0,
            retained_duration: Duration::zero(),
            sampling_interval: Duration::zero(),
            truncation_reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryTruncationReason {
    Retention,
    PerTrackLimit,
    GlobalLimit,
}

/// Identifies one selected-history transfer. Request IDs are scoped by the
/// server session and are never reused by a client while a request is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HistoryRequestId(pub Uuid);

impl HistoryRequestId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for HistoryRequestId {
    fn default() -> Self {
        Self::new()
    }
}

/// A consistent copy of one track's retained samples. The revision and sample
/// cutoff form the watermark for the live operation stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistorySnapshot {
    pub session_id: HistorySessionId,
    pub track_id: TrackId,
    pub server_time: Timestamp,
    pub sample_cutoff: Option<u64>,
    pub revision: u64,
    pub coverage: HistoryCoverage,
    pub samples: Vec<DisplayHistorySample>,
}

/// One authoritative change after a snapshot watermark. `revision` is global
/// to the server session, while the sample sequence is stable per track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryOperation {
    pub session_id: HistorySessionId,
    pub track_id: TrackId,
    /// Highest retained sample sequence after this operation.
    pub sample_cutoff: Option<u64>,
    pub revision: u64,
    pub coverage: HistoryCoverage,
    pub kind: HistoryOperationKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HistoryOperationKind {
    Append(DisplayHistorySample),
    Correction(DisplayHistorySample),
    Prune { through_sequence: u64 },
    Remove,
}

/// Estimate the in-memory footprint of one retained operation.
#[must_use]
pub fn estimate_history_operation_bytes(operation: &HistoryOperation) -> usize {
    size_of::<HistoryOperation>()
        + match &operation.kind {
            HistoryOperationKind::Append(sample) | HistoryOperationKind::Correction(sample) => {
                estimate_history_sample_bytes(sample)
            }
            HistoryOperationKind::Prune { .. } | HistoryOperationKind::Remove => 0,
        }
}

/// Bounded initial history replicated with a display track. This is deliberately
/// a preview, not T5's selected-track snapshot/chunk/live operation stream.
#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayTrail {
    pub session_id: HistorySessionId,
    pub track_id: TrackId,
    pub server_time: Timestamp,
    pub history_revision: u64,
    pub coverage: HistoryCoverage,
    pub samples: Vec<DisplayHistorySample>,
    pub preview_window: Duration,
    pub preview_truncated: bool,
    pub sample_sequence_start: Option<u64>,
    pub sample_sequence_end: Option<u64>,
}

impl Default for DisplayTrail {
    fn default() -> Self {
        Self {
            session_id: HistorySessionId::nil(),
            track_id: TrackId::default(),
            server_time: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
            history_revision: 0,
            coverage: HistoryCoverage::default(),
            samples: Vec::new(),
            preview_window: Duration::zero(),
            preview_truncated: false,
            sample_sequence_start: None,
            sample_sequence_end: None,
        }
    }
}
