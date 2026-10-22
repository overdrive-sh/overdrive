//! Shared fixtures of the forwarder's Tier-3 suite: real sockets on the
//! host role (vsock loopback, TCP and UDP loopback), the end-record wait,
//! and the namespace-scoped fault injections of the probe's gold test.
//!
//! Every fixture that can fail panics naming the broken precondition
//! (`.claude/rules/debugging.md` § 8): no `let _` on fallible setup.

use std::fs;
use std::io::{self, ErrorKind, Read, Write};
use std::mem;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use nix::libc;
use overdrive_core::guest_transport::forwarder::{
    AbortCause, AbortResult, FlowForwarder, ForwardEnd, ForwardId, ForwardOutcome,
    ForwarderInventory, PairMode,
};
use overdrive_fwd::KernelFlowForwarder;

/// The bound on every wait for a kernel event in this suite. The measured
/// propagation is well under a millisecond typical, 202 ms worst (P-68);
/// the bound is generous so a slow CI host fails only on a real defect.
pub const END_WAIT: Duration = Duration::from_secs(10);

/// How long a case observes that something does NOT happen.
pub const QUIET: Duration = Duration::from_millis(500);

/// `VMADDR_CID_LOCAL` (`include/uapi/linux/vm_sockets.h:124 @ v7.0`).
pub const VMADDR_CID_LOCAL: u32 = 1;
/// `VMADDR_PORT_ANY` (`vm_sockets.h:109`).
const VMADDR_PORT_ANY: u32 = u32::MAX;
/// `SO_VM_SOCKETS_BUFFER_SIZE` (`vm_sockets.h:33`), level `AF_VSOCK`.
const SO_VM_SOCKETS_BUFFER_SIZE: libc::c_int = 0;
/// `SO_VM_SOCKETS_BUFFER_MAX_SIZE` (`vm_sockets.h:49`).
const SO_VM_SOCKETS_BUFFER_MAX_SIZE: libc::c_int = 2;
/// `TCP_ULP` (`include/uapi/linux/tcp.h:124 @ v7.0`).
const TCP_ULP: libc::c_int = 31;

/// Opens a forwarder context; panics naming the precondition.
pub fn open_context() -> KernelFlowForwarder {
    KernelFlowForwarder::open()
        .expect("precondition: ovd_fwd is loaded and /dev/ovd_fwd opens (EVMOD-LOAD)")
}

// ---------------------------------------------------------------------------
// vsock STREAM sockets on the loopback transport
// ---------------------------------------------------------------------------

/// An `AF_VSOCK` `SOCK_STREAM` socket, blocking unless set otherwise.
#[derive(Debug)]
pub struct Vsock {
    fd: OwnedFd,
}

impl AsFd for Vsock {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

fn check(ret: libc::c_int, what: &str) -> io::Result<libc::c_int> {
    if ret < 0 {
        let err = io::Error::last_os_error();
        Err(io::Error::new(err.kind(), format!("{what}: {err}")))
    } else {
        Ok(ret)
    }
}

fn sockaddr_vm(cid: u32, port: u32) -> libc::sockaddr_vm {
    // SAFETY: `sockaddr_vm` is plain old data; all-zero is a valid value.
    let mut addr: libc::sockaddr_vm = unsafe { mem::zeroed() };
    addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
    addr.svm_cid = cid;
    addr.svm_port = port;
    addr
}

fn new_vsock_fd() -> OwnedFd {
    // SAFETY: plain syscall; the result is checked before it is owned.
    let raw = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    let raw = check(raw, "socket(AF_VSOCK)")
        .expect("precondition: the kernel offers AF_VSOCK (vsock and vsock_loopback loaded)");
    // SAFETY: `raw` is a fresh descriptor this process owns.
    unsafe { OwnedFd::from_raw_fd(raw) }
}

/// A vsock listener bound to `VMADDR_CID_LOCAL` and a kernel-chosen port.
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
    pub fn bind() -> Self {
        let fd = new_vsock_fd();
        let addr = sockaddr_vm(VMADDR_CID_LOCAL, VMADDR_PORT_ANY);
        // SAFETY: `addr` is a valid `sockaddr_vm` of the passed length.
        let ret = unsafe {
            libc::bind(
                fd.as_raw_fd(),
                (&raw const addr).cast(),
                mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
            )
        };
        check(ret, "bind(vsock)").expect("precondition: vsock loopback bind");
        // SAFETY: plain syscall on an owned descriptor.
        let ret = unsafe { libc::listen(fd.as_raw_fd(), 128) };
        check(ret, "listen(vsock)").expect("precondition: vsock listen");
        // SAFETY: zeroed POD out-parameter of the passed length.
        let mut bound: libc::sockaddr_vm = unsafe { mem::zeroed() };
        let mut len = mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t;
        // SAFETY: `bound` and `len` are valid out-parameters.
        let ret =
            unsafe { libc::getsockname(fd.as_raw_fd(), (&raw mut bound).cast(), &raw mut len) };
        check(ret, "getsockname(vsock)").expect("precondition: vsock getsockname");
        Self { fd, port: bound.svm_port }
    }

    pub fn port(&self) -> u32 {
        self.port
    }

    pub fn accept(&self) -> Vsock {
        // SAFETY: null address out-parameters are permitted by accept4.
        let raw = unsafe {
            libc::accept4(
                self.fd.as_raw_fd(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_CLOEXEC,
            )
        };
        let raw = check(raw, "accept4(vsock)").expect("precondition: vsock accept");
        // SAFETY: fresh descriptor this process owns.
        Vsock { fd: unsafe { OwnedFd::from_raw_fd(raw) } }
    }
}

impl Vsock {
    /// A socket that was never connected (`TCP_CLOSE` without `SOCK_DONE`).
    pub fn unconnected() -> Self {
        Self { fd: new_vsock_fd() }
    }

    /// Connects to `(VMADDR_CID_LOCAL, port)`.
    pub fn connect(port: u32) -> Self {
        let fd = new_vsock_fd();
        let addr = sockaddr_vm(VMADDR_CID_LOCAL, port);
        // SAFETY: `addr` is a valid `sockaddr_vm` of the passed length.
        let ret = unsafe {
            libc::connect(
                fd.as_raw_fd(),
                (&raw const addr).cast(),
                mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
            )
        };
        check(ret, "connect(vsock)").expect("precondition: vsock loopback connect");
        Self { fd }
    }

    pub fn shutdown(&self, how: libc::c_int) {
        // SAFETY: plain syscall on an owned descriptor.
        let ret = unsafe { libc::shutdown(self.fd.as_raw_fd(), how) };
        check(ret, "shutdown(vsock)").expect("vsock shutdown");
    }

    pub fn set_nonblocking(&self, on: bool) {
        set_nonblocking(self.fd.as_raw_fd(), on);
    }

    /// `SO_VM_SOCKETS_BUFFER_SIZE` (and the max, so a size above the
    /// default maximum is accepted).
    pub fn set_buffer_size(&self, bytes: u64) {
        for opt in [SO_VM_SOCKETS_BUFFER_MAX_SIZE, SO_VM_SOCKETS_BUFFER_SIZE] {
            // SAFETY: `bytes` is a valid u64 option value of the passed length.
            let ret = unsafe {
                libc::setsockopt(
                    self.fd.as_raw_fd(),
                    libc::AF_VSOCK,
                    opt,
                    (&raw const bytes).cast(),
                    mem::size_of::<u64>() as libc::socklen_t,
                )
            };
            check(ret, "setsockopt(SO_VM_SOCKETS_BUFFER_*)").expect("vsock buffer size");
        }
    }

    pub fn buffer_size(&self) -> u64 {
        let mut value = 0u64;
        let mut len = mem::size_of::<u64>() as libc::socklen_t;
        // SAFETY: valid out-parameters of the passed length.
        let ret = unsafe {
            libc::getsockopt(
                self.fd.as_raw_fd(),
                libc::AF_VSOCK,
                SO_VM_SOCKETS_BUFFER_SIZE,
                (&raw mut value).cast(),
                &raw mut len,
            )
        };
        check(ret, "getsockopt(SO_VM_SOCKETS_BUFFER_SIZE)").expect("vsock buffer size");
        value
    }

    /// `SO_RCVTIMEO`, so a missing end fails the case instead of hanging it.
    pub fn set_read_timeout(&self, timeout: Duration) {
        let tv = libc::timeval {
            tv_sec: timeout.as_secs() as libc::time_t,
            tv_usec: libc::suseconds_t::from(timeout.subsec_micros() as i32),
        };
        // SAFETY: valid option value of the passed length.
        let ret = unsafe {
            libc::setsockopt(
                self.fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                (&raw const tv).cast(),
                mem::size_of::<libc::timeval>() as libc::socklen_t,
            )
        };
        check(ret, "setsockopt(SO_RCVTIMEO)").expect("vsock read timeout");
    }

    /// The descriptor as a raw number (for `/proc/<pid>/fd` checks).
    pub fn raw(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl Read for &Vsock {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: `buf` is a valid writable buffer of `buf.len()` bytes.
        let n = unsafe { libc::recv(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
    }
}

impl Write for &Vsock {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: `buf` is a valid readable buffer of `buf.len()` bytes.
        let n = unsafe {
            libc::send(self.fd.as_raw_fd(), buf.as_ptr().cast(), buf.len(), libc::MSG_NOSIGNAL)
        };
        if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `(accepted, connector)`: the accepted socket is the half a host owner
/// holds (`V_h`); the connector is its far end (the guest's `V_g`).
pub fn vsock_pair() -> (Vsock, Vsock) {
    let listener = VsockListener::bind();
    let far = Vsock::connect(listener.port());
    let half = listener.accept();
    (half, far)
}

// ---------------------------------------------------------------------------
// TCP and UDP loopback
// ---------------------------------------------------------------------------

/// `(half, far)` over IPv4 loopback.
pub fn tcp_pair() -> (TcpStream, TcpStream) {
    tcp_pair_on(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
}

/// `(half, far)` over IPv6 loopback.
pub fn tcp6_pair() -> (TcpStream, TcpStream) {
    tcp_pair_on(SocketAddr::from((Ipv6Addr::LOCALHOST, 0)))
}

fn tcp_pair_on(at: SocketAddr) -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind(at).expect("precondition: loopback TCP bind");
    let far =
        TcpStream::connect(listener.local_addr().expect("local addr")).expect("loopback connect");
    let (half, _) = listener.accept().expect("loopback accept");
    (half, far)
}

/// A TCP listener (a socket in `LISTEN`).
pub fn tcp_listener() -> TcpListener {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("precondition: loopback TCP bind")
}

/// `(half, far)`: two connected UDP sockets, each the other's peer.
pub fn udp_pair() -> (UdpSocket, UdpSocket) {
    let half = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("udp bind");
    let far = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("udp bind");
    half.connect(far.local_addr().expect("addr")).expect("udp connect");
    far.connect(half.local_addr().expect("addr")).expect("udp connect");
    (half, far)
}

/// Closes a TCP socket with `SO_LINGER{1,0}`: its peer receives a reset.
pub fn reset_close(sock: TcpStream) {
    let linger = libc::linger { l_onoff: 1, l_linger: 0 };
    // SAFETY: valid option value of the passed length.
    let ret = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            (&raw const linger).cast(),
            mem::size_of::<libc::linger>() as libc::socklen_t,
        )
    };
    check(ret, "setsockopt(SO_LINGER)").expect("linger");
    drop(sock);
}

/// Sets a TCP socket's `SO_RCVBUF` and `SO_SNDBUF` (the tight-buffer shape).
pub fn set_tcp_buffers(sock: &TcpStream, bytes: libc::c_int) {
    for opt in [libc::SO_RCVBUF, libc::SO_SNDBUF] {
        // SAFETY: valid option value of the passed length.
        let ret = unsafe {
            libc::setsockopt(
                sock.as_raw_fd(),
                libc::SOL_SOCKET,
                opt,
                (&raw const bytes).cast(),
                mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        check(ret, "setsockopt(SO_RCVBUF/SO_SNDBUF)").expect("tcp buffers");
    }
}

/// Installs the `tls` upper-layer protocol on an established TCP socket.
pub fn install_tls_ulp(sock: &TcpStream) {
    let name = b"tls";
    // SAFETY: valid option value of the passed length.
    let ret = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::IPPROTO_TCP,
            TCP_ULP,
            name.as_ptr().cast(),
            name.len() as libc::socklen_t,
        )
    };
    check(ret, "setsockopt(TCP_ULP, tls)").expect("precondition: the tls module is available");
}

/// `tcpi_state` of a TCP socket (`TCP_ESTABLISHED` 1 … `TCP_CLOSING` 11).
pub fn tcp_state(sock: &TcpStream) -> u8 {
    // SAFETY: zeroed POD out-parameter.
    let mut info: libc::tcp_info = unsafe { mem::zeroed() };
    let mut len = mem::size_of::<libc::tcp_info>() as libc::socklen_t;
    // SAFETY: valid out-parameters of the passed length.
    let ret = unsafe {
        libc::getsockopt(
            sock.as_raw_fd(),
            libc::IPPROTO_TCP,
            libc::TCP_INFO,
            (&raw mut info).cast(),
            &raw mut len,
        )
    };
    check(ret, "getsockopt(TCP_INFO)").expect("tcp info");
    info.tcpi_state
}

/// `TCP_CLOSE`, `TCP_CLOSE_WAIT` (`include/net/tcp_states.h`).
pub const TCP_CLOSE: u8 = 7;
pub const TCP_CLOSE_WAIT: u8 = 8;

/// Waits until `sock` reaches `state`.
pub fn wait_tcp_state(sock: &TcpStream, state: u8) {
    let start = Instant::now();
    while tcp_state(sock) != state {
        assert!(
            start.elapsed() < END_WAIT,
            "TCP socket did not reach state {state} (is {})",
            tcp_state(sock)
        );
        thread::sleep(Duration::from_millis(2));
    }
}

/// Bytes queued in a socket's receive queue (`FIONREAD`).
pub fn queued_bytes(fd: BorrowedFd<'_>) -> usize {
    let mut n: libc::c_int = 0;
    // SAFETY: `FIONREAD` writes one int.
    let ret = unsafe { libc::ioctl(fd.as_raw_fd(), libc::FIONREAD, &raw mut n) };
    check(ret, "ioctl(FIONREAD)").expect("fionread");
    n as usize
}

pub fn set_nonblocking(fd: RawFd, on: bool) {
    // SAFETY: plain fcntl on a live descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    let flags = check(flags, "fcntl(F_GETFL)").expect("fcntl");
    let flags = if on { flags | libc::O_NONBLOCK } else { flags & !libc::O_NONBLOCK };
    // SAFETY: plain fcntl on a live descriptor.
    let ret = unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
    check(ret, "fcntl(F_SETFL)").expect("fcntl");
}

/// `(read, write)` ends of a pipe (a descriptor that is not a socket).
pub fn pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a valid two-int out-parameter.
    let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    check(ret, "pipe2").expect("pipe");
    // SAFETY: fresh descriptors this process owns.
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

/// Clears `FD_CLOEXEC` on a descriptor (the probe's `ContextInheritable`
/// injection).
pub fn clear_cloexec(fd: BorrowedFd<'_>) {
    // SAFETY: plain fcntl on a live descriptor.
    let ret = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, 0) };
    check(ret, "fcntl(F_SETFD, 0)").expect("fcntl");
}

// ---------------------------------------------------------------------------
// Payloads and far-end observation
// ---------------------------------------------------------------------------

/// A deterministic, position-dependent byte stream (splitmix64), so a lost,
/// duplicated or reordered chunk changes the bytes at its position.
pub fn pattern(seed: u64, len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut state = seed;
    while out.len() < len {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        let take = (len - out.len()).min(8);
        out.extend_from_slice(&z.to_le_bytes()[..take]);
    }
    out
}

/// How a far end's read ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FarEnd {
    /// `read` returned 0.
    Eof,
    /// `read` failed with this kind.
    Error(ErrorKind),
}

/// Reads until end-of-stream or an error; returns the bytes and the end.
pub fn read_until_end(mut r: impl Read) -> (Vec<u8>, FarEnd) {
    let mut got = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match r.read(&mut buf) {
            Ok(0) => return (got, FarEnd::Eof),
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return (got, FarEnd::Error(e.kind())),
        }
    }
}

/// Reads exactly `n` bytes or panics naming how the stream ended.
pub fn read_exact_n(mut r: impl Read, n: usize) -> Vec<u8> {
    let mut got = vec![0u8; n];
    r.read_exact(&mut got).unwrap_or_else(|e| panic!("expected {n} bytes, stream ended: {e}"));
    got
}

/// Sets a read timeout on a TCP far end so a missing end fails the case
/// instead of hanging it.
pub fn bounded(sock: &TcpStream) -> &TcpStream {
    sock.set_read_timeout(Some(END_WAIT)).expect("read timeout");
    sock
}

/// Asserts a TCP far end observes a reset, never end-of-stream.
pub fn assert_far_tcp_reset(sock: &TcpStream) {
    let (_, end) = read_until_end(bounded(sock));
    assert_eq!(
        end,
        FarEnd::Error(ErrorKind::ConnectionReset),
        "the TCP far end must read a reset, never EOF (R2-END)"
    );
}

/// Asserts a TCP far end receives `expected` then end-of-stream.
pub fn assert_far_tcp_clean(sock: &TcpStream, expected: &[u8]) {
    let (got, end) = read_until_end(bounded(sock));
    assert_eq!(end, FarEnd::Eof, "the TCP far end must read a clean end-of-stream");
    assert!(got == expected, "byte-exact delivery: got {} of {} bytes", got.len(), expected.len());
}

// ---------------------------------------------------------------------------
// End records
// ---------------------------------------------------------------------------

/// Waits on the context's readiness descriptor until an end record is
/// queued, then takes it.
pub fn wait_end(fwd: &KernelFlowForwarder) -> ForwardEnd {
    let start = Instant::now();
    loop {
        if let Some(end) = fwd.take_end().expect("take_end") {
            return end;
        }
        let left = END_WAIT.checked_sub(start.elapsed()).unwrap_or_else(|| {
            panic!("no end record within {END_WAIT:?}; inventory {:?}", fwd.inventory())
        });
        let mut pfd =
            libc::pollfd { fd: fwd.readiness_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd.
        let ret = unsafe { libc::poll(&raw mut pfd, 1, left.as_millis() as libc::c_int) };
        check(ret, "poll(readiness_fd)").expect("poll");
    }
}

/// Waits for the end record of `id`, asserting no other id's record comes
/// first.
pub fn wait_end_of(fwd: &KernelFlowForwarder, id: ForwardId) -> ForwardEnd {
    let end = wait_end(fwd);
    assert_eq!(end.id, id, "the next end record must be id {id:?}'s");
    end
}

/// Asserts no end record is queued during [`QUIET`].
pub fn assert_no_end(fwd: &KernelFlowForwarder) {
    let start = Instant::now();
    while start.elapsed() < QUIET {
        assert_eq!(fwd.take_end().expect("take_end"), None, "no end record may be queued");
        thread::sleep(Duration::from_millis(10));
    }
}

/// The inventory's live counts and queue depth.
pub fn live(fwd: &KernelFlowForwarder) -> (u32, u32, u32) {
    let ForwarderInventory { live_pairs, live_watches, queued_ends, .. } =
        fwd.inventory().expect("inventory");
    (live_pairs, live_watches, queued_ends)
}

/// Attaches a stream pair `(vsock, tcp)`: half A the vsock, half B the TCP
/// socket (the host's TcpConnect shape).
pub fn attach_stream(fwd: &KernelFlowForwarder, v: &Vsock, t: &TcpStream) -> ForwardId {
    fwd.attach(v as &dyn AsFd, t as &dyn AsFd, PairMode::Stream).expect("attach a stream pair")
}

// ---------------------------------------------------------------------------
// A stream flow
// ---------------------------------------------------------------------------

/// One attached stream pair with its four sockets, and a second context
/// watching the vsock far end (the guest's view).
pub struct StreamFlow {
    pub fwd: KernelFlowForwarder,
    pub peer: KernelFlowForwarder,
    pub id: ForwardId,
    pub far_watch: ForwardId,
    pub v: Vsock,
    pub v_far: Vsock,
    pub t: TcpStream,
    pub t_far: TcpStream,
}

impl StreamFlow {
    pub fn new() -> Self {
        Self::with(|_, _, _, _| {})
    }

    /// `tune` runs on `(v, v_far, t, t_far)` before attach.
    pub fn with(tune: impl FnOnce(&Vsock, &Vsock, &TcpStream, &TcpStream)) -> Self {
        let fwd = open_context();
        let peer = open_context();
        let (v, v_far) = vsock_pair();
        let (t, t_far) = tcp_pair();
        tune(&v, &v_far, &t, &t_far);
        let far_watch = peer.watch(&v_far as &dyn AsFd).expect("watch the vsock far end");
        let id = attach_stream(&fwd, &v, &t);
        v_far.set_read_timeout(END_WAIT);
        t_far.set_read_timeout(Some(END_WAIT)).expect("timeout");
        Self { fwd, peer, id, far_watch, v, v_far, t, t_far }
    }

    pub fn end(&self) -> ForwardEnd {
        wait_end_of(&self.fwd, self.id)
    }

    pub fn far_end(&self) -> ForwardOutcome {
        wait_end_of(&self.peer, self.far_watch).outcome
    }

    /// The owner's step after a pair's end record: its descriptors close.
    pub fn release(self) {
        let Self { fwd, v, t, .. } = self;
        assert_eq!(live(&fwd), (0, 0, 0), "nothing live or queued after the pair's record");
        drop((v, t));
    }
}

/// Closes a vsock far end that `peer` watches, as the guest side's own
/// close: the watch is aborted first, then the socket is closed. A live
/// watch holds its own file reference (`OVD_FWD_WATCH` takes one,
/// § *Control interface*), so a bare `close()` would leave the socket open
/// and send nothing (P-70 § 4.4: `live_watches=1` after the close). A watch
/// that already ended (its record queued or read) is left as it is.
pub fn close_watched(peer: &KernelFlowForwarder, watch: ForwardId, sock: Vsock) {
    match peer.abort(watch).expect("abort the far end's watch") {
        AbortResult::Aborted => {
            let end = wait_end_of(peer, watch);
            assert_eq!(end.outcome, ForwardOutcome::Aborted { cause: AbortCause::Detached });
        }
        AbortResult::AlreadyEnded => {}
    }
    drop(sock);
}

/// Writes `data` to the vsock far end and half-closes it (`0x2`).
pub fn send_vsock(far: &Vsock, data: &[u8]) {
    (&mut &*far).write_all(data).expect("write to the vsock far end");
    far.shutdown(libc::SHUT_WR);
}

/// Writes `data` to the TCP far end and half-closes it (FIN).
pub fn send_tcp(far: &TcpStream, data: &[u8]) {
    (&mut &*far).write_all(data).expect("write to the TCP far end");
    far.shutdown(std::net::Shutdown::Write).expect("FIN");
}

// ---------------------------------------------------------------------------
// Node facts
// ---------------------------------------------------------------------------

/// `SUnreclaim` of `/proc/meminfo`, in KiB.
pub fn unreclaimable_slab_kib() -> u64 {
    let meminfo = fs::read_to_string("/proc/meminfo").expect("read /proc/meminfo");
    meminfo
        .lines()
        .find_map(|l| l.strip_prefix("SUnreclaim:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .expect("SUnreclaim in /proc/meminfo")
}

/// The third field of a `/proc/sys/net/ipv4/tcp_{r,w}mem` triple (the
/// autotune maximum).
pub fn tcp_mem_max(which: &str) -> usize {
    let raw = fs::read_to_string(format!("/proc/sys/net/ipv4/{which}")).expect("read tcp_*mem");
    raw.split_whitespace().nth(2).and_then(|v| v.parse().ok()).expect("tcp_*mem triple")
}

/// Whether a process holds an open descriptor on `/dev/ovd_fwd`.
pub fn holds_forwarder_context(pid: u32) -> bool {
    let dir = PathBuf::from(format!("/proc/{pid}/fd"));
    fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .filter_map(|e| fs::read_link(e.path()).ok())
        .any(|target| target == Path::new("/dev/ovd_fwd"))
}

// ---------------------------------------------------------------------------
// Thread-scoped namespaces (the probe gold test's injections)
// ---------------------------------------------------------------------------

/// Runs `body` on a fresh thread that has unshared `flags`
/// (`CLONE_NEWNS` and/or `CLONE_NEWNET`). Both are per-task: `unshare`
/// accepts them on one thread of a multi-threaded process
/// (`kernel/fork.c:3046-3074 @ v7.0`), so the injection never reaches the
/// rest of the test process or the node. A new mount namespace is made
/// private recursively first, so nothing it mounts propagates back.
pub fn in_namespaces<T: Send + 'static>(
    flags: libc::c_int,
    body: impl FnOnce() -> T + Send + 'static,
) -> T {
    thread::spawn(move || {
        // SAFETY: plain syscall; affects only this thread.
        let ret = unsafe { libc::unshare(flags) };
        check(ret, "unshare").expect("precondition: unshare as root");
        if flags & libc::CLONE_NEWNS != 0 {
            mount_raw(None, "/", None, libc::MS_REC | libc::MS_PRIVATE)
                .expect("make the new mount namespace private");
        }
        body()
    })
    .join()
    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// `mount(2)`.
pub fn mount_raw(
    source: Option<&str>,
    target: &str,
    fstype: Option<&str>,
    flags: libc::c_ulong,
) -> io::Result<()> {
    use std::ffi::CString;
    let src = source.map(|s| CString::new(s).expect("no NUL"));
    let tgt = CString::new(target).expect("no NUL");
    let fst = fstype.map(|s| CString::new(s).expect("no NUL"));
    // SAFETY: every pointer is a valid C string or null.
    let ret = unsafe {
        libc::mount(
            src.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            tgt.as_ptr(),
            fst.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            flags,
            std::ptr::null(),
        )
    };
    check(ret, "mount").map(drop)
}

/// Runs a command (in the calling thread's namespaces) and panics on a
/// non-zero exit, printing its output.
pub fn run(cmd: &str, args: &[&str]) {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("precondition: `{cmd}` runs: {e}"));
    assert!(
        out.status.success(),
        "`{cmd} {}` failed: {}\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Unloads `vsock_loopback` for its lifetime and reloads it on drop (also
/// on a panic), so a failed case leaves the node as it found it.
#[derive(Debug)]
pub struct LoopbackTransportRemoved(());

impl LoopbackTransportRemoved {
    pub fn remove() -> Self {
        assert!(
            Path::new("/sys/module/vsock_loopback").exists(),
            "precondition: vsock_loopback is loaded as a module"
        );
        if Path::new(overdrive_testing::guest_kit::LOOPBACK_MODULE_GUEST_PATH).is_file() {
            // SAFETY: a valid NUL-terminated module name and O_NONBLOCK; the
            // disposable guest owns this module instance exclusively.
            let rc = unsafe {
                libc::syscall(libc::SYS_delete_module, c"vsock_loopback".as_ptr(), libc::O_NONBLOCK)
            };
            assert_eq!(rc, 0, "delete_module(vsock_loopback): {}", io::Error::last_os_error());
        } else {
            run("modprobe", &["-r", "vsock_loopback"]);
        }
        Self(())
    }
}

impl Drop for LoopbackTransportRemoved {
    fn drop(&mut self) {
        let module = Path::new(overdrive_testing::guest_kit::LOOPBACK_MODULE_GUEST_PATH);
        if module.is_file() {
            let result = fs::File::open(module).and_then(|file| {
                // SAFETY: `file` is an open module ELF, params is an empty C
                // string and flags are zero.
                let rc = unsafe {
                    libc::syscall(libc::SYS_finit_module, file.as_raw_fd(), c"".as_ptr(), 0_i32)
                };
                if rc == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
            });
            if let Err(error) = result {
                eprintln!("cleanup: finit_module(vsock_loopback) failed: {error}");
            }
        } else {
            let status = Command::new("modprobe").arg("vsock_loopback").status();
            if !matches!(status, Ok(s) if s.success()) {
                eprintln!(
                    "cleanup: `modprobe vsock_loopback` failed ({status:?}); later cases will fail"
                );
            }
        }
    }
}
