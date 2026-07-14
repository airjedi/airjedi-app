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

//! BEAST binary protocol parser.
//!
//! Decodes Mode-S Beast binary frames from port 30005, including:
//! - Binary frame extraction with 0x1A escape handling
//! - Mode-S downlink format decoding (DF=0/4/5/11/16/17/18/20/21)
//! - ADS-B Extended Squitter decoding (TC 1-4, 5-8, 9-18, 19, 20-22, 28)
//! - CPR (Compact Position Reporting) global and local position decoding
//! - CRC-24 validation and ICAO address extraction from parity
//! - BDS (Binary Data Store) heuristic decoding from Comm-B replies
//! - Signal level and MLAT timestamp extraction

mod adsb;
mod cpr;
mod frame;
mod modes;

use std::collections::{HashMap, HashSet};

use crate::protocol::{AircraftMessage, MessagePayload, ParseError, Protocol};
use cpr::CprState;
use frame::FrameDecoder;

/// Parser for BEAST binary protocol (Mode-S Beast, dump1090 port 30005).
///
/// Maintains internal state for:
/// - Byte buffer for incomplete frame reassembly
/// - CPR decode state per aircraft (odd/even frame pairs)
/// - Known ICAO address set (for parity-based extraction from surveillance replies)
/// - Reference position for local CPR decode
pub struct BeastParser {
    frame_decoder: FrameDecoder,
    cpr_state: HashMap<String, CprState>,
    known_icao: HashSet<String>,
    reference_position: Option<(f64, f64)>,
}

impl std::fmt::Debug for BeastParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BeastParser")
            .field("buffer_len", &self.frame_decoder.buffer_len())
            .field("known_icao_count", &self.known_icao.len())
            .field("cpr_state_count", &self.cpr_state.len())
            .finish()
    }
}

impl BeastParser {
    #[must_use]
    pub fn new() -> Self {
        Self {
            frame_decoder: FrameDecoder::new(),
            cpr_state: HashMap::new(),
            known_icao: HashSet::new(),
            reference_position: None,
        }
    }

    pub fn set_reference_position(&mut self, lat: f64, lon: f64) {
        self.reference_position = Some((lat, lon));
    }

    fn decode_frame(&mut self, beast_frame: &frame::BeastFrame) -> Result<Option<AircraftMessage>, ParseError> {
        let signal_level = Some(beast_frame.signal_level as f32 / 255.0);

        match beast_frame.msg_type {
            frame::MessageType::ModeAC => {
                Ok(None)
            }
            frame::MessageType::ModeSShort => {
                self.decode_modes_short(&beast_frame.data, signal_level)
            }
            frame::MessageType::ModeSLong => {
                self.decode_modes_long(&beast_frame.data, signal_level)
            }
        }
    }

    fn decode_modes_short(&mut self, data: &[u8], signal_level: Option<f32>) -> Result<Option<AircraftMessage>, ParseError> {
        if data.len() < 7 {
            return Ok(None);
        }

        let df = modes::downlink_format(data);
        let icao = match df {
            11 => {
                if !modes::crc_check(data) {
                    return Ok(None);
                }
                let icao = modes::icao_from_bytes(data);
                self.known_icao.insert(icao.clone());
                icao
            }
            0 | 4 | 5 | 16 => {
                let icao = modes::icao_from_parity(data);
                if !self.known_icao.contains(&icao) {
                    return Ok(None);
                }
                icao
            }
            _ => return Ok(None),
        };

        match df {
            0 | 4 => {
                let altitude = modes::decode_altitude_13bit(data);
                let fs = modes::flight_status(data);
                Ok(Some(AircraftMessage {
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
                }))
            }
            5 => {
                let squawk = modes::decode_identity(data);
                let fs = modes::flight_status(data);
                Ok(Some(AircraftMessage {
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
                }))
            }
            11 => {
                Ok(Some(AircraftMessage {
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
                }))
            }
            _ => Ok(None),
        }
    }

    fn decode_modes_long(&mut self, data: &[u8], signal_level: Option<f32>) -> Result<Option<AircraftMessage>, ParseError> {
        if data.len() < 14 {
            return Ok(None);
        }

        let df = modes::downlink_format(data);

        match df {
            17 | 18 => {
                if !modes::crc_check(data) {
                    return Ok(None);
                }
                let icao = modes::icao_from_bytes(data);
                self.known_icao.insert(icao.clone());

                let me = &data[4..11];
                let tc = me[0] >> 3;

                let payload = adsb::decode_adsb(
                    tc,
                    me,
                    &icao,
                    &mut self.cpr_state,
                    self.reference_position,
                )?;

                match payload {
                    Some(p) => Ok(Some(AircraftMessage { icao, signal_level, payload: p })),
                    None => Ok(None),
                }
            }
            16 => {
                let icao = modes::icao_from_parity(data);
                if !self.known_icao.contains(&icao) {
                    return Ok(None);
                }
                let altitude = modes::decode_altitude_13bit(data);
                Ok(Some(AircraftMessage {
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
                }))
            }
            20 => {
                let icao = modes::icao_from_parity(data);
                if !self.known_icao.contains(&icao) {
                    return Ok(None);
                }
                let altitude = modes::decode_altitude_13bit(data);
                let fs = modes::flight_status(data);

                let bds_msg = modes::decode_bds(&data[4..11], &icao, signal_level);
                if let Some(msg) = bds_msg {
                    return Ok(Some(msg));
                }

                Ok(Some(AircraftMessage {
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
                }))
            }
            21 => {
                let icao = modes::icao_from_parity(data);
                if !self.known_icao.contains(&icao) {
                    return Ok(None);
                }
                let squawk = modes::decode_identity(data);
                let fs = modes::flight_status(data);

                let bds_msg = modes::decode_bds(&data[4..11], &icao, signal_level);
                if let Some(msg) = bds_msg {
                    return Ok(Some(msg));
                }

                Ok(Some(AircraftMessage {
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
                }))
            }
            _ => Ok(None),
        }
    }
}

impl Default for BeastParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for BeastParser {
    type Message = AircraftMessage;
    type Error = ParseError;

    fn parse(&mut self, input: &[u8]) -> Result<Option<AircraftMessage>, ParseError> {
        if !input.is_empty() {
            self.frame_decoder.feed(input);
        }

        while let Some(beast_frame) = self.frame_decoder.next_frame() {
            match self.decode_frame(&beast_frame) {
                Ok(Some(msg)) => return Ok(Some(msg)),
                Ok(None) => continue,
                Err(e) => {
                    log::warn!("BEAST decode error: {}", e);
                    continue;
                }
            }
        }

        Ok(None)
    }
}
