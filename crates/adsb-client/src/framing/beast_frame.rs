// Copyright 2025 Chris Custine
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! BEAST binary frame extraction.
//!
//! Handles the 0x1A-escaped binary framing used by Mode-S Beast receivers
//! and dump1090/readsb on port 30005.
//!
//! Frame format: [0x1A] [type] [6 bytes MLAT timestamp] [1 byte signal] [N bytes Mode-S data]
//! - Type 0x31 ('1'): Mode-AC, 2 bytes data
//! - Type 0x32 ('2'): Mode-S Short, 7 bytes data
//! - Type 0x33 ('3'): Mode-S Long, 14 bytes data
//! - Type 0x34 ('4'): Status (implementation-specific, skipped)
//!
//! Literal 0x1A bytes within the payload are escaped as 0x1A 0x1A.

const BEAST_ESCAPE: u8 = 0x1A;

/// BEAST message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    /// Mode-A/C reply (2 bytes data)
    ModeAC,
    /// Mode-S Short (7 bytes data, 56-bit)
    ModeSShort,
    /// Mode-S Long (14 bytes data, 112-bit)
    ModeSLong,
}

impl MessageType {
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x31 => Some(Self::ModeAC),
            0x32 => Some(Self::ModeSShort),
            0x33 => Some(Self::ModeSLong),
            _ => None,
        }
    }

    fn data_len(self) -> usize {
        match self {
            Self::ModeAC => 2,
            Self::ModeSShort => 7,
            Self::ModeSLong => 14,
        }
    }

    fn total_payload_len(self) -> usize {
        6 + 1 + self.data_len() // MLAT timestamp + signal level + data
    }
}

/// A decoded BEAST frame.
#[derive(Debug, Clone)]
pub struct BeastFrame {
    pub msg_type: MessageType,
    pub mlat_timestamp: u64,
    pub signal_level: u8,
    pub data: Vec<u8>,
}

/// Stateful BEAST frame decoder with internal buffer.
#[derive(Debug)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
        }
    }

    pub fn feed(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    #[allow(dead_code)]
    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    /// Try to extract the next complete frame from the buffer.
    pub fn next_frame(&mut self) -> Option<BeastFrame> {
        loop {
            // Find the start byte
            let start = self.buffer.iter().position(|&b| b == BEAST_ESCAPE)?;

            // Skip any leading garbage before the escape
            if start > 0 {
                self.buffer.drain(..start);
            }

            // Skip consecutive 0x1A bytes (sync preamble)
            let mut pos = 0;
            while pos < self.buffer.len() && self.buffer[pos] == BEAST_ESCAPE {
                pos += 1;
            }

            if pos >= self.buffer.len() {
                return None;
            }

            let type_byte = self.buffer[pos];

            // 0x34 is a status frame - skip the escape+type and try again
            if type_byte == 0x34 {
                self.buffer.drain(..pos + 1);
                // Status frames have variable length - try to find next 0x1A start
                continue;
            }

            let msg_type = match MessageType::from_byte(type_byte) {
                Some(t) => t,
                None => {
                    // Unknown type byte - skip past it and try again
                    self.buffer.drain(..pos + 1);
                    continue;
                }
            };

            let payload_len = msg_type.total_payload_len();

            // Try to unescape the payload bytes after the type byte
            let payload_start = pos + 1;
            let mut unescaped = Vec::with_capacity(payload_len);
            let mut src = payload_start;

            while unescaped.len() < payload_len {
                if src >= self.buffer.len() {
                    return None; // Need more data
                }

                let b = self.buffer[src];
                src += 1;

                if b == BEAST_ESCAPE {
                    if src >= self.buffer.len() {
                        return None; // Need more data to determine if escape or new frame
                    }
                    let next = self.buffer[src];
                    if next == BEAST_ESCAPE {
                        // Escaped literal 0x1A
                        unescaped.push(BEAST_ESCAPE);
                        src += 1;
                    } else {
                        // This is the start of a new frame - current frame is truncated
                        self.buffer.drain(..src - 1);
                        break;
                    }
                } else {
                    unescaped.push(b);
                }
            }

            if unescaped.len() < payload_len {
                // Frame was truncated (hit a new 0x1A + type before completing)
                continue;
            }

            // Successfully extracted a complete frame
            self.buffer.drain(..src);

            let mlat_timestamp = u64::from(unescaped[0]) << 40
                | u64::from(unescaped[1]) << 32
                | u64::from(unescaped[2]) << 24
                | u64::from(unescaped[3]) << 16
                | u64::from(unescaped[4]) << 8
                | u64::from(unescaped[5]);

            let signal_level = unescaped[6];

            let data = unescaped[7..].to_vec();

            return Some(BeastFrame {
                msg_type,
                mlat_timestamp,
                signal_level,
                data,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_simple_long_frame() {
        let data = [0x8D, 0xA1, 0xB2, 0xC3, 0x58, 0xB9, 0x86, 0x50, 0x7B, 0x01, 0x0A, 0x00, 0x12, 0x34];
        let raw = make_frame(0x33, &[0x00; 6], 0x80, &data);

        let mut decoder = FrameDecoder::new();
        decoder.feed(&raw);
        let frame = decoder.next_frame().unwrap();

        assert_eq!(frame.msg_type, MessageType::ModeSLong);
        assert_eq!(frame.signal_level, 0x80);
        assert_eq!(frame.data, data);
    }

    #[test]
    fn test_short_frame() {
        let data = [0x02, 0x00, 0x00, 0xA1, 0xB2, 0xC3, 0x00];
        let raw = make_frame(0x32, &[0x00; 6], 0x40, &data);

        let mut decoder = FrameDecoder::new();
        decoder.feed(&raw);
        let frame = decoder.next_frame().unwrap();

        assert_eq!(frame.msg_type, MessageType::ModeSShort);
        assert_eq!(frame.data, data);
    }

    #[test]
    fn test_mode_ac_frame() {
        let data = [0x12, 0x34];
        let raw = make_frame(0x31, &[0x00; 6], 0x20, &data);

        let mut decoder = FrameDecoder::new();
        decoder.feed(&raw);
        let frame = decoder.next_frame().unwrap();

        assert_eq!(frame.msg_type, MessageType::ModeAC);
        assert_eq!(frame.data, data);
    }

    #[test]
    fn test_escape_in_payload() {
        // Data contains a literal 0x1A that must be escaped in the wire format
        let expected_data = [0x8D, 0x1A, 0xB2, 0xC3, 0x58, 0xB9, 0x86, 0x50, 0x7B, 0x01, 0x0A, 0x00, 0x12, 0x34];
        let raw = make_frame(0x33, &[0x00; 6], 0x80, &expected_data);

        let mut decoder = FrameDecoder::new();
        decoder.feed(&raw);
        let frame = decoder.next_frame().unwrap();

        assert_eq!(frame.data, expected_data);
    }

    #[test]
    fn test_multiple_frames() {
        let data1 = [0x8D, 0xA1, 0xB2, 0xC3, 0x58, 0xB9, 0x86, 0x50, 0x7B, 0x01, 0x0A, 0x00, 0x12, 0x34];
        let data2 = [0x02, 0x00, 0x00, 0xA1, 0xB2, 0xC3, 0x00];

        let mut raw = make_frame(0x33, &[0x00; 6], 0x80, &data1);
        raw.extend(make_frame(0x32, &[0x00; 6], 0x40, &data2));

        let mut decoder = FrameDecoder::new();
        decoder.feed(&raw);

        let frame1 = decoder.next_frame().unwrap();
        assert_eq!(frame1.msg_type, MessageType::ModeSLong);
        assert_eq!(frame1.data, data1);

        let frame2 = decoder.next_frame().unwrap();
        assert_eq!(frame2.msg_type, MessageType::ModeSShort);
        assert_eq!(frame2.data, data2);

        assert!(decoder.next_frame().is_none());
    }

    #[test]
    fn test_partial_frame() {
        let data = [0x8D, 0xA1, 0xB2, 0xC3, 0x58, 0xB9, 0x86, 0x50, 0x7B, 0x01, 0x0A, 0x00, 0x12, 0x34];
        let raw = make_frame(0x33, &[0x00; 6], 0x80, &data);

        let mut decoder = FrameDecoder::new();

        // Feed only first half
        decoder.feed(&raw[..10]);
        assert!(decoder.next_frame().is_none());

        // Feed the rest
        decoder.feed(&raw[10..]);
        let frame = decoder.next_frame().unwrap();
        assert_eq!(frame.data, data);
    }

    #[test]
    fn test_mlat_timestamp() {
        let data = [0x00; 14];
        let ts = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB];
        let raw = make_frame(0x33, &ts, 0x00, &data);

        let mut decoder = FrameDecoder::new();
        decoder.feed(&raw);
        let frame = decoder.next_frame().unwrap();

        assert_eq!(frame.mlat_timestamp, 0x01_2345_6789_AB);
    }

    #[test]
    fn test_garbage_before_frame() {
        let data = [0x00; 7];
        let mut raw = vec![0xFF, 0xFE, 0xFD]; // garbage
        raw.extend(make_frame(0x32, &[0x00; 6], 0x10, &data));

        let mut decoder = FrameDecoder::new();
        decoder.feed(&raw);
        let frame = decoder.next_frame().unwrap();
        assert_eq!(frame.msg_type, MessageType::ModeSShort);
    }

    #[test]
    fn test_consecutive_escape_preamble() {
        let data = [0x00; 7];
        let mut raw = vec![BEAST_ESCAPE, BEAST_ESCAPE, BEAST_ESCAPE]; // extra 0x1A sync
        // Then the actual type byte
        raw.push(0x32);
        raw.extend_from_slice(&[0x00; 6]); // timestamp
        raw.push(0x10); // signal
        raw.extend_from_slice(&data); // data

        let mut decoder = FrameDecoder::new();
        decoder.feed(&raw);
        let frame = decoder.next_frame().unwrap();
        assert_eq!(frame.msg_type, MessageType::ModeSShort);
    }
}
