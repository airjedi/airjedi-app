use std::io::Read;
use std::net::TcpStream;
use std::time::Duration;
use adsb_client::protocol::beast::BeastParser;
use adsb_client::protocol::Protocol;

fn main() {
    let mut stream = TcpStream::connect_timeout(
        &"192.168.1.10:30005".parse().unwrap(),
        Duration::from_secs(5),
    )
    .expect("Failed to connect to BEAST feed");

    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();

    let mut parser = BeastParser::new();
    parser.set_reference_position(37.6872, -97.3301);

    let mut buf = [0u8; 4096];
    let mut total_bytes = 0;
    let mut total_messages = 0;

    println!("Connected to BEAST feed at 192.168.1.10:30005");

    for _ in 0..30 {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                total_bytes += n;

                match parser.parse(&buf[..n]) {
                    Ok(Some(msg)) => {
                        total_messages += 1;
                        if total_messages <= 30 {
                            println!("[{}] icao={} {:?}", total_messages, msg.icao(), msg.payload);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => println!("ERR: {}", e),
                }

                loop {
                    match parser.parse(&[]) {
                        Ok(Some(msg)) => {
                            total_messages += 1;
                            if total_messages <= 30 {
                                println!("[{}] icao={} {:?}", total_messages, msg.icao(), msg.payload);
                            }
                        }
                        Ok(None) => break,
                        Err(e) => { println!("ERR: {}", e); break; }
                    }
                }
            }
            Err(e) => {
                println!("Read error: {}", e);
                break;
            }
        }
    }

    println!("\n=== Summary ===");
    println!("Bytes: {}, Messages decoded: {}", total_bytes, total_messages);
}
