//! (d) pre-connect BPF cgroup/connect4 rewrite, then TLS client handshake + kTLS TX/RX on the SAME fd.
use aya::{maps::Array, programs::{CgroupAttachMode, CgroupSockAddr}, EbpfLoader};
use gwk::*;
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd};
use std::time::Duration;

const VIP: Ipv4Addr = Ipv4Addr::new(10, 96, 0, 9);
const VPORT: u32 = 443;
const CG: &str = "/sys/fs/cgroup/gwk-d";

fn sockname(fd: i32, peer: bool) -> String {
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut l = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    let r = unsafe { if peer { libc::getpeername(fd, (&mut sa as *mut libc::sockaddr_in).cast(), &mut l) } else { libc::getsockname(fd, (&mut sa as *mut libc::sockaddr_in).cast(), &mut l) } };
    if r != 0 { return format!("err {}", std::io::Error::last_os_error()); }
    format!("{}:{}", Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr)), u16::from_be(sa.sin_port))
}
fn raw_connect(ip: Ipv4Addr, port: u16, timeout_ms: i32) -> (Result<TcpStream, std::io::Error>, i32) {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        let sa = libc::sockaddr_in { sin_family: libc::AF_INET as u16, sin_port: port.to_be(), sin_addr: libc::in_addr { s_addr: u32::from(ip).to_be() }, sin_zero: [0; 8] };
        set_nonblock(fd, true);
        let r = libc::connect(fd, (&sa as *const libc::sockaddr_in).cast(), 16);
        let mut err = 0;
        if r != 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::EINPROGRESS) { libc::close(fd); return (Err(e), fd); }
            let mut p = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
            let n = libc::poll(&mut p, 1, timeout_ms);
            if n == 0 { libc::close(fd); return (Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "connect timeout")), fd); }
            let mut l = 4u32;
            libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, (&mut err as *mut i32).cast(), &mut l);
            if err != 0 { libc::close(fd); return (Err(std::io::Error::from_raw_os_error(err)), fd); }
        }
        set_nonblock(fd, false);
        (Ok(TcpStream::from_raw_fd(fd)), fd)
    }
}
fn ulp_name(fd: i32) -> String {
    let mut b = [0u8; 16]; let mut l = 16u32;
    let r = unsafe { libc::getsockopt(fd, libc::SOL_TCP, libc::TCP_ULP, b.as_mut_ptr().cast(), &mut l) };
    if r != 0 { return format!("err {}", std::io::Error::last_os_error()); }
    String::from_utf8_lossy(&b[..l as usize]).trim_end_matches('\0').to_string()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.get(1).map(String::as_str) == Some("ctrl") {
        // negative control child: runs OUTSIDE the cgroup; connect to VIP and report
        let mut s = String::new();
        std::io::stdin().read_line(&mut s).unwrap();
        let t = std::time::Instant::now();
        let (r, _) = raw_connect(VIP, VPORT as u16, 1500);
        println!("CTRL(outside cgroup, pid {}): connect {VIP}:{VPORT} -> {} after {:?}", std::process::id(), match &r { Ok(_) => "CONNECTED (unexpected)".to_string(), Err(e) => format!("Err({e})") }, t.elapsed());
        return;
    }
    deadline_guard(120, "d");
    let elf = a.get(1).expect("usage: d <gwk-bpf elf>");
    println!("uname -r: {}  t={}", uname(), now_ms());
    // 1. cgroup + program
    let _ = fs::remove_dir(CG);
    fs::create_dir(CG).expect("mkdir cgroup");
    let bytes = fs::read(elf).unwrap();
    let mut ebpf = EbpfLoader::new().load(&bytes).expect("load");
    let (bl, baddr) = listener();
    let bport = baddr.port() as u32;
    {
        let mut cfg: Array<_, u32> = Array::try_from(ebpf.map_mut("CFG").unwrap()).unwrap();
        cfg.set(0, u32::from(VIP).to_be(), 0).unwrap();
        cfg.set(1, VPORT, 0).unwrap();
        cfg.set(2, u32::from(Ipv4Addr::LOCALHOST).to_be(), 0).unwrap();
        cfg.set(3, bport, 0).unwrap();
    }
    let prog: &mut CgroupSockAddr = ebpf.program_mut("gw_connect4").unwrap().try_into().unwrap();
    prog.load().expect("verifier");
    let cgf = fs::File::open(CG).unwrap();
    let link = prog.attach(&cgf, CgroupAttachMode::Single).expect("attach");
    println!("ATTACHED gw_connect4 to {CG}; backend 127.0.0.1:{bport}, VIP {VIP}:{VPORT}");
    // 2. negative control child BEFORE moving self into the cgroup
    let mut child = std::process::Command::new(std::env::current_exe().unwrap()).arg("ctrl").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::inherit()).spawn().unwrap();
    // 3. backend: rustls TLS server -> kTLS both -> echo with transform; serves N connections
    let nconn = 20usize;
    let be = std::thread::spawn(move || {
        let mut peers = vec![];
        for i in 0..nconn {
            let (mut s, from) = bl.accept().unwrap();
            let est = handshake_server(&mut s, server_config(None, 0, true)).unwrap();
            assert_eq!(est.leftover_ciphertext, 0);
            arm_both(s.as_raw_fd(), &est.secrets).unwrap();
            let mut buf = vec![0u8; 1 << 16];
            let mut got = 0usize;
            let want = if i == 0 { 1 << 20 } else { 100 };
            while got < want {
                let (t, n) = recv_rec(s.as_raw_fd(), &mut buf[..], 0).unwrap();
                assert_eq!(t, REC_DATA);
                if n == 0 { break; }
                got += n;
            }
            let ok = got == want;
            let r = unsafe { libc::send(s.as_raw_fd(), if ok { b"OK-BACKEND".as_ptr() } else { b"BAD".as_ptr() }.cast(), if ok { 10 } else { 3 }, 0) };
            assert!(r > 0);
            if i == 0 { println!("BACKEND conn0: accepted from {from}; kernel-TLS read {got} bytes; replied"); }
            peers.push(s);
        }
        peers
    });
    // 4. move self into the cgroup
    fs::write(format!("{CG}/cgroup.procs"), std::process::id().to_string()).expect("move into cgroup");
    println!("pid {} now in: {}", std::process::id(), fs::read_to_string("/proc/self/cgroup").unwrap().trim());
    // release the negative control now
    child.stdin.as_mut().unwrap().write_all(b"go\n").unwrap();
    child.wait().unwrap();
    // 5. positive: rewrite, then TLS handshake, then kTLS on the same fd
    let cfgmap: Array<_, u32> = Array::try_from(ebpf.map("HITS").unwrap()).unwrap();
    let mut failures = 0;
    for i in 0..nconn {
        let (r, fd) = raw_connect(VIP, VPORT as u16, 2000);
        let mut tcp = match r { Ok(t) => t, Err(e) => { println!("conn{i}: connect to VIP FAILED {e}"); failures += 1; continue; } };
        let pn = sockname(tcp.as_raw_fd(), true);
        let ln = sockname(tcp.as_raw_fd(), false);
        let est = handshake_client(&mut tcp, client_config(true)).unwrap_or_else(|e| { println!("conn{i}: TLS handshake FAILED {e}"); std::process::exit(1) });
        assert_eq!(est.leftover_ciphertext, 0);
        let ulp_before = ulp_name(fd);
        let armed = arm_both(fd, &est.secrets);
        let ulp_after = ulp_name(fd);
        if i == 0 || i == nconn - 1 || armed.is_err() {
            println!("conn{i}: connect(VIP) ok; getpeername={pn} getsockname={ln}; ulp before='{ulp_before}' after='{ulp_after}'; arm_both={armed:?}");
        }
        if armed.is_err() { failures += 1; continue; }
        let payload = if i == 0 { pattern(0, 1 << 20) } else { pattern(0, 100) };
        let mut o = 0;
        while o < payload.len() {
            let w = unsafe { libc::send(fd, payload[o..].as_ptr().cast(), payload.len() - o, 0) };
            assert!(w > 0, "send {}", std::io::Error::last_os_error());
            o += w as usize;
        }
        let mut b = [0u8; 32];
        let (t, n) = recv_rec(fd, &mut b, 0).unwrap();
        let ok = t == REC_DATA && &b[..n] == b"OK-BACKEND";
        if !ok { failures += 1; }
        if i == 0 { println!("conn0: kTLS round trip over rewritten socket: reply={:?} ok={ok}", String::from_utf8_lossy(&b[..n])); }
        std::mem::forget(tcp);
    }
    let hits: u32 = cfgmap.get(&0, 0).unwrap();
    let seen: u32 = cfgmap.get(&1, 0).unwrap();
    println!("RESULT d: {nconn} connections, failures={failures}; BPF HITS(rewrites)={hits} connects_seen={seen}");
    let peers = be.join().unwrap();
    drop(peers);
    // 6. cleanup
    fs::write("/sys/fs/cgroup/cgroup.procs", std::process::id().to_string()).unwrap();
    drop(link);
    drop(ebpf);
    match fs::remove_dir(CG) { Ok(()) => println!("cleanup: cgroup removed"), Err(e) => println!("cleanup: rmdir {CG}: {e}") }
}
