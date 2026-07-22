use std::collections::{HashMap, HashSet};

use crate::framing::{Frame, FrameType};
use crate::protocol::beast::{adsb, cpr::CprState, modes};
use crate::protocol::{AircraftMessage, Icao, MessagePayload};

use super::Decoder;

/// Decoder that uses the built-in Mode-S/ADS-B decode pipeline.
///
/// Wraps the existing decode logic from `protocol::beast::modes` and
/// `protocol::beast::adsb`, operating on `Frame` input from any framer.
pub struct NativeDecoder {
    cpr_state: HashMap<Icao, CprState>,
    known_icao: HashSet<Icao>,
    reference_position: Option<(f64, f64)>,
}

impl std::fmt::Debug for NativeDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeDecoder")
            .field("known_icao_count", &self.known_icao.len())
            .field("cpr_state_count", &self.cpr_state.len())
            .finish()
    }
}

impl NativeDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cpr_state: HashMap::new(),
            known_icao: HashSet::new(),
            reference_position: None,
        }
    }

    fn decode_short(&mut self, data: &[u8], signal_level: Option<f32>) -> Option<AircraftMessage> {
        if data.len() < 7 {
            return None;
        }

        let df = modes::downlink_format(data);
        let icao = match df {
            11 => {
                if !modes::crc_check(data) {
                    return None;
                }
                let icao = modes::icao_from_bytes(data);
                self.known_icao.insert(icao);
                icao
            }
            0 | 4 | 5 | 16 => {
                let icao = modes::icao_from_parity(data);
                if !self.known_icao.contains(&icao) {
                    return None;
                }
                icao
            }
            _ => return None,
        };

        match df {
            0 | 4 => {
                let altitude = modes::decode_altitude_13bit(data);
                let fs = modes::flight_status(data);
                Some(AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Altitude {
                        altitude,
                        squawk: None,
                        alert: Some(fs.alert),
                        emergency: None,
                        spi: Some(fs.spi),
                        is_on_ground: Some(fs.on_ground),
                    },
                })
            }
            5 => {
                let squawk = modes::decode_identity(data);
                let fs = modes::flight_status(data);
                Some(AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Altitude {
                        altitude: None,
                        squawk: Some(squawk),
                        alert: Some(fs.alert),
                        emergency: None,
                        spi: Some(fs.spi),
                        is_on_ground: Some(fs.on_ground),
                    },
                })
            }
            11 => Some(AircraftMessage {
                icao,
                signal_level,
                payload: MessagePayload::Altitude {
                    altitude: None,
                    squawk: None,
                    alert: None,
                    emergency: None,
                    spi: None,
                    is_on_ground: None,
                },
            }),
            _ => None,
        }
    }

    fn decode_long(&mut self, data: &[u8], signal_level: Option<f32>) -> Option<AircraftMessage> {
        if data.len() < 14 {
            return None;
        }

        let df = modes::downlink_format(data);

        match df {
            17 | 18 => {
                if !modes::crc_check(data) {
                    return None;
                }
                let icao = modes::icao_from_bytes(data);
                self.known_icao.insert(icao);

                let me = &data[4..11];
                let tc = me[0] >> 3;

                let payload = adsb::decode_adsb(
                    tc,
                    me,
                    icao,
                    &mut self.cpr_state,
                    self.reference_position,
                )
                .ok()?;

                payload.map(|p| AircraftMessage {
                    icao,
                    signal_level,
                    payload: p,
                })
            }
            16 => {
                let icao = modes::icao_from_parity(data);
                if !self.known_icao.contains(&icao) {
                    return None;
                }
                let altitude = modes::decode_altitude_13bit(data);
                Some(AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Altitude {
                        altitude,
                        squawk: None,
                        alert: None,
                        emergency: None,
                        spi: None,
                        is_on_ground: None,
                    },
                })
            }
            20 => {
                let icao = modes::icao_from_parity(data);
                if !self.known_icao.contains(&icao) {
                    return None;
                }
                let altitude = modes::decode_altitude_13bit(data);
                let fs = modes::flight_status(data);

                let bds_msg = modes::decode_bds(&data[4..11], icao, signal_level);
                if let Some(msg) = bds_msg {
                    return Some(msg);
                }

                Some(AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Altitude {
                        altitude,
                        squawk: None,
                        alert: Some(fs.alert),
                        emergency: None,
                        spi: Some(fs.spi),
                        is_on_ground: Some(fs.on_ground),
                    },
                })
            }
            21 => {
                let icao = modes::icao_from_parity(data);
                if !self.known_icao.contains(&icao) {
                    return None;
                }
                let squawk = modes::decode_identity(data);
                let fs = modes::flight_status(data);

                let bds_msg = modes::decode_bds(&data[4..11], icao, signal_level);
                if let Some(msg) = bds_msg {
                    return Some(msg);
                }

                Some(AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Altitude {
                        altitude: None,
                        squawk: Some(squawk),
                        alert: Some(fs.alert),
                        emergency: None,
                        spi: Some(fs.spi),
                        is_on_ground: Some(fs.on_ground),
                    },
                })
            }
            19 => {
                if !modes::crc_check(data) {
                    return None;
                }
                let icao = modes::icao_from_bytes(data);
                self.known_icao.insert(icao);
                Some(AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Altitude {
                        altitude: None,
                        squawk: None,
                        alert: None,
                        emergency: None,
                        spi: None,
                        is_on_ground: None,
                    },
                })
            }
            _ => None,
        }
    }
}

impl Default for NativeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for NativeDecoder {
    fn decode(&mut self, frame: &Frame) -> Vec<AircraftMessage> {
        match frame.frame_type {
            FrameType::ModeAC | FrameType::TextLine => vec![],
            FrameType::ModeSShort => self
                .decode_short(&frame.data, frame.signal_level)
                .into_iter()
                .collect(),
            FrameType::ModeSLong => self
                .decode_long(&frame.data, frame.signal_level)
                .into_iter()
                .collect(),
        }
    }

    fn set_reference_position(&mut self, lat: f64, lon: f64) {
        self.reference_position = Some((lat, lon));
    }

    fn reset(&mut self) {
        self.cpr_state.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn make_frame(data: &[u8], frame_type: FrameType, signal_level: Option<f32>) -> Frame {
        Frame {
            timestamp: None,
            signal_level,
            data: Bytes::copy_from_slice(data),
            frame_type,
        }
    }

    #[test]
    fn mode_ac_returns_empty() {
        let mut decoder = NativeDecoder::new();
        let frame = make_frame(&[0x00, 0x00], FrameType::ModeAC, None);
        assert!(decoder.decode(&frame).is_empty());
    }

    #[test]
    fn text_line_returns_empty() {
        let mut decoder = NativeDecoder::new();
        let frame = make_frame(b"MSG,3,1,1,A1B2C3,...", FrameType::TextLine, None);
        assert!(decoder.decode(&frame).is_empty());
    }

    #[test]
    fn short_frame_too_small() {
        let mut decoder = NativeDecoder::new();
        let frame = make_frame(&[0x5D, 0xA1, 0xB2], FrameType::ModeSShort, None);
        assert!(decoder.decode(&frame).is_empty());
    }

    #[test]
    fn long_frame_too_small() {
        let mut decoder = NativeDecoder::new();
        let frame = make_frame(&[0x8D; 7], FrameType::ModeSLong, None);
        assert!(decoder.decode(&frame).is_empty());
    }

    #[test]
    fn df11_all_call_registers_icao() {
        let mut decoder = NativeDecoder::new();

        // Build a DF=11 message with valid CRC for ICAO A1B2C3.
        // DF=11 -> first byte top 5 bits = 01011 -> 0x5D with CA=101 -> 0x5D
        let mut msg = [0u8; 7];
        msg[0] = 0x5D; // DF=11, CA=5
        msg[1] = 0xA1;
        msg[2] = 0xB2;
        msg[3] = 0xC3;
        // Compute CRC-24 over first 4 bytes, set last 3 bytes to make remainder = 0
        let crc = compute_crc24(&msg[..4]);
        msg[4] = (crc >> 16) as u8;
        msg[5] = (crc >> 8) as u8;
        msg[6] = crc as u8;

        let frame = make_frame(&msg, FrameType::ModeSShort, Some(0.5));
        let msgs = decoder.decode(&frame);

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].icao, Icao(0xA1B2C3));
        assert_eq!(msgs[0].signal_level, Some(0.5));
        assert!(decoder.known_icao.contains(&Icao(0xA1B2C3)));
    }

    #[test]
    fn df4_altitude_reply_requires_known_icao() {
        let mut decoder = NativeDecoder::new();

        // DF=4 with unknown ICAO should return empty
        let msg = [0x20, 0x00, 0x1A, 0xE0, 0x00, 0x00, 0x00];
        let frame = make_frame(&msg, FrameType::ModeSShort, None);
        assert!(decoder.decode(&frame).is_empty());

        // Pre-register the ICAO that the parity field would resolve to
        let icao = modes::icao_from_parity(&msg);
        decoder.known_icao.insert(icao);

        let msgs = decoder.decode(&frame);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].icao, icao);
    }

    #[test]
    fn df17_adsb_identification() {
        let mut decoder = NativeDecoder::new();

        // DF=17 ADS-B identification message (TC=4)
        // ICAO = A1B2C3, callsign "UAL123  "
        // U=21, A=1, L=12, 1=49, 2=50, 3=51, ' '=32, ' '=32
        let mut msg = [0u8; 14];
        msg[0] = 0x8D; // DF=17, CA=5
        msg[1] = 0xA1;
        msg[2] = 0xB2;
        msg[3] = 0xC3;
        // ME field: TC=4, CA=0 -> 0x20
        msg[4] = 0x20;
        // 8 chars x 6 bits = 48 bits in bytes 5-10
        // U(21)=010101 A(1)=000001 L(12)=001100 1(49)=110001
        // 2(50)=110010 3(51)=110011 ' '(32)=100000 ' '(32)=100000
        // Combined: 010101_000001_001100_110001_110010_110011_100000_100000
        // Byte boundary alignment (48 bits = 6 bytes):
        // 01010100 00010011 00110001 11001011 00111000 00100000
        msg[5] = 0x54;
        msg[6] = 0x13;
        msg[7] = 0x31;
        msg[8] = 0xCB;
        msg[9] = 0x38;
        msg[10] = 0x20;

        // Compute CRC and set last 3 bytes
        let crc = compute_crc24(&msg[..11]);
        msg[11] = (crc >> 16) as u8;
        msg[12] = (crc >> 8) as u8;
        msg[13] = crc as u8;

        let frame = make_frame(&msg, FrameType::ModeSLong, Some(0.8));
        let msgs = decoder.decode(&frame);

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].icao, Icao(0xA1B2C3));
        if let MessagePayload::Identification { ref callsign, .. } = msgs[0].payload {
            assert_eq!(callsign, "UAL123");
        } else {
            panic!("expected Identification payload");
        }
    }

    #[test]
    fn df19_military_extended_squitter() {
        let mut decoder = NativeDecoder::new();

        // DF=19 -> 0x98 with last 3 bits=0
        let mut msg = [0u8; 14];
        msg[0] = 0x98; // DF=19
        msg[1] = 0xA1;
        msg[2] = 0xB2;
        msg[3] = 0xC3;

        let crc = compute_crc24(&msg[..11]);
        msg[11] = (crc >> 16) as u8;
        msg[12] = (crc >> 8) as u8;
        msg[13] = crc as u8;

        let frame = make_frame(&msg, FrameType::ModeSLong, None);
        let msgs = decoder.decode(&frame);

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].icao, Icao(0xA1B2C3));
    }

    #[test]
    fn reset_clears_cpr_but_keeps_icao() {
        let mut decoder = NativeDecoder::new();
        decoder.known_icao.insert(Icao(0xABCDEF));
        decoder
            .cpr_state
            .insert(Icao(0xABCDEF), CprState::new());

        decoder.reset();

        assert!(decoder.cpr_state.is_empty());
        assert!(decoder.known_icao.contains(&Icao(0xABCDEF)));
    }

    #[test]
    fn set_reference_position_stored() {
        let mut decoder = NativeDecoder::new();
        decoder.set_reference_position(37.6872, -97.3301);
        assert_eq!(decoder.reference_position, Some((37.6872, -97.3301)));
    }

    #[test]
    fn default_trait() {
        let decoder = NativeDecoder::default();
        assert!(decoder.known_icao.is_empty());
        assert!(decoder.cpr_state.is_empty());
        assert!(decoder.reference_position.is_none());
    }

    /// Compute CRC-24 over data bytes (without the trailing PI field).
    fn compute_crc24(data: &[u8]) -> u32 {
        const CRC_GENERATOR: u32 = 0x1FFF409;
        let mut crc: u32 = 0;
        for &byte in data {
            crc ^= (byte as u32) << 16;
            for _ in 0..8 {
                crc <<= 1;
                if crc & 0x100_0000 != 0 {
                    crc ^= CRC_GENERATOR;
                }
            }
        }
        crc & 0xFF_FFFF
    }
}
