use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some("listener") => {
            let port = args.get(2).expect("listener port").parse::<u16>().expect("numeric port");
            let listener = TcpListener::bind(("0.0.0.0", port)).expect("bind guest listener");
            for incoming in listener.incoming() {
                let mut stream = incoming.expect("accept guest connection");
                let mut request = [0_u8; 32];
                let _ = stream.read(&mut request);
                stream.write_all(b"H6-GUEST-OK\n").expect("write guest response");
            }
        }
        _ => loop {
            std::thread::sleep(Duration::from_secs(60));
        },
    }
}
