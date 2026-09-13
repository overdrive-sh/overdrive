// Throwaway GH295 mechanism probe. Nothing here is a production API.
// Socket options and the kTLS crypto layout are copied from the production
// mtls_intercept / mtls/ktls primitives; no overdrive crate is linked.
use std::ffi::CString;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::os::fd::{AsRawFd, FromRawFd};
use std::sync::Arc;
use std::time::{Duration, Instant};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, ConnectionTrafficSecrets, ExtractedSecrets, RootCertStore, ServerConfig, ServerConnection};

const REQUEST: &[u8] = b"GH295-PLAINTEXT-GUEST-REQUEST-7\n";
const RESPONSE: &[u8] = b"GH295-BYTE-DISTINCT-PEER-RESPONSE-42\n";

fn check(rc: i32, label: &str) {
    assert!(rc >= 0, "{label}: {}", std::io::Error::last_os_error());
}

fn addr(ip: &str, port: u16) -> libc::sockaddr_in {
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sa.sin_family = libc::AF_INET as u16;
    sa.sin_port = port.to_be();
    sa.sin_addr.s_addr = u32::from_ne_bytes(ip.parse::<Ipv4Addr>().unwrap().octets());
    sa
}

fn option(fd: i32, level: i32, name: i32, value: i32) {
    check(unsafe { libc::setsockopt(fd, level, name, (&value as *const i32).cast(), 4) }, "setsockopt");
}

fn transparent_listener(port: u16) -> TcpListener {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    check(fd, "socket");
    option(fd, libc::SOL_IP, libc::IP_TRANSPARENT, 1);
    option(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, 1);
    let sa = addr("127.0.0.1", port);
    check(unsafe { libc::bind(fd, (&sa as *const libc::sockaddr_in).cast(), 16) }, "bind transparent");
    check(unsafe { libc::listen(fd, 16) }, "listen");
    unsafe { TcpListener::from_raw_fd(fd) }
}

fn connect_marked(dst: SocketAddrV4, mark: i32) -> TcpStream {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    check(fd, "socket marked");
    option(fd, libc::SOL_SOCKET, libc::SO_MARK, mark);
    let sa = addr(&dst.ip().to_string(), dst.port());
    check(unsafe { libc::connect(fd, (&sa as *const libc::sockaddr_in).cast(), 16) }, "connect marked");
    let tcp = unsafe { TcpStream::from_raw_fd(fd) };
    tcp.set_read_timeout(Some(Duration::from_secs(12))).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(12))).unwrap();
    tcp
}

fn original(tcp: &TcpStream, leg: &str) -> SocketAddrV4 {
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = 16;
    check(unsafe { libc::getsockname(tcp.as_raw_fd(), (&mut sa as *mut libc::sockaddr_in).cast(), &mut len) }, "getsockname");
    let original = SocketAddrV4::new(Ipv4Addr::from(sa.sin_addr.s_addr.to_ne_bytes()), u16::from_be(sa.sin_port));
    println!("{leg} GETSOCKNAME_ORIGINAL={original} PEER={}", tcp.peer_addr().unwrap());
    assert_eq!(original, "10.95.0.3:9000".parse().unwrap());
    original
}

fn arm(tcp: &TcpStream, secrets: ExtractedSecrets, leg: &str) {
    let fd = tcp.as_raw_fd();
    check(unsafe { libc::setsockopt(fd, libc::SOL_TCP, libc::TCP_ULP, c"tls".as_ptr().cast(), 3) }, "TCP_ULP tls");
    for (direction, (seq, traffic)) in [(libc::TLS_TX, secrets.tx), (libc::TLS_RX, secrets.rx)] {
        let ConnectionTrafficSecrets::Aes256Gcm { key, iv } = traffic else { panic!("expected AES256 GCM") };
        // Linux tls12_crypto_info_aes_gcm_256: version, cipher, iv, key,
        // salt, rec_seq. Version 0x0304 selects TLS1.3 despite UAPI name.
        let mut info = [0u8; 56];
        info[0..2].copy_from_slice(&0x0304u16.to_ne_bytes());
        info[2..4].copy_from_slice(&52u16.to_ne_bytes());
        info[4..12].copy_from_slice(&iv.as_ref()[4..12]);
        info[12..44].copy_from_slice(key.as_ref());
        info[44..48].copy_from_slice(&iv.as_ref()[0..4]);
        info[48..56].copy_from_slice(&seq.to_be_bytes());
        check(unsafe { libc::setsockopt(fd, libc::SOL_TLS, direction, info.as_ptr().cast(), 56) }, "kTLS crypto");
        println!("{leg} KTLS_ARMED direction={direction} tls_version=0x0304 cipher=52 seq={seq}");
    }
}

fn handshake_client(conn: &mut ClientConnection, tcp: &mut TcpStream) {
    loop {
        while conn.wants_write() { conn.write_tls(tcp).unwrap(); }
        if !conn.is_handshaking() { break; }
        assert!(conn.read_tls(tcp).unwrap() > 0);
        conn.process_new_packets().unwrap();
    }
}

fn handshake_server(conn: &mut ServerConnection, tcp: &mut TcpStream) {
    loop {
        while conn.wants_write() { conn.write_tls(tcp).unwrap(); }
        if !conn.is_handshaking() { break; }
        assert!(conn.read_tls(tcp).unwrap() > 0);
        conn.process_new_packets().unwrap();
    }
}

fn host() {
    let start = Instant::now();
    let mut provider = rustls::crypto::ring::default_provider();
    provider.cipher_suites = vec![rustls::crypto::ring::cipher_suite::TLS13_AES_256_GCM_SHA384];
    provider.install_default().unwrap();
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
    let source_key = rcgen::KeyPair::generate().unwrap();
    let peer_key = rcgen::KeyPair::generate().unwrap();
    let source_cert = rcgen::CertificateParams::new(vec!["agent-source".into()]).unwrap().signed_by(&source_key, &issuer).unwrap();
    let peer_cert = rcgen::CertificateParams::new(vec!["agent-peer".into()]).unwrap().signed_by(&peer_key, &issuer).unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(ca.der().clone()).unwrap();
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots.clone())).build().unwrap();
    let mut server = ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![peer_cert.der().clone()], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(peer_key.serialize_der()))).unwrap();
    server.enable_secret_extraction = true;
    server.send_tls13_tickets = 0;
    let mut client = ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(roots)
        .with_client_auth_cert(vec![source_cert.der().clone()], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(source_key.serialize_der()))).unwrap();
    client.enable_secret_extraction = true;

    let f = transparent_listener(15294);
    let c = transparent_listener(15295);
    let dns = UdpSocket::bind("10.95.0.1:53").unwrap();
    std::thread::spawn(move || {
        loop {
            let mut query = [0u8; 512];
            let (n, from) = dns.recv_from(&mut query).unwrap();
            let q = &query[..n];
            let mut end = 12;
            let mut labels = Vec::new();
            while q[end] != 0 {
                let count = q[end] as usize;
                labels.push(std::str::from_utf8(&q[end+1..end+1+count]).unwrap());
                end += 1 + count;
            }
            let qtype = u16::from_be_bytes([q[end+1], q[end+2]]);
            let name = labels.join(".");
            println!("DNS QUERY source={from} name={name} qtype={qtype} HOME=shared-bridge:no-host-netns-resolv-conf");
            assert_eq!(name, "peer.mesh");
            let answer = qtype == 1;
            let mut response = q[..end+5].to_vec();
            response[2..4].copy_from_slice(&0x8180u16.to_be_bytes());
            response[6..8].copy_from_slice(&u16::from(answer).to_be_bytes());
            response[8..12].fill(0);
            if answer { response.extend_from_slice(&[0xc0, 0x0c, 0,1, 0,1, 0,0,0,1, 0,4, 10,95,0,3]); }
            dns.send_to(&response, from).unwrap();
            println!("DNS ANSWER source={from} name={name} qtype={qtype} ipv4={answer}");
        }
    });

    let armed = Arc::new(std::sync::Barrier::new(2));
    let c_armed = armed.clone();
    let c_thread = std::thread::spawn(move || {
        let (mut leg_c, _) = c.accept().unwrap();
        leg_c.set_read_timeout(Some(Duration::from_secs(12))).unwrap();
        let dst = original(&leg_c, "LEG_C owner=tap295b");
        let mut conn = ServerConnection::new(Arc::new(server)).unwrap();
        handshake_server(&mut conn, &mut leg_c);
        println!("LEG_C MTLS_AUTHENTICATED client_certificates={} version={:?}", conn.peer_certificates().unwrap().len(), conn.protocol_version());
        arm(&leg_c, conn.dangerous_extract_secrets().unwrap(), "LEG_C");
        let mut leg_s = connect_marked(dst, 0x2952);
        println!("LEG_S CLEAR_TO_GUEST local={} peer={} mark=0x2952", leg_s.local_addr().unwrap(), leg_s.peer_addr().unwrap());
        c_armed.wait();
        let mut req = vec![0; REQUEST.len()];
        leg_c.read_exact(&mut req).unwrap();
        assert_eq!(req, REQUEST);
        leg_s.write_all(&req).unwrap();
        let mut reply = vec![0; RESPONSE.len()];
        leg_s.read_exact(&mut reply).unwrap();
        assert_eq!(reply, RESPONSE);
        leg_c.write_all(&reply).unwrap();
        println!("LEG_C COMPLETE request_bytes={} response_bytes={}", req.len(), reply.len());
        std::thread::sleep(Duration::from_millis(400));
    });
    println!("HOST READY legF=127.0.0.1:15294 legC=127.0.0.1:15295 dns=10.95.0.1:53");
    let (mut leg_f, _) = f.accept().unwrap();
    leg_f.set_read_timeout(Some(Duration::from_secs(12))).unwrap();
    let dst = original(&leg_f, "LEG_F owner=tap295a");
    let mut leg_b = connect_marked(dst, 0x2951);
    println!("LEG_B WIRE local={} peer={} mark=0x2951", leg_b.local_addr().unwrap(), leg_b.peer_addr().unwrap());
    let mut conn = ClientConnection::new(Arc::new(client), ServerName::try_from("agent-peer").unwrap()).unwrap();
    handshake_client(&mut conn, &mut leg_b);
    println!("LEG_B MTLS_AUTHENTICATED server_certificates={} version={:?}", conn.peer_certificates().unwrap().len(), conn.protocol_version());
    arm(&leg_b, conn.dangerous_extract_secrets().unwrap(), "LEG_B");
    armed.wait();
    let mut req = vec![0; REQUEST.len()];
    leg_f.read_exact(&mut req).unwrap();
    assert_eq!(req, REQUEST);
    leg_b.write_all(&req).unwrap();
    let mut reply = vec![0; RESPONSE.len()];
    leg_b.read_exact(&mut reply).unwrap();
    assert_eq!(reply, RESPONSE);
    leg_f.write_all(&reply).unwrap();
    println!("LEG_F COMPLETE request_bytes={} response_bytes={}", req.len(), reply.len());
    c_thread.join().unwrap();
    println!("HOST COMPLETE elapsed_seconds={:.6}", start.elapsed().as_secs_f64());
    std::thread::sleep(Duration::from_millis(400));
}

fn guest() {
    check(unsafe { libc::mount(c"proc".as_ptr(), c"/proc".as_ptr(), c"proc".as_ptr(), 0, std::ptr::null()) }, "mount proc");
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap();
    let server = cmdline.contains("spike_role=server");
    let ip = if server { "10.95.0.3" } else { "10.95.0.2" };
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    check(fd, "guest control socket");
    for (nic, address) in [("lo", "127.0.0.1"), ("eth0", ip)] {
        let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
        for (target, source) in ifr.ifr_name.iter_mut().zip(nic.bytes()) { *target = source as i8; }
        let sa = addr(address, 0);
        unsafe { std::ptr::copy_nonoverlapping((&sa as *const libc::sockaddr_in).cast::<u8>(), std::ptr::addr_of_mut!(ifr.ifr_ifru).cast::<u8>(), 16); }
        check(unsafe { libc::ioctl(fd, libc::SIOCSIFADDR as _, &ifr) }, "guest IP");
        let mask = addr(if nic == "lo" { "255.0.0.0" } else { "255.255.255.0" }, 0);
        unsafe { std::ptr::copy_nonoverlapping((&mask as *const libc::sockaddr_in).cast::<u8>(), std::ptr::addr_of_mut!(ifr.ifr_ifru).cast::<u8>(), 16); }
        check(unsafe { libc::ioctl(fd, libc::SIOCSIFNETMASK as _, &ifr) }, "guest netmask");
        check(unsafe { libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr) }, "guest flags read");
        unsafe { ifr.ifr_ifru.ifru_flags |= libc::IFF_UP as i16; }
        check(unsafe { libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &ifr) }, "guest link up");
    }
    let mut route: libc::rtentry = unsafe { std::mem::zeroed() };
    route.rt_dst = unsafe { std::mem::transmute(addr("0.0.0.0", 0)) };
    route.rt_genmask = unsafe { std::mem::transmute(addr("0.0.0.0", 0)) };
    route.rt_gateway = unsafe { std::mem::transmute(addr("10.95.0.1", 0)) };
    route.rt_flags = (libc::RTF_UP | libc::RTF_GATEWAY) as u16;
    let nic = CString::new("eth0").unwrap();
    route.rt_dev = nic.as_ptr().cast_mut();
    check(unsafe { libc::ioctl(fd, libc::SIOCADDRT as _, &route) }, "guest default route");
    check(unsafe { libc::close(fd) }, "guest close control socket");
    println!("GUEST CONFIG ip={ip}/24 gateway=10.95.0.1 role={} PID={} NO_CREDENTIALS", if server {"server"} else {"client"}, unsafe {libc::getpid()});
    if server {
        let listener = TcpListener::bind("0.0.0.0:9000").unwrap();
        println!("GUEST SERVER READY");
        let (mut tcp, from) = listener.accept().unwrap();
        println!("GUEST SERVER ACCEPT peer={from}");
        let mut req = vec![0; REQUEST.len()];
        tcp.read_exact(&mut req).unwrap();
        assert_eq!(req, REQUEST);
        println!("GUEST SERVER REQUEST={}", std::str::from_utf8(&req).unwrap());
        tcp.write_all(RESPONSE).unwrap();
        println!("GUEST SERVER RESPONSE={}", std::str::from_utf8(RESPONSE).unwrap());
        std::thread::sleep(Duration::from_millis(800));
    } else {
        let start = Instant::now();
        println!("GUEST RESOLV_CONF={}", std::fs::read_to_string("/etc/resolv.conf").unwrap().trim());
        let address = ("peer.mesh", 9000).to_socket_addrs().unwrap().find(SocketAddr::is_ipv4).unwrap();
        println!("GUEST DNS RESOLVED peer.mesh={address}");
        let mut tcp = TcpStream::connect_timeout(&address, Duration::from_secs(12)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(12))).unwrap();
        tcp.write_all(REQUEST).unwrap();
        println!("GUEST PLAINTEXT REQUEST={}", std::str::from_utf8(REQUEST).unwrap());
        let mut response = vec![0; RESPONSE.len()];
        tcp.read_exact(&mut response).unwrap();
        assert_eq!(response, RESPONSE);
        println!("GUEST ROUNDTRIP SUCCESS RESPONSE={} elapsed_seconds={:.6}", std::str::from_utf8(&response).unwrap().trim(), start.elapsed().as_secs_f64());
    }
    println!("GUEST EXIT=0");
    std::io::stdout().flush().unwrap();
    unsafe { libc::sync(); libc::reboot(libc::RB_POWER_OFF); }
    loop { std::thread::park(); }
}

fn main() {
    if unsafe { libc::getpid() } == 1 { guest(); } else { host(); }
}
