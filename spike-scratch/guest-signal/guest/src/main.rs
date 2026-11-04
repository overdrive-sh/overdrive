//! guest-signal spike guest: imitates overdrive-init's ordering (hooks attached BEFORE READY; READY; one EXEC;
//! exec workload; LISTEN reports; EXIT) on the beacon vsock, plus the V36 stress/latency scenarios.
//! PID 1 (exec'd by /init after mounts). Cmdline: gs.port=<host vsock port> gs.drain=<0|1>
#[path = "../../common.rs"]
mod common;

use aya::{
    include_bytes_aligned,
    maps::RingBuf,
    programs::{cgroup_sock::CgroupSock, sock_ops::SockOps, CgroupAttachMode},
    Ebpf,
};
use std::collections::BTreeSet;
use std::io::Read;
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const BPF: &[u8] = include_bytes_aligned!("../../out/bpf-target/bpfel-unknown-none/release/gs-bpf");

static PARTIAL_WRITES: AtomicU64 = AtomicU64::new(0);
static T_EXEC: AtomicU64 = AtomicU64::new(0);
static PRE_EXEC: AtomicU64 = AtomicU64::new(0);
static EVENTS: AtomicU64 = AtomicU64::new(0);

fn mono_ns() -> u64 {
    let mut ts: libc::timespec = unsafe { mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

fn cmdline(key: &str) -> Option<String> {
    let c = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    c.split_whitespace().find_map(|t| t.strip_prefix(&format!("{key}=")).map(str::to_string))
}

fn vsock_connect(cid: u32, port: u32) -> std::io::Result<OwnedFd> {
    let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut a: libc::sockaddr_vm = unsafe { mem::zeroed() };
    a.svm_family = libc::AF_VSOCK as _;
    a.svm_cid = cid;
    a.svm_port = port;
    let r = unsafe { libc::connect(fd, &a as *const _ as *const _, mem::size_of::<libc::sockaddr_vm>() as u32) };
    if r < 0 {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// One write(2) per line, no user-space lock: the kernel's per-sendmsg atomicity is what is under test.
fn write_line(fd: RawFd, line: &str) {
    let b = line.as_bytes();
    let mut off = 0;
    let mut first = true;
    while off < b.len() {
        let n = unsafe { libc::write(fd, b[off..].as_ptr().cast(), b.len() - off) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            println!("GS_WRITE_ERR {e}");
            return;
        }
        off += n as usize;
        if off < b.len() && first {
            PARTIAL_WRITES.fetch_add(1, Ordering::Relaxed);
        }
        first = false;
    }
}

struct LineReader {
    fd: RawFd,
    buf: Vec<u8>,
}
impl LineReader {
    fn read_line(&mut self) -> Option<String> {
        loop {
            if let Some(p) = self.buf.iter().position(|b| *b == b'\n') {
                let l: Vec<u8> = self.buf.drain(..=p).collect();
                return Some(String::from_utf8_lossy(&l[..l.len() - 1]).into_owned());
            }
            let mut tmp = [0u8; 4096];
            let n = unsafe { libc::read(self.fd, tmp.as_mut_ptr().cast(), tmp.len()) };
            if n <= 0 {
                println!("GS_READ_END n={n} err={}", std::io::Error::last_os_error());
                return None;
            }
            self.buf.extend_from_slice(&tmp[..n as usize]);
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Event {
    kind: u32,
    port: u32,
    ktime_ns: u64,
}

struct Hooks {
    _bpf: Ebpf,
    ring: RingBuf<aya::maps::MapData>,
}

fn attach_hooks() -> Hooks {
    let t = Instant::now();
    let mut bpf = Ebpf::load(BPF).expect("load bpf");
    let cg = std::fs::File::open("/sys/fs/cgroup").expect("open cgroup2 root");
    let p: &mut SockOps = bpf.program_mut("listen_cb").unwrap().try_into().unwrap();
    p.load().expect("load listen_cb");
    p.attach(&cg, CgroupAttachMode::Single).expect("attach listen_cb");
    for name in ["post_bind4", "post_bind6"] {
        let p: &mut CgroupSock = bpf.program_mut(name).unwrap().try_into().unwrap();
        p.load().unwrap_or_else(|e| panic!("load {name}: {e}"));
        p.attach(&cg, CgroupAttachMode::Single).unwrap_or_else(|e| panic!("attach {name}: {e}"));
    }
    let ring = RingBuf::try_from(bpf.take_map("EVENTS").expect("EVENTS map")).unwrap();
    println!("GS_HOOKS_ATTACHED ms={:.2}", t.elapsed().as_secs_f64() * 1e3);
    Hooks { _bpf: bpf, ring }
}

/// Drain every queued event; returns the events (not deduped).
fn drain(ring: &mut RingBuf<aya::maps::MapData>) -> Vec<Event> {
    let mut v = Vec::new();
    while let Some(item) = ring.next() {
        let b: &[u8] = &item;
        if b.len() >= mem::size_of::<Event>() {
            v.push(unsafe { std::ptr::read_unaligned(b.as_ptr().cast::<Event>()) });
        }
    }
    v
}

fn send_events(fd: RawFd, seen: &mut BTreeSet<(u32, u32)>, evs: &[Event]) {
    for e in evs {
        EVENTS.fetch_add(1, Ordering::Relaxed);
        if e.ktime_ns < T_EXEC.load(Ordering::Relaxed) {
            PRE_EXEC.fetch_add(1, Ordering::Relaxed);
        }
        println!("GS_EVT kind={} port={} ktime_ns={} now_ns={}", e.kind, e.port, e.ktime_ns, mono_ns());
        if seen.insert((e.kind, e.port)) {
            let proto = if e.kind == 1 { "tcp" } else { "udp" };
            write_line(fd, &format!("LISTEN {proto} {}\n", e.port));
        }
    }
}

/// Half-close and wait for the host to close, so no tail data is lost to an early close().
fn finish(raw: RawFd, rd: &mut LineReader) {
    unsafe { libc::shutdown(raw, libc::SHUT_WR) };
    while rd.read_line().is_some() {}
}

fn session(hooks: &mut Option<Hooks>, host_port: u32, drain_before_exit: bool, sess_no: u64) -> bool {
    let fd = loop {
        match vsock_connect(2, host_port) {
            Ok(fd) => break fd,
            Err(e) => {
                println!("GS_CONNECT_RETRY {e}");
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    };
    let raw = fd.as_raw_fd();
    let mut rd = LineReader { fd: raw, buf: Vec::new() };
    // READY after hooks are attached (caller guarantees ordering).
    write_line(raw, &format!("READY pid=1 port={host_port} launch={}\n", "5a".repeat(16)));
    let exec = match rd.read_line() {
        Some(l) => l,
        None => return false,
    };
    let t_exec = mono_ns();
    T_EXEC.store(t_exec, Ordering::Relaxed);
    let argv: Vec<String> = match exec.strip_prefix("EXEC ").map(|j| parse_json_argv(j)) {
        Some(a) => a,
        None => {
            println!("GS_BAD_EXEC {exec}");
            return false;
        }
    };
    println!("GS_EXEC_RX mono_ns={t_exec} argv={argv:?}");
    match argv[0].as_str() {
        "quit" => return false,
        "wl" => {
            if cmdline("gs.late").as_deref() == Some("1") {
                // NEGATIVE CONTROL: workload runs first, hooks attached only afterwards (the wrong order).
                let mut child = std::process::Command::new(&argv[1]).args(&argv[2..]).spawn().expect("spawn workload");
                let code = child.wait().expect("wait").code().unwrap_or(-1);
                *hooks = Some(attach_hooks());
                let evs = drain(&mut hooks.as_mut().unwrap().ring);
                send_events(raw, &mut BTreeSet::new(), &evs);
                write_line(raw, &format!("EXIT {code}\n"));
                finish(raw, &mut rd);
                return true;
            }
            let hk = hooks.as_mut().expect("hooks");
            let stop = Arc::new(AtomicBool::new(false));
            // Forwarder: poll the ring buffer (init would epoll its fd); a thread shares the fd with the main thread.
            let ring_ptr = &mut hk.ring as *mut RingBuf<aya::maps::MapData> as usize;
            let seen_m = Arc::new(Mutex::new(BTreeSet::<(u32, u32)>::new()));
            let (stop2, seen2) = (stop.clone(), seen_m.clone());
            let mut fwd = Some(std::thread::spawn(move || {
                let ring = unsafe { &mut *(ring_ptr as *mut RingBuf<aya::maps::MapData>) };
                let rfd = ring.as_raw_fd();
                while !stop2.load(Ordering::Acquire) {
                    let mut p = libc::pollfd { fd: rfd, events: libc::POLLIN, revents: 0 };
                    unsafe { libc::poll(&mut p, 1, 5) };
                    let evs = drain(ring);
                    send_events(raw, &mut seen2.lock().unwrap(), &evs);
                }
            }));
            let mut child = std::process::Command::new(&argv[1]).args(&argv[2..]).spawn().expect("spawn workload");
            let status = child.wait().expect("wait");
            let code = status.code().unwrap_or(-1);
            if drain_before_exit {
                // D13-recommended: the exit path drains the ring after waitpid, before EXIT.
                // Stop the forwarder first so exactly one thread owns the ring, then drain + send.
                stop.store(true, Ordering::Release);
                fwd.take().unwrap().join().unwrap();
                let ring = &mut hooks.as_mut().unwrap().ring;
                let evs = drain(ring);
                let mut s = seen_m.lock().unwrap();
                send_events(raw, &mut s, &evs);
                write_line(raw, &format!("NOTE preexec={} events={}\n", PRE_EXEC.load(Ordering::Relaxed), EVENTS.load(Ordering::Relaxed)));
            }
            // else naive: EXIT immediately; the forwarder races it.
            write_line(raw, &format!("EXIT {code}\n"));
            if !drain_before_exit {
                std::thread::sleep(std::time::Duration::from_millis(150));
                stop.store(true, Ordering::Release);
                fwd.take().unwrap().join().unwrap();
            }
            finish(raw, &mut rd);
            return true;
        }
        "lat" => {
            // lat <k> <base_idx> <exit_after> : k closed-loop LISTENs; ONE EXIT written by a second thread
            // once <exit_after> LISTENs have been started (so it lands concurrently with the stream).
            let k: u64 = argv[1].parse().unwrap();
            let base: u64 = argv[2].parse().unwrap();
            let exit_after: u64 = argv[3].parse().unwrap();
            let sending = Arc::new(AtomicU64::new(0));
            let s2 = sending.clone();
            let exit_thread = std::thread::spawn(move || {
                while s2.load(Ordering::Acquire) < exit_after {
                    std::hint::spin_loop();
                }
                write_line(raw, &format!("EXIT {sess_no}\n"));
            });
            let mut rtts = Vec::with_capacity(k as usize);
            let mut bad_ack = 0u64;
            for i in 0..k {
                sending.store(i + 1, Ordering::Release);
                let line = common::listen_line(base + i);
                let t0 = mono_ns();
                write_line(raw, &line);
                let ack = rd.read_line();
                let t1 = mono_ns();
                match ack.as_deref().and_then(|a| a.strip_prefix("ACK ")) {
                    Some(rest) if rest.trim() == (base + i).to_string() => rtts.push(t1 - t0),
                    _ => bad_ack += 1,
                }
            }
            exit_thread.join().unwrap();
            for (ci, chunk) in rtts.chunks(200).enumerate() {
                let list: Vec<String> = chunk.iter().map(|r| r.to_string()).collect();
                write_line(raw, &format!("RTTS {} {} {}\n", sess_no, if ci == 0 { bad_ack } else { 0 }, list.join(",")));
            }
            finish(raw, &mut rd);
            return true;
        }
        "sat" => {
            let n: u64 = argv[1].parse().unwrap();
            let locked = argv.get(2).map(String::as_str) == Some("lock");
            static WRITER: Mutex<()> = Mutex::new(());
            let w = move |line: &str| {
                let _g = if locked { Some(WRITER.lock().unwrap()) } else { None };
                write_line(raw, line);
            };
            let t = std::thread::spawn(move || {
                for i in 0..n {
                    w(&common::listen_line(i));
                }
            });
            let e = std::thread::spawn(move || {
                for j in 0..n {
                    w(&format!("EXIT {j}\n"));
                }
            });
            t.join().unwrap();
            e.join().unwrap();
            write_line(raw, &format!("STATS {{\"sat_done\":{n},\"partial_writes\":{}}}\n", PARTIAL_WRITES.load(Ordering::Relaxed)));
            finish(raw, &mut rd);
            return true;
        }
        other => {
            println!("GS_UNKNOWN_EXEC {other}");
            return false;
        }
    }
}

fn parse_json_argv(j: &str) -> Vec<String> {
    // minimal parser for ["a","b",...] without escapes (host side only emits plain tokens)
    j.trim().trim_start_matches('[').trim_end_matches(']').split(',').map(|s| s.trim().trim_matches('"').to_string()).collect()
}

fn main() {
    println!("GS_BOOT uname_r={}", std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim());
    let port: u32 = cmdline("gs.port").and_then(|p| p.parse().ok()).expect("gs.port");
    let drain_before_exit = cmdline("gs.drain").as_deref() != Some("0");
    // 1. hooks attached BEFORE READY (the ordering under test)
    let mut hooks = if cmdline("gs.late").as_deref() == Some("1") { None } else { Some(attach_hooks()) };
    let mut n = 0u64;
    // 2. sessions: READY -> EXEC -> work -> EXIT
    let once = cmdline("gs.once").as_deref() == Some("1");
    while session(&mut hooks, port, drain_before_exit, n) {
        n += 1;
        if once {
            break;
        }
    }
    println!("GS_DONE sessions={n}");
    std::thread::sleep(std::time::Duration::from_millis(300));
    unsafe {
        libc::sync();
        libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF);
    }
    let mut s = String::new();
    let _ = std::io::stdin().read_to_string(&mut s);
}
