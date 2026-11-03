//! Conformance guest for the public-ingress-gateway end-to-end tests.
//!
//! A dependency-free HTTP/1.1 origin that runs inside the VM as a static
//! binary. It speaks plaintext, holds no certificate or key, and answers one
//! request per connection (`Connection: close`), as the gateway's upstream leg
//! sends one. It is the superset of the checked-in example guest
//! (`examples/public-ingress-gateway/guest_server.rs`) that the exact-length,
//! chunked, early-answer, silent-close, slow-head and held-stream cases need.
//! The checked-in example guest itself serves the walking skeleton.
//!
//! Every response body is a deterministic pattern the client verifies
//! octet by octet; every request body is summarised as `len=<n> fnv=<hex>` so
//! the client can compare what the origin received with what it sent.
//!
//! Routes (the request-count marker counts every route except `/ready` and
//! `/count`):
//!
//! * `GET /ready`: `204`.
//! * `GET /count`: `requests=<N>`.
//! * `GET /stream`: chunked, `X-Workload: api`, eight chunks 400 ms apart.
//! * `GET /exact`: `Content-Length` `PIG-EXACT-OK`.
//! * `GET /bytes/<n>`: `Content-Length: n`, pattern body.
//! * `GET /chunked/<n>`: chunked pattern body of `n` octets in varying chunks.
//! * `POST /echo`: reads a `Content-Length` or chunked request body and
//!   answers its length and FNV-1a digest.
//! * `GET /close`: reads the head, then closes with no response.
//! * `POST /early`: answers `401` before reading the body, then drains.
//! * `GET /delay`: waits 4 s (inside the gateway's 5 s response-head budget)
//!   before answering.
//! * `GET /hold`: `Content-Length: 1048576`, 8 KiB every 250 ms (32 s).
//! * any other route: `404`.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

const PORT: u16 = 8080;
const HEAD_LIMIT: usize = 64 * 1024;
const HOLD_TOTAL: usize = 1024 * 1024;
const HOLD_PIECE: usize = 8 * 1024;
const HOLD_PERIOD: Duration = Duration::from_millis(250);

fn pattern(index: u64) -> u8 {
    (index.wrapping_mul(31).wrapping_add(7) % 251) as u8
}

fn fnv_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

struct Head {
    request_line: String,
    content_length: Option<u64>,
    chunked: bool,
}

fn read_head(stream: &mut TcpStream) -> Option<Head> {
    let mut buf = Vec::with_capacity(1024);
    let mut byte = [0_u8; 1];
    while buf.len() < HEAD_LIMIT {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            let text = String::from_utf8(buf).ok()?;
            let mut lines = text.split("\r\n");
            let request_line = lines.next()?.to_owned();
            let mut content_length = None;
            let mut chunked = false;
            for line in lines {
                let lower = line.to_ascii_lowercase();
                if let Some(value) = lower.strip_prefix("content-length:") {
                    content_length = value.trim().parse().ok();
                } else if let Some(value) = lower.strip_prefix("transfer-encoding:") {
                    chunked = value.contains("chunked");
                }
            }
            return Some(Head { request_line, content_length, chunked });
        }
    }
    None
}

fn read_line(stream: &mut TcpStream) -> Option<String> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            line.truncate(line.len() - 2);
            return String::from_utf8(line).ok();
        }
        if line.len() > 4096 {
            return None;
        }
    }
}

/// Reads the request body per its framing; returns `(length, fnv)`.
fn read_body(stream: &mut TcpStream, head: &Head) -> Option<(u64, u64)> {
    let mut length = 0_u64;
    let mut hash = FNV_OFFSET;
    let mut buf = vec![0_u8; 64 * 1024];
    if head.chunked {
        loop {
            let size_line = read_line(stream)?;
            let size = u64::from_str_radix(size_line.split(';').next()?.trim(), 16).ok()?;
            if size == 0 {
                // Trailers (none are sent) end at the empty line.
                loop {
                    if read_line(stream)?.is_empty() {
                        return Some((length, hash));
                    }
                }
            }
            let mut remaining = size;
            while remaining > 0 {
                let want = usize::try_from(remaining.min(buf.len() as u64)).ok()?;
                let n = stream.read(&mut buf[..want]).ok()?;
                if n == 0 {
                    return None;
                }
                hash = fnv_update(hash, &buf[..n]);
                length += n as u64;
                remaining -= n as u64;
            }
            if !read_line(stream)?.is_empty() {
                return None;
            }
        }
    }
    let mut remaining = head.content_length.unwrap_or(0);
    while remaining > 0 {
        let want = usize::try_from(remaining.min(buf.len() as u64)).ok()?;
        let n = stream.read(&mut buf[..want]).ok()?;
        if n == 0 {
            return None;
        }
        hash = fnv_update(hash, &buf[..n]);
        length += n as u64;
        remaining -= n as u64;
    }
    Some((length, hash))
}

fn write_fixed(stream: &mut TcpStream, status: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nX-Workload: api\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn write_pattern_body(stream: &mut TcpStream, total: u64) {
    let mut offset = 0_u64;
    let mut buf = vec![0_u8; 64 * 1024];
    while offset < total {
        let n = usize::try_from((total - offset).min(buf.len() as u64)).unwrap_or(buf.len());
        for (i, slot) in buf[..n].iter_mut().enumerate() {
            *slot = pattern(offset + i as u64);
        }
        if stream.write_all(&buf[..n]).is_err() {
            return;
        }
        offset += n as u64;
    }
}

fn write_bytes(stream: &mut TcpStream, total: u64) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nX-Workload: api\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(head.as_bytes()).is_ok() {
        write_pattern_body(stream, total);
    }
}

fn write_chunked(stream: &mut TcpStream, total: u64) {
    const SIZES: [u64; 8] = [1, 17, 4096, 65_535, 65_536, 65_537, 12_345, 3];
    let head = "HTTP/1.1 200 OK\r\nX-Workload: api\r\nTransfer-Encoding: chunked\r\n\
                Connection: close\r\n\r\n";
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let mut offset = 0_u64;
    let mut which = 0_usize;
    while offset < total {
        let size = SIZES[which % SIZES.len()].min(total - offset);
        which += 1;
        let mut frame = format!("{size:x}\r\n").into_bytes();
        for i in 0..size {
            frame.push(pattern(offset + i));
        }
        frame.extend_from_slice(b"\r\n");
        if stream.write_all(&frame).is_err() {
            return;
        }
        offset += size;
    }
    let _ = stream.write_all(b"0\r\n\r\n");
}

fn write_stream(stream: &mut TcpStream) {
    let head = "HTTP/1.1 200 OK\r\nX-Workload: api\r\nContent-Type: text/plain\r\n\
                Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    if stream.write_all(head.as_bytes()).is_err() || stream.flush().is_err() {
        return;
    }
    for index in 0..8 {
        let chunk = format!("PIG-STREAM-CHUNK-{index:02}\n");
        let frame = format!("{:x}\r\n{chunk}\r\n", chunk.len());
        if stream.write_all(frame.as_bytes()).is_err() || stream.flush().is_err() {
            return;
        }
        thread::sleep(Duration::from_millis(400));
    }
    let _ = stream.write_all(b"0\r\n\r\n");
}

fn write_hold(stream: &mut TcpStream) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nX-Workload: api\r\nContent-Length: {HOLD_TOTAL}\r\n\
         Connection: close\r\n\r\n"
    );
    if stream.write_all(head.as_bytes()).is_err() || stream.flush().is_err() {
        return;
    }
    let mut offset = 0_u64;
    while (offset as usize) < HOLD_TOTAL {
        let mut piece = vec![0_u8; HOLD_PIECE];
        for (i, slot) in piece.iter_mut().enumerate() {
            *slot = pattern(offset + i as u64);
        }
        if stream.write_all(&piece).is_err() || stream.flush().is_err() {
            return;
        }
        offset += HOLD_PIECE as u64;
        thread::sleep(HOLD_PERIOD);
    }
}

fn drain(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut sink = vec![0_u8; 64 * 1024];
    let mut drained = 0_usize;
    while drained < 8 * 1024 * 1024 {
        match stream.read(&mut sink) {
            Ok(0) | Err(_) => return,
            Ok(n) => drained += n,
        }
    }
}

fn handle(mut stream: TcpStream, requests: Arc<AtomicU64>) {
    let Some(head) = read_head(&mut stream) else { return };
    let mut parts = head.request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    if path == "/ready" {
        write_fixed(&mut stream, "204 No Content", b"");
        return;
    }
    if path == "/count" {
        let body = format!("requests={}\n", requests.load(Ordering::SeqCst));
        write_fixed(&mut stream, "200 OK", body.as_bytes());
        return;
    }
    requests.fetch_add(1, Ordering::SeqCst);
    match (method.as_str(), path.as_str()) {
        ("GET", "/stream") => write_stream(&mut stream),
        ("GET", "/exact") => write_fixed(&mut stream, "200 OK", b"PIG-EXACT-OK\n"),
        ("GET", "/delay") => {
            thread::sleep(Duration::from_secs(4));
            write_fixed(&mut stream, "200 OK", b"late\n");
        }
        ("GET", "/hold") => write_hold(&mut stream),
        ("GET", "/close") => {
            let _ = stream.shutdown(Shutdown::Both);
        }
        ("POST", "/early") => {
            let _ = stream.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            let _ = stream.shutdown(Shutdown::Write);
            drain(&mut stream);
        }
        ("POST", "/echo") => match read_body(&mut stream, &head) {
            Some((length, hash)) => {
                let body = format!("len={length} fnv={hash:016x}\n");
                write_fixed(&mut stream, "200 OK", body.as_bytes());
            }
            None => write_fixed(&mut stream, "400 Bad Request", b"bad body\n"),
        },
        ("GET", other) if other.starts_with("/bytes/") => {
            match other["/bytes/".len()..].parse::<u64>() {
                Ok(total) => write_bytes(&mut stream, total),
                Err(_) => write_fixed(&mut stream, "400 Bad Request", b"bad length\n"),
            }
        }
        ("GET", other) if other.starts_with("/chunked/") => {
            match other["/chunked/".len()..].parse::<u64>() {
                Ok(total) => write_chunked(&mut stream, total),
                Err(_) => write_fixed(&mut stream, "400 Bad Request", b"bad length\n"),
            }
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
        thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || handle(stream, requests))
            .expect("spawn connection thread");
    }
}
