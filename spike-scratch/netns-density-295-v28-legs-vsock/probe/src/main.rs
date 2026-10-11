//! v28: leg-S over vsock. One static binary, role by subcommand:
//!   agent   guest control agent (len-prefixed shell commands over vsock 4000)
//!   gapp    guest application stand-in (byte-exact sink/source, http, never-accept, silent)
//!   gagent  guest TcpAccept receiver (vsock 5000 -> TCP connect to the app, optional IP_TRANSPARENT peer)
//!   relay   host leg-C / leg-S owner (rustls -> kTLS RX/TX, then forward to vsock or plain TCP)
//!   client  host TLS peer (stand-in for the originating agent's leg B), scenarios + bench
//!   probe   host owner-opened flow health probe (tcp-connect / http)
mod app;
mod duplex;
mod sys;
mod tlsk;
mod tp;
use app::*;
use duplex::*;
use serde_json::{json, Value};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use sys::*;

const F_NATIVE: u8 = 1;
const F_TRANSPARENT: u8 = 2;
const VPORT_FLOW: u32 = 5000;

fn header(flags: u8, peer: SocketAddrV4, dst: SocketAddrV4, mark: u32, up: u64, down: u64) -> [u8; 40] {
    let mut h = [0u8; 40];
    h[0..4].copy_from_slice(b"TACC");
    h[4] = 1;
    h[5] = flags;
    h[8..12].copy_from_slice(&peer.ip().octets());
    h[12..14].copy_from_slice(&peer.port().to_be_bytes());
    h[14..16].copy_from_slice(&dst.port().to_be_bytes());
    h[16..20].copy_from_slice(&dst.ip().octets());
    h[20..24].copy_from_slice(&mark.to_be_bytes());
    h[24..32].copy_from_slice(&up.to_be_bytes());
    h[32..40].copy_from_slice(&down.to_be_bytes());
    h
}

// ------------------------------------------------------------------ guest control agent
fn cmd_agent() {
    let l = vsock_listen(4000).expect("agent listen");
    loop {
        let Ok(c) = accept(l.as_raw_fd()) else { continue };
        std::thread::spawn(move || {
            let raw = c.as_raw_fd();
            let mut len = [0u8; 4];
            if read_exact(raw, &mut len).is_err() { return; }
            let mut cmd = vec![0u8; u32::from_be_bytes(len) as usize];
            if read_exact(raw, &mut cmd).is_err() { return; }
            let cmd = String::from_utf8_lossy(&cmd).to_string();
            let out = std::process::Command::new("/bin/sh").arg("-c").arg(&cmd).stdin(std::process::Stdio::null()).output();
            let (rc, mut text) = match out {
                Ok(o) => (o.status.code().unwrap_or(-1), [o.stdout, o.stderr].concat()),
                Err(e) => (-1, e.to_string().into_bytes()),
            };
            text.extend_from_slice(format!("\nAGENT_RC={rc}\n").as_bytes());
            let _ = write_all(raw, &text);
        });
    }
}

// ------------------------------------------------------------------ guest TcpAccept receiver
static FLOW_ID: AtomicU64 = AtomicU64::new(1);
fn status_reply(fd: RawFd, code: u8, e: i32) {
    let mut b = vec![code];
    if code != b'P' { b.extend_from_slice(&(e as u32).to_be_bytes()); }
    let _ = write_all(fd, &b);
}
fn cmd_gagent(a: &[String]) {
    let m = args_map(a);
    if let Some(p) = m.get("log") { log_open(p); }
    set_eof_class(&m);
    if EOF_CLASS.load(Ordering::SeqCst) == 2 { tp::init().expect("tp init"); }
    let l = vsock_listen(VPORT_FLOW).expect("gagent listen");
    log(json!({"ev": "gagent_listening", "uname": uname()}));
    loop {
        let Ok(c) = accept(l.as_raw_fd()) else { continue };
        std::thread::spawn(move || {
            let t0 = now_us();
            let id = FLOW_ID.fetch_add(1, Ordering::Relaxed);
            let fd = c.as_raw_fd();
            set_timeouts(fd, 5000);
            let mut h = [0u8; 40];
            if read_exact(fd, &mut h).is_err() || &h[0..4] != b"TACC" { log(json!({"ev": "gagent_bad_header", "id": id})); return; }
            set_timeouts(fd, 0);
            let flags = h[5];
            let peer = SocketAddrV4::new(Ipv4Addr::new(h[8], h[9], h[10], h[11]), u16::from_be_bytes([h[12], h[13]]));
            let dst = SocketAddrV4::new(Ipv4Addr::new(h[16], h[17], h[18], h[19]), u16::from_be_bytes([h[14], h[15]]));
            let mark = u32::from_be_bytes([h[20], h[21], h[22], h[23]]);
            let up = u64::from_be_bytes(h[24..32].try_into().unwrap());
            let down = u64::from_be_bytes(h[32..40].try_into().unwrap());
            if flags & F_NATIVE != 0 {
                status_reply(fd, b'P', 0);
                let sv = Serve { send: down, send_after_read: false, end_after_read: true, end: End::Fin, rst_after: None, hang_after: None };
                let mut v = serve_conn(fd, &sv, "native", "vsock");
                v["id"] = json!(id);
                v["up_expected"] = json!(up);
                log(v);
                return;
            }
            let t = match socket(libc::AF_INET, libc::SOCK_STREAM) { Ok(t) => t, Err(_) => return };
            let tfd = t.as_raw_fd();
            if flags & F_TRANSPARENT != 0 && !peer.ip().is_unspecified() {
                let r1 = setopt(tfd, libc::SOL_IP, 19, 1); // IP_TRANSPARENT
                let r2 = setopt(tfd, libc::SOL_SOCKET, libc::SO_MARK, mark as i32);
                let r3 = bind4(tfd, peer);
                if r1.is_err() || r2.is_err() || r3.is_err() {
                    log(json!({"ev": "gagent_bind_fail", "id": id, "r1": format!("{r1:?}"), "r2": format!("{r2:?}"), "r3": format!("{r3:?}")}));
                    status_reply(fd, b'R', errno());
                    return;
                }
            }
            let tc = now_us();
            match connect4_timeout(tfd, dst, 1500) {
                Ok(()) => {
                    status_reply(fd, b'P', 0);
                    let local = local4(tfd).map(|a| a.to_string()).unwrap_or_default();
                    log(json!({"ev": "gagent_paired", "id": id, "connect_us": now_us() - tc, "tcp_local": local, "peer_req": peer.to_string()}));
                    let r = duplex(Leg::new(fd, Kind::Vsock), Leg::new(tfd, Kind::Tcp), Fwd::Copy, id);
                    log(json!({"ev": "gagent_done", "id": id, "r": r, "us": now_us() - t0}));
                }
                Err(e) => {
                    let code = if e == libc::ETIMEDOUT { b'T' } else { b'R' };
                    log(json!({"ev": "gagent_not_paired", "id": id, "errno": e, "connect_us": now_us() - tc}));
                    status_reply(fd, code, e);
                }
            }
            drop(t);
            drop(c);
        });
    }
}

// ------------------------------------------------------------------ host relay (leg C / leg S)
/// open leg S over vsock; returns (fd, status) where status is "P" | "R:errno" | "T:errno" | "vsock_err:errno"
fn open_vsock_leg(cid: u32, h: &[u8; 40], to_ms: u64) -> Result<OwnedFd, String> {
    let v = vsock_connect(cid, VPORT_FLOW).map_err(|e| format!("vsock_connect_err:{}", e.raw_os_error().unwrap_or(0)))?;
    write_all(v.as_raw_fd(), h).map_err(|e| format!("hdr_write_err:{e}"))?;
    set_timeouts(v.as_raw_fd(), to_ms);
    let mut s = [0u8; 1];
    read_exact(v.as_raw_fd(), &mut s).map_err(|e| format!("status_read_err:{e}"))?;
    if s[0] != b'P' {
        let mut e = [0u8; 4];
        let _ = read_exact(v.as_raw_fd(), &mut e);
        return Err(format!("{}:{}", s[0] as char, u32::from_be_bytes(e)));
    }
    set_timeouts(v.as_raw_fd(), 0);
    Ok(v)
}
fn cmd_relay(a: &[String]) {
    let m = args_map(a);
    if let Some(p) = m.get("events") { log_open(p); }
    set_eof_class(&m);
    if EOF_CLASS.load(Ordering::SeqCst) == 2 { tp::init().expect("tp init"); }
    let dir = m["dir"].clone();
    let cfg = tlsk::server_cfg(&dir);
    let listen: SocketAddrV4 = m["listen"].parse().unwrap();
    let l = tcp_listen(listen, 1024).unwrap();
    let legs = m.get("legs").cloned().unwrap_or("vsock".into());
    let cid: u32 = m.get("cid").map(|v| v.parse().unwrap()).unwrap_or(0);
    let dst: SocketAddrV4 = m.get("dst").map(|v| v.parse().unwrap()).unwrap_or("127.0.0.1:0".parse().unwrap());
    let fwd = if m.get("fwd").map(|s| s.as_str()) == Some("copy") { Fwd::Copy } else { Fwd::Splice };
    let peer_mode = m.get("peer-mode").cloned().unwrap_or("legc".into());
    let gateway: Ipv4Addr = m.get("gateway").map(|v| v.parse().unwrap()).unwrap_or(Ipv4Addr::new(198, 18, 28, 254));
    let native = m.contains_key("native");
    let nup: u64 = m.get("up").map(|v| v.parse().unwrap()).unwrap_or(0);
    let ndown: u64 = m.get("down").map(|v| v.parse().unwrap()).unwrap_or(0);
    let mark: u32 = 1;
    log(json!({"ev": "relay_ready", "uname": uname(), "legs": legs, "fwd": format!("{fwd:?}"), "peer_mode": peer_mode, "native": native}));
    println!("RELAY_READY");
    let ids = AtomicU64::new(1);
    loop {
        let Ok(c) = accept(l.as_raw_fd()) else { continue };
        let id = ids.fetch_add(1, Ordering::Relaxed);
        let (cfg, legs, peer_mode) = (cfg.clone(), legs.clone(), peer_mode.clone());
        std::thread::spawn(move || {
            let t0 = now_us();
            let cfd = c.as_raw_fd();
            let peer_c = peer4(cfd).unwrap_or(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));
            let (sec, early) = match tlsk::server_handshake(cfd, cfg) {
                Ok(x) => x,
                Err(e) => { log(json!({"ev": "relay_hs_fail", "id": id, "err": e.to_string()})); return; }
            };
            if let Err(e) = tlsk::arm_ktls(cfd, sec) { log(json!({"ev": "relay_ktls_fail", "id": id, "err": e.to_string()})); return; }
            let t_hs = now_us();
            let s: Result<(OwnedFd, Kind), String> = if legs == "tcp" {
                (|| {
                    let t = socket(libc::AF_INET, libc::SOCK_STREAM).map_err(|e| e.to_string())?;
                    connect4_timeout(t.as_raw_fd(), dst, 3000).map_err(|e| format!("R:{e}"))?;
                    Ok((t, Kind::Tcp))
                })()
            } else {
                let peer = match peer_mode.as_str() {
                    "legc" => peer_c,
                    "gateway" => SocketAddrV4::new(gateway, 0),
                    _ => SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
                };
                let mut fl = if peer_mode == "none" { 0 } else { F_TRANSPARENT };
                if native { fl |= F_NATIVE; }
                let h = header(fl, peer, dst, mark, nup, ndown);
                open_vsock_leg(cid, &h, 5000).map(|v| (v, Kind::Vsock))
            };
            let t_s = now_us();
            let (sfd, kind) = match s {
                Ok(x) => x,
                Err(st) => {
                    log(json!({"ev": "relay_legs_not_paired", "id": id, "status": st, "legs_open_us": t_s - t_hs}));
                    linger0(cfd);
                    return;
                }
            };
            log(json!({"ev": "relay_paired", "id": id, "peer_c": peer_c.to_string(), "hs_us": t_hs - t0, "legs_open_us": t_s - t_hs, "early": early.len()}));
            if !early.is_empty() { let _ = write_all(sfd.as_raw_fd(), &early); }
            let r = duplex(Leg::new(cfd, Kind::Ktls), Leg::new(sfd.as_raw_fd(), kind), fwd, id);
            log(json!({"ev": "relay_done", "id": id, "r": r, "us": now_us() - t0}));
            drop(sfd);
            drop(c);
        });
    }
}

// ------------------------------------------------------------------ host TLS peer (client)
fn client_connect(dir: &str, to: SocketAddrV4, src: Option<Ipv4Addr>, cfg: &Arc<rustls::ClientConfig>) -> Result<OwnedFd, String> {
    let s = socket(libc::AF_INET, libc::SOCK_STREAM).map_err(|e| e.to_string())?;
    let _ = dir;
    if let Some(ip) = src { bind4(s.as_raw_fd(), SocketAddrV4::new(ip, 0)).map_err(|e| format!("bind:{e}"))?; }
    connect4(s.as_raw_fd(), to).map_err(|e| format!("connect:{e}"))?;
    let sec = tlsk::client_handshake(s.as_raw_fd(), cfg.clone()).map_err(|e| format!("hs:{e}"))?;
    tlsk::arm_ktls(s.as_raw_fd(), sec).map_err(|e| format!("ktls:{e}"))?;
    Ok(s)
}
fn client_once(m: &std::collections::BTreeMap<String, String>, cfg: &Arc<rustls::ClientConfig>) -> Value {
    let dir = m["dir"].clone();
    let to: SocketAddrV4 = m["connect"].parse().unwrap();
    let src: Option<Ipv4Addr> = m.get("src").map(|v| v.parse().unwrap());
    let up: u64 = m.get("up").map(|v| v.parse().unwrap()).unwrap_or(0);
    let down: u64 = m.get("down").map(|v| v.parse().unwrap()).unwrap_or(0);
    let end = m.get("end").cloned().unwrap_or("fin".into());
    let rst_at: Option<u64> = m.get("rst-at").map(|v| v.parse().unwrap());
    let pause: u64 = m.get("pause-ms").map(|v| v.parse().unwrap()).unwrap_or(10);
    let to_ms: u64 = m.get("timeout-ms").map(|v| v.parse().unwrap()).unwrap_or(20000);
    let send_after_eof = m.contains_key("send-after-eof");
    let t_a = now_us();
    let s = match client_connect(&dir, to, src, cfg) {
        Ok(s) => s,
        Err(e) => return json!({"ok": false, "setup_err": e}),
    };
    let fd = s.as_raw_fd();
    let local = local4(fd).map(|a| a.to_string()).unwrap_or_default();
    let t_hs = now_us() - t_a;
    sleep_ms(pause);
    set_timeouts(fd, to_ms);
    let t_start = now_us();
    let eof_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // reader
    let rst_mode = end == "rst";
    let ef = eof_flag.clone();
    let reader = if !rst_mode {
        Some(std::thread::spawn(move || {
            let mut buf = vec![0u8; 256 << 10];
            let (mut got, mut ok, mut bad) = (0u64, true, None);
            let mut t_first = 0u64;
            let end_kind;
            loop {
                let n = rd(fd, &mut buf);
                if n > 0 {
                    if t_first == 0 { t_first = now_us(); }
                    if ok {
                        if let Some(i) = check(&buf[..n as usize], START_B, got) { ok = false; bad = Some(got + i as u64); }
                    }
                    got += n as u64;
                } else if n == 0 {
                    end_kind = "eof".to_string();
                    break;
                } else {
                    let e = errno();
                    if e == libc::EINTR { continue; }
                    end_kind = match e {
                        libc::ECONNRESET => "reset".to_string(),
                        libc::EAGAIN => "timeout".to_string(),
                        libc::EIO => { let mut t = [0u8; 64]; match tlsk::recv_record(fd, &mut t) { Ok((21, _)) => "alert".to_string(), x => format!("EIO:{x:?}") } }
                        _ => format!("err:{e}"),
                    };
                    break;
                }
            }
            ef.store(true, Ordering::SeqCst);
            (got, ok, bad, end_kind, t_first, now_us())
        }))
    } else { None };
    // writer
    if send_after_eof {
        let t = std::time::Instant::now();
        while !eof_flag.load(Ordering::SeqCst) && t.elapsed().as_millis() < to_ms as u128 { sleep_ms(5); }
    }
    let lim = rst_at.map(|k| k.min(up)).unwrap_or(up);
    let mut sent = 0u64;
    let mut up_err = 0;
    let mut buf = vec![0u8; 256 << 10];
    // progress sampler (diagnostic: shows stalls and what a write sees after the path dies)
    let prog = Arc::new(AtomicU64::new(0));
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let samples = {
        let (p, d) = (prog.clone(), done.clone());
        std::thread::spawn(move || {
            let mut v: Vec<(u64, u64)> = vec![];
            let t = now_us();
            while !d.load(Ordering::SeqCst) {
                v.push(((now_us() - t) / 1000, p.load(Ordering::SeqCst)));
                sleep_ms(50);
            }
            v
        })
    };
    let mut last_write_err = 0;
    let mut short_writes = 0u64;
    while sent < lim {
        let n = ((lim - sent) as usize).min(buf.len());
        fill(&mut buf[..n], START_A, sent);
        let w = wr(fd, &buf[..n]);
        if w < 0 { let e = errno(); if e == libc::EINTR { continue; } up_err = e; last_write_err = e; break; }
        if (w as usize) < n { short_writes += 1; }
        sent += w as u64;
        prog.store(sent, Ordering::SeqCst);
    }
    done.store(true, Ordering::SeqCst);
    let samples = samples.join().unwrap();
    let t_sent = now_us();
    match end.as_str() {
        "fin" => { shutdown_wr(fd); }
        "notify" => { let _ = tlsk::send_close_notify(fd); shutdown_wr(fd); }
        "rst" => { linger0(fd); }
        _ => {}
    }
    let (got, ok, bad, down_end, t_first, t_end) = match reader {
        Some(h) => h.join().unwrap(),
        None => (0, true, None, "n/a".to_string(), 0, now_us()),
    };
    let mut r = json!({"ok": true, "local": local, "up_sent": sent, "up_err": up_err, "down_recv": got, "down_ok": ok, "down_bad_at": bad, "down_end": down_end,
        "down_expected": down, "t_hs_us": t_hs, "t_first_us": if t_first > 0 { t_first - t_start } else { 0 }, "t_sent_us": t_sent - t_start, "t_total_us": t_end - t_start, "short_writes": short_writes, "last_write_err": last_write_err,
        "progress_ms_bytes": if sent > 20_000_000 { json!(samples) } else { json!(null) }});
    if rst_mode { drop(s); r["rst_sent"] = json!(true); } else { drop(s); }
    r
}
fn cmd_client(a: &[String]) {
    let m = args_map(a);
    let cfg = tlsk::client_cfg(&m["dir"]);
    let reps: usize = m.get("reps").map(|v| v.parse().unwrap()).unwrap_or(1);
    if reps <= 1 {
        let mut v = client_once(&m, &cfg);
        v["tag"] = json!(m.get("tag").cloned().unwrap_or_default());
        println!("{v}");
        return;
    }
    let (mut times, mut fails, mut first): (Vec<u64>, Vec<String>, Vec<u64>) = (vec![], vec![], vec![]);
    let t0 = now_us();
    for _ in 0..reps {
        let v = client_once(&m, &cfg);
        let good = v["ok"] == true && v["down_end"] == "eof" && v["down_ok"] == true && v["up_err"] == 0
            && v["down_recv"] == v["down_expected"] && v["up_sent"].as_u64() == m.get("up").map(|x| x.parse::<u64>().unwrap()).or(Some(0));
        if good { times.push(v["t_total_us"].as_u64().unwrap()); first.push(v["t_first_us"].as_u64().unwrap()); } else { fails.push(v.to_string()); }
    }
    println!("{}", json!({"bench": true, "tag": m.get("tag"), "reps": reps, "ok": times.len(), "fail": fails.len(), "fail_examples": fails.iter().take(3).collect::<Vec<_>>(),
        "t_total_us": times, "t_first_us": first, "wall_us": now_us() - t0, "up": m.get("up"), "down": m.get("down")}));
}

// ------------------------------------------------------------------ host probe over an owner-opened flow
fn cmd_probe(a: &[String]) {
    let m = args_map(a);
    let cid: u32 = m["cid"].parse().unwrap();
    let dst: SocketAddrV4 = m["dst"].parse().unwrap();
    let kind = m.get("kind").cloned().unwrap_or("tcp".into());
    let to_ms: u64 = m.get("timeout-ms").map(|v| v.parse().unwrap()).unwrap_or(3000);
    let gateway: Ipv4Addr = m.get("gateway").map(|v| v.parse().unwrap()).unwrap_or(Ipv4Addr::new(198, 18, 28, 254));
    let t0 = now_us();
    let h = header(F_TRANSPARENT, SocketAddrV4::new(gateway, 0), dst, 1, 0, 0);
    let out = match open_vsock_leg(cid, &h, to_ms) {
        Err(st) => json!({"result": "unhealthy", "flow": st}),
        Ok(v) => {
            let fd = v.as_raw_fd();
            if kind == "tcp" {
                json!({"result": "healthy", "flow": "P"})
            } else {
                set_timeouts(fd, to_ms);
                let req = b"GET /healthz HTTP/1.1\r\nHost: probe\r\nConnection: close\r\n\r\n";
                let w = write_all(fd, req);
                let mut got = Vec::new();
                let mut b = vec![0u8; 4096];
                let mut rd_err = 0;
                while !got.windows(2).any(|w| w == b"\r\n") {
                    let n = rd(fd, &mut b);
                    if n <= 0 { rd_err = if n < 0 { errno() } else { 0 }; break; }
                    got.extend_from_slice(&b[..n as usize]);
                }
                let line = String::from_utf8_lossy(&got).lines().next().unwrap_or("").to_string();
                let code: u32 = line.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
                let healthy = (200..300).contains(&code);
                json!({"result": if healthy { "healthy" } else { "unhealthy" }, "flow": "P", "http_status": code, "status_line": line, "write": format!("{w:?}"), "read_errno": rd_err})
            }
        }
    };
    let mut out = out;
    out["latency_ms"] = json!((now_us() - t0) as f64 / 1000.0);
    println!("{out}");
}

/// `legs pk <SIG> <role>`: signal every process whose argv[1] == role (no pkill in the guest busybox).
fn cmd_pk(a: &[String]) {
    let sig: i32 = a[0].parse().unwrap();
    let me = std::process::id();
    let mut n = 0;
    for e in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        if pid == me { continue; }
        let Ok(c) = std::fs::read(format!("/proc/{pid}/cmdline")) else { continue };
        let args: Vec<&[u8]> = c.split(|b| *b == 0).collect();
        if args.len() > 1 && args[1] == a[1].as_bytes() && args[0].ends_with(b"legs") {
            unsafe { libc::kill(pid as i32, sig) };
            n += 1;
        }
    }
    println!("pk signalled {n}");
}

fn main() {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    let a: Vec<String> = std::env::args().collect();
    let rest = &a[2.min(a.len())..];
    match a.get(1).map(|s| s.as_str()) {
        Some("agent") => cmd_agent(),
        Some("gapp") => cmd_gapp(rest),
        Some("gagent") => cmd_gagent(rest),
        Some("relay") => cmd_relay(rest),
        Some("client") => cmd_client(rest),
        Some("probe") => cmd_probe(rest),
        Some("pk") => cmd_pk(rest),
        _ => eprintln!("usage: legs agent|gapp|gagent|relay|client|probe"),
    }
}
