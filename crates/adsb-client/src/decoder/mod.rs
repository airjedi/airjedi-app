mod basestation;
#[cfg(feature = "decoder-native")]
mod native;
#[cfg(feature = "decoder-rs1090")]
mod rs1090_decoder;
#[cfg(feature = "decoder-rs1090")]
mod rs1090_mapping;

use crate::framing::Frame;
use crate::protocol::{AircraftMessage, DecodedMessage, MessageTiming};
use airjedi_core::{ObservationIdentity, TimeSourceQuality};
use chrono::{DateTime, Duration, Utc};

pub use basestation::BaseStationDecoder;
#[cfg(feature = "decoder-native")]
pub use native::NativeDecoder;
#[cfg(feature = "decoder-rs1090")]
pub use rs1090_decoder::Rs1090Decoder;

const BEAST_TICK_MASK: u64 = (1 << 48) - 1;
const BEAST_TICK_HALF_RANGE: u64 = 1 << 47;
const BEAST_TICKS_PER_SECOND: u64 = 12_000_000;

/// Converts a BEAST receiver tick counter into anchored UTC observation time.
///
/// BEAST carries a 48-bit 12 MHz counter, not UTC. The first timestamp in a
/// decoder session is anchored to its receipt time. Subsequent timestamps use
/// modular forward deltas, which naturally handles the normal counter rollover.
/// A discontinuity too large to distinguish from a receiver reset starts a new
/// receipt-time anchor and is explicitly marked as such.
#[derive(Debug, Default)]
pub(crate) struct ObservationClock {
    last_ticks: Option<u64>,
    last_observation_time: Option<DateTime<Utc>>,
}

impl ObservationClock {
    pub(crate) fn resolve(
        &mut self,
        receiver_ticks: Option<u64>,
        receipt_time: DateTime<Utc>,
    ) -> (DateTime<Utc>, TimeSourceQuality) {
        let Some(ticks) = receiver_ticks.map(|value| value & BEAST_TICK_MASK) else {
            self.reset();
            return (receipt_time, TimeSourceQuality::ReceiptTime);
        };

        let Some(previous_ticks) = self.last_ticks else {
            self.last_ticks = Some(ticks);
            self.last_observation_time = Some(receipt_time);
            return (receipt_time, TimeSourceQuality::ReceiverClock);
        };

        let delta_ticks = ticks.wrapping_sub(previous_ticks) & BEAST_TICK_MASK;
        if delta_ticks > BEAST_TICK_HALF_RANGE {
            self.last_ticks = Some(ticks);
            self.last_observation_time = Some(receipt_time);
            return (receipt_time, TimeSourceQuality::ReceiverReset);
        }

        let previous_time = self.last_observation_time.unwrap_or(receipt_time);
        let delta_nanos =
            ((u128::from(delta_ticks) * 1_000_000_000) / u128::from(BEAST_TICKS_PER_SECOND)) as i64;
        let observation_time = previous_time + Duration::nanoseconds(delta_nanos);
        self.last_ticks = Some(ticks);
        self.last_observation_time = Some(observation_time);
        (observation_time, TimeSourceQuality::ReceiverClock)
    }

    pub(crate) fn reset(&mut self) {
        self.last_ticks = None;
        self.last_observation_time = None;
    }
}

pub(crate) fn decorate_messages(
    frame: &Frame,
    receipt_time: DateTime<Utc>,
    observation_time: DateTime<Utc>,
    time_source: TimeSourceQuality,
    messages: Vec<AircraftMessage>,
) -> Vec<DecodedMessage> {
    messages
        .into_iter()
        .enumerate()
        .map(|(payload_index, message)| {
            DecodedMessage::new(
                message,
                MessageTiming {
                    observation_time,
                    receipt_time,
                    time_source,
                    identity: ObservationIdentity {
                        frame_sequence: frame.sequence,
                        payload_index: payload_index as u16,
                    },
                },
            )
        })
        .collect()
}

/// Decodes protocol frames into aircraft messages.
///
/// Each implementation is a fully independent decode pipeline. Stateful:
/// maintains known-ICAO set, CPR decode state, and reference position.
pub trait Decoder: Send {
    /// Decode a protocol frame into zero or more aircraft messages.
    ///
    /// Returns an empty Vec for frames that are valid but produce no
    /// message (e.g., Mode-A/C, unknown DF, failed CRC).
    fn decode(&mut self, frame: &Frame) -> Vec<DecodedMessage> {
        self.decode_at(frame, Utc::now())
    }

    /// Decode a frame using an explicitly supplied receipt time.
    ///
    /// Replay and deterministic tests must use this entry point instead of
    /// allowing the decoder to read the process wall clock.
    fn decode_at(&mut self, frame: &Frame, receipt_time: DateTime<Utc>) -> Vec<DecodedMessage>;

    /// Set the reference position for local CPR decode.
    fn set_reference_position(&mut self, lat: f64, lon: f64);

    /// Reset decode state (e.g., after reconnection).
    /// Clears CPR state but preserves known-ICAO set.
    fn reset(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::FrameType;
    use crate::protocol::{Icao, MessagePayload};
    use bytes::Bytes;

    fn frame(sequence: u64, timestamp: Option<u64>) -> Frame {
        Frame {
            sequence,
            timestamp,
            signal_level: None,
            data: Bytes::new(),
            frame_type: FrameType::ModeSLong,
        }
    }

    fn message() -> AircraftMessage {
        AircraftMessage {
            icao: Icao(0xA1B2C3),
            signal_level: None,
            payload: MessagePayload::Altitude {
                altitude: Some(10_000),
                squawk: None,
                alert: None,
                emergency: None,
                spi: None,
                is_on_ground: None,
            },
        }
    }

    #[test]
    fn receiver_clock_handles_rollover_without_wall_clock_jump() {
        let base = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let mut clock = ObservationClock::default();
        let (_, first_quality) = clock.resolve(Some((1 << 48) - 5), base);
        let (observed, second_quality) = clock.resolve(Some(5), base + Duration::seconds(1));

        assert_eq!(first_quality, TimeSourceQuality::ReceiverClock);
        assert_eq!(second_quality, TimeSourceQuality::ReceiverClock);
        assert_eq!(observed - base, Duration::nanoseconds(833));
    }

    #[test]
    fn receiver_reset_uses_receipt_time_and_marks_quality() {
        let base = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let mut clock = ObservationClock::default();
        clock.resolve(Some(100), base);
        let receipt = base + Duration::seconds(2);
        let (observed, quality) = clock.resolve(Some(90), receipt);

        assert_eq!(observed, receipt);
        assert_eq!(quality, TimeSourceQuality::ReceiverReset);
    }

    #[test]
    fn absent_receiver_timestamp_uses_receipt_time() {
        let receipt = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let mut clock = ObservationClock::default();
        let (observed, quality) = clock.resolve(None, receipt);

        assert_eq!(observed, receipt);
        assert_eq!(quality, TimeSourceQuality::ReceiptTime);
    }

    #[test]
    fn decoded_payloads_from_one_frame_have_distinct_identities() {
        let receipt = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let messages = decorate_messages(
            &frame(42, None),
            receipt,
            receipt,
            TimeSourceQuality::ReceiptTime,
            vec![message(), message()],
        );

        assert_eq!(messages[0].timing.observation_time, receipt);
        assert_eq!(messages[0].timing.identity.frame_sequence, 42);
        assert_eq!(messages[0].timing.identity.payload_index, 0);
        assert_eq!(messages[1].timing.identity.frame_sequence, 42);
        assert_eq!(messages[1].timing.identity.payload_index, 1);
    }

    #[test]
    fn receiver_ticks_are_independent_of_execution_speed() {
        let base = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let ticks = [12_000_000, 24_000_000];
        let mut first = ObservationClock::default();
        let mut second = ObservationClock::default();

        let first_times: Vec<_> = ticks
            .into_iter()
            .map(|tick| first.resolve(Some(tick), base).0)
            .collect();
        let second_times: Vec<_> = ticks
            .into_iter()
            .map(|tick| second.resolve(Some(tick), base).0)
            .collect();

        assert_eq!(first_times, second_times);
        assert_eq!(first_times[1] - first_times[0], Duration::seconds(1));
    }
}
