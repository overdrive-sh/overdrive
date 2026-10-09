//! V-17 throwaway probe (GH #295): hybrid unframe under the appliance's interface set (D5a-IFACE).
//! Runs inside netns `v17-host` (the stand-in for the appliance's root namespace); the peers are
//! netns `v17-peerK`. Usage: v17 run <bpf.o>
//! Everything created is prefixed `v17-`.

use std::{
    collections::BTreeMap,
    fs,
    io,
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    os::fd::{AsRawFd, RawFd},
    process::Command,
    sync::mpsc::{channel, Receiver},
    thread,
    time::Duration,
};

use aya::{
    maps::{HashMap as AyaHash, PerCpuArray},
    programs::{
        KProbe,
        tc::{SchedClassifier, TcAttachOptions, TcAttachType},
        LinkOrder, ProgramError,
    },
    Ebpf, EbpfLoader, Pod,
};

const MAGIC: [u8; 4] = *b"ZUD1";
const DST: Ipv4Addr = Ipv4Addr::new(10, 17, 200, 1);
const PORT: u16 = 17001;

#[repr(C)]
#[derive(Clone, Copy)]
struct Tuple {
    src: u32,
    dst: u32,
    sport: u16,
    dport: u16,
}
unsafe impl Pod for Tuple {}

// counters (see ebpf)
const CN: [&str; 8] = ["decoded", "decoded_empty", "frag_first", "frag_shift", "lenient_pass", "err", "seen", "idem_skip"];

fn tuple_of(src: Ipv4Addr, sport: u16, dst: Ipv4Addr, dport: u16) -> Tuple {
    // byte representation as the program reads it from the packet (ctx.load: native read of NBO bytes)
    Tuple {
        src: u32::from_ne_bytes(src.octets()),
        dst: u32::from_ne_bytes(dst.octets()),
        sport: u16::from_ne_bytes(sport.to_be_bytes()),
        dport: u16::from_ne_bytes(dport.to_be_bytes()),
    }
}

// ---------------------------------------------------------------- errors
fn errno_of(e: &(dyn std::error::Error + 'static)) -> Option<i32> {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(x) = cur {
        if let Some(io) = x.downcast_ref::<io::Error>() {
            return io.raw_os_error();
        }
        cur = x.source();
    }
    None
}
fn errno_name(n: i32) -> String {
    let s = unsafe { std::ffi::CStr::from_ptr(libc::strerror(n)) }.to_string_lossy().into_owned();
    format!("errno {n} ({s})")
}
/// The design's classification (UnframeAttachError): ENOMEM -> KernelMemory, anything else -> Attach.
fn classify(e: &ProgramError) -> String {
    match errno_of(e) {
        Some(libc::ENOMEM) => "KernelMemory".into(),
        Some(_) => "Attach".into(),
        None => "Attach (no errno in chain)".into(),
    }
}
fn describe(e: &ProgramError) -> String {
    format!("{} -> {}  [{}]", errno_of(e).map(errno_name).unwrap_or_else(|| "no errno".into()), classify(e), e)
}

// ---------------------------------------------------------------- namespaces & sockets
fn setns_by_name(name: &str) -> io::Result<()> {
    let f = fs::File::open(format!("/var/run/netns/{name}"))?;
    if unsafe { libc::setns(f.as_raw_fd(), libc::CLONE_NEWNET) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn setopt_i32(fd: RawFd, lvl: i32, opt: i32, v: i32) {
    unsafe { libc::setsockopt(fd, lvl, opt, &v as *const _ as *const _, 4) };
}

struct Receiver_ {
    rx: Receiver<Vec<u8>>,
}
/// A UDP receiver bound to 0.0.0.0:PORT inside netns `ns` ("" = this namespace).
fn spawn_receiver(ns: &str) -> Receiver_ {
    let (tx, rx) = channel();
    let (rtx, rrx) = channel();
    let ns = ns.to_string();
    thread::spawn(move || {
        if !ns.is_empty() {
            setns_by_name(&ns).expect("setns peer");
        }
        let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PORT)).expect("bind rx");
        setopt_i32(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF, 8 << 20);
        rtx.send(()).unwrap();
        let mut buf = vec![0u8; 70000];
        loop {
            match s.recv_from(&mut buf) {
                Ok((n, _)) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    });
    rrx.recv().unwrap();
    Receiver_ { rx }
}

fn rng_bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.push((x >> 24) as u8);
    }
    // a random payload must not parse as a frame by accident
    if v.len() >= 4 && v[..4] == MAGIC {
        v[0] ^= 0xff;
    }
    v
}
fn frame(len: usize) -> [u8; 8] {
    let mut f = [0u8; 8];
    f[..4].copy_from_slice(&MAGIC);
    f[4..].copy_from_slice(&(len as u32).to_be_bytes());
    f
}
fn framed(p: &[u8]) -> Vec<u8> {
    let mut w = Vec::with_capacity(p.len() + 8);
    w.extend_from_slice(&frame(p.len()));
    w.extend_from_slice(p);
    w
}

// ---------------------------------------------------------------- egress description
#[derive(Clone)]
struct Eg {
    dev: &'static str,
    peer_ns: &'static str,
    src: Ipv4Addr,
    bind: bool,
    mark: u32,
    mtu: usize,
    stacked: bool,
    lower: &'static str,
}

fn make_sock(eg: &Eg, dst: Ipv4Addr, connect: bool) -> UdpSocket {
    let s = if eg.bind || !connect { UdpSocket::bind((eg.src, 0)).unwrap() } else { UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap() };
    if eg.mark != 0 {
        setopt_i32(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_MARK, eg.mark as i32);
    }
    setopt_i32(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_SNDBUF, 8 << 20);
    if connect {
        s.connect(SocketAddrV4::new(dst, PORT)).unwrap();
    }
    s
}
fn sock_tuple(s: &UdpSocket, dst: Ipv4Addr) -> Tuple {
    // from the connected socket's kernel-reported local address (D5a: tuples come from the kernel)
    let la = match s.local_addr().unwrap() {
        std::net::SocketAddr::V4(a) => a,
        _ => unreachable!(),
    };
    tuple_of(*la.ip(), la.port(), dst, PORT)
}

// ---------------------------------------------------------------- the unframe set (design: boot attach)
struct Unframe {
    prog: &'static str,
    ebpf: Ebpf,
    attached: Vec<String>,
}
impl Unframe {
    fn load(obj: &str) -> Unframe {
        Self::load_prog(obj, "v17_unframe")
    }
    fn load_prog(obj: &str, prog: &'static str) -> Unframe {
        let mut ebpf = EbpfLoader::new().load_file(obj).expect("load bpf object");
        let p: &mut SchedClassifier = ebpf.program_mut(prog).unwrap().try_into().unwrap();
        p.load().expect("load program");
        Unframe { prog, ebpf, attached: vec![] }
    }
    fn attach(&mut self, name: &str) -> Result<(), ProgramError> {
        let p: &mut SchedClassifier = self.ebpf.program_mut(self.prog).unwrap().try_into().unwrap();
        // production guest_tcx.rs attaches TCX egress with LinkOrder::first()
        p.attach_with_options(name, TcAttachType::Egress, TcAttachOptions::TcxOrder(LinkOrder::first()))?;
        self.attached.push(name.to_string());
        Ok(())
    }
    fn register(&mut self, t: Tuple) {
        let mut m: AyaHash<_, Tuple, u32> = AyaHash::try_from(self.ebpf.map_mut("TUPLES").unwrap()).unwrap();
        m.insert(t, 3, 0).unwrap();
    }
    fn counters(&self) -> [u64; 8] {
        let m: PerCpuArray<_, u64> = PerCpuArray::try_from(self.ebpf.map("CNT").unwrap()).unwrap();
        let mut out = [0u64; 8];
        for (i, o) in out.iter_mut().enumerate() {
            *o = m.get(&(i as u32), 0).unwrap().iter().sum();
        }
        out
    }
}
fn fmt_counters(c: &[u64; 8]) -> String {
    CN.iter().zip(c.iter()).map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join(" ")
}

fn iface_list() -> BTreeMap<u32, String> {
    let mut m = BTreeMap::new();
    for e in fs::read_dir("/sys/class/net").unwrap() {
        let n = e.unwrap().file_name().to_string_lossy().into_owned();
        let c = std::ffi::CString::new(n.clone()).unwrap();
        let idx = unsafe { libc::if_nametoindex(c.as_ptr()) };
        if idx != 0 {
            m.insert(idx, n);
        }
    }
    m
}

/// D5a-IFACE: `load` attaches lo, then converge_unframe_interfaces attaches every other interface
/// present; any failure refuses startup. `skip` lets a diagnostic run leave interfaces unattached.
fn boot_attach(u: &mut Unframe, skip: &[&str], verbose: bool) -> Result<(), String> {
    let ifs = iface_list();
    let mut failed: Vec<(u32, String, String)> = vec![];
    let mut order: Vec<(u32, String)> = ifs.into_iter().collect();
    order.sort_by_key(|(i, n)| (n != "lo", *i));
    for (idx, name) in order {
        if skip.contains(&name.as_str()) {
            continue;
        }
        match u.attach(&name) {
            Ok(()) => {
                if verbose {
                    println!("  boot attach  ifindex={idx:<4} {name:<14} OK");
                }
            }
            Err(e) => {
                println!("  boot attach  ifindex={idx:<4} {name:<14} FAILED {}", describe(&e));
                failed.push((idx, name, describe(&e)));
            }
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("UnframeAttach refused startup: {:?}", failed))
    }
}

// ---------------------------------------------------------------- traffic
fn sizes_for(mtu: usize, cap: usize) -> Vec<usize> {
    let mut v: Vec<usize> = vec![0, 1, 2, 7, 8, 9, 15, 16, 17, 100, 1431, 1432, 4096, 4097, 8192, 59000, 65499];
    for d in [-9i64, -8, -7, -1, 0, 1, 7, 8, 9] {
        for base in [mtu as i64 - 28, 2 * (mtu as i64 - 28), 3 * (mtu as i64 - 28)] {
            let s = base + d;
            if s >= 0 {
                v.push(s as usize);
            }
        }
    }
    v.push(mtu - 20);
    v.push(mtu);
    v.push(mtu + 1);
    v.sort();
    v.dedup();
    v.retain(|&s| s <= cap);
    v
}

#[derive(Default)]
struct Tally {
    ok: usize,
    total: usize,
    fails: Vec<String>,
}
impl Tally {
    fn rec(&mut self, size: usize, r: Result<(), String>) {
        self.total += 1;
        match r {
            Ok(()) => self.ok += 1,
            Err(e) => self.fails.push(format!("size {size}: {e}")),
        }
    }
    fn line(&self, label: &str) -> String {
        let mut s = format!("    {label:<34} {}/{} byte-exact", self.ok, self.total);
        if !self.fails.is_empty() {
            s.push_str(&format!("  FAILURES({}):", self.fails.len()));
            for f in self.fails.iter().take(6) {
                s.push_str(&format!("\n        {f}"));
            }
            if self.fails.len() > 6 {
                s.push_str(&format!("\n        ... and {} more", self.fails.len() - 6));
            }
        }
        s
    }
}

fn expect_rx(rx: &Receiver_, want: &[u8]) -> Result<(), String> {
    match rx.rx.recv_timeout(Duration::from_millis(1500)) {
        Err(_) => Err("LOST (nothing received within 1.5 s)".into()),
        Ok(got) => {
            if got == want {
                Ok(())
            } else {
                let off = got.iter().zip(want.iter()).position(|(a, b)| a != b);
                Err(format!(
                    "MISMATCH got {} bytes want {} bytes, first differing offset {:?}{}",
                    got.len(),
                    want.len(),
                    off,
                    if got.len() == want.len() + 8 && got[..4] == MAGIC { " (frame NOT removed: 8 bytes longer, starts ZUD1)" } else { "" }
                ))
            }
        }
    }
}
fn drain(rx: &Receiver_) {
    while rx.rx.try_recv().is_ok() {}
}

/// The size/variant matrix through one egress. `u` is None for the no-program control.
fn matrix(u: &mut Option<Unframe>, eg: &Eg, rx: &Receiver_, label: &str, quick: bool) {
    let before = u.as_ref().map(|u| u.counters());
    let dst = if eg.dev == "lo" { Ipv4Addr::LOCALHOST } else { DST };
    let sizes = if quick { vec![0usize, 1, 8, eg.mtu - 28, eg.mtu - 20, 4097, 59000] } else { sizes_for(eg.mtu, 65499) };
    // registered connected socket
    let reg = make_sock(eg, dst, true);
    if let Some(u) = u.as_mut() {
        u.register(sock_tuple(&reg, dst));
    }
    let unreg = make_sock(eg, dst, true); // different ephemeral sport, never registered
    let unconn = make_sock(eg, dst, false);
    if let Some(u) = u.as_mut() {
        // unconnected socket: tuple registered from its (src, sport) and the destination it will send to
        u.register(sock_tuple(&unconn, dst));
    }
    let seed = (eg.mtu as u64) << 8;
    let (mut a, mut b, mut c, mut d, mut e) = (Tally::default(), Tally::default(), Tally::default(), Tally::default(), Tally::default());
    for &s in &sizes {
        drain(rx);
        // A: frame kept by the verdict (empty / any size): wire = frame + P, expect P
        let p = rng_bytes(seed + s as u64, s);
        reg.send(&framed(&p)).unwrap();
        a.rec(s, expect_rx(rx, &p));
        // B: bare payload on a registered tuple (verdict already stripped): unchanged
        if s > 0 {
            let p = rng_bytes(seed + 77 + s as u64, s);
            reg.send(&p).unwrap();
            b.rec(s, expect_rx(rx, &p));
        }
        // C: frame-shaped payload (escape): P = frame(|Q|)+Q, wire = frame(|P|)+P, expect P
        if s >= 8 && s <= 65490 {
            let q = rng_bytes(seed + 991 + s as u64, s - 8);
            let p = framed(&q);
            reg.send(&framed(&p)).unwrap();
            c.rec(s, expect_rx(rx, &p));
        }
        // D: unregistered tuple: must be left alone (arrives with the frame)
        {
            let p = rng_bytes(seed + 5 + s as u64, s);
            let w = framed(&p);
            unreg.send(&w).unwrap();
            d.rec(s, expect_rx(rx, &w));
        }
        // E: unconnected sendto of a kept frame
        {
            let p = rng_bytes(seed + 9 + s as u64, s);
            unconn.send_to(&framed(&p), SocketAddrV4::new(dst, PORT)).unwrap();
            e.rec(s, expect_rx(rx, &p));
        }
    }
    println!("  [{label}] egress {} (mtu {}, peer ns {:?}, src {}, {} sizes)", eg.dev, eg.mtu, eg.peer_ns, eg.src, sizes.len());
    println!("{}", a.line("A frame kept, strip -> P"));
    println!("{}", b.line("B bare on registered tuple, unchanged"));
    println!("{}", c.line("C frame-shaped payload (escape) -> P"));
    println!("{}", d.line("D unregistered tuple, left framed"));
    println!("{}", e.line("E unconnected sendto, strip -> P"));
    if let (Some(u), Some(b0)) = (u.as_ref(), before) {
        let af = u.counters();
        let mut dlt = [0u64; 8];
        for i in 0..8 {
            dlt[i] = af[i] - b0[i];
        }
        println!("    program counters (delta this egress): {}", fmt_counters(&dlt));
    }
}

fn sh(cmd: &str) -> String {
    let o = Command::new("bash").arg("-c").arg(cmd).output().unwrap();
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "case" {
        refusal_case(&args[2], &args[3]);
        return;
    }
    if args.len() < 3 || args[1] != "run" {
        eprintln!("usage: v17 run <bpf.o> [phases: control design stacked refuse]");
        std::process::exit(2);
    }
    let obj = args[2].clone();
    let phases: Vec<String> = if args.len() > 3 { args[3..].to_vec() } else { ["control", "design", "stacked"].iter().map(|s| s.to_string()).collect() };
    let has = |p: &str| phases.iter().any(|x| x == p);
    println!("kernel: {}", sh("uname -r").trim());
    println!("arch: {}", sh("uname -m").trim());
    println!("interfaces in v17-host: {:?}", iface_list().values().collect::<Vec<_>>());

    let egs: Vec<Eg> = vec![
        Eg { dev: "lo", peer_ns: "", src: Ipv4Addr::LOCALHOST, bind: false, mark: 0, mtu: 65536, stacked: false, lower: "" },
        Eg { dev: "v17-h1", peer_ns: "v17-peer1", src: Ipv4Addr::new(10, 17, 1, 1), bind: true, mark: 0, mtu: 1500, stacked: false, lower: "" },
        Eg { dev: "v17-h2", peer_ns: "v17-peer2", src: Ipv4Addr::new(10, 17, 2, 1), bind: true, mark: 0, mtu: 1280, stacked: false, lower: "" },
        Eg { dev: "v17-h3", peer_ns: "v17-peer3", src: Ipv4Addr::new(10, 17, 3, 1), bind: false, mark: 0x173, mtu: 9000, stacked: false, lower: "" },
        Eg { dev: "v17-mv4", peer_ns: "v17-peer4", src: Ipv4Addr::new(10, 17, 4, 1), bind: true, mark: 0, mtu: 1500, stacked: true, lower: "v17-h4" },
        Eg { dev: "v17-h5.50", peer_ns: "v17-peer5", src: Ipv4Addr::new(10, 17, 5, 1), bind: true, mark: 0, mtu: 1500, stacked: true, lower: "v17-h5" },
        Eg { dev: "v17-br6", peer_ns: "v17-peer6", src: Ipv4Addr::new(10, 17, 6, 1), bind: true, mark: 0, mtu: 1500, stacked: true, lower: "v17-h6" },
    ];
    let rxs: Vec<Receiver_> = egs.iter().map(|e| spawn_receiver(e.peer_ns)).collect();

    if has("control") {
        println!("\n=== PHASE control: no program attached (frames must be visible to the peer: 8 bytes longer, ZUD1)");
        let mut none: Option<Unframe> = None;
        for i in [1usize, 2] {
            matrix(&mut none, &egs[i], &rxs[i], "control/no-program", true);
        }
    }

    if has("design") {
        println!("\n=== PHASE design: boot attach to every interface present (D5a-IFACE), then the matrix through the non-stacked egress");
        let mut u = Unframe::load(&obj);
        match boot_attach(&mut u, &[], true) {
            Ok(()) => println!("  boot: ALL {} interfaces attached; startup proceeds", u.attached.len()),
            Err(e) => println!("  boot: {e}"),
        }
        println!("--- kernel link query (bpftool net) ---\n{}", sh("bpftool net list 2>&1"));
        for (i, eg) in egs.iter().enumerate() {
            if eg.stacked {
                continue;
            }
            let mut o = Some(u);
            matrix(&mut o, eg, &rxs[i], "design/all-attached", false);
            u = o.unwrap();
        }
        println!("  totals: {}", fmt_counters(&u.counters()));
        // keep `u` alive until here: dropping it detaches every TCX link
    }

    if has("stacked") {
        println!("\n=== PHASE stacked: upper devices over an attached lower device (macvlan, vlan, bridge). Both ends are root-namespace interfaces, so the design attaches both.");
        for (i, eg) in egs.iter().enumerate() {
            if !eg.stacked {
                continue;
            }
            for (cfg, skip) in [
                ("both upper and lower attached (design)", vec![]),
                ("only the UPPER attached", vec![eg.lower]),
                ("only the LOWER attached", vec![eg.dev]),
                ("MITIGATION idempotent program, both attached", vec![]),
            ] {
                let mut u = if cfg.starts_with("MITIGATION") { Unframe::load_prog(&obj, "v17_unframe_idem") } else { Unframe::load(&obj) };
                // attach exactly the requested subset of {upper, lower}, never anything else
                let mut sk: Vec<&str> = vec![];
                for (_, n) in iface_list() {
                    if n != eg.dev && n != eg.lower {
                        sk.push(Box::leak(n.into_boxed_str()));
                    }
                }
                sk.extend(skip.iter().copied());
                boot_attach(&mut u, &sk, false).ok();
                println!("  attached set: {:?}", u.attached);
                let mut o = Some(u);
                matrix(&mut o, eg, &rxs[i], cfg, false);
            }
        }
    }

    println!("\n=== done");
}

fn drop_caps() {
    #[repr(C)]
    struct Hdr {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Data {
        eff: u32,
        perm: u32,
        inh: u32,
    }
    let hdr = Hdr { version: 0x20080522, pid: 0 };
    let data = [Data { eff: 0, perm: 0, inh: 0 }; 2];
    let r = unsafe { libc::syscall(libc::SYS_capset, &hdr, data.as_ptr()) };
    println!("    capset(effective/permitted/inheritable = empty) -> {r}");
}

fn res<T>(r: Result<T, ProgramError>) -> String {
    match r {
        Ok(_) => "OK".into(),
        Err(e) => describe(&e),
    }
}

fn load_nop(obj: &str) -> Ebpf {
    let mut e = EbpfLoader::new().load_file(obj).expect("load");
    let p: &mut SchedClassifier = e.program_mut("v17_nop").unwrap().try_into().unwrap();
    p.load().expect("load nop");
    e
}

fn refusal_case(case: &str, obj: &str) {
    let target = "v17-h1";
    println!("kernel: {}", sh("uname -r").trim());
    match case {
        "R1" => {
            let mut u = Unframe::load(obj);
            let first = u.attach(target);
            let second = u.attach(target);
            println!("R1 same program attached twice to {target}: first={} second={}", res(first), res(second));
            // and a second *instance* of the same program (a different prog id) is fine:
            let mut u2 = Unframe::load(obj);
            println!("R1b a different load of the same program, same interface: {}", res(u2.attach(target)));
        }
        "R2" => {
            let mut u = Unframe::load(obj);
            sh("ip link add v17-gone type dummy");
            let gone = iface_list().values().find(|n| *n == "v17-gone").cloned().unwrap();
            sh("ip link del v17-gone");
            println!("R2 interface deleted after enumeration, before attach ({gone}): {}", res(u.attach(&gone)));
        }
        "R3" => {
            let mut u = Unframe::load(obj);
            println!("R3 program loaded with capabilities, capabilities then dropped, then attach {target}:");
            drop_caps();
            println!("    attach: {}", res(u.attach(target)));
            let l = EbpfLoader::new().load_file(obj);
            println!("    (control) loading a new object without capabilities: {}", match l { Ok(_) => "OK".into(), Err(e) => format!("{e}") });
        }
        "R4" => {
            let mut u = Unframe::load(obj);
            let maxfd = fs::read_dir("/proc/self/fd").unwrap().filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_string_lossy().parse::<u64>().ok()).max().unwrap();
            // occupy every free descriptor below the highest one, so the limit leaves no free slot
            for fd in 0..maxfd as i32 {
                if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
                    unsafe { libc::dup2(0, fd) };
                }
            }
            let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) };
            let new = libc::rlimit { rlim_cur: maxfd, rlim_max: lim.rlim_max };
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &new) };
            println!("R4 RLIMIT_NOFILE soft limit set to {} (descriptors 0..{} all occupied; the /proc/self/fd directory handle that fixed {maxfd} is closed, so no slot below the limit is free); attach {target}: {}", maxfd, maxfd, res(u.attach(target)));
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &lim) };
            println!("    limit restored; attach again: {}", res(u.attach(target)));
            let mut u2 = Unframe::load(obj);
            println!("    limit restored, fresh program, attach {target}: {}", res(u2.attach(target)));
        }
        "R5" => {
            // fill the interface's TCX egress set until the kernel refuses
            let mut keep: Vec<Ebpf> = vec![];
            let mut ok = 0;
            let mut first_err = String::new();
            for i in 0..200 {
                let mut e = load_nop(obj);
                let p: &mut SchedClassifier = e.program_mut("v17_nop").unwrap().try_into().unwrap();
                match p.attach_with_options(target, TcAttachType::Egress, TcAttachOptions::TcxOrder(LinkOrder::last())) {
                    Ok(_) => {
                        ok += 1;
                        keep.push(e);
                    }
                    Err(er) => {
                        first_err = format!("attach #{} refused: {}", i + 1, describe(&er));
                        break;
                    }
                }
            }
            println!("R5 TCX egress set of {target} filled: {ok} attachments accepted; {first_err}");
            println!("--- bpftool net (count of tcx/egress entries on {target}): {}", sh(&format!("bpftool net list 2>&1 | grep -c 'v17_nop'")).trim());
            println!("--- boot over every interface present, {target} full:");
            let mut u = Unframe::load(obj);
            match boot_attach(&mut u, &[], true) {
                Ok(()) => println!("  boot: all attached"),
                Err(e) => println!("  boot: {e}"),
            }
            println!("  attached before refusal was reported (this run does not stop at the first failure; the design's converge keeps going and reports all): {:?}", u.attached);
        }
        "R6" => {
            sh("ip tuntap add dev v17-tun mode tun; ip link add v17-dum type dummy");
            let mut u = Unframe::load(obj);
            println!("R6 L3 device (tun, ARPHRD_NONE, no Ethernet header): {}", res(u.attach("v17-tun")));
            println!("R6 dummy interface administratively DOWN: {}", res(u.attach("v17-dum")));
            sh("ip link set v17-dum up");
            println!("R6 dummy UP, second link to the same program: {}", res(u.attach("v17-dum")));
            sh("ip link del v17-tun; ip link del v17-dum");
        }
        "R7" => {
            let mut ebpf = EbpfLoader::new().load_file(obj).expect("load");
            let off: u64 = if cfg!(target_arch = "x86_64") { 112 } else { 0 };
            let sym = if cfg!(target_arch = "x86_64") { "__x64_sys_bpf" } else { "__arm64_sys_bpf" };
            {
                let mut cfgk: aya::maps::Array<_, u64> = aya::maps::Array::try_from(ebpf.map_mut("CFGK").unwrap()).unwrap();
                cfgk.set(0, off, 0).unwrap();
                cfgk.set(1, 0, 0).unwrap();
                cfgk.set(2, std::process::id() as u64, 0).unwrap();
            }
            let kp: &mut KProbe = ebpf.program_mut("v17_enomem").unwrap().try_into().unwrap();
            kp.load().expect("load kprobe");
            kp.attach(sym, 0).expect("attach kprobe");
            let p: &mut SchedClassifier = ebpf.program_mut("v17_unframe").unwrap().try_into().unwrap();
            p.load().expect("load unframe");
            println!("R7 fault injection (kprobe+bpf_override_return on {sym}, BPF_LINK_CREATE only, this tgid only)");
            enomem_run(&mut ebpf, target);
        }
        _ => eprintln!("unknown case"),
    }
}

fn enomem_run(ebpf: &mut Ebpf, target: &str) {
    fn en(ebpf: &mut Ebpf, v: u64) {
        let mut c: aya::maps::Array<_, u64> = aya::maps::Array::try_from(ebpf.map_mut("CFGK").unwrap()).unwrap();
        c.set(1, v, 0).unwrap();
    }
    en(ebpf, 1);
    let r = {
        let p: &mut SchedClassifier = ebpf.program_mut("v17_unframe").unwrap().try_into().unwrap();
        p.attach_with_options(target, TcAttachType::Egress, TcAttachOptions::TcxOrder(LinkOrder::first()))
    };
    en(ebpf, 0);
    println!("    attach {target} with ENOMEM injected: {}", res(r));
    let c: aya::maps::Array<_, u64> = aya::maps::Array::try_from(ebpf.map("CFGK").unwrap()).unwrap();
    println!("    injections performed: {}", c.get(&3, 0).unwrap());
    let r2 = {
        let p: &mut SchedClassifier = ebpf.program_mut("v17_unframe").unwrap().try_into().unwrap();
        p.attach_with_options(target, TcAttachType::Egress, TcAttachOptions::TcxOrder(LinkOrder::first()))
    };
    println!("    injection off, same attach: {}", res(r2));
}
