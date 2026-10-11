//! v32 probe (netns-density-295): guest UDP strparser cell -> vsock SEQPACKET vs STREAM + host strparser.
//! Roles: `agent` (guest PID-1 helper), `host`, `guest`. Neither role reads or writes datagram payload
//! on a paired socket except the guest application (UDP sender) and the host sink (final observer).
mod sys;

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aya::{
    maps::{Array, HashMap, PerCpuArray, RingBuf, SockHash},
    programs::{FExit, SkSkb, TracePoint},
    Btf, Ebpf, EbpfLoader,
};
use serde_json::{json, Value};
use sys::*;

const MAGIC: [u8; 4] = *b"ZUD1";
const SOCK_SEQPACKET: i32 = 5;

#[repr(C)]
#[derive(Clone, Copy)]
struct Route {
    target: u64,
    mode: u32,
    pad: u32,
}
unsafe impl aya::Pod for Route {}

const CNT_NAMES: [(u32, &str); 33] = [
    (0, "guest_encoded"),
    (1, "guest_over_bound_drop"),
    (2, "helper_fail"),
    (3, "noroute"),
    (4, "redirect_ok"),
    (5, "redirect_fail"),
    (6, "strp_frames"),
    (7, "strp_bad_frame"),
    (8, "rx_ok"),
    (9, "rx_lt8"),
    (10, "rx_badmagic"),
    (11, "rx_short"),
    (12, "rx_long"),
    (13, "rx_long_second_magic"),
    (14, "rx_mismatch_passed_check_off"),
    (15, "zero_len"),
    (16, "fexit_calls"),
    (17, "fexit_bytes"),
    (18, "evt_drop"),
    (19, "strp_parse_calls"),
    (20, "tx_alloc_seq_eom"),
    (21, "tx_alloc_seq_noeom"),
    (22, "tx_alloc_seq_noeom_bytes"),
    (23, "tx_alloc_stream"),
    (24, "tx_alloc_stream_bytes"),
    (25, "tx_alloc_seq_eom_bytes"),
    (30, "rx_pkt_seq_eom"),
    (31, "rx_pkt_seq_noeom"),
    (32, "rx_pkt_seq_noeom_bytes"),
    (33, "rx_pkt_stream"),
    (34, "rx_pkt_stream_bytes"),
    (35, "rx_pkt_seq_eom_bytes"),
    (63, "_unused"),
];

// ------------------------------------------------------------------ args
struct Args(BTreeMap<String, String>);
impl Args {
    fn parse(v: &[String]) -> Args {
        let mut m = BTreeMap::new();
        let mut i = 0;
        while i < v.len() {
            if let Some(k) = v[i].strip_prefix("--") {
                let val = v.get(i + 1).cloned().unwrap_or_default();
                m.insert(k.to_string(), val);
                i += 2;
            } else {
                i += 1;
            }
        }
        Args(m)
    }
    fn s(&self, k: &str, d: &str) -> String {
        self.0.get(k).cloned().unwrap_or_else(|| d.to_string())
    }
    fn u(&self, k: &str, d: u64) -> u64 {
        self.0.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
}

fn uname() -> String {
    unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        libc::uname(&mut u);
        std::ffi::CStr::from_ptr(u.release.as_ptr()).to_string_lossy().into_owned()
    }
}

// ------------------------------------------------------------------ datagram content
fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

fn size_for(seq: u64, spec: &str) -> usize {
    if spec == "mix" {
        64 + (splitmix(seq) % (59_000 - 64 + 1)) as usize
    } else if spec == "mixs" {
        64 + (splitmix(seq) % (2048 - 64)) as usize
    } else {
        spec.parse().unwrap()
    }
}

#[inline]
fn pat(seq: u64, j: usize) -> u8 {
    (seq as u8).wrapping_mul(7).wrapping_add(j as u8)
}

fn build(seq: u64, size: usize, buf: &mut Vec<u8>) {
    buf.resize(size, 0);
    buf[0..8].copy_from_slice(&seq.to_be_bytes());
    buf[8..12].copy_from_slice(&(size as u32).to_be_bytes());
    let s7 = (seq as u8).wrapping_mul(7);
    for (j, b) in buf[12..].iter_mut().enumerate() {
        *b = s7.wrapping_add((j + 12) as u8);
    }
}

/// Some(seq) when `p` is a self-consistent datagram for `spec`; Err(reason) otherwise.
fn verify(p: &[u8], spec: &str) -> Result<u64, &'static str> {
    if p.len() < 12 {
        return Err("short_payload");
    }
    let seq = u64::from_be_bytes(p[0..8].try_into().unwrap());
    let size = u32::from_be_bytes(p[8..12].try_into().unwrap()) as usize;
    if size != p.len() {
        return Err("size_field_mismatch");
    }
    if spec != "any" && size != size_for(seq, spec) {
        return Err("size_not_for_seq");
    }
    let s7 = (seq as u8).wrapping_mul(7);
    for (j, b) in p[12..].iter().enumerate() {
        if *b != s7.wrapping_add((j + 12) as u8) {
            return Err("pattern_corrupt");
        }
    }
    Ok(seq)
}

// ------------------------------------------------------------------ bpf
fn load_bpf(obj: &str, role: &str, port: u32, bound: u32, check: u32) -> Ebpf {
    let mut e = EbpfLoader::new().load_file(obj).expect("load ELF");
    {
        let mut c: Array<_, u32> = Array::try_from(e.map_mut("CFG").unwrap()).unwrap();
        c.set(0, bound, 0).unwrap();
        c.set(1, check, 0).unwrap();
        c.set(2, port, 0).unwrap();
        c.set(4, std::env::var("V32_PARSELOG").map(|v| v.parse().unwrap_or(0)).unwrap_or(0), 0).unwrap();
    }
    let vfd = SockHash::<_, u64>::try_from(e.map("VERD").unwrap()).unwrap().fd().try_clone().unwrap();
    let p: &mut SkSkb = e.program_mut("verdict").unwrap().try_into().unwrap();
    p.load().expect("verdict load");
    p.attach(&vfd).expect("verdict attach");
    let sfd = SockHash::<_, u64>::try_from(e.map("STRP").unwrap()).unwrap().fd().try_clone().unwrap();
    let p: &mut SkSkb = e.program_mut("strp_parse").unwrap().try_into().unwrap();
    p.load().expect("parse load");
    p.attach(&sfd).expect("parse attach");
    let p: &mut SkSkb = e.program_mut("strp_verdict").unwrap().try_into().unwrap();
    p.load().expect("strp verdict load");
    p.attach(&sfd).expect("strp verdict attach");
    if role == "guest" {
        let btf = Btf::from_sys_fs().expect("btf");
        let p: &mut FExit = e.program_mut("sent").unwrap().try_into().unwrap();
        p.load("skb_send_sock", &btf).expect("fexit load");
        p.attach().expect("fexit attach");
        let p: &mut TracePoint = e.program_mut("tp_alloc").unwrap().try_into().unwrap();
        p.load().expect("tp_alloc load");
        p.attach("vsock", "virtio_transport_alloc_pkt").expect("tp_alloc attach");
    } else {
        let p: &mut TracePoint = e.program_mut("tp_recv").unwrap().try_into().unwrap();
        p.load().expect("tp_recv load");
        p.attach("vsock", "virtio_transport_recv_pkt").expect("tp_recv attach");
    }
    e
}

fn counters(e: &Ebpf) -> Vec<u64> {
    let a: PerCpuArray<_, u64> = PerCpuArray::try_from(e.map("CNT").unwrap()).unwrap();
    (0..64u32).map(|i| a.get(&i, 0).map(|v| v.iter().sum()).unwrap_or(0)).collect()
}

fn counters_json(c: &[u64]) -> Value {
    let mut m = serde_json::Map::new();
    for (i, n) in CNT_NAMES.iter() {
        if *i != 63 && c[*i as usize] != 0 {
            m.insert((*n).to_string(), json!(c[*i as usize]));
        }
    }
    Value::Object(m)
}

fn errs(e: &Ebpf) -> BTreeMap<String, u64> {
    let m: HashMap<_, u32, u64> = HashMap::try_from(e.map("ERRS").unwrap()).unwrap();
    m.iter()
        .flatten()
        .map(|(k, v)| {
            let name = match k {
                1 => "ok_full".to_string(),
                2 => "ok_partial".to_string(),
                99 => "zero".to_string(),
                k => format!("errno{}", k - 100),
            };
            (name, v)
        })
        .collect()
}

fn err_get(e: &Ebpf, key: u32) -> u64 {
    let m: HashMap<_, u32, u64> = HashMap::try_from(e.map("ERRS").unwrap()).unwrap();
    m.get(&key, 0).unwrap_or(0)
}

fn insert_sock(e: &mut Ebpf, map: &str, fd: RawFd) {
    let ck = cookie(fd);
    let mut m = SockHash::<_, u64>::try_from(e.map_mut(map).unwrap()).unwrap();
    m.insert(ck, fd, 0).unwrap_or_else(|x| panic!("{map} insert: {x:?}"));
}

fn set_route(e: &mut Ebpf, src_fd: RawFd, target_cookie: u64, mode: u32) {
    let mut m: HashMap<_, u64, Route> = HashMap::try_from(e.map_mut("ROUTE").unwrap()).unwrap();
    m.insert(cookie(src_fd), Route { target: target_cookie, mode, pad: 0 }, 0).unwrap();
}

fn vsock_set_opt(fd: RawFd, opt: i32, v: u64) {
    let r = unsafe { libc::setsockopt(fd, 40 /*AF_VSOCK as level*/, opt, &v as *const _ as *const _, 8) };
    assert_eq!(r, 0, "vsock setsockopt {opt}: {}", errno());
}

fn vsock_get_buf(fd: RawFd) -> u64 {
    let mut v = 0u64;
    let mut l = 8u32;
    unsafe { libc::getsockopt(fd, 40, 0, &mut v as *mut _ as *mut _, &mut l) };
    v
}

fn lo(port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
}

/// Connected AF_UNIX SOCK_DGRAM pair (record-preserving parking cell, no strparser needed).
fn unix_dgram_pair() -> (OwnedFd, OwnedFd) {
    let mut fds = [0i32; 2];
    let r = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()) };
    assert_eq!(r, 0, "socketpair {}", errno());
    unsafe { (std::os::fd::FromRawFd::from_raw_fd(fds[0]), std::os::fd::FromRawFd::from_raw_fd(fds[1])) }
}

/// Connected loopback TCP pair (client, accepted).
fn tcp_pair() -> (OwnedFd, OwnedFd) {
    let l = tcp_listen(lo(0)).unwrap();
    let port = local4(l.as_raw_fd()).unwrap().port();
    let c = socket(libc::AF_INET, libc::SOCK_STREAM).unwrap();
    connect4(c.as_raw_fd(), lo(port)).unwrap();
    let s = accept(l.as_raw_fd()).unwrap();
    (c, s)
}

fn pump_events(ring: &mut RingBuf<aya::maps::MapData>, sample: &mut Vec<Value>, kinds: &mut [u64; 8], second: &mut u64) {
    while let Some(it) = ring.next() {
        let b: &[u8] = &it;
        if b.len() < 24 {
            continue;
        }
        let u32_at = |o: usize| u32::from_ne_bytes(b[o..o + 4].try_into().unwrap());
        let kind = u32_at(0);
        if (kind as usize) < 8 {
            kinds[kind as usize] += 1;
        }
        if u32_at(12) != 0 {
            *second += 1;
        }
        if sample.len() < 24 || kind >= 5 && sample.len() < 120 {
            let seq = u64::from_ne_bytes(b[16..24].try_into().unwrap());
            let kname = ["", "short", "long", "lt8", "badmagic", "parse", "parse_bad", ""][kind as usize % 8];
            sample.push(json!({"kind": kname, "n": u32_at(4), "declared": u32_at(8), "second_magic": u32_at(12), "seq": if seq == u64::MAX { Value::Null } else { json!(seq) }, "first8_hex": format!("{:016x}", seq)}));
        }
    }
}

// ------------------------------------------------------------------ host sink
#[derive(Default)]
struct SinkStats {
    frames: AtomicU64,
    ok: AtomicU64,
    bad: AtomicU64,
    desync_bytes: AtomicU64,
    bytes: AtomicU64,
}

struct SinkOut {
    seen: Vec<u8>,
    bad_reasons: BTreeMap<&'static str, u64>,
}

fn now_ms_u() -> u64 {
    now_ms() as u64
}

/// Reads the redirected frame stream from S2, parses [magic][len][payload], verifies each payload.
fn sink_stream(s2: OwnedFd, spec: String, n: u64, st: Arc<SinkStats>, stop: Arc<AtomicBool>, pause_until: Arc<AtomicU64>) -> SinkOut {
    let fd = s2.as_raw_fd();
    let mut out = SinkOut { seen: vec![0u8; n as usize], bad_reasons: BTreeMap::new() };
    let mut buf: Vec<u8> = Vec::with_capacity(8 << 20);
    let mut chunk = vec![0u8; 1 << 20];
    let mut idle = 0;
    loop {
        if now_ms_u() < pause_until.load(Ordering::Relaxed) {
            sleep_ms(20);
            continue;
        }
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut p, 1, 100) };
        if r > 0 {
            let k = unsafe { libc::recv(fd, chunk.as_mut_ptr() as *mut _, chunk.len(), libc::MSG_DONTWAIT) };
            if k > 0 {
                buf.extend_from_slice(&chunk[..k as usize]);
                st.bytes.fetch_add(k as u64, Ordering::Relaxed);
                idle = 0;
            } else if k == 0 {
                break;
            }
        } else {
            idle += 1;
            if stop.load(Ordering::Relaxed) && idle > 5 {
                break;
            }
        }
        let mut pos = 0usize;
        loop {
            if buf.len() - pos < 8 {
                break;
            }
            if buf[pos..pos + 4] != MAGIC {
                // desync: scan forward for the next magic
                let mut q = pos + 1;
                while q + 4 <= buf.len() && buf[q..q + 4] != MAGIC {
                    q += 1;
                }
                st.desync_bytes.fetch_add((q - pos) as u64, Ordering::Relaxed);
                st.bad.fetch_add(1, Ordering::Relaxed);
                *out.bad_reasons.entry("stream_desync").or_default() += 1;
                if q + 4 > buf.len() {
                    pos = buf.len() - 3.min(buf.len() - pos);
                    break;
                }
                pos = q;
                continue;
            }
            let len = u32::from_be_bytes(buf[pos + 4..pos + 8].try_into().unwrap()) as usize;
            if len > 1 << 20 {
                st.bad.fetch_add(1, Ordering::Relaxed);
                *out.bad_reasons.entry("absurd_len").or_default() += 1;
                pos += 4;
                continue;
            }
            if buf.len() - pos < 8 + len {
                break;
            }
            st.frames.fetch_add(1, Ordering::Relaxed);
            match verify(&buf[pos + 8..pos + 8 + len], &spec) {
                Ok(seq) if (seq as usize) < out.seen.len() => {
                    st.ok.fetch_add(1, Ordering::Relaxed);
                    let c = &mut out.seen[seq as usize];
                    *c = c.saturating_add(1);
                }
                Ok(_) => {
                    st.bad.fetch_add(1, Ordering::Relaxed);
                    *out.bad_reasons.entry("seq_out_of_range").or_default() += 1;
                }
                Err(why) => {
                    st.bad.fetch_add(1, Ordering::Relaxed);
                    *out.bad_reasons.entry(why).or_default() += 1;
                }
            }
            pos += 8 + len;
        }
        if pos > 0 {
            buf.drain(..pos);
        }
    }
    out
}

/// Host userspace SEQPACKET reader (variant seqrecv): one recv() = one record up to EOM.
#[derive(Default)]
struct RecvStats {
    records: AtomicU64,
    ok: AtomicU64,
    short: AtomicU64,
    long: AtomicU64,
    long_second_magic: AtomicU64,
    lt8: AtomicU64,
    badmagic: AtomicU64,
    bytes: AtomicU64,
}

fn sink_seqrecv(fd: OwnedFd, spec: String, n: u64, rs: Arc<RecvStats>, stop: Arc<AtomicBool>, pause_until: Arc<AtomicU64>) -> SinkOut {
    let mut out = SinkOut { seen: vec![0u8; n as usize], bad_reasons: BTreeMap::new() };
    let mut buf = vec![0u8; 1 << 20];
    let raw = fd.as_raw_fd();
    let mut idle = 0;
    loop {
        if now_ms_u() < pause_until.load(Ordering::Relaxed) {
            sleep_ms(20);
            continue;
        }
        let mut p = libc::pollfd { fd: raw, events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut p, 1, 100) } <= 0 {
            idle += 1;
            if stop.load(Ordering::Relaxed) && idle > 5 {
                break;
            }
            continue;
        }
        let k = unsafe { libc::recv(raw, buf.as_mut_ptr() as *mut _, buf.len(), libc::MSG_DONTWAIT) };
        if k < 0 {
            continue;
        }
        if k == 0 {
            // SEQPACKET: a zero-length record or peer shutdown
            if p.revents & (libc::POLLHUP | libc::POLLRDHUP) != 0 {
                break;
            }
            continue;
        }
        idle = 0;
        let k = k as usize;
        rs.records.fetch_add(1, Ordering::Relaxed);
        rs.bytes.fetch_add(k as u64, Ordering::Relaxed);
        let rec = &buf[..k];
        if k < 8 {
            rs.lt8.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if rec[0..4] != MAGIC {
            rs.badmagic.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let declared = u32::from_be_bytes(rec[4..8].try_into().unwrap()) as usize;
        let body = k - 8;
        if body == declared {
            match verify(&rec[8..], &spec) {
                Ok(seq) if (seq as usize) < out.seen.len() => {
                    rs.ok.fetch_add(1, Ordering::Relaxed);
                    let c = &mut out.seen[seq as usize];
                    *c = c.saturating_add(1);
                }
                Ok(_) => *out.bad_reasons.entry("seq_out_of_range").or_default() += 1,
                Err(why) => *out.bad_reasons.entry(why).or_default() += 1,
            }
        } else if body < declared {
            rs.short.fetch_add(1, Ordering::Relaxed);
        } else {
            rs.long.fetch_add(1, Ordering::Relaxed);
            if k >= declared + 8 + 4 && rec[declared + 8..declared + 12] == MAGIC {
                rs.long_second_magic.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    out
}

fn outcome(seen: &[u8]) -> Value {
    let (mut once, mut dup, mut dup_extra, mut lost) = (0u64, 0u64, 0u64, 0u64);
    for c in seen {
        match *c {
            0 => lost += 1,
            1 => once += 1,
            c => {
                dup += 1;
                dup_extra += (c - 1) as u64;
            }
        }
    }
    json!({"n": seen.len(), "delivered_once": once, "duplicated_seqs": dup, "duplicate_extra_copies": dup_extra, "lost": lost})
}

// ------------------------------------------------------------------ host
fn cmd_host(a: &Args) {
    let variant = a.s("variant", "seq");
    let spec = a.s("size", "1024");
    let n = a.u("n", 1000);
    let port = a.u("port", 7700) as u32;
    let hostbuf = a.u("hostbuf", 0);
    let pause_ms = a.u("pause-ms", 0);
    let check = a.u("check", 1) as u32;
    let obj = a.s("obj", "v32-bpf.o");
    let out_path = a.s("out", "host.json");
    let t_start = Instant::now();
    let mut e = load_bpf(&obj, "host", port, 0, check);
    // sink pair: S1 is the redirect target for verified frames, S2 is read by the sink thread
    let (s1, s2) = tcp_pair();
    insert_sock(&mut e, "SOCKS", s1.as_raw_fd());
    let s1ck = cookie(s1.as_raw_fd());
    let st = Arc::new(SinkStats::default());
    let rs = Arc::new(RecvStats::default());
    let stop = Arc::new(AtomicBool::new(false));
    let pause_until = Arc::new(AtomicU64::new(u64::MAX));
    let ty = if variant == "stream" { libc::SOCK_STREAM } else { SOCK_SEQPACKET };
    let l = socket(libc::AF_VSOCK, ty).unwrap();
    if hostbuf != 0 {
        if hostbuf > 262_144 {
            vsock_set_opt(l.as_raw_fd(), 2 /*SO_VM_SOCKET_BUFFER_MAX_SIZE*/, hostbuf);
        }
        vsock_set_opt(l.as_raw_fd(), 0 /*SO_VM_SOCKET_BUFFER_SIZE*/, hostbuf);
    }
    let la = vm_addr(libc::VMADDR_CID_ANY, port);
    cvt(unsafe { libc::bind(l.as_raw_fd(), &la as *const _ as *const _, 16) }).unwrap();
    cvt(unsafe { libc::listen(l.as_raw_fd(), 8) }).unwrap();
    let lc = vsock_listen(libc::VMADDR_CID_ANY, port + 1, libc::SOCK_STREAM).unwrap();
    let listener_buf = vsock_get_buf(l.as_raw_fd());
    println!("HOST_READY");
    let child = accept(l.as_raw_fd()).expect("accept data");
    let child_buf = vsock_get_buf(child.as_raw_fd());
    let (h1, h2);
    let sink_thread;
    let recv_thread;
    let child_raw = child.as_raw_fd();
    match variant.as_str() {
        "seq" => {
            set_route(&mut e, child_raw, s1ck, 2);
            insert_sock(&mut e, "VERD", child_raw);
            h1 = None;
            h2 = None;
            let (sp, stc, stp, pu) = (spec.clone(), st.clone(), stop.clone(), pause_until.clone());
            sink_thread = Some(std::thread::spawn(move || sink_stream(s2, sp, n, stc, stp, pu)));
            recv_thread = None;
        }
        "stream" => {
            let (a1, a2) = tcp_pair();
            insert_sock(&mut e, "SOCKS", a1.as_raw_fd());
            set_route(&mut e, a2.as_raw_fd(), s1ck, 5);
            insert_sock(&mut e, "STRP", a2.as_raw_fd());
            if a.u("nodelay", 0) != 0 {
                setopt_i32(a1.as_raw_fd(), libc::IPPROTO_TCP, libc::TCP_NODELAY, 1).unwrap();
            }
            set_route(&mut e, child_raw, cookie(a1.as_raw_fd()), a.u("h1mode", 4) as u32);
            insert_sock(&mut e, "VERD", child_raw);
            h1 = Some(a1);
            h2 = Some(a2);
            let (sp, stc, stp, pu) = (spec.clone(), st.clone(), stop.clone(), pause_until.clone());
            sink_thread = Some(std::thread::spawn(move || sink_stream(s2, sp, n, stc, stp, pu)));
            recv_thread = None;
        }
        "seqrecv" => {
            h1 = None;
            h2 = None;
            sink_thread = None;
            let (sp, rsc, stp, pu) = (spec.clone(), rs.clone(), stop.clone(), pause_until.clone());
            let dup = unsafe { libc::dup(child_raw) };
            let dfd: OwnedFd = unsafe { std::os::fd::FromRawFd::from_raw_fd(dup) };
            recv_thread = Some(std::thread::spawn(move || sink_seqrecv(dfd, sp, n, rsc, stp, pu)));
            drop(s2);
        }
        v => panic!("variant {v}"),
    }
    let mut ring = RingBuf::try_from(e.take_map("EVT").unwrap()).unwrap();
    let ctl = accept(lc.as_raw_fd()).expect("accept ctl");
    write_all_ctl(ctl.as_raw_fd(), b"G").unwrap();
    let go = now_ms_u();
    pause_until.store(go + pause_ms, Ordering::Relaxed);
    // wait for the guest's DONE line
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    let deadline = Instant::now() + Duration::from_secs(a.u("timeout-s", 900));
    let mut sample = Vec::new();
    let mut kinds = [0u64; 8];
    let mut second = 0u64;
    loop {
        if Instant::now() > deadline {
            break;
        }
        let mut p = libc::pollfd { fd: ctl.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut p, 1, 200) } > 0 {
            let k = unsafe { libc::recv(ctl.as_raw_fd(), b.as_mut_ptr() as *mut _, 1, 0) };
            if k <= 0 {
                break;
            }
            if b[0] == b'\n' {
                break;
            }
            line.push(b[0]);
        }
        pump_events(&mut ring, &mut sample, &mut kinds, &mut second);
    }
    let guest_done = String::from_utf8_lossy(&line).to_string();
    // quiesce: progress = verdict counters + sink progress
    let mut last = (0u64, Instant::now());
    let qdeadline = Instant::now() + Duration::from_secs(a.u("quiesce-max-s", 300));
    loop {
        sleep_ms(250);
        pump_events(&mut ring, &mut sample, &mut kinds, &mut second);
        let c = counters(&e);
        let prog: u64 = c[8..15].iter().sum::<u64>() + c[30] + c[31] + c[33] + st.frames.load(Ordering::Relaxed) + st.bad.load(Ordering::Relaxed) + rs.records.load(Ordering::Relaxed);
        if prog != last.0 {
            last = (prog, Instant::now());
        }
        let paused = now_ms_u() < pause_until.load(Ordering::Relaxed);
        if !paused && last.1.elapsed() > Duration::from_secs(3) {
            break;
        }
        if Instant::now() > qdeadline {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    let (seen, bad_reasons) = if let Some(t) = sink_thread {
        let o = t.join().unwrap();
        (o.seen, o.bad_reasons)
    } else {
        let o = recv_thread.unwrap().join().unwrap();
        (o.seen, o.bad_reasons)
    };
    pump_events(&mut ring, &mut sample, &mut kinds, &mut second);
    let c = counters(&e);
    let cfg_buf: u32 = {
        let c: Array<_, u32> = Array::try_from(e.map("CFG").unwrap()).unwrap();
        c.get(&3, 0).unwrap_or(0)
    };
    let _ = write_all_ctl(ctl.as_raw_fd(), b"F");
    let res = json!({
        "role": "host", "uname": uname(), "variant": variant, "size": spec, "n": n, "hostbuf_arg": hostbuf, "check": check, "pause_ms": pause_ms,
        "listener_buffer_size": listener_buf, "accepted_child_buffer_size": child_buf, "peer_buf_alloc_seen_by_host_recv_pkt": cfg_buf,
        "guest_done": guest_done,
        "bpf_counters": counters_json(&c),
        "events": {"short": kinds[1], "long": kinds[2], "lt8": kinds[3], "badmagic": kinds[4], "long_with_second_magic": second, "sample": sample},
        "sink": {"frames_parsed": st.frames.load(Ordering::Relaxed), "frames_verified_ok": st.ok.load(Ordering::Relaxed), "bad": st.bad.load(Ordering::Relaxed),
                 "desync_bytes_skipped": st.desync_bytes.load(Ordering::Relaxed), "bytes": st.bytes.load(Ordering::Relaxed), "bad_reasons": bad_reasons},
        "seqrecv": {"records": rs.records.load(Ordering::Relaxed), "ok": rs.ok.load(Ordering::Relaxed), "short": rs.short.load(Ordering::Relaxed),
                    "long": rs.long.load(Ordering::Relaxed), "long_second_magic": rs.long_second_magic.load(Ordering::Relaxed),
                    "lt8": rs.lt8.load(Ordering::Relaxed), "badmagic": rs.badmagic.load(Ordering::Relaxed), "bytes": rs.bytes.load(Ordering::Relaxed)},
        "outcome": outcome(&seen),
        "wall_s": t_start.elapsed().as_secs_f64(),
    });
    std::fs::write(&out_path, serde_json::to_string_pretty(&res).unwrap()).unwrap();
    println!("HOST_DONE {}", res["outcome"]);
    drop((h1, h2, child, s1));
}

// ------------------------------------------------------------------ guest
fn udp_snmp() -> BTreeMap<String, u64> {
    let t = std::fs::read_to_string("/proc/net/snmp").unwrap_or_default();
    let mut lines = t.lines().filter(|l| l.starts_with("Udp:"));
    let (Some(h), Some(v)) = (lines.next(), lines.next()) else { return BTreeMap::new() };
    h.split_whitespace().zip(v.split_whitespace()).skip(1).map(|(k, v)| (k.to_string(), v.parse().unwrap_or(0))).collect()
}

fn cmd_guest(a: &Args) {
    let variant = a.s("variant", "seq");
    let spec = a.s("size", "1024");
    let n = a.u("n", 1000);
    let port = a.u("port", 7700) as u32;
    let bound = a.u("bound", 59_000) as u32;
    let cell_bytes = a.u("cell-bytes", 8 << 20);
    let synth = a.u("synth", 0);
    let obj = a.s("obj", "v32-bpf.o");
    let out_path = a.s("out", "guest.json");
    let mut e = load_bpf(&obj, "guest", port, bound, 1);
    let mut gring = RingBuf::try_from(e.take_map("EVT").unwrap()).unwrap();
    // cell: UDP intake -> P1 =TCP=> P2 (strparser) -> vsock leg
    let intake = socket(libc::AF_INET, libc::SOCK_DGRAM).unwrap();
    let r = unsafe { libc::setsockopt(intake.as_raw_fd(), libc::SOL_SOCKET, 33 /*SO_RCVBUFFORCE*/, &(256i32 << 20) as *const _ as *const _, 4) };
    assert_eq!(r, 0, "SO_RCVBUFFORCE {}", errno());
    bind4(intake.as_raw_fd(), lo(0)).unwrap();
    let intake_port = local4(intake.as_raw_fd()).unwrap().port();
    let cell = a.s("cell", "tcp");
    let k1 = a.u("k1", 0);
    let (p1, p2) = if cell == "unix" { unix_dgram_pair() } else { tcp_pair() };
    let ty = if variant == "stream" { libc::SOCK_STREAM } else { SOCK_SEQPACKET };
    let vs = vsock_connect(libc::VMADDR_CID_HOST, port, ty).expect("vsock data connect");
    let ctl = vsock_connect(libc::VMADDR_CID_HOST, port + 1, libc::SOCK_STREAM).expect("vsock ctl connect");
    let guest_buf = vsock_get_buf(vs.as_raw_fd());
    let vsck = cookie(vs.as_raw_fd());
    if synth == 0 {
        insert_sock(&mut e, "SOCKS", vs.as_raw_fd());
        {
            let mut t: HashMap<_, u64, u32> = HashMap::try_from(e.map_mut("TRACK").unwrap()).unwrap();
            t.insert(vsck, 1, 0).unwrap();
        }
        match cell.as_str() {
            "tcp" => {
                if a.u("nodelay", 0) != 0 {
                    setopt_i32(p1.as_raw_fd(), libc::IPPROTO_TCP, libc::TCP_NODELAY, 1).unwrap();
                }
                insert_sock(&mut e, "SOCKS", p1.as_raw_fd());
                set_route(&mut e, p2.as_raw_fd(), vsck, a.u("p2mode", 4) as u32);
                insert_sock(&mut e, "STRP", p2.as_raw_fd());
                set_route(&mut e, intake.as_raw_fd(), cookie(p1.as_raw_fd()), 1);
            }
            "direct" => {
                set_route(&mut e, intake.as_raw_fd(), vsck, 1);
            }
            "unix" => {
                insert_sock(&mut e, "SOCKS", p1.as_raw_fd());
                // a unix datagram skb is owned by its SENDER (p1), so the verdict on p2 resolves the route by p1's cookie
                set_route(&mut e, p1.as_raw_fd(), vsck, a.u("p2mode", 4) as u32);
                set_route(&mut e, p2.as_raw_fd(), vsck, a.u("p2mode", 4) as u32);
                if k1 == 0 {
                    insert_sock(&mut e, "VERD", p2.as_raw_fd());
                }
                set_route(&mut e, intake.as_raw_fd(), cookie(p1.as_raw_fd()), 1);
            }
            c => panic!("cell {c}"),
        }
        insert_sock(&mut e, "VERD", intake.as_raw_fd());
    }
    let mut g = [0u8; 1];
    read_exact_ctl(ctl.as_raw_fd(), &mut g, Duration::from_secs(60)).expect("wait G");
    let snmp0 = udp_snmp();
    let t0 = Instant::now();
    let mut sent = 0u64;
    let mut stalled = false;
    if synth != 0 {
        synth_send(vs.as_raw_fd());
    } else {
        let app = socket(libc::AF_INET, libc::SOCK_DGRAM).unwrap();
        connect4(app.as_raw_fd(), lo(intake_port)).unwrap();
        let mut buf = Vec::new();
        let avg = if spec == "mix" { 30_000 } else if spec == "mixs" { 1_100 } else { spec.parse::<u64>().unwrap() + 8 };
        let cell_frames = (cell_bytes / avg.max(1)).max(32);
        let mut last_progress = Instant::now();
        let mut last_seen = 0u64;
        for seq in 0..n {
            let size = size_for(seq, &spec);
            build(seq, size, &mut buf);
            let r = send_blocking(app.as_raw_fd(), &buf);
            if r != size as isize {
                eprintln!("sendto seq {seq}: {r} {}", errno());
            } else {
                sent += 1;
            }
            if k1 != 0 && seq + 1 == k1 {
                // K1 probe (unix cell): let the first k1 frames queue in P2 with no verdict, then install it
                let end = Instant::now() + Duration::from_secs(10);
                while counters(&e)[0] < k1 && Instant::now() < end {
                    sleep_ms(20);
                }
                sleep_ms(200);
                insert_sock(&mut e, "VERD", p2.as_raw_fd());
                if a.u("k1rearm", 0) != 0 {
                    let r = set_rcvlowat(p2.as_raw_fd(), 1);
                    eprintln!("k1 rearm SO_RCVLOWAT: {:?}", r);
                }
            }
            if seq % 32 == 31 {
                loop {
                    let c = counters(&e);
                    let sn = udp_snmp();
                    let rbuf = sn.get("RcvbufErrors").copied().unwrap_or(0) - snmp0.get("RcvbufErrors").copied().unwrap_or(0);
                    let accounted = c[0] + c[1] + c[2] + rbuf;
                    let full_ok = err_get(&e, 1);
                    let intake_out = sent.saturating_sub(accounted);
                    let cell_out = c[0].saturating_sub(full_ok);
                    if intake_out <= 256 && cell_out <= cell_frames {
                        break;
                    }
                    let prog = c[0] + full_ok + c[16];
                    if prog != last_seen {
                        last_seen = prog;
                        last_progress = Instant::now();
                    }
                    if last_progress.elapsed() > Duration::from_secs(20) {
                        stalled = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_micros(200));
                }
                if stalled {
                    break;
                }
            }
        }
    }
    let send_wall = t0.elapsed().as_secs_f64();
    // drain: counters stable for 2 s and every datagram accounted
    let mut last = (u64::MAX, Instant::now());
    let drain_deadline = Instant::now() + Duration::from_secs(a.u("drain-max-s", 120));
    loop {
        sleep_ms(200);
        let c = counters(&e);
        let prog = c[0] + c[1] + c[2] + c[6] + c[16] + c[17] + c[20] + c[21] + c[23];
        if prog != last.0 {
            last = (prog, Instant::now());
        }
        if last.1.elapsed() > Duration::from_secs(2) || Instant::now() > drain_deadline {
            break;
        }
    }
    let drain_wall = t0.elapsed().as_secs_f64();
    let snmp1 = udp_snmp();
    let c = counters(&e);
    let mut udp_delta = BTreeMap::new();
    for (k, v) in &snmp1 {
        let d = v - snmp0.get(k).copied().unwrap_or(0);
        if d != 0 {
            udp_delta.insert(k.clone(), d);
        }
    }
    let mut gsample = Vec::new();
    let mut gkinds = [0u64; 8];
    let mut gsecond = 0u64;
    pump_events(&mut gring, &mut gsample, &mut gkinds, &mut gsecond);
    let line = format!("{}\n", json!({"sent": sent, "stalled": stalled}));
    write_all_ctl(ctl.as_raw_fd(), line.as_bytes()).unwrap();
    let mut f = [0u8; 1];
    let _ = read_exact_ctl(ctl.as_raw_fd(), &mut f, Duration::from_secs(a.u("final-wait-s", 400)));
    let res = json!({
        "role": "guest", "uname": uname(), "variant": variant, "cell": cell, "k1": k1, "size": spec, "n": n, "synth": synth, "bound": bound,
        "guest_vsock_buffer_size": guest_buf, "app_sent_ok": sent, "sender_stalled": stalled,
        "send_wall_s": send_wall, "drain_wall_s": drain_wall,
        "bpf_counters": counters_json(&c), "skb_send_sock_outcomes_on_vsock_leg": errs(&e), "udp_snmp_delta": udp_delta,
        "guest_events": {"parse_bad": gkinds[6], "sample": gsample},
    });
    std::fs::write(&out_path, serde_json::to_string_pretty(&res).unwrap()).unwrap();
    println!("GUEST_DONE sent={sent} stalled={stalled}");
    drop((p1, p2, vs, intake));
}

fn frame(seq: u64, size: usize) -> Vec<u8> {
    let mut p = Vec::new();
    build(seq, size, &mut p);
    let mut f = Vec::with_capacity(8 + size);
    f.extend_from_slice(&MAGIC);
    f.extend_from_slice(&(size as u32).to_be_bytes());
    f.extend_from_slice(&p);
    f
}

/// Synthetic records straight onto the guest's SEQPACKET (no BPF on the guest): exercises the host check only.
fn synth_send(fd: RawFd) {
    let mut sends: Vec<Vec<u8>> = vec![frame(0, 100)];
    let mut merged = frame(1, 100);
    merged.extend(frame(2, 200));
    sends.push(merged); // two whole frames in one record
    let t = frame(3, 300);
    sends.push(t[..150].to_vec()); // truncated frame
    let mut tr = frame(4, 100);
    tr.extend_from_slice(&[1, 2, 3]);
    sends.push(tr); // frame + 3 trailing bytes
    sends.push(frame(5, 100));
    let mut h = frame(6, 500);
    h.truncate(8 + 20);
    sends.push(h); // header declares 500, 20 present
    sends.push(frame(7, 59_000));
    // a whole frame delivered as two records (what a partial send that is NOT followed by EAGAIN looks like to the verdict)
    let f8 = frame(8, 3000);
    sends.push(f8[..1000].to_vec());
    sends.push(f8[1000..].to_vec());
    // fragment forgery: the outer datagram carries a valid inner frame header exactly at the split point
    let mut inner = frame(9, 100);
    let mut outer_payload = vec![0xEEu8; 92];
    outer_payload.append(&mut inner);
    let mut outer = Vec::new();
    outer.extend_from_slice(&MAGIC);
    outer.extend_from_slice(&(outer_payload.len() as u32).to_be_bytes());
    outer.extend_from_slice(&outer_payload);
    sends.push(outer[..100].to_vec());
    sends.push(outer[100..].to_vec());
    for s in sends {
        let r = send_blocking(fd, &s);
        assert_eq!(r, s.len() as isize, "synth send {}", errno());
        sleep_ms(50);
    }
}

// ------------------------------------------------------------------ guest agent (command runner on vsock 4000)
fn recv_exact(fd: RawFd, buf: &mut [u8]) -> Result<(), ()> {
    let mut off = 0;
    while off < buf.len() {
        let n = recv_blocking(fd, &mut buf[off..]);
        if n <= 0 {
            return Err(());
        }
        off += n as usize;
    }
    Ok(())
}

fn cmd_agent() {
    let l = vsock_listen(libc::VMADDR_CID_ANY, 4000, libc::SOCK_STREAM).expect("agent listen");
    loop {
        let Ok(c) = accept(l.as_raw_fd()) else { continue };
        std::thread::spawn(move || {
            let raw = c.as_raw_fd();
            let mut len = [0u8; 4];
            if recv_exact(raw, &mut len).is_err() {
                return;
            }
            let mut cmd = vec![0u8; u32::from_be_bytes(len) as usize];
            if recv_exact(raw, &mut cmd).is_err() {
                return;
            }
            let cmd = String::from_utf8_lossy(&cmd).to_string();
            let out = std::process::Command::new("/bin/sh").arg("-c").arg(&cmd).stdin(std::process::Stdio::null()).output();
            let (rc, mut text) = match out {
                Ok(o) => (o.status.code().unwrap_or(-1), [o.stdout, o.stderr].concat()),
                Err(e) => (-1, e.to_string().into_bytes()),
            };
            text.extend_from_slice(format!("\nAGENT_RC={rc}\n").as_bytes());
            let mut off = 0;
            while off < text.len() {
                let n = send_blocking(raw, &text[off..]);
                if n <= 0 {
                    break;
                }
                off += n as usize;
            }
        });
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let a = Args::parse(&argv[2.min(argv.len())..]);
    match argv.get(1).map(String::as_str) {
        Some("host") => cmd_host(&a),
        Some("guest") => cmd_guest(&a),
        Some("agent") => cmd_agent(),
        _ => eprintln!("usage: v32probe host|guest|agent [--k v]"),
    }
}
