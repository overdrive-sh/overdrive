// Throwaway guest program (std only). Built twice: static musl and dynamic glibc.
//   dns-probe all <mesh-name>   V-2 query phase, mesh control, V-5 TCP/53, V-3 (then runs the glibc build)
//   dns-probe v3 <label>        V-3 only
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

fn ms(t: Instant) -> String { format!("{:.2}", t.elapsed().as_secs_f64() * 1000.0) }

fn query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut q = Vec::new();
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for l in name.trim_end_matches('.').split('.') { q.push(l.len() as u8); q.extend_from_slice(l.as_bytes()); }
    q.push(0); q.extend_from_slice(&qtype.to_be_bytes()); q.extend_from_slice(&[0, 1]);
    q
}

fn skip_name(b: &[u8], mut p: usize) -> usize {
    while p < b.len() {
        let l = b[p] as usize;
        if l == 0 { return p + 1; }
        if l & 0xc0 == 0xc0 { return p + 2; }
        p += 1 + l;
    }
    p
}

fn answers(b: &[u8]) -> Vec<IpAddr> {
    let mut out = vec![];
    if b.len() < 12 { return out; }
    let (qd, an) = (u16::from_be_bytes([b[4], b[5]]), u16::from_be_bytes([b[6], b[7]]));
    let mut p = 12;
    for _ in 0..qd { p = skip_name(b, p) + 4; }
    for _ in 0..an {
        p = skip_name(b, p);
        if p + 10 > b.len() { break; }
        let t = u16::from_be_bytes([b[p], b[p + 1]]);
        let rl = u16::from_be_bytes([b[p + 8], b[p + 9]]) as usize;
        p += 10;
        if p + rl > b.len() { break; }
        match (t, rl) {
            (1, 4) => out.push(IpAddr::from([b[p], b[p + 1], b[p + 2], b[p + 3]])),
            (28, 16) => { let mut a = [0u8; 16]; a.copy_from_slice(&b[p..p + 16]); out.push(IpAddr::from(a)); }
            _ => {}
        }
        p += rl;
    }
    out
}

fn raw(label: &str, server: Ipv4Addr, name: &str, qtype: u16, tries: u32, to: u64) -> Vec<IpAddr> {
    for n in 1..=tries {
        let t = Instant::now();
        let s = UdpSocket::bind("0.0.0.0:0").unwrap();
        s.set_read_timeout(Some(Duration::from_secs(to))).unwrap();
        s.send_to(&query(0x4242 + n as u16, name, qtype), SocketAddr::V4(SocketAddrV4::new(server, 53))).unwrap();
        let mut buf = [0u8; 1500];
        match s.recv_from(&mut buf) {
            Ok((k, from)) if k >= 12 => {
                let a = answers(&buf[..k]);
                println!("probe: {label} try={n} REPLY from={from} bytes={k} rcode={} ancount={} addrs={a:?} elapsed_ms={}", buf[3] & 0xf, u16::from_be_bytes([buf[6], buf[7]]), ms(t));
                return a;
            }
            Ok((k, _)) => println!("probe: {label} try={n} SHORT bytes={k}"),
            Err(e) => println!("probe: {label} try={n} NO-REPLY kind={:?} elapsed_ms={}", e.kind(), ms(t)),
        }
    }
    vec![]
}

fn read_or(p: &str) -> String { std::fs::read_to_string(p).unwrap_or_else(|e| format!("<unreadable: {e}>")) }

fn v5(gw: Ipv4Addr) {
    let target = SocketAddr::V4(SocketAddrV4::new(gw, 53));
    let q = query(0x5151, "example.com", 1);
    let framed = |q: &[u8]| { let mut f = (q.len() as u16).to_be_bytes().to_vec(); f.extend_from_slice(q); f };
    // No listener yet: what does the guest see TODAY?
    let t = Instant::now();
    match TcpStream::connect_timeout(&target, Duration::from_secs(8)) {
        Ok(mut s) => {
            println!("probe: V5 nolistener connect OK elapsed_ms={}", ms(t));
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let w = s.write_all(&framed(&q));
            let t2 = Instant::now();
            let mut b = [0u8; 64];
            println!("probe: V5 nolistener write={w:?} read={:?} elapsed_ms={}", s.read(&mut b), ms(t2));
        }
        Err(e) => println!("probe: V5 nolistener connect FAILED kind={:?} os={:?} err={e} elapsed_ms={}", e.kind(), e.raw_os_error(), ms(t)),
    }
    println!("probe: V5 nolistener done");
    for attempt in 1..=40 {
        std::thread::sleep(Duration::from_secs(2));
        let t = Instant::now();
        match TcpStream::connect_timeout(&target, Duration::from_secs(5)) {
            Err(e) => { println!("probe: V5 attempt={attempt} connect FAILED kind={:?} os={:?} elapsed_ms={}", e.kind(), e.raw_os_error(), ms(t)); continue; }
            Ok(mut s) => {
                println!("probe: V5 attempt={attempt} connect OK elapsed_ms={}", ms(t));
                s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                if let Err(e) = s.write_all(&framed(&q)) { println!("probe: V5 attempt={attempt} write FAILED {e}"); continue; }
                let mut len = [0u8; 2];
                if let Err(e) = s.read_exact(&mut len) { println!("probe: V5 attempt={attempt} read-len FAILED kind={:?} {e} elapsed_ms={}", e.kind(), ms(t)); continue; }
                let n = u16::from_be_bytes(len) as usize;
                let mut body = vec![0u8; n];
                if let Err(e) = s.read_exact(&mut body) { println!("probe: V5 attempt={attempt} read-body FAILED {e}"); continue; }
                let lit = body.windows(12).any(|w| w == b"GPD-V5-REPLY");
                println!("probe: V5 attempt={attempt} FRAMED REPLY len={n} id_echo={} litmus_GPD-V5-REPLY={lit} hex={:02x?} elapsed_ms={}", u16::from_be_bytes([body[0], body[1]]) == 0x5151, &body[..body.len().min(48)], ms(t));
                return;
            }
        }
    }
    println!("probe: V5 gave up");
}

fn v3(label: &str) {
    println!("probe: V3 {label} resolv.conf = {:?}", read_or("/etc/resolv.conf"));
    for name in ["www.google.com", "cloudflare.com", "example.com", "github.com"] {
        let a4 = raw(&format!("V3 {label} {name} rawA@1.1.1.1"), Ipv4Addr::new(1, 1, 1, 1), name, 1, 3, 3);
        let a6 = raw(&format!("V3 {label} {name} rawAAAA@1.1.1.1"), Ipv4Addr::new(1, 1, 1, 1), name, 28, 3, 3);
        println!("probe: V3 {label} {name} upstream A={} AAAA={}", a4.len(), a6.len());
        for round in 1..=3 {
            let t = Instant::now();
            let res: Vec<SocketAddr> = match (name, 443).to_socket_addrs() {
                Ok(it) => it.collect(),
                Err(e) => { println!("probe: V3 {label} {name} round={round} RESOLVE FAILED {e} elapsed_ms={}", ms(t)); continue; }
            };
            let resolve_ms = ms(t);
            // A-only baseline: connect to the first IPv4 address alone.
            let v4 = res.iter().find(|a| a.is_ipv4()).copied();
            let tb = Instant::now();
            let base = v4.map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(10)).map(|_| ()).map_err(|e| e.to_string()));
            let base_ms = ms(tb);
            // Sequential connect over the list in getaddrinfo order (what std/most clients do).
            let ts = Instant::now();
            let mut trace = vec![];
            let mut connected = None;
            for a in &res {
                let t1 = Instant::now();
                match TcpStream::connect_timeout(a, Duration::from_secs(10)) {
                    Ok(_) => { trace.push(format!("{a} OK {}ms", ms(t1))); connected = Some(*a); break; }
                    Err(e) => trace.push(format!("{a} ERR kind={:?} os={:?} {}ms", e.kind(), e.raw_os_error(), ms(t1))),
                }
            }
            println!("probe: V3 {label} {name} round={round} list={res:?} resolve_ms={resolve_ms} baseline_firstv4={base:?} baseline_ms={base_ms} seq_total_ms={} connected={connected:?} trace={trace:?}", ms(ts));
        }
    }
    println!("probe: V3 {label} done");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.get(1).map(String::as_str) == Some("v3") { v3(&a[2]); return; }
    let mesh = a.get(2).cloned().unwrap_or_default();
    let resolv = read_or("/etc/resolv.conf");
    println!("probe: resolv.conf = {resolv:?}");
    let gw: Ipv4Addr = resolv.lines().find_map(|l| l.strip_prefix("nameserver ").and_then(|x| x.trim().parse().ok())).expect("nameserver");
    println!("probe: if_inet6 = {:?}", read_or("/proc/net/if_inet6"));
    println!("probe: ipv6_route = {:?}", read_or("/proc/net/ipv6_route"));
    println!("probe: net/dev = {:?}", read_or("/proc/net/dev"));
    println!("probe: net/route = {:?}", read_or("/proc/net/route"));
    println!("probe: ipv6 disable sysctl = {:?}", read_or("/proc/sys/net/ipv6/conf/all/disable_ipv6"));
    std::thread::sleep(Duration::from_secs(20)); // harness runs the host-side V-2 probe and arms its observer
    println!("probe: V2 begin gateway={gw}");
    for i in 0..3 { raw(&format!("V2 raw gateway example.com #{i}"), gw, "example.com", 1, 1, 2); }
    println!("probe: V2 end");
    std::thread::sleep(Duration::from_secs(2));
    raw("D mesh-name via gateway (production responder alive)", gw, &mesh, 1, 3, 3);
    v5(gw);
    std::fs::write("/etc/resolv.conf", "nameserver 1.1.1.1\n").unwrap();
    println!("probe: DEVIATION guest resolv.conf rewritten to nameserver 1.1.1.1 (stand-in for forwarder pass-through)");
    v3("musl-static");
    let g = "/usr/local/bin/dns-probe-glibc";
    if std::path::Path::new(g).exists() {
        match std::process::Command::new(g).args(["v3", "glibc-dynamic"]).status() {
            Ok(s) => println!("probe: glibc child exit={s}"),
            Err(e) => println!("probe: glibc child spawn FAILED {e}"),
        }
    } else { println!("probe: glibc binary absent"); }
    println!("probe: done");
    std::io::stdout().flush().ok();
    loop { std::thread::sleep(Duration::from_secs(3600)); }
}
