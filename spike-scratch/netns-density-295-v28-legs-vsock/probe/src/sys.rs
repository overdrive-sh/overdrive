//! libc helpers and the byte-exact pattern.
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}
pub fn cvt(r: i64) -> io::Result<i64> {
    if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
}
pub fn now_us() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as u64
}
pub fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

// ------------------------------------------------------------ pattern
const PL: usize = 1_000_003;
pub const START_A: usize = 0; // client -> app
pub const START_B: usize = 777_777; // app -> client
static BLK: OnceLock<Vec<u8>> = OnceLock::new();
fn blk() -> &'static [u8] {
    BLK.get_or_init(|| {
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        (0..PL)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    })
}
pub fn fill(buf: &mut [u8], start: usize, off: u64) {
    let b = blk();
    let mut p = (start + (off % PL as u64) as usize) % PL;
    let mut i = 0;
    while i < buf.len() {
        let n = (PL - p).min(buf.len() - i);
        buf[i..i + n].copy_from_slice(&b[p..p + n]);
        i += n;
        p = (p + n) % PL;
    }
}
/// first mismatching index, or None
pub fn check(buf: &[u8], start: usize, off: u64) -> Option<usize> {
    let b = blk();
    let mut p = (start + (off % PL as u64) as usize) % PL;
    let mut i = 0;
    while i < buf.len() {
        let n = (PL - p).min(buf.len() - i);
        if buf[i..i + n] != b[p..p + n] {
            return Some(i + buf[i..i + n].iter().zip(&b[p..p + n]).position(|(a, b)| a != b).unwrap());
        }
        i += n;
        p = (p + n) % PL;
    }
    None
}

// ------------------------------------------------------------ sockets
#[repr(C)]
pub struct SockaddrVm {
    pub family: u16,
    pub reserved: u16,
    pub port: u32,
    pub cid: u32,
    pub flags: u8,
    pub zero: [u8; 3],
}
pub fn vm_addr(cid: u32, port: u32) -> SockaddrVm {
    SockaddrVm { family: libc::AF_VSOCK as u16, reserved: 0, port, cid, flags: 0, zero: [0; 3] }
}
pub fn socket(domain: i32, ty: i32) -> io::Result<OwnedFd> {
    let fd = cvt(unsafe { libc::socket(domain, ty | libc::SOCK_CLOEXEC, 0) } as i64)? as i32;
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}
pub fn vsock_listen(port: u32) -> io::Result<OwnedFd> {
    let fd = socket(libc::AF_VSOCK, libc::SOCK_STREAM)?;
    let a = vm_addr(libc::VMADDR_CID_ANY, port);
    cvt(unsafe { libc::bind(fd.as_raw_fd(), &a as *const _ as *const _, 16) } as i64)?;
    cvt(unsafe { libc::listen(fd.as_raw_fd(), 1024) } as i64)?;
    Ok(fd)
}
pub fn vsock_connect(cid: u32, port: u32) -> io::Result<OwnedFd> {
    let fd = socket(libc::AF_VSOCK, libc::SOCK_STREAM)?;
    let a = vm_addr(cid, port);
    cvt(unsafe { libc::connect(fd.as_raw_fd(), &a as *const _ as *const _, 16) } as i64)?;
    Ok(fd)
}
pub fn accept(fd: RawFd) -> io::Result<OwnedFd> {
    let n = cvt(unsafe { libc::accept4(fd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) } as i64)? as i32;
    Ok(unsafe { OwnedFd::from_raw_fd(n) })
}
pub fn sin(a: SocketAddrV4) -> libc::sockaddr_in {
    libc::sockaddr_in {
        sin_family: libc::AF_INET as u16,
        sin_port: a.port().to_be(),
        sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes(a.ip().octets()) },
        sin_zero: [0; 8],
    }
}
pub fn from_sin(s: &libc::sockaddr_in) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::from(s.sin_addr.s_addr.to_ne_bytes()), u16::from_be(s.sin_port))
}
pub fn bind4(fd: RawFd, a: SocketAddrV4) -> io::Result<()> {
    let s = sin(a);
    cvt(unsafe { libc::bind(fd, &s as *const _ as *const _, 16) } as i64)?;
    Ok(())
}
pub fn connect4(fd: RawFd, a: SocketAddrV4) -> io::Result<()> {
    let s = sin(a);
    cvt(unsafe { libc::connect(fd, &s as *const _ as *const _, 16) } as i64)?;
    Ok(())
}
/// connect with a deadline: Ok / Err(raw errno) ; ETIMEDOUT on deadline. Socket left blocking.
pub fn connect4_timeout(fd: RawFd, a: SocketAddrV4, ms: i32) -> Result<(), i32> {
    nonblock(fd, true);
    let s = sin(a);
    let r = unsafe { libc::connect(fd, &s as *const _ as *const _, 16) };
    let res = if r == 0 {
        Ok(())
    } else if errno() == libc::EINPROGRESS {
        let mut p = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
        let n = unsafe { libc::poll(&mut p, 1, ms) };
        if n == 0 {
            Err(libc::ETIMEDOUT)
        } else {
            let e = so_error(fd);
            if e == 0 { Ok(()) } else { Err(e) }
        }
    } else {
        Err(errno())
    };
    nonblock(fd, false);
    res
}
pub fn tcp_listen(a: SocketAddrV4, backlog: i32) -> io::Result<OwnedFd> {
    let fd = socket(libc::AF_INET, libc::SOCK_STREAM)?;
    setopt(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_REUSEADDR, 1)?;
    bind4(fd.as_raw_fd(), a)?;
    cvt(unsafe { libc::listen(fd.as_raw_fd(), backlog) } as i64)?;
    Ok(fd)
}
pub fn local4(fd: RawFd) -> io::Result<SocketAddrV4> {
    let mut s: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut l = 16u32;
    cvt(unsafe { libc::getsockname(fd, &mut s as *mut _ as *mut _, &mut l) } as i64)?;
    Ok(from_sin(&s))
}
pub fn peer4(fd: RawFd) -> io::Result<SocketAddrV4> {
    let mut s: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut l = 16u32;
    cvt(unsafe { libc::getpeername(fd, &mut s as *mut _ as *mut _, &mut l) } as i64)?;
    Ok(from_sin(&s))
}
pub fn setopt(fd: RawFd, level: i32, name: i32, v: i32) -> io::Result<()> {
    cvt(unsafe { libc::setsockopt(fd, level, name, &v as *const _ as *const _, 4) } as i64)?;
    Ok(())
}
pub fn so_error(fd: RawFd) -> i32 {
    let mut v = 0i32;
    let mut l = 4u32;
    unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, &mut v as *mut _ as *mut _, &mut l) };
    v
}
pub fn linger0(fd: RawFd) {
    let l = libc::linger { l_onoff: 1, l_linger: 0 };
    unsafe { libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_LINGER, &l as *const _ as *const _, 8) };
}
pub fn nonblock(fd: RawFd, on: bool) {
    unsafe {
        let f = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, if on { f | libc::O_NONBLOCK } else { f & !libc::O_NONBLOCK });
    }
}
pub fn shutdown_wr(fd: RawFd) -> i32 {
    unsafe { libc::shutdown(fd, libc::SHUT_WR) }
}
pub fn rd(fd: RawFd, buf: &mut [u8]) -> isize {
    unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) }
}
pub fn wr(fd: RawFd, buf: &[u8]) -> isize {
    unsafe { libc::send(fd, buf.as_ptr().cast(), buf.len(), libc::MSG_NOSIGNAL) }
}
pub fn write_all(fd: RawFd, mut buf: &[u8]) -> Result<(), i32> {
    while !buf.is_empty() {
        let n = wr(fd, buf);
        if n < 0 {
            let e = errno();
            if e == libc::EINTR { continue; }
            return Err(e);
        }
        buf = &buf[n as usize..];
    }
    Ok(())
}
pub fn read_exact(fd: RawFd, buf: &mut [u8]) -> Result<(), i32> {
    let mut o = 0;
    while o < buf.len() {
        let n = rd(fd, &mut buf[o..]);
        if n < 0 {
            let e = errno();
            if e == libc::EINTR { continue; }
            return Err(e);
        }
        if n == 0 { return Err(0); }
        o += n as usize;
    }
    Ok(())
}
pub fn set_timeouts(fd: RawFd, ms: u64) {
    let tv = libc::timeval { tv_sec: (ms / 1000) as i64, tv_usec: ((ms % 1000) * 1000) as i64 };
    unsafe {
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const _, 16);
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDTIMEO, &tv as *const _ as *const _, 16);
    }
}
/// poll one fd; returns revents (0 on timeout)
pub fn poll1(fd: RawFd, events: i16, ms: i32) -> i16 {
    let mut p = libc::pollfd { fd, events, revents: 0 };
    let n = unsafe { libc::poll(&mut p, 1, ms) };
    if n <= 0 { 0 } else { p.revents }
}
/// vsock half-close discriminator: a zero-length send fails with EPIPE iff the peer sent
/// SHUTDOWN with the RCV flag (peer will not read: close()/abort); returns 0 after a bare SHUT_WR.
pub fn zero_send(fd: RawFd) -> i32 {
    let r = unsafe { libc::send(fd, std::ptr::null(), 0, libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT) };
    if r < 0 { errno() } else { 0 }
}
pub fn args_map(a: &[String]) -> std::collections::BTreeMap<String, String> {
    let mut m = std::collections::BTreeMap::new();
    let mut i = 0;
    while i < a.len() {
        if let Some(k) = a[i].strip_prefix("--") {
            if i + 1 < a.len() && !a[i + 1].starts_with("--") {
                m.insert(k.to_string(), a[i + 1].clone());
                i += 2;
                continue;
            }
            m.insert(k.to_string(), "1".to_string());
        }
        i += 1;
    }
    m
}
pub fn uname() -> String {
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    unsafe { libc::uname(&mut u) };
    unsafe { std::ffi::CStr::from_ptr(u.release.as_ptr()) }.to_string_lossy().to_string()
}

/// EOF classification mode for a vsock source: 0 = every EOF is a clean half-close (errors still abort),
/// 1 = zero-length-send discriminator (EPIPE => abort), 2 = kernel tracepoint (see tp.rs; not built yet).
pub static EOF_CLASS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
pub fn set_eof_class(m: &std::collections::BTreeMap<String, String>) {
    let v = match m.get("eof-class").map(|s| s.as_str()) { Some("zero") => 1, Some("tp") => 2, _ => 0 };
    EOF_CLASS.store(v, std::sync::atomic::Ordering::SeqCst);
}

/// (peer_cid, peer_port, local_port) of a connected AF_VSOCK socket
pub fn vsock_names(fd: RawFd) -> Option<(u32, u32, u32)> {
    let mut p = vm_addr(0, 0);
    let mut l = vm_addr(0, 0);
    let (mut pl, mut ll) = (16u32, 16u32);
    if unsafe { libc::getpeername(fd, &mut p as *mut _ as *mut _, &mut pl) } < 0 { return None; }
    if unsafe { libc::getsockname(fd, &mut l as *mut _ as *mut _, &mut ll) } < 0 { return None; }
    Some((p.cid, p.port, l.port))
}
