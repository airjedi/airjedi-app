use std::io::Read;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use adsb_client::framing::{BeastFramer, Framer};
use adsb_client::decoder::{Decoder, NativeDecoder};
use adsb_client::MessagePayload;

#[cfg(feature = "decoder-rs1090")]
use adsb_client::Rs1090Decoder;

fn main() {
    let mut stream = TcpStream::connect_timeout(
        &"192.168.1.10:30005".parse().unwrap(),
        Duration::from_secs(5),
    )
    .expect("Failed to connect to BEAST feed");

    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    let start = Instant::now();
    let collect_secs = 30;

    let mut framer = BeastFramer::new();

    #[cfg(feature = "decoder-rs1090")]
    let mut decoder = Rs1090Decoder::new();
    #[cfg(not(feature = "decoder-rs1090"))]
    let mut decoder = NativeDecoder::new();

    decoder.set_reference_position(37.6872, -97.3301);

    let mut buf = [0u8; 4096];
    let mut total_messages = 0;
    let mut bds50_count = 0;
    let mut bds60_count = 0;
    let mut bds40_count = 0;
    let mut bds44_count = 0;
    let mut bds45_count = 0;
    let mut ident_count = 0;
    let mut position_count = 0;
    let mut velocity_count = 0;
    let mut altitude_count = 0;

    #[cfg(feature = "decoder-rs1090")]
    println!("BDS Diagnostic - using Rs1090Decoder");
    #[cfg(not(feature = "decoder-rs1090"))]
    println!("BDS Diagnostic - using NativeDecoder");

    println!("Connected to BEAST feed at 192.168.1.10:30005");
    println!("Collecting for {} seconds...\n", collect_secs);

    while start.elapsed() < Duration::from_secs(collect_secs) {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                framer.feed(&buf[..n]);
                while let Some(frame) = framer.next_frame() {
                    let messages = decoder.decode(&frame);
                    for msg in messages {
                        total_messages += 1;
                        match &msg.payload {
                            MessagePayload::Identification { .. } => ident_count += 1,
                            MessagePayload::Position { .. } => position_count += 1,
                            MessagePayload::Velocity { roll_angle, track_angle_rate, heading, .. } => {
                                velocity_count += 1;
                                if roll_angle.is_some() || track_angle_rate.is_some() {
                                    bds50_count += 1;
                                    println!(
                                        "  BDS 5,0 [{:>6}] roll={:>7.2} deg  tar={:>6.3} deg/s  ({})",
                                        msg.icao,
                                        roll_angle.unwrap_or(0.0),
                                        track_angle_rate.unwrap_or(0.0),
                                        msg.icao
                                    );
                                } else if heading.is_some() {
                                    bds60_count += 1;
                                }
                            }
                            MessagePayload::Altitude { .. } => altitude_count += 1,
                            MessagePayload::SelectedAltitude { mcp_altitude, barometric_setting, .. } => {
                                bds40_count += 1;
                                println!(
                                    "  BDS 4,0 [{:>6}] sel_alt={:?} ft  baro={:?} hPa",
                                    msg.icao, mcp_altitude, barometric_setting
                                );
                            }
                            MessagePayload::Meteorological { temperature, wind_speed, wind_direction, .. } => {
                                bds44_count += 1;
                                println!(
                                    "  BDS 4,4 [{:>6}] temp={:.1}C  wind={:?}kt @ {:?}deg",
                                    msg.icao, temperature, wind_speed, wind_direction
                                );
                            }
                            MessagePayload::MeteorologicalHazard { turbulence, temperature, .. } => {
                                bds45_count += 1;
                                println!(
                                    "  BDS 4,5 [{:>6}] turb={:?}  temp={:?}C",
                                    msg.icao, turbulence, temperature
                                );
                            }
                        }
                    }
                }
            }
            Err(_) => continue,
        }
    }

    println!("\n=== BDS Diagnostic Summary ===");
    println!("Total messages: {}", total_messages);
    println!("  Identification:  {}", ident_count);
    println!("  Position:        {}", position_count);
    println!("  Velocity:        {}", velocity_count);
    println!("  Altitude:        {}", altitude_count);
    println!();
    println!("BDS Comm-B breakdown:");
    println!("  BDS 5,0 (roll/turn):      {}", bds50_count);
    println!("  BDS 6,0 (heading/speed):  {}", bds60_count);
    println!("  BDS 4,0 (selected alt):   {}", bds40_count);
    println!("  BDS 4,4 (meteorological): {}", bds44_count);
    println!("  BDS 4,5 (met hazard):     {}", bds45_count);
}
