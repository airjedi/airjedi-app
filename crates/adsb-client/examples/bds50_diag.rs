use adsb_client::protocol::beast::BeastParser;
use adsb_client::protocol::{MessagePayload, Protocol};
use std::io::Read;
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn main() {
    let host = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "airjedi.custine.com:30005".to_string());
    println!("Connecting to {} ...", host);

    let stream = TcpStream::connect(&host).expect("Failed to connect");
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok();
    let mut reader = std::io::BufReader::new(stream);

    let mut parser = BeastParser::new();
    parser.set_reference_position(37.6872, -97.3301);

    let start = Instant::now();
    let mut total_msgs = 0u64;
    let mut bds_velocity = 0u64;
    let mut bds_with_roll = 0u64;
    let mut bds_with_tar = 0u64;
    let mut bds_ident = 0u64;
    let mut bytes_fed = 0u64;

    println!("Listening for 30 seconds...\n");

    let mut buf = [0u8; 4096];
    while start.elapsed() < Duration::from_secs(30) {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => {
                eprintln!("Read error: {}", e);
                break;
            }
        };
        bytes_fed += n as u64;

        for byte in &buf[..n] {
            match parser.parse(std::slice::from_ref(byte)) {
                Ok(Some(msg)) => {
                    total_msgs += 1;
                    match &msg.payload {
                        MessagePayload::Velocity {
                            roll_angle,
                            track_angle_rate,
                            speed,
                            track,
                            ..
                        } => {
                            bds_velocity += 1;
                            if roll_angle.is_some() {
                                bds_with_roll += 1;
                                println!(
                                    "  ROLL: {} roll={:.1} tar={:?} spd={:.0} trk={:.0}",
                                    msg.icao,
                                    roll_angle.unwrap(),
                                    track_angle_rate,
                                    speed,
                                    track,
                                );
                            }
                            if track_angle_rate.is_some() {
                                bds_with_tar += 1;
                            }
                        }
                        MessagePayload::Identification { .. } => {
                            bds_ident += 1;
                        }
                        _ => {}
                    }
                }
                Ok(None) => {}
                Err(_) => {}
            }
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    println!("\n--- BDS 5,0 Diagnostic ({:.1}s) ---", elapsed);
    println!("Bytes received:           {}", bytes_fed);
    println!("Total messages parsed:    {}", total_msgs);
    println!(
        "Velocity payloads:        {} (BDS 5,0 + 6,0 + TC19)",
        bds_velocity
    );
    println!("  with roll_angle:        {}", bds_with_roll);
    println!("  with track_angle_rate:  {}", bds_with_tar);
    println!("Identification payloads:  {} (BDS 2,0 + TC1-4)", bds_ident);
    println!("Rate: {:.1} msg/s", total_msgs as f64 / elapsed);
}
