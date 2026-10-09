//! V-26 throwaway probe (GH #295). Kernel facts of the guest-prefix steering.
//! Everything runs in network namespaces prefixed `v26-`; pins live under /sys/fs/bpf/v26-steer.
//! Usage: v26 run <bpf.o>   |   v26 hold <bpf.o> <addr> <port>   (child of `run`)

use std::{
    fs::{self, File},
    io::{self, BufRead, Write},
    net::{Ipv4Addr, SocketAddrV4},
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use aya::{
    maps::{HashMap as AyaHash, MapData, SockHash},
    programs::{links::FdLink, SkLookup, Xdp},
    Ebpf, EbpfLoader, Pod,
};

const PIN_DIR: &str = "/sys/fs/bpf/v26-steer";
const PIN_LINK: &str = "/sys/fs/bpf/v26-steer/link";
const PORT: u16 = 22022; // the "declared port"; Lima's own sshd owns 22, so 22 is not used
const LEGC_PORT: u16 = 22100;
const MARK_LEG_S: u32 = 0x2; // MTLS_LEG_S_DIAL_MARK
const GW: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
const A_STEERED: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 2); // a leased workload_addr
const A_UNASSIGNED: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7); // no listener, only the wildcard service can answer
const A_UNASSIGNED_REMOTE: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 77);

#[repr(C)]
#[derive(Clone, Copy)]
struct SteerKey {
    addr: u32, // byte order of the lookup context (octets in memory order)
    port: u32,
}
unsafe impl Pod for SteerKey {}

fn key(ip: Ipv4Addr, port: u16) -> SteerKey {
    SteerKey { addr: u32::from_ne_bytes(ip.octets()), port: u32::from(port) }
}

// ---------------------------------------------------------------- raw bpf(2)
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
fn obj_get(path: &str) -> Result<OwnedFd, i32> {
    let c = std::ffi::CString::new(path).unwrap();
    let mut a = [0u8; 128];
    put64(&mut a, 0, c.as_ptr() as u64);
    let fd = bpf(7, &mut a)?; // BPF_OBJ_GET
    Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
}
/// BPF_LINK_UPDATE; `old` = Some(fd) adds BPF_F_REPLACE (expected_old).
fn link_update(link: RawFd, new_prog: RawFd, old: Option<RawFd>) -> Result<(), i32> {
    let mut a = [0u8; 128];
    put32(&mut a, 0, link as u32);
    put32(&mut a, 4, new_prog as u32);
    if let Some(o) = old {
        put32(&mut a, 8, 4); // BPF_F_REPLACE (1 << 2)
        put32(&mut a, 12, o as u32);
    }
    bpf(29, &mut a).map(|_| ()) // BPF_LINK_UPDATE
}
fn info(fd: RawFd) -> [u8; 128] {
    let mut buf = [0u8; 128];
    let mut a = [0u8; 128];
    put32(&mut a, 0, fd as u32);
    put32(&mut a, 4, 128);
    put64(&mut a, 8, buf.as_mut_ptr() as u64);
    bpf(15, &mut a).expect("BPF_OBJ_GET_INFO_BY_FD");
    buf
}
fn u32at(b: &[u8], off: usize) -> u32 {
    u32::from_ne_bytes(b[off..off + 4].try_into().unwrap())
}
/// bpf_link_info: type@0 id@4 prog_id@8
fn link_prog_id(link: RawFd) -> u32 {
    u32at(&info(link), 8)
}
fn link_id(link: RawFd) -> u32 {
    u32at(&info(link), 4)
}
/// bpf_prog_info: type@0 id@4
fn prog_id(fd: RawFd) -> u32 {
    u32at(&info(fd), 4)
}

// ---------------------------------------------------------------- namespaces & sockets
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
fn listen_on(ip: Ipv4Addr, port: u16, transparent: bool, backlog: i32) -> io::Result<OwnedFd> {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let o = OwnedFd::from_raw_fd(fd);
        setopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, 1i32);
        if transparent {
            setopt(fd, libc::SOL_IP, libc::IP_TRANSPARENT, 1i32);
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

#[derive(Clone, Copy, PartialEq, Debug)]
enum Outcome {
    Connected,
    Refused,
    Timeout,
    Other(i32),
}
struct Conn {
    outcome: Outcome,
    dur: Duration,
    fd: Option<OwnedFd>,
}
fn connect_once(ip: Ipv4Addr, port: u16, mark: u32, tmo_ms: i32) -> Conn {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0);
        let o = OwnedFd::from_raw_fd(fd);
        if mark != 0 {
            setopt(fd, libc::SOL_SOCKET, libc::SO_MARK, mark);
        }
        let sa = sockaddr(ip, port);
        let t = Instant::now();
        let r = libc::connect(fd, &sa as *const _ as *const _, std::mem::size_of_val(&sa) as u32);
        let mut outcome = if r == 0 {
            Outcome::Connected
        } else {
            let e = io::Error::last_os_error().raw_os_error().unwrap();
            if e == libc::EINPROGRESS {
                let mut p = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
                if libc::poll(&mut p, 1, tmo_ms) == 0 {
                    Outcome::Timeout
                } else {
                    let mut err = 0i32;
                    let mut l = 4u32;
                    libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, &mut err as *mut _ as *mut _, &mut l);
                    match err {
                        0 => Outcome::Connected,
                        libc::ECONNREFUSED => Outcome::Refused,
                        e => Outcome::Other(e),
                    }
                }
            } else if e == libc::ECONNREFUSED {
                Outcome::Refused
            } else {
                Outcome::Other(e)
            }
        };
        let dur = t.elapsed();
        if outcome == Outcome::Timeout {
            outcome = Outcome::Timeout;
        }
        Conn { outcome, dur, fd: if outcome == Outcome::Connected { Some(o) } else { None } }
    }
}

#[derive(Default, Clone)]
struct Stats {
    n: u64,
    connected: u64,
    refused: u64,
    timeout: u64,
    other: u64,
    refused_lat_ms: Vec<f64>,
}
impl Stats {
    fn add(&mut self, c: &Conn) {
        self.n += 1;
        match c.outcome {
            Outcome::Connected => self.connected += 1,
            Outcome::Refused => {
                self.refused += 1;
                self.refused_lat_ms.push(c.dur.as_secs_f64() * 1e3)
            }
            Outcome::Timeout => self.timeout += 1,
            Outcome::Other(_) => self.other += 1,
        }
    }
    fn merge(&mut self, o: &Stats) {
        self.n += o.n;
        self.connected += o.connected;
        self.refused += o.refused;
        self.timeout += o.timeout;
        self.other += o.other;
        self.refused_lat_ms.extend_from_slice(&o.refused_lat_ms);
    }
    fn pct(&self, p: f64) -> f64 {
        let mut v = self.refused_lat_ms.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if v.is_empty() {
            return 0.0;
        }
        v[(((v.len() - 1) as f64) * p) as usize]
    }
    fn max_ms(&self) -> f64 {
        self.refused_lat_ms.iter().cloned().fold(0.0, f64::max)
    }
    fn show(&self) -> String {
        format!(
            "n={} connected={} refused={} timeout={} other={} refused_latency_ms p50={:.3} p99={:.3} max={:.3}",
            self.n,
            self.connected,
            self.refused,
            self.timeout,
            self.other,
            self.pct(0.5),
            self.pct(0.99),
            self.max_ms()
        )
    }
}

/// n connects from namespace `ns` (None = the current host namespace), on a fresh thread.
fn conn_loop(ns: Option<&'static str>, ip: Ipv4Addr, port: u16, mark: u32, n: u64, pace_ms: u64) -> Stats {
    thread::spawn(move || {
        if let Some(ns) = ns {
            setns_by_name(ns).expect("setns peer");
        } else {
            setns_by_name("v26-host").expect("setns host");
        }
        let mut s = Stats::default();
        for _ in 0..n {
            let c = connect_once(ip, port, mark, 1000);
            s.add(&c);
            if pace_ms > 0 {
                thread::sleep(Duration::from_millis(pace_ms));
            }
        }
        s
    })
    .join()
    .unwrap()
}

struct Acceptor {
    count: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    kept: Arc<Mutex<Vec<OwnedFd>>>,
    h: Option<JoinHandle<()>>,
    fd: RawFd,
}
impl Acceptor {
    /// Takes ownership of the listening fd; the accept thread drops (closes) it when stopped.
    fn spawn(lfd: OwnedFd, keep: bool) -> Self {
        let count = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let kept = Arc::new(Mutex::new(Vec::new()));
        let rawfd = lfd.as_raw_fd();
        let (c2, s2, k2) = (count.clone(), stop.clone(), kept.clone());
        let h = thread::spawn(move || {
            let fd = lfd.as_raw_fd();
            while !s2.load(Ordering::Acquire) {
                let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
                unsafe { libc::poll(&mut p, 1, 1) };
                loop {
                    let a = unsafe { libc::accept4(fd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) };
                    if a < 0 {
                        break;
                    }
                    c2.fetch_add(1, Ordering::AcqRel);
                    let o = unsafe { OwnedFd::from_raw_fd(a) };
                    if keep {
                        k2.lock().unwrap().push(o);
                    }
                }
            }
            drop(lfd); // the listener's close, in the accept thread
        });
        Acceptor { count, stop, kept, h: Some(h), fd: rawfd }
    }
    fn count(&self) -> u64 {
        self.count.load(Ordering::Acquire)
    }
    fn fd_hint(&self) -> RawFd {
        self.fd
    }
    /// Stops the thread; returns when the listening socket is closed.
    fn close(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.h.take() {
            h.join().unwrap();
        }
    }
}

// ---------------------------------------------------------------- programs
struct Prog {
    ebpf: Ebpf,
    steer: SockHash<MapData, SteerKey>,
    fd: OwnedFd,
    id: u32,
}
fn load_steer(elf: &str) -> Prog {
    let mut ebpf = EbpfLoader::new().load_file(elf).expect("load elf");
    let p: &mut SkLookup = ebpf.program_mut("v26_steer").unwrap().try_into().unwrap();
    p.load().expect("load v26_steer (kernel verifier)");
    let fd = p.fd().unwrap().as_fd().try_clone_to_owned().unwrap();
    let id = prog_id(fd.as_raw_fd());
    let steer = SockHash::try_from(ebpf.take_map("STEER").unwrap()).unwrap();
    Prog { ebpf, steer, fd, id }
}
struct Raw(RawFd);
impl AsRawFd for Raw {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}
impl Prog {
    fn steer(&mut self, ip: Ipv4Addr, port: u16, l: RawFd) -> Result<(), i32> {
        self.steer.insert(key(ip, port), Raw(l), 0).map_err(|e| match e {
            aya::maps::MapError::SyscallError(s) => s.io_error.raw_os_error().unwrap_or(-1),
            _ => -2,
        })
    }
    fn has(&self, ip: Ipv4Addr, port: u16) -> bool {
        let k = key(ip, port);
        self.steer.keys().filter_map(|k| k.ok()).any(|x| x.addr == k.addr && x.port == k.port)
    }
    fn n_keys(&self) -> usize {
        self.steer.keys().filter(|k| k.is_ok()).count()
    }
    /// first attach + pin (`bpf_link_create` to the namespace, then the pin).
    fn attach_pin(&mut self, netns: &File, pin: &str) -> OwnedFd {
        let p: &mut SkLookup = self.ebpf.program_mut("v26_steer").unwrap().try_into().unwrap();
        let id = p.attach(netns).expect("attach");
        let link = p.take_link(id).unwrap();
        let fdl: FdLink = link.into();
        fdl.pin(pin).expect("pin");
        obj_get(pin).expect("reopen pin")
    }
}

// ---------------------------------------------------------------- reporting
static RESULTS: Mutex<Vec<(String, String, String)>> = Mutex::new(Vec::new());
fn say(s: impl AsRef<str>) {
    println!("{}", s.as_ref());
    let _ = io::stdout().flush();
}
fn verdict(claim: &str, ok: bool, detail: impl AsRef<str>) {
    let v = if ok { "OK" } else { "FAIL" };
    say(format!("CHECK [{v}] {claim} :: {}", detail.as_ref()));
    RESULTS.lock().unwrap().push((claim.into(), v.into(), detail.as_ref().into()));
}
fn sh(cmd: &str) -> String {
    let o = Command::new("bash").arg("-c").arg(cmd).output().expect("sh");
    let s = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    say(format!("$ {cmd}\n{}", s.trim_end()));
    s
}
fn sh_q(cmd: &str) -> bool {
    Command::new("bash").arg("-c").arg(cmd).status().map(|s| s.success()).unwrap_or(false)
}
fn ifindex(name: &str) -> u32 {
    let c = std::ffi::CString::new(name).unwrap();
    unsafe { libc::if_nametoindex(c.as_ptr()) }
}
fn settle() {
    thread::sleep(Duration::from_millis(150));
}

fn nft_rules_full() {
    sh_q("nft delete table ip v26-mtls 2>/dev/null");
    let script = format!(
        "nft add table ip v26-mtls && \
         nft add chain ip v26-mtls prerouting '{{ type filter hook prerouting priority mangle; }}' && \
         nft add chain ip v26-mtls output '{{ type route hook output priority mangle; }}' && \
         nft add rule ip v26-mtls prerouting ct state established,related accept && \
         nft add rule ip v26-mtls prerouting meta mark {MARK_LEG_S} accept && \
         nft add rule ip v26-mtls prerouting ip daddr {A_STEERED} tcp dport {PORT} meta mark set 1 tproxy ip to 127.0.0.1:{LEGC_PORT} && \
         nft add rule ip v26-mtls prerouting ip daddr 100.95.0.0/16 drop && \
         nft add rule ip v26-mtls output ct state established,related accept && \
         nft add rule ip v26-mtls output meta mark {MARK_LEG_S} accept && \
         nft add rule ip v26-mtls output ip daddr 100.95.0.0/16 meta l4proto tcp reject with tcp reset && \
         nft add rule ip v26-mtls output ip daddr 100.95.0.0/16 reject"
    );
    assert!(sh_q(&script), "nft install");
}
fn nft_tproxy_only() {
    sh_q("nft delete table ip v26-mtls 2>/dev/null");
    let script = format!(
        "nft add table ip v26-mtls && \
         nft add chain ip v26-mtls prerouting '{{ type filter hook prerouting priority mangle; }}' && \
         nft add rule ip v26-mtls prerouting ip daddr {A_STEERED} tcp dport {PORT} meta mark set 1 tproxy ip to 127.0.0.1:{LEGC_PORT}"
    );
    assert!(sh_q(&script), "nft install");
}
fn nft_off() {
    sh_q("nft delete table ip v26-mtls 2>/dev/null");
}

// ---------------------------------------------------------------- hold (child)
fn hold(elf: &str, ip: Ipv4Addr, port: u16) {
    setns_by_name("v26-host").unwrap();
    // Boot sequence of D8a-LOAD-SWAP against the pinned link: open it, load, swap.
    let link = obj_get(PIN_LINK).expect("open pinned link");
    let mut p = load_steer(elf);
    let before = link_prog_id(link.as_raw_fd());
    link_update(link.as_raw_fd(), p.fd.as_raw_fd(), None).expect("swap");
    let after = link_prog_id(link.as_raw_fd());
    let l = listen_on(ip, 0, false, 4096).expect("listen");
    p.steer(ip, port, l.as_raw_fd()).expect("steer");
    let acc = Acceptor::spawn(l, false);
    println!("ready link_prog_before={before} link_prog_after={after} new_prog={}", p.id);
    let _ = io::stdout().flush();
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let _ = line;
        println!("accepts={}", acc.count());
        let _ = io::stdout().flush();
    }
    // stdin EOF = graceful shutdown: process exits, listener and descriptors close.
}

// ---------------------------------------------------------------- scenario
fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("hold") => hold(&args[2], args[3].parse().unwrap(), args[4].parse().unwrap()),
        Some("run") => scenario(&args[2], args.get(3).map(String::as_str)),
        _ => eprintln!("usage: v26 run <bpf.o> [only-phase] | v26 hold <bpf.o> <addr> <port>"),
    }
}

fn raise_nofile() {
    let r = libc::rlimit { rlim_cur: 1_000_000, rlim_max: 1_000_000 };
    let rc = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &r) };
    assert_eq!(rc, 0, "setrlimit NOFILE: {}", io::Error::last_os_error());
}

fn scenario(elf: &str, only: Option<&str>) {
    setns_by_name("v26-host").expect("setns v26-host");
    raise_nofile();
    let want = |p: &str| only.is_none() || only == Some(p);
    let vh_idx = ifindex("v26-h");
    say(format!("== environment: lo ifindex={} v26-h ifindex={vh_idx}", ifindex("lo")));
    sh("uname -r; uname -m; bpftool version | head -2");
    sh("ip route show table local | grep 100.95; ip rule; ip route show table 100");
    let _ = fs::remove_file(PIN_LINK);
    let _ = fs::create_dir(PIN_DIR);
    let netns = File::open("/var/run/netns/v26-host").unwrap();

    // Shared fixtures: wildcard host service ("sshd") on 0.0.0.0:PORT (TCP and UDP), and leg-C.
    let mut wild = Acceptor::spawn(listen_on(Ipv4Addr::UNSPECIFIED, PORT, false, 4096).unwrap(), false);
    let udp_hits = Arc::new(AtomicU64::new(0));
    let udp_sock = unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_NONBLOCK, 0);
        let sa = sockaddr(Ipv4Addr::UNSPECIFIED, PORT);
        assert_eq!(libc::bind(fd, &sa as *const _ as *const _, 16), 0);
        OwnedFd::from_raw_fd(fd)
    };
    {
        let (h, fd) = (udp_hits.clone(), udp_sock.as_raw_fd());
        thread::spawn(move || loop {
            let mut buf = [0u8; 64];
            let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, 64, 0) };
            if n > 0 {
                h.fetch_add(1, Ordering::AcqRel);
            } else {
                thread::sleep(Duration::from_millis(1));
            }
        });
    }
    let mut legc = Acceptor::spawn(listen_on(Ipv4Addr::new(127, 0, 0, 1), LEGC_PORT, true, 4096).unwrap(), false);

    let mut w_base;
    macro_rules! wild_delta {
        () => {{
            settle();
            let d = wild.count() - w_base;
            w_base = wild.count();
            d
        }};
    }
    w_base = 0;

    // ============================================================== P0 positive control: no program
    if want("p0") {
        say("\n######## P0 control: no sk_lookup program attached ########");
        // F-1: can an intake listener bound exactly to workload_addr:PORT coexist with a wildcard host service on PORT?
        let e = listen_on(A_STEERED, PORT, false, 16);
        let e_ip = format!("{:?}", e.as_ref().err().map(|e| e.raw_os_error()));
        let e_fb = {
            // also with IP_FREEBIND / SO_REUSEPORT on the intake side only
            unsafe {
                let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
                setopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, 1i32);
                setopt(fd, libc::SOL_SOCKET, libc::SO_REUSEPORT, 1i32);
                setopt(fd, libc::SOL_IP, libc::IP_FREEBIND, 1i32);
                let sa = sockaddr(A_STEERED, PORT);
                let r = libc::bind(fd, &sa as *const _ as *const _, 16);
                let en = if r != 0 { io::Error::last_os_error().raw_os_error() } else { None };
                libc::close(fd);
                en
            }
        };
        say(format!("F-1: bind({A_STEERED}:{PORT}) with a wildcard 0.0.0.0:{PORT} listener present -> {e_ip} (EADDRINUSE={}); with REUSEADDR+REUSEPORT+FREEBIND on the intake side only -> {e_fb:?}", libc::EADDRINUSE));
        drop(e);
        let mut i1 = Acceptor::spawn(listen_on(A_STEERED, 0, false, 4096).unwrap(), false);
        let s_i1 = Stats::default();
        let s_w = conn_loop(None, A_UNASSIGNED, PORT, 0, 200, 0);
        let wd = wild_delta!();
        let s_r = conn_loop(Some("v26-peer"), A_UNASSIGNED, PORT, 0, 200, 0);
        let wd_r = wild_delta!();
        let _ = &s_i1;
        say(format!("local -> {A_UNASSIGNED}:{PORT}: {} ; wildcard accepts={wd}", s_w.show()));
        say(format!("remote -> {A_UNASSIGNED}:{PORT}: {} ; wildcard accepts={wd_r}", s_r.show()));
        verdict(
            "P0 control: with no program, prefix traffic reaches host sockets (test can fail)",
            wd == 200 && wd_r == 200,
            format!("wildcard local {wd}/200, wildcard remote {wd_r}/200"),
        );
        verdict("F-1 an intake listener bound exactly to workload_addr:PORT cannot coexist with a wildcard host service on PORT", e_ip.contains("98") && e_fb == Some(libc::EADDRINUSE), format!("bind -> {e_ip}; with reuse/freebind on the intake side only -> {e_fb:?}"));
        // UDP control
        let udp_probe = |ip: Ipv4Addr| -> (bool, i32) {
            unsafe {
                let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
                let sa = sockaddr(ip, PORT);
                libc::connect(fd, &sa as *const _ as *const _, 16);
                libc::send(fd, b"x".as_ptr() as *const _, 1, 0);
                thread::sleep(Duration::from_millis(30));
                let r = libc::send(fd, b"y".as_ptr() as *const _, 1, 0);
                let e = if r < 0 { io::Error::last_os_error().raw_os_error().unwrap() } else { 0 };
                libc::close(fd);
                (r >= 0, e)
            }
        };
        let u0 = udp_hits.load(Ordering::Acquire);
        let _ = udp_probe(A_UNASSIGNED);
        settle();
        verdict("P0 control: UDP to prefix reaches wildcard UDP service", udp_hits.load(Ordering::Acquire) > u0, format!("wildcard udp hits +{}", udp_hits.load(Ordering::Acquire) - u0));
        i1.close();
        // control: an exact-bound listener is what natural lookup picks when no wildcard holds the port
        wild.close();
        let mut ie = Acceptor::spawn(listen_on(A_STEERED, PORT, false, 4096).unwrap(), false);
        let s = conn_loop(None, A_STEERED, PORT, 0, 200, 0);
        settle();
        verdict("P0 control: with no program, a host-local connect reaches the exact-bound listener", ie.count() == 200 && s.connected == 200, format!("exact-bound accepts {}/200", ie.count()));
        ie.close();
        wild = Acceptor::spawn(listen_on(Ipv4Addr::UNSPECIFIED, PORT, false, 4096).unwrap(), false);
        w_base = 0;
    }

    // ============================================================== P1 K-L5 / K-L2 via the counting program, and the non-atomic control
    if want("p1") {
        say("\n######## P1 ifindex (K-L5) and TPROXY bypass (K-L2) with the counting program (always SK_PASS) ########");
        let mut ebpf = EbpfLoader::new().load_file(elf).unwrap();
        let p: &mut SkLookup = ebpf.program_mut("v26_ifx").unwrap().try_into().unwrap();
        p.load().expect("load ifx");
        let lid = p.attach(&netns).expect("attach ifx");
        let read = |ebpf: &Ebpf| -> std::collections::BTreeMap<(u32, u32), u64> {
            let m: AyaHash<_, u64, u64> = AyaHash::try_from(ebpf.map("IFX").unwrap()).unwrap();
            m.iter().filter_map(|x| x.ok()).map(|(k, v)| (((k >> 32) as u32, (k & 0xffff_ffff) as u32), v)).collect()
        };
        let get = |m: &std::collections::BTreeMap<(u32, u32), u64>, i: u32| *m.get(&(i, u32::from(PORT))).unwrap_or(&0);
        let mut i1 = Acceptor::spawn(listen_on(A_STEERED, 0, false, 4096).unwrap(), false);
        let b = read(&ebpf);
        let s_l = conn_loop(None, A_UNASSIGNED, PORT, 0, 1000, 0);
        let s_lm = conn_loop(None, A_UNASSIGNED, PORT, MARK_LEG_S, 1000, 0);
        let m1 = read(&ebpf);
        let s_r = conn_loop(Some("v26-peer"), A_UNASSIGNED, PORT, 0, 1000, 0);
        let m2 = read(&ebpf);
        say(format!("host-local connects: {} / marked {}", s_l.show(), s_lm.show()));
        say(format!("remote connects: {}", s_r.show()));
        say(format!("IFX counters (ifindex,port)->lookups after local+marked: {:?}", m1));
        say(format!("IFX counters after remote: {:?}", m2));
        let d_lo = get(&m1, 1) - get(&b, 1);
        let d_vh_after_local = get(&m1, vh_idx) - get(&b, vh_idx);
        let d_vh = get(&m2, vh_idx) - get(&m1, vh_idx);
        verdict(
            "K-L5 (ix) ingress_ifindex is the loopback index for host-local connects, the device index for remote ones",
            d_lo == 2000 && d_vh_after_local == 0 && d_vh == 1000,
            format!("lookups with ifindex=1(lo): +{d_lo} for 2000 local connects (1000 unmarked + 1000 marked); ifindex={vh_idx}(v26-h): +{d_vh_after_local} during local, +{d_vh} for 1000 remote connects"),
        );
        // K-L2: TPROXY bypass
        nft_tproxy_only();
        sh("nft list ruleset");
        let c0 = legc.count();
        let m3_before = read(&ebpf);
        let s_t = conn_loop(Some("v26-peer"), A_STEERED, PORT, 0, 1000, 0);
        settle();
        let m3 = read(&ebpf);
        let d_vh_t = get(&m3, vh_idx) - get(&m3_before, vh_idx);
        say(format!("remote -> {A_STEERED}:{PORT} with nft tproxy: {} ; leg-C accepts +{} ; v26-h lookups +{d_vh_t}", s_t.show(), legc.count() - c0));
        let ok_by = legc.count() - c0 == 1000 && d_vh_t == 0;
        nft_off();
        let i1_before = i1.count();
        let s_u = conn_loop(Some("v26-peer"), A_STEERED, PORT, 0, 200, 0);
        settle();
        let m4 = read(&ebpf);
        let d_vh_u = get(&m4, vh_idx) - get(&m3, vh_idx);
        say(format!("positive control, tproxy rule removed: {} ; v26-h lookups +{d_vh_u} ; exact listener accepts +{}", s_u.show(), i1.count() - i1_before));
        verdict(
            "K-L2 (v): prerouting TPROXY-assigned packets never reach sk_lookup (counter does not move); same packets reach it once the rule is gone",
            ok_by && d_vh_u == 200,
            format!("with rule: leg-C {}/1000, sk_lookup runs +{d_vh_t}; without rule: sk_lookup runs +{d_vh_u}/200", legc.count() - c0),
        );
        i1.close();
        wild_delta!();

        // Non-atomic control: destructive swap (detach, then attach) under a connect storm. Shows the storm can see a hole.
        say("-- control: NON-atomic replace (detach, attach new) of an unpinned link under a connect storm --");
        let mut a = load_steer(elf);
        let mut b2 = load_steer(elf);
        let mut i1 = Acceptor::spawn(listen_on(A_STEERED, 0, false, 4096).unwrap(), false);
        let stop = Arc::new(AtomicBool::new(false));
        let ths: Vec<_> = (0..4)
            .map(|_| {
                let st = stop.clone();
                thread::spawn(move || {
                    setns_by_name("v26-host").unwrap();
                    let mut s = Stats::default();
                    while !st.load(Ordering::Acquire) {
                        s.add(&connect_once(A_STEERED, PORT, MARK_LEG_S, 500));
                    }
                    s
                })
            })
            .collect();
        drop(p_detach(&mut ebpf, lid)); // remove the ifx link before the control
        let pa: &mut SkLookup = a.ebpf.program_mut("v26_steer").unwrap().try_into().unwrap();
        let mut cur = pa.attach(&netns).unwrap();
        let t0 = Instant::now();
        for i in 0..150 {
            let (from, to) = if i % 2 == 0 { (&mut a, &mut b2) } else { (&mut b2, &mut a) };
            let pf: &mut SkLookup = from.ebpf.program_mut("v26_steer").unwrap().try_into().unwrap();
            pf.detach(cur).unwrap();
            let pt: &mut SkLookup = to.ebpf.program_mut("v26_steer").unwrap().try_into().unwrap();
            cur = pt.attach(&netns).unwrap();
        }
        let dur = t0.elapsed();
        stop.store(true, Ordering::Release);
        let mut tot = Stats::default();
        for t in ths {
            tot.merge(&t.join().unwrap());
        }
        settle();
        say(format!("storm over 150 detach+attach cycles in {:?}: {} ; intake (exact-bound, unsteered) accepts={} wildcard accepts={}", dur, tot.show(), i1.count(), wild.count() - w_base));
        let leaked = i1.count() + (wild.count() - w_base);
        verdict(
            "control: a non-atomic replace is observable by the same connect storm (the swap test has teeth)",
            leaked > 0,
            format!("{leaked} connections reached host sockets during detach/attach windows"),
        );
        w_base = wild.count();
        i1.close();
    }

    // ============================================================== P2 steering decisions on a pinned link
    let mut a_prog = load_steer(elf);
    let link_fd: OwnedFd;
    if want("p2") || want("p3") || want("p4") {
        say("\n######## P2 first attach + pin of the decision program ########");
        let _ = fs::remove_file(PIN_LINK);
        link_fd = a_prog.attach_pin(&netns, PIN_LINK);
        sh(&format!("bpftool link show pinned {PIN_LINK}"));
        verdict("first attach + pin of the sk_lookup link", true, format!("link id={} prog id={}", link_id(link_fd.as_raw_fd()), link_prog_id(link_fd.as_raw_fd())));
    } else {
        link_fd = unsafe { OwnedFd::from_raw_fd(libc::dup(0)) };
    }

    if want("p2") {
        say("\n######## P2 (i)(ii)(iii)(iv)(v)(vii): decisions ########");
        // intake listener: bound exactly to the intake, NOT yet steered; wildcard 'sshd' also present
        let mut i1 = Acceptor::spawn(listen_on(A_STEERED, 0, false, 4096).unwrap(), true);

        // (i) no entry
        let u = conn_loop(None, A_STEERED, PORT, 0, 1000, 0);
        let m = conn_loop(None, A_STEERED, PORT, MARK_LEG_S, 1000, 0);
        let u2 = conn_loop(None, A_UNASSIGNED, PORT, 0, 1000, 0);
        let wd = wild_delta!();
        say(format!("(i) no entry; exact-bound unsteered listener present; nft absent\n  unmarked -> {A_STEERED}: {}\n  marked   -> {A_STEERED}: {}\n  unmarked -> {A_UNASSIGNED}: {}\n  intake accepts={} wildcard accepts={wd}", u.show(), m.show(), u2.show(), i1.count()));
        verdict(
            "K-L1/(i)+(iv) no entry: marked and unmarked host-local connects reset (< 5 ms), before the exact-bound listener and the wildcard; nft table absent",
            u.refused == 1000 && m.refused == 1000 && u2.refused == 1000 && i1.count() == 0 && wd == 0 && u.max_ms().max(m.max_ms()) < 5.0,
            format!("refused {}+{}+{}/3000; max reset latency {:.3} ms (p99 {:.3}); listener accepts {}; wildcard accepts {wd}", u.refused, m.refused, u2.refused, u.max_ms().max(m.max_ms()).max(u2.max_ms()), u.pct(0.99).max(m.pct(0.99)), i1.count()),
        );
        // UDP: dropped, port unreachable
        let u0 = udp_hits.load(Ordering::Acquire);
        let (_, e2) = unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
            let sa = sockaddr(A_UNASSIGNED, PORT);
            libc::connect(fd, &sa as *const _ as *const _, 16);
            libc::send(fd, b"x".as_ptr() as *const _, 1, 0);
            thread::sleep(Duration::from_millis(30));
            let r = libc::send(fd, b"y".as_ptr() as *const _, 1, 0);
            let e = if r < 0 { io::Error::last_os_error().raw_os_error().unwrap() } else { 0 };
            libc::close(fd);
            (r, e)
        };
        verdict("K-L1 UDP to the prefix: dropped, sender sees port-unreachable (ECONNREFUSED)", udp_hits.load(Ordering::Acquire) == u0 && e2 == libc::ECONNREFUSED, format!("wildcard udp hits +{}, second send errno={e2}", udp_hits.load(Ordering::Acquire) - u0));

        // with the production-shaped constant nft rules present: a marked connect (exempt) reaches the program
        nft_rules_full();
        sh("nft list ruleset");
        let mm = conn_loop(None, A_STEERED, PORT, MARK_LEG_S, 300, 0);
        let uu = conn_loop(None, A_STEERED, PORT, 0, 300, 0);
        let wd = wild_delta!();
        say(format!("   constant nft rules present, no entry: marked {} ; unmarked (rejected by nft output rule) {} ; wildcard accepts={wd}", mm.show(), uu.show()));
        verdict("(i) with the constant nft rules present, marked connects are decided by the program (reset), unmarked by nft; nothing reaches the wildcard", mm.refused == 300 && uu.refused == 300 && wd == 0 && i1.count() == 0, format!("marked refused {}/300, unmarked refused {}/300", mm.refused, uu.refused));

        // (ii) entry
        nft_off();
        let mut pr = a_prog;
        pr.steer(A_STEERED, PORT, last_fd(&i1)).expect("steer");
        let mut i1_ref = &mut i1;
        let _ = &mut i1_ref;
        say(format!("   entry inserted; map keys now: {}", pr.n_keys()));
        let b = i1.count();
        let u = conn_loop(None, A_STEERED, PORT, 0, 1000, 0);
        let m = conn_loop(None, A_STEERED, PORT, MARK_LEG_S, 1000, 0);
        settle();
        let wd = wild_delta!();
        say(format!("(ii) entry present\n  unmarked: {}\n  marked: {}\n  intake accepts +{} wildcard +{wd}", u.show(), m.show(), i1.count() - b));
        verdict("K-L1/(ii) entry present: host-local connects (marked and unmarked) are assigned to the intake listener", u.connected == 1000 && m.connected == 1000 && i1.count() - b == 2000 && wd == 0, format!("connected {}+{}/2000, intake accepts +{}, wildcard +{wd}", u.connected, m.connected, i1.count() - b));

        // with nft present + entry + marked (production-shaped)
        nft_rules_full();
        let b = i1.count();
        let mm = conn_loop(None, A_STEERED, PORT, MARK_LEG_S, 300, 0);
        settle();
        verdict("(ii) with the constant nft rules present, a marked connect reaches the steered intake listener", mm.connected == 300 && i1.count() - b == 300, format!("{}; intake +{}", mm.show(), i1.count() - b));
        nft_off();

        // assignment is the program's, not the natural lookup: steer a listener bound elsewhere
        let mut other = Acceptor::spawn(listen_on(Ipv4Addr::new(127, 0, 0, 1), 34567, false, 4096).unwrap(), false);
        let res = pr.steer(Ipv4Addr::new(100, 95, 0, 3), PORT, last_fd(&other));
        let s = conn_loop(None, Ipv4Addr::new(100, 95, 0, 3), PORT, 0, 200, 0);
        settle();
        say(format!("   edge: entry for 100.95.0.3:{PORT} -> listener bound 127.0.0.1:34567 (steer result {res:?}): {} ; that listener accepts={}", s.show(), other.count()));
        verdict("K-L1 edge: the assigned socket is the one the program names, not the one natural lookup would pick", res.is_ok() && other.count() == 200, format!("steer={res:?}, accepts={}/200", other.count()));
        other.close();
        let _ = pr.n_keys();

        // K-L5 behaviour: remote with entry present
        let b = i1.count();
        let s = conn_loop(Some("v26-peer"), A_STEERED, PORT, 0, 1000, 0);
        let wd = wild_delta!();
        say(format!("   entry present, REMOTE connect to the steered address, nft absent: {} ; intake +{} wildcard +{wd}", s.show(), i1.count() - b));
        verdict("K-L5/R5-2 remote arrival for a steered address is dropped by the program, never assigned", s.refused == 1000 && i1.count() == b && wd == 0, format!("refused {}/1000, intake +0, wildcard +{wd}", s.refused));

        // (v) leg-C TPROXY with entry present and program attached
        nft_tproxy_only();
        let c0 = legc.count();
        let b = i1.count();
        let s = conn_loop(Some("v26-peer"), A_STEERED, PORT, 0, 1000, 0);
        settle();
        let wd = wild_delta!();
        say(format!("(v) program attached AND entry present, nft tproxy on: {} ; leg-C +{} intake +{} wildcard +{wd}", s.show(), legc.count() - c0, i1.count() - b));
        verdict("K-L2/(v) leg-C TPROXY still receives remote connects to the declared port (program and entry present)", s.connected == 1000 && legc.count() - c0 == 1000 && i1.count() == b && wd == 0, format!("leg-C +{}/1000", legc.count() - c0));
        nft_off();

        // (vii) remote to unassigned prefix address, nft deleted
        let s = conn_loop(Some("v26-peer"), A_UNASSIGNED_REMOTE, PORT, 0, 1000, 0);
        let wd = wild_delta!();
        verdict("(vii) remote connect to an unassigned prefix address with the nft rules deleted is dropped by the program", s.refused == 1000 && wd == 0, format!("{} ; wildcard +{wd}", s.show()));

        // (iii) close: accepted connections persist, next connects are reset, key gone
        let c1 = connect_once(A_STEERED, PORT, 0, 1000);
        settle();
        assert_eq!(c1.outcome, Outcome::Connected);
        let had_key = pr.has(A_STEERED, PORT);
        let keys_before = pr.n_keys();
        i1.close();
        let has_key_after = pr.has(A_STEERED, PORT);
        let keys_after = pr.n_keys();
        let b = wild.count();
        let s = conn_loop(None, A_STEERED, PORT, 0, 1000, 0);
        let sm = conn_loop(None, A_STEERED, PORT, MARK_LEG_S, 1000, 0);
        let wd = wild.count() - b;
        w_base = wild.count();
        // accepted child continues
        let mut transfer = false;
        {
            let kept = i1.kept.lock().unwrap();
            // the accepted child of c1 is the last kept fd of the closed listener
            if let (Some(srv), Some(cli)) = (kept.last(), c1.fd.as_ref()) {
                unsafe {
                    let w = libc::write(cli.as_raw_fd(), b"ping".as_ptr() as *const _, 4);
                    thread::sleep(Duration::from_millis(30));
                    let mut buf = [0u8; 8];
                    let r = libc::read(srv.as_raw_fd(), buf.as_mut_ptr() as *mut _, 8);
                    let w2 = libc::write(srv.as_raw_fd(), b"pong".as_ptr() as *const _, 4);
                    thread::sleep(Duration::from_millis(30));
                    let mut buf2 = [0u8; 8];
                    let r2 = libc::read(cli.as_raw_fd(), buf2.as_mut_ptr() as *mut _, 8);
                    transfer = w == 4 && r == 4 && &buf[..4] == b"ping" && w2 == 4 && r2 == 4 && &buf2[..4] == b"pong";
                }
            }
        }
        say(format!("(iii) listener closed: key present before={had_key} after={has_key_after}; keys {keys_before}->{keys_after}\n  unmarked: {}\n  marked: {}\n  accepted child still transfers: {transfer} ; wildcard +{wd}", s.show(), sm.show()));
        verdict("K-L3/(iii) closing the listener removes its entry in the same step; the next connects are reset; already-accepted connections keep transferring",
            had_key && !has_key_after && keys_after + 1 == keys_before && s.refused == 1000 && sm.refused == 1000 && wd == 0 && transfer,
            format!("key present {had_key} -> {has_key_after}, refused {}+{}/2000, transfer={transfer}, wildcard +{wd}", s.refused, sm.refused));

        // tight close/connect race: no SYN after close() returns is ever assigned to the closed listener
        let mut bad = 0u32;
        let mut ok_conn = 0u32;
        let mut ent_after = 0u32;
        for _ in 0..1000 {
            let l = listen_on(A_STEERED, 0, false, 16).unwrap();
            pr.steer(A_STEERED, PORT, l.as_raw_fd()).expect("steer in cycle");
            if connect_once(A_STEERED, PORT, 0, 1000).outcome == Outcome::Connected {
                ok_conn += 1;
            }
            drop(l);
            if pr.has(A_STEERED, PORT) {
                ent_after += 1;
            }
            if connect_once(A_STEERED, PORT, 0, 1000).outcome != Outcome::Refused {
                bad += 1;
            }
        }
        settle();
        verdict("K-L3 race: 1000 steer/connect/close/connect cycles: the connect after close() is always reset; the key is gone at once", ok_conn == 1000 && bad == 0 && ent_after == 0 && wild.count() == w_base, format!("connect-before-close ok {ok_conn}/1000, post-close not-refused {bad}, key lingering {ent_after}, wildcard +{}", wild.count() - w_base));

        // Part B: an exact-bound intake listener (possible only while no wildcard holds PORT, F-1)
        wild.close();
        let mut ie = Acceptor::spawn(listen_on(A_STEERED, PORT, false, 4096).unwrap(), false);
        let s1 = conn_loop(None, A_STEERED, PORT, 0, 1000, 0);
        let s2 = conn_loop(None, A_STEERED, PORT, MARK_LEG_S, 1000, 0);
        settle();
        say(format!("(i-B) exact-bound listener {A_STEERED}:{PORT}, no entry, no wildcard: unmarked {} / marked {} ; listener accepts={}", s1.show(), s2.show(), ie.count()));
        verdict("K-L1/(i) the program decides before the listener lookup: an exact-bound, unsteered listener is never reached", s1.refused == 1000 && s2.refused == 1000 && ie.count() == 0, format!("refused {}+{}/2000, listener accepts {}", s1.refused, s2.refused, ie.count()));
        pr.steer(A_STEERED, PORT, ie.fd_hint()).expect("steer exact");
        let s3 = conn_loop(None, A_STEERED, PORT, 0, 1000, 0);
        settle();
        verdict("K-L1/(ii) with the entry, the exact-bound listener (the design's precondition) is assigned", s3.connected == 1000 && ie.count() == 1000, format!("connected {}/1000, accepts {}", s3.connected, ie.count()));
        ie.close();
        wild = Acceptor::spawn(listen_on(Ipv4Addr::UNSPECIFIED, PORT, false, 4096).unwrap(), false);
        w_base = 0;
        a_prog = pr;
    }

    // ============================================================== P3 pinned link without a process; swap (K-L9)
    if want("p3") {
        say("\n######## P3 K-L4: a pinned link keeps program and map deciding with no process ########");
        // the child is a separate process: boot-sequence (open pin, load, swap), steer, serve
        let exe = std::env::current_exe().unwrap();
        let mut i1 = Acceptor::spawn(listen_on(A_UNASSIGNED, PORT + 1, false, 16).unwrap(), false); // unrelated, keeps fixture symmetrical
        i1.close();
        for (mode, how) in [("SIGKILL", true), ("graceful exit", false)] {
            let mut child = Command::new(&exe)
                .args(["hold", elf, &A_STEERED.to_string(), &PORT.to_string()])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut out = io::BufReader::new(child.stdout.take().unwrap());
            let mut line = String::new();
            out.read_line(&mut line).unwrap();
            say(format!("[{mode}] child: {}", line.trim()));
            let u = conn_loop(None, A_STEERED, PORT, 0, 1000, 0);
            let m = conn_loop(None, A_STEERED, PORT, MARK_LEG_S, 1000, 0);
            settle();
            let mut stdin = child.stdin.take().unwrap();
            writeln!(stdin, "q").unwrap();
            line.clear();
            out.read_line(&mut line).unwrap();
            say(format!("[{mode}] child serving: unmarked {} / marked {} ; {}", u.show(), m.show(), line.trim()));
            let served = u.connected == 1000 && m.connected == 1000 && line.trim() == "accepts=2000";
            let t0 = Instant::now();
            if how {
                child.kill().unwrap();
            } else {
                drop(stdin);
            }
            child.wait().unwrap();
            let gone = t0.elapsed();
            let lfd = obj_get(PIN_LINK).expect("pin still there");
            let pid = link_prog_id(lfd.as_raw_fd());
            say(format!("[{mode}] owner is gone ({gone:?}); pinned link:"));
            sh(&format!("bpftool link show pinned {PIN_LINK}"));
            let pinfo = sh(&format!("bpftool prog show id {pid}"));
            let map_id: Option<String> = pinfo.split("map_ids ").nth(1).map(|s| s.split(|c: char| !c.is_ascii_digit()).next().unwrap().to_string());
            let mut map_empty = false;
            if let Some(mid) = &map_id {
                sh(&format!("bpftool map show id {mid}"));
                let d = sh(&format!("bpftool map dump id {mid}"));
                map_empty = d.contains("Found 0 elements") || d.trim().is_empty();
            }
            let u = conn_loop(None, A_STEERED, PORT, 0, 1000, 0);
            let m = conn_loop(None, A_STEERED, PORT, MARK_LEG_S, 1000, 0);
            let r = conn_loop(Some("v26-peer"), A_STEERED, PORT, 0, 1000, 0);
            let r2 = conn_loop(Some("v26-peer"), A_UNASSIGNED_REMOTE, PORT, 0, 1000, 0);
            let wd = wild_delta!();
            say(format!("[{mode}] no owner: unmarked {} ; marked {} ; remote {} ; remote-unassigned {} ; wildcard +{wd}", u.show(), m.show(), r.show(), r2.show()));
            verdict(
                &format!("K-L4/(vi) owner {mode}: pinned link keeps its program deciding, its map holds no live socket, every connect is reset and none reaches the wildcard"),
                served && u.refused == 1000 && m.refused == 1000 && r.refused == 1000 && r2.refused == 1000 && wd == 0 && map_empty && pid != 0,
                format!("served-before={served}; link prog id={pid} map_id={map_id:?} map_empty={map_empty}; refused {}+{}+{}+{}/4000; wildcard +{wd}", u.refused, m.refused, r.refused, r2.refused),
            );
        }
    }

    if want("p4") {
        say("\n######## P4 K-L9: BPF_LINK_UPDATE atomicity and failure behaviour ########");
        let lfd = obj_get(PIN_LINK).expect("pinned link");
        let l = lfd.as_raw_fd();
        let mut i1 = Acceptor::spawn(listen_on(A_STEERED, 0, false, 4096).unwrap(), false); // exact-bound, unsteered: any hole shows up here
        let id0 = link_prog_id(l);
        say(format!("link prog id before: {id0}"));
        sh(&format!("bpftool link show pinned {PIN_LINK}"));

        // (viii-a) swap under a 10 ms loop
        let b = load_steer(elf);
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        let looper = thread::spawn(move || {
            setns_by_name("v26-host").unwrap();
            let mut s = Stats::default();
            while !st.load(Ordering::Acquire) {
                s.add(&connect_once(A_STEERED, PORT, 0, 500));
                thread::sleep(Duration::from_millis(10));
            }
            s
        });
        thread::sleep(Duration::from_millis(300));
        let r = link_update(l, b.fd.as_raw_fd(), None);
        thread::sleep(Duration::from_millis(300));
        stop.store(true, Ordering::Release);
        let s = looper.join().unwrap();
        let id1 = link_prog_id(l);
        let wd = wild_delta!();
        say(format!("(viii) swap A->B under a 10 ms connect loop: update={r:?}; loop: {} ; intake accepts={} wildcard +{wd}", s.show(), i1.count()));
        sh(&format!("bpftool link show pinned {PIN_LINK}"));
        verdict("K-L9(viii) BPF_LINK_UPDATE to a newly loaded program: link names the new program, a 10 ms connect loop never reaches a host socket", r.is_ok() && id1 == b.id && id1 != id0 && s.refused == s.n && s.n > 30 && i1.count() == 0 && wd == 0, format!("update ok={}, link prog {id0} -> {id1} (new prog {}), loop {}/{} reset, host sockets reached: intake {} wildcard {wd}", r.is_ok(), b.id, s.refused, s.n, i1.count()));

        // (viii-b) failing updates
        let cur = id1;
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        let looper = thread::spawn(move || {
            setns_by_name("v26-host").unwrap();
            let mut s = Stats::default();
            while !st.load(Ordering::Acquire) {
                s.add(&connect_once(A_STEERED, PORT, 0, 500));
                thread::sleep(Duration::from_millis(10));
            }
            s
        });
        let mut xdp_ebpf = EbpfLoader::new().load_file(elf).unwrap();
        let xp: &mut Xdp = xdp_ebpf.program_mut("v26_wrongtype").unwrap().try_into().unwrap();
        xp.load().expect("load xdp");
        let xfd = xp.fd().unwrap().as_fd().try_clone_to_owned().unwrap();
        let c = load_steer(elf); // loaded sk_lookup, attached nowhere, not the link's current program
        let pipe_fd = unsafe {
            let mut p = [0i32; 2];
            libc::pipe(p.as_mut_ptr());
            OwnedFd::from_raw_fd(p[0])
        };
        type Case<'a> = (&'a str, Result<(), i32>);
        let cases: Vec<Case> = vec![
            ("replacement of another attach type (XDP program)", link_update(l, xfd.as_raw_fd(), None)),
            ("expected_old mismatch (BPF_F_REPLACE, old = a program that is not the link's)", link_update(l, c.fd.as_raw_fd(), Some(c.fd.as_raw_fd()))),
            ("expected_old mismatch with a valid new program (new = C, old = A)", link_update(l, c.fd.as_raw_fd(), Some(a_prog.fd.as_raw_fd()))),
            ("replacement fd is not a program (a pipe)", link_update(l, pipe_fd.as_raw_fd(), None)),
            ("replacement fd is invalid (-1)", link_update(l, -1, None)),
        ];
        // the matching expected_old succeeds (positive control for the BPF_F_REPLACE path); done after the failures
        let mut all_unchanged = true;
        for (name, res) in &cases {
            let id = link_prog_id(l);
            say(format!("  failing update [{name}] -> {res:?} ; link prog id after = {id} (expected {cur})"));
            all_unchanged &= res.is_err() && id == cur;
        }
        thread::sleep(Duration::from_millis(200));
        stop.store(true, Ordering::Release);
        let s = looper.join().unwrap();
        let wd = wild_delta!();
        sh(&format!("bpftool link show pinned {PIN_LINK}"));
        say(format!("  connect loop across the failing updates: {} ; intake accepts={} wildcard +{wd}", s.show(), i1.count()));
        verdict("K-L9(viii) every failing BPF_LINK_UPDATE returns an error and the link keeps running the earlier program; loop outcomes unchanged", all_unchanged && s.refused == s.n && i1.count() == 0 && wd == 0, format!("errors {:?}; link prog stays {cur}; loop {}/{} reset", cases.iter().map(|c| c.1.err()).collect::<Vec<_>>(), s.refused, s.n));
        let ok_exp = link_update(l, c.fd.as_raw_fd(), Some(b.fd.as_raw_fd()));
        say(format!("  positive control: BPF_F_REPLACE with the matching expected_old (B) -> {ok_exp:?}; link prog now {} (C = {})", link_prog_id(l), c.id));
        drop(xdp_ebpf);

        // (viii-c) stress: many swaps under 4 flat-out threads
        let stop = Arc::new(AtomicBool::new(false));
        let ths: Vec<_> = (0..4)
            .map(|_| {
                let st = stop.clone();
                thread::spawn(move || {
                    setns_by_name("v26-host").unwrap();
                    let mut s = Stats::default();
                    while !st.load(Ordering::Acquire) {
                        s.add(&connect_once(A_STEERED, PORT, MARK_LEG_S, 500));
                    }
                    s
                })
            })
            .collect();
        let progs = [load_steer(elf), load_steer(elf)];
        let ids = [progs[0].id, progs[1].id];
        let t0 = Instant::now();
        let mut fails = 0u64;
        let mut swaps = 0u64;
        let sampler_stop = Arc::new(AtomicBool::new(false));
        let sampler = {
            let st = sampler_stop.clone();
            let lraw = unsafe { libc::dup(l) };
            thread::spawn(move || {
                let mut seen = std::collections::BTreeSet::new();
                while !st.load(Ordering::Acquire) {
                    seen.insert(link_prog_id(lraw));
                }
                unsafe { libc::close(lraw) };
                seen
            })
        };
        while t0.elapsed() < Duration::from_secs(3) {
            if link_update(l, progs[(swaps % 2) as usize].fd.as_raw_fd(), None).is_err() {
                fails += 1;
            }
            swaps += 1;
        }
        let dur = t0.elapsed();
        sampler_stop.store(true, Ordering::Release);
        let seen = sampler.join().unwrap();
        stop.store(true, Ordering::Release);
        let mut tot = Stats::default();
        for t in ths {
            tot.merge(&t.join().unwrap());
        }
        let wd = wild_delta!();
        say(format!("(viii) {swaps} atomic swaps (A<->B, prog ids {ids:?}) in {dur:?} under 4 flat-out connect threads: {} ; update failures={fails} ; link prog ids sampled={seen:?} ; intake accepts={} wildcard +{wd}", tot.show(), i1.count()));
        verdict("K-L9(viii) thousands of atomic swaps under a flat-out connect storm: no connect ever reaches a host socket (contrast with the non-atomic control in P1)", fails == 0 && swaps > 1000 && tot.refused == tot.n && i1.count() == 0 && wd == 0 && tot.n > 1000 && seen.len() == 2, format!("{swaps} swaps in {dur:?}, {} connects all reset, 0 reached host sockets; link observed running both programs {seen:?}", tot.n));
        // after swaps the (new, empty) map decides: a steered listener of the old map no longer matches
        i1.close();
    }

    // ============================================================== P5 capacity
    if want("p5") {
        say("\n######## P5 (x) capacity: 262,144 entries, E2BIG, slot free in the close step ########");
        const N: usize = 262_144;
        let mut cap = load_steer(elf);
        let t0 = Instant::now();
        let mut ls: Vec<(Ipv4Addr, u16, OwnedFd)> = Vec::with_capacity(N + 8);
        let addr_of = |i: usize| -> (Ipv4Addr, u16) {
            let idx = i / 5 + 2; // skip .0.0/.0.1 (gateway)
            (Ipv4Addr::new(100, 95, (idx >> 8) as u8, (idx & 0xff) as u8), 30000 + (i % 5) as u16)
        };
        let mut fill_err = None;
        for i in 0..N {
            let (ip, port) = addr_of(i);
            let l = match listen_on(ip, port, false, 1) {
                Ok(l) => l,
                Err(e) => {
                    fill_err = Some(format!("listen {i}: {e}"));
                    break;
                }
            };
            if let Err(e) = cap.steer(ip, port, l.as_raw_fd()) {
                fill_err = Some(format!("steer {i}: errno {e}"));
                break;
            }
            ls.push((ip, port, l));
        }
        say(format!("filled {} listeners in {:?}; error={fill_err:?}", ls.len(), t0.elapsed()));
        verdict("(x) fill: GUEST_INTAKE_STEERING_MAX listening sockets inserted", ls.len() == N, format!("{} entries", ls.len()));
        sh("grep -E 'MemTotal|MemAvailable' /proc/meminfo");
        // one more
        let (ipx, portx) = addr_of(N);
        let lx = listen_on(ipx, portx, false, 1).unwrap();
        let r = cap.steer(ipx, portx, lx.as_raw_fd());
        say(format!("insert #{} -> {r:?} (E2BIG={})", N + 1, libc::E2BIG));
        verdict("(x) one more insert at capacity fails E2BIG", r == Err(libc::E2BIG), format!("{r:?}"));
        // close any one, insert a new one, 1000 cycles
        let mut cycle_fail = 0;
        let mut spare = lx;
        let (mut sip, mut sport) = (ipx, portx);
        let t1 = Instant::now();
        for c in 0..1000 {
            let victim = (c * 257) % ls.len();
            let (vip, vport, vl) = ls.swap_remove(victim);
            drop(vl); // close
            // immediately: the slot must be free
            match cap.steer(sip, sport, spare.as_raw_fd()) {
                Ok(()) => {}
                Err(_) => cycle_fail += 1,
            }
            ls.push((sip, sport, spare));
            // the victim's address becomes the next spare
            spare = listen_on(vip, vport, false, 1).unwrap();
            sip = vip;
            sport = vport;
        }
        say(format!("1000 close-one/insert-new cycles in {:?}: failures={cycle_fail}", t1.elapsed()));
        // still exactly full
        let r2 = cap.steer(sip, sport, spare.as_raw_fd());
        let nk = cap.n_keys();
        say(format!("after the cycles: keys={nk}; one more insert -> {r2:?}"));
        verdict("(x)/K-L3 a closed listener's slot is free in the same step: 1000/1000 immediate inserts after a close succeed, the map stays exactly full", cycle_fail == 0 && r2 == Err(libc::E2BIG) && nk == N, format!("cycle failures {cycle_fail}/1000, keys={nk}, extra insert {r2:?}"));
        drop(ls);
        drop(spare);
        let nk_after = cap.n_keys();
        verdict("(x)/K-L3 closing all 262,144 listeners empties the map", nk_after == 0, format!("keys after closing everything: {nk_after}"));
    }

    // ============================================================== teardown
    drop(link_fd);
    let _ = fs::remove_file(PIN_LINK);
    let _ = fs::remove_dir(PIN_DIR);
    nft_off();
    wild.close();
    legc.close();
    say("\n======== SUMMARY ========");
    for (c, v, d) in RESULTS.lock().unwrap().iter() {
        say(format!("[{v}] {c}\n       {d}"));
    }
    let failed = RESULTS.lock().unwrap().iter().filter(|r| r.1 != "OK").count();
    say(format!("{} checks, {} failed", RESULTS.lock().unwrap().len(), failed));
}

/// The raw descriptor of an acceptor's listener is needed once, to insert it into the map. The accept
/// thread owns the fd; we kept its number in the acceptor via a side channel (the first fd of this process
/// that is a listening socket bound to PORT) -- here simply recorded at spawn time.
fn last_fd(a: &Acceptor) -> RawFd {
    a.fd_hint()
}
fn p_detach(ebpf: &mut Ebpf, lid: aya::programs::sk_lookup::SkLookupLinkId) {
    let p: &mut SkLookup = ebpf.program_mut("v26_ifx").unwrap().try_into().unwrap();
    p.detach(lid).unwrap();
}
