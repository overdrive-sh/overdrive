//! Guest application stand-in ("gapp"): byte-exact sink/source, HTTP responder, never-accepting and
//! silent listeners. `serve_conn` also runs on a vsock fd for the "native" isolation mode.
use crate::sys::*;
use serde_json::json;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::SocketAddrV4;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::Mutex;
use std::time::Instant;

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);
pub fn log_open(path: &str) {
    *LOG.lock().unwrap() = Some(OpenOptions::new().create(true).append(true).open(path).unwrap());
}
pub fn log(v: serde_json::Value) {
    let mut g = LOG.lock().unwrap();
    let line = format!("{v}\n");
    if let Some(f) = g.as_mut() {
        let _ = f.write_all(line.as_bytes());
    } else {
        print!("{line}");
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum End { Fin, Close, Rst, None }
impl End {
    pub fn parse(s: &str) -> End {
        match s { "fin" => End::Fin, "close" => End::Close, "rst" => End::Rst, _ => End::None }
    }
}
#[derive(Clone)]
pub struct Serve {
    pub send: u64,
    pub send_after_read: bool,
    pub end_after_read: bool,
    pub end: End,
    pub rst_after: Option<u64>,
    pub hang_after: Option<u64>,
}
impl Serve {
    pub fn from_args(m: &std::collections::BTreeMap<String, String>) -> Serve {
        Serve {
            send: m.get("send").map(|v| v.parse().unwrap()).unwrap_or(0),
            send_after_read: m.contains_key("send-after-read"),
            end_after_read: m.contains_key("end-after-read"),
            end: End::parse(m.get("end").map(|s| s.as_str()).unwrap_or("fin")),
            rst_after: m.get("rst-after").map(|v| v.parse().unwrap()),
            hang_after: m.get("hang-after").map(|v| v.parse().unwrap()),
        }
    }
}

/// Single-threaded poll loop over one connection (so close()/RST semantics are the app's own).
pub fn serve_conn(fd: RawFd, p: &Serve, tag: &str, peer: &str) -> serde_json::Value {
    nonblock(fd, true);
    let t0 = Instant::now();
    let mut rbuf = vec![0u8; 256 << 10];
    let mut wbuf = vec![0u8; 256 << 10];
    let (mut up, mut up_ok, mut bad_at) = (0u64, true, None::<u64>);
    let mut up_end = "none".to_string();
    let (mut sent, mut down_err) = (0u64, 0i32);
    let (mut rd_done, mut wr_done, mut wr_dead) = (false, p.send == 0 && p.end == End::None, false);
    let mut end_action = "none";
    let mut sent_end = false;
    loop {
        let may_write = !wr_done && !wr_dead && (!p.send_after_read || rd_done);
        let mut ev = 0i16;
        if !rd_done { ev |= libc::POLLIN; }
        if may_write { ev |= libc::POLLOUT; }
        if ev == 0 && !sent_end { /* nothing to wait for but the end action */ }
        let re = if ev != 0 { poll1(fd, ev, 200) } else { 0 };
        if re & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 && !rd_done {
            loop {
                let n = rd(fd, &mut rbuf);
                if n > 0 {
                    let n = n as usize;
                    if up_ok {
                        if let Some(i) = check(&rbuf[..n], START_A, up) {
                            up_ok = false;
                            bad_at = Some(up + i as u64);
                        }
                    }
                    up += n as u64;
                    if let Some(k) = p.hang_after {
                        if up >= k {
                            log(json!({"tag": tag, "peer": peer, "ev": "hang", "up_bytes": up}));
                            loop { sleep_ms(1000); }
                        }
                    }
                    continue;
                } else if n == 0 {
                    rd_done = true;
                    up_end = "eof".into();
                } else {
                    let e = errno();
                    if e == libc::EAGAIN { break; }
                    if e == libc::EINTR { continue; }
                    rd_done = true;
                    up_end = format!("err:{}", if e == libc::ECONNRESET { "ECONNRESET".to_string() } else { e.to_string() });
                    wr_dead = true;
                }
                break;
            }
        }
        if may_write && re & libc::POLLOUT != 0 && !wr_dead {
            while sent < p.send {
                let lim = match p.rst_after { Some(k) if k < p.send => k, _ => p.send };
                let n = ((lim - sent) as usize).min(wbuf.len());
                if n == 0 { break; }
                fill(&mut wbuf[..n], START_B, sent);
                let w = wr(fd, &wbuf[..n]);
                if w > 0 { sent += w as u64; continue; }
                let e = errno();
                if e == libc::EAGAIN { break; }
                if e == libc::EINTR { continue; }
                down_err = e;
                wr_dead = true;
                break;
            }
            let lim = match p.rst_after { Some(k) if k < p.send => k, _ => p.send };
            if !wr_dead && sent >= lim {
                wr_done = true;
            }
        }
        if wr_done && !sent_end && (!p.end_after_read || rd_done) {
            sent_end = true;
            match p.end {
                End::Fin => { shutdown_wr(fd); end_action = "fin"; }
                End::Close => { end_action = "close"; break; }
                End::Rst => { linger0(fd); end_action = "rst"; break; }
                End::None => {}
            }
            if p.rst_after.map_or(false, |k| k < p.send) {
                linger0(fd);
                end_action = "rst";
                break;
            }
        }
        if rd_done && (wr_done || wr_dead) { break; }
        if rd_done && !may_write && !wr_done && !wr_dead && !p.send_after_read { break; }
        if t0.elapsed().as_secs() > 120 { up_end = "timeout".into(); break; }
    }
    let v = json!({"tag": tag, "peer": peer, "ev": "conn", "up_bytes": up, "up_ok": up_ok, "up_bad_at": bad_at, "up_end": up_end,
        "down_sent": sent, "down_err": down_err, "end_action": end_action, "ms": t0.elapsed().as_millis() as u64});
    // close happens when the caller drops/closes the fd
    v
}

fn http_conn(fd: RawFd, status: u32, peer: &str) {
    set_timeouts(fd, 5000);
    let mut buf = vec![0u8; 4096];
    let mut got = Vec::new();
    while !got.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = rd(fd, &mut buf);
        if n <= 0 { break; }
        got.extend_from_slice(&buf[..n as usize]);
    }
    let req = String::from_utf8_lossy(&got).lines().next().unwrap_or("").to_string();
    let resp = format!("HTTP/1.1 {status} X\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let _ = write_all(fd, resp.as_bytes());
    log(json!({"ev": "http", "peer": peer, "req": req, "status": status}));
}

pub fn cmd_gapp(a: &[String]) {
    let m = args_map(a);
    if let Some(p) = m.get("log") { log_open(p); }
    let addr: SocketAddrV4 = m["listen"].parse().unwrap();
    let mode = m.get("mode").map(|s| s.as_str()).unwrap_or("serve").to_string();
    let backlog: i32 = m.get("backlog").map(|v| v.parse().unwrap()).unwrap_or(128);
    let l = tcp_listen(addr, backlog).unwrap();
    log(json!({"ev": "listening", "addr": addr.to_string(), "mode": mode, "backlog": backlog}));
    let sv = Serve::from_args(&m);
    let status: u32 = m.get("status").map(|v| v.parse().unwrap()).unwrap_or(200);
    if mode == "noaccept" {
        loop { sleep_ms(1000); }
    }
    let tag = m.get("tag").cloned().unwrap_or_default();
    loop {
        let Ok(c) = accept(l.as_raw_fd()) else { continue };
        let sv = sv.clone();
        let mode = mode.clone();
        let tag = tag.clone();
        std::thread::spawn(move || {
            let fd = c.as_raw_fd();
            let peer = peer4(fd).map(|p| p.to_string()).unwrap_or_default();
            let me = local4(fd).map(|p| p.to_string()).unwrap_or_default();
            match mode.as_str() {
                "http" => { http_conn(fd, status, &peer); }
                "silent" => {
                    let mut b = vec![0u8; 65536];
                    loop { let n = rd(fd, &mut b); if n <= 0 { break; } }
                }
                _ => {
                    let mut v = serve_conn(fd, &sv, &tag, &peer);
                    v["local"] = json!(me);
                    log(v);
                }
            }
            drop(c);
        });
    }
}
