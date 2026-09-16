// Throwaway GH #295 Part-B guest. The guest is identity-unaware and holds no credentials.
use std::ffi::CString;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

const REQUEST_ONE: &[u8] = b"GH295B-WARMUP-REQUEST\n";
const RESPONSE_ONE: &[u8] = b"GH295B-WARMUP-RESPONSE\n";
const REQUEST_TWO: &[u8] = b"GH295B-STEADY-STATE-REQUEST-ZEROCOPY-71\n";
const RESPONSE_TWO: &[u8] = b"GH295B-STEADY-STATE-RESPONSE-ZEROCOPY-93\n";

fn check(rc: i32, label: &str) {
    assert!(rc >= 0, "{label}: {}", std::io::Error::last_os_error());
}

fn sockaddr(ip: &str) -> libc::sockaddr_in {
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    addr.sin_family = libc::AF_INET as u16;
    addr.sin_addr.s_addr = u32::from_ne_bytes(ip.parse::<Ipv4Addr>().unwrap().octets());
    addr
}

fn configure_guest(ip: &str) {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    check(fd, "guest control socket");
    for (nic, address, mask) in [
        ("lo", "127.0.0.1", "255.0.0.0"),
        ("eth0", ip, "255.255.255.0"),
    ] {
        let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
        for (target, source) in ifr.ifr_name.iter_mut().zip(nic.bytes()) {
            *target = source as i8;
        }
        let address = sockaddr(address);
        unsafe {
            std::ptr::copy_nonoverlapping(
                (&address as *const libc::sockaddr_in).cast::<u8>(),
                std::ptr::addr_of_mut!(ifr.ifr_ifru).cast::<u8>(),
                16,
            );
        }
        check(unsafe { libc::ioctl(fd, libc::SIOCSIFADDR as _, &ifr) }, "guest IP");
        let mask = sockaddr(mask);
        unsafe {
            std::ptr::copy_nonoverlapping(
                (&mask as *const libc::sockaddr_in).cast::<u8>(),
                std::ptr::addr_of_mut!(ifr.ifr_ifru).cast::<u8>(),
                16,
            );
        }
        check(
            unsafe { libc::ioctl(fd, libc::SIOCSIFNETMASK as _, &ifr) },
            "guest netmask",
        );
        check(
            unsafe { libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr) },
            "guest flags read",
        );
        unsafe {
            ifr.ifr_ifru.ifru_flags |= libc::IFF_UP as i16;
        }
        check(
            unsafe { libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &ifr) },
            "guest link up",
        );
    }

    let mut route: libc::rtentry = unsafe { std::mem::zeroed() };
    route.rt_dst = unsafe { std::mem::transmute(sockaddr("0.0.0.0")) };
    route.rt_genmask = unsafe { std::mem::transmute(sockaddr("0.0.0.0")) };
    route.rt_gateway = unsafe { std::mem::transmute(sockaddr("10.95.0.1")) };
    route.rt_flags = (libc::RTF_UP | libc::RTF_GATEWAY) as u16;
    let nic = CString::new("eth0").unwrap();
    route.rt_dev = nic.as_ptr().cast_mut();
    check(unsafe { libc::ioctl(fd, libc::SIOCADDRT as _, &route) }, "guest default route");
    check(unsafe { libc::close(fd) }, "guest close control socket");
}

fn exchange(stream: &mut TcpStream, sent: &[u8], expected: &[u8]) {
    stream.write_all(sent).unwrap();
    let mut received = vec![0; expected.len()];
    stream.read_exact(&mut received).unwrap();
    assert_eq!(received, expected);
}

fn main() {
    check(
        unsafe {
            libc::mount(
                c"proc".as_ptr(),
                c"/proc".as_ptr(),
                c"proc".as_ptr(),
                0,
                std::ptr::null(),
            )
        },
        "mount proc",
    );
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap();
    let server = cmdline.contains("spike_role=server");
    let ip = if server { "10.95.0.3" } else { "10.95.0.2" };
    configure_guest(ip);
    println!(
        "GUEST CONFIG ip={ip}/24 gateway=10.95.0.1 role={} PID={} NO_CREDENTIALS",
        if server { "server" } else { "client" },
        unsafe { libc::getpid() }
    );

    if server {
        let listener = TcpListener::bind("0.0.0.0:9000").unwrap();
        println!("GUEST SERVER READY");
        let (mut stream, peer) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        println!("GUEST SERVER ACCEPT peer={peer}");
        let mut first = vec![0; REQUEST_ONE.len()];
        stream.read_exact(&mut first).unwrap();
        assert_eq!(first, REQUEST_ONE);
        stream.write_all(RESPONSE_ONE).unwrap();
        let mut second = vec![0; REQUEST_TWO.len()];
        stream.read_exact(&mut second).unwrap();
        assert_eq!(second, REQUEST_TWO);
        stream.write_all(RESPONSE_TWO).unwrap();
        println!("GUEST SERVER TWO-PHASE BYTE-DISTINCT EXCHANGE COMPLETE");
        std::thread::sleep(Duration::from_secs(4));
    } else {
        let start = Instant::now();
        println!("GUEST RESOLV_CONF={}", std::fs::read_to_string("/etc/resolv.conf").unwrap().trim());
        let destination = ("peer.mesh", 9000)
            .to_socket_addrs()
            .unwrap()
            .find(SocketAddr::is_ipv4)
            .unwrap();
        println!("GUEST DNS RESOLVED peer.mesh={destination}");
        let mut stream = TcpStream::connect_timeout(&destination, Duration::from_secs(15)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        exchange(&mut stream, REQUEST_ONE, RESPONSE_ONE);
        println!("GUEST WARMUP ROUNDTRIP COMPLETE");
        exchange(&mut stream, REQUEST_TWO, RESPONSE_TWO);
        println!(
            "GUEST STEADY ROUNDTRIP SUCCESS request_bytes={} response_bytes={} elapsed_seconds={:.6}",
            REQUEST_TWO.len(),
            RESPONSE_TWO.len(),
            start.elapsed().as_secs_f64()
        );
        std::thread::sleep(Duration::from_secs(4));
    }
    println!("GUEST EXIT=0");
    std::io::stdout().flush().unwrap();
    unsafe {
        libc::sync();
        libc::reboot(libc::RB_POWER_OFF);
    }
    loop {
        std::thread::park();
    }
}
