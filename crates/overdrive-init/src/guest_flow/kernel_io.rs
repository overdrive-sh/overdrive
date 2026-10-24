#![allow(
    unsafe_code,
    reason = "the real guest adapter is the narrow POSIX socket and poll boundary"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, ErrorKind};
use std::mem::size_of;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::time::Instant;

use aya::maps::{Array, HashMap as AyaHashMap, Map, MapData, Queue, RingBuf};
use nix::errno::Errno;
use nix::libc;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::socket::{
    AddressFamily, MsgFlags, NetlinkAddr, SockFlag, SockProtocol, SockType, SockaddrIn, VsockAddr,
    accept4, bind, connect, recv, sendto, socket,
};
use overdrive_core::guest_transport::forwarder::{
    AbortResult, FlowForwarder, ForwardId, ForwarderError, ForwarderInventory, PairMode, PairSide,
    PairState,
};
use overdrive_core::guest_transport::wire::FLOW_REQUEST_LEN;
use overdrive_core::guest_transport::{GUEST_TCP_PENDING_MAX, GUEST_UDP_SLOT_BASE};

use super::{
    AppConn, CapturedConn, GuestAttachRefused, GuestHangup, GuestIoEvent, GuestListener,
    GuestOwnedSocket, GuestOwnerIo, GuestPair, GuestPairSocket, GuestRecord,
    GuestSlotAttachRefused, Handle, HostChannel, HostConn, InboundConn, SeamInstance, SocketCookie,
    SocketId,
};
use crate::transport::{
    GUEST_TPROXY_MARK, GuestProgram, GuestTransportSetupError, KernelGuestTransportOps,
};

const USER_EVENT_LEN: usize = 16;
const USER_EVENT_SOCKET_RELEASED: u32 = 1;
const CHILD_PENDING: u32 = 1;
const CHILD_READY: u32 = 2;
const CHILD_REJECTED: u32 = 3;
const CHILD_CANDIDATE_READY: u32 = 4;
const CHILD_CANDIDATE_REJECTED: u32 = 5;
const TCP_SYN_RECV_STATE: u8 = 3;
const TCP_ESTABLISHED_STATE: u8 = 1;
const SO_COOKIE: i32 = 57;
const IP_TRANSPARENT: i32 = 19;
const SOCK_DIAG_BY_FAMILY: u16 = 20;
const NETLINK_HEADER_LEN: usize = 16;
const INET_DIAG_MSG_LEN: usize = 72;

type ReadCompletion = (GuestRecord, Result<Vec<u8>, ErrorKind>);

const fn poll_revents(poll_fd: &PollFd<'_>) -> PollFlags {
    // SAFETY: nix 0.30.1 declares `PollFd` as `#[repr(transparent)]` over
    // `libc::pollfd`; reading its raw `revents` preserves kernel flags that
    // nix's `PollFlags` does not name, including Linux `POLLRDHUP`.
    let raw = unsafe { &*std::ptr::from_ref(poll_fd).cast::<libc::pollfd>() };
    PollFlags::from_bits_retain(raw.revents)
}

#[derive(Clone, Copy)]
struct ReadRequest {
    record: GuestRecord,
    expected: usize,
}

struct SocketRecord {
    fd: OwnedFd,
    kind: SocketKind,
    connecting: bool,
    read: Option<(ReadRequest, Vec<u8>)>,
    hangup_reported: bool,
}

#[derive(Clone, Copy)]
enum SocketKind {
    Host(HostChannel),
    Inbound,
    Captured,
    App,
    Pair,
}

struct HeldPendingChild {
    socket: SocketId,
}

#[derive(Clone, Copy)]
enum PollTag {
    TcpListener,
    InboundListener,
    UdpSlot(SocketAddrV4),
    Forwarder,
    UserEvents,
    Socket(SocketId),
}

pub(crate) struct KernelGuestOwnerIo {
    ops: KernelGuestTransportOps,
    seam: SeamInstance,
    next_socket: u64,
    sockets: BTreeMap<SocketId, SocketRecord>,
    events: VecDeque<GuestIoEvent>,
    pending_children: BTreeMap<u64, HeldPendingChild>,
    captured_total: usize,
    connected_slots: BTreeSet<SocketAddrV4>,
    slot_pairs: BTreeMap<SocketAddrV4, ForwardId>,
    slot_readiness_reported: BTreeSet<SocketAddrV4>,
    child_map: AyaHashMap<MapData, u64, [u8; 24]>,
    events_ring: RingBuf<MapData>,
}

impl KernelGuestOwnerIo {
    pub(crate) fn new(mut ops: KernelGuestTransportOps) -> Result<Self, GuestTransportSetupError> {
        let bpf = ops.bpf.as_mut().ok_or_else(|| {
            correlation_program_load_error(aya::EbpfError::ProgramError(
                aya::programs::ProgramError::NotLoaded,
            ))
        })?;
        let child_map = AyaHashMap::<_, u64, [u8; 24]>::try_from(
            take_guest_map(bpf, "CHILD_CORRELATION").map_err(correlation_program_load_error)?,
        )
        .map_err(|error| correlation_program_load_error(aya::EbpfError::MapError(error)))?;
        let events_ring = RingBuf::try_from(
            take_guest_map(bpf, "USER_EVENTS").map_err(correlation_program_load_error)?,
        )
        .map_err(|error| correlation_program_load_error(aya::EbpfError::MapError(error)))?;
        Ok(Self {
            ops,
            seam: SeamInstance::next(),
            next_socket: 1,
            sockets: BTreeMap::new(),
            events: VecDeque::new(),
            pending_children: BTreeMap::new(),
            captured_total: 0,
            connected_slots: BTreeSet::new(),
            slot_pairs: BTreeMap::new(),
            slot_readiness_reported: BTreeSet::new(),
            child_map,
            events_ring,
        })
    }

    fn issue(&mut self, fd: OwnedFd, kind: SocketKind, connecting: bool) -> io::Result<Handle> {
        let id = SocketId(self.next_socket);
        self.next_socket = self
            .next_socket
            .checked_add(1)
            .ok_or_else(|| io::Error::other("guest socket handle id space exhausted"))?;
        self.sockets
            .insert(id, SocketRecord { fd, kind, connecting, read: None, hangup_reported: false });
        Ok(Handle { seam: self.seam, id })
    }

    fn check(&self, handle: &Handle) -> SocketId {
        assert_eq!(handle.seam, self.seam, "a handle belongs to a different guest I/O seam");
        handle.id
    }

    fn socket(&self, id: SocketId) -> io::Result<&SocketRecord> {
        self.sockets
            .get(&id)
            .ok_or_else(|| io::Error::other("guest owner referenced a released socket"))
    }

    fn socket_mut(&mut self, id: SocketId) -> io::Result<&mut SocketRecord> {
        self.sockets
            .get_mut(&id)
            .ok_or_else(|| io::Error::other("guest owner referenced a released socket"))
    }

    fn listener_fd(&self, listener: GuestListener) -> Option<RawFd> {
        match listener {
            GuestListener::Inbound => self.ops.inbound_listener.as_ref().map(AsRawFd::as_raw_fd),
            GuestListener::TcpIntake => self.ops.tcp_intake.as_ref().map(AsRawFd::as_raw_fd),
        }
    }

    fn child_state(&self, cookie: u64) -> Option<[u8; 24]> {
        self.child_map.get(&cookie, 0).ok()
    }

    fn child_state_code(&self, cookie: u64) -> Option<u32> {
        let state = self.child_state(cookie)?;
        Some(u32::from_ne_bytes(state[0..4].try_into().ok()?))
    }

    fn child_ready(&self, cookie: u64) -> Option<(SocketCookie, SocketAddrV4)> {
        let state = self.child_state(cookie)?;
        if u32::from_ne_bytes(state[0..4].try_into().ok()?) != CHILD_READY {
            return None;
        }
        let application_cookie = u64::from_ne_bytes(state[8..16].try_into().ok()?);
        let ip = Ipv4Addr::from(u32::from_ne_bytes(state[16..20].try_into().ok()?).to_ne_bytes());
        // `Destination.port` is a native BPF map field in host order.
        let port = u16::from_ne_bytes(state[20..22].try_into().ok()?);
        Some((SocketCookie::new(application_cookie).ok()?, SocketAddrV4::new(ip, port)))
    }

    fn scan_held_children(&mut self) -> io::Result<()> {
        let pending = self.pending_children.keys().copied().collect::<Vec<_>>();
        for cookie in pending {
            let Some(child_socket) = self.pending_children.get(&cookie).map(|child| child.socket)
            else {
                continue;
            };
            match self.child_state_code(cookie) {
                Some(CHILD_PENDING | CHILD_CANDIDATE_READY | CHILD_CANDIDATE_REJECTED) => {
                    let fd = self.socket(child_socket)?.fd.as_raw_fd();
                    if socket_hung_up(fd)? {
                        // A held preterminal Fast Open child is not eligible
                        // for capture after its peer reset it. Re-read the row
                        // because the post-validation callback may have
                        // promoted it while this scan was checking the fd;
                        // a terminal Ready row keeps its normal capture path.
                        match self.child_state_code(cookie) {
                            Some(CHILD_READY) => {}
                            Some(_) => self.reject_held_child(cookie)?,
                            None => self.discard_held_child(cookie),
                        }
                    }
                }
                Some(CHILD_READY | CHILD_REJECTED) => {
                    let fd = self.socket(child_socket)?.fd.as_raw_fd();
                    let state = socket_tcp_state(fd)?;
                    if state != TCP_ESTABLISHED_STATE {
                        if socket_hung_up(fd)? {
                            self.reject_held_child(cookie)?;
                        }
                        continue;
                    }
                    if self.child_state_code(cookie) == Some(CHILD_READY) {
                        if let Some((application_cookie, destination)) = self.child_ready(cookie) {
                            let _ = self.child_map.remove(&cookie);
                            let Some(held) = self.pending_children.remove(&cookie) else {
                                return Err(io::Error::other(
                                    "held pending child disappeared during classification",
                                ));
                            };
                            self.events.push_back(GuestIoEvent::OutboundCaptured {
                                child: CapturedConn(Handle { seam: self.seam, id: held.socket }),
                                original_destination: destination,
                                cookie: application_cookie,
                            });
                        } else {
                            self.reject_held_child(cookie)?;
                        }
                    } else {
                        self.reject_held_child(cookie)?;
                    }
                }
                Some(_) => self.reject_held_child(cookie)?,
                None => {
                    if socket_hung_up(self.socket(child_socket)?.fd.as_raw_fd())? {
                        self.discard_held_child(cookie);
                    }
                }
            }
        }
        Ok(())
    }

    fn discard_held_child(&mut self, cookie: u64) {
        if let Some(held) = self.pending_children.remove(&cookie) {
            self.sockets.remove(&held.socket);
            self.captured_total = self.captured_total.saturating_sub(1);
            let _ = self.child_map.remove(&cookie);
        }
    }

    fn accept_tcp_children(&mut self) -> io::Result<()> {
        let Some(listener_fd) = self.ops.tcp_intake.as_ref().map(AsRawFd::as_raw_fd) else {
            return Ok(());
        };
        loop {
            match accept4(listener_fd, SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK) {
                Ok(raw) => {
                    // SAFETY: accept4 returns a new descriptor owned here.
                    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
                    self.classify_tcp_child(fd)?;
                }
                Err(Errno::EAGAIN | Errno::ECONNABORTED | Errno::EMFILE | Errno::ENFILE) => {
                    return Ok(());
                }
                Err(error) => {
                    self.events.push_back(GuestIoEvent::ListenerFailed {
                        listener: GuestListener::TcpIntake,
                        kind: errno_kind(error as i32),
                    });
                    return Ok(());
                }
            }
        }
    }

    fn classify_tcp_child(&mut self, fd: OwnedFd) -> io::Result<()> {
        let cookie = socket_cookie(fd.as_raw_fd())?;
        let pending_max = usize::try_from(GUEST_TCP_PENDING_MAX).unwrap_or(usize::MAX);
        if self.captured_total >= pending_max {
            set_linger(fd.as_raw_fd(), true)?;
            drop(fd);
            let _ = self.child_map.remove(&cookie);
            return Ok(());
        }
        let state = self.child_state_code(cookie);
        match state {
            Some(CHILD_PENDING | CHILD_CANDIDATE_READY | CHILD_CANDIDATE_REJECTED) => {
                self.hold_pending_child(cookie, fd)?;
            }
            Some(CHILD_READY) => {
                if socket_tcp_state(fd.as_raw_fd())? == TCP_SYN_RECV_STATE {
                    self.hold_pending_child(cookie, fd)?;
                } else if let Some((application_cookie, destination)) = self.child_ready(cookie) {
                    let _ = self.child_map.remove(&cookie);
                    let handle = self.issue(fd, SocketKind::Captured, false)?;
                    self.captured_total += 1;
                    self.events.push_back(GuestIoEvent::OutboundCaptured {
                        child: CapturedConn(handle),
                        original_destination: destination,
                        cookie: application_cookie,
                    });
                } else {
                    set_linger(fd.as_raw_fd(), true)?;
                    let _ = self.child_map.remove(&cookie);
                }
            }
            Some(CHILD_REJECTED) => {
                set_linger(fd.as_raw_fd(), true)?;
                let _ = self.child_map.remove(&cookie);
            }
            Some(_) | None => {
                // An uncorrelated or rejected intake connection is reset and
                // emits no owner event.
                set_linger(fd.as_raw_fd(), true)?;
                let _ = self.child_map.remove(&cookie);
            }
        }
        Ok(())
    }

    fn hold_pending_child(&mut self, cookie: u64, fd: OwnedFd) -> io::Result<()> {
        let handle = self.issue(fd, SocketKind::Captured, false)?;
        self.captured_total += 1;
        self.pending_children.insert(cookie, HeldPendingChild { socket: handle.id });
        Ok(())
    }

    fn reject_held_child(&mut self, cookie: u64) -> io::Result<()> {
        if let Some(held) = self.pending_children.remove(&cookie) {
            let linger_result = self
                .sockets
                .get(&held.socket)
                .map_or(Ok(()), |record| set_linger(record.fd.as_raw_fd(), true));
            self.sockets.remove(&held.socket);
            self.captured_total = self.captured_total.saturating_sub(1);
            let _ = self.child_map.remove(&cookie);
            linger_result?;
        }
        Ok(())
    }

    fn accept_inbound(&mut self) -> io::Result<()> {
        let Some(listener_fd) = self.ops.inbound_listener.as_ref().map(AsRawFd::as_raw_fd) else {
            return Ok(());
        };
        loop {
            match accept4(listener_fd, SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK) {
                Ok(raw) => {
                    // SAFETY: accept4 returned a new descriptor owned by this adapter.
                    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
                    let handle = self.issue(fd, SocketKind::Inbound, false)?;
                    self.events
                        .push_back(GuestIoEvent::InboundAccepted { conn: InboundConn(handle) });
                }
                Err(Errno::EAGAIN | Errno::ECONNABORTED | Errno::EMFILE | Errno::ENFILE) => {
                    return Ok(());
                }
                Err(error) => {
                    self.events.push_back(GuestIoEvent::ListenerFailed {
                        listener: GuestListener::Inbound,
                        kind: errno_kind(error as i32),
                    });
                    return Ok(());
                }
            }
        }
    }

    fn drain_user_events(&mut self) {
        let mut released = Vec::new();
        while let Some(item) = self.events_ring.next() {
            let bytes: &[u8] = &item;
            if bytes.len() != USER_EVENT_LEN {
                continue;
            }
            let kind = u32::from_ne_bytes(bytes[0..4].try_into().unwrap_or([0; 4]));
            let value = u64::from_ne_bytes(bytes[8..16].try_into().unwrap_or([0; 8]));
            if kind == USER_EVENT_SOCKET_RELEASED {
                released.push(value);
            }
        }
        for raw in released {
            if let Ok(cookie) = SocketCookie::new(raw) {
                self.events.push_back(GuestIoEvent::SocketReleased { cookie });
            }
        }
    }

    fn build_poll_set(&self) -> (Vec<PollFd<'_>>, Vec<PollTag>) {
        let mut fds = Vec::with_capacity(self.sockets.len() + usize::from(GUEST_UDP_SLOT_BASE) + 5);
        let mut tags = Vec::with_capacity(fds.capacity());
        if let Some(listener) = self.ops.tcp_intake.as_ref() {
            fds.push(PollFd::new(listener.as_fd(), PollFlags::POLLIN));
            tags.push(PollTag::TcpListener);
        }
        if let Some(listener) = self.ops.inbound_listener.as_ref() {
            fds.push(PollFd::new(listener.as_fd(), PollFlags::POLLIN));
            tags.push(PollTag::InboundListener);
        }
        for (offset, socket) in self.ops.udp_slots.iter().enumerate() {
            if let Some(token) = self.ops.token {
                let slot = SocketAddrV4::new(
                    token.workload_addr,
                    GUEST_UDP_SLOT_BASE + u16::try_from(offset).unwrap_or(u16::MAX),
                );
                let mut flags = PollFlags::empty();
                if !self.slot_readiness_reported.contains(&slot)
                    && !self.slot_pairs.contains_key(&slot)
                {
                    flags |= PollFlags::POLLIN;
                }
                fds.push(PollFd::new(socket.as_fd(), flags));
                tags.push(PollTag::UdpSlot(slot));
            }
        }
        if let Some(forwarder) = self.ops.forwarder.as_ref() {
            fds.push(PollFd::new(forwarder.readiness_fd(), PollFlags::POLLIN));
            tags.push(PollTag::Forwarder);
        }
        // RingBuf exposes its map fd as RawFd; its lifetime is owned by `events_ring`.
        let ring_fd = unsafe { BorrowedFd::borrow_raw(self.events_ring.as_raw_fd()) };
        fds.push(PollFd::new(ring_fd, PollFlags::POLLIN));
        tags.push(PollTag::UserEvents);
        for (id, record) in &self.sockets {
            let mut flags = PollFlags::POLLERR
                | PollFlags::POLLHUP
                | PollFlags::from_bits_retain(libc::POLLRDHUP);
            if record.connecting {
                flags |= PollFlags::POLLOUT;
            }
            if record.read.is_some() {
                flags |= PollFlags::POLLIN;
            }
            fds.push(PollFd::new(record.fd.as_fd(), flags));
            tags.push(PollTag::Socket(*id));
        }
        (fds, tags)
    }

    fn poll_once(&mut self, deadline: Option<Instant>) -> io::Result<bool> {
        let (mut fds, tags) = self.build_poll_set();
        let timeout = deadline.map_or(PollTimeout::NONE, poll_timeout_until);
        let ready = poll(&mut fds, timeout).map_err(errno_to_io)?;
        if ready == 0 {
            return Ok(false);
        }
        let mut work = Vec::new();
        for (index, poll_fd) in fds.iter().enumerate() {
            let flags = poll_revents(poll_fd);
            if flags.is_empty() {
                continue;
            }
            work.push((tags[index], flags));
        }
        for (tag, flags) in work {
            match tag {
                PollTag::TcpListener
                    if flags.intersects(PollFlags::POLLIN | PollFlags::POLLERR) =>
                {
                    self.accept_tcp_children()?;
                }
                PollTag::InboundListener
                    if flags.intersects(PollFlags::POLLIN | PollFlags::POLLERR) =>
                {
                    self.accept_inbound()?;
                }
                PollTag::UdpSlot(slot)
                    if flags.contains(PollFlags::POLLIN)
                        && !self.slot_readiness_reported.contains(&slot)
                        && !self.slot_pairs.contains_key(&slot) =>
                {
                    self.slot_readiness_reported.insert(slot);
                    self.events.push_back(GuestIoEvent::SlotReady { slot });
                }
                PollTag::Forwarder if flags.contains(PollFlags::POLLIN) => {
                    if let Some(forwarder) = self.ops.forwarder.as_ref() {
                        while let Some(end) = forwarder.take_end().map_err(io::Error::other)? {
                            self.events.push_back(GuestIoEvent::ForwardEnded { end });
                        }
                    }
                }
                PollTag::UserEvents => self.drain_user_events(),
                PollTag::Socket(id) => self.process_socket(id, flags)?,
                _ => {}
            }
        }
        self.scan_held_children()?;
        Ok(true)
    }

    fn process_socket(&mut self, id: SocketId, flags: PollFlags) -> io::Result<()> {
        let connecting = self.sockets.get(&id).is_some_and(|record| record.connecting);
        if connecting
            && flags.intersects(PollFlags::POLLOUT | PollFlags::POLLERR | PollFlags::POLLHUP)
        {
            let fd = self.socket(id)?.fd.as_raw_fd();
            let error = socket_error(fd)?;
            let result = if error == 0 { Ok(()) } else { Err(errno_kind(error)) };
            self.socket_mut(id)?.connecting = false;
            self.events.push_back(GuestIoEvent::Connected { socket: id, result });
            return Ok(());
        }
        if self.sockets.get(&id).is_some_and(|record| record.read.is_some())
            && flags.intersects(
                PollFlags::POLLIN
                    | PollFlags::POLLERR
                    | PollFlags::POLLHUP
                    | PollFlags::from_bits_retain(libc::POLLRDHUP),
            )
        {
            if let Some((request, result)) = self.read_fixed_record(id)? {
                self.events.push_back(GuestIoEvent::Read { socket: id, result });
                let _ = request;
            }
            return Ok(());
        }
        if flags.intersects(
            PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::from_bits_retain(libc::POLLRDHUP),
        ) {
            let Some(record) = self.sockets.get(&id) else { return Ok(()) };
            if record.hangup_reported {
                return Ok(());
            }
            let hangup = if flags.contains(PollFlags::POLLERR) {
                let raw = socket_error(record.fd.as_raw_fd())?;
                GuestHangup::Error(errno_kind(if raw == 0 { libc::EIO } else { raw }))
            } else if flags.contains(PollFlags::from_bits_retain(libc::POLLRDHUP)) {
                GuestHangup::ReadHangup
            } else {
                GuestHangup::Hangup
            };
            self.socket_mut(id)?.hangup_reported = true;
            self.events.push_back(GuestIoEvent::Hangup { socket: id, hangup });
        }
        Ok(())
    }

    fn read_fixed_record(&mut self, id: SocketId) -> io::Result<Option<ReadCompletion>> {
        let Some(record) = self.sockets.get_mut(&id) else { return Ok(None) };
        let Some((request, mut bytes)) = record.read.take() else { return Ok(None) };
        let mut buffer = [0_u8; FLOW_REQUEST_LEN];
        loop {
            let fd = record.fd.as_raw_fd();
            let remaining = request.expected.saturating_sub(bytes.len());
            let received = unsafe {
                libc::recv(fd, buffer.as_mut_ptr().cast(), remaining, libc::MSG_DONTWAIT)
            };
            if received < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == ErrorKind::WouldBlock || error.kind() == ErrorKind::Interrupted {
                    record.read = Some((request, bytes));
                    return Ok(None);
                }
                return Ok(Some((request.record, Err(error.kind()))));
            }
            if received == 0 {
                let result =
                    if bytes.is_empty() { Ok(Vec::new()) } else { Err(ErrorKind::UnexpectedEof) };
                return Ok(Some((request.record, result)));
            }
            let received = usize::try_from(received).unwrap_or(0);
            bytes.extend_from_slice(&buffer[..received]);
            if bytes.len() >= request.expected {
                bytes.truncate(request.expected);
                let result = Ok(bytes);
                return Ok(Some((request.record, result)));
            }
        }
    }

    fn new_connect(&mut self, channel: HostChannel) -> io::Result<HostConn> {
        let token = self.ops.token.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "guest transport token is unavailable")
        })?;
        let port = match channel {
            HostChannel::FlowStream => token.ports.stream,
            HostChannel::Control => token.ports.control,
        };
        let fd = socket(
            AddressFamily::Vsock,
            SockType::Stream,
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            None,
        )
        .map_err(errno_to_io)?;
        let address = VsockAddr::new(libc::VMADDR_CID_HOST, port.as_u32());
        let connecting = match connect(fd.as_raw_fd(), &address) {
            Ok(()) | Err(Errno::EINPROGRESS) => true,
            Err(error) => return Err(errno_to_io(error)),
        };
        let handle = self.issue(fd, SocketKind::Host(channel), connecting)?;
        Ok(HostConn(handle))
    }

    fn new_app_connect(
        &mut self,
        client: SocketAddrV4,
        destination: SocketAddrV4,
    ) -> io::Result<AppConn> {
        let fd = socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
            None,
        )
        .map_err(errno_to_io)?;
        set_linger(fd.as_raw_fd(), true)?;
        set_option_i32(fd.as_raw_fd(), libc::SOL_IP, IP_TRANSPARENT, 1)?;
        set_option_i32(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_REUSEADDR, 1)?;
        let mark = i32::try_from(GUEST_TPROXY_MARK)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        set_option_i32(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_MARK, mark)?;
        bind(fd.as_raw_fd(), &SockaddrIn::from(client)).map_err(errno_to_io)?;
        let connecting = match connect(fd.as_raw_fd(), &SockaddrIn::from(destination)) {
            Ok(()) | Err(Errno::EINPROGRESS) => true,
            Err(error) => return Err(errno_to_io(error)),
        };
        let handle = self.issue(fd, SocketKind::App, connecting)?;
        Ok(AppConn(handle))
    }

    fn write_all(&self, id: SocketId, bytes: &[u8]) -> io::Result<()> {
        let fd = self.socket(id)?.fd.as_raw_fd();
        let mut offset = 0;
        while offset < bytes.len() {
            let written = unsafe {
                libc::send(
                    fd,
                    bytes[offset..].as_ptr().cast(),
                    bytes.len() - offset,
                    libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                )
            };
            if written < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if written == 0 {
                return Err(io::Error::new(ErrorKind::WriteZero, "socket write made no progress"));
            }
            offset += usize::try_from(written).unwrap_or(0);
        }
        Ok(())
    }

    fn forwarder(&self) -> Result<&overdrive_fwd::KernelFlowForwarder, ForwarderError> {
        self.ops.forwarder.as_ref().ok_or_else(|| ForwarderError::DeviceAbsent {
            source: io::Error::new(
                ErrorKind::NotConnected,
                "guest forwarder context is unavailable",
            ),
        })
    }

    fn set_socket_linger(&self, id: SocketId, enabled: bool) -> io::Result<()> {
        set_linger(self.socket(id)?.fd.as_raw_fd(), enabled)
    }

    fn release_kernel_slot(&mut self, slot: SocketAddrV4) {
        let Some(index) = self.slot_index(slot) else { return };
        let Some(bpf) = self.ops.bpf.as_mut() else { return };
        let value = {
            let Some(map) = bpf.map_mut("SLOT_META") else { return };
            let Ok(meta) = Array::<_, [u8; 16]>::try_from(map) else { return };
            let Ok(value) = meta.get(&index, 0) else { return };
            value
        };
        if u64::from_ne_bytes(value[0..8].try_into().unwrap_or([0; 8])) == 0 {
            return;
        }
        let key: [u8; 16] = value;
        if let Some(map) = bpf.map_mut("SLOT_OWNER")
            && let Ok(mut owners) = AyaHashMap::<_, [u8; 16], u32>::try_from(map)
        {
            let _ = owners.remove(&key);
        }
        if let Some(map) = bpf.map_mut("SLOT_META")
            && let Ok(mut meta) = Array::<_, [u8; 16]>::try_from(map)
        {
            let _ = meta.set(index, [0; 16], 0);
        }
        if let Some(map) = bpf.map_mut("FREE_SLOTS")
            && let Ok(mut free) = Queue::<_, u32>::try_from(map)
        {
            let _ = free.push(index, 0);
        }
    }

    fn slot_index(&self, slot: SocketAddrV4) -> Option<u32> {
        let token = self.ops.token?;
        if *slot.ip() != token.workload_addr || slot.port() < GUEST_UDP_SLOT_BASE {
            return None;
        }
        let index = u32::from(slot.port() - GUEST_UDP_SLOT_BASE);
        let slots_len = u16::try_from(self.ops.udp_slots.len()).ok()?;
        (index < u32::from(slots_len)).then_some(index)
    }

    fn connect_slot_to_app(&mut self, slot: SocketAddrV4, source_port: u16) -> io::Result<()> {
        if self.connected_slots.contains(&slot) {
            return Ok(());
        }
        let token = self.ops.token.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "guest transport token is unavailable")
        })?;
        let index = usize::from(slot.port().checked_sub(GUEST_UDP_SLOT_BASE).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "slot address is outside the guest pool")
        })?);
        let udp = self.ops.udp_slots.get(index).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "slot address is outside the guest pool")
        })?;
        udp.connect(SocketAddrV4::new(token.workload_addr, source_port))?;
        self.connected_slots.insert(slot);
        Ok(())
    }
}

fn poll_timeout_until(deadline: Instant) -> PollTimeout {
    let remaining = deadline.saturating_duration_since(Instant::now());
    let whole_milliseconds = remaining.as_millis();
    let has_submillisecond_remainder = !remaining.subsec_nanos().is_multiple_of(1_000_000);
    let timeout_milliseconds = whole_milliseconds + u128::from(has_submillisecond_remainder);
    PollTimeout::try_from(timeout_milliseconds).unwrap_or(PollTimeout::MAX)
}

impl GuestOwnerIo for KernelGuestOwnerIo {
    fn next_event(&mut self, deadline: Option<Instant>) -> io::Result<Option<GuestIoEvent>> {
        loop {
            self.drain_user_events();
            self.scan_held_children()?;
            if let Some(event) = self.events.pop_front() {
                return Ok(Some(event));
            }
            if deadline.is_some_and(|value| value <= Instant::now()) {
                return Ok(None);
            }
            if !self.poll_once(deadline)? {
                return Ok(None);
            }
        }
    }

    fn start_connect_host(&mut self, channel: HostChannel) -> io::Result<HostConn> {
        self.new_connect(channel)
    }

    fn start_read_host(&mut self, conn: &HostConn, record: GuestRecord) -> io::Result<()> {
        let id = self.check(&conn.0);
        let socket = self.socket_mut(id)?;
        if socket.connecting {
            return Err(io::Error::from(ErrorKind::NotConnected));
        }
        socket.read = Some((read_request(record), Vec::new()));
        Ok(())
    }

    fn start_read_inbound(&mut self, conn: &InboundConn) -> io::Result<()> {
        let id = self.check(&conn.0);
        self.socket_mut(id)?.read = Some((read_request(GuestRecord::FlowRequest), Vec::new()));
        Ok(())
    }

    fn write_host(&mut self, conn: &HostConn, bytes: &[u8]) -> io::Result<()> {
        let id = self.check(&conn.0);
        if self.socket(id)?.connecting {
            return Err(io::Error::from(ErrorKind::NotConnected));
        }
        self.write_all(id, bytes)
    }

    fn inbound_ended(&mut self, conn: &InboundConn) -> io::Result<bool> {
        let id = self.check(&conn.0);
        let mut byte = 0_u8;
        let result = unsafe {
            libc::recv(
                self.socket(id)?.fd.as_raw_fd(),
                (&raw mut byte).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        match result.cmp(&0) {
            std::cmp::Ordering::Equal => Ok(true),
            std::cmp::Ordering::Greater => Ok(false),
            std::cmp::Ordering::Less => {
                let error = io::Error::last_os_error();
                match error.raw_os_error() {
                    Some(libc::EAGAIN) => Ok(false),
                    Some(libc::ECONNRESET | libc::ENOTCONN) => Ok(true),
                    _ if error.kind() == ErrorKind::Interrupted => self.inbound_ended(conn),
                    _ => Err(error),
                }
            }
        }
    }

    fn write_request(
        &mut self,
        vsock: &HostConn,
        request: &[u8; FLOW_REQUEST_LEN],
    ) -> io::Result<()> {
        let id = self.check(&vsock.0);
        if self.socket(id)?.connecting {
            return Err(io::Error::from(ErrorKind::NotConnected));
        }
        self.write_all(id, request)
    }

    fn attach_outbound(
        &mut self,
        child: CapturedConn,
        vsock: HostConn,
    ) -> Result<GuestPair, GuestAttachRefused> {
        let child_id = self.check(&child.0);
        let vsock_id = self.check(&vsock.0);
        let Some(child_socket_fd) =
            self.sockets.get(&child_id).map(|socket| &socket.fd as &dyn AsFd)
        else {
            return Err(GuestAttachRefused {
                error: ForwarderError::NotConnected { side: PairSide::A },
                vsock: vsock.into(),
                outer: child.into(),
            });
        };
        let Some(host_socket_fd) =
            self.sockets.get(&vsock_id).map(|socket| &socket.fd as &dyn AsFd)
        else {
            return Err(GuestAttachRefused {
                error: ForwarderError::NotConnected { side: PairSide::B },
                vsock: vsock.into(),
                outer: child.into(),
            });
        };
        let forwarder = match self.forwarder() {
            Ok(forwarder) => forwarder,
            Err(error) => {
                return Err(GuestAttachRefused { error, vsock: vsock.into(), outer: child.into() });
            }
        };
        match forwarder.attach(child_socket_fd, host_socket_fd, PairMode::Stream) {
            Ok(id) => {
                if let Some(socket) = self.sockets.get_mut(&child_id) {
                    socket.kind = SocketKind::Pair;
                }
                if let Some(socket) = self.sockets.get_mut(&vsock_id) {
                    socket.kind = SocketKind::Pair;
                }
                self.captured_total = self.captured_total.saturating_sub(1);
                Ok(GuestPair {
                    id,
                    vsock: GuestPairSocket(Handle { seam: self.seam, id: child_id }),
                    outer: GuestPairSocket(Handle { seam: self.seam, id: vsock_id }),
                })
            }
            Err(error) => {
                Err(GuestAttachRefused { error, vsock: vsock.into(), outer: child.into() })
            }
        }
    }

    fn start_connect_app(
        &mut self,
        client: SocketAddrV4,
        destination: SocketAddrV4,
    ) -> io::Result<AppConn> {
        self.new_app_connect(client, destination)
    }

    fn attach_inbound(
        &mut self,
        vsock: InboundConn,
        app: AppConn,
    ) -> Result<GuestPair, GuestAttachRefused> {
        let vsock_id = self.check(&vsock.0);
        let app_id = self.check(&app.0);
        let Some(host_socket_fd) =
            self.sockets.get(&vsock_id).map(|socket| &socket.fd as &dyn AsFd)
        else {
            return Err(GuestAttachRefused {
                error: ForwarderError::NotConnected { side: PairSide::A },
                vsock: vsock.into(),
                outer: app.into(),
            });
        };
        let Some(application_socket_fd) =
            self.sockets.get(&app_id).map(|socket| &socket.fd as &dyn AsFd)
        else {
            return Err(GuestAttachRefused {
                error: ForwarderError::NotConnected { side: PairSide::B },
                vsock: vsock.into(),
                outer: app.into(),
            });
        };
        let forwarder = match self.forwarder() {
            Ok(forwarder) => forwarder,
            Err(error) => {
                return Err(GuestAttachRefused { error, vsock: vsock.into(), outer: app.into() });
            }
        };
        match forwarder.attach(host_socket_fd, application_socket_fd, PairMode::Stream) {
            Ok(id) => {
                if let Some(socket) = self.sockets.get_mut(&vsock_id) {
                    socket.kind = SocketKind::Pair;
                }
                if let Some(socket) = self.sockets.get_mut(&app_id) {
                    socket.kind = SocketKind::Pair;
                }
                Ok(GuestPair {
                    id,
                    vsock: GuestPairSocket(Handle { seam: self.seam, id: vsock_id }),
                    outer: GuestPairSocket(Handle { seam: self.seam, id: app_id }),
                })
            }
            Err(error) => Err(GuestAttachRefused { error, vsock: vsock.into(), outer: app.into() }),
        }
    }

    fn abort_forward(&mut self, id: ForwardId) -> Result<AbortResult, ForwarderError> {
        self.forwarder()?.abort(id)
    }

    fn pair_state(&mut self, id: ForwardId) -> Result<Option<PairState>, ForwarderError> {
        self.forwarder()?.pair_state(id)
    }

    fn inventory(&mut self) -> Result<ForwarderInventory, ForwarderError> {
        self.forwarder()?.inventory()
    }

    fn clear_linger(&mut self, socket: &GuestPairSocket) -> io::Result<()> {
        let id = self.check(&socket.0);
        self.set_socket_linger(id, false)
    }

    fn abort(&mut self, socket: GuestOwnedSocket) {
        self.close_socket(&socket, true);
    }

    fn close(&mut self, socket: GuestOwnedSocket) {
        self.close_socket(&socket, false);
    }

    fn slot_owner(
        &mut self,
        slot: SocketAddrV4,
    ) -> io::Result<Option<(SocketCookie, SocketAddrV4)>> {
        let Some(index) = self.slot_index(slot) else { return Ok(None) };
        let bpf = self.ops.bpf.as_mut().ok_or_else(|| io::Error::from(ErrorKind::NotConnected))?;
        let metadata = Array::<_, [u8; 16]>::try_from(
            bpf.map_mut("SLOT_META")
                .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "SLOT_META is missing"))?,
        )
        .map_err(map_to_io)?;
        let value = metadata.get(&index, 0).map_err(map_to_io)?;
        let raw_cookie = u64::from_ne_bytes(value[0..8].try_into().unwrap_or([0; 8]));
        if raw_cookie == 0 {
            return Ok(None);
        }
        let ip = Ipv4Addr::from(
            u32::from_ne_bytes(value[8..12].try_into().unwrap_or([0; 4])).to_ne_bytes(),
        );
        let port = u16::from_ne_bytes(value[12..14].try_into().unwrap_or([0; 2]));
        let destination = SocketAddrV4::new(ip, port);
        let Some(source_port) = diagnostic_source_port(raw_cookie)? else { return Ok(None) };
        self.connect_slot_to_app(slot, source_port)?;
        Ok(Some((
            SocketCookie::new(raw_cookie)
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "SO_COOKIE returned zero"))?,
            destination,
        )))
    }

    fn write_slot_request(
        &mut self,
        vsock: &HostConn,
        request: &[u8; FLOW_REQUEST_LEN],
    ) -> io::Result<()> {
        let id = self.check(&vsock.0);
        if self.socket(id)?.connecting {
            return Err(io::Error::from(ErrorKind::NotConnected));
        }
        self.write_all(id, request)
    }

    fn attach_slot(
        &mut self,
        slot: SocketAddrV4,
        vsock: HostConn,
    ) -> Result<ForwardId, GuestSlotAttachRefused> {
        let Some(index) = self.slot_index(slot) else {
            panic!("owner passed an unknown UDP slot to attach_slot");
        };
        let vsock_id = self.check(&vsock.0);
        let udp = &self.ops.udp_slots[usize::try_from(index).unwrap_or(usize::MAX)];
        let udp_fd = udp as &dyn AsFd;
        let Some(host_socket_fd) =
            self.sockets.get(&vsock_id).map(|socket| &socket.fd as &dyn AsFd)
        else {
            return Err(GuestSlotAttachRefused {
                error: ForwarderError::NotConnected { side: PairSide::B },
                vsock,
            });
        };
        let forwarder = match self.forwarder() {
            Ok(forwarder) => forwarder,
            Err(error) => return Err(GuestSlotAttachRefused { error, vsock }),
        };
        match forwarder.attach(udp_fd, host_socket_fd, PairMode::Datagram { udp: PairSide::A }) {
            Ok(id) => {
                self.slot_pairs.insert(slot, id);
                self.sockets.remove(&vsock_id);
                Ok(id)
            }
            Err(error) => Err(GuestSlotAttachRefused { error, vsock }),
        }
    }

    fn end_association(&mut self, slot: SocketAddrV4) {
        self.slot_pairs.remove(&slot);
        self.slot_readiness_reported.remove(&slot);
        if let Some(index) = self.slot_index(slot)
            && let Some(udp) = self.ops.udp_slots.get(usize::try_from(index).unwrap_or(usize::MAX))
        {
            drain_udp(udp.as_raw_fd());
            disconnect_udp(udp.as_raw_fd());
        }
        self.connected_slots.remove(&slot);
    }

    fn release_slot(&mut self, slot: SocketAddrV4) {
        self.release_kernel_slot(slot);
    }

    fn slot_counters(&mut self, slot: SocketAddrV4) -> io::Result<(u64, u64)> {
        let Some(id) = self.slot_pairs.get(&slot).copied() else {
            return Ok((0, 0));
        };
        let state = self
            .forwarder()
            .map_err(io::Error::other)?
            .pair_state(id)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::from(ErrorKind::NotFound))?;
        Ok((state.a_to_b.moved, state.b_to_a.moved))
    }

    fn live_cookies(
        &mut self,
        cookies: &BTreeSet<SocketCookie>,
    ) -> io::Result<BTreeSet<SocketCookie>> {
        let live = diagnostic_cookies()?;
        Ok(cookies.iter().copied().filter(|cookie| live.contains(&cookie.get())).collect())
    }
}

impl KernelGuestOwnerIo {
    fn close_socket(&mut self, socket: &GuestOwnedSocket, reset: bool) {
        let handle = match socket {
            GuestOwnedSocket::Host(handle) => &handle.0,
            GuestOwnedSocket::Inbound(handle) => &handle.0,
            GuestOwnedSocket::Captured(handle) => &handle.0,
            GuestOwnedSocket::App(handle) => &handle.0,
            GuestOwnedSocket::Pair(handle) => &handle.0,
        };
        let id = self.check(handle);
        let Some((captured, fd)) = self
            .sockets
            .get(&id)
            .map(|record| (matches!(record.kind, SocketKind::Captured), record.fd.as_raw_fd()))
        else {
            return;
        };
        if captured {
            self.captured_total = self.captured_total.saturating_sub(1);
        }
        if reset {
            let _ = set_linger(fd, true);
        }
        self.sockets.remove(&id);
    }
}

fn take_guest_map(bpf: &mut aya::Ebpf, name: &str) -> Result<Map, aya::EbpfError> {
    bpf.take_map(name).ok_or_else(|| missing_guest_map(name))
}

fn missing_guest_map(name: &str) -> aya::EbpfError {
    aya::EbpfError::MapError(aya::maps::MapError::InvalidName { name: name.to_owned() })
}

const fn correlation_program_load_error(source: aya::EbpfError) -> GuestTransportSetupError {
    GuestTransportSetupError::ProgramLoad { program: GuestProgram::TcpCorrelation, source }
}

const fn read_request(record: GuestRecord) -> ReadRequest {
    let expected = match record {
        GuestRecord::ControlMessage => overdrive_core::guest_transport::wire::CONTROL_MESSAGE_LEN,
        GuestRecord::FlowRequest => FLOW_REQUEST_LEN,
    };
    ReadRequest { record, expected }
}

fn set_linger(fd: RawFd, reset: bool) -> io::Result<()> {
    let linger = libc::linger { l_onoff: i32::from(reset), l_linger: 0 };
    set_option(fd, libc::SOL_SOCKET, libc::SO_LINGER, &linger)
}

fn socket_tcp_state(fd: RawFd) -> io::Result<u8> {
    let mut info = unsafe { std::mem::zeroed::<libc::tcp_info>() };
    let mut length = libc::socklen_t::try_from(size_of::<libc::tcp_info>())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    // SAFETY: TCP_INFO writes no more than `length` bytes into the initialized
    // kernel-compatible structure.
    if unsafe {
        libc::getsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_INFO,
            std::ptr::from_mut(&mut info).cast(),
            &raw mut length,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    if length == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "TCP_INFO omitted its state"));
    }
    Ok(info.tcpi_state)
}

fn set_option_i32(fd: RawFd, level: i32, name: i32, value: i32) -> io::Result<()> {
    set_option(fd, level, name, &value)
}

fn set_option<T>(fd: RawFd, level: i32, name: i32, value: &T) -> io::Result<()> {
    let result = unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            std::ptr::from_ref(value).cast(),
            socklen(core::mem::size_of::<T>())?,
        )
    };
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

fn socket_cookie(fd: RawFd) -> io::Result<u64> {
    let mut cookie = 0_u64;
    let mut length = socklen(core::mem::size_of::<u64>())?;
    let result = unsafe {
        libc::getsockopt(fd, libc::SOL_SOCKET, SO_COOKIE, (&raw mut cookie).cast(), &raw mut length)
    };
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(cookie) }
}

fn socket_error(fd: RawFd) -> io::Result<i32> {
    let mut error = 0_i32;
    let mut length = socklen(core::mem::size_of::<i32>())?;
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            (&raw mut error).cast(),
            &raw mut length,
        )
    };
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(error) }
}

fn socket_hung_up(fd: RawFd) -> io::Result<bool> {
    let mut poll_fd = [PollFd::new(
        // SAFETY: the socket record owns this descriptor for this check.
        unsafe { BorrowedFd::borrow_raw(fd) },
        PollFlags::from_bits_retain(libc::POLLRDHUP) | PollFlags::POLLHUP | PollFlags::POLLERR,
    )];
    poll(&mut poll_fd, PollTimeout::ZERO).map_err(errno_to_io)?;
    Ok(poll_revents(&poll_fd[0]).intersects(
        PollFlags::from_bits_retain(libc::POLLRDHUP) | PollFlags::POLLHUP | PollFlags::POLLERR,
    ))
}

fn errno_to_io(error: Errno) -> io::Error {
    io::Error::from_raw_os_error(error as i32)
}

fn map_to_io(error: aya::maps::MapError) -> io::Error {
    io::Error::other(error)
}

const fn errno_kind(raw: i32) -> ErrorKind {
    match raw {
        libc::ECONNREFUSED => ErrorKind::ConnectionRefused,
        libc::ECONNRESET => ErrorKind::ConnectionReset,
        libc::ECONNABORTED => ErrorKind::ConnectionAborted,
        libc::ETIMEDOUT => ErrorKind::TimedOut,
        libc::EACCES | libc::EPERM => ErrorKind::PermissionDenied,
        libc::ENOENT => ErrorKind::NotFound,
        libc::EADDRINUSE => ErrorKind::AddrInUse,
        libc::EADDRNOTAVAIL => ErrorKind::AddrNotAvailable,
        libc::ENETUNREACH => ErrorKind::NetworkUnreachable,
        libc::EHOSTUNREACH => ErrorKind::HostUnreachable,
        libc::EPIPE => ErrorKind::BrokenPipe,
        libc::ENOTCONN => ErrorKind::NotConnected,
        libc::EAGAIN => ErrorKind::WouldBlock,
        _ => ErrorKind::Other,
    }
}

fn diagnostic_source_port(cookie: u64) -> io::Result<Option<u16>> {
    let entries = diagnostic_sockets()?;
    Ok(entries.iter().find_map(|(found, port)| (*found == cookie).then_some(*port)))
}

fn diagnostic_cookies() -> io::Result<BTreeSet<u64>> {
    Ok(diagnostic_sockets()?.into_iter().map(|(cookie, _)| cookie).collect())
}

fn diagnostic_sockets() -> io::Result<Vec<(u64, u16)>> {
    let fd = socket(
        AddressFamily::Netlink,
        SockType::Raw,
        SockFlag::SOCK_CLOEXEC,
        Some(SockProtocol::NetlinkSockDiag),
    )
    .map_err(errno_to_io)?;
    let mut found = Vec::new();
    let tcp_protocol = u8::try_from(libc::IPPROTO_TCP)
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    let udp_protocol = u8::try_from(libc::IPPROTO_UDP)
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    let inet_family = u8::try_from(libc::AF_INET)
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    for protocol in [tcp_protocol, udp_protocol] {
        let mut request = vec![0_u8; NETLINK_HEADER_LEN + 56];
        let length = u32::try_from(request.len()).unwrap_or(u32::MAX);
        request[0..4].copy_from_slice(&length.to_ne_bytes());
        request[4..6].copy_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
        request[6..8].copy_from_slice(&(1_u16 | 0x300).to_ne_bytes());
        request[8..12].copy_from_slice(&1_u32.to_ne_bytes());
        request[NETLINK_HEADER_LEN] = inet_family;
        request[NETLINK_HEADER_LEN + 1] = protocol;
        request[NETLINK_HEADER_LEN + 4..NETLINK_HEADER_LEN + 8]
            .copy_from_slice(&u32::MAX.to_ne_bytes());
        sendto(fd.as_raw_fd(), &request, &NetlinkAddr::new(0, 0), MsgFlags::empty())
            .map_err(errno_to_io)?;
        loop {
            let mut response = vec![0_u8; 32 * 1024];
            let count =
                recv(fd.as_raw_fd(), &mut response, MsgFlags::empty()).map_err(errno_to_io)?;
            let mut offset = 0;
            let mut done = false;
            while offset + NETLINK_HEADER_LEN <= count {
                let length = usize::try_from(u32::from_ne_bytes(
                    response[offset..offset + 4].try_into().unwrap_or([0; 4]),
                ))
                .unwrap_or(0);
                if length < NETLINK_HEADER_LEN || offset + length > count {
                    return Err(io::Error::new(
                        ErrorKind::InvalidData,
                        "malformed socket diagnostic message",
                    ));
                }
                let kind = u16::from_ne_bytes([response[offset + 4], response[offset + 5]]);
                if kind == 3 {
                    done = true;
                } else if kind == 2 {
                    let error = i32::from_ne_bytes(
                        response[offset + NETLINK_HEADER_LEN..offset + NETLINK_HEADER_LEN + 4]
                            .try_into()
                            .unwrap_or([0; 4]),
                    );
                    if error != 0 {
                        return Err(io::Error::from_raw_os_error(-error));
                    }
                } else if kind == SOCK_DIAG_BY_FAMILY
                    && length >= NETLINK_HEADER_LEN + INET_DIAG_MSG_LEN
                {
                    let payload = offset + NETLINK_HEADER_LEN;
                    let source_port =
                        u16::from_be_bytes([response[payload + 4], response[payload + 5]]);
                    let cookie_low = u32::from_ne_bytes(
                        response[payload + 44..payload + 48].try_into().unwrap_or([0; 4]),
                    );
                    let cookie_high = u32::from_ne_bytes(
                        response[payload + 48..payload + 52].try_into().unwrap_or([0; 4]),
                    );
                    found.push((
                        u64::from(cookie_low) | (u64::from(cookie_high) << 32),
                        source_port,
                    ));
                }
                offset += (length + 3) & !3;
            }
            if done {
                break;
            }
        }
    }
    Ok(found)
}

fn drain_udp(fd: RawFd) {
    loop {
        let received = unsafe {
            libc::recv(fd, core::ptr::null_mut(), 0, libc::MSG_DONTWAIT | libc::MSG_TRUNC)
        };
        if received < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == ErrorKind::Interrupted {
                continue;
            }
            return;
        }
    }
}

fn disconnect_udp(fd: RawFd) {
    let address = libc::sockaddr { sa_family: 0, sa_data: [0; 14] };
    let Ok(length) = socklen(core::mem::size_of::<libc::sockaddr>()) else {
        return;
    };
    let _ = unsafe { libc::connect(fd, (&raw const address).cast(), length) };
}

fn socklen(length: usize) -> io::Result<libc::socklen_t> {
    libc::socklen_t::try_from(length)
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))
}
