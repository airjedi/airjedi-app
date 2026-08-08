use std::collections::VecDeque;

use bytes::Bytes;

use super::{Frame, FrameType, Framer};

/// Framer for SDR-demodulated Mode-S frames.
///
/// Paired with `SdrTransport`. Each `feed()` call receives exactly one frame
/// in the internal encoding: `[4 bytes f32 LE signal_level][7 or 14 bytes Mode-S]`.
#[derive(Debug)]
pub struct SdrFramer {
    frames: VecDeque<Frame>,
}

impl SdrFramer {
    pub fn new() -> Self {
        Self {
            frames: VecDeque::new(),
        }
    }
}

impl Default for SdrFramer {
    fn default() -> Self {
        Self::new()
    }
}

const RSSI_HEADER_LEN: usize = 4;
const SHORT_MSG_LEN: usize = 7;
const LONG_MSG_LEN: usize = 14;

impl Framer for SdrFramer {
    fn feed(&mut self, data: &[u8]) {
        if data.len() < RSSI_HEADER_LEN + SHORT_MSG_LEN {
            return;
        }

        let signal_level = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let mode_s = &data[RSSI_HEADER_LEN..];

        let frame_type = if mode_s.len() >= LONG_MSG_LEN {
            FrameType::ModeSLong
        } else {
            FrameType::ModeSShort
        };

        let frame_len = if frame_type == FrameType::ModeSLong {
            LONG_MSG_LEN
        } else {
            SHORT_MSG_LEN
        };

        self.frames.push_back(Frame {
            data: Bytes::copy_from_slice(&mode_s[..frame_len]),
            frame_type,
            timestamp: None,
            signal_level: if signal_level.is_nan() {
                None
            } else {
                Some(signal_level)
            },
        });
    }

    fn next_frame(&mut self) -> Option<Frame> {
        self.frames.pop_front()
    }

    fn reset(&mut self) {
        self.frames.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sdr_frame(signal_level: f32, mode_s: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + mode_s.len());
        buf.extend_from_slice(&signal_level.to_le_bytes());
        buf.extend_from_slice(mode_s);
        buf
    }

    #[test]
    fn short_frame() {
        let mode_s = [0x02, 0x00, 0x00, 0xA1, 0xB2, 0xC3, 0x00];
        let data = make_sdr_frame(-10.5, &mode_s);

        let mut framer = SdrFramer::new();
        framer.feed(&data);
        let frame = framer.next_frame().unwrap();

        assert_eq!(frame.frame_type, FrameType::ModeSShort);
        assert_eq!(frame.data.as_ref(), &mode_s);
        assert!((frame.signal_level.unwrap() - (-10.5)).abs() < f32::EPSILON);
    }

    #[test]
    fn long_frame() {
        let mode_s = [0x8D; 14];
        let data = make_sdr_frame(-5.0, &mode_s);

        let mut framer = SdrFramer::new();
        framer.feed(&data);
        let frame = framer.next_frame().unwrap();

        assert_eq!(frame.frame_type, FrameType::ModeSLong);
        assert_eq!(frame.data.len(), 14);
    }

    #[test]
    fn nan_signal_becomes_none() {
        let mode_s = [0x02, 0x00, 0x00, 0xA1, 0xB2, 0xC3, 0x00];
        let data = make_sdr_frame(f32::NAN, &mode_s);

        let mut framer = SdrFramer::new();
        framer.feed(&data);
        let frame = framer.next_frame().unwrap();

        assert!(frame.signal_level.is_none());
    }

    #[test]
    fn too_short_data_ignored() {
        let mut framer = SdrFramer::new();
        framer.feed(&[0x00; 8]); // 4 header + 4 = too short
        assert!(framer.next_frame().is_none());
    }

    #[test]
    fn reset_clears() {
        let mode_s = [0x02, 0x00, 0x00, 0xA1, 0xB2, 0xC3, 0x00];
        let data = make_sdr_frame(-10.0, &mode_s);

        let mut framer = SdrFramer::new();
        framer.feed(&data);
        framer.reset();
        assert!(framer.next_frame().is_none());
    }
}
