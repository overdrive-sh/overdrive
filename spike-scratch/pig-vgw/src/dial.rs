//! vgwdial: userspace side of the spike. Prepares sockets per the design (SO_MARK, bind lo,
//! SO_COOKIE, intent registration), connects, reads receipts, optionally attempts TLS.
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    os::fd::{FromRawFd, RawFd},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use aya::maps::{HashMap, Map, MapData};

const PIN: &str = "/sys/fs/bpf/pigvgw";

#[repr(C)]
#[derive(Clone, Copy)]
struct ServiceKey {
    vip_host: u32,
    port_host: u16,
    proto: u8,
    pad: u8,
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Receipt {
    verdict: u32,
    reason: u32,
    backend_id: u32,
    ip_host: u32,
    port_host: u32,
    healthy: u32,
}
unsafe impl aya::Pod for ServiceKey {}
unsafe impl aya::Pod for Receipt {}

fn pinned<K: aya::Pod, V: aya::Pod>(name: &str) -> HashMap<MapData, K, V> {
    let d = MapData::from_pin(format!("{PIN}/{name}")).unwrap_or_else(|e| panic!("pin {name}: {e}"));
    HashMap::try_from(Map::HashMap(d)).expect("hashmap")
}

fn join_cgroup(path: &str) {
    std::fs::write(format!("{path}/cgroup.procs"), std::process::id().to_string()).expect("join cgroup");
}

fn sock() -> RawFd {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    assert!(fd >= 0, "socket");
    fd
}

fn cookie(fd: RawFd) -> u64 {
    let mut c: u64 = 0;
    let mut l = 8 as libc::socklen_t;
    let rc = unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_COOKIE, (&raw mut c).cast(), &mut l) };
    assert_eq!(rc, 0, "SO_COOKIE");
    c
}

fn setopt(fd: RawFd, lvl: i32, name: i32, v: i32) {
    let rc = unsafe { libc::setsockopt(fd, lvl, name, (&raw const v).cast(), 4) };
    assert_eq!(rc, 0, "setsockopt {name}: {}", std::io::Error::last_os_error());
}

fn bind_lo(fd: RawFd) {
    let a = libc::sockaddr_in {
        sin_family: libc::AF_INET as u16,
        sin_port: 0,
        sin_addr: libc::in_addr { s_addr: u32::from(Ipv4Addr::LOCALHOST).to_be() },
        sin_zero: [0; 8],
    };
    let rc = unsafe { libc::bind(fd, (&raw const a).cast(), 16) };
    assert_eq!(rc, 0, "bind lo: {}", std::io::Error::last_os_error());
}

fn connect(fd: RawFd, ip: Ipv4Addr, port: u16, to_ms: u64) -> Result<(), i32> {
    let tv = libc::timeval { tv_sec: (to_ms / 1000) as i64, tv_usec: ((to_ms % 1000) * 1000) as i64 };
    unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDTIMEO, (&raw const tv).cast(), 16) };
    let a = libc::sockaddr_in {
        sin_family: libc::AF_INET as u16,
        sin_port: port.to_be(),
        sin_addr: libc::in_addr { s_addr: u32::from(ip).to_be() },
        sin_zero: [0; 8],
    };
    let rc = unsafe { libc::connect(fd, (&raw const a).cast(), 16) };
    if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(-1)) }
}

fn peer_local(fd: RawFd) -> String {
    let mut a: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut l = 16 as libc::socklen_t;
    let p = unsafe { libc::getpeername(fd, (&raw mut a).cast(), &mut l) };
    let peer = if p == 0 {
        format!("{}:{}", Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr)), u16::from_be(a.sin_port))
    } else {
        "-".into()
    };
    let mut b: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut l2 = 16 as libc::socklen_t;
    let s = unsafe { libc::getsockname(fd, (&raw mut b).cast(), &mut l2) };
    let loc = if s == 0 {
        format!("{}:{}", Ipv4Addr::from(u32::from_be(b.sin_addr.s_addr)), u16::from_be(b.sin_port))
    } else {
        "-".into()
    };
    format!("local={loc} peer={peer}")
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
}

fn reason_name(v: u32, r: u32) -> &'static str {
    match (v, r) {
        (1, _) => "Selected",
        (2, 1) => "NoBackend/no-outer-entry",
        (2, 2) => "NoBackend/empty-slot",
        (2, 3) => "NoBackend/missing-backend-entry",
        (2, 4) => "NoBackend/healthy==0",
        _ => "?",
    }
}

type Dial = (RawFd, Result<(), i32>, u64, Option<Receipt>);

/// One registered dial. `mark`/`bindlo` toggle the design's preparation steps.
fn dial_once(vip: Ipv4Addr, port: u16, mark: bool, bindlo: bool, register: bool, to_ms: u64) -> Dial {
    let fd = sock();
    if mark {
        setopt(fd, libc::SOL_SOCKET, libc::SO_MARK, 1);
    }
    if bindlo {
        setopt(fd, libc::SOL_IP, libc::IP_BIND_ADDRESS_NO_PORT, 1);
        bind_lo(fd);
    }
    let c = cookie(fd);
    let mut intent: HashMap<MapData, u64, ServiceKey> = pinned("INTENT");
    let mut rec: HashMap<MapData, u64, Receipt> = pinned("RECEIPT");
    let _ = rec.remove(&c);
    if register {
        intent.insert(c, ServiceKey { vip_host: u32::from(vip), port_host: port, proto: 6, pad: 0 }, 0).unwrap();
    }
    let r = connect(fd, vip, port, to_ms);
    let receipt = rec.get(&c, 0).ok();
    if register {
        let _ = intent.remove(&c);
    }
    let _ = rec.remove(&c);
    (fd, r, c, receipt)
}

fn tls_attempt(fd: RawFd) {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, SignatureScheme};
    #[derive(Debug)]
    struct Dump;
    impl ServerCertVerifier for Dump {
        fn verify_server_cert(
            &self,
            end: &CertificateDer<'_>,
            inter: &[CertificateDer<'_>],
            _n: &ServerName<'_>,
            _o: &[u8],
            _t: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            std::fs::write("/tmp/vgw-leaf.der", end.as_ref()).unwrap();
            for (i, c) in inter.iter().enumerate() {
                std::fs::write(format!("/tmp/vgw-inter{i}.der"), c.as_ref()).unwrap();
            }
            println!("TLS server leaf received ({} bytes), {} intermediates", end.len(), inter.len());
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(&self, _m: &[u8], _c: &CertificateDer<'_>, _d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(&self, _m: &[u8], _c: &CertificateDer<'_>, _d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![SignatureScheme::ECDSA_NISTP256_SHA256, SignatureScheme::ECDSA_NISTP384_SHA384, SignatureScheme::ED25519, SignatureScheme::RSA_PSS_SHA256]
        }
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Dump))
        .with_no_client_auth();
    let mut conn = rustls::ClientConnection::new(Arc::new(cfg), ServerName::try_from("gateway-probe.invalid").unwrap()).unwrap();
    let mut s = unsafe { TcpStream::from_raw_fd(fd) };
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut tls = rustls::Stream::new(&mut conn, &mut s);
    let r = tls.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    println!("TLS write_all: {r:?}");
    let mut buf = [0u8; 512];
    let rr = tls.read(&mut buf);
    println!("TLS read: {rr:?}");
    println!("TLS negotiated version={:?} suite={:?}", conn.protocol_version(), conn.negotiated_cipher_suite().map(|c| c.suite()));
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let flag = |n: &str| a.iter().any(|x| x == n);
    let val = |n: &str, d: &str| a.iter().position(|x| x == n).and_then(|i| a.get(i + 1)).cloned().unwrap_or(d.to_string());
    if let Some(cg) = a.iter().position(|x| x == "--cgroup").and_then(|i| a.get(i + 1)) {
        join_cgroup(cg);
    }
    match a[1].as_str() {
        // V-GW-5: pre-connect SO_COOKIE == hook-side bpf_get_socket_cookie
        "cookie" => {
            let n: u16 = val("--n", "20").parse().unwrap();
            let base: u16 = 41000;
            let mut ok = 0;
            for i in 0..n {
                let p = base + i;
                let l = TcpListener::bind(("127.0.0.1", p)).unwrap();
                let fd = sock();
                if flag("--mark-bind") {
                    setopt(fd, libc::SOL_SOCKET, libc::SO_MARK, 1);
                    setopt(fd, libc::SOL_IP, libc::IP_BIND_ADDRESS_NO_PORT, 1);
                    bind_lo(fd);
                }
                let c1 = cookie(fd);
                let c1b = cookie(fd);
                connect(fd, Ipv4Addr::LOCALHOST, p, 2000).expect("connect");
                let c2 = cookie(fd);
                let m: HashMap<MapData, u32, u64> = pinned("COOKIE_BY_PORT");
                let hook = m.get(&u32::from(p), 0).unwrap_or(0);
                let eq = c1 == hook && c1 == c1b && c1 == c2;
                if eq {
                    ok += 1;
                }
                println!("port={p} pre_connect={c1:#x} pre_again={c1b:#x} post_connect={c2:#x} hook={hook:#x} equal={eq}");
                unsafe { libc::close(fd) };
                drop(l);
            }
            println!("COOKIE_RESULT {ok}/{n} equal");
        }
        // V-GW-2: plain connect from this process's cgroup (counted by the vgw_cookie hook)
        "conn" => {
            let ip: Ipv4Addr = val("--ip", "127.0.0.1").parse().unwrap();
            let port: u16 = val("--port", "0").parse().unwrap();
            let fd = sock();
            let r = connect(fd, ip, port, 500);
            println!("conn {ip}:{port} -> {r:?}");
        }
        "dial" => {
            let vip: Ipv4Addr = val("--vip", "10.96.0.2").parse().unwrap();
            let port: u16 = val("--port", "18080").parse().unwrap();
            let (fd, r, c, rc) = dial_once(
                vip,
                port,
                !flag("--no-mark"),
                !flag("--no-bind"),
                !flag("--no-register"),
                val("--to", "3000").parse().unwrap(),
            );
            println!(
                "DIAL cookie={c:#x} connect={r:?} receipt={rc:?} {}",
                rc.map(|x| reason_name(x.verdict, x.reason)).unwrap_or("none")
            );
            if r.is_ok() {
                println!("sock {}", peer_local(fd));
                if flag("--tls") {
                    tls_attempt(fd);
                }
                let hold: u64 = val("--hold", "0").parse().unwrap();
                std::thread::sleep(Duration::from_secs(hold));
            }
        }
        "poll" => {
            let vip: Ipv4Addr = val("--vip", "10.96.0.2").parse().unwrap();
            let port: u16 = val("--port", "18080").parse().unwrap();
            let dur: u64 = val("--secs", "300").parse().unwrap();
            let t0 = Instant::now();
            let mut last = String::new();
            while t0.elapsed().as_secs() < dur {
                let (fd, r, _c, rc) = dial_once(vip, port, true, true, true, 300);
                let key = format!(
                    "connect={r:?} {} be={}",
                    rc.map(|x| reason_name(x.verdict, x.reason)).unwrap_or("none"),
                    rc.map(|x| format!("{}:{}/h{}", Ipv4Addr::from(x.ip_host), x.port_host, x.healthy)).unwrap_or_default()
                );
                if key != last {
                    println!("{} +{:>6}ms {key}", now_ms(), t0.elapsed().as_millis());
                    last = key;
                }
                unsafe { libc::close(fd) };
                std::thread::sleep(Duration::from_millis(50));
            }
            println!("poll done");
        }
        _ => panic!("subcommand"),
    }
}
