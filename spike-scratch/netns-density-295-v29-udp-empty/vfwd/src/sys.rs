//! Raw socket helpers (libc). Control-path only: nothing here reads or writes
//! application payload on a paired socket.
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

pub fn errno() -> io::Error {
    io::Error::last_os_error()
}

pub fn cvt(r: i32) -> io::Result<i32> {
    if r < 0 {
        Err(errno())
    } else {
        Ok(r)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
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
    let fd = cvt(unsafe { libc::socket(domain, ty | libc::SOCK_CLOEXEC, 0) })?;
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

pub fn vsock_listen(cid: u32, port: u32, ty: i32) -> io::Result<OwnedFd> {
    let fd = socket(libc::AF_VSOCK, ty)?;
    let a = vm_addr(cid, port);
    cvt(unsafe { libc::bind(fd.as_raw_fd(), &a as *const _ as *const _, 16) })?;
    cvt(unsafe { libc::listen(fd.as_raw_fd(), 1024) })?;
    Ok(fd)
}

pub fn vsock_connect(cid: u32, port: u32, ty: i32) -> io::Result<OwnedFd> {
    let fd = socket(libc::AF_VSOCK, ty)?;
    let a = vm_addr(cid, port);
    cvt(unsafe { libc::connect(fd.as_raw_fd(), &a as *const _ as *const _, 16) })?;
    Ok(fd)
}

pub fn accept(fd: RawFd) -> io::Result<OwnedFd> {
    let n = cvt(unsafe { libc::accept4(fd, std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_CLOEXEC) })?;
    Ok(unsafe { OwnedFd::from_raw_fd(n) })
}

pub fn vsock_peer(fd: RawFd) -> io::Result<(u32, u32)> {
    let mut a = vm_addr(0, 0);
    let mut len = 16u32;
    cvt(unsafe { libc::getpeername(fd, &mut a as *mut _ as *mut _, &mut len) })?;
    Ok((a.cid, a.port))
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
    cvt(unsafe { libc::bind(fd, &s as *const _ as *const _, 16) }).map(|_| ())
}

pub fn connect4(fd: RawFd, a: SocketAddrV4) -> io::Result<()> {
    let s = sin(a);
    cvt(unsafe { libc::connect(fd, &s as *const _ as *const _, 16) }).map(|_| ())
}

pub fn disconnect(fd: RawFd) -> io::Result<()> {
    let s = libc::sockaddr { sa_family: libc::AF_UNSPEC as u16, sa_data: [0; 14] };
    cvt(unsafe { libc::connect(fd, &s as *const _, 16) }).map(|_| ())
}

/// Non-blocking connect with a deadline, then back to blocking.
pub fn connect4_timeout(fd: RawFd, a: SocketAddrV4, t: Duration) -> io::Result<()> {
    unsafe {
        let fl = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
        let s = sin(a);
        let r = libc::connect(fd, &s as *const _ as *const _, 16);
        let res = if r == 0 {
            Ok(())
        } else if errno().raw_os_error() == Some(libc::EINPROGRESS) {
            let mut p = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
            let n = libc::poll(&mut p, 1, t.as_millis() as i32);
            if n == 0 {
                Err(io::Error::from_raw_os_error(libc::ETIMEDOUT))
            } else {
                let e = so_error(fd);
                if e == 0 {
                    Ok(())
                } else {
                    Err(io::Error::from_raw_os_error(e))
                }
            }
        } else {
            Err(errno())
        };
        libc::fcntl(fd, libc::F_SETFL, fl);
        res
    }
}

pub fn local4(fd: RawFd) -> io::Result<SocketAddrV4> {
    let mut s: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = 16u32;
    cvt(unsafe { libc::getsockname(fd, &mut s as *mut _ as *mut _, &mut len) })?;
    Ok(from_sin(&s))
}

pub fn peer4(fd: RawFd) -> io::Result<SocketAddrV4> {
    let mut s: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = 16u32;
    cvt(unsafe { libc::getpeername(fd, &mut s as *mut _ as *mut _, &mut len) })?;
    Ok(from_sin(&s))
}

pub fn setopt_i32(fd: RawFd, level: i32, name: i32, v: i32) -> io::Result<()> {
    cvt(unsafe { libc::setsockopt(fd, level, name, &v as *const _ as *const _, 4) }).map(|_| ())
}

pub fn so_error(fd: RawFd) -> i32 {
    let mut e = 0i32;
    let mut len = 4u32;
    unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, &mut e as *mut _ as *mut _, &mut len) };
    e
}

pub fn cookie(fd: RawFd) -> u64 {
    let mut n = 0u64;
    let mut len = 8u32;
    let r = unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, 57 /*SO_COOKIE*/, &mut n as *mut _ as *mut _, &mut len) };
    assert_eq!(r, 0, "SO_COOKIE: {}", errno());
    n
}

pub fn set_timeouts(fd: RawFd, t: Duration) {
    let tv = libc::timeval { tv_sec: t.as_secs() as i64, tv_usec: t.subsec_micros() as i64 };
    unsafe {
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const _, 16);
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDTIMEO, &tv as *const _ as *const _, 16);
    }
}

/// Exact-length control read (deadline). Never reads past `buf.len()`.
pub fn read_exact_ctl(fd: RawFd, buf: &mut [u8], t: Duration) -> io::Result<()> {
    let deadline = Instant::now() + t;
    let mut off = 0;
    while off < buf.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::from_raw_os_error(libc::ETIMEDOUT));
        }
        let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        unsafe { libc::poll(&mut p, 1, left.as_millis().max(1) as i32) };
        let n = unsafe { libc::recv(fd, buf[off..].as_mut_ptr() as *mut _, buf.len() - off, libc::MSG_DONTWAIT) };
        if n == 0 {
            return Err(io::Error::from_raw_os_error(libc::ECONNRESET));
        }
        if n < 0 {
            let e = errno();
            if e.raw_os_error() == Some(libc::EAGAIN) {
                continue;
            }
            return Err(e);
        }
        off += n as usize;
    }
    Ok(())
}

pub fn write_all_ctl(fd: RawFd, buf: &[u8]) -> io::Result<()> {
    let mut off = 0;
    while off < buf.len() {
        let n = unsafe { libc::send(fd, buf[off..].as_ptr() as *const _, buf.len() - off, libc::MSG_NOSIGNAL) };
        if n < 0 {
            return Err(errno());
        }
        off += n as usize;
    }
    Ok(())
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct TcpInfoPrefix {
    pub state: u8,
    pub ca_state: u8,
    pub retransmits: u8,
    pub probes: u8,
    pub backoff: u8,
    pub options: u8,
    pub wscale: u8,
    pub flags: u8,
    pub u32s: [u32; 24],
    pub pacing_rate: u64,
    pub max_pacing_rate: u64,
    pub bytes_acked: u64,
    pub bytes_received: u64,
}

pub fn tcp_bytes_received(fd: RawFd) -> Option<u64> {
    let mut t = TcpInfoPrefix::default();
    let mut len = std::mem::size_of::<TcpInfoPrefix>() as u32;
    let r = unsafe { libc::getsockopt(fd, libc::IPPROTO_TCP, libc::TCP_INFO, &mut t as *mut _ as *mut _, &mut len) };
    if r != 0 || (len as usize) < std::mem::size_of::<TcpInfoPrefix>() {
        return None;
    }
    Some(t.bytes_received)
}

pub fn inq(fd: RawFd) -> i32 {
    let mut n = 0i32;
    unsafe { libc::ioctl(fd, libc::FIONREAD, &mut n) };
    n
}

pub fn linger0_close(fd: OwnedFd) {
    let l = libc::linger { l_onoff: 1, l_linger: 0 };
    unsafe { libc::setsockopt(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_LINGER, &l as *const _ as *const _, 8) };
    drop(fd);
}

pub struct Epoll(pub OwnedFd);
impl Epoll {
    pub fn new() -> Self {
        Epoll(unsafe { OwnedFd::from_raw_fd(libc::epoll_create1(libc::EPOLL_CLOEXEC)) })
    }
    pub fn add(&self, fd: RawFd, ev: u32) {
        let mut e = libc::epoll_event { events: ev, u64: fd as u64 };
        let r = unsafe { libc::epoll_ctl(self.0.as_raw_fd(), libc::EPOLL_CTL_ADD, fd, &mut e) };
        if r != 0 {
            unsafe { libc::epoll_ctl(self.0.as_raw_fd(), libc::EPOLL_CTL_MOD, fd, &mut e) };
        }
    }
    pub fn modify(&self, fd: RawFd, ev: u32) {
        let mut e = libc::epoll_event { events: ev, u64: fd as u64 };
        unsafe { libc::epoll_ctl(self.0.as_raw_fd(), libc::EPOLL_CTL_MOD, fd, &mut e) };
    }
    pub fn del(&self, fd: RawFd) {
        unsafe { libc::epoll_ctl(self.0.as_raw_fd(), libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut()) };
    }
    pub fn wait(&self, ms: i32) -> Vec<(RawFd, u32)> {
        let mut evs = [libc::epoll_event { events: 0, u64: 0 }; 64];
        let n = unsafe { libc::epoll_wait(self.0.as_raw_fd(), evs.as_mut_ptr(), 64, ms) };
        if n <= 0 {
            return vec![];
        }
        evs[..n as usize].iter().map(|e| (e.u64 as RawFd, e.events)).collect()
    }
}

pub fn now_ms() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut t) };
    t.tv_sec as f64 * 1000.0 + t.tv_nsec as f64 / 1e6
}
