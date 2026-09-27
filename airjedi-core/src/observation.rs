use serde::{Deserialize, Serialize};

/// Identity of one decoded report within a source/decoder session.
///
/// `frame_sequence` identifies the source frame and `payload_index` separates
/// multiple logical reports decoded from that frame. Consumers combine this
/// with their sensor/feed identifier when comparing reports from independent
/// sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObservationIdentity {
    pub frame_sequence: u64,
    pub payload_index: u16,
}

/// Quality of the timestamp assigned to an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeSourceQuality {
    /// A receiver clock was anchored to a UTC receipt time and advanced from
    /// receiver ticks, including across a normal tick-counter rollover.
    ReceiverClock,
    /// The upstream protocol supplied an absolute timestamp.
    ProtocolTimestamp,
    /// No measurement timestamp was available, so receipt time was used.
    ReceiptTime,
    /// A receiver reset or invalid tick discontinuity forced a new receipt-time
    /// anchor.
    ReceiverReset,
}

/// Timing and identity for one independently fresh measurement field.
///
/// The identity remains source-local. Consumers combine it with the sensor or
/// feed identifier when deduplicating observations from independent sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObservationFreshness {
    pub observation_time: chrono::DateTime<chrono::Utc>,
    pub receipt_time: chrono::DateTime<chrono::Utc>,
    pub time_source: TimeSourceQuality,
    pub identity: ObservationIdentity,
}
