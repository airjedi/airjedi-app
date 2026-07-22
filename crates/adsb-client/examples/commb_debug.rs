use std::io::Read;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use adsb_client::framing::{BeastFramer, Framer, FrameType};
use rs1090::decode::{DF, Message};

fn main() {
    let mut stream = TcpStream::connect_timeout(
        &"192.168.1.10:30005".parse().unwrap(),
        Duration::from_secs(5),
    ).expect("Failed to connect");

    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();

    let mut framer = BeastFramer::new();
    let mut buf = [0u8; 4096];
    let start = Instant::now();
    let mut df20_count = 0;
    let mut df21_count = 0;
    let mut printed = 0;

    println!("Watching for DF20/21 Comm-B frames (15 seconds)...\n");

    while start.elapsed() < Duration::from_secs(15) {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                framer.feed(&buf[..n]);
                while let Some(frame) = framer.next_frame() {
                    if frame.frame_type != FrameType::ModeSLong {
                        continue;
                    }
                    let data = &frame.data[..];
                    let df_byte = data[0] >> 3;
                    if df_byte != 20 && df_byte != 21 {
                        continue;
                    }

                    match Message::try_from(data) {
                        Ok(msg) => match &msg.df {
                            DF::CommBAltitudeReply { bds, ac, .. } => {
                                df20_count += 1;
                                if printed < 10 {
                                    printed += 1;
                                    let has_50 = bds.bds50.is_some();
                                    let has_60 = bds.bds60.is_some();
                                    let has_40 = bds.bds40.is_some();
                                    let has_44 = bds.bds44.is_some();
                                    let has_20 = bds.bds20.is_some();
                                    println!("DF20 icao={:06X} alt={:?} bds20={} bds40={} bds44={} bds50={} bds60={}",
                                        msg.crc, ac.0,
                                        has_20, has_40, has_44, has_50, has_60);
                                    if has_50 {
                                        let t = bds.bds50.as_ref().unwrap();
                                        println!("  BDS50: roll={:?} track={:?} gs={:?} tar={:?} tas={:?}",
                                            t.roll_angle, t.track_angle, t.groundspeed, t.track_rate, t.true_airspeed);
                                    }
                                    if has_60 {
                                        let h = bds.bds60.as_ref().unwrap();
                                        println!("  BDS60: heading={:?} ias={:?} mach={:?} vr_baro={:?}",
                                            h.magnetic_heading, h.indicated_airspeed, h.mach_number, h.barometric_altitude_rate);
                                    }
                                }
                            }
                            DF::CommBIdentityReply { bds, .. } => {
                                df21_count += 1;
                                if printed < 10 {
                                    printed += 1;
                                    let has_50 = bds.bds50.is_some();
                                    let has_60 = bds.bds60.is_some();
                                    let has_40 = bds.bds40.is_some();
                                    let has_20 = bds.bds20.is_some();
                                    println!("DF21 icao={:06X} bds20={} bds40={} bds50={} bds60={}",
                                        msg.crc, has_20, has_40, has_50, has_60);
                                }
                            }
                            _ => {}
                        }
                        Err(e) => {
                            if df_byte == 20 { df20_count += 1; }
                            if df_byte == 21 { df21_count += 1; }
                        }
                    }
                }
            }
            Err(_) => continue,
        }
    }

    println!("\n=== Comm-B Summary ===");
    println!("DF20 frames: {}", df20_count);
    println!("DF21 frames: {}", df21_count);
}
