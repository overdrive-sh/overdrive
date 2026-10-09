//! V-13 observers: ordinary AF_INET stream applications that know nothing about vsock.
//! Each streams a byte pattern in both directions continuously ("mid-transfer") and records
//! HOW its connection ended: EOF (recv()==0) or an errno (ECONNRESET ...), separately for the
//! read side and the write side, with CLOCK_REALTIME timestamps.
use crate::sys::*;
use serde_json::json;
use std::net::{SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn now_ns() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
}
fn ename(e: i32) -> &'static str {
    match e {
        libc::ECONNRESET => "ECONNRESET",
        libc::EPIPE => "EPIPE",
        libc::ETIMEDOUT => "ETIMEDOUT",
        libc::ECONNREFUSED => "ECONNREFUSED",
        libc::ENOTCONN => "ENOTCONN",
        libc::ECONNABORTED => "ECONNABORTED",
        libc::EAGAIN => "EAGAIN",
        _ => "OTHER",
    }
}

#[derive(Clone)]
struct Ev {
    kind: &'static str, // "rx_eof" | "rx_err" | "tx_err"
    errno: i32,
    t_ns: u128,
}

fn ev_json(e: &Ev) -> serde_json::Value {
    json!({"kind": e.kind, "errno": e.errno, "name": if e.errno != 0 { ename(e.errno) } else { "" }, "t_ns": e.t_ns.to_string()})
}

/// The verdict is the FIRST terminal event, which is what the application experiences first:
/// a recv()==0 is EOF (and, once SOCK_DONE is set, recv keeps returning 0 even if an RST follows);
/// ECONNRESET from recv or send is a reset. A single thread, recv before send, so a kernel error
/// consumed by one call cannot be misattributed by a racing sibling thread.
fn verdict(events: &[Ev]) -> &'static str {
    match events.first() {
        None => "NONE",
        Some(e) if e.kind == "rx_eof" => "EOF",
        Some(e) if e.errno == libc::ECONNRESET => "RESET",
        Some(_) => "OTHER_ERR",
    }
}

fn write_out(path: &str, role: &str, peer: &str, local: &str, start_ns: u128, rx: u64, tx: u64, events: &[Ev], done: bool, tx_shut: bool) {
    let v = json!({"role": role, "local": local, "peer": peer, "start_ns": start_ns.to_string(), "done": done,
        "bytes_rx": rx, "bytes_tx": tx, "events": events.iter().map(ev_json).collect::<Vec<_>>(),
        "first_event": events.first().map(ev_json), "rx_verdict": verdict(events), "finite_tx_shut": tx_shut});
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, v.to_string()).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

/// Stream until the first terminal event plus a 2 s grace (to record what follows), or `max_secs`.
pub fn session(stream: TcpStream, role: &str, out: &str, max_secs: u64, sleep_us: u64, finite: Option<u64>) {
    let fd = stream.as_raw_fd();
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    let local = stream.local_addr().map(|a| a.to_string()).unwrap_or_default();
    stream.set_nonblocking(true).unwrap();
    let chunk: Vec<u8> = (0..4096usize).map(|i| ((i * 7 + 3) % 251) as u8).collect();
    let mut buf = vec![0u8; 65536];
    let (mut rx, mut tx) = (0u64, 0u64);
    let mut events: Vec<Ev> = vec![];
    let (mut rx_dead, mut tx_dead) = (false, false);
    let start_ns = now_ns();
    let t0 = Instant::now();
    let mut first_term: Option<Instant> = None;
    let mut tx_shut = false;
    let mut last_status = Instant::now();
    let mut last_send = Instant::now();
    write_out(out, role, &peer, &local, start_ns, rx, tx, &events, false, tx_shut);
    loop {
        if t0.elapsed() > Duration::from_secs(max_secs) {
            break;
        }
        if let Some(f) = first_term {
            if f.elapsed() > Duration::from_secs(2) || (rx_dead && (tx_dead || tx_shut)) {
                break;
            }
        }
        let mut pfd = libc::pollfd { fd, events: (libc::POLLIN | libc::POLLOUT | libc::POLLRDHUP) as i16, revents: 0 };
        unsafe { libc::poll(&mut pfd, 1, 1) };
        // 1. read side first
        if !rx_dead {
            loop {
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), libc::MSG_DONTWAIT) };
                if n > 0 {
                    rx += n as u64;
                    continue;
                }
                if n == 0 {
                    rx_dead = true;
                    events.push(Ev { kind: "rx_eof", errno: 0, t_ns: now_ns() });
                    first_term.get_or_insert(Instant::now());
                    break;
                }
                let e = errno().raw_os_error().unwrap_or(0);
                if e == libc::EAGAIN || e == libc::EINTR {
                    break;
                }
                rx_dead = true;
                events.push(Ev { kind: "rx_err", errno: e, t_ns: now_ns() });
                first_term.get_or_insert(Instant::now());
                break;
            }
        }
        // 2. write side (paced); after the read side ended keep writing to learn how the write side ends
        if !tx_dead && !tx_shut && last_send.elapsed() >= Duration::from_micros(sleep_us) {
            for _ in 0..2 {
                if let Some(total) = finite {
                    if tx >= total {
                        unsafe { libc::shutdown(fd, libc::SHUT_WR) };
                        tx_shut = true;
                        break;
                    }
                }
                let want = match finite { Some(total) => chunk.len().min((total - tx) as usize), None => chunk.len() };
                let n = unsafe { libc::send(fd, chunk.as_ptr() as *const _, want, libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT) };
                if n > 0 {
                    tx += n as u64;
                    last_send = Instant::now();
                    continue;
                }
                let e = errno().raw_os_error().unwrap_or(0);
                if n < 0 && (e == libc::EAGAIN || e == libc::EINTR) {
                    break;
                }
                tx_dead = true;
                events.push(Ev { kind: "tx_err", errno: e, t_ns: now_ns() });
                first_term.get_or_insert(Instant::now());
                break;
            }
        }
        if last_status.elapsed() > Duration::from_millis(100) {
            write_out(out, role, &peer, &local, start_ns, rx, tx, &events, false, tx_shut);
            last_status = Instant::now();
        }
    }
    write_out(out, role, &peer, &local, start_ns, rx, tx, &events, true, tx_shut);
    drop(stream);
}

fn arg(args: &[String], k: &str) -> Option<String> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1).cloned())
}

pub fn client(args: &[String]) {
    let dst: SocketAddrV4 = arg(args, "--dst").unwrap().parse().unwrap();
    let out = arg(args, "--out").unwrap();
    let max: u64 = arg(args, "--max-secs").map(|s| s.parse().unwrap()).unwrap_or(40);
    let sleep_us: u64 = arg(args, "--sleep-us").map(|s| s.parse().unwrap()).unwrap_or(200);
    let finite: Option<u64> = arg(args, "--finite").map(|s| s.parse().unwrap());
    let s = TcpStream::connect_timeout(&SocketAddr::V4(dst), Duration::from_secs(5)).expect("connect");
    session(s, "client", &out, max, sleep_us, finite);
}

pub fn server(args: &[String]) {
    let bind: SocketAddrV4 = arg(args, "--bind").unwrap().parse().unwrap();
    let out = arg(args, "--out").unwrap();
    let max: u64 = arg(args, "--max-secs").map(|s| s.parse().unwrap()).unwrap_or(40);
    let sleep_us: u64 = arg(args, "--sleep-us").map(|s| s.parse().unwrap()).unwrap_or(200);
    let l = TcpListener::bind(bind).expect("bind");
    println!("STREAM_SERVER_READY {bind}");
    let (s, _) = l.accept().expect("accept");
    drop(l);
    // `--once` semantics: serve this one connection, then exit.
    let finite: Option<u64> = arg(args, "--finite").map(|s| s.parse().unwrap());
    session(s, "server", &out, max, sleep_us, finite);
}
