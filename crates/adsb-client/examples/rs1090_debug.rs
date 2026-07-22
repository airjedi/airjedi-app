use std::io::Read;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use adsb_client::framing::{BeastFramer, Framer, FrameType};
use rs1090::decode::Message;

fn main() {
    let mut stream = TcpStream::connect_timeout(
        &"192.168.1.10:30005".parse().unwrap(),
        Duration::from_secs(5),
    )
    .expect("Failed to connect");

    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();

    let mut framer = BeastFramer::new();
    let mut buf = [0u8; 4096];
    let mut frames = 0;
    let mut ok = 0;
    let mut errs = 0;
    let start = Instant::now();

    println!("Connected. Collecting 10s of data...\n");

    while start.elapsed() < Duration::from_secs(10) {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                framer.feed(&buf[..n]);
                while let Some(frame) = framer.next_frame() {
                    frames += 1;
                    let data = &frame.data[..];
                    let len = data.len();
                    let ft = frame.frame_type;

                    match Message::try_from(data) {
                        Ok(msg) => {
                            ok += 1;
                            if ok <= 5 {
                                println!("OK  [{:>4}] {:?} len={} df={:?}",
                                    frames, ft, len, msg.df);
                            }
                        }
                        Err(e) => {
                            errs += 1;
                            if errs <= 10 {
                                let hex: String = data.iter()
                                    .map(|b| format!("{:02X}", b))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                println!("ERR [{:>4}] {:?} len={} hex=[{}] err={:?}",
                                    frames, ft, len, hex, e);
                            }
                        }
                    }
                }
            }
            Err(_) => continue,
        }
    }

    println!("\n=== Summary ===");
    println!("Frames: {}, OK: {}, Errors: {}", frames, ok, errs);
    println!("Success rate: {:.1}%", ok as f64 / frames as f64 * 100.0);
}
