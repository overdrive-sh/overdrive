//! V-LEGB-MARK probe socket tool. libc only. Subcommands:
//!   listen   --port P [--idle-ms N]          IP_TRANSPARENT TCP listener on 127.0.0.1:P (stands in for leg-C)
//!   wild-tcp --port P [--idle-ms N]          plain 0.0.0.0 TCP listener (host service decoy)
//!   wild-udp --port P [--idle-ms N]          plain 0.0.0.0 UDP socket (host service decoy)
//!   dial     --ip A --port P [--mark M] [--bind A] [--dev IF] [--timeout-ms T] [--udp]
use std::ffi::CString;
use std::mem;
use std::net::Ipv4Addr;
use std::time::Instant;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}
fn flag(args: &[String], name: &str) -> bool { args.iter().any(|a| a == name) }

fn errname(e: i32) -> &'static str {
    match e {
        libc::ENETUNREACH => "ENETUNREACH",
        libc::EINVAL => "EINVAL",
        libc::ECONNREFUSED => "ECONNREFUSED",
        libc::EHOSTUNREACH => "EHOSTUNREACH",
        libc::EPERM => "EPERM",
        libc::ETIMEDOUT => "ETIMEDOUT",
        libc::EACCES => "EACCES",
        libc::EADDRNOTAVAIL => "EADDRNOTAVAIL",
        _ => "other",
    }
}
fn sin(ip: Ipv4Addr, port: u16) -> libc::sockaddr_in {
    let mut a: libc::sockaddr_in = unsafe { mem::zeroed() };
    a.sin_family = libc::AF_INET as u16;
    a.sin_port = port.to_be();
    a.sin_addr.s_addr = u32::from_ne_bytes(ip.octets());
    a
}
fn from_sin(a: &libc::sockaddr_in) -> String {
    let ip = Ipv4Addr::from(a.sin_addr.s_addr.to_ne_bytes());
    format!("{}:{}", ip, u16::from_be(a.sin_port))
}
fn sockname(fd: i32, peer: bool) -> String {
    let mut a: libc::sockaddr_in = unsafe { mem::zeroed() };
    let mut l = mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    let rc = unsafe {
        if peer { libc::getpeername(fd, &mut a as *mut _ as *mut _, &mut l) }
        else { libc::getsockname(fd, &mut a as *mut _ as *mut _, &mut l) }
    };
    if rc != 0 { return format!("err{}", std::io::Error::last_os_error().raw_os_error().unwrap_or(0)); }
    from_sin(&a)
}
fn setopt_i32(fd: i32, lvl: i32, opt: i32, v: i32) -> i32 {
    unsafe { libc::setsockopt(fd, lvl, opt, &v as *const _ as *const _, 4) }
}
fn errno() -> i32 { std::io::Error::last_os_error().raw_os_error().unwrap_or(0) }
fn pollin(fd: i32, ms: i32) -> bool {
    let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    unsafe { libc::poll(&mut p, 1, ms) > 0 }
}
macro_rules! say { ($($t:tt)*) => {{ println!($($t)*); use std::io::Write; let _ = std::io::stdout().flush(); }} }

fn listener(transparent: bool, ip: Ipv4Addr, port: u16) -> i32 {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    setopt_i32(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, 1);
    if transparent && setopt_i32(fd, libc::SOL_IP, libc::IP_TRANSPARENT, 1) != 0 {
        say!("FATAL IP_TRANSPARENT errno={}", errno()); std::process::exit(2);
    }
    let a = sin(ip, port);
    if unsafe { libc::bind(fd, &a as *const _ as *const _, 16) } != 0 { say!("FATAL bind errno={}", errno()); std::process::exit(2); }
    unsafe { libc::listen(fd, 16) };
    fd
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    let port: u16 = arg(&args, "--port").and_then(|p| p.parse().ok()).unwrap_or(0);
    let idle: i32 = arg(&args, "--idle-ms").and_then(|p| p.parse().ok()).unwrap_or(4000);
    match cmd {
        "listen" | "wild-tcp" => {
            let tp = cmd == "listen";
            let ip = if tp { Ipv4Addr::LOCALHOST } else { Ipv4Addr::UNSPECIFIED };
            let fd = listener(tp, ip, port);
            let tag = if tp { "ACCEPT" } else { "WILD-ACCEPT" };
            say!("{} ready port={}", if tp { "listener" } else { "wild-tcp" }, port);
            while pollin(fd, idle) {
                let c = unsafe { libc::accept(fd, std::ptr::null_mut(), std::ptr::null_mut()) };
                if c < 0 { continue; }
                let peer = sockname(c, true);
                let local = sockname(c, false);
                let mut buf = [0u8; 128];
                let tv = libc::timeval { tv_sec: 1, tv_usec: 0 };
                unsafe { libc::setsockopt(c, libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const _, mem::size_of::<libc::timeval>() as u32) };
                let n = unsafe { libc::read(c, buf.as_mut_ptr().cast(), buf.len()) };
                let req = if n > 0 { String::from_utf8_lossy(&buf[..n as usize]).to_string() } else { String::new() };
                say!("{} peer={} local={} req={:?}", tag, peer, local, req);
                let resp = format!("RESP:{}", req);
                unsafe { libc::write(c, resp.as_ptr().cast(), resp.len()); libc::close(c); }
            }
            say!("{} idle-exit", tag);
        }
        "wild-udp" => {
            let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
            let a = sin(Ipv4Addr::UNSPECIFIED, port);
            if unsafe { libc::bind(fd, &a as *const _ as *const _, 16) } != 0 { say!("FATAL bind errno={}", errno()); std::process::exit(2); }
            say!("wild-udp ready port={}", port);
            while pollin(fd, idle) {
                let mut buf = [0u8; 128];
                let mut from: libc::sockaddr_in = unsafe { mem::zeroed() };
                let mut l = 16u32;
                let n = unsafe { libc::recvfrom(fd, buf.as_mut_ptr().cast(), 128, 0, &mut from as *mut _ as *mut _, &mut l) };
                say!("WILD-UDP-RECV from={} n={}", from_sin(&from), n);
            }
            say!("WILD-UDP idle-exit");
        }

        "vm-guest" => {
            // Runs INSIDE the real microVM (PID-1 shell starts it): AF_VSOCK server standing in for the
            // guest app behind the owner's vsock flow. Replies with proof of where it runs.
            let vport: u32 = arg(&args, "--vport").and_then(|p| p.parse().ok()).unwrap_or(5000);
            let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
            let mut a: libc::sockaddr_vm = unsafe { mem::zeroed() };
            a.svm_family = libc::AF_VSOCK as u16; a.svm_port = vport; a.svm_cid = libc::VMADDR_CID_ANY;
            if unsafe { libc::bind(fd, &a as *const _ as *const _, mem::size_of::<libc::sockaddr_vm>() as u32) } != 0 { say!("GUEST FATAL vsock bind errno={}", errno()); std::process::exit(2); }
            unsafe { libc::listen(fd, 8) };
            let mut u: libc::utsname = unsafe { mem::zeroed() };
            unsafe { libc::uname(&mut u) };
            let rel = unsafe { std::ffi::CStr::from_ptr(u.release.as_ptr()) }.to_string_lossy().to_string();
            say!("GUEST_VSOCK_READY port={} uname_r={}", vport, rel);
            loop {
                let c = unsafe { libc::accept(fd, std::ptr::null_mut(), std::ptr::null_mut()) };
                if c < 0 { continue; }
                let mut buf = [0u8; 256];
                let n = unsafe { libc::read(c, buf.as_mut_ptr().cast(), buf.len()) };
                let req = if n > 0 { String::from_utf8_lossy(&buf[..n as usize]).to_string() } else { String::new() };
                let resp = format!("FROM-MICROVM uname_r={} req={}", rel, req);
                say!("GUEST_SERVED {}", resp);
                unsafe { libc::write(c, resp.as_ptr().cast(), resp.len()); libc::close(c); }
            }
        }
        "vm-legc" => {
            // leg-C stand-in: IP_TRANSPARENT listener on 127.0.0.1:port (receives the TPROXY'd connection, local
            // address = original destination); per accept, opens an AF_VSOCK stream to the real microVM and splices.
            use std::os::fd::FromRawFd;
            use std::io::{Read, Write};
            let cid: u32 = arg(&args, "--cid").and_then(|p| p.parse().ok()).unwrap();
            let vport: u32 = arg(&args, "--vport").and_then(|p| p.parse().ok()).unwrap_or(5000);
            let fd = listener(true, Ipv4Addr::LOCALHOST, port);
            say!("legc-vm ready port={} -> vsock cid={} port={}", port, cid, vport);
            while pollin(fd, idle) {
                let c = unsafe { libc::accept(fd, std::ptr::null_mut(), std::ptr::null_mut()) };
                if c < 0 { continue; }
                let peer = sockname(c, true); let local = sockname(c, false);
                let v = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
                let mut a: libc::sockaddr_vm = unsafe { mem::zeroed() };
                a.svm_family = libc::AF_VSOCK as u16; a.svm_port = vport; a.svm_cid = cid;
                if unsafe { libc::connect(v, &a as *const _ as *const _, mem::size_of::<libc::sockaddr_vm>() as u32) } != 0 {
                    say!("LEGC ACCEPT peer={} local={} vsock-connect-fail errno={}", peer, local, errno()); unsafe { libc::close(c); } continue;
                }
                say!("LEGC ACCEPT peer={} local={} -> vsock cid={} port={} connected", peer, local, cid, vport);
                let mut tcp = unsafe { std::fs::File::from_raw_fd(c) };
                let mut vs = unsafe { std::fs::File::from_raw_fd(v) };
                let mut tcp2 = tcp.try_clone().unwrap(); let mut vs2 = vs.try_clone().unwrap();
                let t = std::thread::spawn(move || { let _ = std::io::copy(&mut tcp2, &mut vs2); unsafe { libc::shutdown(libc::AF_VSOCK * 0 + std::os::fd::AsRawFd::as_raw_fd(&vs2), libc::SHUT_WR) }; });
                let mut b = Vec::new(); let _ = vs.read_to_end(&mut b); let _ = tcp.write_all(&b);
                unsafe { libc::shutdown(std::os::fd::AsRawFd::as_raw_fd(&tcp), libc::SHUT_WR) };
                let _ = t.join();
            }
            say!("LEGC idle-exit");
        }
        "dial" => {
            let ip: Ipv4Addr = arg(&args, "--ip").unwrap().parse().unwrap();
            let udp = flag(&args, "--udp");
            let tmo: i32 = arg(&args, "--timeout-ms").and_then(|p| p.parse().ok()).unwrap_or(1500);
            let ty = if udp { libc::SOCK_DGRAM } else { libc::SOCK_STREAM };
            let fd = unsafe { libc::socket(libc::AF_INET, ty | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0) };
            let mut setup = String::new();
            // --leg-b: the PROPOSED leg-B dial contract. Destination inside GUEST_NODE_PREFIX
            // (100.95.0.0/16, overdrive-core guest_transport/mod.rs:382) => SO_MARK=MTLS_TPROXY_MARK
            // (0x1, core dataplane/mtls_mark.rs:27) + bind(127.0.0.1) before connect; any other
            // destination => untouched.
            let legb = flag(&args, "--leg-b") && { let o = ip.octets(); o[0] == 100 && o[1] == 95 };
            let mark_arg = arg(&args, "--mark").or(if legb { Some("1".into()) } else { None });
            let bind_arg = arg(&args, "--bind").or(if legb { Some("127.0.0.1".into()) } else { None });
            if let Some(m) = mark_arg {
                let m: i32 = m.parse().unwrap();
                if setopt_i32(fd, libc::SOL_SOCKET, libc::SO_MARK, m) != 0 { say!("RESULT setsockopt(SO_MARK) errno={}", errno()); return; }
                setup += &format!(" mark={m}");
            }
            if let Some(d) = arg(&args, "--dev") {
                let c = CString::new(d.clone()).unwrap();
                if unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_BINDTODEVICE, c.as_ptr().cast(), d.len() as u32) } != 0 { say!("RESULT SO_BINDTODEVICE errno={}", errno()); return; }
                setup += &format!(" dev={d}");
            }
            if let Some(b) = bind_arg {
                let b: Ipv4Addr = b.parse().unwrap();
                // IP_BIND_ADDRESS_NO_PORT (24): defer port allocation to connect (research step 1)
                if !udp { setopt_i32(fd, libc::SOL_IP, 24, 1); }
                let a = sin(b, 0);
                if unsafe { libc::bind(fd, &a as *const _ as *const _, 16) } != 0 { say!("RESULT bind errno={} {}", errno(), errname(errno())); return; }
                setup += &format!(" bind={b}");
            }
            let a = sin(ip, port);
            let t0 = Instant::now();
            let rc = unsafe { libc::connect(fd, &a as *const _ as *const _, 16) };
            let mut e = if rc == 0 { 0 } else { errno() };
            if e == libc::EINPROGRESS {
                let mut p = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
                if unsafe { libc::poll(&mut p, 1, tmo) } == 0 { e = libc::ETIMEDOUT; }
                else { let mut so = 0i32; let mut l = 4u32; unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, &mut so as *mut _ as *mut _, &mut l) }; e = so; }
            }
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            if e != 0 { say!("RESULT connect-fail errno={} {} elapsed_ms={:.1} setup=[{}]", e, errname(e), ms, setup.trim()); return; }
            let local = sockname(fd, false); let peer = sockname(fd, true);
            let req = format!("REQ-{}:{}", ip, port);
            let w = unsafe { libc::send(fd, req.as_ptr().cast(), req.len(), 0) };
            if w < 0 { let e = errno(); say!("RESULT send-fail errno={} {} local={} setup=[{}]", e, errname(e), local, setup.trim()); return; }
            let mut buf = [0u8; 128];
            let mut got = String::new(); let mut re = 0;
            if pollin(fd, tmo) {
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), 128, 0) };
                if n >= 0 { got = String::from_utf8_lossy(&buf[..n as usize]).to_string(); } else { re = errno(); }
            } else { re = libc::ETIMEDOUT; }
            if got.is_empty() { say!("RESULT no-reply errno={} {} client-local={} client-peer={} setup=[{}]", re, errname(re), local, peer, setup.trim()); }
            else { say!("RESULT ok client-local={} client-peer={} reply={:?} elapsed_ms={:.1} setup=[{}]", local, peer, got, ms, setup.trim()); }
        }
        _ => { eprintln!("usage"); std::process::exit(64); }
    }
}
