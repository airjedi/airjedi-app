use std::collections::{HashMap, HashSet};

use rs1090::decode::bds::bds05::Source;
use rs1090::decode::bds::bds09::AirborneVelocitySubType;
use rs1090::decode::bds::bds61::EmergencyState;
use rs1090::decode::adsb::ME;
use rs1090::decode::{DF, FlightStatus, Message};

use crate::framing::{Frame, FrameType};
use crate::protocol::beast::cpr::CprState;
use crate::protocol::{AircraftMessage, Icao, MessagePayload};
use super::Decoder;
use super::rs1090_mapping;

pub struct Rs1090Decoder {
    known_icao: HashSet<Icao>,
    cpr_state: HashMap<Icao, CprState>,
    reference_position: Option<(f64, f64)>,
}

impl std::fmt::Debug for Rs1090Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rs1090Decoder")
            .field("known_icao_count", &self.known_icao.len())
            .field("cpr_state_count", &self.cpr_state.len())
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
            cpr_state: HashMap::new(),
            reference_position: None,
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

    fn decode_modes(&mut self, data: &[u8], signal_level: Option<f32>) -> Vec<AircraftMessage> {
        let msg = match Message::try_from(data) {
            Ok(m) => m,
            Err(_) => return vec![],
        };

        let icao = match self.extract_icao_and_validate(&msg) {
            Some(i) => i,
            None => return vec![],
        };

        match msg.df {
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
                        alert: Some(fs_is_alert(&fs)),
                        emergency: None,
                        spi: Some(fs_is_spi(&fs)),
                        is_on_ground: Some(fs_is_on_ground(&fs)),
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
                        alert: Some(fs_is_alert(&fs)),
                        emergency: None,
                        spi: Some(fs_is_spi(&fs)),
                        is_on_ground: Some(fs_is_on_ground(&fs)),
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
                self.decode_adsb(icao, signal_level, &adsb.message)
            }
            DF::ExtendedSquitterTisB { cf, .. } => {
                self.decode_adsb(icao, signal_level, &cf.me)
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
                let mut msgs = self.decode_commb_bds(&bds, icao, signal_level);
                if msgs.is_empty() {
                    msgs.push(AircraftMessage {
                        icao,
                        signal_level,
                        payload: MessagePayload::Altitude {
                            altitude: ac.0,
                            squawk: None,
                            alert: Some(fs_is_alert(&fs)),
                            emergency: None,
                            spi: Some(fs_is_spi(&fs)),
                            is_on_ground: Some(fs_is_on_ground(&fs)),
                        },
                    });
                }
                msgs
            }
            DF::CommBIdentityReply { id, bds, fs, .. } => {
                let mut msgs = self.decode_commb_bds(&bds, icao, signal_level);
                if msgs.is_empty() {
                    msgs.push(AircraftMessage {
                        icao,
                        signal_level,
                        payload: MessagePayload::Altitude {
                            altitude: None,
                            squawk: Some(format!("{:04o}", id.0)),
                            alert: Some(fs_is_alert(&fs)),
                            emergency: None,
                            spi: Some(fs_is_spi(&fs)),
                            is_on_ground: Some(fs_is_on_ground(&fs)),
                        },
                    });
                }
                msgs
            }
            DF::CommDExtended { .. } => vec![],
        }
    }

    fn decode_adsb(
        &mut self,
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
                let is_odd = inner.parity == rs1090::decode::cpr::CPRFormat::Odd;
                let altitude = inner.alt;
                let altitude_gnss = if inner.source == Source::Gnss { inner.alt } else { None };
                let is_surface = false;

                let position = self.cpr_state
                    .entry(icao)
                    .or_insert_with(CprState::new)
                    .update(
                        inner.lat_cpr,
                        inner.lon_cpr,
                        is_odd,
                        is_surface,
                        self.reference_position,
                    );

                match position {
                    Some((lat, lon)) => {
                        vec![AircraftMessage {
                            icao,
                            signal_level,
                            payload: MessagePayload::Position {
                                latitude: lat,
                                longitude: lon,
                                altitude,
                                ground_speed: None,
                                track: None,
                                is_on_ground: None,
                                altitude_gnss,
                            },
                        }]
                    }
                    None => {
                        if let Some(alt) = altitude {
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
                let is_odd = inner.parity == rs1090::decode::cpr::CPRFormat::Odd;
                let is_surface = true;

                let position = self.cpr_state
                    .entry(icao)
                    .or_insert_with(CprState::new)
                    .update(
                        inner.lat_cpr,
                        inner.lon_cpr,
                        is_odd,
                        is_surface,
                        self.reference_position,
                    );

                match position {
                    Some((lat, lon)) => {
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
                    None => vec![],
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

    fn decode_commb_bds<T: CommBFields>(
        &self,
        bds: &T,
        icao: Icao,
        signal_level: Option<f32>,
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
        self.reference_position = Some((lat, lon));
    }

    fn reset(&mut self) {
        self.cpr_state.clear();
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
