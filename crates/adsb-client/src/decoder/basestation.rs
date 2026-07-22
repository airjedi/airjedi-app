use log::warn;

use crate::framing::{Frame, FrameType};
use crate::protocol::{AircraftMessage, BaseStationParser, Protocol};

use super::Decoder;

/// Decoder for BaseStation/SBS-1 text frames.
///
/// Handles `TextLine` frames by parsing the CSV content using `BaseStationParser`.
/// All other frame types return an empty vec.
pub struct BaseStationDecoder {
    parser: BaseStationParser,
}

impl std::fmt::Debug for BaseStationDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BaseStationDecoder").finish()
    }
}

impl BaseStationDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            parser: BaseStationParser::new(),
        }
    }
}

impl Default for BaseStationDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for BaseStationDecoder {
    fn decode(&mut self, frame: &Frame) -> Vec<AircraftMessage> {
        if frame.frame_type != FrameType::TextLine {
            return vec![];
        }

        match self.parser.parse(&frame.data) {
            Ok(Some(msg)) => vec![msg],
            Ok(None) => vec![],
            Err(e) => {
                warn!("BaseStation parse error: {}", e);
                vec![]
            }
        }
    }

    fn set_reference_position(&mut self, _lat: f64, _lon: f64) {
        // BaseStation messages contain absolute positions; no reference needed.
    }

    fn reset(&mut self) {
        self.parser.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn text_frame(line: &str) -> Frame {
        Frame {
            timestamp: None,
            signal_level: None,
            data: Bytes::copy_from_slice(line.as_bytes()),
            frame_type: FrameType::TextLine,
        }
    }

    fn binary_frame(data: &[u8], frame_type: FrameType) -> Frame {
        Frame {
            timestamp: None,
            signal_level: None,
            data: Bytes::copy_from_slice(data),
            frame_type,
        }
    }

    #[test]
    fn decodes_identification() {
        let mut decoder = BaseStationDecoder::new();
        let frame = text_frame(
            "MSG,1,1,1,A1B2C3,1,2024/01/01,12:00:00.000,2024/01/01,12:00:00.000,UAL123",
        );
        let msgs = decoder.decode(&frame);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].icao.0, 0xA1B2C3);
    }

    #[test]
    fn decodes_position() {
        let mut decoder = BaseStationDecoder::new();
        let frame = text_frame(
            "MSG,3,1,1,A1B2C3,1,2024/01/01,12:00:00.000,2024/01/01,12:00:00.000,,35000,,,33.9425,-118.4081,",
        );
        let msgs = decoder.decode(&frame);
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn ignores_binary_frames() {
        let mut decoder = BaseStationDecoder::new();
        let frame = binary_frame(&[0x8D; 14], FrameType::ModeSLong);
        assert!(decoder.decode(&frame).is_empty());

        let frame = binary_frame(&[0x5D; 7], FrameType::ModeSShort);
        assert!(decoder.decode(&frame).is_empty());

        let frame = binary_frame(&[0x00, 0x00], FrameType::ModeAC);
        assert!(decoder.decode(&frame).is_empty());
    }

    #[test]
    fn ignores_non_msg_lines() {
        let mut decoder = BaseStationDecoder::new();
        let frame = text_frame("STA,1,1,1,A1B2C3");
        assert!(decoder.decode(&frame).is_empty());
    }

    #[test]
    fn ignores_empty_lines() {
        let mut decoder = BaseStationDecoder::new();
        let frame = text_frame("");
        assert!(decoder.decode(&frame).is_empty());
    }

    #[test]
    fn default_trait() {
        let decoder = BaseStationDecoder::default();
        assert!(format!("{:?}", decoder).contains("BaseStationDecoder"));
    }
}
