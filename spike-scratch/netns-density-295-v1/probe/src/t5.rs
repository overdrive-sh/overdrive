//! T5: payload-free discrimination of "peer closed or died" from "peer half-closed" at the transport
//! level, by a BPF program on the stable tracepoint vsock:virtio_transport_recv_pkt (op, flags).
//! Modes: obs (observer: accepts, epolls RDHUP, reads the tracepoint ring), runner (spawns peer
//! children and applies SIGKILL / shutdown(SHUT_WR) / close), child (one peer connection).
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::os::fd::{AsRawFd, OwnedFd};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::bpfh::{Attach, Bpf, ShutRec};
use crate::sys::*;
use crate::Opts;

const ACTIONS: [&str; 3] = ["kill", "shut_wr", "close"];
const VARIANTS: [&str; 2] = ["idle", "unread"];

fn sig(r: &ShutRec) -> String {
    match r.op {
        4 => format!("SHUTDOWN:{:#x}", r.flags),
        3 => "RST".to_string(),
        o => format!("op{o}"),
    }
}

pub fn run(o: &Opts) {
    match o.s("mode", "obs").as_str() {
        "obs" => obs(o),
        "runner" => runner(o),
        "child" => child(o),
        m => panic!("t5 --mode {m}"),
    }
}

fn child(o: &Opts) {
    let cid = o.u("cid", 1) as u32;
    let port = o.u("port", 7960) as u32;
    let action = o.u("action", 0) as u8;
    let variant = o.u("variant", 0) as u8;
    let fd = vsock_connect(cid, port, libc::SOCK_STREAM).expect("child connect");
    let raw = fd.as_raw_fd();
    let hdr = [action, variant];
    send_blocking(raw, &hdr);
    let mut ack = [0u8; 1];
    if recv_blocking(raw, &mut ack) != 1 {
        std::process::exit(3);
    }
    println!("READY");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    match action {
        0 => loop {
            sleep_ms(1000);
        },
        1 => {
            shutdown_wr(raw).unwrap();
            sleep_ms(1500);
        }
        _ => {
            drop(fd);
            sleep_ms(100);
        }
    }
}

fn runner(o: &Opts) {
    let cid = o.u("cid", 1);
    let port = o.u("port", 7960);
    let trials = o.u("trials", 110);
    let exe = std::env::current_exe().unwrap();
    let mut done = 0u64;
    let mut errs = 0u64;
    for (ai, _a) in ACTIONS.iter().enumerate() {
        for (vi, _v) in VARIANTS.iter().enumerate() {
            for _ in 0..trials {
                let mut c = Command::new(&exe)
                    .args(["t5", "--mode", "child", "--cid", &cid.to_string(), "--port", &port.to_string(), "--action", &ai.to_string(), "--variant", &vi.to_string()])
                    .stdout(Stdio::piped())
                    .spawn()
                    .expect("spawn child");
                let mut rd = BufReader::new(c.stdout.take().unwrap());
                let mut line = String::new();
                let ok = rd.read_line(&mut line).map(|n| n > 0).unwrap_or(false) && line.trim() == "READY";
                if !ok {
                    errs += 1;
                    let _ = c.kill();
                    let _ = c.wait();
                    continue;
                }
                if ai == 0 {
                    // SIGKILL while the socket is established and idle (or has unread data)
                    unsafe { libc::kill(c.id() as i32, libc::SIGKILL) };
                }
                let _ = c.wait();
                done += 1;
                sleep_ms(o.u("pace-ms", 40));
            }
        }
    }
    println!("{}", json!({"cmd":"t5-runner","done":done,"errors":errs}));
}

fn drain(ring: &mut aya::maps::RingBuf<aya::maps::MapData>, into: &mut Vec<ShutRec>) {
    while let Some(item) = ring.next() {
        if let Some(r) = ShutRec::parse(&item) {
            into.push(r);
        }
    }
}

fn obs(o: &Opts) {
    let port = o.u("port", 7960) as u32;
    let trials = o.u("trials", 110) * (ACTIONS.len() * VARIANTS.len()) as u64;
    let obj = o.s("obj", "/tmp/v1-work/out/drain-bpf.o");
    let bpf = Bpf::load(&obj, Attach { forwarder: false, tracepoint: true }, None).expect("bpf load");
    let mut ring = bpf.take_ring("SHUTRB");
    let l = vsock_listen(libc::VMADDR_CID_ANY, port, libc::SOCK_STREAM).expect("listen");
    println!("T5_OBS_READY");
    let ep = Epoll::new();
    // cell key: action/variant -> signature histogram
    let mut at_wake: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut at_late: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut missing_at_wake: BTreeMap<String, u64> = BTreeMap::new();
    let mut late_only: BTreeMap<String, u64> = BTreeMap::new();
    let mut pollbits: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut lag_ns: Vec<i64> = vec![];
    let mut n = 0u64;
    let mut timeouts = 0u64;
    let mut pending = vec![];
    set_nonblock(l.as_raw_fd());
    let t_end = Instant::now() + Duration::from_secs(o.u("wait-s", 600));
    while n + timeouts < trials && Instant::now() < t_end {
        let mut p = libc::pollfd { fd: l.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut p, 1, 500) } <= 0 {
            continue;
        }
        let Ok(c) = accept(l.as_raw_fd()) else { continue };
        let raw = c.as_raw_fd();
        set_timeouts(raw, Duration::from_secs(3));
        let mut h = [0u8; 2];
        if recv_blocking(raw, &mut h) != 2 {
            continue;
        }
        let (ai, vi) = (h[0] as usize, h[1] as usize);
        let key = format!("{}/{}", ACTIONS[ai.min(2)], VARIANTS[vi.min(1)]);
        let peer = {
            let mut a = vm_addr(0, 0);
            let mut len = 16u32;
            unsafe { libc::getpeername(raw, &mut a as *mut _ as *mut _, &mut len) };
            (a.cid, a.port)
        };
        let local = {
            let mut a = vm_addr(0, 0);
            let mut len = 16u32;
            unsafe { libc::getsockname(raw, &mut a as *mut _ as *mut _, &mut len) };
            (a.cid, a.port)
        };
        if vi == 1 {
            send_blocking(raw, &[0xAAu8; 4096]); // the peer will never read this
        }
        pending.clear();
        drain(&mut ring, &mut pending); // discard anything older than this trial
        pending.clear();
        ep.add(raw, (libc::EPOLLRDHUP | libc::EPOLLHUP | libc::EPOLLERR) as u32);
        send_blocking(raw, &[1u8]); // ACK: the peer may now act
        let evs = ep.wait(3000);
        let t_wake = mono_ns();
        let bits = evs.iter().find(|(f, _)| *f == raw).map(|(_, e)| epoll_bits(*e));
        let Some(bits) = bits else {
            timeouts += 1;
            ep.del(raw);
            continue;
        };
        let mut recs = vec![];
        drain(&mut ring, &mut recs);
        let mine = |r: &&ShutRec| r.src_port == peer.1 && r.dst_port == local.1 && (r.op == 4 || r.op == 3);
        let sigs_wake: Vec<String> = recs.iter().filter(mine).map(sig).collect();
        for r in recs.iter().filter(mine) {
            lag_ns.push(t_wake as i64 - r.t_ns as i64);
        }
        sleep_ms(60);
        let mut late = vec![];
        drain(&mut ring, &mut late);
        let sigs_late: Vec<String> = recs.iter().chain(late.iter()).filter(mine).map(sig).collect();
        let hist = |m: &mut BTreeMap<String, BTreeMap<String, u64>>, v: &Vec<String>| {
            let mut s = v.clone();
            s.sort();
            let k = if s.is_empty() { "NONE".to_string() } else { s.join("+") };
            *m.entry(key.clone()).or_default().entry(k).or_insert(0) += 1;
        };
        hist(&mut at_wake, &sigs_wake);
        hist(&mut at_late, &sigs_late);
        if sigs_wake.is_empty() {
            *missing_at_wake.entry(key.clone()).or_insert(0) += 1;
            if !sigs_late.is_empty() {
                *late_only.entry(key.clone()).or_insert(0) += 1;
            }
        }
        *pollbits.entry(key.clone()).or_default().entry(bits).or_insert(0) += 1;
        ep.del(raw);
        n += 1;
        drop::<OwnedFd>(c);
    }
    lag_ns.sort();
    let pct = |p: f64| lag_ns.get(((lag_ns.len() as f64 - 1.0) * p) as usize).copied().unwrap_or(0);
    println!(
        "{}",
        json!({"cmd":"t5-obs","uname":crate::uname(),"trials_observed":n,"timeouts":timeouts,"signature_at_rdhup_wake":at_wake,
            "signature_after_60ms":at_late,"no_record_at_rdhup_wake":missing_at_wake,"record_only_after_wake":late_only,
            "poll_bits_at_wake":pollbits,"record_to_wake_lag_ns":{"n":lag_ns.len(),"min":lag_ns.first(),"p50":pct(0.5),"p99":pct(0.99),"max":lag_ns.last()},
            "bpf_counters": crate::bpf_counters_pub(&bpf)})
    );
}
