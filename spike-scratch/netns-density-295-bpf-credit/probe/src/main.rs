//! V-VALVE spike probe (netns-density-295). One static binary, run on the guest and/or the host.
//!   valveprobe agent                     guest command agent on vsock port 4000
//!   valveprobe sink  --kind vsock|tcpsrc ...  the owner: accepts a vsock flow (or builds a local TCP source),
//!                                        puts it in a sockmap with a stream verdict that redirects (egress) into
//!                                        a loopback TCP socket whose peer is "the target app" (a reader thread that
//!                                        follows a stall / slow / fast schedule and verifies every byte)
//!   valveprobe send  --cid C --port P ... the pattern sender (vsock)
//! Every byte stream is the splitmix64 sequence word k = splitmix(k) (little endian). The reader compares every
//! 8-byte word with the independently generated expected word and keeps an order-sensitive hash of both.
//! All measurements are JSON lines on stdout.
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aya::{
    maps::{Array, MapData, RingBuf, SockHash},
    programs::{FExit, KProbe, SkSkb},
    Btf, Ebpf, EbpfLoader,
};

// ---------------------------------------------------------------- small helpers
fn arg(a: &[String], k: &str) -> Option<String> {
    a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned()
}
fn argn(a: &[String], k: &str, d: u64) -> u64 {
    arg(a, k).map(|v| v.parse().expect("number")).unwrap_or(d)
}
fn uname() -> String {
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    unsafe { libc::uname(&mut u) };
    unsafe { std::ffi::CStr::from_ptr(u.release.as_ptr()) }.to_string_lossy().into_owned()
}
fn out(s: String) {
    // one write(2) per line so concurrent writers on the serial console cannot split it
    let mut line = s.into_bytes();
    line.push(b'\n');
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(&line);
    let _ = o.flush();
}
fn mono_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}
fn splitmix(k: u64) -> u64 {
    let mut z = k.wrapping_add(0x9E37_79B9_7F4A_7C15).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
const HK: u64 = 0x9E37_79B9_7F4A_7C15;

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
fn set_vsock_buf(fd: RawFd, bytes: u64) {
    // SOL_VSOCK = 40, SO_VM_SOCKETS_BUFFER_SIZE = 0, SO_VM_SOCKETS_BUFFER_MAX_SIZE = 2
    unsafe {
        libc::setsockopt(fd, 40, 2, &bytes as *const _ as *const _, 8);
        libc::setsockopt(fd, 40, 0, &bytes as *const _ as *const _, 8);
    }
}
fn vsock_listen(port: u32, buf: u64) -> std::io::Result<OwnedFd> {
    let fd = cvt(unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) })?;
    let o = unsafe { OwnedFd::from_raw_fd(fd) };
    if buf > 0 { set_vsock_buf(fd, buf); }
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
fn set_tmo(fd: RawFd, opt: i32, ms: u64) {
    let tv = libc::timeval { tv_sec: (ms / 1000) as i64, tv_usec: ((ms % 1000) * 1000) as i64 };
    unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, opt, &tv as *const _ as *const _, 16) };
}
fn setsockopt_i32(fd: RawFd, lvl: i32, opt: i32, v: i32) -> i32 {
    unsafe { libc::setsockopt(fd, lvl, opt, &v as *const _ as *const _, 4) }
}
fn ioctl_i32(fd: RawFd, req: u64) -> i64 {
    let mut v: i32 = 0;
    if unsafe { libc::ioctl(fd, req as _, &mut v) } < 0 { -1 } else { v as i64 }
}
const SIOCINQ: u64 = 0x541B;
const SIOCOUTQ: u64 = 0x5411;
fn read_fd(fd: RawFd, buf: &mut [u8]) -> isize {
    unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) }
}
fn write_fd(fd: RawFd, buf: &[u8]) -> isize {
    unsafe { libc::write(fd, buf.as_ptr() as *const _, buf.len()) }
}
fn poll1(fd: RawFd, ev: i16, ms: i32) -> bool {
    let mut p = libc::pollfd { fd, events: ev, revents: 0 };
    unsafe { libc::poll(&mut p, 1, ms) > 0 }
}

// ---------------------------------------------------------------- memory sampling
fn cpu_busy_jiffies() -> (u64, u64) {
    // (busy, total) jiffies across all cpus from /proc/stat
    let t = fs::read_to_string("/proc/stat").unwrap_or_default();
    let l = t.lines().next().unwrap_or("");
    let v: Vec<u64> = l.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let total: u64 = v.iter().take(8).sum();
    let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
    (total - idle, total)
}
fn thread_cpu_ms() -> u64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut ru) };
    (ru.ru_utime.tv_sec as u64 + ru.ru_stime.tv_sec as u64) * 1000 + (ru.ru_utime.tv_usec as u64 + ru.ru_stime.tv_usec as u64) / 1000
}
fn meminfo_kb(keys: &[&str]) -> Vec<u64> {
    let t = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    keys.iter()
        .map(|k| {
            t.lines()
                .find(|l| l.starts_with(k))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        })
        .collect()
}
fn skb_active() -> u64 {
    for l in fs::read_to_string("/proc/slabinfo").unwrap_or_default().lines().skip(2) {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() > 2 && f[0] == "skbuff_head_cache" {
            return f[1].parse().unwrap_or(0);
        }
    }
    0
}

// ---------------------------------------------------------------- pattern verify + generate
struct Verifier {
    k: u64,
    carry: [u8; 8],
    clen: usize,
    hash: u64,
    ehash: u64,
    first_bad: Option<u64>,
}
impl Verifier {
    fn new() -> Self {
        Verifier { k: 0, carry: [0; 8], clen: 0, hash: 0, ehash: 0, first_bad: None }
    }
    #[inline(always)]
    fn word(&mut self, w: u64) {
        let e = splitmix(self.k);
        if w != e && self.first_bad.is_none() {
            self.first_bad = Some(self.k * 8);
        }
        self.hash = (self.hash.rotate_left(5) ^ w).wrapping_mul(HK);
        self.ehash = (self.ehash.rotate_left(5) ^ e).wrapping_mul(HK);
        self.k += 1;
    }
    fn feed(&mut self, mut d: &[u8]) {
        while self.clen > 0 && self.clen < 8 && !d.is_empty() {
            self.carry[self.clen] = d[0];
            self.clen += 1;
            d = &d[1..];
        }
        if self.clen == 8 {
            let w = u64::from_le_bytes(self.carry);
            self.word(w);
            self.clen = 0;
        }
        if self.clen > 0 {
            return;
        }
        let mut ch = d.chunks_exact(8);
        for c in &mut ch {
            self.word(u64::from_le_bytes(c.try_into().unwrap()));
        }
        let r = ch.remainder();
        self.carry[..r.len()].copy_from_slice(r);
        self.clen = r.len();
    }
}
fn gen_fill(buf: &mut [u8], first_word: u64) {
    for (i, c) in buf.chunks_exact_mut(8).enumerate() {
        c.copy_from_slice(&splitmix(first_word + i as u64).to_le_bytes());
    }
}

// ---------------------------------------------------------------- the target app (reader)
#[derive(Clone, Debug)]
enum Phase {
    Stall(u64),
    Rate(u64, Option<u64>), // MiB/s, optional duration ms
    Fast(Option<u64>),
}
fn parse_phases(s: &str) -> Vec<Phase> {
    s.split(',')
        .map(|p| {
            let (head, dur) = match p.split_once(':') {
                Some((h, d)) => (h, Some(d.to_string())),
                None => (p, None),
            };
            let dms = dur.as_ref().and_then(|d| d.parse::<u64>().ok());
            match head.chars().next() {
                Some('s') => Phase::Stall(head[1..].parse().expect("stall ms")),
                Some('r') => Phase::Rate(head[1..].parse().expect("rate MiB/s"), dms),
                Some('f') => Phase::Fast(dms),
                _ => panic!("bad phase {p}"),
            }
        })
        .collect()
}

#[derive(Default)]
struct RStats {
    read: AtomicU64,
    done: AtomicBool,
    abort: AtomicBool,
    first_byte_ns: AtomicU64,
    done_ns: AtomicU64,
    res: Mutex<Option<RRes>>,
    stall_ends: Mutex<Vec<(u64, u64)>>, // (mono ns at end of a stall phase, bytes read at that time)
}
struct RRes {
    ok: bool,
    first_bad: Option<u64>,
    hash: u64,
    ehash: u64,
    bytes: u64,
}

fn reader(s: TcpStream, total: u64, phases: Vec<Phase>, st: Arc<RStats>, verify: bool) {
    let fd = s.as_raw_fd();
    set_tmo(fd, libc::SO_RCVTIMEO, 50);
    let mut v = Verifier::new();
    let mut buf = vec![0u8; 65536];
    let mut got = 0u64;
    let mut idx = 0usize;
    let mut ph_start = Instant::now();
    let mut ph_bytes = 0u64;
    'outer: while got < total && !st.abort.load(Ordering::Relaxed) {
        let ph = phases.get(idx).cloned().unwrap_or(Phase::Fast(None));
        let dur = match &ph {
            Phase::Stall(ms) => Some(*ms),
            Phase::Rate(_, d) | Phase::Fast(d) => *d,
        };
        if let Some(ms) = dur {
            if ph_start.elapsed() >= Duration::from_millis(ms) {
                if let Phase::Stall(_) = ph {
                    st.stall_ends.lock().unwrap().push((mono_ns(), got));
                }
                idx += 1;
                ph_start = Instant::now();
                ph_bytes = 0;
                continue;
            }
        }
        match ph {
            Phase::Stall(_) => {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            _ => {}
        }
        let n = read_fd(fd, &mut buf);
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut || e.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break 'outer;
        }
        if n == 0 {
            break;
        }
        let n = n as usize;
        if got == 0 {
            st.first_byte_ns.store(mono_ns(), Ordering::Relaxed);
        }
        if verify {
            v.feed(&buf[..n]);
        }
        got += n as u64;
        ph_bytes += n as u64;
        st.read.store(got, Ordering::Relaxed);
        if let Phase::Rate(mibps, _) = ph {
            let allowed_t = ph_bytes as f64 / (mibps as f64 * 1048576.0);
            let el = ph_start.elapsed().as_secs_f64();
            if allowed_t > el {
                std::thread::sleep(Duration::from_secs_f64(allowed_t - el));
            }
        }
    }
    st.done_ns.store(mono_ns(), Ordering::Relaxed);
    *st.res.lock().unwrap() = Some(RRes { ok: got == total && v.first_bad.is_none() && v.hash == v.ehash && v.clen == 0 && (verify || got == total), first_bad: v.first_bad, hash: v.hash, ehash: v.ehash, bytes: got });
    st.done.store(true, Ordering::Relaxed);
}

// ---------------------------------------------------------------- pattern writer (used by the sender and the local TCP source app)
struct WStats {
    sent: u64,
    secs: f64,
    max_send_ms: u128,
    blocks: Vec<(u64, u64)>, // (mono ns at end, ms) of sends that blocked >= 50 ms
    sent_at_longest_block_start: u64,
    outcome: String,
}
fn write_pattern(fd: RawFd, total: u64, gap_ms: u64, burst_kb: u64, wait_progress: Option<&AtomicU64>, fast_gen: bool) -> WStats {
    let mut buf = vec![0u8; 65536];
    let (mut sent, mut max_ms, mut blocks) = (0u64, 0u128, Vec::new());
    let mut longest_start_sent = 0u64;
    let t0 = Instant::now();
    let mut outcome = "done".to_string();
    let mut since_gap = 0u64;
    while sent < total {
        let n = std::cmp::min(buf.len() as u64, total - sent) as usize;
        if !fast_gen || sent == 0 {
            gen_fill(&mut buf[..n], sent / 8);
        }
        let mut off = 0usize;
        while off < n {
            let ts = Instant::now();
            let r = unsafe { libc::send(fd, buf[off..n].as_ptr() as *const _, n - off, libc::MSG_NOSIGNAL) };
            let el = ts.elapsed().as_millis();
            if r < 0 {
                let e = std::io::Error::last_os_error();
                outcome = format!("err_{}", e.raw_os_error().unwrap_or(0));
                return WStats { sent, secs: t0.elapsed().as_secs_f64(), max_send_ms: max_ms, blocks, sent_at_longest_block_start: longest_start_sent, outcome };
            }
            if el > max_ms {
                max_ms = el;
                longest_start_sent = sent;
            }
            if el >= 50 && blocks.len() < 64 {
                blocks.push((mono_ns(), el as u64));
            }
            off += r as usize;
            sent += r as u64;
            if let Some(p) = wait_progress {
                p.store(sent, Ordering::Relaxed);
            }
        }
        since_gap += n as u64;
        if gap_ms > 0 && burst_kb > 0 && since_gap >= burst_kb * 1024 {
            since_gap = 0;
            std::thread::sleep(Duration::from_millis(gap_ms));
        }
    }
    WStats { sent, secs: t0.elapsed().as_secs_f64(), max_send_ms: max_ms, blocks, sent_at_longest_block_start: longest_start_sent, outcome }
}

/// small-message latency: 16-byte records {seq, sender mono ns}; the reader records recv_mono - send_mono (raw, two clock domains for
/// vsock; the driver subtracts the synchronised offset)
fn reader_lat(s: TcpStream, n: u64, st: Arc<RStats>, lat: Arc<Mutex<Vec<i64>>>) {
    let fd = s.as_raw_fd();
    set_tmo(fd, libc::SO_RCVTIMEO, 50);
    let mut got = 0u64;
    let mut carry: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 4096];
    let mut seq_ok = true;
    let mut next_seq = 0u64;
    while got < n && !st.abort.load(Ordering::Relaxed) {
        let r = read_fd(fd, &mut buf);
        if r < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut || e.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        if r == 0 {
            break;
        }
        let now = mono_ns() as i64;
        carry.extend_from_slice(&buf[..r as usize]);
        while carry.len() >= 16 {
            let seq = u64::from_le_bytes(carry[0..8].try_into().unwrap());
            let t = u64::from_le_bytes(carry[8..16].try_into().unwrap()) as i64;
            if seq != next_seq {
                seq_ok = false;
            }
            next_seq += 1;
            lat.lock().unwrap().push(now - t);
            carry.drain(..16);
            got += 1;
        }
        st.read.store(got * 16, Ordering::Relaxed);
    }
    st.done_ns.store(mono_ns(), Ordering::Relaxed);
    *st.res.lock().unwrap() = Some(RRes { ok: got == n && seq_ok, first_bad: None, hash: 0, ehash: 0, bytes: got * 16 });
    st.done.store(true, Ordering::Relaxed);
}
fn write_lat(fd: RawFd, n: u64, interval_us: u64) -> WStats {
    let t0 = Instant::now();
    let mut sent = 0u64;
    for i in 0..n {
        let mut rec = [0u8; 16];
        rec[0..8].copy_from_slice(&i.to_le_bytes());
        rec[8..16].copy_from_slice(&mono_ns().to_le_bytes());
        let r = unsafe { libc::send(fd, rec.as_ptr() as *const _, 16, libc::MSG_NOSIGNAL) };
        if r != 16 {
            return WStats { sent, secs: t0.elapsed().as_secs_f64(), max_send_ms: 0, blocks: Vec::new(), sent_at_longest_block_start: 0, outcome: format!("err_{}", std::io::Error::last_os_error().raw_os_error().unwrap_or(0)) };
        }
        sent += 16;
        let target = t0 + Duration::from_micros((i + 1) * interval_us);
        while Instant::now() < target {
            std::hint::spin_loop();
        }
    }
    WStats { sent, secs: t0.elapsed().as_secs_f64(), max_send_ms: 0, blocks: Vec::new(), sent_at_longest_block_start: 0, outcome: "done".into() }
}
fn pct(v: &mut Vec<i64>, p: f64) -> i64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[((v.len() as f64 - 1.0) * p) as usize]
}

// ---------------------------------------------------------------- BPF
const FWD: u32 = 0;
const SENT: u32 = 1;
const VERDICTS: u32 = 2;
const SEND_OK: u32 = 3;
const SEND_FAIL: u32 = 4;
const LAST_ERRNO: u32 = 5;
const REFUSED: u32 = 6;
const PEAK: u32 = 7;
const H_EMITTED: u32 = 8;
const H_ACKED: u32 = 9;
const D_EMITTED: u32 = 10;
const PASSED: u32 = 11;
const HARD_DROPS: u32 = 12;
const RB_FAIL_V: u32 = 13;
const RB_FAIL_F: u32 = 14;
const PARKED: u32 = 16;
const FORWARDED: u32 = 17;
const HOLD: u32 = 18;
const RS_OK: u32 = 19;
const RS_ZERO: u32 = 20;
const RS_ERR: u32 = 21;
const RS_BYTES: u32 = 22;
const VR_OK: u32 = 23;
const VR_ZERO: u32 = 24;
const VR_ERR: u32 = 25;

struct Bpf {
    e: Ebpf,
    rb: RingBuf<MapData>,
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
        // optional: count what sk_psock_verdict_recv returns (0 = psock was gone, the dequeued skb was dropped)
        {
            let p: &mut FExit = e.program_mut("vr_ret").unwrap().try_into().unwrap();
            if let Err(x) = p.load("sk_psock_verdict_recv", &btf).and_then(|_| p.attach().map(|_| ())) {
                out(format!("{{\"ev\":\"warn\",\"msg\":\"fexit sk_psock_verdict_recv unavailable: {x:?}\"}}"));
            }
        }
        // optional: count what virtio_transport_read_skb returns (0 = an skb was dequeued and dropped)
        let kp: &mut KProbe = e.program_mut("rs_ret").unwrap().try_into().unwrap();
        match kp.load().and_then(|_| kp.attach("virtio_transport_read_skb", 0)) {
            Ok(_) => {}
            Err(x) => out(format!("{{\"ev\":\"warn\",\"msg\":\"kretprobe read_skb unavailable: {x:?}\"}}")),
        }
        let rb = RingBuf::try_from(e.take_map("EVENTS").unwrap()).map_err(|x| format!("ringbuf: {x:?}"))?;
        Ok(Bpf { e, rb })
    }
    fn st(&self, i: u32) -> u64 {
        let a: Array<_, u64> = Array::try_from(self.e.map("STATE").unwrap()).unwrap();
        a.get(&i, 0).unwrap_or(0)
    }
    fn set_st(&mut self, i: u32, v: u64) {
        let mut a: Array<_, u64> = Array::try_from(self.e.map_mut("STATE").unwrap()).unwrap();
        a.set(i, v, 0).unwrap();
    }
    fn set_cfg(&mut self, i: u32, v: u64) {
        let mut a: Array<_, u64> = Array::try_from(self.e.map_mut("CFG").unwrap()).unwrap();
        a.set(i, v, 0).unwrap();
    }
    fn insert(&mut self, map: &str, key: u32, fd: RawFd) -> Result<(), String> {
        let mut m = SockHash::<_, u32>::try_from(self.e.map_mut(map).unwrap()).unwrap();
        m.insert(key, fd, 0).map_err(|x| format!("{map} insert: {x:?}"))
    }
    fn remove(&mut self, map: &str, key: u32) {
        let mut m = SockHash::<_, u32>::try_from(self.e.map_mut(map).unwrap()).unwrap();
        let _ = m.remove(&key);
    }
    fn queued(&self) -> u64 {
        self.st(FWD).saturating_sub(self.st(SENT))
    }
    /// pending ring buffer event kinds, in order
    fn events(&mut self) -> Vec<(u32, u64, u64, u64)> {
        let mut v = Vec::new();
        while let Some(it) = self.rb.next() {
            let b: &[u8] = &it;
            if b.len() >= 32 {
                let k = u32::from_ne_bytes(b[0..4].try_into().unwrap());
                let f = u64::from_ne_bytes(b[8..16].try_into().unwrap());
                let s = u64::from_ne_bytes(b[16..24].try_into().unwrap());
                let t = u64::from_ne_bytes(b[24..32].try_into().unwrap());
                v.push((k, f, s, t));
            }
        }
        v
    }
}

// ---------------------------------------------------------------- the pump (owner drains the residue in order)
#[derive(Default)]
struct PumpStats {
    recv: u64,
    sent: u64,
    ms: u64,
    rejoins: u64,
    rejoin_fail: u64,
}
enum PumpEnd {
    Eof,
    Done,
    Rejoined,
    Err(String),
}
fn pump(bpf: &mut Bpf, src: RawFd, tgt: RawFd, done: &AtomicBool, rejoin: bool, gate_lw: i32, ps: &mut PumpStats, deadline: Instant) -> PumpEnd {
    let t0 = Instant::now();
    let mut buf = vec![0u8; 65536];
    let (mut have, mut off) = (0usize, 0usize);
    let r = loop {
        if done.load(Ordering::Relaxed) {
            break PumpEnd::Done;
        }
        if Instant::now() > deadline {
            break PumpEnd::Err("deadline".into());
        }
        if off == have {
            let n = unsafe { libc::recv(src, buf.as_mut_ptr() as *mut _, buf.len(), libc::MSG_DONTWAIT) };
            if n > 0 {
                have = n as usize;
                off = 0;
                ps.recv += n as u64;
            } else if n == 0 {
                break PumpEnd::Eof;
            } else {
                let e = std::io::Error::last_os_error();
                if e.kind() != std::io::ErrorKind::WouldBlock {
                    break PumpEnd::Err(format!("recv {e}"));
                }
                if rejoin {
                    let he = bpf.st(H_EMITTED);
                    bpf.set_st(H_ACKED, he);
                    if gate_lw > 0 {
                        // undo the gate before the source goes back into the sockmap, otherwise vsock_data_ready never fires
                        setsockopt_i32(src, libc::SOL_SOCKET, libc::SO_RCVLOWAT, 1);
                    }
                    if let Err(x) = bpf.insert("VERD", 0, src) {
                        break PumpEnd::Err(x);
                    }
                    if ioctl_i32(src, SIOCINQ) > 0 {
                        if gate_lw > 0 {
                            setsockopt_i32(src, libc::SOL_SOCKET, libc::SO_RCVLOWAT, gate_lw);
                        }
                        bpf.remove("VERD", 0);
                        ps.rejoin_fail += 1;
                    } else {
                        ps.rejoins += 1;
                        break PumpEnd::Rejoined;
                    }
                    continue;
                }
                poll1(src, libc::POLLIN, 20);
                continue;
            }
        }
        let n = unsafe { libc::send(tgt, buf[off..have].as_ptr() as *const _, have - off, libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL) };
        if n > 0 {
            off += n as usize;
            ps.sent += n as u64;
        } else {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::WouldBlock {
                poll1(tgt, libc::POLLOUT, 20);
            } else {
                break PumpEnd::Err(format!("send {e}"));
            }
        }
    };
    ps.ms += t0.elapsed().as_millis() as u64;
    r
}

/// TCP source, mode 3 (park). Started once the target backlog is empty (queued <= LOW). The owner receives the bytes
/// the verdict parked on the source (SK_PASS) and sends them to the target itself, in order, then publishes the
/// cumulative byte count (FORWARDED). The verdict program resumes redirecting on its own once PARKED == FORWARDED.
fn park_drain(bpf: &mut Bpf, src: RawFd, tgt: RawFd, done: &AtomicBool, ps: &mut PumpStats, deadline: Instant) -> PumpEnd {
    let t0 = Instant::now();
    let mut buf = vec![0u8; 65536];
    let (mut have, mut off) = (0usize, 0usize);
    let r = loop {
        if done.load(Ordering::Relaxed) {
            break PumpEnd::Done;
        }
        if Instant::now() > deadline {
            break PumpEnd::Err("deadline".into());
        }
        if off == have {
            bpf.set_st(HOLD, 1);
            let n = unsafe { libc::recv(src, buf.as_mut_ptr() as *mut _, buf.len(), libc::MSG_DONTWAIT) };
            if n > 0 {
                have = n as usize;
                off = 0;
                ps.recv += n as u64;
            } else if n == 0 {
                bpf.set_st(HOLD, 0);
                break PumpEnd::Eof;
            } else {
                let e = std::io::Error::last_os_error();
                if e.kind() != std::io::ErrorKind::WouldBlock {
                    break PumpEnd::Err(format!("recv {e}"));
                }
                // everything received so far has been sent: publish it, release the hold, and see whether we caught up
                bpf.set_st(FORWARDED, ps.recv);
                bpf.set_st(HOLD, 0);
                if bpf.st(PARKED) <= ps.recv {
                    let he = bpf.st(H_EMITTED);
                    bpf.set_st(H_ACKED, he);
                    break PumpEnd::Rejoined;
                }
                poll1(src, libc::POLLIN, 5);
                continue;
            }
        }
        let n = unsafe { libc::send(tgt, buf[off..have].as_ptr() as *const _, have - off, libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL) };
        if n > 0 {
            off += n as usize;
            ps.sent += n as u64;
        } else {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::WouldBlock {
                poll1(tgt, libc::POLLOUT, 20);
            } else {
                break PumpEnd::Err(format!("send {e}"));
            }
        }
    };
    ps.ms += t0.elapsed().as_millis() as u64;
    r
}

// ---------------------------------------------------------------- sink (the owner)
struct Pair {
    c: TcpStream, // redirect target (in SOCKS)
    s: TcpStream, // its peer: the target app
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
    let label = arg(a, "--label").unwrap_or("vv".into());
    let obj = arg(a, "--obj").unwrap_or("/valve-bpf.o".into());
    let port = argn(a, "--port", 7000) as u32;
    let trials = argn(a, "--trials", 1);
    let bytes = if argn(a, "--lat", 0) > 0 { argn(a, "--lat", 0) * 16 } else { argn(a, "--bytes", 64 << 20) & !7 };
    let high_kb = argn(a, "--high-kb", 0); // 0 = no valve
    let low_kb = argn(a, "--low-kb", 0);
    let resume = arg(a, "--resume").unwrap_or("pump".into()); // pump | kick | naive
    let rejoin = argn(a, "--rejoin", 0) == 1;
    let phases = parse_phases(&arg(a, "--app").unwrap_or("f".into()));
    let mode = argn(a, "--mode", 0);
    let redir_flags = argn(a, "--redir-flags", 0);
    let hard_kb = argn(a, "--hard-kb", 0);
    let owner_delay_ms = argn(a, "--owner-delay-ms", 0);
    let src_rcvbuf = argn(a, "--src-rcvbuf", 0) as i32;
    let tgt_sndbuf = argn(a, "--tgt-sndbuf", 0) as i32;
    let vsock_buf = argn(a, "--vsock-buf", 0);
    let abort_mb = argn(a, "--abort-queued-mb", 0);
    let max_s = argn(a, "--max-s", 90);
    let sample_ms = argn(a, "--sample-ms", 100);
    let verbose_trials = argn(a, "--verbose-trials", 1);
    let gap_ms = argn(a, "--gap-ms", 0);
    let burst_kb = argn(a, "--burst-kb", 0);
    let hold_ms = argn(a, "--hold-ms", 200);
    let verify = argn(a, "--verify", 1) == 1;
    let gate = arg(a, "--gate").unwrap_or("none".into()); // lowat: raise SO_RCVLOWAT before removing the source
    let force_pump = argn(a, "--force-pump", 0) == 1;
    let fast_gen = argn(a, "--fast-gen", 0) == 1;
    let gate_delay_us = argn(a, "--gate-delay-us", 0);
    let lat_n = argn(a, "--lat", 0);
    let lat_interval = argn(a, "--lat-interval-us", 300);

    let mut bpf = Bpf::load(&obj).unwrap_or_else(|e| {
        out(format!("{{\"ev\":\"fatal\",\"err\":{:?}}}", e));
        std::process::exit(2)
    });
    bpf.set_cfg(0, mode);
    bpf.set_cfg(1, high_kb * 1024);
    bpf.set_cfg(2, redir_flags);
    bpf.set_cfg(3, low_kb * 1024);
    bpf.set_cfg(4, hard_kb * 1024);
    let lst = if kind == "tcpsrc" { None } else { Some(vsock_listen(port, vsock_buf).expect("vsock listen")) };
    out(format!(
        "{{\"ev\":\"sink_ready\",\"label\":\"{}\",\"kind\":\"{}\",\"uname\":\"{}\",\"port\":{},\"bytes\":{},\"high_kb\":{},\"low_kb\":{},\"resume\":\"{}\",\"rejoin\":{},\"app\":{:?},\"mode\":{},\"redir_flags\":{},\"owner_delay_ms\":{}}}",
        label, kind, uname(), port, bytes, high_kb, low_kb, resume, rejoin, arg(a, "--app").unwrap_or("f".into()), mode, redir_flags, owner_delay_ms
    ));
    let epoch = Instant::now();

    for trial in 0..trials {
        let verbose = trial < verbose_trials;
        for i in 0..32u32 {
            bpf.set_st(i, 0);
        }
        let _ = bpf.events();
        let pair = tcp_pair().expect("tcp pair");
        if tgt_sndbuf > 0 {
            setsockopt_i32(pair.c.as_raw_fd(), libc::SOL_SOCKET, libc::SO_SNDBUF, tgt_sndbuf);
        }
        let rst = Arc::new(RStats::default());
        // ---- the flow source
        let v: Option<OwnedFd>;
        let mut src_pair: Option<(TcpStream, TcpStream, TcpListener)> = None; // tcpsrc: (src, app writer end, l)
        let mut off_info = (0i64, 0u64);
        if kind == "tcpsrc" {
            let l2 = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
            let ap = TcpStream::connect(l2.local_addr().unwrap()).unwrap();
            let (src, _) = l2.accept().unwrap();
            if src_rcvbuf > 0 {
                setsockopt_i32(src.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF, src_rcvbuf);
            }
            src_pair = Some((src, ap, l2));
            v = None;
        } else {
            let l = lst.as_ref().unwrap();
            let f = accept(l.as_raw_fd()).expect("accept");
            if vsock_buf > 0 {
                set_vsock_buf(f.as_raw_fd(), vsock_buf);
            }
            // clock sync service: 'P' -> reply mono ns; 'D' -> done
            set_tmo(f.as_raw_fd(), libc::SO_RCVTIMEO, 20000);
            loop {
                let mut b = [0u8; 1];
                let n = read_fd(f.as_raw_fd(), &mut b);
                if n != 1 {
                    break;
                }
                if b[0] == b'P' {
                    write_fd(f.as_raw_fd(), &mono_ns().to_le_bytes());
                } else {
                    break;
                }
            }
            set_tmo(f.as_raw_fd(), libc::SO_RCVTIMEO, 0);
            v = Some(f);
        }
        let src_fd: RawFd = match (&v, &src_pair) {
            (Some(v), _) => v.as_raw_fd(),
            (None, Some((s, _, _))) => s.as_raw_fd(),
            _ => unreachable!(),
        };
        // ---- target app
        let rs2 = rst.clone();
        let s_clone = pair.s.try_clone().unwrap();
        let ph = phases.clone();
        let lat_vec: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
        let lv2 = lat_vec.clone();
        let rd = if lat_n > 0 {
            std::thread::spawn(move || reader_lat(s_clone, lat_n, rs2, lv2))
        } else {
            std::thread::spawn(move || reader(s_clone, bytes, ph, rs2, verify))
        };
        let (m0, sk0) = (meminfo_kb(&["MemAvailable", "SUnreclaim"]), skb_active());
        let (avail0, sun0) = (m0[0], m0[1]);
        let (mut min_avail, mut max_sun, mut max_skb) = (avail0, sun0, sk0);
        if !force_pump {
            bpf.insert("SOCKS", 1, pair.c.as_raw_fd()).expect("insert target");
            bpf.insert("VERD", 0, src_fd).expect("insert source");
        }
        let t0 = Instant::now();
        let t0_ns = mono_ns();
        let (busy0, _tot0) = cpu_busy_jiffies();
        let owner_cpu0 = thread_cpu_ms();
        let mut writer: Option<std::thread::JoinHandle<WStats>> = None;
        if let Some((_, ap, _)) = &src_pair {
            let apfd = ap.as_raw_fd();
            writer = Some(std::thread::spawn(move || if lat_n > 0 { write_lat(apfd, lat_n, lat_interval) } else { write_pattern(apfd, bytes, gap_ms, burst_kb, None, fast_gen) }));
        } else if let Some(v) = &v {
            if write_fd(v.as_raw_fd(), b"G") != 1 {
                out("{\"ev\":\"warn\",\"msg\":\"G write failed\"}".to_string());
            }
        }
        // ---- the owner loop
        let mut outcome = String::new();
        let mut forced_state = "";
        let mut closed_events: Vec<String> = Vec::new();
        let mut ps = PumpStats::default();
        let (mut valve_closes, mut resumes) = (0u64, 0u64);
        let mut state = "open";
        let mut done_at: Option<Instant> = None;
        let mut last_sample = Instant::now() - Duration::from_secs(1);
        let mut close_queued: Vec<u64> = Vec::new();
        let deadline = t0 + Duration::from_secs(max_s);
        if force_pump {
            // the flow is never put in a sockmap: the owner reads the source and writes the target itself
            let pe = pump(&mut bpf, src_fd, pair.c.as_raw_fd(), &rst.done, false, 0, &mut ps, deadline);
            forced_state = match pe {
                PumpEnd::Done => "done",
                PumpEnd::Eof => "eof",
                PumpEnd::Rejoined => "rejoined",
                PumpEnd::Err(e) => {
                    outcome = format!("pump_err_{}", e);
                    "err"
                }
            };
            state = forced_state;
        }
        loop {
            // wait for a ring buffer event (or 2 ms)
            poll1(bpf.rb.as_raw_fd(), libc::POLLIN, 2);
            for (k, f, s, kt) in bpf.events() {
                if k == 1 && high_kb > 0 && resume == "park" {
                    valve_closes += 1;
                    state = "parked";
                    close_queued.push(bpf.queued());
                    closed_events.push(format!("{{\"t_ms\":{},\"ev_fwd\":{},\"ev_sent\":{},\"queued_at_event\":{},\"src_inq\":{},\"ktime\":{}}}", t0.elapsed().as_millis(), f, s, bpf.queued(), ioctl_i32(src_fd, SIOCINQ), kt));
                } else if k == 2 && resume == "park" {
                    resumes += 1;
                    state = "park-drain";
                    let pe = park_drain(&mut bpf, src_fd, pair.c.as_raw_fd(), &rst.done, &mut ps, deadline);
                    match pe {
                        PumpEnd::Rejoined => state = "open",
                        PumpEnd::Eof => state = "eof",
                        PumpEnd::Done => state = "done",
                        PumpEnd::Err(e) => {
                            outcome = format!("park_err_{}", e);
                            state = "err";
                        }
                    }
                } else if k == 1 && high_kb > 0 && (resume != "none") {
                    if owner_delay_ms > 0 {
                        std::thread::sleep(Duration::from_millis(owner_delay_ms));
                    }
                    if gate == "lowat" {
                        let lw = if vsock_buf > 0 { vsock_buf } else { 262144 } as i32;
                        setsockopt_i32(src_fd, libc::SOL_SOCKET, libc::SO_RCVLOWAT, lw);
                        if gate_delay_us > 0 {
                            std::thread::sleep(Duration::from_micros(gate_delay_us));
                        }
                    }
                    bpf.remove("VERD", 0);
                    valve_closes += 1;
                    state = "closed";
                    let q = bpf.queued();
                    close_queued.push(q);
                    closed_events.push(format!("{{\"t_ms\":{},\"ev_fwd\":{},\"ev_sent\":{},\"queued_at_remove\":{},\"src_inq\":{},\"ktime\":{}}}", t0.elapsed().as_millis(), f, s, q, ioctl_i32(src_fd, SIOCINQ), kt));
                } else if k == 2 && state == "closed" {
                    resumes += 1;
                    match resume.as_str() {
                        "pump" => {
                            state = "pump";
                            let gl: i32 = if gate == "lowat" { (if vsock_buf > 0 { vsock_buf } else { 262144 }) as i32 } else { 0 };
                            let pe = pump(&mut bpf, src_fd, pair.c.as_raw_fd(), &rst.done, rejoin, gl, &mut ps, deadline);
                            match pe {
                                PumpEnd::Rejoined => state = "open",
                                PumpEnd::Eof => state = "eof",
                                PumpEnd::Done => state = "done",
                                PumpEnd::Err(e) => {
                                    outcome = format!("pump_err_{}", e);
                                    state = "err";
                                }
                            }
                        }
                        "kick" => {
                            let he = bpf.st(H_EMITTED);
                            bpf.set_st(H_ACKED, he);
                            bpf.insert("VERD", 0, src_fd).expect("reinsert");
                            setsockopt_i32(src_fd, libc::SOL_SOCKET, libc::SO_RCVLOWAT, 1);
                            state = "open";
                        }
                        _ => {
                            // naive: re-insert only
                            let he = bpf.st(H_EMITTED);
                            bpf.set_st(H_ACKED, he);
                            bpf.insert("VERD", 0, src_fd).expect("reinsert");
                            state = "open";
                        }
                    }
                }
            }
            let queued = bpf.queued();
            let inq_src = ioctl_i32(src_fd, SIOCINQ);
            if last_sample.elapsed() >= Duration::from_millis(sample_ms.min(50).max(10)) {
                last_sample = Instant::now();
                let m = meminfo_kb(&["MemAvailable", "SUnreclaim"]);
                let sk = if verbose { skb_active() } else { 0 };
                min_avail = min_avail.min(m[0]);
                max_sun = max_sun.max(m[1]);
                max_skb = max_skb.max(sk);
                if verbose {
                    out(format!(
                        "{{\"ev\":\"sample\",\"label\":\"{}\",\"trial\":{},\"t_ms\":{},\"state\":\"{}\",\"fwd\":{},\"sent\":{},\"queued\":{},\"peak\":{},\"src_inq\":{},\"tgt_outq\":{},\"peer_inq\":{},\"app_read\":{},\"pumped\":{},\"parked\":{},\"rs_zero\":{},\"mem_avail_kb\":{},\"sunreclaim_kb\":{},\"skb_head\":{}}}",
                        label, trial, t0.elapsed().as_millis(), state, bpf.st(FWD), bpf.st(SENT), queued, bpf.st(PEAK), inq_src,
                        ioctl_i32(pair.c.as_raw_fd(), SIOCOUTQ), ioctl_i32(pair.s.as_raw_fd(), SIOCINQ), rst.read.load(Ordering::Relaxed), ps.sent, bpf.st(PARKED).saturating_sub(bpf.st(FORWARDED)), bpf.st(RS_ZERO), m[0], m[1], sk
                    ));
                }
            }
            if let Some(dn) = done_at {
                if dn.elapsed() >= Duration::from_millis(hold_ms) {
                    break;
                }
                continue;
            }
            if rst.done.load(Ordering::Relaxed) {
                outcome = if outcome.is_empty() { "complete".into() } else { outcome };
                done_at = Some(Instant::now());
                continue;
            }
            if abort_mb > 0 && (queued >= abort_mb * 1048576 || bpf.st(PARKED).saturating_sub(bpf.st(FORWARDED)) >= abort_mb * 1048576) {
                outcome = "abort_cap".into();
                break;
            }
            if Instant::now() > deadline {
                outcome = "timeout".into();
                break;
            }
            if state == "err" {
                break;
            }
        }
        let secs = t0.elapsed().as_secs_f64();
        let (busy1, _tot1) = cpu_busy_jiffies();
        let owner_cpu_ms = thread_cpu_ms() - owner_cpu0;
        let busy_ms = (busy1 - busy0) * 10; // USER_HZ = 100
        let st_snap: Vec<u64> = (0..26).map(|i| bpf.st(i)).collect();
        let queued_end = st_snap[0].saturating_sub(st_snap[1]);
        let src_inq_end = ioctl_i32(src_fd, SIOCINQ);
        let app_read = rst.read.load(Ordering::Relaxed);
        rst.abort.store(true, Ordering::Relaxed);
        // wait for the reader
        let _ = rd.join();
        let rr = rst.res.lock().unwrap().take();
        let stall_ends = rst.stall_ends.lock().unwrap().clone();
        let first_byte = rst.first_byte_ns.load(Ordering::Relaxed);
        let done_ns = rst.done_ns.load(Ordering::Relaxed);
        // local tcp-source writer stats (same clock domain as the reader)
        let mut wjson = String::from("null");
        if let Some((s, ap, l)) = src_pair.take() {
            // closing the app end unblocks the writer if it is stuck
            unsafe { libc::shutdown(ap.as_raw_fd(), libc::SHUT_RDWR) };
            if let Some(h) = writer.take() {
                if let Ok(w) = h.join() {
                    let blocks: Vec<String> = w.blocks.iter().map(|(e, ms)| format!("[{},{}]", e, ms)).collect();
                    wjson = format!("{{\"sent\":{},\"secs\":{:.3},\"max_send_ms\":{},\"sent_at_longest_block_start\":{},\"outcome\":\"{}\",\"blocks\":[{}]}}", w.sent, w.secs, w.max_send_ms, w.sent_at_longest_block_start, w.outcome, blocks.join(","));
                }
            }
            drop((s, ap, l));
        }
        let (mem1, mem2);
        // tear down
        bpf.remove("VERD", 0);
        bpf.remove("SOCKS", 1);
        mem1 = meminfo_kb(&["MemAvailable", "SUnreclaim"]);
        drop(v);
        drop(pair);
        std::thread::sleep(Duration::from_millis(1500));
        mem2 = meminfo_kb(&["MemAvailable", "SUnreclaim"]);
        let (rok, rbad, rhash, rehash, rbytes) = match &rr {
            Some(r) => (r.ok, r.first_bad.map(|x| x as i64).unwrap_or(-1), r.hash, r.ehash, r.bytes),
            None => (false, -2, 0, 0, app_read),
        };
        let (lp50, lp90, lp99, lmax, lmin, lcount) = {
            let mut v = lat_vec.lock().unwrap().clone();
            (pct(&mut v, 0.5), pct(&mut v, 0.9), pct(&mut v, 0.99), pct(&mut v, 1.0), pct(&mut v, 0.0), v.len())
        };
        let stalls: Vec<String> = stall_ends.iter().map(|(t, b)| format!("[{},{}]", t, b)).collect();
        out(format!(
            "{{\"ev\":\"trial_end\",\"label\":\"{}\",\"kind\":\"{}\",\"trial\":{},\"outcome\":\"{}\",\"secs\":{:.3},\"bytes\":{},\"app_read\":{},\"app_ok\":{},\"first_bad\":{},\"hash\":{},\"ehash\":{},\"reader_first_byte_ns\":{},\"reader_done_ns\":{},\"reader_mibps\":{:.1},\"stall_ends\":[{}],\"t0_ns\":{},\"fwd\":{},\"sent\":{},\"queued_end\":{},\"peak_queued\":{},\"verdicts\":{},\"refused\":{},\"send_ok\":{},\"send_fail\":{},\"last_errno\":{},\"high_events\":{},\"drained_events\":{},\"passed\":{},\"hard_drops\":{},\"rb_fail\":{},\"valve_closes\":{},\"resumes\":{},\"close_queued\":{:?},\"pump_recv\":{},\"pump_sent\":{},\"pump_ms\":{},\"rejoins\":{},\"rejoin_fail\":{},\"final_state\":\"{}\",\"src_inq_end\":{},\"closes\":[{}],\"writer\":{},\"avail_start_kb\":{},\"min_avail_kb\":{},\"avail_end_kb\":{},\"avail_after_close_kb\":{},\"sunreclaim_start_kb\":{},\"max_sunreclaim_kb\":{},\"sunreclaim_end_kb\":{},\"sunreclaim_after_close_kb\":{},\"max_skb_head\":{},\"parked\":{},\"forwarded\":{},\"rs_ok\":{},\"rs_zero\":{},\"rs_err\":{},\"rs_bytes\":{},\"vr_ok\":{},\"vr_zero\":{},\"vr_err\":{},\"owner_cpu_ms\":{},\"sys_busy_ms\":{},\"lat_raw_ns\":{{\"count\":{},\"min\":{},\"p50\":{},\"p90\":{},\"p99\":{},\"max\":{}}}}}",
            label, kind, trial, outcome, secs, bytes, rbytes, rok, rbad, rhash, rehash, first_byte, done_ns,
            if done_ns > first_byte && first_byte > 0 { rbytes as f64 / 1048576.0 / ((done_ns - first_byte) as f64 / 1e9) } else { 0.0 },
            stalls.join(","), t0_ns, st_snap[0], st_snap[1], queued_end, st_snap[7], st_snap[2], st_snap[6], st_snap[3], st_snap[4], st_snap[5],
            st_snap[8], st_snap[10], st_snap[11], st_snap[12], st_snap[13] + st_snap[14], valve_closes, resumes, close_queued, ps.recv, ps.sent, ps.ms, ps.rejoins, ps.rejoin_fail,
            state, src_inq_end, closed_events.join(","), wjson, avail0, min_avail, mem1[0], mem2[0], sun0, max_sun, mem1[1], mem2[1], max_skb, st_snap[16], st_snap[17], st_snap[19], st_snap[20], st_snap[21], st_snap[22], st_snap[23], st_snap[24], st_snap[25], owner_cpu_ms, busy_ms, lcount, lmin, lp50, lp90, lp99, lmax
        ));
    }
    out(format!("{{\"ev\":\"sink_done\",\"label\":\"{}\",\"t_ms\":{}}}", label, epoch.elapsed().as_millis()));
}

// ---------------------------------------------------------------- sender
fn send(a: &[String]) {
    let cid = argn(a, "--cid", 2) as u32;
    let port = argn(a, "--port", 7000) as u32;
    let bytes = argn(a, "--bytes", 64 << 20) & !7;
    let trials = argn(a, "--trials", 1);
    let label = arg(a, "--label").unwrap_or("vv".into());
    let gap_ms = argn(a, "--gap-ms", 0);
    let burst_kb = argn(a, "--burst-kb", 0);
    let vsock_buf = argn(a, "--vsock-buf", 0);
    let sync = argn(a, "--sync", 1) == 1;
    let fast_gen = argn(a, "--fast-gen", 0) == 1;
    let lat_n = argn(a, "--lat", 0);
    let lat_interval = argn(a, "--lat-interval-us", 300);
    out(format!("{{\"ev\":\"send_ready\",\"label\":\"{}\",\"uname\":\"{}\",\"cid\":{},\"port\":{},\"bytes\":{},\"trials\":{}}}", label, uname(), cid, port, bytes, trials));
    for trial in 0..trials {
        let mut fd = None;
        for _ in 0..400 {
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
        if vsock_buf > 0 {
            set_vsock_buf(raw, vsock_buf);
        }
        // clock sync: sink_clock = sender_clock + off
        let (mut best_rtt, mut off_ns) = (u64::MAX, 0i64);
        if sync {
            set_tmo(raw, libc::SO_RCVTIMEO, 5000);
            for _ in 0..24 {
                let t0 = mono_ns();
                if write_fd(raw, b"P") != 1 {
                    break;
                }
                let mut b = [0u8; 8];
                let mut got = 0;
                while got < 8 {
                    let n = read_fd(raw, &mut b[got..]);
                    if n <= 0 {
                        break;
                    }
                    got += n as usize;
                }
                let t1 = mono_ns();
                if got < 8 {
                    break;
                }
                let ts = u64::from_le_bytes(b);
                let rtt = t1 - t0;
                if rtt < best_rtt {
                    best_rtt = rtt;
                    off_ns = ts as i64 - ((t0 + t1) / 2) as i64;
                }
            }
        }
        write_fd(raw, b"D");
        set_tmo(raw, libc::SO_RCVTIMEO, 30000);
        let mut g = [0u8; 1];
        if read_fd(raw, &mut g) != 1 {
            out(format!("{{\"ev\":\"send_end\",\"label\":\"{}\",\"trial\":{},\"outcome\":\"no_go\"}}", label, trial));
            continue;
        }
        let t_start_ns = mono_ns();
        let w = if lat_n > 0 { write_lat(raw, lat_n, lat_interval) } else { write_pattern(raw, bytes, gap_ms, burst_kb, None, fast_gen) };
        // wait for the sink to close the flow
        set_tmo(raw, libc::SO_RCVTIMEO, 120000);
        let mut b = [0u8; 16];
        let rr = read_fd(raw, &mut b);
        let blocks: Vec<String> = w.blocks.iter().map(|(e, ms)| format!("[{},{}]", e, ms)).collect();
        out(format!(
            "{{\"ev\":\"send_end\",\"label\":\"{}\",\"trial\":{},\"outcome\":\"{}\",\"sent\":{},\"secs\":{:.3},\"mibps\":{:.1},\"max_send_ms\":{},\"sent_at_longest_block_start\":{},\"blocks\":[{}],\"off_ns\":{},\"best_rtt_ns\":{},\"t_start_ns\":{},\"peer_closed_read\":{}}}",
            label, trial, w.outcome, w.sent, w.secs, w.sent as f64 / w.secs.max(1e-6) / 1048576.0, w.max_send_ms, w.sent_at_longest_block_start, blocks.join(","), off_ns, if best_rtt == u64::MAX { 0 } else { best_rtt }, t_start_ns, rr
        ));
        drop(fd);
    }
}

// ---------------------------------------------------------------- agent (guest command channel)
fn agent() {
    let l = vsock_listen(4000, 0).expect("agent listen");
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
    match a.get(1).map(|s| s.as_str()) {
        Some("agent") => agent(),
        Some("sink") => sink(&a[2..]),
        Some("send") => send(&a[2..]),
        _ => {
            eprintln!("usage: valveprobe agent|sink|send");
            std::process::exit(1)
        }
    }
}
