use std::collections::{BTreeMap, HashSet};

use rs1090::decode::bds::bds05::Source;
use rs1090::decode::bds::bds09::AirborneVelocitySubType;
use rs1090::decode::bds::bds61::EmergencyState;
use rs1090::decode::adsb::ME;
use rs1090::decode::cpr::{AircraftState, Position, decode_position, update_global_reference};
use rs1090::decode::{DF, FlightStatus, ICAO, Message};

use crate::framing::{Frame, FrameType};
use crate::protocol::{AircraftMessage, Icao, MessagePayload};
use super::Decoder;
use super::rs1090_mapping;

pub struct Rs1090Decoder {
    known_icao: HashSet<Icao>,
    aircraft_state: BTreeMap<ICAO, AircraftState>,
    reference: Option<Position>,
    decode_count: u64,
}

impl std::fmt::Debug for Rs1090Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rs1090Decoder")
            .field("known_icao_count", &self.known_icao.len())
            .field("aircraft_state_count", &self.aircraft_state.len())
            .finish()
    }
}

fn fs_is_alert(fs: &FlightStatus) -> bool {
    matches!(
        fs,
        FlightStatus::AlertNoSpiAirborne
            | FlightStatus::AlertNoSpiOnGround
            | FlightStatus::AlertSpiAirborneGround
    )
}

fn fs_is_spi(fs: &FlightStatus) -> bool {
    matches!(
        fs,
        FlightStatus::AlertSpiAirborneGround | FlightStatus::NoAlertSpiAirborneGround
    )
}

fn fs_is_on_ground(fs: &FlightStatus) -> bool {
    matches!(
        fs,
        FlightStatus::NoAlertNoSpiOnGround | FlightStatus::AlertNoSpiOnGround
    )
}

impl Rs1090Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            known_icao: HashSet::new(),
            aircraft_state: BTreeMap::new(),
            reference: None,
            decode_count: 0,
        }
    }

    fn extract_icao_and_validate(&mut self, msg: &Message) -> Option<Icao> {
        match &msg.df {
            DF::AllCallReply { icao, .. } => {
                let addr = Icao(icao.0);
                self.known_icao.insert(addr);
                Some(addr)
            }
            DF::ExtendedSquitterADSB(adsb) => {
                let addr = Icao(adsb.icao24.0);
                self.known_icao.insert(addr);
                Some(addr)
            }
            DF::ExtendedSquitterTisB { cf, .. } => {
                let addr = Icao(cf.aa.0);
                self.known_icao.insert(addr);
                Some(addr)
            }
            _ => {
                let addr = Icao::from_parity(msg.crc);
                if self.known_icao.contains(&addr) {
                    Some(addr)
                } else {
                    None
                }
            }
        }
    }

    fn timestamp(&self) -> f64 {
        let now = chrono::Utc::now();
        now.timestamp() as f64 + f64::from(now.timestamp_subsec_millis()) / 1000.0
    }

    fn decode_modes(&mut self, data: &[u8], signal_level: Option<f32>) -> Vec<AircraftMessage> {
        let mut msg = match Message::try_from(data) {
            Ok(m) => m,
            Err(_) => return vec![],
        };

        let icao = match self.extract_icao_and_validate(&msg) {
            Some(i) => i,
            None => return vec![],
        };

        let rs_icao = ICAO(icao.0);
        let ts = self.timestamp();

        match &mut msg.df {
            DF::ShortAirAirSurveillance { ac, .. } => {
                vec![AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Altitude {
                        altitude: ac.0,
                        squawk: None,
                        alert: None,
                        emergency: None,
                        spi: None,
                        is_on_ground: None,
                    },
                }]
            }
            DF::SurveillanceAltitudeReply { ac, fs, .. } => {
                vec![AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Altitude {
                        altitude: ac.0,
                        squawk: None,
                        alert: Some(fs_is_alert(fs)),
                        emergency: None,
                        spi: Some(fs_is_spi(fs)),
                        is_on_ground: Some(fs_is_on_ground(fs)),
                    },
                }]
            }
            DF::SurveillanceIdentityReply { id, fs, .. } => {
                vec![AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Altitude {
                        altitude: None,
                        squawk: Some(format!("{:04o}", id.0)),
                        alert: Some(fs_is_alert(fs)),
                        emergency: None,
                        spi: Some(fs_is_spi(fs)),
                        is_on_ground: Some(fs_is_on_ground(fs)),
                    },
                }]
            }
            DF::AllCallReply { .. } => {
                vec![AircraftMessage {
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
                }]
            }
            DF::LongAirAirSurveillance { ac, .. } => {
                vec![AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Altitude {
                        altitude: ac.0,
                        squawk: None,
                        alert: None,
                        emergency: None,
                        spi: None,
                        is_on_ground: None,
                    },
                }]
            }
            DF::ExtendedSquitterADSB(adsb) => {
                self.decode_and_extract(&mut adsb.message, ICAO(adsb.icao24.0), icao, signal_level, ts)
            }
            DF::ExtendedSquitterTisB { cf, .. } => {
                self.decode_and_extract(&mut cf.me, ICAO(cf.aa.0), icao, signal_level, ts)
            }
            DF::ExtendedSquitterMilitary { .. } => {
                vec![AircraftMessage {
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
                }]
            }
            DF::CommBAltitudeReply { ac, bds, fs, .. } => {
                let raw_mb = if data.len() >= 11 { &data[4..11] } else { &[] };
                let mut msgs = self.decode_commb_bds(bds, icao, signal_level, raw_mb);
                if msgs.is_empty() {
                    msgs.push(AircraftMessage {
                        icao,
                        signal_level,
                        payload: MessagePayload::Altitude {
                            altitude: ac.0,
                            squawk: None,
                            alert: Some(fs_is_alert(fs)),
                            emergency: None,
                            spi: Some(fs_is_spi(fs)),
                            is_on_ground: Some(fs_is_on_ground(fs)),
                        },
                    });
                }
                msgs
            }
            DF::CommBIdentityReply { id, bds, fs, .. } => {
                let raw_mb = if data.len() >= 11 { &data[4..11] } else { &[] };
                let mut msgs = self.decode_commb_bds(bds, icao, signal_level, raw_mb);
                if msgs.is_empty() {
                    msgs.push(AircraftMessage {
                        icao,
                        signal_level,
                        payload: MessagePayload::Altitude {
                            altitude: None,
                            squawk: Some(format!("{:04o}", id.0)),
                            alert: Some(fs_is_alert(fs)),
                            emergency: None,
                            spi: Some(fs_is_spi(fs)),
                            is_on_ground: Some(fs_is_on_ground(fs)),
                        },
                    });
                }
                msgs
            }
            DF::CommDExtended { .. } => vec![],
        }
    }

    fn decode_and_extract(
        &mut self,
        me: &mut ME,
        rs_icao: ICAO,
        icao: Icao,
        signal_level: Option<f32>,
        ts: f64,
    ) -> Vec<AircraftMessage> {
        decode_position(
            me,
            ts,
            &rs_icao,
            &mut self.aircraft_state,
            &mut self.reference,
            &None,
        );
        self.decode_count += 1;
        self.maybe_update_global_reference(ts);
        self.extract_adsb_messages(icao, signal_level, me)
    }

    fn extract_adsb_messages(
        &self,
        icao: Icao,
        signal_level: Option<f32>,
        me: &ME,
    ) -> Vec<AircraftMessage> {
        match me {
            ME::BDS08 { inner, .. } => {
                let callsign = inner.callsign.trim().to_string();
                if callsign.is_empty() {
                    return vec![];
                }
                vec![AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Identification {
                        callsign,
                        category: None,
                    },
                }]
            }
            ME::BDS05 { inner, .. } => {
                let altitude_gnss = if inner.source == Source::Gnss { inner.alt } else { None };

                match (inner.latitude, inner.longitude) {
                    (Some(lat), Some(lon)) => {
                        vec![AircraftMessage {
                            icao,
                            signal_level,
                            payload: MessagePayload::Position {
                                latitude: lat,
                                longitude: lon,
                                altitude: inner.alt,
                                ground_speed: None,
                                track: None,
                                is_on_ground: None,
                                altitude_gnss,
                            },
                        }]
                    }
                    _ => {
                        if let Some(alt) = inner.alt {
                            vec![AircraftMessage {
                                icao,
                                signal_level,
                                payload: MessagePayload::Altitude {
                                    altitude: Some(alt),
                                    squawk: None,
                                    alert: None,
                                    emergency: None,
                                    spi: None,
                                    is_on_ground: None,
                                },
                            }]
                        } else {
                            vec![]
                        }
                    }
                }
            }
            ME::BDS06 { inner, .. } => {
                match (inner.latitude, inner.longitude) {
                    (Some(lat), Some(lon)) => {
                        vec![AircraftMessage {
                            icao,
                            signal_level,
                            payload: MessagePayload::Position {
                                latitude: lat,
                                longitude: lon,
                                altitude: None,
                                ground_speed: inner.groundspeed.map(f64::from),
                                track: inner.track,
                                is_on_ground: Some(true),
                                altitude_gnss: None,
                            },
                        }]
                    }
                    _ => vec![],
                }
            }
            ME::BDS09(velocity) => {
                match &velocity.velocity {
                    AirborneVelocitySubType::GroundSpeedDecoding(gs) => {
                        vec![AircraftMessage {
                            icao,
                            signal_level,
                            payload: MessagePayload::Velocity {
                                speed: gs.groundspeed,
                                track: gs.track,
                                vertical_rate: velocity.vertical_rate.map(i32::from),
                                is_on_ground: Some(false),
                                heading: None,
                                airspeed: None,
                                roll_angle: None,
                                track_angle_rate: None,
                            },
                        }]
                    }
                    AirborneVelocitySubType::AirspeedSubsonic(asp) => {
                        let heading = asp.heading.map(f64::from);
                        let airspeed = asp.airspeed.map(f64::from);
                        if let (Some(hdg), Some(aspd)) = (heading, airspeed) {
                            vec![AircraftMessage {
                                icao,
                                signal_level,
                                payload: MessagePayload::Velocity {
                                    speed: aspd,
                                    track: hdg,
                                    vertical_rate: velocity.vertical_rate.map(i32::from),
                                    is_on_ground: Some(false),
                                    heading: Some(hdg),
                                    airspeed: Some(aspd),
                                    roll_angle: None,
                                    track_angle_rate: None,
                                },
                            }]
                        } else {
                            vec![]
                        }
                    }
                    AirborneVelocitySubType::AirspeedSupersonic(asp) => {
                        let heading = asp.heading.map(f64::from);
                        let airspeed = asp.airspeed.map(f64::from);
                        if let (Some(hdg), Some(aspd)) = (heading, airspeed) {
                            vec![AircraftMessage {
                                icao,
                                signal_level,
                                payload: MessagePayload::Velocity {
                                    speed: aspd,
                                    track: hdg,
                                    vertical_rate: velocity.vertical_rate.map(i32::from),
                                    is_on_ground: Some(false),
                                    heading: Some(hdg),
                                    airspeed: Some(aspd),
                                    roll_angle: None,
                                    track_angle_rate: None,
                                },
                            }]
                        } else {
                            vec![]
                        }
                    }
                    _ => vec![],
                }
            }
            ME::BDS61(status) => {
                vec![AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Altitude {
                        altitude: None,
                        squawk: Some(format!("{:04o}", status.squawk.0)),
                        alert: None,
                        emergency: Some(!matches!(
                            status.emergency_state,
                            EmergencyState::None
                        )),
                        spi: None,
                        is_on_ground: None,
                    },
                }]
            }
            _ => vec![],
        }
    }

    fn maybe_update_global_reference(&mut self, ts: f64) {
        if self.decode_count % 100 == 0 {
            update_global_reference(&self.aircraft_state, &mut self.reference, ts);
        }
    }

    fn decode_commb_bds<T: CommBFields>(
        &self,
        bds: &T,
        icao: Icao,
        signal_level: Option<f32>,
        raw_mb: &[u8],
    ) -> Vec<AircraftMessage> {
        let mut msgs = Vec::new();

        if let Some(bds20) = bds.bds20() {
            let callsign = bds20.callsign.trim().to_string();
            if !callsign.is_empty() {
                msgs.push(AircraftMessage {
                    icao,
                    signal_level,
                    payload: MessagePayload::Identification {
                        callsign,
                        category: None,
                    },
                });
            }
        }

        for decoded in bds.decoded_bds() {
            if let Some(payload) = rs1090_mapping::map_bds(&decoded) {
                if !matches!(payload, MessagePayload::Identification { .. }) {
                    msgs.push(AircraftMessage {
                        icao,
                        signal_level,
                        payload,
                    });
                }
            }
        }

        // Fall back to native BDS heuristic when rs1090 found nothing beyond callsign
        let has_velocity = msgs.iter().any(|m| matches!(m.payload, MessagePayload::Velocity { .. }));
        if !has_velocity && raw_mb.len() >= 7 {
            if let Some(msg) = crate::protocol::beast::modes::decode_bds(raw_mb, icao, signal_level) {
                msgs.push(msg);
            }
        }

        msgs
    }
}

impl Default for Rs1090Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for Rs1090Decoder {
    fn decode(&mut self, frame: &Frame) -> Vec<AircraftMessage> {
        match frame.frame_type {
            FrameType::ModeSShort | FrameType::ModeSLong => {
                self.decode_modes(&frame.data, frame.signal_level)
            }
            _ => vec![],
        }
    }

    fn set_reference_position(&mut self, lat: f64, lon: f64) {
        self.reference = Some(Position {
            latitude: lat,
            longitude: lon,
        });
    }

    fn reset(&mut self) {
        self.aircraft_state.clear();
        self.reference = None;
        self.decode_count = 0;
    }
}

use rs1090::decode::bds::{DecodedBds, bds20};
use rs1090::decode::commb::{DF20DataSelector, DF21DataSelector};

trait CommBFields {
    fn bds20(&self) -> Option<&bds20::AircraftIdentification>;
    fn decoded_bds(&self) -> Vec<DecodedBds>;
}

impl CommBFields for DF20DataSelector {
    fn bds20(&self) -> Option<&bds20::AircraftIdentification> {
        self.bds20.as_ref()
    }

    fn decoded_bds(&self) -> Vec<DecodedBds> {
        let mut result = Vec::new();
        if let Some(ref v) = self.bds50 { result.push(DecodedBds::Bds50(v.clone())); }
        if let Some(ref v) = self.bds60 { result.push(DecodedBds::Bds60(v.clone())); }
        if let Some(ref v) = self.bds40 { result.push(DecodedBds::Bds40(v.clone())); }
        if let Some(ref v) = self.bds44 { result.push(DecodedBds::Bds44(v.clone())); }
        if let Some(ref v) = self.bds45 { result.push(DecodedBds::Bds45(v.clone())); }
        result
    }
}

impl CommBFields for DF21DataSelector {
    fn bds20(&self) -> Option<&bds20::AircraftIdentification> {
        self.bds20.as_ref()
    }

    fn decoded_bds(&self) -> Vec<DecodedBds> {
        let mut result = Vec::new();
        if let Some(ref v) = self.bds50 { result.push(DecodedBds::Bds50(v.clone())); }
        if let Some(ref v) = self.bds60 { result.push(DecodedBds::Bds60(v.clone())); }
        if let Some(ref v) = self.bds40 { result.push(DecodedBds::Bds40(v.clone())); }
        if let Some(ref v) = self.bds44 { result.push(DecodedBds::Bds44(v.clone())); }
        if let Some(ref v) = self.bds45 { result.push(DecodedBds::Bds45(v.clone())); }
        result
    }
}
