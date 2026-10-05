use std::io::Read;
use std::net::TcpStream;
use std::time::Duration;

fn main() {
    let mut stream = TcpStream::connect_timeout(
        &"192.168.1.10:30005".parse().unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();

    let mut raw_buf = Vec::new();
    let mut read_buf = [0u8; 4096];
    for _ in 0..100 {
        match stream.read(&mut read_buf) {
            Ok(0) => break,
            Ok(n) => raw_buf.extend_from_slice(&read_buf[..n]),
            Err(_) => break,
        }
    }

    let mut pos = 0;
    while pos < raw_buf.len() {
        if raw_buf[pos] != 0x1A {
            pos += 1;
            continue;
        }
        let mut start = pos;
        while start < raw_buf.len() && raw_buf[start] == 0x1A {
            start += 1;
        }
        if start >= raw_buf.len() {
            break;
        }

        let msg_type = raw_buf[start];
        let data_len = match msg_type {
            0x32 => 7,
            0x33 => 14,
            _ => {
                pos = start + 1;
                continue;
            }
        };
        let payload_start = start + 1;
        let mut unescaped = Vec::new();
        let mut src = payload_start;
        let total_needed = 6 + 1 + data_len;
        while unescaped.len() < total_needed && src < raw_buf.len() {
            let b = raw_buf[src];
            src += 1;
            if b == 0x1A {
                if src < raw_buf.len() && raw_buf[src] == 0x1A {
                    unescaped.push(0x1A);
                    src += 1;
                } else {
                    break;
                }
            } else {
                unescaped.push(b);
            }
        }
        if unescaped.len() < total_needed {
            pos = src;
            continue;
        }

        let data = &unescaped[7..];
        let df = data[0] >> 3;

        if matches!(df, 0 | 4 | 16 | 20) && data.len() >= 4 {
            let ac13 = ((data[2] as u16) << 8 | data[3] as u16) & 0x1FFF;
            let q_bit = (ac13 >> 4) & 1;
            let m_bit = (ac13 >> 6) & 1;

            if q_bit != 0 && m_bit == 0 {
                // Correct formula (dump1090-style): remove Q bit, reassemble 11 bits
                let n_correct = ((ac13 & 0x1F80) >> 2) | ((ac13 & 0x0020) >> 1) | (ac13 & 0x000F);
                let alt_correct = n_correct as i32 * 25 - 1000;

                // What does our library produce?
                // We can't call modes:: directly, but we verify the formula matches
                let icao = crc24_icao(data);
                println!(
                    "DF={:<2} ICAO={} ac13=0x{:04X} alt={}",
                    df, icao, ac13, alt_correct
                );
            }
        }

        if df == 17 && data.len() >= 11 {
            let me = &data[4..11];
            let tc = me[0] >> 3;
            if (9..=18).contains(&tc) {
                let alt_bits = (me[1] as u16) << 4 | (me[2] as u16) >> 4;
                let q_bit = (alt_bits >> 4) & 1;
                if q_bit != 0 {
                    let n = ((alt_bits >> 5) << 4) | (alt_bits & 0x0F);
                    if n > 0 {
                        let alt = n as i32 * 25 - 1000;
                        let icao = format!("{:02X}{:02X}{:02X}", data[1], data[2], data[3]);
                        println!("DF=17 ICAO={} TC={:<2} alt={}", icao, tc, alt);
                    }
                }
            }
        }

        pos = src;
    }
}

fn crc24_icao(data: &[u8]) -> String {
    let generator: u32 = 0x1FFF409;
    let mut crc: u32 = 0;
    for &byte in &data[..data.len() - 3] {
        crc ^= (byte as u32) << 16;
        for _ in 0..8 {
            crc <<= 1;
            if crc & 0x1000000 != 0 {
                crc ^= generator;
            }
        }
    }
    let pi = (data[data.len() - 3] as u32) << 16
        | (data[data.len() - 2] as u32) << 8
        | data[data.len() - 1] as u32;
    let r = (crc ^ pi) & 0xFFFFFF;
    format!(
        "{:02X}{:02X}{:02X}",
        (r >> 16) & 0xFF,
        (r >> 8) & 0xFF,
        r & 0xFF
    )
}
