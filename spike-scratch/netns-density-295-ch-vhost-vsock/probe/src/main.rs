//! Throwaway spike probe (netns-density-295, V-1(b)): host and guest side of
//! the CH vhost-kernel vsock validation. One static binary, libc only.
//!
//! Subcommands (all output is line-oriented `KEY=value` / `RESULT ...` text):
//!   local-cid                              IOCTL_VM_SOCKETS_GET_LOCAL_CID
//!   agent                                  guest agent (listeners + control)
//!   client <ep> <stream|seqpacket> <bytes> <label>
//!   serve  <ep> <stream|seqpacket> <count> run litmus server for N connections
//!   recorder <ep> <count>                  peer recorder (getpeername + claim)
//!   ctl <ep> <command...>                  send a control command, print reply
//!
//! Endpoints: `vsock:<cid>:<port>` or `unix:<path>[:<port>]` (unix with a port
//! means: connect to the CH muxer socket and send `CONNECT <port>`; for a
//! listener it means listen on `<path>_<port>` per the CH unix backend).

use std::ffi::CString;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::Instant;

const AF_VSOCK: i32 = libc::AF_VSOCK;
const VMADDR_CID_ANY: u32 = u32::MAX;
const IOCTL_VM_SOCKETS_GET_LOCAL_CID: libc::c_ulong = 0x7b9;
const SO_VM_SOCKETS_CONNECT_TIMEOUT_NEW: i32 = 8; // struct __kernel_sock_timeval
const AF_VSOCK_LEVEL: i32 = 40; // SOL_VSOCK == AF_VSOCK

const AGENT_CTL_PORT: u32 = 4000;
const AGENT_STREAM_PORT: u32 = 5000;
const AGENT_SEQ_PORT: u32 = 5001;
const SEQ_SIZES: &[usize] = &[1, 7, 1500, 4096, 65536, 200_000];

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("FATAL {msg}");
    std::process::exit(2)
}

fn errno() -> io::Error {
    io::Error::last_os_error()
}

fn check(rc: libc::c_int, what: &str) -> io::Result<libc::c_int> {
    if rc < 0 {
        let e = errno();
        Err(io::Error::new(e.kind(), format!("{what}: {e}")))
    } else {
        Ok(rc)
    }
}

// ---------------------------------------------------------------- addressing

fn sockaddr_vm(cid: u32, port: u32) -> libc::sockaddr_vm {
    let mut sa: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
    sa.svm_family = AF_VSOCK as libc::sa_family_t;
    sa.svm_cid = cid;
    sa.svm_port = port;
    sa
}

fn sockaddr_un(path: &str) -> (libc::sockaddr_un, libc::socklen_t) {
    let mut sa: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    sa.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_bytes();
    assert!(bytes.len() < sa.sun_path.len(), "unix path too long");
    for (i, b) in bytes.iter().enumerate() {
        sa.sun_path[i] = *b as libc::c_char;
    }
    let len = std::mem::size_of::<libc::sa_family_t>() + bytes.len() + 1;
    (sa, len as libc::socklen_t)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ty {
    Stream,
    Seq,
}

impl Ty {
    fn parse(s: &str) -> Ty {
        match s {
            "stream" => Ty::Stream,
            "seqpacket" => Ty::Seq,
            _ => die(format!("bad socket type {s}")),
        }
    }
    fn raw(self) -> i32 {
        match self {
            Ty::Stream => libc::SOCK_STREAM,
            Ty::Seq => libc::SOCK_SEQPACKET,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Ty::Stream => "stream",
            Ty::Seq => "seqpacket",
        }
    }
}

#[derive(Clone)]
enum Ep {
    Vsock { cid: u32, port: u32 },
    Unix { path: String, port: Option<u32> },
}

impl Ep {
    fn parse(s: &str) -> Ep {
        let parts: Vec<&str> = s.split(':').collect();
        match parts.as_slice() {
            ["vsock", cid, port] => Ep::Vsock {
                cid: if *cid == "any" { VMADDR_CID_ANY } else { cid.parse().unwrap_or_else(|_| die("bad cid")) },
                port: port.parse().unwrap_or_else(|_| die("bad port")),
            },
            ["unix", path] => Ep::Unix { path: path.to_string(), port: None },
            ["unix", path, port] => Ep::Unix {
                path: path.to_string(),
                port: Some(port.parse().unwrap_or_else(|_| die("bad port"))),
            },
            _ => die(format!("bad endpoint {s}")),
        }
    }
}

fn new_socket(domain: i32, ty: i32) -> io::Result<OwnedFd> {
    let fd = check(unsafe { libc::socket(domain, ty | libc::SOCK_CLOEXEC, 0) }, "socket")?;
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn set_connect_timeout(fd: RawFd, secs: i64) {
    let tv = libc::timeval { tv_sec: secs, tv_usec: 0 };
    unsafe {
        libc::setsockopt(
            fd,
            AF_VSOCK_LEVEL,
            SO_VM_SOCKETS_CONNECT_TIMEOUT_NEW,
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as u32,
        );
    }
}

fn vsock_connect(cid: u32, port: u32, ty: Ty, bind_cid: Option<u32>) -> io::Result<OwnedFd> {
    let fd = new_socket(AF_VSOCK, ty.raw())?;
    set_connect_timeout(fd.as_raw_fd(), 5);
    if let Some(bc) = bind_cid {
        let sa = sockaddr_vm(bc, u32::MAX); // VMADDR_PORT_ANY
        check(
            unsafe {
                libc::bind(
                    fd.as_raw_fd(),
                    &sa as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_vm>() as u32,
                )
            },
            "bind",
        )?;
    }
    let sa = sockaddr_vm(cid, port);
    check(
        unsafe {
            libc::connect(
                fd.as_raw_fd(),
                &sa as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_vm>() as u32,
            )
        },
        "connect",
    )?;
    Ok(fd)
}

fn connect(ep: &Ep, ty: Ty) -> io::Result<OwnedFd> {
    match ep {
        Ep::Vsock { cid, port } => vsock_connect(*cid, *port, ty, None),
        Ep::Unix { path, port } => {
            if ty != Ty::Stream {
                return Err(io::Error::other("unix backend is stream-only"));
            }
            let fd = new_socket(libc::AF_UNIX, libc::SOCK_STREAM)?;
            let (sa, len) = sockaddr_un(path);
            check(
                unsafe { libc::connect(fd.as_raw_fd(), &sa as *const _ as *const libc::sockaddr, len) },
                "connect(unix)",
            )?;
            if let Some(p) = port {
                write_all(fd.as_raw_fd(), format!("CONNECT {p}\n").as_bytes())?;
                let line = read_line(fd.as_raw_fd())?;
                if !line.starts_with("OK ") {
                    return Err(io::Error::other(format!("CONNECT refused: {line:?}")));
                }
            }
            Ok(fd)
        }
    }
}

fn listen(ep: &Ep, ty: Ty) -> io::Result<OwnedFd> {
    match ep {
        Ep::Vsock { cid, port } => {
            let fd = new_socket(AF_VSOCK, ty.raw())?;
            let sa = sockaddr_vm(*cid, *port);
            check(
                unsafe {
                    libc::bind(
                        fd.as_raw_fd(),
                        &sa as *const _ as *const libc::sockaddr,
                        std::mem::size_of::<libc::sockaddr_vm>() as u32,
                    )
                },
                "bind",
            )?;
            check(unsafe { libc::listen(fd.as_raw_fd(), 64) }, "listen")?;
            Ok(fd)
        }
        Ep::Unix { path, port } => {
            let full = match port {
                Some(p) => format!("{path}_{p}"),
                None => path.clone(),
            };
            let _ = std::fs::remove_file(&full);
            let fd = new_socket(libc::AF_UNIX, libc::SOCK_STREAM)?;
            let (sa, len) = sockaddr_un(&full);
            check(
                unsafe { libc::bind(fd.as_raw_fd(), &sa as *const _ as *const libc::sockaddr, len) },
                "bind(unix)",
            )?;
            check(unsafe { libc::listen(fd.as_raw_fd(), 64) }, "listen")?;
            Ok(fd)
        }
    }
}

fn accept(fd: RawFd) -> io::Result<OwnedFd> {
    let c = check(
        unsafe { libc::accept4(fd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) },
        "accept",
    )?;
    Ok(unsafe { OwnedFd::from_raw_fd(c) })
}

fn peer_desc(fd: RawFd) -> String {
    let mut ss: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    let rc = unsafe { libc::getpeername(fd, &mut ss as *mut _ as *mut libc::sockaddr, &mut len) };
    if rc < 0 {
        return format!("getpeername-error:{}", errno());
    }
    if ss.ss_family as i32 == AF_VSOCK {
        let vm: &libc::sockaddr_vm = unsafe { &*(&ss as *const _ as *const libc::sockaddr_vm) };
        format!("vsock:{}:{}", vm.svm_cid, vm.svm_port)
    } else if ss.ss_family as i32 == libc::AF_UNIX {
        "unix".to_string()
    } else {
        format!("family-{}", ss.ss_family)
    }
}

fn local_desc(fd: RawFd) -> String {
    let mut ss: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    let rc = unsafe { libc::getsockname(fd, &mut ss as *mut _ as *mut libc::sockaddr, &mut len) };
    if rc < 0 {
        return format!("getsockname-error:{}", errno());
    }
    if ss.ss_family as i32 == AF_VSOCK {
        let vm: &libc::sockaddr_vm = unsafe { &*(&ss as *const _ as *const libc::sockaddr_vm) };
        format!("vsock:{}:{}", vm.svm_cid, vm.svm_port)
    } else {
        "unix".to_string()
    }
}

// ------------------------------------------------------------------ raw I/O

fn write_all(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        let n = unsafe { libc::send(fd, buf.as_ptr() as *const _, buf.len(), libc::MSG_NOSIGNAL) };
        if n < 0 {
            let e = errno();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        buf = &buf[n as usize..];
    }
    Ok(())
}

fn read_some(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut _, buf.len(), 0) };
        if n < 0 {
            let e = errno();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        return Ok(n as usize);
    }
}

/// Byte-at-a-time line read (so no payload bytes are over-consumed).
fn read_line(fd: RawFd) -> io::Result<String> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    loop {
        let n = read_some(fd, &mut b)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof in line"));
        }
        if b[0] == b'\n' {
            return Ok(String::from_utf8_lossy(&out).into_owned());
        }
        out.push(b[0]);
        if out.len() > 4096 {
            return Err(io::Error::other("line too long"));
        }
    }
}

fn shutdown_wr(fd: RawFd) -> io::Result<()> {
    check(unsafe { libc::shutdown(fd, libc::SHUT_WR) }, "shutdown").map(|_| ())
}

// ------------------------------------------------------- deterministic data

fn seed_of(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// xorshift64* byte stream; the byte at offset i depends on seed and i only.
struct Pattern {
    state: u64,
    buf: u64,
    left: u32,
}

impl Pattern {
    fn new(seed: u64) -> Self {
        Pattern { state: seed | 1, buf: 0, left: 0 }
    }
    fn next(&mut self) -> u8 {
        if self.left == 0 {
            self.state ^= self.state >> 12;
            self.state ^= self.state << 25;
            self.state ^= self.state >> 27;
            self.buf = self.state.wrapping_mul(0x2545_f491_4f6c_dd1d);
            self.left = 8;
        }
        let b = self.buf as u8;
        self.buf >>= 8;
        self.left -= 1;
        b
    }
    fn fill(&mut self, out: &mut [u8]) {
        for b in out {
            *b = self.next();
        }
    }
}

fn send_pattern(fd: RawFd, seed: u64, len: usize) -> io::Result<()> {
    let mut p = Pattern::new(seed);
    let mut chunk = vec![0u8; 64 * 1024];
    let mut left = len;
    while left > 0 {
        let n = left.min(chunk.len());
        p.fill(&mut chunk[..n]);
        write_all(fd, &chunk[..n])?;
        left -= n;
    }
    Ok(())
}

/// Read exactly `len` bytes and verify them against the pattern.
fn recv_pattern(fd: RawFd, seed: u64, len: usize) -> io::Result<()> {
    let mut p = Pattern::new(seed);
    let mut chunk = vec![0u8; 64 * 1024];
    let mut got = 0usize;
    while got < len {
        let want = (len - got).min(chunk.len());
        let n = read_some(fd, &mut chunk[..want])?;
        if n == 0 {
            return Err(io::Error::other(format!("short read: {got}/{len}")));
        }
        for (i, b) in chunk[..n].iter().enumerate() {
            let e = p.next();
            if *b != e {
                return Err(io::Error::other(format!("byte mismatch at {}", got + i)));
            }
        }
        got += n;
    }
    Ok(())
}

fn expect_eof(fd: RawFd) -> io::Result<()> {
    let mut b = [0u8; 16];
    match read_some(fd, &mut b)? {
        0 => Ok(()),
        n => Err(io::Error::other(format!("expected EOF, got {n} extra bytes"))),
    }
}

fn kv(line: &str, key: &str) -> Option<String> {
    line.split_whitespace()
        .find_map(|t| t.strip_prefix(&format!("{key}=")).map(str::to_owned))
}

// ----------------------------------------------------------- stream litmus

fn identity() -> String {
    format!("cid-{}", local_cid().map(|c| c.to_string()).unwrap_or_else(|_| "host".into()))
}

fn stream_server_conn(fd: RawFd) -> io::Result<String> {
    let peer = peer_desc(fd);
    let hdr = read_line(fd)?;
    if !hdr.starts_with("LITMUS-REQ ") {
        return Err(io::Error::other(format!("bad header {hdr:?}")));
    }
    let label = kv(&hdr, "label").ok_or_else(|| io::Error::other("no label"))?;
    let len: usize = kv(&hdr, "len").and_then(|v| v.parse().ok()).ok_or_else(|| io::Error::other("no len"))?;
    recv_pattern(fd, seed_of(&format!("req/{label}")), len)?;
    // The client half-closes after its payload: we must observe EOF here
    // while our write side is still open.
    expect_eof(fd)?;
    write_all(
        fd,
        format!("LITMUS-RSP label={label} got={len} peer={peer} server={}\n", identity()).as_bytes(),
    )?;
    send_pattern(fd, seed_of(&format!("rsp/{label}")), len)?;
    Ok(format!("SERVED stream label={label} bytes={len} half_close_seen=1 peer={peer} local={}", local_desc(fd)))
}

fn stream_client(ep: &Ep, len: usize, label: &str) -> io::Result<String> {
    let t0 = Instant::now();
    let fd = connect(ep, Ty::Stream)?;
    let raw = fd.as_raw_fd();
    let local = local_desc(raw);
    write_all(raw, format!("LITMUS-REQ label={label} len={len}\n").as_bytes())?;
    send_pattern(raw, seed_of(&format!("req/{label}")), len)?;
    shutdown_wr(raw)?; // half-close: the response must still flow back.
    let hdr = read_line(raw)?;
    if !hdr.starts_with("LITMUS-RSP ") || kv(&hdr, "label").as_deref() != Some(label) {
        return Err(io::Error::other(format!("bad response header {hdr:?}")));
    }
    if kv(&hdr, "got").and_then(|v| v.parse::<usize>().ok()) != Some(len) {
        return Err(io::Error::other(format!("server byte count mismatch {hdr:?}")));
    }
    recv_pattern(raw, seed_of(&format!("rsp/{label}")), len)?;
    expect_eof(raw)?;
    Ok(format!(
        "OK stream label={label} sent={len} received={len} half_close=1 local={local} server_saw_peer={} server={} elapsed_ms={}",
        kv(&hdr, "peer").unwrap_or_default(),
        kv(&hdr, "server").unwrap_or_default(),
        t0.elapsed().as_millis()
    ))
}

// -------------------------------------------------------- seqpacket litmus

fn recv_msg(fd: RawFd, buf: &mut [u8]) -> io::Result<(usize, i32)> {
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr() as *mut _, iov_len: buf.len() };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    let n = unsafe { libc::recvmsg(fd, &mut msg, 0) };
    if n < 0 {
        return Err(errno());
    }
    Ok((n as usize, msg.msg_flags))
}

fn send_msg(fd: RawFd, data: &[u8]) -> io::Result<()> {
    let n = unsafe { libc::send(fd, data.as_ptr() as *const _, data.len(), libc::MSG_NOSIGNAL) };
    if n < 0 {
        return Err(errno());
    }
    if n as usize != data.len() {
        return Err(io::Error::other(format!("partial seqpacket send {n}/{}", data.len())));
    }
    Ok(())
}

fn rsp_size(i: usize, size: usize) -> usize {
    (size * 3 + i * 11) % 70_000 + 1
}

fn expect_record(fd: RawFd, buf: &mut [u8], seed: u64, size: usize, what: &str) -> io::Result<()> {
    let (n, flags) = recv_msg(fd, buf)?;
    if flags & libc::MSG_TRUNC != 0 {
        return Err(io::Error::other(format!("{what}: MSG_TRUNC")));
    }
    if n != size {
        return Err(io::Error::other(format!("{what}: boundary violation got {n} want {size}")));
    }
    let mut want = vec![0u8; size];
    Pattern::new(seed).fill(&mut want);
    if buf[..n] != want[..] {
        return Err(io::Error::other(format!("{what}: content mismatch")));
    }
    Ok(())
}

fn seq_server_conn(fd: RawFd) -> io::Result<String> {
    let peer = peer_desc(fd);
    let mut buf = vec![0u8; 1 << 20];
    let (n, _) = recv_msg(fd, &mut buf)?;
    let manifest = String::from_utf8_lossy(&buf[..n]).into_owned();
    if !manifest.starts_with("SEQ-REQ ") {
        return Err(io::Error::other(format!("bad manifest {manifest:?}")));
    }
    let label = kv(&manifest, "label").ok_or_else(|| io::Error::other("no label"))?;
    let sizes: Vec<usize> = kv(&manifest, "sizes")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.parse().unwrap_or(0))
        .collect();
    for (i, s) in sizes.iter().enumerate() {
        expect_record(fd, &mut buf, seed_of(&format!("sq/{label}/{i}")), *s, "request")?;
    }
    let (eof, _) = recv_msg(fd, &mut buf)?;
    if eof != 0 {
        return Err(io::Error::other("expected EOF after requests"));
    }
    send_msg(
        fd,
        format!("SEQ-RSP label={label} records={} peer={peer} server={}", sizes.len(), identity()).as_bytes(),
    )?;
    for (i, s) in sizes.iter().enumerate() {
        let mut out = vec![0u8; rsp_size(i, *s)];
        Pattern::new(seed_of(&format!("sr/{label}/{i}"))).fill(&mut out);
        send_msg(fd, &out)?;
    }
    Ok(format!("SERVED seqpacket label={label} records={} peer={peer} local={}", sizes.len(), local_desc(fd)))
}

fn seq_client(ep: &Ep, label: &str) -> io::Result<String> {
    let t0 = Instant::now();
    let fd = connect(ep, Ty::Seq)?;
    let raw = fd.as_raw_fd();
    let local = local_desc(raw);
    let sizes: Vec<String> = SEQ_SIZES.iter().map(|s| s.to_string()).collect();
    send_msg(raw, format!("SEQ-REQ label={label} sizes={}", sizes.join(",")).as_bytes())?;
    for (i, s) in SEQ_SIZES.iter().enumerate() {
        let mut out = vec![0u8; *s];
        Pattern::new(seed_of(&format!("sq/{label}/{i}"))).fill(&mut out);
        send_msg(raw, &out)?;
    }
    shutdown_wr(raw)?;
    let mut buf = vec![0u8; 1 << 20];
    let (n, _) = recv_msg(raw, &mut buf)?;
    let hdr = String::from_utf8_lossy(&buf[..n]).into_owned();
    if !hdr.starts_with("SEQ-RSP ") || kv(&hdr, "label").as_deref() != Some(label) {
        return Err(io::Error::other(format!("bad seq response header {hdr:?}")));
    }
    for (i, s) in SEQ_SIZES.iter().enumerate() {
        expect_record(raw, &mut buf, seed_of(&format!("sr/{label}/{i}")), rsp_size(i, *s), "response")?;
    }
    let (eof, _) = recv_msg(raw, &mut buf)?;
    if eof != 0 {
        return Err(io::Error::other("expected EOF after responses"));
    }
    Ok(format!(
        "OK seqpacket label={label} records={} sizes={} boundaries_exact=1 local={local} server_saw_peer={} server={} elapsed_ms={}",
        SEQ_SIZES.len(),
        sizes.join(","),
        kv(&hdr, "peer").unwrap_or_default(),
        kv(&hdr, "server").unwrap_or_default(),
        t0.elapsed().as_millis()
    ))
}

// ---------------------------------------------------------------- servers

fn serve(ep: &Ep, ty: Ty, count: usize) {
    let l = listen(ep, ty).unwrap_or_else(|e| die(e));
    println!("LISTENING {} {}", ty.name(), local_desc(l.as_raw_fd()));
    io::stdout().flush().ok();
    for _ in 0..count {
        let c = match accept(l.as_raw_fd()) {
            Ok(c) => c,
            Err(e) => {
                println!("RESULT FAIL accept {e}");
                continue;
            }
        };
        let r = match ty {
            Ty::Stream => stream_server_conn(c.as_raw_fd()),
            Ty::Seq => seq_server_conn(c.as_raw_fd()),
        };
        match r {
            Ok(s) => println!("RESULT {s}"),
            Err(e) => println!("RESULT FAIL serve {e}"),
        }
        io::stdout().flush().ok();
    }
}

/// Records what the host kernel reports as the peer address, alongside what
/// the peer claims to be.
fn recorder(ep: &Ep, count: usize) {
    let l = listen(ep, Ty::Stream).unwrap_or_else(|e| die(e));
    println!("LISTENING recorder {}", local_desc(l.as_raw_fd()));
    io::stdout().flush().ok();
    for _ in 0..count {
        let Ok(c) = accept(l.as_raw_fd()) else { continue };
        let fd = c.as_raw_fd();
        let peer = peer_desc(fd);
        let claim = read_line(fd).unwrap_or_else(|e| format!("read-error:{e}"));
        let _ = write_all(fd, format!("SEEN peer={peer}\n").as_bytes());
        println!("RECORDED peer={peer} claim=[{claim}]");
        io::stdout().flush().ok();
    }
}

// ------------------------------------------------------------------ agent

fn local_cid() -> io::Result<u32> {
    let path = CString::new("/dev/vsock").unwrap();
    let fd = check(unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) }, "open /dev/vsock")?;
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut cid: u32 = 0;
    check(
        unsafe { libc::ioctl(owned.as_raw_fd(), IOCTL_VM_SOCKETS_GET_LOCAL_CID, &mut cid) },
        "IOCTL_VM_SOCKETS_GET_LOCAL_CID",
    )?;
    Ok(cid)
}

/// Listen, retrying while no vsock transport is registered yet (a hot-plugged
/// device appears after boot).
fn listen_retry(port: u32, ty: Ty) -> OwnedFd {
    let mut last = String::new();
    for _ in 0..600 {
        match listen(&Ep::Vsock { cid: VMADDR_CID_ANY, port }, ty) {
            Ok(fd) => return fd,
            Err(e) => {
                let msg = e.to_string();
                if msg != last {
                    println!("AGENT_WAIT port={port} err={msg}");
                    io::stdout().flush().ok();
                    last = msg;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    }
    die(format!("listen {port}: {last}"))
}

fn spawn_server(port: u32, ty: Ty) {
    std::thread::spawn(move || {
        let l = listen_retry(port, ty);
        loop {
            let Ok(c) = accept(l.as_raw_fd()) else { continue };
            std::thread::spawn(move || {
                let r = match ty {
                    Ty::Stream => stream_server_conn(c.as_raw_fd()),
                    Ty::Seq => seq_server_conn(c.as_raw_fd()),
                };
                match r {
                    Ok(s) => println!("AGENT {s}"),
                    Err(e) => println!("AGENT FAIL {e}"),
                }
            });
        }
    });
}

/// Guest-side spoofing attempts toward a host recorder at `host_cid:port`.
fn spoof(host_cid: u32, port: u32) -> String {
    let own = local_cid().unwrap_or(0);
    let mut out = Vec::new();
    let mut attempt = |case: &str, bind_cid: Option<u32>| {
        match vsock_connect(host_cid, port, Ty::Stream, bind_cid) {
            Ok(fd) => {
                let raw = fd.as_raw_fd();
                let claim = match bind_cid {
                    Some(c) => c.to_string(),
                    None => "unbound".to_string(),
                };
                let _ = write_all(raw, format!("case={case} claimed_cid={claim} local={}\n", local_desc(raw)).as_bytes());
                let seen = read_line(raw).unwrap_or_else(|e| format!("read-error:{e}"));
                out.push(format!("SPOOF case={case} connect=ok local={} host_says=[{seen}]", local_desc(raw)));
            }
            Err(e) => out.push(format!("SPOOF case={case} connect=refused err=[{e}]")),
        }
    };
    attempt("forged-cid-own+100", Some(own + 100));
    attempt("forged-cid-host-2", Some(2));
    attempt("forged-cid-1-local", Some(1));
    attempt("bind-any", Some(VMADDR_CID_ANY));
    attempt("bind-own", Some(own));
    attempt("unbound", None);
    out.join("\n")
}

fn control_command(cmd: &str) -> String {
    let words: Vec<&str> = cmd.split_whitespace().collect();
    match words.as_slice() {
        ["ident"] => {
            let uname = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
            let uptime = std::fs::read_to_string("/proc/uptime").unwrap_or_default();
            format!(
                "IDENT local_cid={} uname={} uptime_s={}",
                local_cid().map(|c| c.to_string()).unwrap_or_else(|e| e.to_string()),
                uname.trim(),
                uptime.split_whitespace().next().unwrap_or("?")
            )
        }
        ["client", host_cid, stream_port, seq_port, bytes] => {
            let cid: u32 = host_cid.parse().unwrap_or(2);
            let sp: u32 = stream_port.parse().unwrap_or(6000);
            let qp: u32 = seq_port.parse().unwrap_or(6001);
            let len: usize = bytes.parse().unwrap_or(1 << 20);
            let label = format!("g2h-{}", identity());
            let a = stream_client(&Ep::Vsock { cid, port: sp }, len, &label).unwrap_or_else(|e| format!("FAIL stream {e}"));
            let b = if qp == 0 {
                "SKIP seqpacket".to_string()
            } else {
                seq_client(&Ep::Vsock { cid, port: qp }, &label).unwrap_or_else(|e| format!("FAIL seqpacket {e}"))
            };
            format!("{a}\n{b}")
        }
        ["unix-client", port, bytes] => {
            // unix backend: guest -> host CID 2 lands on <socket>_<port>.
            let p: u32 = port.parse().unwrap_or(6000);
            let len: usize = bytes.parse().unwrap_or(1 << 20);
            stream_client(&Ep::Vsock { cid: 2, port: p }, len, &format!("g2h-{}", identity()))
                .unwrap_or_else(|e| format!("FAIL stream {e}"))
        }
        ["spoof", host_cid, port] => spoof(host_cid.parse().unwrap_or(2), port.parse().unwrap_or(7000)),
        ["peer", cid, port] => {
            let c: u32 = cid.parse().unwrap_or(0);
            let p: u32 = port.parse().unwrap_or(AGENT_STREAM_PORT);
            let t0 = Instant::now();
            match vsock_connect(c, p, Ty::Stream, None) {
                Ok(fd) => format!("PEER cid={c} connect=ok local={} elapsed_ms={}", local_desc(fd.as_raw_fd()), t0.elapsed().as_millis()),
                Err(e) => format!("PEER cid={c} connect=failed err=[{e}] elapsed_ms={}", t0.elapsed().as_millis()),
            }
        }
        _ => format!("UNKNOWN {cmd}"),
    }
}

fn agent() {
    spawn_server(AGENT_STREAM_PORT, Ty::Stream);
    spawn_server(AGENT_SEQ_PORT, Ty::Seq);
    let ctl = listen_retry(AGENT_CTL_PORT, Ty::Stream);
    std::thread::sleep(std::time::Duration::from_millis(200));
    println!("AGENT_READY local_cid={:?}", local_cid());
    io::stdout().flush().ok();
    loop {
        let Ok(c) = accept(ctl.as_raw_fd()) else { continue };
        let fd = c.as_raw_fd();
        let Ok(cmd) = read_line(fd) else { continue };
        println!("AGENT_CMD {cmd} from={}", peer_desc(fd));
        match cmd.trim() {
            "reboot" | "poweroff" => {
                let _ = write_all(fd, b"OK going down\n");
                drop(c);
                unsafe { libc::sync() };
                let how = if cmd.trim() == "reboot" { libc::RB_AUTOBOOT } else { libc::RB_POWER_OFF };
                unsafe { libc::reboot(how) };
            }
            _ => {
                let reply = control_command(&cmd);
                println!("AGENT_REPLY {}", reply.replace('\n', " || "));
                let _ = write_all(fd, format!("{reply}\nEND\n").as_bytes());
            }
        }
        io::stdout().flush().ok();
    }
}

fn ctl(ep: &Ep, command: &str) -> io::Result<String> {
    let fd = connect(ep, Ty::Stream)?;
    write_all(fd.as_raw_fd(), format!("{command}\n").as_bytes())?;
    let mut out = String::new();
    let mut f = std::fs::File::from(fd);
    f.read_to_string(&mut out)?;
    Ok(out)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.get(1..).unwrap_or(&[]) {
        ["local-cid"] => match local_cid() {
            Ok(c) => println!("LOCAL_CID={c}"),
            Err(e) => die(e),
        },
        ["agent"] => agent(),
        ["client", ep, ty, bytes, label] => {
            let ep = Ep::parse(ep);
            let r = match Ty::parse(ty) {
                Ty::Stream => stream_client(&ep, bytes.parse().unwrap_or_else(|_| die("bytes")), label),
                Ty::Seq => seq_client(&ep, label),
            };
            match r {
                Ok(s) => println!("RESULT {s}"),
                Err(e) => {
                    println!("RESULT FAIL {e}");
                    std::process::exit(1);
                }
            }
        }
        ["serve", ep, ty, count] => serve(&Ep::parse(ep), Ty::parse(ty), count.parse().unwrap_or(1)),
        ["recorder", ep, count] => recorder(&Ep::parse(ep), count.parse().unwrap_or(1)),
        ["ctl", ep, rest @ ..] => match ctl(&Ep::parse(ep), &rest.join(" ")) {
            Ok(s) => print!("{s}"),
            Err(e) => {
                println!("CTL_FAIL {e}");
                std::process::exit(1);
            }
        },
        _ => die("usage: see module docs"),
    }
}
