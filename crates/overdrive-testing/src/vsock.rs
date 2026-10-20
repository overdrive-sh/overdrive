//! Raw `AF_VSOCK` stream sockets for the native-metal microVM lanes (GH #295,
//! T-1): the host end of a guest-kit fixture's scripted guest
//! (`guest_peer`), the guest end inside the scripted guest
//! (`bin/guest_peer.rs`), and the host halves the `overdrive-fwd` guest
//! boundary cells hand to the host forwarder.
//!
//! Test support only. Every constant is the kernel's
//! (`include/uapi/linux/vm_sockets.h @ v7.0`); every call is the plain
//! syscall, so what a test observes is the kernel's behaviour, not a
//! wrapper's.

#![cfg(target_os = "linux")]
#![allow(clippy::doc_markdown, reason = "kernel symbol names in prose")]
#![allow(
    clippy::too_long_first_doc_paragraph,
    reason = "protocol docs open with the full sentence they define"
)]
#![allow(
    clippy::cast_possible_truncation,
    reason = "`AF_VSOCK` and `sizeof(struct sockaddr_vm)` fit their C field types"
)]

use std::io::{self, Read, Write};
use std::mem;
use std::net::Shutdown;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

/// `VMADDR_CID_ANY` (`vm_sockets.h:117`).
pub const VMADDR_CID_ANY: u32 = u32::MAX;
/// `VMADDR_CID_LOCAL` (`vm_sockets.h:124`): the loopback transport.
pub const VMADDR_CID_LOCAL: u32 = 1;
/// `VMADDR_CID_HOST` (`vm_sockets.h:127`): the host, as a guest addresses it.
pub const VMADDR_CID_HOST: u32 = 2;
/// `VMADDR_PORT_ANY` (`vm_sockets.h:109`).
pub const VMADDR_PORT_ANY: u32 = u32::MAX;

/// A vsock address: context id and port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VsockAddr {
    /// The context id.
    pub cid: u32,
    /// The port.
    pub port: u32,
}

/// The syscall's result, or its errno as an `io::Error` whose
/// `raw_os_error()` is the kernel's errno (tests branch on it; the syscall's
/// name is in the caller's assertion text).
fn check(ret: libc::c_int, _what: &str) -> io::Result<libc::c_int> {
    if ret < 0 { Err(io::Error::last_os_error()) } else { Ok(ret) }
}

const fn sockaddr_vm(addr: VsockAddr) -> libc::sockaddr_vm {
    // SAFETY: `sockaddr_vm` is plain old data; all-zero is a valid value.
    let mut raw: libc::sockaddr_vm = unsafe { mem::zeroed() };
    raw.svm_family = libc::AF_VSOCK as libc::sa_family_t;
    raw.svm_cid = addr.cid;
    raw.svm_port = addr.port;
    raw
}

const SOCKADDR_VM_LEN: libc::socklen_t = mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t;

fn new_stream_fd() -> io::Result<OwnedFd> {
    // SAFETY: plain syscall; the result is checked before it is owned.
    let raw = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    let raw = check(raw, "socket(AF_VSOCK, SOCK_STREAM)")?;
    // SAFETY: `raw` is a fresh descriptor this process owns.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn bind(fd: &OwnedFd, addr: VsockAddr) -> io::Result<()> {
    let raw = sockaddr_vm(addr);
    // SAFETY: `raw` is a valid `sockaddr_vm` of the passed length.
    let ret = unsafe { libc::bind(fd.as_raw_fd(), (&raw const raw).cast(), SOCKADDR_VM_LEN) };
    check(ret, "bind(AF_VSOCK)").map(drop)
}

fn name_of(fd: BorrowedFd<'_>, peer: bool) -> io::Result<VsockAddr> {
    // SAFETY: zeroed POD out-parameter of the passed length.
    let mut raw: libc::sockaddr_vm = unsafe { mem::zeroed() };
    let mut len = SOCKADDR_VM_LEN;
    // SAFETY: `raw` and `len` are valid out-parameters.
    let ret = unsafe {
        if peer {
            libc::getpeername(fd.as_raw_fd(), (&raw mut raw).cast(), &raw mut len)
        } else {
            libc::getsockname(fd.as_raw_fd(), (&raw mut raw).cast(), &raw mut len)
        }
    };
    check(ret, if peer { "getpeername(AF_VSOCK)" } else { "getsockname(AF_VSOCK)" })?;
    Ok(VsockAddr { cid: raw.svm_cid, port: raw.svm_port })
}

/// Waits up to `bound` for `events` on `fd`. `Ok(true)`: ready.
///
/// # Errors
///
/// `poll(2)`'s error.
pub fn wait_ready(fd: BorrowedFd<'_>, events: libc::c_short, bound: Duration) -> io::Result<bool> {
    let deadline = Instant::now() + bound;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let millis = libc::c_int::try_from(left.as_millis()).unwrap_or(libc::c_int::MAX);
        let mut pfd = libc::pollfd { fd: fd.as_raw_fd(), events, revents: 0 };
        // SAFETY: one valid `pollfd`.
        let ret = unsafe { libc::poll(&raw mut pfd, 1, millis) };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        return Ok(ret > 0);
    }
}

/// A listening vsock stream socket.
#[derive(Debug)]
pub struct VsockListener {
    fd: OwnedFd,
    port: u32,
}

impl AsFd for VsockListener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl VsockListener {
    /// Binds `(cid, port)` (`port` may be [`VMADDR_PORT_ANY`]) and listens.
    ///
    /// # Errors
    ///
    /// The kernel's errno (`EADDRINUSE` when the port is taken).
    pub fn bind(cid: u32, port: u32) -> io::Result<Self> {
        let fd = new_stream_fd()?;
        bind(&fd, VsockAddr { cid, port })?;
        // SAFETY: plain syscall on an owned descriptor.
        check(unsafe { libc::listen(fd.as_raw_fd(), 128) }, "listen(AF_VSOCK)")?;
        let port = name_of(fd.as_fd(), false)?.port;
        Ok(Self { fd, port })
    }

    /// The bound port.
    #[must_use]
    pub const fn port(&self) -> u32 {
        self.port
    }

    /// The next connection and its peer address, blocking.
    ///
    /// # Errors
    ///
    /// `accept4(2)`'s error.
    pub fn accept(&self) -> io::Result<(VsockStream, VsockAddr)> {
        // SAFETY: zeroed POD out-parameter of the passed length.
        let mut raw: libc::sockaddr_vm = unsafe { mem::zeroed() };
        let mut len = SOCKADDR_VM_LEN;
        // SAFETY: `raw` and `len` are valid out-parameters.
        let fd = unsafe {
            libc::accept4(
                self.fd.as_raw_fd(),
                (&raw mut raw).cast(),
                &raw mut len,
                libc::SOCK_CLOEXEC,
            )
        };
        let fd = check(fd, "accept4(AF_VSOCK)")?;
        // SAFETY: a fresh descriptor this process owns.
        let stream = VsockStream { fd: unsafe { OwnedFd::from_raw_fd(fd) } };
        Ok((stream, VsockAddr { cid: raw.svm_cid, port: raw.svm_port }))
    }

    /// The next connection within `bound`, or `None`.
    ///
    /// # Errors
    ///
    /// `poll(2)`'s or `accept4(2)`'s error.
    pub fn accept_within(&self, bound: Duration) -> io::Result<Option<(VsockStream, VsockAddr)>> {
        if wait_ready(self.fd.as_fd(), libc::POLLIN, bound)? {
            self.accept().map(Some)
        } else {
            Ok(None)
        }
    }
}

/// A connected vsock stream socket.
#[derive(Debug)]
pub struct VsockStream {
    fd: OwnedFd,
}

impl AsFd for VsockStream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl From<VsockStream> for OwnedFd {
    fn from(stream: VsockStream) -> Self {
        stream.fd
    }
}

impl VsockStream {
    /// Connects to `(cid, port)`, first binding the local port `source_port`
    /// when `Some`.
    ///
    /// # Errors
    ///
    /// The kernel's errno (`ECONNRESET` when no listener holds the port,
    /// `ENODEV` when the peer is gone).
    pub fn connect(cid: u32, port: u32, source_port: Option<u32>) -> io::Result<Self> {
        let fd = new_stream_fd()?;
        if let Some(source) = source_port {
            bind(&fd, VsockAddr { cid: VMADDR_CID_ANY, port: source })?;
        }
        let raw = sockaddr_vm(VsockAddr { cid, port });
        // SAFETY: `raw` is a valid `sockaddr_vm` of the passed length.
        let ret =
            unsafe { libc::connect(fd.as_raw_fd(), (&raw const raw).cast(), SOCKADDR_VM_LEN) };
        check(ret, "connect(AF_VSOCK)")?;
        Ok(Self { fd })
    }

    /// Connects with retries until `bound` (a guest's virtio-vsock device is
    /// probed asynchronously after its module loads).
    ///
    /// # Errors
    ///
    /// The last attempt's error.
    pub fn connect_retrying(cid: u32, port: u32, bound: Duration) -> io::Result<Self> {
        let deadline = Instant::now() + bound;
        loop {
            match Self::connect(cid, port, None) {
                Ok(stream) => return Ok(stream),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    /// The local address.
    ///
    /// # Errors
    ///
    /// `getsockname(2)`'s error.
    pub fn local_addr(&self) -> io::Result<VsockAddr> {
        name_of(self.fd.as_fd(), false)
    }

    /// The peer address.
    ///
    /// # Errors
    ///
    /// `getpeername(2)`'s error.
    pub fn peer_addr(&self) -> io::Result<VsockAddr> {
        name_of(self.fd.as_fd(), true)
    }

    /// `shutdown(2)`.
    ///
    /// # Errors
    ///
    /// The kernel's errno.
    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        let how = match how {
            Shutdown::Read => libc::SHUT_RD,
            Shutdown::Write => libc::SHUT_WR,
            Shutdown::Both => libc::SHUT_RDWR,
        };
        // SAFETY: plain syscall on an owned descriptor.
        check(unsafe { libc::shutdown(self.fd.as_raw_fd(), how) }, "shutdown(AF_VSOCK)").map(drop)
    }

    /// A second handle on the same socket.
    ///
    /// # Errors
    ///
    /// `fcntl(F_DUPFD_CLOEXEC)`'s error.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self { fd: self.fd.try_clone()? })
    }

    /// One `read` within `bound`: `Ok(None)` when nothing arrived.
    ///
    /// # Errors
    ///
    /// `read(2)`'s error (`ECONNRESET` after a reset, `ENOTCONN` after a
    /// shut).
    pub fn read_within(&self, buf: &mut [u8], bound: Duration) -> io::Result<Option<usize>> {
        if wait_ready(self.fd.as_fd(), libc::POLLIN | libc::POLLRDHUP, bound)? {
            (&*self).read(buf).map(Some)
        } else {
            Ok(None)
        }
    }
}

impl Read for &VsockStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: `buf` is valid for `buf.len()` bytes.
        let ret = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if ret < 0 { Err(io::Error::last_os_error()) } else { Ok(ret.unsigned_abs()) }
    }
}

impl Write for &VsockStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: `buf` is valid for `buf.len()` bytes.
        let ret = unsafe { libc::write(self.fd.as_raw_fd(), buf.as_ptr().cast(), buf.len()) };
        if ret < 0 { Err(io::Error::last_os_error()) } else { Ok(ret.unsigned_abs()) }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for VsockStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self).read(buf)
    }
}

impl Write for VsockStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
