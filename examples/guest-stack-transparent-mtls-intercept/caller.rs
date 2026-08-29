use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

const REQUEST: &[u8] = b"GTI_E07_REQUEST_guest_to_exec_service\n";
const RESPONSE: &[u8] = b"GTI_E07_REPLY_exec_service_to_guest\n";

fn main() {
    let mut args = env::args().skip(1);
    let host = args.next().expect("service name argument");
    let port = args.next().expect("service port argument");
    let address = format!("{host}:{port}");
    let deadline = Instant::now() + Duration::from_secs(90);

    while Instant::now() < deadline {
        if let Ok(mut stream) = TcpStream::connect(&address) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            if stream.write_all(REQUEST).is_ok() {
                let mut reply = vec![0_u8; RESPONSE.len()];
                if stream.read_exact(&mut reply).is_ok() && reply == RESPONSE {
                    println!("GTI_E07_EXPECTED_REPLY_RECEIVED");
                    return;
                }
            }
        }
        thread::sleep(Duration::from_millis(500));
    }

    eprintln!("expected reply was not received before the deadline");
    std::process::exit(42);
}
