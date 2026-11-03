//! Dependency-free HTTP/1.1 guest for the public-ingress-gateway example.
//!
//! Runs inside the VM as a static binary and speaks plaintext HTTP; it holds no
//! certificate or key. Routes:
//!
//! * `GET /stream` - `200`, `X-Workload: api`, `Transfer-Encoding: chunked`; the
//!   body is `CHUNKS` chunks of `PIG-STREAM-CHUNK-NN\n`, one every
//!   `CHUNK_DELAY` and flushed as written, so the first bytes leave long before
//!   the last. A buffering relay is observable as time-to-first-byte close to
//!   total time.
//! * `GET /exact` - `200` with `Content-Length` and the body `PIG-EXACT-OK\n`.
//! * `GET /count` - the request-count marker, `requests=<N>\n`, where N counts
//!   `/stream` and `/exact` requests served since start (health probes and
//!   `/count` itself are not counted).
//! * `GET /ready` - `204`, the readiness probe target.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

const PORT: u16 = 8080;
const CHUNKS: usize = 8;
const CHUNK_DELAY: Duration = Duration::from_millis(400);
const EXACT_BODY: &[u8] = b"PIG-EXACT-OK\n";

fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut byte = [0_u8; 1];
    while buf.len() < 8192 {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            return String::from_utf8(buf).ok();
        }
    }
    None
}

fn write_fixed(stream: &mut TcpStream, status: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn write_stream(stream: &mut TcpStream) {
    let head = "HTTP/1.1 200 OK\r\nX-Workload: api\r\nContent-Type: text/plain\r\n\
                Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    if stream.write_all(head.as_bytes()).is_err() || stream.flush().is_err() {
        return;
    }
    for index in 0..CHUNKS {
        let chunk = format!("PIG-STREAM-CHUNK-{index:02}\n");
        let frame = format!("{:x}\r\n{chunk}\r\n", chunk.len());
        if stream.write_all(frame.as_bytes()).is_err() || stream.flush().is_err() {
            return;
        }
        thread::sleep(CHUNK_DELAY);
    }
    let _ = stream.write_all(b"0\r\n\r\n");
}

fn handle(mut stream: TcpStream, requests: Arc<AtomicU64>) {
    let Some(head) = read_head(&mut stream) else { return };
    let line = head.lines().next().unwrap_or_default();
    match line {
        l if l.starts_with("GET /ready ") => write_fixed(&mut stream, "204 No Content", b""),
        l if l.starts_with("GET /stream ") => {
            requests.fetch_add(1, Ordering::SeqCst);
            write_stream(&mut stream);
        }
        l if l.starts_with("GET /exact ") => {
            requests.fetch_add(1, Ordering::SeqCst);
            write_fixed(&mut stream, "200 OK", EXACT_BODY);
        }
        l if l.starts_with("GET /count ") => {
            let body = format!("requests={}\n", requests.load(Ordering::SeqCst));
            write_fixed(&mut stream, "200 OK", body.as_bytes());
        }
        _ => write_fixed(&mut stream, "404 Not Found", b"not found\n"),
    }
}

fn main() {
    let listener = TcpListener::bind(("0.0.0.0", PORT)).expect("bind guest listener");
    let requests = Arc::new(AtomicU64::new(0));
    for incoming in listener.incoming() {
        let stream = incoming.expect("accept connection");
        let requests = Arc::clone(&requests);
        thread::spawn(move || handle(stream, requests));
    }
}
