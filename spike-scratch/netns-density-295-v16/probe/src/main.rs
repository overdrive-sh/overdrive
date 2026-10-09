//! V-16 throwaway probe userspace (GH #295). Standalone; never part of crates/.
//!
//! v16 tx <bpf.o> <ifname> <srcip> <dstip> <set> [--legacy] [--unregistered] [--rounds N] [--no-attach]
//! v16 rx <bindip> <set> [idle_secs]
//! v16 cases <set>            (print the case table)
//!
//! `set`: plain | gso | all. Payload generator, frame layout and port plan are shared by tx, rx and verify_pcap.py.
use aya::{
    maps::{HashMap as AyaHashMap, PerCpuArray},
    programs::{links::LinkOrder, tc::{SchedClassifier, TcAttachOptions, TcAttachType}},
    Ebpf, Pod,
};
use std::{
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    os::fd::AsRawFd,
    time::{Duration, Instant},
};

const MAGIC: [u8; 4] = *b"ZUD1";
const GSO_SIZE: u16 = 1200;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Bare,     // non-empty payload that is not frame-shaped: the verdict already stripped; wire = payload
    Empty,    // empty datagram: verdict keeps the frame; wire = frame(0) (8 bytes)
    Escaped,  // frame-shaped payload: verdict keeps the frame; wire = frame(n) + payload
    NearMiss, // magic but wrong length: not frame-shaped -> bare on the wire
}
#[derive(Clone, Copy, Debug)]
struct Case {
    id: u32,
    kind: Kind,
    n: usize, // application payload length
    gso: bool,
}
impl Case {
    fn sport(&self) -> u16 {
        41000 + self.id as u16
    }
    fn dport(&self) -> u16 {
        42000 + self.id as u16
    }
    fn payload(&self) -> Vec<u8> {
        let mut v: Vec<u8> = (0..self.n)
            .map(|i| ((i * 131 + self.id as usize * 17 + (i >> 8) * 7 + 1) & 0xff) as u8)
            .collect();
        match self.kind {
            Kind::Bare => {
                if !v.is_empty() {
                    v[0] |= 0x80;
                }
            }
            Kind::Escaped => {
                v[0..4].copy_from_slice(&MAGIC);
                v[4..8].copy_from_slice(&((self.n - 8) as u32).to_be_bytes());
            }
            Kind::NearMiss => {
                v[0..4].copy_from_slice(&MAGIC);
                v[4..8].copy_from_slice(&(self.n as u32).to_be_bytes());
            }
            Kind::Empty => {}
        }
        v
    }
    fn wire(&self) -> Vec<u8> {
        match self.kind {
            Kind::Bare | Kind::NearMiss => self.payload(),
            Kind::Empty => {
                let mut f = MAGIC.to_vec();
                f.extend_from_slice(&0u32.to_be_bytes());
                f
            }
            Kind::Escaped => {
                let mut f = MAGIC.to_vec();
                f.extend_from_slice(&(self.n as u32).to_be_bytes());
                f.extend_from_slice(&self.payload());
                f
            }
        }
    }
}

fn cases(set: &str) -> Vec<Case> {
    let mut v = Vec::new();
    let mut id = 0u32;
    let mut push = |kind, n, gso, v: &mut Vec<Case>| {
        v.push(Case { id, kind, n, gso });
        id += 1;
    };
    if set == "plain" || set == "all" {
        push(Kind::Empty, 0, false, &mut v);
        // Bare payloads: size classes incl. around every MTU boundary (1500 MTU -> 1472 max unfragmented payload)
        for n in [
            1, 2, 7, 8, 9, 15, 16, 17, 100, 1000, 1431, 1463, 1464, 1465, 1471, 1472, 1473, 1480, 1481, 2000, 2944, 2945,
            4096, 4097, 8192, 16000, 32768, 59000, 65000, 65507,
        ] {
            push(Kind::Bare, n, false, &mut v);
        }
        // frame-shaped payloads (escape rule): wire = 8 + n. 1464 -> wire 1472 (fits), 1465 -> wire 1473 (fragments)
        for n in [8, 9, 16, 100, 1431, 1463, 1464, 1465, 1472, 1473, 2000, 2944, 2945, 4096, 4097, 8192, 16000, 32768, 59000, 65499] {
            push(Kind::Escaped, n, false, &mut v);
        }
        for n in [8, 100, 1465, 4097, 59000] {
            push(Kind::NearMiss, n, false, &mut v);
        }
    }
    if set == "gso" || set == "all" {
        // UDP_SEGMENT = GSO_SIZE. Bytes must arrive concatenated; boundaries per GSO_SIZE.
        push(Kind::Empty, 0, true, &mut v);
        for n in [1, 1199, 1200, 1201, 2400, 2401, 5000, 14000, 59000] {
            push(Kind::Bare, n, true, &mut v);
        }
        for n in [8, 100, 1200, 1201, 2000, 5000, 59000] {
            push(Kind::Escaped, n, true, &mut v);
        }
        push(Kind::NearMiss, 5000, true, &mut v);
    }
    v
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Tuple {
    src: u32,
    dst: u32,
    sport: u16,
    dport: u16,
}
unsafe impl Pod for Tuple {}

fn arg(args: &[String], f: &str) -> bool {
    args.iter().any(|a| a == f)
}
fn argv(args: &[String], f: &str) -> Option<String> {
    args.iter().position(|a| a == f).and_then(|i| args.get(i + 1).cloned())
}

fn fnv(b: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(|s| s.as_str()) {
        Some("cases") => {
            for c in cases(&a[2]) {
                println!("{} {:?} n={} gso={} wire={} sport={} dport={}", c.id, c.kind, c.n, c.gso, c.wire().len(), c.sport(), c.dport());
            }
        }
        Some("tx") => tx(&a),
        Some("rx") => rx(&a),
        _ => eprintln!("usage: v16 cases|tx|rx ..."),
    }
}

fn tx(a: &[String]) {
    let obj = &a[2];
    let ifname = &a[3];
    let src: Ipv4Addr = a[4].parse().unwrap();
    let dst: Ipv4Addr = a[5].parse().unwrap();
    let set = &a[6];
    let rounds: usize = argv(a, "--rounds").map(|s| s.parse().unwrap()).unwrap_or(3);
    let mut bpf = Ebpf::load_file(obj).expect("load");
    let _link;
    if !arg(a, "--no-attach") {
        let p: &mut SchedClassifier = bpf.program_mut("eg_decode_frag").unwrap().try_into().unwrap();
        p.load().expect("prog load");
        if arg(a, "--legacy") {
            let _ = aya::programs::tc::qdisc_add_clsact(ifname);
            _link = p.attach(ifname, TcAttachType::Egress).expect("attach clsact");
            println!("ATTACH legacy clsact/cls_bpf egress on {ifname}");
        } else {
            _link = p
                .attach_with_options(ifname, TcAttachType::Egress, TcAttachOptions::TcxOrder(LinkOrder::default()))
                .expect("attach tcx");
            println!("ATTACH tcx egress on {ifname}");
        }
    }
    let cs = cases(set);
    if !arg(a, "--unregistered") {
        let mut m: AyaHashMap<_, Tuple, u32> = AyaHashMap::try_from(bpf.map_mut("TUPLES").unwrap()).unwrap();
        for c in &cs {
            let t = Tuple {
                src: u32::from_ne_bytes(src.octets()),
                dst: u32::from_ne_bytes(dst.octets()),
                sport: c.sport().to_be(),
                dport: c.dport().to_be(),
            };
            m.insert(t, 3, 0).unwrap();
        }
        println!("REGISTERED {} tuples (mode 3 = hybrid/lenient)", cs.len());
    } else {
        println!("TUPLES NOT REGISTERED (control)");
    }
    std::thread::sleep(Duration::from_millis(300));
    for c in &cs {
        let s = UdpSocket::bind(SocketAddrV4::new(src, c.sport())).expect("bind tx");
        if c.gso {
            let v: libc::c_int = GSO_SIZE as libc::c_int;
            let r = unsafe { libc::setsockopt(s.as_raw_fd(), libc::SOL_UDP, 103 /*UDP_SEGMENT*/, &v as *const _ as *const _, 4) };
            if r != 0 {
                println!("SETSOCKOPT UDP_SEGMENT failed: {}", std::io::Error::last_os_error());
            }
        }
        let w = c.wire();
        let rr = if c.gso { 1 } else { rounds };
        for _ in 0..rr {
            match s.send_to(&w, SocketAddrV4::new(dst, c.dport())) {
                Ok(n) if n == w.len() => {}
                Ok(n) => println!("SEND short case {} {} of {}", c.id, n, w.len()),
                Err(e) => println!("SEND error case {} {:?} n={} wire={}: {}", c.id, c.kind, c.n, w.len(), e),
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    std::thread::sleep(Duration::from_millis(500));
    let cnt: PerCpuArray<_, u64> = PerCpuArray::try_from(bpf.map("CNT").unwrap()).unwrap();
    let names = [
        (13u32, "tc_decoded"),
        (15, "tc_decoded_empty"),
        (32, "frag_first_unframed"),
        (33, "frag_shifted"),
        (37, "tc_err_shot"),
        (39, "lenient_pass"),
        (50, "gso_skbs_seen_first_frag"),
        (51, "gso_skbs_unframed"),
        (52, "udp_first_frags_seen"),
        (60, "fail_adjust_room"),
        (61, "fail_store_udphdr"),
        (62, "fail_l3_csum_replace"),
        (63, "sum_of_negated_errnos"),
    ];
    for (i, n) in names {
        let v: u64 = cnt.get(&i, 0).map(|x| x.iter().sum()).unwrap_or(0);
        println!("COUNTER {n} = {v}");
    }
}

fn rx(a: &[String]) {
    let bind: Ipv4Addr = a[2].parse().unwrap();
    let set = &a[3];
    let idle: u64 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(4);
    let rounds = 3usize;
    let cs = cases(set);
    let socks: Vec<UdpSocket> = cs
        .iter()
        .map(|c| {
            let s = UdpSocket::bind(SocketAddrV4::new(bind, c.dport())).expect("bind rx");
            s.set_nonblocking(true).unwrap();
            s
        })
        .collect();
    println!("RX READY {} sockets", socks.len());
    let mut got: Vec<Vec<Vec<u8>>> = vec![Vec::new(); cs.len()];
    let mut buf = vec![0u8; 70000];
    let mut last = Instant::now();
    loop {
        let mut pfds: Vec<libc::pollfd> = socks.iter().map(|s| libc::pollfd { fd: s.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
        let r = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as _, 200) };
        if r > 0 {
            for (i, p) in pfds.iter().enumerate() {
                if p.revents & libc::POLLIN != 0 {
                    while let Ok((n, _)) = socks[i].recv_from(&mut buf) {
                        got[i].push(buf[..n].to_vec());
                        last = Instant::now();
                    }
                }
            }
        }
        let started = got.iter().any(|g| !g.is_empty());
        let done = cs.iter().enumerate().all(|(i, c)| complete(c, &got[i], rounds));
        if done {
            std::thread::sleep(Duration::from_millis(500));
            drain(&socks, &mut got, &mut buf);
            break;
        }
        if started && last.elapsed() > Duration::from_secs(idle) {
            break;
        }
        if !started && last.elapsed() > Duration::from_secs(90) {
            break;
        }
    }
    let (mut ok_cases, mut bad_cases) = (0, 0);
    for (i, c) in cs.iter().enumerate() {
        let exp = c.payload();
        let g = &got[i];
        let (ok, detail) = if c.gso {
            let cat: Vec<u8> = g.iter().flat_map(|d| d.iter().copied()).collect();
            let segs: Vec<usize> = g.iter().map(|d| d.len()).collect();
            let ok = cat == exp && !(g.is_empty() && !exp.is_empty());
            (ok, format!("datagrams={} sizes={} total={} exp_total={}", g.len(), fmt_sizes(&segs), cat.len(), exp.len()))
        } else {
            let mism: Vec<String> = g.iter().enumerate().filter(|(_, d)| **d != exp).map(|(k, d)| format!("#{k}:len={} fnv={:016x}", d.len(), fnv(d))).collect();
            let ok = g.len() == rounds && mism.is_empty();
            (ok, format!("got={}/{} mismatches=[{}] lens={}", g.len(), rounds, mism.join(","), fmt_sizes(&g.iter().map(|d| d.len()).collect::<Vec<_>>())))
        };
        if ok {
            ok_cases += 1
        } else {
            bad_cases += 1
        }
        println!("RESULT {} id={} {:?} n={} wire={} gso={} {}", if ok { "BYTE-EXACT" } else { "FAIL" }, c.id, c.kind, c.n, c.wire().len(), c.gso, detail);
    }
    println!("SUMMARY byte-exact={ok_cases} fail={bad_cases} total={}", cs.len());
}

fn fmt_sizes(v: &[usize]) -> String {
    if v.len() <= 6 {
        return format!("{v:?}");
    }
    format!("[{}, {}, {}, ..., {}] ({} items)", v[0], v[1], v[2], v[v.len() - 1], v.len())
}

fn drain(socks: &[UdpSocket], got: &mut [Vec<Vec<u8>>], buf: &mut [u8]) {
    for (i, s) in socks.iter().enumerate() {
        while let Ok((n, _)) = s.recv_from(buf) {
            got[i].push(buf[..n].to_vec());
        }
    }
}

fn complete(c: &Case, g: &[Vec<u8>], rounds: usize) -> bool {
    if c.gso {
        let t: usize = g.iter().map(|d| d.len()).sum();
        t >= c.n && (c.n > 0 || !g.is_empty())
    } else {
        g.len() >= rounds
    }
}
