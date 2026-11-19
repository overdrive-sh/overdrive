//! In-guest operator program (V-B2). Run by the production `overdrive-init` guest through
//! `overdrive-guest-peer run-with-modules <loopback.ko> -- init-return-guest <args>`.
//!
//! Plays the host's flow peer (loopback vsock to GUEST_INBOUND_PORT with `TcpAccept`) and the
//! application (TCP listener on the workload address), times each step on CLOCK_MONOTONIC, then
//! prints the framed series lines on stdout (the guest console) and exits at once.
//!
//! args: <workload_addr> <gateway> <launch_hex32> <accept_warm> <accept_n> <rt_warm> <rt_n> <bare|lead>
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::fd::{FromRawFd, OwnedFd};
use std::str::FromStr;
use std::time::{Duration, Instant};

use overdrive_core::guest_transport::wire::{FlowId, FlowOrigin, FlowRequest};
use overdrive_core::guest_transport::{GUEST_INBOUND_PORT, LaunchToken};
use overdrive_core::net::ListenPort;

const APP_PORT: u16 = 8080;
const MSG: usize = 64;
const HELLO: &[u8; 16] = b"app-to-vsock-ok!";

fn fnv(values: &[u64]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in values {
        for b in v.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

fn frame(out: &mut String, metric: &str, values: &[u64]) {
    use std::fmt::Write as _;
    let n = values.len();
    let _ = writeln!(out, "bench-series-begin: {metric} {n}");
    for chunk in values.chunks(16) {
        let l: Vec<String> = chunk.iter().map(u64::to_string).collect();
        let _ = writeln!(out, "bench-values: {metric} {}", l.join(","));
    }
    let _ = writeln!(out, "bench-series-end: {metric} {n} {:016x}", fnv(values));
}

fn vsock_connect_inbound() -> Result<std::fs::File, String> {
    // SAFETY: plain socket/connect on an fd this block then owns.
    unsafe {
        let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(format!("socket: {}", std::io::Error::last_os_error()));
        }
        let owned = OwnedFd::from_raw_fd(fd);
        let tv = libc::timeval { tv_sec: 10, tv_usec: 0 };
        for opt in [libc::SO_RCVTIMEO, libc::SO_SNDTIMEO] {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                opt,
                (&raw const tv).cast(),
                std::mem::size_of::<libc::timeval>() as libc::socklen_t,
            );
        }
        let mut addr: libc::sockaddr_vm = std::mem::zeroed();
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_cid = libc::VMADDR_CID_LOCAL;
        addr.svm_port = GUEST_INBOUND_PORT.as_u32();
        let rc = libc::connect(
            fd,
            (&raw const addr).cast::<libc::sockaddr>(),
            std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
        );
        if rc != 0 {
            return Err(format!("connect inbound: {}", std::io::Error::last_os_error()));
        }
        Ok(std::fs::File::from(owned))
    }
}

struct Ctx {
    gateway: Ipv4Addr,
    launch: LaunchToken,
    listener: TcpListener,
    seq: u32,
}

impl Ctx {
    /// Opens one flow: returns (vsock side, app side, ns from request write to accept return).
    fn open_flow(&mut self) -> Result<(std::fs::File, TcpStream, u64), String> {
        self.seq += 1;
        let mut vsock = vsock_connect_inbound()?;
        let request = FlowRequest::TcpAccept {
            flow: FlowId::new(FlowOrigin::Host, self.seq).map_err(|e| format!("flow id: {e:?}"))?,
            peer: self.gateway,
            listen_port: ListenPort::from_str(&APP_PORT.to_string()).map_err(|e| format!("{e:?}"))?,
            launch: self.launch,
        };
        let bytes = request.encode();
        let t0 = Instant::now();
        vsock.write_all(&bytes).map_err(|e| format!("write request: {e}"))?;
        let (app, _) = self.listener.accept().map_err(|e| format!("accept: {e}"))?;
        let ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
        app.set_nodelay(true).ok();
        app.set_read_timeout(Some(Duration::from_secs(10))).ok();
        app.set_write_timeout(Some(Duration::from_secs(10))).ok();
        Ok((vsock, app, ns))
    }
}

fn run(a: &[String], out: &mut String) -> Result<(), (String, String)> {
    let f = |m: &str| ("flow".to_owned(), m.to_owned());
    let workload: Ipv4Addr = a[1].parse().map_err(|e| f(&format!("workload addr: {e}")))?;
    let gateway: Ipv4Addr = a[2].parse().map_err(|e| f(&format!("gateway: {e}")))?;
    let launch = LaunchToken::from_str(&a[3]).map_err(|e| f(&format!("launch: {e:?}")))?;
    let p = |i: usize| a[i].parse::<usize>().map_err(|e| f(&format!("arg {i}: {e}")));
    let (aw, an, rw, rn) = (p(4)?, p(5)?, p(6)?, p(7)?);
    let listener = TcpListener::bind((workload, APP_PORT)).map_err(|e| f(&format!("bind: {e}")))?;
    let mut ctx = Ctx { gateway, launch, listener, seq: 0 };

    // accept_ns
    let mut accepts = Vec::with_capacity(an);
    for i in 0..(aw + an) {
        let (v, app, ns) = ctx.open_flow().map_err(|e| ("accept_ns".to_owned(), e))?;
        drop((v, app));
        if i >= aw {
            accepts.push(ns);
        }
    }

    // forwarded round trip, both directions
    let (mut vsock, mut app, _) = ctx.open_flow().map_err(|e| ("rtt_ns".to_owned(), e))?;
    let echo = std::thread::spawn(move || -> Result<(), String> {
        app.write_all(HELLO).map_err(|e| format!("app hello: {e}"))?;
        let mut buf = [0u8; MSG];
        loop {
            match app.read_exact(&mut buf) {
                Ok(()) => app.write_all(&buf).map_err(|e| format!("app echo write: {e}"))?,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(e) => return Err(format!("app read: {e}")),
            }
        }
    });
    let rf = |m: String| ("rtt_ns".to_owned(), m);
    let mut hello = [0u8; 16];
    vsock.read_exact(&mut hello).map_err(|e| rf(format!("vsock read hello (app->vsock): {e}")))?;
    if &hello != HELLO {
        return Err(rf(format!("app->vsock bytes differ: {hello:?}")));
    }
    let _ = &out; // direction results are reported after the blocks (see main)
    let mut rtts = Vec::with_capacity(rn);
    let mut req = [0u8; MSG];
    let mut rep = [0u8; MSG];
    let mut mismatches = 0u64;
    for i in 0..(rw + rn) {
        for (k, b) in req.iter_mut().enumerate() {
            *b = (i as u8).wrapping_add(k as u8);
        }
        let t0 = Instant::now();
        vsock.write_all(&req).map_err(|e| rf(format!("vsock write: {e}")))?;
        vsock.read_exact(&mut rep).map_err(|e| rf(format!("vsock read reply {i}: {e}")))?;
        let ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
        if req != rep {
            mismatches += 1;
        }
        if i >= rw {
            rtts.push(ns);
        }
    }
    if mismatches != 0 {
        return Err(rf(format!("{mismatches} echoed replies differed from their requests")));
    }
    drop(vsock);
    let _ = echo.join();
    frame(out, "accept_ns", &accepts);
    frame(out, "rtt_ns", &rtts);
    Ok(())
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let lead = a.get(8).is_some_and(|m| m == "lead");
    let started = Instant::now();
    let mut blocks = String::with_capacity(96 * 1024);
    let result = run(&a, &mut blocks);
    let workload_ns = started.elapsed().as_nanos();
    let mut out = String::with_capacity(blocks.len() + 512);
    // `lead`: one leading newline so the first block line starts at a line start even when the
    // production init's own partial log line is the console's open line. `bare`: nothing before.
    if lead {
        out.push('\n');
    }
    out.push_str(&blocks);
    let code = match result {
        Ok(()) => {
            out.push_str("direction-check: app_to_vsock=ok\n");
            out.push_str("direction-check: vsock_to_app_echoed=ok\n");
            0
        }
        Err((metric, reason)) => {
            out.push_str(&format!("bench-failure: {metric} {reason}\n"));
            40
        }
    };
    let rel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    out.push_str(&format!("init-return-guest-kernel: {}\n", rel.trim()));
    out.push_str(&format!("init-return-workload-ns: {workload_ns}\n"));
    // One write of the whole block, then exit at once (no sleep, no drain step).
    let t = Instant::now();
    let mut so = std::io::stdout().lock();
    let _ = so.write_all(out.as_bytes());
    let _ = so.flush();
    let _ = writeln!(so, "init-return-print-ns: {}", t.elapsed().as_nanos());
    let _ = so.flush();
    drop(so);
    std::process::exit(code);
}
