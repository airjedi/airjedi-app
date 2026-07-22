use bytes::Bytes;

use super::beast_frame;
use super::{Frame, FrameType, Framer};

#[derive(Debug)]
pub struct BeastFramer {
    inner: beast_frame::FrameDecoder,
}

impl BeastFramer {
    pub fn new() -> Self {
        Self {
            inner: beast_frame::FrameDecoder::new(),
        }
    }
}

impl Default for BeastFramer {
    fn default() -> Self {
        Self::new()
    }
}

impl Framer for BeastFramer {
    fn feed(&mut self, data: &[u8]) {
        self.inner.feed(data);
    }

    fn next_frame(&mut self) -> Option<Frame> {
        let beast = self.inner.next_frame()?;
        let frame_type = match beast.msg_type {
            beast_frame::MessageType::ModeAC => FrameType::ModeAC,
            beast_frame::MessageType::ModeSShort => FrameType::ModeSShort,
            beast_frame::MessageType::ModeSLong => FrameType::ModeSLong,
        };
        Some(Frame {
            timestamp: Some(beast.mlat_timestamp),
            signal_level: Some(beast.signal_level as f32 / 255.0),
            data: Bytes::from(beast.data),
            frame_type,
        })
    }

    fn reset(&mut self) {
        self.inner.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BEAST_ESCAPE: u8 = 0x1A;

    fn make_frame(msg_type: u8, timestamp: &[u8; 6], signal: u8, data: &[u8]) -> Vec<u8> {
        let mut frame = vec![BEAST_ESCAPE, msg_type];
        frame.extend_from_slice(timestamp);
        frame.push(signal);
        for &b in data {
            if b == BEAST_ESCAPE {
                frame.push(BEAST_ESCAPE);
            }
            frame.push(b);
        }
        frame
    }

    #[test]
    fn long_frame_produces_mode_s_long() {
        let data = [0x8D, 0xA1, 0xB2, 0xC3, 0x58, 0xB9, 0x86, 0x50, 0x7B, 0x01, 0x0A, 0x00, 0x12, 0x34];
        let raw = make_frame(0x33, &[0x00; 6], 0x80, &data);

        let mut framer = BeastFramer::new();
        framer.feed(&raw);
        let frame = framer.next_frame().unwrap();

        assert_eq!(frame.frame_type, FrameType::ModeSLong);
        assert_eq!(frame.data.as_ref(), &data);
        assert_eq!(frame.timestamp, Some(0));
        assert!((frame.signal_level.unwrap() - 0x80 as f32 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn short_frame_produces_mode_s_short() {
        let data = [0x02, 0x00, 0x00, 0xA1, 0xB2, 0xC3, 0x00];
        let raw = make_frame(0x32, &[0x00; 6], 0x40, &data);

        let mut framer = BeastFramer::new();
        framer.feed(&raw);
        let frame = framer.next_frame().unwrap();

        assert_eq!(frame.frame_type, FrameType::ModeSShort);
        assert_eq!(frame.data.as_ref(), &data);
    }

    #[test]
    fn mode_ac_frame() {
        let data = [0x12, 0x34];
        let raw = make_frame(0x31, &[0x00; 6], 0x20, &data);

        let mut framer = BeastFramer::new();
        framer.feed(&raw);
        let frame = framer.next_frame().unwrap();

        assert_eq!(frame.frame_type, FrameType::ModeAC);
        assert_eq!(frame.data.as_ref(), &data);
    }

    #[test]
    fn timestamp_preserved() {
        let data = [0x00; 14];
        let ts = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB];
        let raw = make_frame(0x33, &ts, 0x00, &data);

        let mut framer = BeastFramer::new();
        framer.feed(&raw);
        let frame = framer.next_frame().unwrap();

        assert_eq!(frame.timestamp, Some(0x01_2345_6789_AB));
    }

    #[test]
    fn signal_level_normalized() {
        let raw = make_frame(0x33, &[0x00; 6], 0xFF, &[0x00; 14]);

        let mut framer = BeastFramer::new();
        framer.feed(&raw);
        let frame = framer.next_frame().unwrap();

        assert!((frame.signal_level.unwrap() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn multiple_frames() {
        let data1 = [0x8D, 0xA1, 0xB2, 0xC3, 0x58, 0xB9, 0x86, 0x50, 0x7B, 0x01, 0x0A, 0x00, 0x12, 0x34];
        let data2 = [0x02, 0x00, 0x00, 0xA1, 0xB2, 0xC3, 0x00];

        let mut raw = make_frame(0x33, &[0x00; 6], 0x80, &data1);
        raw.extend(make_frame(0x32, &[0x00; 6], 0x40, &data2));

        let mut framer = BeastFramer::new();
        framer.feed(&raw);

        let f1 = framer.next_frame().unwrap();
        assert_eq!(f1.frame_type, FrameType::ModeSLong);
        assert_eq!(f1.data.as_ref(), &data1);

        let f2 = framer.next_frame().unwrap();
        assert_eq!(f2.frame_type, FrameType::ModeSShort);
        assert_eq!(f2.data.as_ref(), &data2);

        assert!(framer.next_frame().is_none());
    }

    #[test]
    fn partial_feed() {
        let data = [0x8D, 0xA1, 0xB2, 0xC3, 0x58, 0xB9, 0x86, 0x50, 0x7B, 0x01, 0x0A, 0x00, 0x12, 0x34];
        let raw = make_frame(0x33, &[0x00; 6], 0x80, &data);

        let mut framer = BeastFramer::new();
        framer.feed(&raw[..10]);
        assert!(framer.next_frame().is_none());

        framer.feed(&raw[10..]);
        let frame = framer.next_frame().unwrap();
        assert_eq!(frame.data.as_ref(), &data);
    }

    #[test]
    fn reset_clears_state() {
        let data = [0x8D, 0xA1, 0xB2, 0xC3, 0x58, 0xB9, 0x86, 0x50, 0x7B, 0x01, 0x0A, 0x00, 0x12, 0x34];
        let raw = make_frame(0x33, &[0x00; 6], 0x80, &data);

        let mut framer = BeastFramer::new();
        framer.feed(&raw[..10]);
        framer.reset();
        assert!(framer.next_frame().is_none());

        framer.feed(&raw);
        assert!(framer.next_frame().is_some());
    }
}
