use std::collections::HashMap;
use std::io::Read;
use std::net::TcpStream;
use std::time::{Duration, Instant};

const BEAST_ESCAPE: u8 = 0x1A;

fn main() {
    let host = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "airjedi.custine.com:30005".to_string());
    println!("Connecting to {} ...", host);

    let stream = TcpStream::connect(&host).expect("Failed to connect");
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok();
    let mut reader = std::io::BufReader::new(stream);

    let start = Instant::now();
    let mut df_counts: HashMap<u8, u64> = HashMap::new();
    let mut total_frames = 0u64;
    let mut bytes_total = 0u64;

    // Simple BEAST frame scanner: look for 0x1A + type byte, then extract DF
    // Type 0x31 = Mode-AC (no DF), 0x32 = Mode-S short (7B), 0x33 = Mode-S long (14B)
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];

    println!("Listening for 15 seconds...\n");

    while start.elapsed() < Duration::from_secs(15) {
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
        bytes_total += n as u64;
        raw.extend_from_slice(&buf[..n]);

        // Scan for BEAST frames: 0x1A <type> <6B timestamp> <1B signal> <data...>
        let mut i = 0;
        while i < raw.len() {
            if raw[i] != BEAST_ESCAPE {
                i += 1;
                continue;
            }
            if i + 1 >= raw.len() {
                break;
            }
            let msg_type = raw[i + 1];
            let data_len = match msg_type {
                0x31 => 2,  // Mode-AC
                0x32 => 7,  // Mode-S short
                0x33 => 14, // Mode-S long
                BEAST_ESCAPE => {
                    i += 2; // escaped 0x1A
                    continue;
                }
                _ => {
                    i += 1;
                    continue;
                }
            };

            // Need: 1A + type + 6 timestamp + 1 signal + data_len
            let frame_len = 2 + 6 + 1 + data_len;
            if i + frame_len > raw.len() {
                break; // incomplete frame
            }

            // Data starts at offset 9 (after 1A + type + 6B timestamp + 1B signal)
            let data_start = i + 9;
            if data_start < raw.len() {
                let df = raw[data_start] >> 3;
                *df_counts.entry(df).or_insert(0) += 1;
                total_frames += 1;
            }

            i += frame_len;
        }

        // Keep unprocessed tail
        if i > 0 {
            raw.drain(..i);
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    println!("--- DF Distribution ({:.1}s) ---", elapsed);
    println!("Bytes received:  {}", bytes_total);
    println!("Total frames:    {}", total_frames);
    println!();

    let mut sorted: Vec<_> = df_counts.iter().collect();
    sorted.sort_by_key(|(df, _)| *df);
    for (df, count) in &sorted {
        let pct = **count as f64 / total_frames as f64 * 100.0;
        let label = match **df {
            0 => "Short ACAS",
            4 => "Altitude Reply",
            5 => "Identity Reply",
            11 => "All-Call Reply",
            16 => "Long ACAS",
            17 => "ADS-B (Extended Squitter)",
            18 => "TIS-B / ADS-R",
            20 => "Comm-B Altitude  ** BDS carrier **",
            21 => "Comm-B Identity  ** BDS carrier **",
            _ => "Other",
        };
        println!("  DF={:2}: {:>6}  ({:5.1}%)  {}", df, count, pct, label);
    }
}
