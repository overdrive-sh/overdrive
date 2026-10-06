//! vfwd — throwaway spike binary (netns-density-295, guest vsock capture).
//!   vfwd guest|host ...      flow owner (control plane only; see owner.rs)
//!   vfwd agent               guest test-command agent (test harness, vsock 4000)
//!   vfwd tcp-client|tcp-server|tcp-stress|udp-client|udp-server|udp-stress
//!                            ordinary AF_INET test applications (know nothing about vsock)
mod owner;
mod sys;

use serde_json::json;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use sys::*;

fn arg(args: &[String], k: &str) -> Option<String> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1).cloned())
}
fn flag(args: &[String], k: &str) -> bool {
    args.iter().any(|a| a == k)
}

fn pat_a(i: usize) -> u8 {
    ((i * 7 + 3) % 251) as u8
}
fn pat_b(i: usize) -> u8 {
    ((i * 13 + 5) % 253) as u8
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args[0].as_str() {
        "guest" | "host" => owner::run(&args),
        "agent" => agent(),
        "tcp-client" => {
            let r = tcp_client(&args);
            println!("{}", r);
            if r["ok"] != true {
                std::process::exit(1)
            }
        }
        "tcp-server" => tcp_server(&args),
        "tcp-stress" => tcp_stress(&args),
        "udp-client" => udp_client(&args),
        "udp-server" => udp_server(&args),
        "udp-stress" => udp_stress(&args),
        "sockname" => sockname(&args),
        x => panic!("unknown subcommand {x}"),
    }
}

// ------------------------------------------------------------------ agent (test harness)
fn agent() {
    let l = vsock_listen(u32::MAX, 4000, libc::SOCK_STREAM).unwrap();
    println!("AGENT_READY");
    loop {
        let c = accept(l.as_raw_fd()).unwrap();
        std::thread::spawn(move || {
            let mut len = [0u8; 4];
            if read_exact_ctl(c.as_raw_fd(), &mut len, Duration::from_secs(10)).is_err() {
                return;
            }
            let mut cmd = vec![0u8; u32::from_be_bytes(len) as usize];
            if read_exact_ctl(c.as_raw_fd(), &mut cmd, Duration::from_secs(10)).is_err() {
                return;
            }
            let cmd = String::from_utf8_lossy(&cmd).to_string();
            let out = c.try_clone().unwrap();
            let err = c.try_clone().unwrap();
            let rc = Command::new("/bin/sh")
                .arg("-c")
                .arg(&cmd)
                .stdin(Stdio::null())
                .stdout(Stdio::from(out))
                .stderr(Stdio::from(err))
                .status()
                .map(|s| s.code().unwrap_or(-1))
                .unwrap_or(-2);
            let _ = write_all_ctl(c.as_raw_fd(), format!("\nAGENT_RC={rc}\n").as_bytes());
        });
    }
}

// ------------------------------------------------------------------ TCP litmus
fn tcp_client(args: &[String]) -> serde_json::Value {
    let dst: SocketAddrV4 = arg(args, "--dst").unwrap().parse().unwrap();
    let size: usize = arg(args, "--size").map(|s| s.parse().unwrap()).unwrap_or(1024);
    let banner = flag(args, "--banner");
    let rst = flag(args, "--expect-rst");
    let delay: u64 = arg(args, "--read-delay").map(|s| s.parse().unwrap()).unwrap_or(0);
    tcp_once(dst, size, banner, rst, delay, &Opts::from(args))
}

#[derive(Clone, Copy)]
struct Opts {
    early_ack: bool,
    hold_ms: u64,
    timeout_ms: u64,
}
impl Opts {
    fn from(args: &[String]) -> Self {
        Opts {
            early_ack: flag(args, "--early-ack"),
            hold_ms: arg(args, "--hold-ms").map(|s| s.parse().unwrap()).unwrap_or(0),
            timeout_ms: arg(args, "--timeout-ms").map(|s| s.parse().unwrap()).unwrap_or(8000),
        }
    }
}

fn tcp_once(dst: SocketAddrV4, size: usize, banner: bool, rst: bool, delay: u64, o: &Opts) -> serde_json::Value {
    let t0 = Instant::now();
    let mut r = json!({"ok":false,"dst":dst.to_string(),"size":size});
    let s = match TcpStream::connect_timeout(&SocketAddr::V4(dst), Duration::from_secs(5)) {
        Ok(s) => s,
        Err(e) => {
            r["err"] = json!(format!("connect: {e}"));
            return r;
        }
    };
    let connect_ms = t0.elapsed().as_secs_f64() * 1000.0;
    r["local"] = json!(s.local_addr().map(|a| a.to_string()).unwrap_or_default());
    r["peer"] = json!(s.peer_addr().map(|a| a.to_string()).unwrap_or_default());
    s.set_read_timeout(Some(Duration::from_millis(o.timeout_ms))).unwrap();
    let mut s = s;
    if banner {
        let mut b = [0u8; 64];
        if let Err(e) = s.read_exact(&mut b) {
            r["err"] = json!(format!("banner read: {e}"));
            r["stage"] = json!("banner");
            return r;
        }
        r["banner"] = json!(String::from_utf8_lossy(&b).trim_end_matches(['\0', ' ', '\n']).to_string());
    }
    // Early bytes: written immediately after connect() returns.
    let mut req = Vec::with_capacity(12 + size);
    req.extend_from_slice(b"LREQ");
    req.extend_from_slice(&(size as u64).to_be_bytes());
    req.extend((0..size).map(pat_a));
    if let Err(e) = s.write_all(&req) {
        r["err"] = json!(format!("write: {e}"));
        return r;
    }
    if o.early_ack {
        // K1 without FIN: the server acknowledges the request before our half-close
        let mut a = [0u8; 4];
        if let Err(e) = s.read_exact(&mut a) {
            r["err"] = json!(format!("early ack read: {e}"));
            r["stage"] = json!("early_ack");
            return r;
        }
    }
    if o.hold_ms > 0 {
        std::thread::sleep(Duration::from_millis(o.hold_ms));
    }
    let _ = s.shutdown(Shutdown::Write);
    if delay > 0 {
        std::thread::sleep(Duration::from_millis(delay));
    }
    let mut resp = Vec::new();
    match s.read_to_end(&mut resp) {
        Ok(_) => {}
        Err(e) => {
            r["err"] = json!(format!("read: {e}"));
            r["read_err_kind"] = json!(format!("{:?}", e.kind()));
            r["os_error"] = json!(e.raw_os_error());
            r["got"] = json!(resp.len());
            if rst && e.raw_os_error() == Some(libc::ECONNRESET) {
                r["ok"] = json!(true);
                r["reset_observed"] = json!(true);
            }
            return r;
        }
    }
    if rst {
        r["err"] = json!("expected reset, got clean EOF");
        r["got"] = json!(resp.len());
        return r;
    }
    let want = 12 + 48 + size;
    if resp.len() != want || &resp[..4] != b"LRSP" || u64::from_be_bytes(resp[4..12].try_into().unwrap()) != size as u64 {
        r["err"] = json!(format!("bad response len {} want {}", resp.len(), want));
        r["head"] = json!(String::from_utf8_lossy(&resp[..resp.len().min(64)]).to_string());
        return r;
    }
    if !resp[60..].iter().enumerate().all(|(i, b)| *b == pat_b(i)) {
        r["err"] = json!("response pattern mismatch");
        return r;
    }
    r["server_saw_peer"] = json!(String::from_utf8_lossy(&resp[12..60]).trim_end_matches(['\0', ' ']).to_string());
    r["ok"] = json!(true);
    r["connect_ms"] = json!(connect_ms);
    r["ms"] = json!(t0.elapsed().as_secs_f64() * 1000.0);
    r
}

fn tcp_server(args: &[String]) {
    let bind = arg(args, "--bind").unwrap();
    let banner = flag(args, "--banner");
    let rst = flag(args, "--rst");
    let delay: u64 = arg(args, "--read-delay").map(|s| s.parse().unwrap()).unwrap_or(0);
    let early_ack = flag(args, "--early-ack");
    let l = TcpListener::bind(&bind).unwrap();
    println!("{}", json!({"ev":"tcp_server_ready","bind":bind,"banner":banner,"rst":rst}));
    for c in l.incoming() {
        let Ok(mut c) = c else { continue };
        std::thread::spawn(move || {
            let peer = c.peer_addr().map(|a| a.to_string()).unwrap_or_default();
            let local = c.local_addr().map(|a| a.to_string()).unwrap_or_default();
            c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let res = (|| -> Result<usize, String> {
                if banner {
                    let mut b = format!("BANNER peer={peer} local={local}").into_bytes();
                    b.resize(64, b' ');
                    c.write_all(&b).map_err(|e| format!("banner write {e}"))?;
                }
                if delay > 0 {
                    std::thread::sleep(Duration::from_millis(delay));
                }
                let mut h = [0u8; 12];
                c.read_exact(&mut h).map_err(|e| format!("hdr {e}"))?;
                if &h[..4] != b"LREQ" {
                    return Err("bad magic".into());
                }
                let n = u64::from_be_bytes(h[4..12].try_into().unwrap()) as usize;
                if rst {
                    let fd = c.as_raw_fd();
                    let l = libc::linger { l_onoff: 1, l_linger: 0 };
                    unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_LINGER, &l as *const _ as *const _, 8) };
                    return Ok(0);
                }
                let mut body = vec![0u8; n];
                c.read_exact(&mut body).map_err(|e| format!("body {e}"))?;
                if !body.iter().enumerate().all(|(i, b)| *b == pat_a(i)) {
                    return Err("request pattern mismatch".into());
                }
                if early_ack {
                    c.write_all(b"ACK!").map_err(|e| format!("ack {e}"))?;
                }
                let mut z = [0u8; 1];
                match c.read(&mut z) {
                    Ok(0) => {}
                    Ok(_) => return Err("extra bytes after request".into()),
                    Err(e) => return Err(format!("eof wait {e}")),
                }
                let mut resp = Vec::with_capacity(60 + n);
                resp.extend_from_slice(b"LRSP");
                resp.extend_from_slice(&(n as u64).to_be_bytes());
                let mut p = peer.clone().into_bytes();
                p.resize(48, b' ');
                resp.extend_from_slice(&p);
                resp.extend((0..n).map(pat_b));
                c.write_all(&resp).map_err(|e| format!("resp {e}"))?;
                let _ = c.shutdown(Shutdown::Write);
                Ok(n)
            })();
            match res {
                Ok(n) => println!("{}", json!({"ev":"conn","peer":peer,"local":local,"bytes":n,"ok":true})),
                Err(e) => println!("{}", json!({"ev":"conn","peer":peer,"local":local,"ok":false,"err":e})),
            }
        });
    }
}

fn tcp_stress(args: &[String]) {
    let dst: SocketAddrV4 = arg(args, "--dst").unwrap().parse().unwrap();
    let n: usize = arg(args, "--iterations").unwrap().parse().unwrap();
    let max: usize = arg(args, "--max-size").map(|s| s.parse().unwrap()).unwrap_or(4096);
    let par: usize = arg(args, "--parallel").map(|s| s.parse().unwrap()).unwrap_or(1);
    let banner = flag(args, "--banner");
    let o = Opts::from(args);
    let t0 = Instant::now();
    let handles: Vec<_> = (0..par)
        .map(|t| {
            std::thread::spawn(move || {
                let mut fails = vec![];
                let mut ok = 0usize;
                let mut seed: u64 = 0x9e3779b97f4a7c15 ^ (t as u64 + 1);
                for i in (t..n).step_by(par) {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    let size = 1 + (seed as usize % max);
                    let r = tcp_once(dst, size, banner, false, 0, &o);
                    if r["ok"] == true {
                        ok += 1
                    } else {
                        fails.push(json!({"i":i,"r":r}))
                    }
                }
                (ok, fails)
            })
        })
        .collect();
    let mut ok = 0;
    let mut fails = vec![];
    for h in handles {
        let (o, f) = h.join().unwrap();
        ok += o;
        fails.extend(f);
    }
    let nf = fails.len();
    fails.truncate(20);
    println!("{}", json!({"ev":"tcp_stress","dst":dst.to_string(),"iterations":n,"parallel":par,"banner":banner,"early_ack":o.early_ack,"hold_ms":o.hold_ms,"timeout_ms":o.timeout_ms,"ok":ok,"failed":nf,"first_failures":fails,"s":t0.elapsed().as_secs_f64()}));
}

// ------------------------------------------------------------------ UDP litmus
fn udp_payload(seq: u8, n: usize) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seq)).collect()
}

fn udp_exchange(s: &UdpSocket, dst: SocketAddrV4, connected: bool, seq: u8, n: usize) -> serde_json::Value {
    let p = udp_payload(seq, n);
    let sent = if connected { s.send(&p) } else { s.send_to(&p, dst) };
    if let Err(e) = sent {
        return json!({"size":n,"ok":false,"err":format!("send {e}")});
    }
    let mut buf = vec![0u8; 65536];
    match s.recv_from(&mut buf) {
        Ok((m, from)) => {
            let want: Vec<u8> = p.iter().map(|b| b ^ 0xa5).collect();
            let ok = m == n && buf[..m] == want[..] && from == SocketAddr::V4(dst);
            json!({"size":n,"ok":ok,"got":m,"from":from.to_string()})
        }
        Err(e) => json!({"size":n,"ok":false,"err":format!("recv {e}")}),
    }
}

fn udp_client(args: &[String]) {
    let dst: SocketAddrV4 = arg(args, "--dst").unwrap().parse().unwrap();
    let sizes: Vec<usize> = arg(args, "--sizes").unwrap_or("0,1,1431,59000".into()).split(',').map(|s| s.parse().unwrap()).collect();
    let connected = flag(args, "--connect");
    let s = UdpSocket::bind("0.0.0.0:0").unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    if connected {
        s.connect(dst).unwrap();
    }
    let local_before = s.local_addr().unwrap().to_string();
    let res: Vec<_> = sizes.iter().enumerate().map(|(i, n)| udp_exchange(&s, dst, connected, i as u8, *n)).collect();
    let all = res.iter().all(|r| r["ok"] == true);
    let peer = if connected { s.peer_addr().map(|a| a.to_string()).unwrap_or_default() } else { String::new() };
    println!("{}", json!({"ev":"udp_client","dst":dst.to_string(),"connected":connected,"local":local_before,"peer":peer,"ok":all,"results":res}));
    if !all {
        std::process::exit(1)
    }
}

fn udp_server(args: &[String]) {
    let bind = arg(args, "--bind").unwrap();
    let s = UdpSocket::bind(&bind).unwrap();
    println!("{}", json!({"ev":"udp_server_ready","bind":bind}));
    let mut buf = vec![0u8; 65536];
    let mut n = 0u64;
    loop {
        let Ok((m, from)) = s.recv_from(&mut buf) else { continue };
        let out: Vec<u8> = buf[..m].iter().map(|b| b ^ 0xa5).collect();
        let _ = s.send_to(&out, from);
        n += 1;
        if n <= 20 || n % 1000 == 0 {
            println!("{}", json!({"ev":"dgram","from":from.to_string(),"len":m,"n":n}));
        }
    }
}

fn udp_stress(args: &[String]) {
    let dst: SocketAddrV4 = arg(args, "--dst").unwrap().parse().unwrap();
    let n: usize = arg(args, "--iterations").unwrap().parse().unwrap();
    let connected = flag(args, "--connect");
    let size: usize = arg(args, "--size").map(|s| s.parse().unwrap()).unwrap_or(32);
    let tmo: u64 = arg(args, "--timeout-ms").map(|s| s.parse().unwrap()).unwrap_or(3000);
    let t0 = Instant::now();
    let mut ok = 0;
    let mut fails = vec![];
    for i in 0..n {
        // fresh socket each iteration: every exchange is a FIRST datagram
        let s = UdpSocket::bind("0.0.0.0:0").unwrap();
        s.set_read_timeout(Some(Duration::from_millis(tmo))).unwrap();
        if connected {
            if let Err(e) = s.connect(dst) {
                if fails.len() < 20 {
                    fails.push(json!({"i":i,"connect_err":e.to_string()}));
                }
                continue;
            }
        }
        let r = udp_exchange(&s, dst, connected, i as u8, size);
        if r["ok"] == true {
            ok += 1
        } else if fails.len() < 20 {
            fails.push(json!({"i":i,"r":r}))
        } else {
            fails.push(json!(null))
        }
    }
    let nf = n - ok;
    fails.retain(|f| !f.is_null());
    println!("{}", json!({"ev":"udp_stress","dst":dst.to_string(),"connected":connected,"iterations":n,"ok":ok,"failed":nf,"first_failures":fails,"s":t0.elapsed().as_secs_f64()}));
}

// D15: an application binding/reading the workload address.
fn sockname(args: &[String]) {
    let dst: SocketAddrV4 = arg(args, "--dst").unwrap().parse().unwrap();
    let bind = arg(args, "--bind");
    let fd = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
    let mut r = json!({});
    if let Some(b) = bind {
        r["bind"] = json!(b);
        r["bind_result"] = json!(bind4(fd.as_raw_fd(), b.parse().unwrap()).map(|_| "ok".to_string()).unwrap_or_else(|e| e.to_string()));
    }
    r["connect"] = json!(connect4(fd.as_raw_fd(), dst).map(|_| "ok".to_string()).unwrap_or_else(|e| e.to_string()));
    r["getsockname"] = json!(local4(fd.as_raw_fd()).map(|a| a.to_string()).unwrap_or_default());
    r["getpeername"] = json!(peer4(fd.as_raw_fd()).map(|a| a.to_string()).unwrap_or_default());
    let _ = unsafe { OwnedFd::from_raw_fd(libc::dup(fd.as_raw_fd())) };
    println!("{r}");
}
