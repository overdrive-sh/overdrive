//! `overdrive-fwd` — the kernel adapter of the core port
//! [`FlowForwarder`]: the control context of the in-kernel event-driven
//! flow forwarder `ovd_fwd` (EVMOD-PORT, EVMOD-CTL, EVMOD-REC;
//! ADR-0197, ADR-0199, ADR-0202).
//!
//! # Kernel requirement
//!
//! The adapter drives the module `ovd_fwd` (`kmod/ovd_fwd/`, its control
//! ABI SSOT `include/uapi/linux/ovd_fwd.h`) built for **Linux 7.0**, the
//! pinned appliance kernel (ADR-0068), or for one of the stock 7.0 kernels
//! of the Tier-3 hosts and guest fixture (EVMOD-BUILD, ADR-0203). Every
//! kernel fact this adapter rests on is cited against `vendors/linux` at
//! `v7.0` in `docs/feature/netns-density-295/feature-delta.md`, § *[REF]
//! vsock data path — the in-kernel event-driven forwarder (EVMOD)*. The
//! module unloads only when nothing references it (EVMOD-LIFE as amended by
//! MODULE-REFCOUNT, ADR-0198): every open context and every hooked socket
//! holds a module reference; the appliance kernel has no module unloading.
//!
//! The module is loaded by the host image and by `overdrive-init`, never by
//! `overdrive serve` (EVMOD-LOAD, ADR-0205); this crate never loads it.
//!
//! # Shape
//!
//! - [`KernelFlowForwarder::open`] opens one context on `/dev/ovd_fwd` with
//!   `O_RDWR | O_CLOEXEC | O_NONBLOCK`. The type is not `Clone`; dropping it
//!   closes the context, whose release aborts every pair and watch the
//!   context created and queues no record (EVMOD-CTL).
//! - It implements `FlowForwarder<dyn AsFd>` and lends the context's
//!   descriptor for the owners' event loops through
//!   [`KernelFlowForwarder::readiness_fd`] (`EPOLLIN` while an end record is
//!   queued, EVMOD-REC).
//! - The crate-private `uapi` module holds the `#[repr(C)]` mirrors of the
//!   control ABI with `const` layout assertions.
//!
//! The crate depends on `overdrive-core`, `nix` and `thiserror` only (no
//! async runtime), so both `overdrive-control-plane` and the static
//! `overdrive-init` link it.
//!
//! # Tests
//!
//! The Tier-3 suite (`tests/integration/`, `integration-tests` feature)
//! runs against the real module through this adapter on the host role in
//! Lima (`cargo xtask lima run --`), in the `host-kernel-shared` nextest
//! group.
//!
//! The adapter contains no async runtime and exposes only the core port.

mod uapi;

use std::fs::OpenOptions;
use std::io;
use std::net::{Ipv4Addr, TcpListener, TcpStream, UdpSocket};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

use nix::errno::Errno;
use nix::libc;
use overdrive_core::guest_transport::forwarder::{
    AbortCause, AbortResult, DirectionState, FlowForwarder, ForwardEnd, ForwardId, ForwardOutcome,
    ForwarderError, ForwarderInventory, ForwarderOp, ForwarderProbeObservation,
    ForwarderProbeStage, PairMode, PairSide, PairState, SourceEnd,
};
use overdrive_core::guest_transport::wire::{DatagramFrame, KernelErrno, MAX_GUEST_DATAGRAM_FRAME};

use crate::uapi::{
    END_RECORD_LEN, OvdFwdAttach, OvdFwdInventory, OvdFwdPairState, OvdFwdVersion, OvdFwdWatch,
    abort_all_ioctl, abort_ioctl, attach_ioctl, decode_end_record, inventory_ioctl,
    pair_state_ioctl, version_ioctl, version_verdict, watch_ioctl,
};

/// One forwarder context: an open `/dev/ovd_fwd` (EVMOD-CTL, ADR-0199).
///
/// Not `Clone`, never duplicated, sent or inherited (`O_CLOEXEC`). Dropping
/// it closes the descriptor; the release aborts every live pair (TCP halves
/// reset, vsock halves `SHUT_RDWR`) and every live watch (its `V_h` shut
/// `SHUT_RDWR` — the only operation that shuts a watched socket,
/// WATCH-ABORT, ADR-0211) and queues no record.
#[derive(Debug)]
pub struct KernelFlowForwarder {
    context: OwnedFd,
}

impl KernelFlowForwarder {
    /// Opens a new context on `/dev/ovd_fwd` with
    /// `O_RDWR | O_CLOEXEC | O_NONBLOCK`.
    ///
    /// # Errors
    ///
    /// [`ForwarderError::DeviceAbsent`] when the device cannot be opened.
    /// The ABI version is checked by [`FlowForwarder::probe`] (`Version`
    /// stage), not here.
    pub fn open() -> Result<Self, ForwarderError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(uapi::FORWARDER_DEVICE_PATH)
            .map_err(|source| ForwarderError::DeviceAbsent { source })?;
        Ok(Self { context: file.into() })
    }

    /// The context's descriptor, for the owner's event loop: readable
    /// (`EPOLLIN`) while an end record is queued (EVMOD-REC).
    #[must_use]
    pub fn readiness_fd(&self) -> BorrowedFd<'_> {
        self.context.as_fd()
    }
}

fn io_failure(op: ForwarderOp, error: Errno) -> ForwarderError {
    ForwarderError::Io { op, source: io::Error::from_raw_os_error(error as i32) }
}

fn invalid_uapi(op: ForwarderOp) -> ForwarderError {
    ForwarderError::Io { op, source: io::Error::from_raw_os_error(libc::EPROTO) }
}

fn pair_side(raw: u8, op: ForwarderOp) -> Result<PairSide, ForwarderError> {
    match raw {
        0 => Ok(PairSide::A),
        1 => Ok(PairSide::B),
        _ => Err(invalid_uapi(op)),
    }
}

fn refusal(op: ForwarderOp, error: Errno, raw_side: u8, end_errno: u16) -> ForwarderError {
    let side = match pair_side(raw_side, op) {
        Ok(side) => side,
        Err(error) => return error,
    };
    match error {
        Errno::ESOCKTNOSUPPORT => ForwarderError::UnsupportedSocket { side },
        Errno::EPROTONOSUPPORT => ForwarderError::UpperLayerProtocol { side },
        Errno::ENOTCONN => ForwarderError::NotConnected { side },
        Errno::ECONNABORTED => KernelErrno::new(end_errno)
            .map_or_else(|_| invalid_uapi(op), |errno| ForwarderError::HalfEnded { side, errno }),
        Errno::EBUSY => ForwarderError::AlreadyHooked { side },
        Errno::EMSGSIZE => ForwarderError::BufferTooSmall { side },
        other => io_failure(op, other),
    }
}

impl FlowForwarder<dyn AsFd> for KernelFlowForwarder {
    /// The forwarder probe (§ *Earned Trust — the forwarder probe*): the
    /// `ForwarderProbeStage` order `DeviceOpen`, `Version`,
    /// `StreamRoundTrip`, `HalfClose`, `PeerAbortClassified`,
    /// `Backpressure`, `DatagramRoundTrip`, `WatchClassified`,
    /// `ContextRelease`, `Cleanup`, over the vsock loopback transport
    /// (CID 1). A failure names its stage.
    fn probe(&self) -> Result<(), ForwarderError> {
        probe_forwarder(self)
    }

    fn attach(
        &self,
        a: &dyn AsFd,
        b: &dyn AsFd,
        mode: PairMode,
    ) -> Result<ForwardId, ForwarderError> {
        let mode = match mode {
            PairMode::Stream => 0,
            PairMode::Datagram { udp: PairSide::A } => 1,
            PairMode::Datagram { udp: PairSide::B } => 2,
        };
        let mut request = OvdFwdAttach {
            fd_a: a.as_fd().as_raw_fd(),
            fd_b: b.as_fd().as_raw_fd(),
            mode,
            flags: 0,
            ..OvdFwdAttach::default()
        };
        match unsafe { attach_ioctl(self.context.as_raw_fd(), &raw mut request) } {
            Ok(_) => ForwardId::new(request.id).map_err(|_| invalid_uapi(ForwarderOp::Attach)),
            Err(error) => {
                Err(refusal(ForwarderOp::Attach, error, request.bad_side, request.end_errno))
            }
        }
    }

    fn watch(&self, vsock: &dyn AsFd) -> Result<ForwardId, ForwarderError> {
        let mut request =
            OvdFwdWatch { fd: vsock.as_fd().as_raw_fd(), flags: 0, ..OvdFwdWatch::default() };
        match unsafe { watch_ioctl(self.context.as_raw_fd(), &raw mut request) } {
            Ok(_) => ForwardId::new(request.id).map_err(|_| invalid_uapi(ForwarderOp::Watch)),
            Err(error) => {
                if request.bad_side != 0 {
                    return Err(invalid_uapi(ForwarderOp::Watch));
                }
                match refusal(ForwarderOp::Watch, error, 0, request.end_errno) {
                    ForwarderError::UnsupportedSocket { .. } => {
                        Err(ForwarderError::UnsupportedSocket { side: PairSide::A })
                    }
                    ForwarderError::UpperLayerProtocol { .. } => {
                        Err(ForwarderError::UpperLayerProtocol { side: PairSide::A })
                    }
                    ForwarderError::NotConnected { .. } => {
                        Err(ForwarderError::NotConnected { side: PairSide::A })
                    }
                    ForwarderError::HalfEnded { errno, .. } => {
                        Err(ForwarderError::HalfEnded { side: PairSide::A, errno })
                    }
                    ForwarderError::AlreadyHooked { .. } => {
                        Err(ForwarderError::AlreadyHooked { side: PairSide::A })
                    }
                    other => Err(other),
                }
            }
        }
    }

    fn abort(&self, id: ForwardId) -> Result<AbortResult, ForwarderError> {
        let raw_id = id.get();
        match unsafe { abort_ioctl(self.context.as_raw_fd(), &raw const raw_id) } {
            Ok(_) => Ok(AbortResult::Aborted),
            Err(Errno::ENOENT) => Ok(AbortResult::AlreadyEnded),
            Err(error) => Err(io_failure(ForwarderOp::Abort, error)),
        }
    }

    fn abort_all(&self) -> Result<u32, ForwarderError> {
        match unsafe { abort_all_ioctl(self.context.as_raw_fd()) } {
            Ok(count) => u32::try_from(count).map_err(|_| invalid_uapi(ForwarderOp::AbortAll)),
            Err(error) => Err(io_failure(ForwarderOp::AbortAll, error)),
        }
    }

    fn take_end(&self) -> Result<Option<ForwardEnd>, ForwarderError> {
        let mut bytes = [0_u8; END_RECORD_LEN];
        // SAFETY: `bytes` is writable for exactly its declared length and
        // the descriptor is the owned forwarder context.
        let result =
            unsafe { libc::read(self.context.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len()) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(ForwarderError::Io { op: ForwarderOp::Read, source: error });
        }
        if result == 0 {
            return Ok(None);
        }
        if usize::try_from(result).unwrap_or(0) != END_RECORD_LEN {
            return Err(ForwarderError::Io {
                op: ForwarderOp::Read,
                source: io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "partial forwarder end record",
                ),
            });
        }
        decode_end_record(&bytes).map(Some)
    }

    fn pair_state(&self, id: ForwardId) -> Result<Option<PairState>, ForwarderError> {
        let mut state = OvdFwdPairState { id: id.get(), ..OvdFwdPairState::default() };
        match unsafe { pair_state_ioctl(self.context.as_raw_fd(), &raw mut state) } {
            Err(Errno::ENOENT) => Ok(None),
            Err(error) => Err(io_failure(ForwarderOp::PairState, error)),
            Ok(_) => {
                if state.reserved != [0; 4] || state.target_shut.iter().any(|flag| *flag > 1) {
                    return Err(invalid_uapi(ForwarderOp::PairState));
                }
                let source_end = state.source_end.map(|value| match value {
                    0 => Ok(None),
                    1 => Ok(Some(SourceEnd::Clean)),
                    2 => Ok(Some(SourceEnd::Abort)),
                    _ => Err(invalid_uapi(ForwarderOp::PairState)),
                });
                let [a_to_b, b_to_a] = source_end;
                Ok(Some(PairState {
                    a_to_b: DirectionState {
                        source_end: a_to_b?,
                        target_shut: state.target_shut[0] == 1,
                        moved: state.moved[0],
                    },
                    b_to_a: DirectionState {
                        source_end: b_to_a?,
                        target_shut: state.target_shut[1] == 1,
                        moved: state.moved[1],
                    },
                }))
            }
        }
    }

    fn inventory(&self) -> Result<ForwarderInventory, ForwarderError> {
        let mut inventory = OvdFwdInventory::default();
        if let Err(error) = unsafe { inventory_ioctl(self.context.as_raw_fd(), &raw mut inventory) }
        {
            return Err(io_failure(ForwarderOp::Inventory, error));
        }
        if inventory.reserved != 0 {
            return Err(invalid_uapi(ForwarderOp::Inventory));
        }
        Ok(ForwarderInventory {
            live_pairs: inventory.live_pairs,
            live_watches: inventory.live_watches,
            queued_ends: inventory.queued_ends,
            attached: inventory.attached,
            completed: inventory.completed,
            aborted: inventory.aborted_peer
                + inventory.aborted_socket
                + inventory.aborted_frame
                + inventory.aborted_detached,
            unclassified_eof: inventory.unclassified_eof,
            dgram_oversize: inventory.dgram_oversize,
        })
    }
}

fn probe_io(stage: ForwarderProbeStage, op: ForwarderOp, error: io::Error) -> ForwarderError {
    ForwarderError::Probe { stage, source: Box::new(ForwarderError::Io { op, source: error }) }
}

const fn probe_require(
    condition: bool,
    stage: ForwarderProbeStage,
    observed: ForwarderProbeObservation,
) -> Result<(), ForwarderError> {
    if condition { Ok(()) } else { Err(probe_mismatch(stage, observed)) }
}

struct ProbeCleanup<'a> {
    forwarder: &'a KernelFlowForwarder,
    ids: Vec<ForwardId>,
}

impl<'a> ProbeCleanup<'a> {
    const fn new(forwarder: &'a KernelFlowForwarder) -> Self {
        Self { forwarder, ids: Vec::new() }
    }

    fn track(&mut self, id: ForwardId) {
        self.ids.push(id);
    }

    fn clear(&mut self, id: ForwardId) {
        self.ids.retain(|tracked| *tracked != id);
    }
}

impl Drop for ProbeCleanup<'_> {
    fn drop(&mut self) {
        for id in self.ids.drain(..) {
            let _ = self.forwarder.abort(id);
        }
        while matches!(self.forwarder.take_end(), Ok(Some(_))) {}
    }
}

fn probe_wait_end(
    forwarder: &KernelFlowForwarder,
    id: ForwardId,
    stage: ForwarderProbeStage,
) -> Result<ForwardEnd, ForwarderError> {
    loop {
        if let Some(end) = forwarder.take_end()? {
            if end.id == id {
                return Ok(end);
            }
            return Err(probe_mismatch(stage, ForwarderProbeObservation::WrongOutcome));
        }
        probe_wait(forwarder.readiness_fd().as_raw_fd(), libc::POLLIN)
            .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
    }
}

fn probe_abort_and_clear(
    cleanup: &mut ProbeCleanup<'_>,
    id: ForwardId,
    stage: ForwarderProbeStage,
) -> Result<ForwardEnd, ForwarderError> {
    cleanup.forwarder.abort(id)?;
    let end = probe_wait_end(cleanup.forwarder, id, stage)?;
    cleanup.clear(id);
    Ok(end)
}

fn probe_check_version(forwarder: &KernelFlowForwarder) -> Result<(), ForwarderError> {
    let mut version = OvdFwdVersion::default();
    unsafe { version_ioctl(forwarder.context.as_raw_fd(), &raw mut version) }
        .map_err(|error| io_failure(ForwarderOp::Version, error))?;
    version_verdict(version)?;
    // SAFETY: fcntl reads flags from the owned context descriptor.
    let flags = unsafe { libc::fcntl(forwarder.context.as_raw_fd(), libc::F_GETFD) };
    if flags < 0 {
        return Err(ForwarderError::Io {
            op: ForwarderOp::Version,
            source: io::Error::last_os_error(),
        });
    }
    if flags & libc::FD_CLOEXEC == 0 {
        return Err(ForwarderError::ProbeMismatch {
            stage: ForwarderProbeStage::Version,
            observed: ForwarderProbeObservation::ContextInheritable,
        });
    }
    Ok(())
}

fn probe_set_tcp_rcvbuf(stream: &TcpStream, bytes: libc::c_int) -> io::Result<()> {
    let option_len =
        libc::socklen_t::try_from(std::mem::size_of::<libc::c_int>()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "socket option length overflow")
        })?;
    // SAFETY: bytes is a valid integer socket option value.
    let result = unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&raw const bytes).cast(),
            option_len,
        )
    };
    check_socket_result(result, "setsockopt(SO_RCVBUF)").map(|_| ())
}

fn probe_nonblocking_send_until_stalled(fd: RawFd) -> io::Result<bool> {
    let chunk = vec![0x5a_u8; 64 * 1024].into_boxed_slice();
    let mut total = 0_u64;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        // SAFETY: chunk is a valid readable buffer and the call is non-blocking.
        let result = unsafe {
            libc::send(
                fd,
                chunk.as_ptr().cast(),
                chunk.len(),
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            )
        };
        if result >= 0 {
            total = total.saturating_add(u64::try_from(result).unwrap_or(0));
            if total >= 32 * 1024 * 1024 {
                return Ok(false);
            }
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if error.kind() != io::ErrorKind::WouldBlock {
            return Err(error);
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let timeout = libc::c_int::try_from(left.min(Duration::from_millis(250)).as_millis())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "poll timeout overflow"))?;
        let mut descriptor = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
        // SAFETY: descriptor is initialized for one poll entry.
        let ready = unsafe { libc::poll(&raw mut descriptor, 1, timeout) };
        if ready < 0 {
            let poll_error = io::Error::last_os_error();
            if poll_error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(poll_error);
        }
        if ready == 0 {
            return Ok(true);
        }
        if std::time::Instant::now() >= deadline {
            return Ok(true);
        }
    }
}

fn probe_make_frame(payload: &[u8]) -> Vec<u8> {
    DatagramFrame::new(payload)
        .unwrap_or_else(|_| unreachable!("probe payload is within the pinned datagram bound"))
        .encode()
}

const PROBE_TIMEOUT_MS: libc::c_int = 5_000;
const PROBE_CID_LOCAL: u32 = 1;
const PROBE_PORT_ANY: u32 = u32::MAX;

fn probe_wrap<T>(
    stage: ForwarderProbeStage,
    result: Result<T, ForwarderError>,
) -> Result<T, ForwarderError> {
    result.map_err(|error| match error {
        ForwarderError::Probe { .. } | ForwarderError::ProbeMismatch { .. } => error,
        other => ForwarderError::Probe { stage, source: Box::new(other) },
    })
}

const fn probe_mismatch(
    stage: ForwarderProbeStage,
    observed: ForwarderProbeObservation,
) -> ForwarderError {
    ForwarderError::ProbeMismatch { stage, observed }
}

fn check_socket_result(result: libc::c_int, operation: &str) -> io::Result<libc::c_int> {
    if result < 0 {
        let error = io::Error::last_os_error();
        Err(io::Error::new(error.kind(), format!("{operation}: {error}")))
    } else {
        Ok(result)
    }
}

fn probe_vsock_address(cid: u32, port: u32) -> io::Result<libc::sockaddr_vm> {
    // SAFETY: zero is a valid starting representation for sockaddr_vm.
    let mut address: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
    address.svm_family = libc::sa_family_t::try_from(libc::AF_VSOCK).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "AF_VSOCK does not fit sa_family_t")
    })?;
    address.svm_cid = cid;
    address.svm_port = port;
    Ok(address)
}

fn new_vsock_fd() -> io::Result<OwnedFd> {
    // SAFETY: a successful socket call returns a fresh descriptor owned below.
    let raw = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    let raw = check_socket_result(raw, "socket(AF_VSOCK)")?;
    // SAFETY: `raw` is a fresh descriptor returned by socket and is owned exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn set_probe_timeout(fd: RawFd) -> io::Result<()> {
    let timeout = libc::timeval { tv_sec: 5, tv_usec: 0 };
    let option_len =
        libc::socklen_t::try_from(std::mem::size_of::<libc::timeval>()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "socket option length overflow")
        })?;
    // SAFETY: timeout is a valid timeval for both socket timeout options.
    for option in [libc::SO_RCVTIMEO, libc::SO_SNDTIMEO] {
        let result = unsafe {
            libc::setsockopt(fd, libc::SOL_SOCKET, option, (&raw const timeout).cast(), option_len)
        };
        check_socket_result(result, "setsockopt(socket timeout)")?;
    }
    Ok(())
}

fn probe_vsock_pair() -> io::Result<(OwnedFd, OwnedFd)> {
    let listener = new_vsock_fd()?;
    let address = probe_vsock_address(PROBE_CID_LOCAL, PROBE_PORT_ANY)?;
    let address_len = libc::socklen_t::try_from(std::mem::size_of::<libc::sockaddr_vm>())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "sockaddr length overflow"))?;
    // SAFETY: address is a valid sockaddr_vm and listener is an owned socket.
    let result =
        unsafe { libc::bind(listener.as_raw_fd(), (&raw const address).cast(), address_len) };
    check_socket_result(result, "bind(vsock loopback)")?;
    // SAFETY: listener is bound and the backlog value is valid.
    check_socket_result(unsafe { libc::listen(listener.as_raw_fd(), 16) }, "listen(vsock)")?;
    // SAFETY: address and length are valid output parameters.
    let mut bound: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
    let mut bound_len = address_len;
    check_socket_result(
        unsafe {
            libc::getsockname(listener.as_raw_fd(), (&raw mut bound).cast(), &raw mut bound_len)
        },
        "getsockname(vsock)",
    )?;
    let peer = new_vsock_fd()?;
    let destination = probe_vsock_address(PROBE_CID_LOCAL, bound.svm_port)?;
    // SAFETY: destination is a valid sockaddr_vm and peer is an owned socket.
    check_socket_result(
        unsafe { libc::connect(peer.as_raw_fd(), (&raw const destination).cast(), address_len) },
        "connect(vsock loopback)",
    )?;
    let mut pollfd = libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: pollfd points to one initialized descriptor entry.
    let ready = unsafe { libc::poll(&raw mut pollfd, 1, PROBE_TIMEOUT_MS) };
    if ready <= 0 {
        return Err(if ready == 0 {
            io::Error::new(io::ErrorKind::TimedOut, "vsock accept timed out")
        } else {
            io::Error::last_os_error()
        });
    }
    // SAFETY: accept4 returns a fresh descriptor and writes a valid peer address.
    let accepted = unsafe {
        libc::accept4(
            listener.as_raw_fd(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            libc::SOCK_CLOEXEC,
        )
    };
    let accepted = check_socket_result(accepted, "accept(vsock loopback)")?;
    // SAFETY: accepted is a fresh descriptor returned by accept4.
    let accepted = unsafe { OwnedFd::from_raw_fd(accepted) };
    set_probe_timeout(accepted.as_raw_fd())?;
    set_probe_timeout(peer.as_raw_fd())?;
    Ok((accepted, peer))
}

fn probe_tcp_pair() -> io::Result<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let far = TcpStream::connect(listener.local_addr()?)?;
    let (half, _) = listener.accept()?;
    half.set_read_timeout(Some(Duration::from_secs(5)))?;
    half.set_write_timeout(Some(Duration::from_secs(5)))?;
    far.set_read_timeout(Some(Duration::from_secs(5)))?;
    far.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok((half, far))
}

fn probe_wait(fd: RawFd, events: libc::c_short) -> io::Result<()> {
    let mut descriptor = libc::pollfd { fd, events, revents: 0 };
    // SAFETY: descriptor is initialized for one poll entry.
    let result = unsafe { libc::poll(&raw mut descriptor, 1, PROBE_TIMEOUT_MS) };
    if result == 0 {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "forwarder probe socket timed out"));
    }
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if descriptor.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0
        && descriptor.revents & events == 0
    {
        return Err(io::Error::new(io::ErrorKind::BrokenPipe, "forwarder probe socket ended"));
    }
    Ok(())
}

fn probe_write_all(fd: RawFd, bytes: &[u8]) -> io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        probe_wait(fd, libc::POLLOUT)?;
        // SAFETY: bytes[offset..] is a valid readable slice.
        let result = unsafe {
            libc::send(
                fd,
                bytes[offset..].as_ptr().cast(),
                bytes.len() - offset,
                libc::MSG_NOSIGNAL,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if result == 0 {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "probe send returned zero"));
        }
        offset += usize::try_from(result).unwrap_or(0);
    }
    Ok(())
}

fn probe_read_exact(fd: RawFd, bytes: &mut [u8]) -> io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        probe_wait(fd, libc::POLLIN)?;
        // SAFETY: bytes[offset..] is a valid writable slice.
        let result =
            unsafe { libc::recv(fd, bytes[offset..].as_mut_ptr().cast(), bytes.len() - offset, 0) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if result == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "probe stream ended early"));
        }
        offset += usize::try_from(result).unwrap_or(0);
    }
    Ok(())
}

fn probe_read_end(fd: RawFd) -> io::Result<()> {
    let mut byte = 0_u8;
    loop {
        probe_wait(fd, libc::POLLIN)?;
        // SAFETY: byte is writable for one byte.
        let result = unsafe { libc::recv(fd, (&raw mut byte).cast(), 1, 0) };
        if result == 0 {
            return Ok(());
        }
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
    }
}

fn probe_expect_reset(fd: RawFd) -> io::Result<bool> {
    let mut byte = 0_u8;
    probe_wait(fd, libc::POLLIN)?;
    // SAFETY: byte is writable for one byte.
    let result = unsafe { libc::recv(fd, (&raw mut byte).cast(), 1, 0) };
    if result == 0 {
        return Ok(false);
    }
    if result < 0 {
        let error = io::Error::last_os_error();
        return Ok(matches!(
            error.kind(),
            io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
        ));
    }
    Ok(false)
}

fn probe_stream_round_trip(
    forwarder: &KernelFlowForwarder,
    cleanup: &mut ProbeCleanup<'_>,
) -> Result<(), ForwarderError> {
    let stage = ForwarderProbeStage::StreamRoundTrip;
    probe_wrap(
        stage,
        (|| {
            let (vsock, vsock_far) =
                probe_vsock_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let (tcp, tcp_far) =
                probe_tcp_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let id = forwarder.attach(&vsock as &dyn AsFd, &tcp as &dyn AsFd, PairMode::Stream)?;
            cleanup.track(id);
            let request = b"probe-request-ovd-fwd";
            let response = b"probe-response-ovd-fwd";
            probe_write_all(vsock_far.as_raw_fd(), request)
                .map_err(|error| probe_io(stage, ForwarderOp::Attach, error))?;
            let mut received = vec![0_u8; request.len()];
            probe_read_exact(tcp_far.as_raw_fd(), &mut received)
                .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
            if received != request {
                return Err(probe_mismatch(stage, ForwarderProbeObservation::BytesDiffer));
            }
            probe_write_all(tcp_far.as_raw_fd(), response)
                .map_err(|error| probe_io(stage, ForwarderOp::Attach, error))?;
            unsafe { libc::shutdown(tcp_far.as_raw_fd(), libc::SHUT_WR) };
            let mut received = vec![0_u8; response.len()];
            probe_read_exact(vsock_far.as_raw_fd(), &mut received)
                .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
            if received != response {
                return Err(probe_mismatch(stage, ForwarderProbeObservation::BytesDiffer));
            }
            unsafe { libc::shutdown(vsock_far.as_raw_fd(), libc::SHUT_WR) };
            probe_read_end(tcp_far.as_raw_fd())
                .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
            probe_read_end(vsock_far.as_raw_fd())
                .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
            let end = probe_wait_end(forwarder, id, stage)?;
            if end.outcome != ForwardOutcome::Completed {
                return Err(probe_mismatch(stage, ForwarderProbeObservation::WrongOutcome));
            }
            cleanup.clear(id);
            Ok(())
        })(),
    )
}

fn probe_half_close(
    forwarder: &KernelFlowForwarder,
    cleanup: &mut ProbeCleanup<'_>,
) -> Result<(), ForwarderError> {
    let stage = ForwarderProbeStage::HalfClose;
    probe_wrap(
        stage,
        (|| {
            let (vsock, vsock_far) =
                probe_vsock_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let (tcp, tcp_far) =
                probe_tcp_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let id = forwarder.attach(&vsock as &dyn AsFd, &tcp as &dyn AsFd, PairMode::Stream)?;
            cleanup.track(id);
            let tail = b"half-close-tail";
            probe_write_all(vsock_far.as_raw_fd(), tail)
                .map_err(|error| probe_io(stage, ForwarderOp::Attach, error))?;
            // SAFETY: the peer descriptor is owned by this probe.
            check_socket_result(
                unsafe { libc::shutdown(vsock_far.as_raw_fd(), libc::SHUT_WR) },
                "shutdown(vsock write)",
            )
            .map_err(|error| probe_io(stage, ForwarderOp::Attach, error))?;
            let mut received = vec![0_u8; tail.len()];
            probe_read_exact(tcp_far.as_raw_fd(), &mut received)
                .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
            if received != tail {
                return Err(probe_mismatch(stage, ForwarderProbeObservation::TailTruncated));
            }
            probe_read_end(tcp_far.as_raw_fd())
                .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
            probe_write_all(tcp_far.as_raw_fd(), b"reverse-tail")
                .map_err(|error| probe_io(stage, ForwarderOp::Attach, error))?;
            // SAFETY: this probe owns the TCP peer descriptor.
            check_socket_result(
                unsafe { libc::shutdown(tcp_far.as_raw_fd(), libc::SHUT_WR) },
                "shutdown(TCP write)",
            )
            .map_err(|error| probe_io(stage, ForwarderOp::Attach, error))?;
            let reverse = b"reverse-tail";
            let mut received = vec![0_u8; reverse.len()];
            probe_read_exact(vsock_far.as_raw_fd(), &mut received)
                .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
            if received != reverse {
                return Err(probe_mismatch(stage, ForwarderProbeObservation::TailTruncated));
            }
            probe_read_end(vsock_far.as_raw_fd())
                .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
            let end = probe_wait_end(forwarder, id, stage)?;
            if end.outcome != ForwardOutcome::Completed {
                return Err(probe_mismatch(stage, ForwarderProbeObservation::WrongOutcome));
            }
            cleanup.clear(id);
            Ok(())
        })(),
    )
}

fn probe_peer_abort_classified(
    forwarder: &KernelFlowForwarder,
    cleanup: &mut ProbeCleanup<'_>,
) -> Result<(), ForwarderError> {
    let stage = ForwarderProbeStage::PeerAbortClassified;
    probe_wrap(
        stage,
        (|| {
            let (vsock, vsock_far) =
                probe_vsock_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let (tcp, tcp_far) =
                probe_tcp_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let id = forwarder.attach(&vsock as &dyn AsFd, &tcp as &dyn AsFd, PairMode::Stream)?;
            cleanup.track(id);
            // SAFETY: this probe owns the vsock peer descriptor.
            check_socket_result(
                unsafe { libc::shutdown(vsock_far.as_raw_fd(), libc::SHUT_RDWR) },
                "shutdown(vsock both)",
            )
            .map_err(|error| probe_io(stage, ForwarderOp::Attach, error))?;
            let reset = probe_expect_reset(tcp_far.as_raw_fd())
                .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
            probe_require(reset, stage, ForwarderProbeObservation::FarEndNotReset)?;
            let end = probe_wait_end(forwarder, id, stage)?;
            probe_require(
                end.outcome
                    == ForwardOutcome::Aborted {
                        cause: AbortCause::PeerAbort { side: PairSide::A },
                    },
                stage,
                ForwarderProbeObservation::WrongOutcome,
            )?;
            cleanup.clear(id);
            Ok(())
        })(),
    )
}

fn probe_backpressure(
    forwarder: &KernelFlowForwarder,
    cleanup: &mut ProbeCleanup<'_>,
) -> Result<(), ForwarderError> {
    let stage = ForwarderProbeStage::Backpressure;
    probe_wrap(
        stage,
        (|| {
            let (vsock, vsock_far) =
                probe_vsock_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let (tcp, tcp_far) =
                probe_tcp_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            probe_set_tcp_rcvbuf(&tcp_far, 4 * 1024)
                .map_err(|error| probe_io(stage, ForwarderOp::Attach, error))?;
            let id = forwarder.attach(&vsock as &dyn AsFd, &tcp as &dyn AsFd, PairMode::Stream)?;
            cleanup.track(id);
            let blocked = probe_nonblocking_send_until_stalled(vsock_far.as_raw_fd())
                .map_err(|error| probe_io(stage, ForwarderOp::Attach, error))?;
            if !blocked {
                return Err(probe_mismatch(stage, ForwarderProbeObservation::SenderNotBlocked));
            }
            probe_abort_and_clear(cleanup, id, stage)?;
            Ok(())
        })(),
    )
}

fn probe_datagram_round_trip(
    forwarder: &KernelFlowForwarder,
    cleanup: &mut ProbeCleanup<'_>,
) -> Result<(), ForwarderError> {
    let stage = ForwarderProbeStage::DatagramRoundTrip;
    probe_wrap(
        stage,
        (|| {
            let (vsock, vsock_far) =
                probe_vsock_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
                .map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let udp_far = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
                .map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            udp.connect(
                udp_far.local_addr().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?,
            )
            .map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            udp_far
                .connect(
                    udp.local_addr().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?,
                )
                .map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            udp_far
                .set_read_timeout(Some(Duration::from_secs(5)))
                .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
            let id = forwarder.attach(
                &vsock as &dyn AsFd,
                &udp as &dyn AsFd,
                PairMode::Datagram { udp: PairSide::B },
            )?;
            cleanup.track(id);
            let maximum = vec![0x7b; MAX_GUEST_DATAGRAM_FRAME as usize - 8];
            for payload in [b"x".as_slice(), b"".as_slice(), maximum.as_slice()] {
                udp_far
                    .send(payload)
                    .map_err(|error| probe_io(stage, ForwarderOp::Attach, error))?;
                let expected = probe_make_frame(payload);
                let mut received = vec![0_u8; expected.len()];
                probe_read_exact(vsock_far.as_raw_fd(), &mut received)
                    .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
                if received != expected {
                    return Err(probe_mismatch(stage, ForwarderProbeObservation::DatagramSplit));
                }
                let reply = probe_make_frame(payload);
                probe_write_all(vsock_far.as_raw_fd(), &reply)
                    .map_err(|error| probe_io(stage, ForwarderOp::Attach, error))?;
                let mut received = vec![0_u8; payload.len().max(1)];
                let length = udp_far
                    .recv(&mut received)
                    .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
                if received[..length] != *payload {
                    return Err(probe_mismatch(stage, ForwarderProbeObservation::DatagramSplit));
                }
            }
            probe_abort_and_clear(cleanup, id, stage)?;
            Ok(())
        })(),
    )
}

fn probe_watch_classified(
    forwarder: &KernelFlowForwarder,
    cleanup: &mut ProbeCleanup<'_>,
) -> Result<(), ForwarderError> {
    let stage = ForwarderProbeStage::WatchClassified;
    probe_wrap(
        stage,
        (|| {
            let (half, peer) =
                probe_vsock_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let id = forwarder.watch(&half as &dyn AsFd)?;
            cleanup.track(id);
            check_socket_result(
                unsafe { libc::shutdown(peer.as_raw_fd(), libc::SHUT_WR) },
                "shutdown(vsock watch half-close)",
            )
            .map_err(|error| probe_io(stage, ForwarderOp::Watch, error))?;
            let end = probe_wait_end(forwarder, id, stage)?;
            probe_require(
                end.outcome == ForwardOutcome::GuestHalfClose,
                stage,
                ForwarderProbeObservation::WrongOutcome,
            )?;
            cleanup.clear(id);

            let (half, peer) =
                probe_vsock_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let id = forwarder.watch(&half as &dyn AsFd)?;
            cleanup.track(id);
            check_socket_result(
                unsafe { libc::shutdown(peer.as_raw_fd(), libc::SHUT_RDWR) },
                "shutdown(vsock watch abort)",
            )
            .map_err(|error| probe_io(stage, ForwarderOp::Watch, error))?;
            let end = probe_wait_end(forwarder, id, stage)?;
            probe_require(
                end.outcome
                    == ForwardOutcome::Aborted {
                        cause: AbortCause::PeerAbort { side: PairSide::A },
                    },
                stage,
                ForwarderProbeObservation::WrongOutcome,
            )?;
            cleanup.clear(id);
            Ok(())
        })(),
    )
}

fn probe_context_release(stage: ForwarderProbeStage) -> Result<(), ForwarderError> {
    probe_wrap(
        stage,
        (|| {
            let second = KernelFlowForwarder::open()?;
            let (vsock, _vsock_far) =
                probe_vsock_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            let (tcp, tcp_far) =
                probe_tcp_pair().map_err(|error| probe_io(stage, ForwarderOp::Open, error))?;
            second.attach(&vsock as &dyn AsFd, &tcp as &dyn AsFd, PairMode::Stream)?;
            drop(second);
            let reset = probe_expect_reset(tcp_far.as_raw_fd())
                .map_err(|error| probe_io(stage, ForwarderOp::Read, error))?;
            probe_require(reset, stage, ForwarderProbeObservation::FarEndNotReset)
        })(),
    )
}

fn probe_forwarder(owner: &KernelFlowForwarder) -> Result<(), ForwarderError> {
    let device_probe = probe_wrap(ForwarderProbeStage::DeviceOpen, KernelFlowForwarder::open())?;
    drop(device_probe);

    probe_wrap(ForwarderProbeStage::Version, probe_check_version(owner))?;
    let mut cleanup = ProbeCleanup::new(owner);
    probe_stream_round_trip(owner, &mut cleanup)?;
    probe_half_close(owner, &mut cleanup)?;
    probe_peer_abort_classified(owner, &mut cleanup)?;
    probe_backpressure(owner, &mut cleanup)?;
    probe_datagram_round_trip(owner, &mut cleanup)?;
    probe_watch_classified(owner, &mut cleanup)?;
    probe_context_release(ForwarderProbeStage::ContextRelease)?;

    probe_wrap(
        ForwarderProbeStage::Cleanup,
        (|| {
            let inventory = owner.inventory()?;
            if inventory.live_pairs != 0
                || inventory.live_watches != 0
                || inventory.queued_ends != 0
            {
                let pairs = inventory.live_pairs + inventory.live_watches + inventory.queued_ends;
                return Err(probe_mismatch(
                    ForwarderProbeStage::Cleanup,
                    ForwarderProbeObservation::ResidueRemains { pairs },
                ));
            }
            if owner.take_end()?.is_some() {
                return Err(probe_mismatch(
                    ForwarderProbeStage::Cleanup,
                    ForwarderProbeObservation::ResidueRemains { pairs: 1 },
                ));
            }
            Ok(())
        })(),
    )?;
    Ok(())
}
