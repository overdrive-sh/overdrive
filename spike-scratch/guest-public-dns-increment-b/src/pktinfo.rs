//! Spike-only UDP socket with IP_PKTINFO (wildcard bind), mirroring the production
//! responder's receive shape (`dns_responder/responder.rs`: SO_REUSEADDR + IP_PKTINFO).
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};

pub struct Pkt {
    pub src: SocketAddrV4,
    pub ifindex: i32,
    pub spec_dst: Ipv4Addr,
    pub ipi_addr: Ipv4Addr,
    pub data: Vec<u8>,
}

pub struct PktSock(libc::c_int);

fn ip(a: libc::in_addr) -> Ipv4Addr { Ipv4Addr::from(u32::from_be(a.s_addr)) }

impl PktSock {
    pub fn bind_wildcard(port: u16, timeout_ms: u64) -> io::Result<Self> {
        unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
            if fd < 0 { return Err(io::Error::last_os_error()); }
            let one: libc::c_int = 1;
            let sz = std::mem::size_of::<libc::c_int>() as u32;
            for (lvl, opt) in [(libc::SOL_SOCKET, libc::SO_REUSEADDR), (libc::IPPROTO_IP, libc::IP_PKTINFO)] {
                if libc::setsockopt(fd, lvl, opt, &one as *const _ as *const _, sz) < 0 {
                    let e = io::Error::last_os_error(); libc::close(fd); return Err(e);
                }
            }
            let tv = libc::timeval { tv_sec: (timeout_ms / 1000) as _, tv_usec: ((timeout_ms % 1000) * 1000) as _ };
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const _, std::mem::size_of::<libc::timeval>() as u32);
            let mut sa: libc::sockaddr_in = std::mem::zeroed();
            sa.sin_family = libc::AF_INET as _;
            sa.sin_port = port.to_be();
            if libc::bind(fd, &sa as *const _ as *const _, std::mem::size_of::<libc::sockaddr_in>() as u32) < 0 {
                let e = io::Error::last_os_error(); libc::close(fd); return Err(e);
            }
            Ok(Self(fd))
        }
    }

    /// `Ok(None)` on receive timeout.
    pub fn recv(&self) -> io::Result<Option<Pkt>> {
        unsafe {
            let mut buf = [0u8; 2048];
            let mut cbuf = [0u64; 16];
            let mut src: libc::sockaddr_in = std::mem::zeroed();
            let mut iov = libc::iovec { iov_base: buf.as_mut_ptr() as *mut _, iov_len: buf.len() };
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_name = &mut src as *mut _ as *mut _;
            msg.msg_namelen = std::mem::size_of::<libc::sockaddr_in>() as u32;
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = cbuf.as_mut_ptr() as *mut _;
            msg.msg_controllen = std::mem::size_of_val(&cbuf) as _;
            let n = libc::recvmsg(self.0, &mut msg, 0);
            if n < 0 {
                let e = io::Error::last_os_error();
                return if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) { Ok(None) } else { Err(e) };
            }
            let (mut spec_dst, mut ipi_addr, mut ifindex) = (Ipv4Addr::UNSPECIFIED, Ipv4Addr::UNSPECIFIED, 0);
            let mut c = libc::CMSG_FIRSTHDR(&msg);
            while !c.is_null() {
                if (*c).cmsg_level == libc::IPPROTO_IP && (*c).cmsg_type == libc::IP_PKTINFO {
                    let pi = std::ptr::read_unaligned(libc::CMSG_DATA(c) as *const libc::in_pktinfo);
                    spec_dst = ip(pi.ipi_spec_dst); ipi_addr = ip(pi.ipi_addr); ifindex = pi.ipi_ifindex;
                }
                c = libc::CMSG_NXTHDR(&msg, c);
            }
            Ok(Some(Pkt {
                src: SocketAddrV4::new(ip(src.sin_addr), u16::from_be(src.sin_port)),
                ifindex, spec_dst, ipi_addr, data: buf[..n as usize].to_vec(),
            }))
        }
    }
}
impl Drop for PktSock { fn drop(&mut self) { unsafe { libc::close(self.0); } } }
