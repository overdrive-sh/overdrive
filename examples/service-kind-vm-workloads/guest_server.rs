use std::env;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const BODY: &[u8] = b"SVM-E08-GUEST-OK";
const FAILURE_DIAGNOSTIC_SENTINEL: &[u8] = b"SVM-E10-FAILURE-BODY-MUST-NOT-LEAK";

#[derive(Debug)]
struct Config {
    raw_port: u16,
    http_port: u16,
    ready_sequence: Vec<u16>,
    ready_phase: Duration,
    liveness_fail_after: Option<Duration>,
}

impl Config {
    fn from_args() -> Self {
        let mut config = Self {
            raw_port: 18_081,
            http_port: 18_080,
            ready_sequence: vec![204],
            ready_phase: Duration::from_secs(4),
            liveness_fail_after: None,
        };
        let mut args = env::args().skip(1);
        while let Some(flag) = args.next() {
            let value = args.next().unwrap_or_else(|| panic!("missing value for {flag}"));
            match flag.as_str() {
                "--raw-port" => config.raw_port = parse_u16(&value, &flag),
                "--http-port" => config.http_port = parse_u16(&value, &flag),
                "--ready-status" => config.ready_sequence = vec![parse_status(&value)],
                "--ready-sequence" => {
                    config.ready_sequence = value.split(',').map(parse_status).collect()
                }
                "--ready-phase-ms" => {
                    config.ready_phase = Duration::from_millis(parse_u64(&value, &flag))
                }
                "--liveness-fail-after-ms" => {
                    config.liveness_fail_after =
                        Some(Duration::from_millis(parse_u64(&value, &flag)));
                }
                _ => panic!("unknown argument {flag}"),
            }
        }
        config
    }
}

fn parse_u16(value: &str, flag: &str) -> u16 {
    value.parse().unwrap_or_else(|_| panic!("invalid {flag}: {value}"))
}

fn parse_u64(value: &str, flag: &str) -> u64 {
    value.parse().unwrap_or_else(|_| panic!("invalid {flag}: {value}"))
}

fn parse_status(value: &str) -> u16 {
    let status = parse_u16(value, "HTTP status");
    assert!((100..=599).contains(&status), "invalid HTTP status: {status}");
    status
}

fn serve_raw(port: u16) {
    let listener = TcpListener::bind(("0.0.0.0", port)).expect("bind raw guest listener");
    for incoming in listener.incoming() {
        let mut stream = incoming.expect("accept raw connection");
        let mut request = [0_u8; 64];
        let _ = stream.read(&mut request);
        stream.write_all(BODY).expect("write raw guest response");
    }
}

fn ready_status(config: &Config, elapsed: Duration) -> u16 {
    let phase_ms = config.ready_phase.as_millis().max(1);
    let index = (elapsed.as_millis() / phase_ms) as usize;
    config.ready_sequence[index.min(config.ready_sequence.len() - 1)]
}

fn reason(status: u16) -> &'static str {
    match status {
        204 => "No Content",
        302 => "Found",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ if (200..300).contains(&status) => "OK",
        _ => "Probe Status",
    }
}

fn write_status(stream: &mut impl Write, status: u16) {
    let body: &[u8] = if status == 503 { FAILURE_DIAGNOSTIC_SENTINEL } else { &[] };
    let response = format!(
        "HTTP/1.1 {status} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len()
    );
    stream.write_all(response.as_bytes()).expect("write health response header");
    stream.write_all(body).expect("write bounded health response body");
}

fn serve_http(config: Arc<Config>, started: Instant) {
    let listener =
        TcpListener::bind(("0.0.0.0", config.http_port)).expect("bind HTTP guest listener");
    for incoming in listener.incoming() {
        let mut stream = incoming.expect("accept HTTP connection");
        let mut request = [0_u8; 1024];
        let count = stream.read(&mut request).expect("read bounded HTTP request");
        let request = String::from_utf8_lossy(&request[..count]);
        if request.starts_with("GET /ready ") {
            write_status(&mut stream, ready_status(&config, started.elapsed()));
        } else if request.starts_with("GET /live ") {
            let status = config
                .liveness_fail_after
                .filter(|deadline| started.elapsed() >= *deadline)
                .map_or(204, |_| 503);
            write_status(&mut stream, status);
        } else {
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                BODY.len()
            );
            stream.write_all(header.as_bytes()).expect("write response header");
            stream.write_all(BODY).expect("write response body");
        }
    }
}

fn main() {
    let config = Arc::new(Config::from_args());
    let raw_port = config.raw_port;
    let started = Instant::now();
    let raw = thread::spawn(move || serve_raw(raw_port));
    let http = thread::spawn(move || serve_http(config, started));
    raw.join().expect("raw listener thread remains live");
    http.join().expect("HTTP listener thread remains live");
}
