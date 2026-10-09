//! V-20 throwaway probe (GH #295): intake-child identity by clone-flagged socket storage (M-1).
//! All namespaces / cgroups / pins are prefixed v20-. Usage: v20 run <bpf.o> <cgroup-dir>
//! (run.sh creates netns v20-host / v20-peer, the cgroup and the nft/routing model; this binary does the rest).

use std::{
    fs::{self, File},
    io::{self, Write},
    net::Ipv4Addr,
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use aya::{
    maps::{HashMap as AyaHash, MapData, PerCpuArray, SockHash},
    programs::{CgroupAttachMode, SkLookup, SockOps},
    Ebpf, EbpfLoader, Pod,
};

const PIN_DIR: &str = "/sys/fs/bpf/v20-pins";
const NONE: u32 = 0xFFFF_FFFF;
const DECLARED: u16 = 8080;
const BIND_BASE: u16 = 20480;
const LEGC_PORT: u16 = 19999;
const SRC_INTAKE: Ipv4Addr = Ipv4Addr::new(10, 77, 20, 2); // peer client class that steering hands to an intake
const SRC_LEGC: Ipv4Addr = Ipv4Addr::new(10, 77, 20, 3); // peer client class that nft TPROXYs to leg-C
const CLIENT_THREADS: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
struct SteerKey {
    addr: u32,
    port: u32,
}
unsafe impl Pod for SteerKey {}
fn key(ip: Ipv4Addr, port: u16) -> SteerKey {
    SteerKey { addr: u32::from_ne_bytes(ip.octets()), port: port as u32 }
}

// ------------------------------------------------------------------ raw bpf(2)
fn bpf(cmd: libc::c_long, attr: &mut [u8; 128]) -> Result<i64, i32> {
    let r = unsafe { libc::syscall(libc::SYS_bpf, cmd, attr.as_mut_ptr(), attr.len()) };
    if r < 0 {
        Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1))
    } else {
        Ok(r)
    }
}
fn put32(a: &mut [u8; 128], off: usize, v: u32) {
    a[off..off + 4].copy_from_slice(&v.to_ne_bytes());
}
fn put64(a: &mut [u8; 128], off: usize, v: u64) {
    a[off..off + 8].copy_from_slice(&v.to_ne_bytes());
}
/// Minimal BTF: one 32-bit INT type (id 1), used as key and value type of the sk_storage maps.
fn btf_blob() -> Vec<u8> {
    let strs = b"\0int\0";
    let mut types = Vec::new();
    types.extend_from_slice(&1u32.to_ne_bytes()); // name_off
    types.extend_from_slice(&(1u32 << 24).to_ne_bytes()); // info: kind INT
    types.extend_from_slice(&4u32.to_ne_bytes()); // size
    types.extend_from_slice(&32u32.to_ne_bytes()); // int_data: 32 bits
    let mut b = Vec::new();
    b.extend_from_slice(&0xeB9Fu16.to_ne_bytes());
    b.push(1);
    b.push(0);
    b.extend_from_slice(&24u32.to_ne_bytes());
    b.extend_from_slice(&0u32.to_ne_bytes());
    b.extend_from_slice(&(types.len() as u32).to_ne_bytes());
    b.extend_from_slice(&(types.len() as u32).to_ne_bytes());
    b.extend_from_slice(&(strs.len() as u32).to_ne_bytes());
    b.extend_from_slice(&types);
    b.extend_from_slice(strs);
    b
}
fn btf_load() -> Result<OwnedFd, i32> {
    let blob = btf_blob();
    let mut a = [0u8; 128];
    put64(&mut a, 0, blob.as_ptr() as u64);
    put32(&mut a, 16, blob.len() as u32);
    bpf(18, &mut a).map(|fd| unsafe { OwnedFd::from_raw_fd(fd as i32) })
}
fn create_sk_storage(name: &str, flags: u32, btf: RawFd) -> Result<OwnedFd, i32> {
    let mut a = [0u8; 128];
    put32(&mut a, 0, 24); // BPF_MAP_TYPE_SK_STORAGE
    put32(&mut a, 4, 4);
    put32(&mut a, 8, 4);
    put32(&mut a, 12, 0);
    put32(&mut a, 16, flags);
    for (i, c) in name.bytes().take(15).enumerate() {
        a[28 + i] = c;
    }
    put32(&mut a, 48, btf as u32);
    put32(&mut a, 52, 1);
    put32(&mut a, 56, 1);
    bpf(0, &mut a).map(|fd| unsafe { OwnedFd::from_raw_fd(fd as i32) })
}
fn obj_pin(fd: RawFd, path: &str) -> Result<(), i32> {
    let c = std::ffi::CString::new(path).unwrap();
    let mut a = [0u8; 128];
    put64(&mut a, 0, c.as_ptr() as u64);
    put32(&mut a, 8, fd as u32);
    bpf(6, &mut a).map(|_| ())
}
fn obj_get(path: &str) -> Result<OwnedFd, i32> {
    let c = std::ffi::CString::new(path).unwrap();
    let mut a = [0u8; 128];
    put64(&mut a, 0, c.as_ptr() as u64);
    bpf(7, &mut a).map(|fd| unsafe { OwnedFd::from_raw_fd(fd as i32) })
}
fn elem(cmd: libc::c_long, map: RawFd, sock: RawFd, val: &mut u32) -> Result<(), i32> {
    let mut k: i32 = sock;
    let mut a = [0u8; 128];
    put32(&mut a, 0, map as u32);
    put64(&mut a, 8, &mut k as *mut i32 as u64);
    if cmd != 3 {
        // BPF_MAP_DELETE_ELEM rejects any attr bytes beyond `key`
        put64(&mut a, 16, val as *mut u32 as u64);
        put64(&mut a, 24, 0); // BPF_ANY
    }
    bpf(cmd, &mut a).map(|_| ())
}
/// Tag a socket in an sk_storage map (BPF_MAP_UPDATE_ELEM with the socket fd as key).
fn sk_put(map: RawFd, sock: RawFd, v: u32) -> Result<(), i32> {
    let mut v = v;
    elem(2, map, sock, &mut v)
}
/// Read a socket's entry (BPF_MAP_LOOKUP_ELEM); ENOENT -> None.
fn sk_get(map: RawFd, sock: RawFd) -> Option<u32> {
    let mut v = 0u32;
    match elem(1, map, sock, &mut v) {
        Ok(()) => Some(v),
        Err(e) if e == libc::ENOENT => None,
        Err(e) => panic!("sk_get errno {e}"),
    }
}
fn sk_del(map: RawFd, sock: RawFd) -> Result<(), i32> {
    let mut v = 0u32;
    elem(3, map, sock, &mut v)
}

// ------------------------------------------------------------------ sockets
fn setns_by_name(name: &str) -> io::Result<()> {
    let f = File::open(format!("/var/run/netns/{name}"))?;
    if unsafe { libc::setns(f.as_raw_fd(), libc::CLONE_NEWNET) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn sockaddr(ip: Ipv4Addr, port: u16) -> libc::sockaddr_in {
    libc::sockaddr_in {
        sin_family: libc::AF_INET as u16,
        sin_port: port.to_be(),
        sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes(ip.octets()) },
        sin_zero: [0; 8],
    }
}
fn setopt<T>(fd: RawFd, lvl: i32, opt: i32, v: T) {
    let r = unsafe { libc::setsockopt(fd, lvl, opt, &v as *const T as *const _, std::mem::size_of::<T>() as u32) };
    assert_eq!(r, 0, "setsockopt {lvl}/{opt}: {}", io::Error::last_os_error());
}
fn listen_on(ip: Ipv4Addr, port: u16, transparent: bool, backlog: i32, tfo: bool) -> io::Result<OwnedFd> {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let o = OwnedFd::from_raw_fd(fd);
        setopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, 1i32);
        setopt(fd, libc::SOL_IP, libc::IP_FREEBIND, 1i32);
        if transparent {
            setopt(fd, libc::SOL_IP, libc::IP_TRANSPARENT, 1i32);
        }
        if tfo {
            setopt(fd, libc::IPPROTO_TCP, libc::TCP_FASTOPEN, 1024i32);
        }
        let sa = sockaddr(ip, port);
        if libc::bind(fd, &sa as *const _ as *const _, std::mem::size_of_val(&sa) as u32) != 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::listen(fd, backlog) != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(o)
    }
}
fn cookie_of(fd: RawFd) -> u64 {
    let mut c = 0u64;
    let mut l = 8u32;
    let r = unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, 57 /* SO_COOKIE */, &mut c as *mut u64 as *mut _, &mut l) };
    assert_eq!(r, 0, "SO_COOKIE");
    c
}
fn local_of(fd: RawFd) -> (Ipv4Addr, u16) {
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut l = std::mem::size_of_val(&sa) as u32;
    let r = unsafe { libc::getsockname(fd, &mut sa as *mut _ as *mut _, &mut l) };
    assert_eq!(r, 0);
    (Ipv4Addr::from(sa.sin_addr.s_addr.to_ne_bytes()), u16::from_be(sa.sin_port))
}
/// Blocking connect from `src` (in the peer netns thread), optional TFO data. Returns fd or errno.
fn connect_from(src: Ipv4Addr, dst: Ipv4Addr, port: u16, tfo_data: bool) -> Result<OwnedFd, i32> {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1));
        }
        let o = OwnedFd::from_raw_fd(fd);
        setopt(fd, libc::SOL_IP, libc::IP_BIND_ADDRESS_NO_PORT, 1i32);
        let tv = libc::timeval { tv_sec: 10, tv_usec: 0 };
        setopt(fd, libc::SOL_SOCKET, libc::SO_SNDTIMEO, tv);
        setopt(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, tv);
        let sa = sockaddr(src, 0);
        if libc::bind(fd, &sa as *const _ as *const _, std::mem::size_of_val(&sa) as u32) != 0 {
            return Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1));
        }
        let da = sockaddr(dst, port);
        if tfo_data {
            let r = libc::sendto(fd, b"tfo-data".as_ptr() as *const _, 8, libc::MSG_FASTOPEN, &da as *const _ as *const _, std::mem::size_of_val(&da) as u32);
            if r < 0 {
                return Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1));
            }
        } else if libc::connect(fd, &da as *const _ as *const _, std::mem::size_of_val(&da) as u32) != 0 {
            return Err(io::Error::last_os_error().raw_os_error().unwrap_or(-1));
        }
        Ok(o)
    }
}
fn raise_nofile() {
    let r = libc::rlimit { rlim_cur: 1_000_000, rlim_max: 1_000_000 };
    unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &r) };
}

// ------------------------------------------------------------------ world
struct World {
    _ebpf: Ebpf,
    steer: SockHash<MapData, SteerKey>,
    seen: AyaHash<MapData, u64, u64>,
    cnt: PerCpuArray<MapData, u64>,
    clone_fd: OwnedFd,
    nc_fd: OwnedFd,
}
type Cnt = [u64; 16];
impl World {
    fn counters(&self) -> Cnt {
        let mut c = [0u64; 16];
        for (i, slot) in c.iter_mut().enumerate() {
            *slot = self.cnt.get(&(i as u32), 0).map(|v| v.iter().sum()).unwrap_or(0);
        }
        c
    }
    fn steer_put(&mut self, ip: Ipv4Addr, port: u16, l: RawFd) -> Result<(), i32> {
        struct Raw(RawFd);
        impl AsRawFd for Raw {
            fn as_raw_fd(&self) -> RawFd {
                self.0
            }
        }
        self.steer.insert(key(ip, port), Raw(l), 0).map_err(|e| match e {
            aya::maps::MapError::SyscallError(s) => s.io_error.raw_os_error().unwrap_or(-1),
            _ => -2,
        })
    }
    fn clear_seen(&mut self) {
        let ks: Vec<u64> = self.seen.keys().filter_map(|k| k.ok()).collect();
        for k in ks {
            let _ = self.seen.remove(&k);
        }
    }
}
fn delta(a: &Cnt, b: &Cnt) -> Cnt {
    let mut d = [0u64; 16];
    for i in 0..16 {
        d[i] = b[i].wrapping_sub(a[i]);
    }
    d
}
const NAMES: [&str; 14] = [
    "passive", "passive_tagged", "passive_untagged", "active", "active_tagged", "shared(100.96.0/24:8080)",
    "shared_tagged", "shared_untagged", "noclone_tagged", "state_cb", "state_cb_seen_tagged", "state_cb_now_untagged",
    "no_sk", "seen_insert_fail",
];
fn show_cnt(d: &Cnt) -> String {
    NAMES.iter().enumerate().map(|(i, n)| format!("{n}={}", d[i])).collect::<Vec<_>>().join(" ")
}

// ------------------------------------------------------------------ reporting
static RESULTS: Mutex<Vec<(String, bool, String)>> = Mutex::new(Vec::new());
fn say(s: impl AsRef<str>) {
    println!("{}", s.as_ref());
    let _ = io::stdout().flush();
}
fn verdict(claim: &str, ok: bool, detail: impl AsRef<str>) {
    say(format!("VERDICT [{}] {claim}: {}", if ok { "HOLDS" } else { "FALSIFIED" }, detail.as_ref()));
    RESULTS.lock().unwrap().push((claim.to_string(), ok, detail.as_ref().to_string()));
}
fn sh(cmd: &str) -> String {
    let o = Command::new("bash").arg("-c").arg(cmd).output().expect("sh");
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}
fn host(cmd: &str) -> String {
    sh(&format!("ip netns exec v20-host {cmd}"))
}
fn tcpext(name: &str) -> u64 {
    let s = fs::read_to_string("/proc/thread-self/net/netstat").unwrap_or_default();
    let mut lines = s.lines();
    while let Some(h) = lines.next() {
        if let Some(v) = lines.next() {
            if h.starts_with("TcpExt:") {
                let hs: Vec<&str> = h.split_whitespace().collect();
                let vs: Vec<&str> = v.split_whitespace().collect();
                if let Some(i) = hs.iter().position(|x| *x == name) {
                    return vs[i].parse().unwrap_or(0);
                }
            }
        }
    }
    0
}

// ------------------------------------------------------------------ intake model
struct Intake {
    id: u32,
    addr: Ipv4Addr, // workload_addr
    bind_port: u16,
    fd: OwnedFd,
}
fn make_intake(w: &mut World, id: u32, addr: Ipv4Addr, bind_port: u16, tag: bool, steer: bool) -> Intake {
    let fd = listen_on(addr, bind_port, false, 65535, false).expect("intake listen");
    if tag {
        sk_put(w.clone_fd.as_raw_fd(), fd.as_raw_fd(), id).expect("tag listener");
        // negative control: the same listener also tagged in the map WITHOUT the clone flag
        sk_put(w.nc_fd.as_raw_fd(), fd.as_raw_fd(), id).expect("tag listener (noclone)");
    }
    if steer {
        w.steer_put(addr, DECLARED, fd.as_raw_fd()).expect("steer");
    }
    Intake { id, addr, bind_port, fd }
}

struct Acceptor {
    fd: RawFd,
    expected: Option<u32>, // the tag every child of this listener must carry (None: none)
}
struct Job {
    src: Ipv4Addr,
    dst: Ipv4Addr,
    port: u16,
    n: usize,
    tfo: bool,
}
struct Drive {
    children: Vec<(Option<u32>, OwnedFd)>,
    held: Vec<OwnedFd>,
    connect_fail: usize,
    first_err: i32,
}

/// Run clients (in the peer netns, 16 threads) against `jobs`; accept on `acceptors` either concurrently
/// (`early == false`, `accept_threads` per listener) or only after every client wrote and closed (`early`).
fn drive(jobs: &[Job], acceptors: &[Acceptor], early: bool, accept_threads: usize) -> Drive {
    let total: usize = jobs.iter().map(|j| j.n).sum();
    let children: Arc<Mutex<Vec<(Option<u32>, OwnedFd)>>> = Arc::new(Mutex::new(Vec::with_capacity(total)));
    let done = Arc::new(AtomicBool::new(false));
    let accepted = Arc::new(AtomicUsize::new(0));
    let mut ahandles = Vec::new();
    let start_acceptors = |ahandles: &mut Vec<thread::JoinHandle<()>>| {
        for a in acceptors {
            for _ in 0..accept_threads {
                let (children, done, accepted, fd, exp) = (children.clone(), done.clone(), accepted.clone(), a.fd, a.expected);
                ahandles.push(thread::spawn(move || loop {
                    let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
                    unsafe { libc::poll(&mut p, 1, 20) };
                    loop {
                        let c = unsafe { libc::accept4(fd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
                        if c < 0 {
                            break;
                        }
                        children.lock().unwrap().push((exp, unsafe { OwnedFd::from_raw_fd(c) }));
                        accepted.fetch_add(1, Ordering::SeqCst);
                    }
                    if done.load(Ordering::SeqCst) {
                        break;
                    }
                }));
            }
        }
    };
    if !early {
        start_acceptors(&mut ahandles);
    }
    // flat, interleaved job list
    let mut flat: Vec<usize> = Vec::with_capacity(total);
    let maxn = jobs.iter().map(|j| j.n).max().unwrap_or(0);
    for i in 0..maxn {
        for (ji, j) in jobs.iter().enumerate() {
            if i < j.n {
                flat.push(ji);
            }
        }
    }
    let flat = Arc::new(flat);
    let held: Arc<Mutex<Vec<OwnedFd>>> = Arc::new(Mutex::new(Vec::new()));
    let fails = Arc::new(AtomicUsize::new(0));
    let first_err = Arc::new(AtomicUsize::new(0));
    let jobs_arc: Arc<Vec<(Ipv4Addr, Ipv4Addr, u16, bool)>> = Arc::new(jobs.iter().map(|j| (j.src, j.dst, j.port, j.tfo)).collect());
    let mut chandles = Vec::new();
    for t in 0..CLIENT_THREADS {
        let (flat, held, fails, first_err, jobs_arc) = (flat.clone(), held.clone(), fails.clone(), first_err.clone(), jobs_arc.clone());
        chandles.push(thread::spawn(move || {
            setns_by_name("v20-peer").expect("client setns");
            let mut mine = Vec::new();
            let mut i = t;
            while i < flat.len() {
                let (src, dst, port, tfo) = jobs_arc[flat[i]];
                match connect_from(src, dst, port, tfo) {
                    Ok(fd) => {
                        if early {
                            unsafe { libc::write(fd.as_raw_fd(), b"early".as_ptr() as *const _, 5) };
                            drop(fd); // write + FIN before any accept()
                        } else {
                            mine.push(fd);
                        }
                    }
                    Err(e) => {
                        fails.fetch_add(1, Ordering::SeqCst);
                        first_err.store(e as usize, Ordering::SeqCst);
                    }
                }
                i += CLIENT_THREADS;
            }
            held.lock().unwrap().extend(mine);
        }));
    }
    for h in chandles {
        h.join().unwrap();
    }
    if early {
        start_acceptors(&mut ahandles);
    }
    let want = total - fails.load(Ordering::SeqCst);
    let t0 = Instant::now();
    while accepted.load(Ordering::SeqCst) < want && t0.elapsed() < Duration::from_secs(20) {
        thread::sleep(Duration::from_millis(10));
    }
    done.store(true, Ordering::SeqCst);
    for h in ahandles {
        h.join().unwrap();
    }
    let children = std::mem::take(&mut *children.lock().unwrap());
    let held = std::mem::take(&mut *held.lock().unwrap());
    Drive { children, held, connect_fail: fails.load(Ordering::SeqCst), first_err: first_err.load(Ordering::SeqCst) as i32 }
}

#[derive(Default, Debug)]
struct Check {
    n: usize,
    user_ok: usize,
    user_bad: usize,
    seen_ok: usize,
    seen_missing: usize,
    seen_bad: usize,
    nc_tagged: usize,
    shared_local: usize,
}
impl Check {
    fn perfect(&self) -> bool {
        self.n > 0 && self.user_ok == self.n && self.seen_ok == self.n && self.nc_tagged == 0
    }
    fn show(&self) -> String {
        format!(
            "children={} userspace_lookup ok={} bad={} | sock_ops(PASSIVE_ESTABLISHED) ok={} missing={} bad={} | noclone_map_tagged={} | local=100.96.0.x:8080 {}",
            self.n, self.user_ok, self.user_bad, self.seen_ok, self.seen_missing, self.seen_bad, self.nc_tagged, self.shared_local
        )
    }
}
/// Compare each child's tag (read from the child fd in userspace AND as recorded by sock_ops at
/// PASSIVE_ESTABLISHED, keyed by socket cookie) with what its listener should have given it.
fn verify(w: &World, children: &[(Option<u32>, OwnedFd)]) -> Check {
    let mut c = Check::default();
    for (exp, fd) in children {
        c.n += 1;
        let want = exp.unwrap_or(NONE);
        let u = sk_get(w.clone_fd.as_raw_fd(), fd.as_raw_fd()).unwrap_or(NONE);
        if u == want {
            c.user_ok += 1;
        } else {
            c.user_bad += 1;
        }
        if sk_get(w.nc_fd.as_raw_fd(), fd.as_raw_fd()).is_some() {
            c.nc_tagged += 1;
        }
        match w.seen.get(&cookie_of(fd.as_raw_fd()), 0) {
            Ok(v) => {
                if (v as u32) == want {
                    c.seen_ok += 1;
                } else {
                    c.seen_bad += 1;
                }
            }
            Err(_) => c.seen_missing += 1,
        }
        let (la, lp) = local_of(fd.as_raw_fd());
        if lp == DECLARED && la.octets()[..3] == [100, 96, 0] {
            c.shared_local += 1;
        }
    }
    c
}

fn intake_addr(j: usize) -> Ipv4Addr {
    Ipv4Addr::new(100, 96, 0, 2 + j as u8)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 || a[1] != "run" {
        eprintln!("usage: v20 run <bpf.o> <cgroup-dir>");
        std::process::exit(2);
    }
    raise_nofile();
    let ok = scenario(&a[2], &a[3]);
    std::process::exit(if ok { 0 } else { 1 });
}

fn scenario(elf: &str, cgdir: &str) -> bool {
    say(format!("uname -r: {}", sh("uname -r").trim()));
    say(format!("uname -m: {}", sh("uname -m").trim()));
    // ---- maps: BTF-typed sk_storage maps, created + pinned here; aya reuses them (pinning = ByName)
    let _ = fs::remove_dir_all(PIN_DIR);
    let _ = fs::create_dir(PIN_DIR);
    let btf = btf_load().expect("BTF_LOAD");
    let m_clone = create_sk_storage("v20_tags", 1 | (1 << 9), btf.as_raw_fd()).expect("create TAGS (BPF_F_NO_PREALLOC|BPF_F_CLONE)");
    let m_nc = create_sk_storage("v20_tags_nc", 1, btf.as_raw_fd()).expect("create TAGS_NOCLONE");
    obj_pin(m_clone.as_raw_fd(), &format!("{PIN_DIR}/TAGS")).expect("pin TAGS");
    obj_pin(m_nc.as_raw_fd(), &format!("{PIN_DIR}/TAGS_NOCLONE")).expect("pin TAGS_NOCLONE");
    let mut ebpf = EbpfLoader::new().allow_unsupported_maps().map_pin_path(PIN_DIR).load_file(elf).expect("load elf");
    {
        let p: &mut SockOps = ebpf.program_mut("v20_sockops").unwrap().try_into().unwrap();
        p.load().expect("load v20_sockops (verifier)");
        let cg = File::open(cgdir).expect("open cgroup");
        p.attach(cg, CgroupAttachMode::Single).expect("attach sock_ops to the cgroup");
    }
    {
        let p: &mut SkLookup = ebpf.program_mut("v20_steer").unwrap().try_into().unwrap();
        p.load().expect("load v20_steer");
        let ns = File::open("/var/run/netns/v20-host").unwrap();
        p.attach(ns).expect("attach sk_lookup");
    }
    let steer = SockHash::try_from(ebpf.take_map("STEER").unwrap()).unwrap();
    let seen = AyaHash::try_from(ebpf.take_map("SEEN").unwrap()).unwrap();
    let cnt = PerCpuArray::try_from(ebpf.take_map("CNT").unwrap()).unwrap();
    // the fds aya holds are the pinned maps; use our own handles to the same objects
    let clone_fd = obj_get(&format!("{PIN_DIR}/TAGS")).unwrap();
    let nc_fd = obj_get(&format!("{PIN_DIR}/TAGS_NOCLONE")).unwrap();
    let mut w = World { _ebpf: ebpf, steer, seen, cnt, clone_fd, nc_fd };
    // this process (and every thread it makes) now lives in the owner cgroup; main thread goes to the host netns
    fs::write(format!("{cgdir}/cgroup.procs"), format!("{}", std::process::id())).expect("join cgroup");
    setns_by_name("v20-host").expect("setns v20-host");
    say(format!("cgroup: {cgdir}; map flags: TAGS=NO_PREALLOC|CLONE(0x{:x}) TAGS_NOCLONE=NO_PREALLOC(0x1)", 1 | (1 << 9)));

    // ---------------------------------------------------------------- S0: wiring sanity (small)
    say("\n== S0 wiring: one intake, one leg-C, 20 connects each");
    let a90 = Ipv4Addr::new(100, 96, 0, 90);
    let i0 = make_intake(&mut w, 100, a90, BIND_BASE, true, true);
    let legc = listen_on(Ipv4Addr::new(127, 0, 0, 1), LEGC_PORT, true, 65535, false).expect("leg-C listen");
    let c0 = w.counters();
    let d = drive(
        &[
            Job { src: SRC_INTAKE, dst: a90, port: DECLARED, n: 20, tfo: false },
            Job { src: SRC_LEGC, dst: a90, port: DECLARED, n: 20, tfo: false },
        ],
        &[Acceptor { fd: i0.fd.as_raw_fd(), expected: Some(100) }, Acceptor { fd: legc.as_raw_fd(), expected: None }],
        false,
        1,
    );
    let cc = w.counters();
    say(format!("connect_fail={} first_err={}", d.connect_fail, d.first_err));
    let ck = verify(&w, &d.children);
    say(ck.show());
    say(format!("counters: {}", show_cnt(&delta(&c0, &cc))));
    drop(d);
    if ck.n != 40 {
        say("wiring failed; aborting");
        return false;
    }

    // ---------------------------------------------------------------- S1+S2: 10,000 intake + 10,000 leg-C, concurrent, shared addr:port
    say("\n== S1+S2: 8 intakes (ids 101..108, distinct workload addrs, declared 8080, bind 20480): 10,000 intake children;");
    say("   same time 10,000 leg-C (IP_TRANSPARENT, nft TPROXY) children whose local addr:port is the same 100.96.0.x:8080");
    let intakes: Vec<Intake> = (0..8).map(|j| make_intake(&mut w, 101 + j as u32, intake_addr(j), BIND_BASE, true, true)).collect();
    let mut jobs = Vec::new();
    let mut accs = Vec::new();
    for (j, it) in intakes.iter().enumerate() {
        jobs.push(Job { src: SRC_INTAKE, dst: intake_addr(j), port: DECLARED, n: 1250, tfo: false });
        jobs.push(Job { src: SRC_LEGC, dst: intake_addr(j), port: DECLARED, n: 1250, tfo: false });
        accs.push(Acceptor { fd: it.fd.as_raw_fd(), expected: Some(it.id) });
    }
    accs.push(Acceptor { fd: legc.as_raw_fd(), expected: None });
    w.clear_seen();
    let c0 = w.counters();
    let t0 = Instant::now();
    // 4 accept threads per intake and per leg-C listener: concurrent accepts on every listener
    let d = drive(&jobs, &accs, false, 4);
    let cc = w.counters();
    let dc = delta(&c0, &cc);
    say(format!("elapsed {:.2}s connect_fail={} first_err={}", t0.elapsed().as_secs_f64(), d.connect_fail, d.first_err));
    let (intake_ch, legc_ch): (Vec<_>, Vec<_>) = d.children.into_iter().partition(|(e, _)| e.is_some());
    let ck1 = verify(&w, &intake_ch);
    let ck2 = verify(&w, &legc_ch);
    say(format!("intake children: {}", ck1.show()));
    say(format!("leg-C children:  {}", ck2.show()));
    say(format!("sock_ops counters during S1+S2: {}", show_cnt(&dc)));
    // distinct ids per intake
    let mut per: std::collections::BTreeMap<u32, usize> = Default::default();
    for (e, fd) in &intake_ch {
        if let Some(t) = sk_get(w.clone_fd.as_raw_fd(), fd.as_raw_fd()) {
            *per.entry(t).or_default() += 1;
        }
        let _ = e;
    }
    say(format!("children per tag value read from the child fd: {per:?}"));
    verdict(
        "S1 every intake child carries its listener's tag at PASSIVE_ESTABLISHED (10,000)",
        ck1.n == 10_000 && ck1.perfect() && dc[C_SHARED_TAGGED] == 10_000,
        ck1.show(),
    );
    verdict(
        "S2 leg-C transparent children with the same local addr:port in the same cgroup carry no tag (10,000)",
        ck2.n == 10_000 && ck2.user_ok == 10_000 && ck2.seen_ok == 10_000 && dc[C_SHARED_UNTAGGED] == 10_000 && ck2.shared_local == 10_000,
        format!("{} ; sock_ops shared untagged={} shared tagged={}", ck2.show(), dc[C_SHARED_UNTAGGED], dc[C_SHARED_TAGGED]),
    );
    verdict(
        "S1/S4 concurrent accepts (4 accept threads per listener, 16 client threads): no child mis-tagged",
        ck1.user_bad == 0 && ck1.seen_bad == 0 && ck1.seen_missing == 0 && per.len() == 8 && per.values().all(|&n| n == 1250),
        format!("per-tag counts {per:?}"),
    );
    verdict(
        "S10 negative control: a tag in the map WITHOUT BPF_F_CLONE is not inherited (listeners tagged in both maps)",
        ck1.nc_tagged == 0 && dc[C_NOCLONE_TAGGED] == 0 && ck1.perfect(),
        format!("noclone-tagged children=0 of {}; sock_ops noclone_tagged={}", ck1.n, dc[C_NOCLONE_TAGGED]),
    );
    verdict(
        "S7a active (client-side) sockets in the cgroup never carry the tag",
        dc[C_ACTIVE] > 0 && dc[C_ACTIVE_TAGGED] == 0,
        format!("active_established={} tagged={}", dc[C_ACTIVE], dc[C_ACTIVE_TAGGED]),
    );
    drop(d.held);
    drop(intake_ch);
    drop(legc_ch);

    // ---------------------------------------------------------------- S3: write + FIN before accept()
    say("\n== S3: children that write and FIN before accept(): 10,000 intake + 2,000 leg-C, accepted only afterwards");
    w.clear_seen();
    let c0 = w.counters();
    let mut jobs = Vec::new();
    for j in 0..8 {
        jobs.push(Job { src: SRC_INTAKE, dst: intake_addr(j), port: DECLARED, n: 1250, tfo: false });
        jobs.push(Job { src: SRC_LEGC, dst: intake_addr(j), port: DECLARED, n: 250, tfo: false });
    }
    let d = drive(&jobs, &accs, true, 2);
    let dc = delta(&c0, &w.counters());
    let (ic, lc): (Vec<_>, Vec<_>) = d.children.into_iter().partition(|(e, _)| e.is_some());
    let ck1 = verify(&w, &ic);
    let ck2 = verify(&w, &lc);
    // data really was written before accept
    let mut readable = 0;
    for (_, fd) in ic.iter().take(200) {
        let mut b = [0u8; 8];
        let r = unsafe { libc::recv(fd.as_raw_fd(), b.as_mut_ptr() as *mut _, 8, libc::MSG_DONTWAIT) };
        if r == 5 {
            readable += 1;
        }
    }
    say(format!("connect_fail={} intake: {}", d.connect_fail, ck1.show()));
    say(format!("leg-C: {}", ck2.show()));
    say(format!("first 200 intake children had the 5 early bytes buffered: {readable}/200; counters: {}", show_cnt(&dc)));
    verdict(
        "S3 intake children that wrote and sent FIN before accept() carry the tag at PASSIVE_ESTABLISHED (10,000)",
        ck1.n == 10_000 && ck1.perfect() && readable == 200,
        ck1.show(),
    );
    verdict("S3 leg-C children in the same situation stay untagged (2,000)", ck2.n == 2_000 && ck2.user_ok == 2_000 && ck2.seen_ok == 2_000, ck2.show());
    drop(ic);
    drop(lc);

    // ---------------------------------------------------------------- S4: one listener, 16 accept threads, 16 client threads
    say("\n== S4: ONE intake listener, 16 concurrent accept threads, 16 client threads, 10,000 connects");
    w.clear_seen();
    let i4 = make_intake(&mut w, 140, Ipv4Addr::new(100, 96, 0, 40), BIND_BASE, true, true);
    let d = drive(
        &[Job { src: SRC_INTAKE, dst: Ipv4Addr::new(100, 96, 0, 40), port: DECLARED, n: 10_000, tfo: false }],
        &[Acceptor { fd: i4.fd.as_raw_fd(), expected: Some(140) }],
        false,
        16,
    );
    let ck = verify(&w, &d.children);
    say(format!("connect_fail={} {}", d.connect_fail, ck.show()));
    verdict("S4 one listener, 16 accept threads racing: every child tagged 140 (10,000)", ck.n == 10_000 && ck.perfect(), ck.show());
    drop(d);
    drop(i4);

    // ---------------------------------------------------------------- S5: listener closes with children alive
    say("\n== S5: close the listener (and drop its steer key) while 1,000 accepted children are alive");
    w.clear_seen();
    let i5 = make_intake(&mut w, 500, Ipv4Addr::new(100, 96, 0, 50), BIND_BASE, true, true);
    let d = drive(
        &[Job { src: SRC_INTAKE, dst: Ipv4Addr::new(100, 96, 0, 50), port: DECLARED, n: 1000, tfo: false }],
        &[Acceptor { fd: i5.fd.as_raw_fd(), expected: Some(500) }],
        false,
        2,
    );
    let before = verify(&w, &d.children);
    drop(i5); // listener fd closed; the steer key disappears with it
    thread::sleep(Duration::from_millis(100));
    let after = verify(&w, &d.children);
    // children still usable: server writes, client reads
    let mut echoed = 0;
    for (_, fd) in d.children.iter().take(100) {
        unsafe { libc::write(fd.as_raw_fd(), b"pong".as_ptr() as *const _, 4) };
    }
    thread::sleep(Duration::from_millis(200));
    // the client ends are not index-aligned with the accepted ends: count all 4-byte replies, expect exactly 100
    for fd in d.held.iter() {
        let mut b = [0u8; 8];
        let r = unsafe { libc::recv(fd.as_raw_fd(), b.as_mut_ptr() as *mut _, 8, libc::MSG_DONTWAIT) };
        if r == 4 {
            echoed += 1;
        }
    }
    // closing the clients drives STATE_CB on the children: sock_ops re-reads the tag there
    let c0 = w.counters();
    drop(d.held);
    thread::sleep(Duration::from_millis(500));
    let dc = delta(&c0, &w.counters());
    let still: usize = d.children.iter().filter(|(_, fd)| sk_get(w.clone_fd.as_raw_fd(), fd.as_raw_fd()) == Some(500)).count();
    say(format!("before close: {}", before.show()));
    say(format!("after  close: {}", after.show()));
    say(format!("children still exchanging data after listener close: {echoed}/100; STATE_CB counters after peer FIN: {}", show_cnt(&dc)));
    verdict(
        "S5 children keep their tag after the listener is closed (tag is on the child, not looked up through the listener)",
        before.perfect() && after.perfect() && still == 1000 && echoed == 100,
        format!("{}; state_cb seen_tagged={} now_untagged={}", after.show(), dc[C_STATECB_SEEN_TAGGED], dc[C_STATECB_NOW_UNTAGGED]),
    );
    verdict(
        "S5b sock_ops still reads the tag on a surviving child in a later callback (STATE_CB) after the listener is gone",
        dc[C_STATECB_SEEN_TAGGED] > 0 && dc[C_STATECB_NOW_UNTAGGED] == 0,
        format!("state_cb={} seen_tagged={} now_untagged={}", dc[C_STATECB], dc[C_STATECB_SEEN_TAGGED], dc[C_STATECB_NOW_UNTAGGED]),
    );
    drop(d.children);

    // ---------------------------------------------------------------- S6: re-creation on the same address:port
    say("\n== S6: listener re-creation reusing the same bind address:port (100.96.0.60:20480) and steer key");
    w.clear_seen();
    let a60 = Ipv4Addr::new(100, 96, 0, 60);
    let job = |n| Job { src: SRC_INTAKE, dst: a60, port: DECLARED, n, tfo: false };
    let l1 = make_intake(&mut w, 600, a60, BIND_BASE, true, true);
    let d1 = drive(&[job(500)], &[Acceptor { fd: l1.fd.as_raw_fd(), expected: Some(600) }], false, 2);
    drop(l1);
    let l2 = make_intake(&mut w, 601, a60, BIND_BASE, true, true); // same addr:port, new id
    let d2 = drive(&[job(500)], &[Acceptor { fd: l2.fd.as_raw_fd(), expected: Some(601) }], false, 2);
    let k1 = verify(&w, &d1.children);
    let k2 = verify(&w, &d2.children);
    // in-place re-tag of the live listener
    sk_put(w.clone_fd.as_raw_fd(), l2.fd.as_raw_fd(), 603).unwrap();
    let d3 = drive(&[job(500)], &[Acceptor { fd: l2.fd.as_raw_fd(), expected: Some(603) }], false, 2);
    let k3 = verify(&w, &d3.children);
    let k2again = verify(&w, &d2.children);
    drop(l2);
    // re-creation by an owner that forgot to tag
    let l4 = make_intake(&mut w, 604, a60, BIND_BASE, false, true);
    let d4 = drive(&[job(500)], &[Acceptor { fd: l4.fd.as_raw_fd(), expected: None }], false, 2);
    let k4 = verify(&w, &d4.children);
    // tag removed from a live, tagged listener
    sk_put(w.clone_fd.as_raw_fd(), l4.fd.as_raw_fd(), 605).unwrap();
    sk_del(w.clone_fd.as_raw_fd(), l4.fd.as_raw_fd()).unwrap();
    let d5 = drive(&[job(500)], &[Acceptor { fd: l4.fd.as_raw_fd(), expected: None }], false, 2);
    let k5 = verify(&w, &d5.children);
    let k1again = verify(&w, &d1.children);
    say(format!("L1 (id 600) children, at creation:   {}", k1.show()));
    say(format!("L1 (id 600) children, after L2..L4 exist: {}", k1again.show()));
    say(format!("L2 (id 601, same addr:port) children:  {}", k2.show()));
    say(format!("L2 re-tagged in place 601->603, new children: {}", k3.show()));
    say(format!("L2 children created before the re-tag, rechecked: {}", k2again.show()));
    say(format!("L4 (same addr:port, never tagged) children (expect none): {}", k4.show()));
    say(format!("L4 tagged then entry deleted, children (expect none): {}", k5.show()));
    verdict(
        "S6 a re-created listener on the same address:port gives its children its own tag; the old listener's children keep the old tag",
        k1.perfect() && k1again.perfect() && k2.perfect() && k2again.perfect(),
        format!("old(600) {}; new(601) {}", k1again.show(), k2.show()),
    );
    verdict(
        "S6b tag is on the socket, not the address:port — a re-created UNTAGGED listener yields untagged children; deleting the entry stops inheritance; re-tag in place applies to later children only",
        k3.perfect() && k4.user_ok == k4.n && k4.seen_ok == k4.n && k5.user_ok == k5.n && k5.seen_ok == k5.n && k4.n == 500 && k5.n == 500,
        format!("retag(603) {} ; untagged-recreate {} ; deleted-entry {}", k3.show(), k4.show(), k5.show()),
    );
    drop((d1.children, d2.children, d3.children, d4.children, d5.children));
    drop((d1.held, d2.held, d3.held, d4.held, d5.held));
    drop(l4);

    // ---------------------------------------------------------------- S7: foreign sockets in the cgroup
    say("\n== S7: foreign listeners in the same cgroup and the same map universe");
    w.clear_seen();
    let c0 = w.counters();
    let plain = listen_on(Ipv4Addr::new(127, 0, 0, 1), 30000, false, 4096, false).expect("plain listen");
    let mut kids = Vec::new();
    for _ in 0..2000 {
        let c = connect_from(Ipv4Addr::new(127, 0, 0, 1), Ipv4Addr::new(127, 0, 0, 1), 30000, false).expect("loopback connect");
        let a = unsafe { libc::accept4(plain.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
        assert!(a >= 0, "accept");
        kids.push((None, unsafe { OwnedFd::from_raw_fd(a) }));
        kids.push((None, c));
    }
    let k = verify(&w, &kids.iter().step_by(2).map(|(e, f)| (*e, f.try_clone().unwrap())).collect::<Vec<_>>());
    // wildcard host service model: 0.0.0.0:30001, hit from the peer via the veth
    let wild = listen_on(Ipv4Addr::UNSPECIFIED, 30001, false, 4096, false).expect("wildcard listen");
    let d = drive(
        &[Job { src: SRC_INTAKE, dst: Ipv4Addr::new(10, 77, 20, 1), port: 30001, n: 1000, tfo: false }],
        &[Acceptor { fd: wild.as_raw_fd(), expected: None }],
        false,
        2,
    );
    let kw = verify(&w, &d.children);
    let dc = delta(&c0, &w.counters());
    say(format!("untagged loopback listener children: {}", k.show()));
    say(format!("untagged wildcard host service children (remote): {}", kw.show()));
    say(format!("counters: {}", show_cnt(&dc)));
    verdict(
        "S7 an untagged non-intake listener (loopback and wildcard) in the same cgroup yields untagged children (3,000)",
        k.n == 2000 && k.user_ok == 2000 && k.seen_ok == 2000 && kw.n == 1000 && kw.user_ok == 1000 && kw.seen_ok == 1000,
        format!("{}; {}", k.show(), kw.show()),
    );
    drop((d.children, d.held, kids, plain, wild));

    // ---------------------------------------------------------------- S8: syncookies
    say("\n== S8: net.ipv4.tcp_syncookies=2 in v20-host (children created from the cookie ACK, no request sock)");
    say(host("sysctl -w net.ipv4.tcp_syncookies=2").trim());
    w.clear_seen();
    let sc0 = tcpext("SyncookiesRecv");
    let a70 = Ipv4Addr::new(100, 96, 0, 70);
    let i8 = make_intake(&mut w, 800, a70, BIND_BASE, true, true);
    let d = drive(
        &[Job { src: SRC_INTAKE, dst: a70, port: DECLARED, n: 2000, tfo: false }],
        &[Acceptor { fd: i8.fd.as_raw_fd(), expected: Some(800) }],
        false,
        4,
    );
    let ck = verify(&w, &d.children);
    let sc1 = tcpext("SyncookiesRecv");
    say(format!("TcpExt SyncookiesRecv +{} ; connect_fail={} ; {}", sc1 - sc0, d.connect_fail, ck.show()));
    verdict(
        "S8 children created via the syncookie path carry the tag (2,000)",
        ck.n == 2000 && ck.perfect() && sc1 - sc0 >= 1900,
        format!("SyncookiesRecv +{}; {}", sc1 - sc0, ck.show()),
    );
    say(host("sysctl -w net.ipv4.tcp_syncookies=1").trim());
    drop((d.children, d.held, i8));

    // ---------------------------------------------------------------- S9: TCP Fast Open
    say("\n== S9: TCP Fast Open (child created at SYN time, data in the SYN)");
    say(host("sysctl -w net.ipv4.tcp_fastopen=3").trim());
    w.clear_seen();
    let a71 = Ipv4Addr::new(100, 96, 0, 71);
    let i9 = listen_on(a71, BIND_BASE, false, 65535, true).expect("tfo intake");
    sk_put(w.clone_fd.as_raw_fd(), i9.as_raw_fd(), 900).unwrap();
    w.steer_put(a71, DECLARED, i9.as_raw_fd()).unwrap();
    let f0 = tcpext("TCPFastOpenPassive");
    // prime the client-side cookie cache with one normal TFO attempt, then send data in the SYN
    let prime = drive(&[Job { src: SRC_INTAKE, dst: a71, port: DECLARED, n: 1, tfo: true }], &[Acceptor { fd: i9.as_raw_fd(), expected: Some(900) }], false, 1);
    let d = drive(&[Job { src: SRC_INTAKE, dst: a71, port: DECLARED, n: 1000, tfo: true }], &[Acceptor { fd: i9.as_raw_fd(), expected: Some(900) }], false, 2);
    let f1 = tcpext("TCPFastOpenPassive");
    let mut ch = prime.children;
    ch.extend(d.children);
    let ck = verify(&w, &ch);
    say(format!("TcpExt TCPFastOpenPassive +{} ; {}", f1 - f0, ck.show()));
    verdict(
        "S9 children created by TCP Fast Open carry the tag (data-in-SYN path actually exercised)",
        ck.n == 1001 && ck.perfect() && f1 - f0 >= 900,
        format!("TCPFastOpenPassive +{}; {}", f1 - f0, ck.show()),
    );
    say(host("sysctl -w net.ipv4.tcp_fastopen=1").trim());
    drop((ch, d.held, prime.held, i9));

    // ---------------------------------------------------------------- summary
    let c = w.counters();
    say(format!("\nfinal sock_ops counters (cumulative): {}", show_cnt(&c)));
    say(format!("insert failures into SEEN: {} ; sk==NULL callbacks: {}", c[13], c[12]));
    drop(intakes);
    drop(legc);
    drop(i0);
    let res = RESULTS.lock().unwrap();
    let bad = res.iter().filter(|r| !r.1).count();
    say(format!("\n== SUMMARY: {} verdicts, {} falsified", res.len(), bad));
    for (c, ok, _) in res.iter() {
        say(format!("  [{}] {c}", if *ok { "HOLDS" } else { "FALSIFIED" }));
    }
    bad == 0
}

const C_SHARED_TAGGED: usize = 6;
const C_SHARED_UNTAGGED: usize = 7;
const C_NOCLONE_TAGGED: usize = 8;
const C_ACTIVE: usize = 3;
const C_ACTIVE_TAGGED: usize = 4;
const C_STATECB: usize = 9;
const C_STATECB_SEEN_TAGGED: usize = 10;
const C_STATECB_NOW_UNTAGGED: usize = 11;
