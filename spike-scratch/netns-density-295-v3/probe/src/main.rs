//! V-3 spike probe (netns-density-295). One static binary, run on the guest and/or the host.
//!   v3probe agent                      guest command agent on vsock port 4000
//!   v3probe sink  --kind vsock|plain|tcpsrc ...   the redirect target host: accepts a vsock flow (or builds
//!                                      a local TCP source), puts it in a sockmap with a stream verdict that
//!                                      redirects (egress) into a loopback TCP socket whose peer NEVER reads
//!   v3probe send  --cid C --port P ... the continuous vsock sender
//! All measurements are JSON lines on stdout.
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aya::{
    maps::{PerCpuArray, SockHash},
    programs::{FExit, SkSkb},
    Btf, Ebpf, EbpfLoader,
};

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
fn t_ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn arg(a: &[String], k: &str) -> Option<String> {
    a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned()
}
fn argn(a: &[String], k: &str, d: u64) -> u64 {
    arg(a, k).map(|v| v.parse().expect("number")).unwrap_or(d)
}
fn uname() -> String {
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    unsafe { libc::uname(&mut u) };
    let r = unsafe { std::ffi::CStr::from_ptr(u.release.as_ptr()) };
    r.to_string_lossy().into_owned()
}
fn out(s: String) {
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(s.as_bytes());
    let _ = o.write_all(b"\n");
    let _ = o.flush();
}

// ---------------------------------------------------------------- vsock helpers
#[repr(C)]
struct SockaddrVm {
    family: u16,
    reserved: u16,
    port: u32,
    cid: u32,
    flags: u8,
    zero: [u8; 3],
}
fn vm_addr(cid: u32, port: u32) -> SockaddrVm {
    SockaddrVm { family: libc::AF_VSOCK as u16, reserved: 0, port, cid, flags: 0, zero: [0; 3] }
}
fn cvt(r: i32) -> std::io::Result<i32> {
    if r < 0 { Err(std::io::Error::last_os_error()) } else { Ok(r) }
}
fn vsock_listen(port: u32) -> std::io::Result<OwnedFd> {
    let fd = cvt(unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) })?;
    let o = unsafe { OwnedFd::from_raw_fd(fd) };
    let a = vm_addr(libc::VMADDR_CID_ANY, port);
    cvt(unsafe { libc::bind(fd, &a as *const _ as *const _, 16) })?;
    cvt(unsafe { libc::listen(fd, 128) })?;
    Ok(o)
}
fn vsock_connect(cid: u32, port: u32) -> std::io::Result<OwnedFd> {
    let fd = cvt(unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) })?;
    let o = unsafe { OwnedFd::from_raw_fd(fd) };
    let a = vm_addr(cid, port);
    cvt(unsafe { libc::connect(fd, &a as *const _ as *const _, 16) })?;
    Ok(o)
}
fn accept(fd: RawFd) -> std::io::Result<OwnedFd> {
    let n = cvt(unsafe { libc::accept4(fd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) })?;
    Ok(unsafe { OwnedFd::from_raw_fd(n) })
}
fn set_tmo(fd: RawFd, opt: i32, secs: u64) {
    let tv = libc::timeval { tv_sec: secs as i64, tv_usec: 0 };
    unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, opt, &tv as *const _ as *const _, 16) };
}
fn ioctl_i32(fd: RawFd, req: u64) -> i64 {
    let mut v: i32 = 0;
    if unsafe { libc::ioctl(fd, req as _, &mut v) } < 0 { -1 } else { v as i64 }
}
const SIOCINQ: u64 = 0x541B;
const SIOCOUTQ: u64 = 0x5411;

// ---------------------------------------------------------------- memory sampling
fn meminfo() -> BTreeMap<String, u64> {
    let mut m = BTreeMap::new();
    for l in fs::read_to_string("/proc/meminfo").unwrap_or_default().lines() {
        let mut it = l.split_whitespace();
        if let (Some(k), Some(v)) = (it.next(), it.next()) {
            m.insert(k.trim_end_matches(':').to_string(), v.parse().unwrap_or(0));
        }
    }
    m
}
fn tcp_mem_pages() -> i64 {
    for l in fs::read_to_string("/proc/net/sockstat").unwrap_or_default().lines() {
        if l.starts_with("TCP:") {
            let f: Vec<&str> = l.split_whitespace().collect();
            if let Some(i) = f.iter().position(|x| *x == "mem") {
                return f.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(-1);
            }
        }
    }
    -1
}
/// top 3 slab caches by bytes (needs root) and skbuff_head_cache active objects
fn slab_top() -> (String, u64) {
    let mut v: Vec<(String, u64)> = Vec::new();
    let mut skb = 0;
    for l in fs::read_to_string("/proc/slabinfo").unwrap_or_default().lines().skip(2) {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 15 {
            continue;
        }
        let ppslab: u64 = f[5].parse().unwrap_or(1);
        let numslabs: u64 = f[14].parse().unwrap_or(0);
        if f[0] == "skbuff_head_cache" {
            skb = f[1].parse().unwrap_or(0);
        }
        v.push((f[0].to_string(), numslabs * ppslab * 4096));
    }
    v.sort_by(|a, b| b.1.cmp(&a.1));
    let s: Vec<String> = v.iter().take(3).map(|(n, b)| format!("[\"{}\",{}]", n, b / 1048576)).collect();
    (format!("[{}]", s.join(",")), skb)
}
static CGROUP: std::sync::OnceLock<String> = std::sync::OnceLock::new();
fn cg_json() -> String {
    let Some(p) = CGROUP.get() else { return String::new() };
    let rd = |f: &str| fs::read_to_string(format!("{p}/{f}")).unwrap_or_default();
    let cur: i64 = rd("memory.current").trim().parse().unwrap_or(-1);
    let mut st: BTreeMap<String, i64> = BTreeMap::new();
    for l in rd("memory.stat").lines() {
        let mut it = l.split_whitespace();
        if let (Some(k), Some(v)) = (it.next(), it.next()) { st.insert(k.to_string(), v.parse().unwrap_or(-1)); }
    }
    let mut oomk = -1i64;
    for l in rd("memory.events").lines() { if let Some(v) = l.strip_prefix("oom_kill ") { oomk = v.trim().parse().unwrap_or(-1); } }
    let g = |k: &str| st.get(k).copied().unwrap_or(-1);
    format!(",\"cg_current\":{},\"cg_anon\":{},\"cg_kernel\":{},\"cg_slab\":{},\"cg_sock\":{},\"cg_oom_kill\":{}", cur, g("anon"), g("kernel"), g("slab"), g("sock"), oomk)
}
fn mem_json() -> String {
    let m = meminfo();
    let g = |k: &str| m.get(k).copied().unwrap_or(0);
    let (top, skb) = slab_top();
    format!(
        "\"mem_total_kb\":{},\"mem_free_kb\":{},\"mem_avail_kb\":{},\"slab_kb\":{},\"sunreclaim_kb\":{},\"sreclaim_kb\":{},\"tcp_mem_pages\":{},\"skbuff_head_active\":{},\"slab_top_mib\":{}",
        g("MemTotal"), g("MemFree"), g("MemAvailable"), g("Slab"), g("SUnreclaim"), g("SReclaimable"), tcp_mem_pages(), skb, top
    ) + &cg_json()
}

// ---------------------------------------------------------------- BPF
struct Bpf {
    e: Ebpf,
}
impl Bpf {
    fn load(obj: &str) -> Result<Bpf, String> {
        let mut e = EbpfLoader::new().load_file(obj).map_err(|x| format!("load ELF: {x:?}"))?;
        let vfd = SockHash::<_, u32>::try_from(e.map("VERD").unwrap()).unwrap().fd().try_clone().unwrap();
        let p: &mut SkSkb = e.program_mut("verdict").unwrap().try_into().unwrap();
        p.load().map_err(|x| format!("verdict load: {x:?}"))?;
        p.attach(&vfd).map_err(|x| format!("verdict attach: {x:?}"))?;
        let btf = Btf::from_sys_fs().map_err(|x| format!("btf: {x:?}"))?;
        let p: &mut FExit = e.program_mut("sent").unwrap().try_into().unwrap();
        p.load("skb_send_sock", &btf).map_err(|x| format!("fexit load: {x:?}"))?;
        p.attach().map_err(|x| format!("fexit attach: {x:?}"))?;
        Ok(Bpf { e })
    }
    fn cnt(&self) -> [u64; 7] {
        let m: PerCpuArray<_, u64> = PerCpuArray::try_from(self.e.map("CNT").unwrap()).unwrap();
        let mut r = [0u64; 7];
        for (i, slot) in r.iter_mut().enumerate() {
            if let Ok(v) = m.get(&(i as u32), 0) {
                *slot = if i == 6 { v.iter().copied().max().unwrap_or(0) } else { v.iter().sum() };
            }
        }
        r
    }
    fn insert(&mut self, map: &str, key: u32, fd: RawFd) -> Result<(), String> {
        let mut m = SockHash::<_, u32>::try_from(self.e.map_mut(map).unwrap()).unwrap();
        m.insert(key, fd, 0).map_err(|x| format!("{map} insert: {x:?}"))
    }
    fn remove(&mut self, map: &str, key: u32) {
        let mut m = SockHash::<_, u32>::try_from(self.e.map_mut(map).unwrap()).unwrap();
        let _ = m.remove(&key);
    }
}

// ---------------------------------------------------------------- sink
struct Pair {
    c: TcpStream, // redirect target (in SOCKS)
    s: TcpStream, // its peer: the application that never reads
    _l: TcpListener,
}
fn tcp_pair() -> std::io::Result<Pair> {
    let l = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
    let c = TcpStream::connect(l.local_addr()?)?;
    let (s, _) = l.accept()?;
    Ok(Pair { c, s, _l: l })
}

fn sink(a: &[String]) {
    let kind = arg(a, "--kind").unwrap_or("vsock".into());
    let label = arg(a, "--label").unwrap_or("v3".into());
    let obj = arg(a, "--obj").unwrap_or("/v3-bpf.o".into());
    let port = argn(a, "--port", 7000) as u32;
    let trials = argn(a, "--trials", 1);
    let bytes = argn(a, "--bytes", 0);
    let hold_ms = argn(a, "--hold-ms", 300);
    let sample_ms = argn(a, "--sample-ms", 250);
    let idle_s = argn(a, "--idle-s", 15);
    let max_s = argn(a, "--max-s", 900);
    let abort_mb = argn(a, "--abort-queued-mb", 0);
    let verbose = argn(a, "--samples", 1) == 1;
    let app_cap = argn(a, "--app-bytes", 1 << 30); // tcpsrc: bytes the local sender app tries to push
    let stall_s = argn(a, "--stall-s", 10);
    let valve_mb = argn(a, "--valve-mb", 0); // close the verdict valve (remove the source from VERD) at this queue size
    let valve_kick = argn(a, "--valve-kick", 0); // at re-open also recv() and DISCARD the residue left in the source rx queue (probe-only; loses those bytes)
    let valve_reopen_ms = argn(a, "--valve-reopen-ms", 0); // re-insert the source this long after closing (0 = never)
    if let Some(name) = arg(a, "--cgroup") {
        let mb = argn(a, "--cgroup-mb", 64);
        let base = "/sys/fs/cgroup";
        let p = format!("{base}/{name}");
        let r1 = fs::write(format!("{base}/cgroup.subtree_control"), "+memory");
        let r2 = fs::create_dir_all(&p);
        let r3 = fs::write(format!("{p}/memory.max"), format!("{}", mb * 1048576));
        let r4 = fs::write(format!("{p}/cgroup.procs"), format!("{}", std::process::id()));
        out(format!("{{\"ev\":\"cgroup_setup\",\"path\":\"{}\",\"max_mb\":{},\"subtree\":\"{:?}\",\"mkdir\":\"{:?}\",\"max\":\"{:?}\",\"procs\":\"{:?}\"}}", p, mb, r1.is_ok(), r2.is_ok(), r3.is_ok(), r4.is_ok()));
        let _ = CGROUP.set(p);
    }

    let mut bpf = if kind == "plain" {
        None
    } else {
        Some(Bpf::load(&obj).unwrap_or_else(|e| {
            out(format!("{{\"ev\":\"fatal\",\"err\":{:?}}}", e));
            std::process::exit(2)
        }))
    };
    let lst = if kind == "tcpsrc" { None } else { Some(vsock_listen(port).expect("vsock listen")) };
    out(format!(
        "{{\"ev\":\"sink_ready\",\"label\":\"{}\",\"kind\":\"{}\",\"uname\":\"{}\",\"port\":{},\"bytes\":{},\"abort_queued_mb\":{},{}}}",
        label, kind, uname(), port, bytes, abort_mb, mem_json()
    ));

    for trial in 0..trials {
        // ---- set up the flow
        let v: Option<OwnedFd>;
        let pair = tcp_pair().expect("tcp pair");
        let mut src_pair: Option<(TcpStream, TcpListener)> = None; // tcpsrc: (A, L2)
        let app_sent = Arc::new(AtomicU64::new(0));
        let app_state = Arc::new(AtomicU64::new(0)); // 0 running, 1 done, 2 stalled, 3 error
        let app_thread;
        if kind == "tcpsrc" {
            let l2 = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
            let ap = TcpStream::connect(l2.local_addr().unwrap()).unwrap();
            let (src, _) = l2.accept().unwrap();
            src_pair = Some((src, l2));
            v = None;
            let (sent, st) = (app_sent.clone(), app_state.clone());
            set_tmo(ap.as_raw_fd(), libc::SO_SNDTIMEO, stall_s);
            app_thread = Some(std::thread::spawn(move || {
                let mut ap = ap;
                let buf = vec![0x5au8; 65536];
                while sent.load(Ordering::Relaxed) < app_cap {
                    match ap.write(&buf) {
                        Ok(n) => {
                            sent.fetch_add(n as u64, Ordering::Relaxed);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                            st.store(2, Ordering::Relaxed);
                            std::thread::sleep(Duration::from_secs(3600));
                            return;
                        }
                        Err(_) => {
                            st.store(3, Ordering::Relaxed);
                            return;
                        }
                    }
                }
                st.store(1, Ordering::Relaxed);
                std::thread::sleep(Duration::from_secs(3600));
            }));
        } else {
            app_thread = None;
            let l = lst.as_ref().unwrap();
            v = Some(accept(l.as_raw_fd()).expect("accept"));
        }
        let (src_fd, src_name): (RawFd, &str) = match (&v, &src_pair) {
            (Some(v), _) => (v.as_raw_fd(), "vsock"),
            (None, Some((s, _))) => (s.as_raw_fd(), "tcp"),
            _ => unreachable!(),
        };
        let base = bpf.as_ref().map(|b| b.cnt()).unwrap_or([0; 7]);
        let mem0 = meminfo();
        let avail0 = mem0.get("MemAvailable").copied().unwrap_or(0);
        let sun0 = mem0.get("SUnreclaim").copied().unwrap_or(0);
        if let Some(b) = bpf.as_mut() {
            b.insert("SOCKS", 1, pair.c.as_raw_fd()).expect("insert target");
            b.insert("VERD", 0, src_fd).expect("insert source");
        }
        if let Some(v) = &v {
            let r = unsafe { libc::write(v.as_raw_fd(), b"G".as_ptr() as *const _, 1) };
            if r != 1 {
                out("{\"ev\":\"warn\",\"msg\":\"G write failed\"}".to_string());
            }
        }
        let t0 = Instant::now();
        let (mut last_prog, mut last_change) = (0u64, Instant::now());
        let (mut peak_q, mut peak_q_t) = (0u64, 0u64);
        let mut outcome = String::new();
        let mut done_at: Option<Instant> = None;
        let mut seen_data = false;
        let mut n_samples = 0u64;
        let mut last_sample = [0u64; 7];
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        let mut valve_closed: Option<Instant> = None;
        let mut valve_reopened = false;
        loop {
            std::thread::sleep(Duration::from_millis(if valve_mb > 0 { 1 } else { sample_ms }));
            let c = bpf.as_ref().map(|b| b.cnt()).unwrap_or([0; 7]);
            let d = |i: usize| c[i].wrapping_sub(base[i]);
            let (fwd, sent_b) = (d(0), d(3));
            let inq_src = ioctl_i32(src_fd, SIOCINQ);
            let progress = if kind == "plain" { inq_src.max(0) as u64 } else { fwd };
            let queued = fwd.saturating_sub(sent_b);
            if queued > peak_q {
                peak_q = queued;
                peak_q_t = t0.elapsed().as_millis() as u64;
            }
            if progress != last_prog {
                last_prog = progress;
                last_change = Instant::now();
                if progress > 0 {
                    seen_data = true;
                }
            }
            n_samples += 1;
            last_sample = c;
            if let (Some(tc), false) = (valve_closed, valve_reopened) {
                if valve_reopen_ms > 0 && tc.elapsed() >= Duration::from_millis(valve_reopen_ms) {
                    let r = bpf.as_mut().unwrap().insert("VERD", 0, src_fd);
                    valve_reopened = true;
                    if valve_kick == 1 {
                        let mut tmp = vec![0u8; 65536];
                        let mut got = 0i64;
                        loop {
                            let n = unsafe { libc::recv(src_fd, tmp.as_mut_ptr() as *mut _, tmp.len(), libc::MSG_DONTWAIT) };
                            if n <= 0 { break; }
                            got += n as i64;
                        }
                        out(format!("{{\"ev\":\"valve_kick\",\"label\":\"{}\",\"discarded\":{}}}", label, got));
                    }
                    out(format!("{{\"ev\":\"valve_reopened\",\"label\":\"{}\",\"t_ms\":{},\"fwd\":{},\"sent\":{},\"queued\":{},\"src_inq\":{},\"insert_ok\":{}}}", label, t0.elapsed().as_millis(), fwd, sent_b, queued, inq_src, r.is_ok()));
                }
            }
            if valve_mb > 0 && valve_closed.is_none() && queued >= valve_mb * 1048576 {
                bpf.as_mut().unwrap().remove("VERD", 0);
                valve_closed = Some(Instant::now());
                out(format!("{{\"ev\":\"valve_closed\",\"label\":\"{}\",\"t_ms\":{},\"fwd\":{},\"sent\":{},\"queued\":{},\"src_inq\":{}}}", label, t0.elapsed().as_millis(), fwd, sent_b, queued, inq_src));
            }
            if verbose && last_emit.elapsed() >= Duration::from_millis(sample_ms) {
                last_emit = Instant::now();
                out(format!(
                    "{{\"ev\":\"sample\",\"label\":\"{}\",\"trial\":{},\"t_ms\":{},\"fwd\":{},\"sent\":{},\"queued\":{},\"verdicts\":{},\"drops\":{},\"send_calls\":{},\"send_fail\":{},\"last_errno\":{},\"src_inq\":{},\"tgt_outq\":{},\"peer_inq\":{},\"app_sent\":{},\"app_state\":{},{}}}",
                    label, trial, t0.elapsed().as_millis(), fwd, sent_b, queued, d(1), d(2), d(4), d(5), c[6], inq_src,
                    ioctl_i32(pair.c.as_raw_fd(), SIOCOUTQ), ioctl_i32(pair.s.as_raw_fd(), SIOCINQ),
                    app_sent.load(Ordering::Relaxed), app_state.load(Ordering::Relaxed), mem_json()
                ));
            }
            if let Some(dn) = done_at {
                if dn.elapsed() >= Duration::from_millis(hold_ms) {
                    break;
                }
                continue;
            }
            if abort_mb > 0 && queued >= abort_mb * 1048576 {
                outcome = "abort_cap".into();
                break;
            }
            if bytes > 0 && fwd >= bytes {
                outcome = "complete".into();
                done_at = Some(Instant::now());
                continue;
            }
            if seen_data && last_change.elapsed() >= Duration::from_secs(idle_s) {
                outcome = "plateau_idle".into();
                break;
            }
            if !seen_data && t0.elapsed() >= Duration::from_secs(30) {
                outcome = "no_data".into();
                break;
            }
            if t0.elapsed() >= Duration::from_secs(max_s) {
                outcome = "max_s".into();
                break;
            }
        }
        let c = bpf.as_ref().map(|b| b.cnt()).unwrap_or(last_sample);
        let d = |i: usize| c[i].wrapping_sub(base[i]);
        let mem1 = meminfo();
        let (avail1, sun1) = (mem1.get("MemAvailable").copied().unwrap_or(0), mem1.get("SUnreclaim").copied().unwrap_or(0));
        let queued_end = d(0).saturating_sub(d(3));
        let app_s = app_sent.load(Ordering::Relaxed);
        let app_st = app_state.load(Ordering::Relaxed);
        // ---- tear down: remove from maps, close everything
        if let Some(b) = bpf.as_mut() {
            b.remove("VERD", 0);
            b.remove("SOCKS", 1);
        }
        drop(v);
        drop(src_pair);
        drop(pair);
        let _ = app_thread; // the app thread parks; its socket is closed above
        std::thread::sleep(Duration::from_millis(1500));
        let mem2 = meminfo();
        let (avail2, sun2) = (mem2.get("MemAvailable").copied().unwrap_or(0), mem2.get("SUnreclaim").copied().unwrap_or(0));
        out(format!(
            "{{\"ev\":\"trial_end\",\"label\":\"{}\",\"kind\":\"{}\",\"src\":\"{}\",\"trial\":{},\"outcome\":\"{}\",\"secs\":{:.2},\"samples\":{},\"fwd\":{},\"sent\":{},\"queued_end\":{},\"peak_queued\":{},\"peak_queued_at_ms\":{},\"verdicts\":{},\"drops\":{},\"send_calls\":{},\"send_fail\":{},\"app_sent\":{},\"app_state\":{},\"avail_start_kb\":{},\"avail_end_kb\":{},\"avail_after_close_kb\":{},\"sunreclaim_start_kb\":{},\"sunreclaim_end_kb\":{},\"sunreclaim_after_close_kb\":{}}}",
            label, kind, src_name, trial, outcome, t0.elapsed().as_secs_f64(), n_samples, d(0), d(3), queued_end, peak_q, peak_q_t, d(1), d(2), d(4), d(5),
            app_s, app_st, avail0, avail1, avail2, sun0, sun1, sun2
        ));
    }
    out(format!("{{\"ev\":\"sink_done\",\"label\":\"{}\",\"t_ms\":{}}}", label, t_ms()));
}

// ---------------------------------------------------------------- sender
fn send(a: &[String]) {
    let cid = argn(a, "--cid", 2) as u32;
    let port = argn(a, "--port", 7000) as u32;
    let bytes = argn(a, "--bytes", 0); // 0 = until cap
    let cap = argn(a, "--cap-bytes", 1 << 32);
    let stall_s = argn(a, "--stall-s", 10);
    let trials = argn(a, "--trials", 1);
    let label = arg(a, "--label").unwrap_or("v3".into());
    let verbose = argn(a, "--verbose", 1) == 1;
    let want_g = argn(a, "--wait-g", 1) == 1;
    let total = if bytes > 0 { bytes } else { cap };
    out(format!(
        "{{\"ev\":\"send_ready\",\"label\":\"{}\",\"uname\":\"{}\",\"cid\":{},\"port\":{},\"total\":{},\"trials\":{}}}",
        label, uname(), cid, port, total, trials
    ));
    let buf = vec![0xa5u8; 65536];
    for trial in 0..trials {
        let mut fd = None;
        for _ in 0..200 {
            match vsock_connect(cid, port) {
                Ok(f) => {
                    fd = Some(f);
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        let Some(fd) = fd else {
            out(format!("{{\"ev\":\"send_end\",\"label\":\"{}\",\"trial\":{},\"outcome\":\"connect_fail\"}}", label, trial));
            continue;
        };
        let raw = fd.as_raw_fd();
        if want_g {
            set_tmo(raw, libc::SO_RCVTIMEO, 20);
            let mut g = [0u8; 1];
            let n = unsafe { libc::read(raw, g.as_mut_ptr() as *mut _, 1) };
            if n != 1 {
                out(format!("{{\"ev\":\"send_end\",\"label\":\"{}\",\"trial\":{},\"outcome\":\"no_go\"}}", label, trial));
                continue;
            }
        }
        set_tmo(raw, libc::SO_SNDTIMEO, stall_s);
        let t0 = Instant::now();
        let (mut sent, mut max_send_ms, mut slow_sends, mut sends) = (0u64, 0u128, 0u64, 0u64);
        let mut last_prog = Instant::now();
        let mut outcome = "done".to_string();
        while sent < total {
            let n = std::cmp::min(buf.len() as u64, total - sent) as usize;
            let ts = Instant::now();
            let r = unsafe { libc::send(raw, buf.as_ptr() as *const _, n, libc::MSG_NOSIGNAL) };
            let el = ts.elapsed().as_millis();
            sends += 1;
            if el > max_send_ms {
                max_send_ms = el;
            }
            if el >= 100 {
                slow_sends += 1;
            }
            if r < 0 {
                let e = std::io::Error::last_os_error();
                outcome = if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut {
                    format!("stall_{}s", stall_s)
                } else {
                    format!("err_{}", e.raw_os_error().unwrap_or(0))
                };
                break;
            }
            sent += r as u64;
            if verbose && last_prog.elapsed() >= Duration::from_millis(500) {
                last_prog = Instant::now();
                out(format!(
                    "{{\"ev\":\"send_prog\",\"label\":\"{}\",\"trial\":{},\"t_ms\":{},\"sent\":{},\"max_send_ms\":{},\"slow_sends\":{}}}",
                    label, trial, t0.elapsed().as_millis(), sent, max_send_ms, slow_sends
                ));
            }
        }
        let secs = t0.elapsed().as_secs_f64();
        // wait for the sink to close the flow
        set_tmo(raw, libc::SO_RCVTIMEO, 30);
        let mut b = [0u8; 16];
        let rr = unsafe { libc::read(raw, b.as_mut_ptr() as *mut _, 16) };
        out(format!(
            "{{\"ev\":\"send_end\",\"label\":\"{}\",\"trial\":{},\"outcome\":\"{}\",\"sent\":{},\"secs\":{:.3},\"mbps\":{:.1},\"max_send_ms\":{},\"slow_sends\":{},\"sends\":{},\"peer_closed_read\":{}}}",
            label, trial, outcome, sent, secs, sent as f64 / secs.max(1e-6) / 1048576.0, max_send_ms, slow_sends, sends, rr
        ));
        drop(fd);
    }
}

// ---------------------------------------------------------------- agent (guest command channel)
fn agent() {
    let l = vsock_listen(4000).expect("agent listen");
    out(format!("{{\"ev\":\"agent_ready\",\"uname\":\"{}\"}}", uname()));
    loop {
        let Ok(c) = accept(l.as_raw_fd()) else { continue };
        std::thread::spawn(move || {
            let mut f = std::fs::File::from(c);
            let mut len = [0u8; 4];
            if f.read_exact(&mut len).is_err() {
                return;
            }
            let mut cmd = vec![0u8; u32::from_be_bytes(len) as usize];
            if f.read_exact(&mut cmd).is_err() {
                return;
            }
            let o = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(String::from_utf8_lossy(&cmd).into_owned())
                .stdin(std::process::Stdio::null())
                .output();
            match o {
                Ok(o) => {
                    let _ = f.write_all(&o.stdout);
                    let _ = f.write_all(&o.stderr);
                    let _ = f.write_all(format!("\nAGENT_RC={}\n", o.status.code().unwrap_or(-1)).as_bytes());
                }
                Err(e) => {
                    let _ = f.write_all(format!("\nspawn failed {e}\nAGENT_RC=-1\n").as_bytes());
                }
            }
        });
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let _ = START.get_or_init(Instant::now);
    match a.get(1).map(|s| s.as_str()) {
        Some("agent") => agent(),
        Some("sink") => sink(&a[2..]),
        Some("send") => send(&a[2..]),
        _ => {
            eprintln!("usage: v3probe agent|sink|send");
            std::process::exit(1)
        }
    }
}
