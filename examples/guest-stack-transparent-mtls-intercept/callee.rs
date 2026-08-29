use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

const REQUEST: &[u8] = b"GTI_E07_REQUEST_guest_to_exec_service\n";
const RESPONSE: &[u8] = b"GTI_E07_REPLY_exec_service_to_guest\n";

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", 18_951))?;
    for stream in listener.incoming() {
        let mut stream = stream?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut request = vec![0_u8; REQUEST.len()];
        if stream.read_exact(&mut request).is_ok() && request == REQUEST {
            stream.write_all(RESPONSE)?;
        }
    }
    Ok(())
}
