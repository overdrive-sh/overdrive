// Throwaway guest program (std only, static musl). Probes public-name DNS from a VM guest.
//   dns-probe <mesh-name>
// Hypothesis: the node responder at the guest gateway drops non-mesh queries, so the
// default resolver times out on public names. Each step prints one "probe: ..." line.
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

fn query(id: u16, name: &str) -> Vec<u8> {
    let mut q = Vec::with_capacity(12 + name.len() + 6);
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]); // RD, QDCOUNT=1
    for label in name.trim_end_matches('.').split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 1, 0, 1]); // root, QTYPE=A, QCLASS=IN
    q
}

fn raw(label: &str, server: Ipv4Addr, name: &str, tries: u32) {
    for n in 1..=tries {
        let start = Instant::now();
        let s = UdpSocket::bind("0.0.0.0:0").unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let target = SocketAddr::V4(SocketAddrV4::new(server, 53));
        if let Err(e) = s.send_to(&query(0x4242 + n as u16, name), target) {
            println!("probe: {label} try={n} send error: {e}");
            continue;
        }
        let mut buf = [0u8; 1500];
        match s.recv_from(&mut buf) {
            Ok((k, from)) if k >= 12 => {
                let rcode = buf[3] & 0x0f;
                let an = u16::from_be_bytes([buf[6], buf[7]]);
                println!("probe: {label} try={n} REPLY from={from} bytes={k} rcode={rcode} ancount={an} elapsed_ms={}", start.elapsed().as_millis());
                return;
            }
            Ok((k, from)) => println!("probe: {label} try={n} SHORT reply from={from} bytes={k}"),
            Err(e) => println!("probe: {label} try={n} NO-REPLY kind={:?} elapsed_ms={}", e.kind(), start.elapsed().as_millis()),
        }
    }
}

fn resolve(label: &str, host: &str) {
    let start = Instant::now();
    match (host, 443).to_socket_addrs() {
        Ok(addrs) => println!("probe: {label} RESOLVED {:?} elapsed_ms={}", addrs.collect::<Vec<_>>(), start.elapsed().as_millis()),
        Err(e) => println!("probe: {label} FAILED err={e} elapsed_ms={}", start.elapsed().as_millis()),
    }
}

fn main() {
    let mesh = std::env::args().nth(1).unwrap_or_default();
    let resolv = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_else(|e| format!("<unreadable: {e}>"));
    println!("probe: resolv.conf = {:?}", resolv);
    let gw: Ipv4Addr = resolv
        .lines()
        .find_map(|l| l.strip_prefix("nameserver ").and_then(|a| a.trim().parse().ok()))
        .expect("nameserver line");
    std::thread::sleep(Duration::from_secs(5)); // let the backend row and name index publish
    println!("probe: begin gateway={gw}");
    resolve("A1 default-resolver example.com", "example.com");
    resolve("A2 default-resolver api.anthropic.com", "api.anthropic.com");
    raw("B  raw gateway example.com", gw, "example.com", 3);
    raw("C  raw 1.1.1.1 example.com", Ipv4Addr::new(1, 1, 1, 1), "example.com", 3);
    raw("D  raw gateway mesh-name", gw, &mesh, 5);
    resolve("D2 default-resolver mesh-name", &mesh);
    let start = Instant::now();
    match TcpStream::connect_timeout(&"1.1.1.1:443".parse().unwrap(), Duration::from_secs(5)) {
        Ok(_) => println!("probe: E  tcp 1.1.1.1:443 CONNECTED elapsed_ms={}", start.elapsed().as_millis()),
        Err(e) => println!("probe: E  tcp 1.1.1.1:443 FAILED err={e} elapsed_ms={}", start.elapsed().as_millis()),
    }
    println!("probe: done");
    std::io::stdout().flush().ok();
    loop { std::thread::sleep(Duration::from_secs(3600)); }
}
