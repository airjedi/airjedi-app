use rs1090::decode::bds::DecodedBds;
use rs1090::decode::bds::bds45::Level;
use crate::protocol::MessagePayload;

fn level_to_u8(level: &Level) -> u8 {
    match level {
        Level::Nil => 0,
        Level::Light => 1,
        Level::Moderate => 2,
        Level::Severe => 3,
    }
}

pub fn map_bds(decoded: &DecodedBds) -> Option<MessagePayload> {
    match decoded {
        DecodedBds::Bds20(id) => {
            let callsign = id.callsign.trim().to_string();
            if callsign.is_empty() {
                return None;
            }
            Some(MessagePayload::Identification {
                callsign,
                category: None,
            })
        }
        DecodedBds::Bds50(t) => {
            let speed = t.groundspeed.map(f64::from)
                .or_else(|| t.true_airspeed.map(f64::from))?;
            let track = t.track_angle?;
            Some(MessagePayload::Velocity {
                speed,
                track,
                vertical_rate: None,
                is_on_ground: None,
                heading: None,
                airspeed: t.true_airspeed.map(f64::from),
                roll_angle: t.roll_angle,
                track_angle_rate: t.track_rate,
            })
        }
        DecodedBds::Bds60(h) => {
            let heading = h.magnetic_heading?;
            let speed = h.indicated_airspeed.map(f64::from)?;
            Some(MessagePayload::Velocity {
                speed,
                track: heading,
                vertical_rate: h.barometric_altitude_rate.map(i32::from),
                is_on_ground: None,
                heading: Some(heading),
                airspeed: h.indicated_airspeed.map(f64::from),
                roll_angle: None,
                track_angle_rate: None,
            })
        }
        DecodedBds::Bds40(s) => Some(MessagePayload::SelectedAltitude {
            mcp_altitude: s.selected_altitude_mcp.map(i32::from),
            fms_altitude: s.selected_altitude_fms.map(i32::from),
            barometric_setting: s.barometric_setting,
        }),
        DecodedBds::Bds44(m) => Some(MessagePayload::Meteorological {
            wind_speed: m.wind_speed,
            wind_direction: m.wind_direction,
            temperature: m.temperature,
            pressure: m.pressure,
        }),
        DecodedBds::Bds45(m) => Some(MessagePayload::MeteorologicalHazard {
            turbulence: m.turbulence.as_ref().map(level_to_u8),
            wind_shear: m.wind_shear.as_ref().map(level_to_u8),
            icing: m.icing.as_ref().map(level_to_u8),
            wake_vortex: m.wake_vortex.as_ref().map(level_to_u8),
            temperature: m.static_temperature,
            pressure: m.static_pressure.map(|p| p as u16),
        }),
        _ => None,
    }
}
