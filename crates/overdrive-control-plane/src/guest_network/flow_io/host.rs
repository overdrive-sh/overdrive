//! Production socket and forwarder adapter for the host guest-flow owner.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ipnet::Ipv4Net;
use nix::sys::socket::{MsgFlags, sockopt};
use overdrive_core::guest_transport::dial::{GuestStream, GuestStreamEnd, GuestStreamIo};
use overdrive_core::guest_transport::forwarder::{
    AbortResult, FlowForwarder, ForwardEnd, ForwardId, ForwardOutcome, ForwarderError,
    ForwarderGeneration, ForwarderOp, PairMode,
};
use overdrive_core::guest_transport::wire::FlowId;
use overdrive_core::guest_transport::{GUEST_INBOUND_PORT, GuestCid, VsockListenPort};
use overdrive_core::net::InterfaceIndex;
use overdrive_core::vm::config::VsockPort;
use overdrive_fwd::KernelFlowForwarder;
use overdrive_netlink::Client;
use parking_lot::Mutex;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tokio::io::unix::AsyncFd;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::{
    AttachRefused, AttachedPair, FlowIoError, FlowListener, GuestFlowIo, GuestHostFacts,
    HostRouteType, OwnedSocket, PairSocket, PeerProtocol, PeerSocket, PeerSocketSpec, RecordSocket,
    SeamHandle,
};
use crate::guest_network::{GuestConnKey, GuestNetworkError, GuestNetworkOperation};

static NEXT_SEAM: AtomicU64 = AtomicU64::new(1);

/// The production adapter holds every socket the owner can name and its one
/// forwarder context. Construction is inert; effects start at phase one.
pub(crate) struct HostGuestFlowIo {
    inner: Arc<HostShared>,
}

struct HostShared {
    seam: u64,
    state: Mutex<HostState>,
    stream_changed: Notify,
    forwarder_changed: Notify,
}

struct HostState {
    next_handle: u64,
    last_generation: Option<ForwarderGeneration>,
    handles: HashMap<u64, SocketEntry>,
    listeners: BTreeMap<FlowListener, ListenerEntry>,
    forwarder: Option<Arc<ForwarderSlot>>,
    retired_forwarders: Vec<Arc<ForwarderSlot>>,
    watches: HashMap<(ForwarderGeneration, ForwardId), StreamResolution>,
    netlink: Option<Arc<Client>>,
}

struct SocketEntry {
    socket: Arc<AsyncFd<Socket>>,
    kind: SocketKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SocketKind {
    Record,
    Peer(PeerProtocol),
    PairVsock,
    PairPeer(PeerProtocol),
}

#[derive(Debug, Clone, Copy)]
enum OwnedKind {
    Record,
    Peer,
    Pair,
}

#[derive(Clone)]
struct ListenerEntry {
    socket: Arc<AsyncFd<Socket>>,
    port: VsockPort,
}

struct ForwarderSlot {
    context: Arc<KernelFlowForwarder>,
    readiness: AsyncFd<ForwarderReadiness>,
    generation: ForwarderGeneration,
    shutdown: CancellationToken,
}

struct ForwarderReadiness(Arc<KernelFlowForwarder>);

impl AsRawFd for ForwarderReadiness {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.0.readiness_fd().as_raw_fd()
    }
}

#[derive(Debug, Clone, Copy)]
enum StreamResolution {
    Pending,
    OwnerAbort,
    Record(ForwardOutcome),
}

impl HostGuestFlowIo {
    /// Creates an empty handle table without touching kernel state.
    pub(crate) fn new() -> Self {
        let seam = NEXT_SEAM.fetch_add(1, Ordering::Relaxed);
        assert_ne!(seam, 0, "host guest-flow seam id exhausted");
        Self {
            inner: Arc::new(HostShared {
                seam,
                state: Mutex::new(HostState {
                    next_handle: 1,
                    last_generation: None,
                    handles: HashMap::new(),
                    listeners: BTreeMap::new(),
                    forwarder: None,
                    retired_forwarders: Vec::new(),
                    watches: HashMap::new(),
                    netlink: None,
                }),
                stream_changed: Notify::new(),
                forwarder_changed: Notify::new(),
            }),
        }
    }

    fn handle(&self, handle: SeamHandle, expected: SocketKind) -> Arc<AsyncFd<Socket>> {
        if handle.seam != self.inner.seam {
            unreachable!("a handle from another guest-flow seam reached this adapter");
        }
        let state = self.inner.state.lock();
        let Some(entry) = state.handles.get(&handle.id) else {
            unreachable!("a consumed or closed guest-flow handle reached this adapter");
        };
        if entry.kind != expected {
            unreachable!("guest-flow handle stage does not match the operation");
        }
        let socket = Arc::clone(&entry.socket);
        drop(state);
        socket
    }

    fn peer_socket(&self, handle: SeamHandle) -> Arc<AsyncFd<Socket>> {
        if handle.seam != self.inner.seam {
            unreachable!("a peer from another guest-flow seam reached this adapter");
        }
        let state = self.inner.state.lock();
        let Some(entry) = state.handles.get(&handle.id) else {
            unreachable!("a consumed or closed peer reached this adapter");
        };
        if !matches!(entry.kind, SocketKind::Peer(_)) {
            unreachable!("a non-peer handle reached a peer operation");
        }
        let socket = Arc::clone(&entry.socket);
        drop(state);
        socket
    }

    fn insert_handle(&self, socket: Socket, kind: SocketKind) -> Result<SeamHandle, FlowIoError> {
        let socket = async_socket(socket).map_err(flow_io)?;
        let mut state = self.inner.state.lock();
        let id = state.next_handle;
        state.next_handle = state
            .next_handle
            .checked_add(1)
            .unwrap_or_else(|| unreachable!("guest-flow seam handle id exhausted"));
        state.handles.insert(id, SocketEntry { socket, kind });
        drop(state);
        Ok(SeamHandle { seam: self.inner.seam, id })
    }

    fn forwarder(&self) -> Arc<ForwarderSlot> {
        self.inner
            .state
            .lock()
            .forwarder
            .as_ref()
            .map_or_else(|| unreachable!("forwarding effect before open_forwarder"), Arc::clone)
    }

    fn netlink_client(&self) -> Result<Arc<Client>, overdrive_netlink::NetlinkError> {
        let mut state = self.inner.state.lock();
        if let Some(client) = &state.netlink {
            return Ok(Arc::clone(client));
        }
        let client = Arc::new(Client::new()?);
        state.netlink = Some(Arc::clone(&client));
        drop(state);
        Ok(client)
    }
}

impl Default for HostGuestFlowIo {
    fn default() -> Self {
        Self::new()
    }
}

fn async_socket(socket: Socket) -> io::Result<Arc<AsyncFd<Socket>>> {
    AsyncFd::new(socket).map(Arc::new)
}

fn flow_io(source: io::Error) -> FlowIoError {
    FlowIoError::Io { source }
}

fn operation_io(operation: GuestNetworkOperation, source: io::Error) -> GuestNetworkError {
    GuestNetworkError::Io { operation, source }
}

fn forwarder_io(operation: ForwarderOp, source: io::Error) -> ForwarderError {
    ForwarderError::Io { op: operation, source }
}

fn is_connect_pending(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EINPROGRESS | libc::EALREADY | libc::EWOULDBLOCK | libc::EINTR)
    ) || error.kind() == io::ErrorKind::WouldBlock
}

fn socket_for_vsock() -> io::Result<Socket> {
    #[cfg(target_os = "linux")]
    {
        let socket = Socket::new(Domain::VSOCK, Type::STREAM.nonblocking().cloexec(), None)?;
        Ok(socket)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(io::Error::new(io::ErrorKind::Unsupported, "AF_VSOCK is available only on Linux"))
    }
}

fn set_abortive_linger(socket: &Socket) -> io::Result<()> {
    socket.set_linger(Some(Duration::ZERO))
}

#[cfg(target_os = "linux")]
fn set_no_check(socket: &Socket) -> io::Result<()> {
    // SAFETY: the socket is a live UDP socket, `value` is a valid `c_int`,
    // and the kernel reads exactly its size for `SO_NO_CHECK`.
    #[allow(unsafe_code)]
    unsafe {
        let value: libc::c_int = 1;
        let value_len =
            libc::socklen_t::try_from(std::mem::size_of_val(&value)).map_err(io::Error::other)?;
        let result = libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NO_CHECK,
            (&raw const value).cast(),
            value_len,
        );
        if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }
}

#[cfg(not(target_os = "linux"))]
fn set_no_check(_socket: &Socket) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "SO_NO_CHECK is available only on Linux"))
}

async fn finish_nonblocking_connect(socket: &AsyncFd<Socket>) -> io::Result<()> {
    let ready = socket.writable().await?;
    drop(ready);
    if let Some(error) = socket.get_ref().take_error()? {
        return Err(error);
    }
    Ok(())
}

fn current_cgroup_path(contents: &str) -> io::Result<PathBuf> {
    let relative = contents.lines().find_map(|line| line.strip_prefix("0::")).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "`/proc/self/cgroup` has no cgroup-v2 hierarchy entry",
        )
    })?;
    Ok(PathBuf::from("/sys/fs/cgroup").join(relative.trim().trim_start_matches('/')))
}

fn interface_facts() -> io::Result<BTreeMap<InterfaceIndex, BTreeSet<Ipv4Net>>> {
    let mut interfaces = BTreeMap::<InterfaceIndex, BTreeSet<Ipv4Net>>::new();
    let addresses = nix::ifaddrs::getifaddrs().map_err(io::Error::other)?;
    for address in addresses {
        let (Some(sockaddr), Some(netmask)) = (address.address, address.netmask) else {
            continue;
        };
        let (Some(sockaddr), Some(netmask)) = (sockaddr.as_sockaddr_in(), netmask.as_sockaddr_in())
        else {
            continue;
        };
        let raw_mask = u32::from(netmask.ip());
        let prefix = u8::try_from(raw_mask.leading_ones()).map_err(io::Error::other)?;
        let expected_mask = if prefix == 0 { 0 } else { u32::MAX << (32 - u32::from(prefix)) };
        if raw_mask != expected_mask {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "interface {} returned a non-contiguous IPv4 netmask",
                    address.interface_name
                ),
            ));
        }
        let index = nix::net::if_::if_nametoindex(address.interface_name.as_str())
            .map_err(io::Error::other)?;
        let index = InterfaceIndex::new(index).map_err(io::Error::other)?;
        let network = Ipv4Net::new(sockaddr.ip(), prefix).map_err(io::Error::other)?;
        interfaces.entry(index).or_default().insert(network);
    }
    Ok(interfaces)
}

#[async_trait::async_trait]
impl GuestFlowIo for HostGuestFlowIo {
    async fn host_facts(&self) -> Result<GuestHostFacts, GuestNetworkError> {
        let interfaces = tokio::task::spawn_blocking(interface_facts)
            .await
            .map_err(|error| {
                operation_io(GuestNetworkOperation::StartupProbe, io::Error::other(error))
            })?
            .map_err(|source| operation_io(GuestNetworkOperation::StartupProbe, source))?;
        let cgroup = tokio::fs::read_to_string("/proc/self/cgroup")
            .await
            .map_err(|source| operation_io(GuestNetworkOperation::StartupProbe, source))?;
        let owner_cgroup = current_cgroup_path(&cgroup)
            .map_err(|source| operation_io(GuestNetworkOperation::StartupProbe, source))?;
        let nofile_limit = rustix::process::getrlimit(rustix::process::Resource::Nofile)
            .current
            .unwrap_or(u64::MAX);
        Ok(GuestHostFacts { interfaces, nofile_limit, owner_cgroup })
    }

    async fn probe_forwarder(&self) -> Result<(), ForwarderError> {
        let context = KernelFlowForwarder::open()?;
        context.probe()
    }

    async fn open_forwarder(&self) -> Result<ForwarderGeneration, ForwarderError> {
        self.close_forwarder();
        self.await_retired_forwarders().await;

        let context = Arc::new(KernelFlowForwarder::open()?);
        context.probe()?;
        let readiness = AsyncFd::new(ForwarderReadiness(Arc::clone(&context)))
            .map_err(|source| forwarder_io(ForwarderOp::Open, source))?;
        let mut state = self.inner.state.lock();
        let generation = match state.last_generation {
            Some(previous) => previous.next().ok_or_else(|| {
                forwarder_io(
                    ForwarderOp::Open,
                    io::Error::new(io::ErrorKind::InvalidInput, "forwarder generation exhausted"),
                )
            })?,
            None => ForwarderGeneration::FIRST,
        };
        state.last_generation = Some(generation);
        state.forwarder = Some(Arc::new(ForwarderSlot {
            context,
            readiness,
            generation,
            shutdown: CancellationToken::new(),
        }));
        drop(state);
        self.inner.forwarder_changed.notify_waiters();
        Ok(generation)
    }

    fn close_forwarder(&self) {
        let mut state = self.inner.state.lock();
        if let Some(old) = state.forwarder.take() {
            old.shutdown.cancel();
            state.retired_forwarders.push(old);
        }
        drop(state);
        self.inner.forwarder_changed.notify_waiters();
    }

    async fn bind_listener(
        &self,
        listener: FlowListener,
        port: VsockListenPort,
    ) -> Result<VsockPort, FlowIoError> {
        let existing = self.inner.state.lock().listeners.get(&listener).cloned();
        if let Some(existing) = existing {
            let listening =
                nix::sys::socket::getsockopt(existing.socket.get_ref(), sockopt::AcceptConn)
                    .map_err(|error| flow_io(io::Error::from(error)))?;
            if listening {
                return Ok(existing.port);
            }
            return Err(flow_io(io::Error::other(
                "a held guest-flow listener is no longer listening",
            )));
        }

        #[cfg(target_os = "linux")]
        {
            let socket = socket_for_vsock().map_err(flow_io)?;
            let raw_port = match port {
                VsockListenPort::Fixed(port) => port.as_u32(),
                VsockListenPort::KernelAssigned => u32::MAX,
            };
            socket.bind(&SockAddr::vsock(u32::MAX, raw_port)).map_err(flow_io)?;
            socket.listen(i32::MAX).map_err(flow_io)?;
            let Some((_, bound_port)) = socket.local_addr().map_err(flow_io)?.as_vsock_address()
            else {
                unreachable!("AF_VSOCK getsockname returned another address family");
            };
            let bound_port = VsockPort::new(bound_port);
            let socket = async_socket(socket).map_err(flow_io)?;
            self.inner
                .state
                .lock()
                .listeners
                .insert(listener, ListenerEntry { socket, port: bound_port });
            Ok(bound_port)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (listener, port);
            Err(flow_io(io::Error::new(
                io::ErrorKind::Unsupported,
                "AF_VSOCK listeners are available only on Linux",
            )))
        }
    }

    async fn converge_gateway_route(&self, gateway: Ipv4Addr) -> Result<(), GuestNetworkError> {
        let client = self.netlink_client().map_err(|source| GuestNetworkError::Netlink {
            operation: GuestNetworkOperation::LocalRouteConverge,
            source,
        })?;
        client
            .converge_ipv4_host_local_route(
                gateway,
                overdrive_core::guest_transport::GUEST_PREFIX_ROUTE_PROTOCOL,
            )
            .await
            .map_err(|source| GuestNetworkError::Netlink {
                operation: GuestNetworkOperation::LocalRouteConverge,
                source,
            })
    }

    async fn stale_objects_present(&self) -> Result<bool, GuestNetworkError> {
        let state = self.inner.state.lock();
        Ok(!state.listeners.is_empty() || !state.handles.is_empty() || state.forwarder.is_some())
    }

    async fn accept(
        &self,
        listener: FlowListener,
    ) -> Result<(RecordSocket, GuestConnKey), FlowIoError> {
        loop {
            let held = self
                .inner
                .state
                .lock()
                .listeners
                .get(&listener)
                .cloned()
                .unwrap_or_else(|| unreachable!("accept called before bind_listener"));
            let mut ready = held.socket.readable().await.map_err(flow_io)?;
            let accepted = ready.try_io(|listener| accept_nonblocking(listener.get_ref()));
            let (socket, peer) = match accepted {
                Ok(Ok(accepted)) => accepted,
                Ok(Err(source)) => return Err(flow_io(source)),
                Err(_would_block) => continue,
            };
            let Some((raw_cid, raw_port)) = peer.as_vsock_address() else {
                unreachable!("AF_VSOCK accept returned another address family");
            };
            let Ok(peer_cid) = GuestCid::new(raw_cid) else {
                drop(socket);
                continue;
            };
            let handle = self.insert_handle(socket, SocketKind::Record)?;
            return Ok((
                RecordSocket(handle),
                GuestConnKey { listener: held.port, peer_cid, peer_port: VsockPort::new(raw_port) },
            ));
        }
    }

    async fn read_record(&self, socket: &RecordSocket, max: usize) -> Result<Vec<u8>, FlowIoError> {
        if max == 0 {
            return Err(flow_io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "read_record requires max >= 1",
            )));
        }
        let socket = self.handle(socket.0, SocketKind::Record);
        let mut bytes = vec![0; max];
        loop {
            let mut ready = socket.readable().await.map_err(flow_io)?;
            match ready.try_io(|socket| {
                nix::sys::socket::recv(
                    socket.get_ref().as_raw_fd(),
                    &mut bytes,
                    MsgFlags::MSG_DONTWAIT,
                )
                .map_err(io::Error::from)
            }) {
                Ok(Ok(length)) => {
                    bytes.truncate(length);
                    return Ok(bytes);
                }
                Ok(Err(source)) => return Err(flow_io(source)),
                Err(_would_block) => {}
            }
        }
    }

    async fn write_record(&self, socket: &RecordSocket, bytes: &[u8]) -> Result<(), FlowIoError> {
        let socket = self.handle(socket.0, SocketKind::Record);
        let mut written = 0;
        while written < bytes.len() {
            let mut ready = socket.writable().await.map_err(flow_io)?;
            match ready.try_io(|socket| {
                nix::sys::socket::send(
                    socket.get_ref().as_raw_fd(),
                    &bytes[written..],
                    MsgFlags::MSG_DONTWAIT | MsgFlags::MSG_NOSIGNAL,
                )
                .map_err(io::Error::from)
            }) {
                Ok(Ok(0)) => {
                    return Err(flow_io(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "record write made no progress",
                    )));
                }
                Ok(Ok(length)) => written += length,
                Ok(Err(source)) => return Err(flow_io(source)),
                Err(_would_block) => {}
            }
        }
        Ok(())
    }

    async fn connect_guest(&self, cid: GuestCid) -> Result<RecordSocket, FlowIoError> {
        #[cfg(target_os = "linux")]
        {
            let socket = socket_for_vsock().map_err(flow_io)?;
            let socket = async_socket(socket).map_err(flow_io)?;
            let address = SockAddr::vsock(cid.get(), GUEST_INBOUND_PORT.as_u32());
            match socket.get_ref().connect(&address) {
                Ok(()) => {}
                Err(error) if is_connect_pending(&error) => {
                    finish_nonblocking_connect(&socket).await.map_err(flow_io)?;
                }
                Err(error) => return Err(flow_io(error)),
            }
            let handle = {
                let mut state = self.inner.state.lock();
                let id = state.next_handle;
                state.next_handle = state
                    .next_handle
                    .checked_add(1)
                    .unwrap_or_else(|| unreachable!("guest-flow seam handle id exhausted"));
                state.handles.insert(id, SocketEntry { socket, kind: SocketKind::Record });
                drop(state);
                SeamHandle { seam: self.inner.seam, id }
            };
            Ok(RecordSocket(handle))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = cid;
            Err(flow_io(io::Error::new(
                io::ErrorKind::Unsupported,
                "AF_VSOCK is available only on Linux",
            )))
        }
    }

    fn watch(&self, vsock: &RecordSocket) -> Result<ForwardId, ForwarderError> {
        let socket = self.handle(vsock.0, SocketKind::Record);
        let forwarder = self.forwarder();
        let id = forwarder.context.watch(socket.get_ref())?;
        self.inner
            .state
            .lock()
            .watches
            .insert((forwarder.generation, id), StreamResolution::Pending);
        Ok(id)
    }

    fn into_guest_stream(
        &self,
        vsock: RecordSocket,
        watch: ForwardId,
        flow: FlowId,
        target: SocketAddrV4,
    ) -> GuestStream {
        if vsock.0.seam != self.inner.seam {
            unreachable!("a V_h from another guest-flow seam reached this adapter");
        }
        let socket = {
            let mut state = self.inner.state.lock();
            let forwarder = state
                .forwarder
                .as_ref()
                .unwrap_or_else(|| unreachable!("guest stream created without a context"));
            let generation = forwarder.generation;
            if !state.watches.contains_key(&(generation, watch)) {
                unreachable!("guest stream created without its forwarder watch");
            }
            let Some(entry) = state.handles.remove(&vsock.0.id) else {
                unreachable!("consumed V_h is missing from the seam handle table");
            };
            if entry.kind != SocketKind::Record {
                unreachable!("guest stream received a non-record V_h");
            }
            let socket = (entry.socket, generation);
            drop(state);
            socket
        };
        GuestStream::new(
            target,
            Box::new(HostGuestStream {
                socket: socket.0,
                shared: Arc::clone(&self.inner),
                generation: socket.1,
                watch,
                flow,
                host_shut: false,
            }),
        )
    }

    async fn open_peer(&self, spec: PeerSocketSpec) -> Result<PeerSocket, FlowIoError> {
        let (ty, protocol) = match spec.protocol {
            PeerProtocol::Tcp => (Type::STREAM, Protocol::TCP),
            PeerProtocol::Udp => (Type::DGRAM, Protocol::UDP),
        };
        let socket = Socket::new(Domain::IPV4, ty.nonblocking().cloexec(), Some(protocol))
            .map_err(flow_io)?;
        if spec.owner_egress_mark {
            #[cfg(target_os = "linux")]
            nix::sys::socket::setsockopt(
                &socket,
                sockopt::Mark,
                &overdrive_core::dataplane::mtls_mark::OWNER_EGRESS_MARK,
            )
            .map_err(|error| flow_io(io::Error::from(error)))?;
            #[cfg(not(target_os = "linux"))]
            return Err(flow_io(io::Error::new(
                io::ErrorKind::Unsupported,
                "SO_MARK is available only on Linux",
            )));
        }
        if spec.abortive_linger {
            set_abortive_linger(&socket).map_err(flow_io)?;
        }
        if spec.protocol == PeerProtocol::Udp {
            set_no_check(&socket).map_err(flow_io)?;
        }
        if let Some(bind) = spec.bind {
            socket.bind(&SockAddr::from(SocketAddr::V4(bind))).map_err(flow_io)?;
        }
        Ok(PeerSocket(self.insert_handle(socket, SocketKind::Peer(spec.protocol))?))
    }

    fn local_addr(&self, peer: &PeerSocket) -> Result<SocketAddrV4, FlowIoError> {
        let socket = self.peer_socket(peer.0);
        match socket.get_ref().local_addr().map_err(flow_io)?.as_socket() {
            Some(SocketAddr::V4(address)) => Ok(address),
            _ => unreachable!("an IPv4 peer returned a non-IPv4 local address"),
        }
    }

    fn peer_addr(&self, peer: &PeerSocket) -> Result<SocketAddrV4, FlowIoError> {
        let socket = self.peer_socket(peer.0);
        match socket.get_ref().peer_addr().map_err(flow_io)?.as_socket() {
            Some(SocketAddr::V4(address)) => Ok(address),
            _ => unreachable!("an IPv4 peer returned a non-IPv4 peer address"),
        }
    }

    async fn connect_udp(
        &self,
        peer: &PeerSocket,
        destination: SocketAddrV4,
    ) -> Result<(), FlowIoError> {
        let socket = self.handle(peer.0, SocketKind::Peer(PeerProtocol::Udp));
        socket.get_ref().connect(&SockAddr::from(SocketAddr::V4(destination))).map_err(flow_io)
    }

    async fn connect_peer(
        &self,
        peer: &PeerSocket,
        destination: SocketAddrV4,
    ) -> Result<(), FlowIoError> {
        let socket = self.handle(peer.0, SocketKind::Peer(PeerProtocol::Tcp));
        match socket.get_ref().connect(&SockAddr::from(SocketAddr::V4(destination))) {
            Ok(()) => Ok(()),
            Err(error) if is_connect_pending(&error) => {
                finish_nonblocking_connect(&socket).await.map_err(flow_io)
            }
            Err(error) => Err(flow_io(error)),
        }
    }

    async fn route_type(
        &self,
        destination: Ipv4Addr,
    ) -> Result<HostRouteType, overdrive_netlink::NetlinkError> {
        let client = self.netlink_client()?;
        client.ipv4_fib_route_type(destination).await
    }

    fn attach(
        &self,
        vsock: RecordSocket,
        peer: PeerSocket,
        mode: PairMode,
    ) -> Result<AttachedPair, AttachRefused> {
        let expected_peer_kind = match mode {
            PairMode::Stream => SocketKind::Peer(PeerProtocol::Tcp),
            PairMode::Datagram { .. } => SocketKind::Peer(PeerProtocol::Udp),
        };
        let (vsock_fd, peer_fd) =
            (self.handle(vsock.0, SocketKind::Record), self.handle(peer.0, expected_peer_kind));
        let forwarder = self.forwarder();
        let id = match forwarder.context.attach(vsock_fd.get_ref(), peer_fd.get_ref(), mode) {
            Ok(id) => id,
            Err(error) => return Err(AttachRefused { error, vsock, peer }),
        };
        let pair_peer_kind = match mode {
            PairMode::Stream => SocketKind::PairPeer(PeerProtocol::Tcp),
            PairMode::Datagram { .. } => SocketKind::PairPeer(PeerProtocol::Udp),
        };
        {
            let mut state = self.inner.state.lock();
            let Some(vsock_entry) = state.handles.get_mut(&vsock.0.id) else {
                unreachable!("attached V_h disappeared from the seam handle table");
            };
            vsock_entry.kind = SocketKind::PairVsock;
            let Some(peer_entry) = state.handles.get_mut(&peer.0.id) else {
                unreachable!("attached peer disappeared from the seam handle table");
            };
            peer_entry.kind = pair_peer_kind;
            drop(state);
        }
        Ok(AttachedPair { id, vsock: PairSocket(vsock.0), peer: PairSocket(peer.0) })
    }

    fn abort_forward(&self, id: ForwardId) -> Result<AbortResult, ForwarderError> {
        let forwarder = self.forwarder();
        let result = forwarder.context.abort(id)?;
        if result == AbortResult::Aborted {
            let mut state = self.inner.state.lock();
            if let Some(resolution) = state.watches.get_mut(&(forwarder.generation, id)) {
                *resolution = StreamResolution::OwnerAbort;
                self.inner.stream_changed.notify_waiters();
            }
        }
        Ok(result)
    }

    fn abort_all(&self) -> Result<u32, ForwarderError> {
        let forwarder = self.forwarder();
        let aborted = forwarder.context.abort_all()?;
        let mut state = self.inner.state.lock();
        for ((generation, _), resolution) in &mut state.watches {
            if *generation == forwarder.generation {
                *resolution = StreamResolution::OwnerAbort;
            }
        }
        self.inner.stream_changed.notify_waiters();
        drop(state);
        Ok(aborted)
    }

    async fn next_end(&self) -> Result<ForwardEnd, ForwarderError> {
        loop {
            let changed = self.inner.forwarder_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let forwarder = self.inner.state.lock().forwarder.as_ref().map(Arc::clone);
            let Some(forwarder) = forwarder else {
                changed.await;
                continue;
            };
            let mut ready = tokio::select! {
                biased;
                () = &mut changed => continue,
                () = forwarder.shutdown.cancelled() => {
                    drop(forwarder);
                    self.inner.forwarder_changed.notify_waiters();
                    continue;
                }
                result = forwarder.readiness.readable() => {
                    result.map_err(|source| forwarder_io(ForwarderOp::Read, source))?
                }
            };
            match forwarder.context.take_end() {
                Ok(Some(end)) => {
                    let mut state = self.inner.state.lock();
                    if let Some(resolution) = state.watches.get_mut(&(forwarder.generation, end.id))
                        && !matches!(resolution, StreamResolution::OwnerAbort)
                    {
                        *resolution = StreamResolution::Record(end.outcome);
                    }
                    self.inner.stream_changed.notify_waiters();
                    drop(state);
                    return Ok(end);
                }
                Ok(None) => ready.clear_ready(),
                Err(ForwarderError::Io { op, source })
                    if source.kind() == io::ErrorKind::WouldBlock =>
                {
                    ready.clear_ready();
                    let _ = op;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn pair_state(
        &self,
        id: ForwardId,
    ) -> Result<Option<overdrive_core::guest_transport::forwarder::PairState>, ForwarderError> {
        self.forwarder().context.pair_state(id)
    }

    fn inventory(
        &self,
    ) -> Result<overdrive_core::guest_transport::forwarder::ForwarderInventory, ForwarderError>
    {
        self.forwarder().context.inventory()
    }

    async fn clear_linger(&self, socket: &PairSocket) -> Result<(), FlowIoError> {
        let kind = self.pair_socket_kind(socket.0);
        if matches!(kind, SocketKind::PairVsock) {
            unreachable!("clear_linger called for the vsock half");
        }
        self.handle(socket.0, kind).get_ref().set_linger(None).map_err(flow_io)
    }

    async fn abort(&self, socket: OwnedSocket) {
        let (handle, kind) = owned_socket_parts(socket);
        self.close_owned_kind(handle, kind, true);
    }

    async fn close(&self, socket: OwnedSocket) {
        let (handle, kind) = owned_socket_parts(socket);
        self.close_owned_kind(handle, kind, false);
    }
}

impl HostGuestFlowIo {
    async fn await_retired_forwarders(&self) {
        loop {
            let changed = self.inner.forwarder_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let retired = {
                let mut state = self.inner.state.lock();
                if state.retired_forwarders.iter().all(|old| Arc::strong_count(old) == 1) {
                    Some(std::mem::take(&mut state.retired_forwarders))
                } else {
                    None
                }
            };
            if let Some(retired) = retired {
                drop(retired);
                return;
            }
            changed.await;
        }
    }

    fn pair_socket_kind(&self, handle: SeamHandle) -> SocketKind {
        if handle.seam != self.inner.seam {
            unreachable!("a pair from another guest-flow seam reached this adapter");
        }
        self.inner.state.lock().handles.get(&handle.id).map_or_else(
            || unreachable!("consumed or closed pair reached this adapter"),
            |entry| entry.kind,
        )
    }

    fn close_owned_kind(&self, handle: SeamHandle, expected: OwnedKind, abort: bool) {
        if handle.seam != self.inner.seam {
            unreachable!("a handle from another guest-flow seam reached this adapter");
        }
        let entry = self
            .inner
            .state
            .lock()
            .handles
            .remove(&handle.id)
            .unwrap_or_else(|| unreachable!("consumed or closed handle reached this adapter"));
        let matches_expected = match expected {
            OwnedKind::Record => entry.kind == SocketKind::Record,
            OwnedKind::Peer => matches!(entry.kind, SocketKind::Peer(_)),
            OwnedKind::Pair => {
                matches!(entry.kind, SocketKind::PairVsock | SocketKind::PairPeer(_))
            }
        };
        if !matches_expected {
            unreachable!("closed guest-flow handle stage does not match the owner operation");
        }
        if abort {
            match entry.kind {
                SocketKind::Record | SocketKind::PairVsock => {
                    let _ = entry.socket.get_ref().shutdown(std::net::Shutdown::Both);
                }
                SocketKind::Peer(PeerProtocol::Tcp | PeerProtocol::Udp)
                | SocketKind::PairPeer(PeerProtocol::Tcp | PeerProtocol::Udp) => {
                    let _ = set_abortive_linger(entry.socket.get_ref());
                }
            }
        }
        drop(entry);
    }
}

fn owned_socket_parts(socket: OwnedSocket) -> (SeamHandle, OwnedKind) {
    match socket {
        OwnedSocket::Record(socket) => (socket.0, OwnedKind::Record),
        OwnedSocket::Peer(socket) => (socket.0, OwnedKind::Peer),
        OwnedSocket::Pair(socket) => (socket.0, OwnedKind::Pair),
    }
}

#[cfg(target_os = "linux")]
fn accept_nonblocking(listener: &Socket) -> io::Result<(Socket, SockAddr)> {
    listener.accept4(libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC)
}

#[cfg(not(target_os = "linux"))]
fn accept_nonblocking(listener: &Socket) -> io::Result<(Socket, SockAddr)> {
    let (socket, peer) = listener.accept()?;
    socket.set_nonblocking(true)?;
    Ok((socket, peer))
}

struct HostGuestStream {
    socket: Arc<AsyncFd<Socket>>,
    shared: Arc<HostShared>,
    generation: ForwarderGeneration,
    watch: ForwardId,
    flow: FlowId,
    host_shut: bool,
}

impl std::fmt::Debug for HostGuestStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostGuestStream")
            .field("flow", &self.flow)
            .field("watch", &self.watch)
            .finish_non_exhaustive()
    }
}

impl HostGuestStream {
    fn resolution(&self) -> StreamResolution {
        self.shared
            .state
            .lock()
            .watches
            .get(&(self.generation, self.watch))
            .copied()
            .unwrap_or(StreamResolution::Pending)
    }

    async fn wait_for_resolution(&self) -> StreamResolution {
        loop {
            let changed = self.shared.stream_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let resolution = self.resolution();
            if !matches!(resolution, StreamResolution::Pending) {
                return resolution;
            }
            changed.await;
        }
    }

    fn read_end(&self) -> Option<io::Result<usize>> {
        match self.resolution() {
            StreamResolution::OwnerAbort => Some(Err(connection_aborted())),
            StreamResolution::Record(
                ForwardOutcome::GuestHalfClose | ForwardOutcome::Completed,
            ) => Some(Ok(0)),
            StreamResolution::Record(ForwardOutcome::Aborted { .. }) => {
                Some(Err(connection_reset()))
            }
            StreamResolution::Pending => None,
        }
    }

    fn write_end(&self) -> Option<io::Error> {
        match self.resolution() {
            StreamResolution::OwnerAbort => Some(connection_aborted()),
            StreamResolution::Record(ForwardOutcome::Aborted { .. }) => Some(connection_reset()),
            StreamResolution::Record(
                ForwardOutcome::GuestHalfClose | ForwardOutcome::Completed,
            )
            | StreamResolution::Pending => None,
        }
    }
}

impl Drop for HostGuestStream {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
        if let Some(forwarder) = state.forwarder.as_ref() {
            let _ = forwarder.context.abort(self.watch);
        }
        state.watches.remove(&(self.generation, self.watch));
        drop(state);
        let _ = self.socket.get_ref().shutdown(std::net::Shutdown::Both);
    }
}

#[async_trait::async_trait]
impl GuestStreamIo for HostGuestStream {
    async fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            if let Some(end) = self.read_end() {
                return end;
            }
            let mut ready = self.socket.readable().await?;
            match ready.try_io(|socket| {
                nix::sys::socket::recv(socket.get_ref().as_raw_fd(), buffer, MsgFlags::MSG_DONTWAIT)
                    .map_err(io::Error::from)
            }) {
                Ok(Ok(length)) if length > 0 => return Ok(length),
                Ok(Ok(_)) => {
                    if let Some(end) = self.read_end() {
                        return end;
                    }
                    let _ = self.wait_for_resolution().await;
                }
                Ok(Err(error)) => {
                    if let Some(end) = self.read_end() {
                        return end;
                    }
                    if error.kind() != io::ErrorKind::WouldBlock {
                        let resolution = self.wait_for_resolution().await;
                        if !matches!(resolution, StreamResolution::Pending)
                            && let Some(end) = self.read_end()
                        {
                            return end;
                        }
                    }
                }
                Err(_would_block) => {}
            }
        }
    }

    async fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.host_shut {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe));
        }
        let mut written = 0;
        while written < bytes.len() {
            if let Some(error) = self.write_end() {
                return Err(error);
            }
            let mut ready = self.socket.writable().await?;
            match ready.try_io(|socket| {
                nix::sys::socket::send(
                    socket.get_ref().as_raw_fd(),
                    &bytes[written..],
                    MsgFlags::MSG_DONTWAIT | MsgFlags::MSG_NOSIGNAL,
                )
                .map_err(io::Error::from)
            }) {
                Ok(Ok(0)) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(Ok(length)) => written += length,
                Ok(Err(error)) => {
                    if let Some(end) = self.write_end() {
                        return if written == 0 { Err(end) } else { Ok(written) };
                    }
                    if error.kind() != io::ErrorKind::WouldBlock {
                        let resolution = self.wait_for_resolution().await;
                        if !matches!(resolution, StreamResolution::Pending)
                            && let Some(end) = self.write_end()
                        {
                            return if written == 0 { Err(end) } else { Ok(written) };
                        }
                    }
                }
                Err(_would_block) => {}
            }
        }
        Ok(written)
    }

    async fn shutdown_write(&mut self) -> io::Result<()> {
        if self.host_shut {
            return Ok(());
        }
        if let Some(error) = self.write_end() {
            return Err(error);
        }
        loop {
            match self.socket.get_ref().shutdown(std::net::Shutdown::Write) {
                Ok(()) => {
                    self.host_shut = true;
                    return Ok(());
                }
                Err(error) => {
                    if let Some(end) = self.write_end() {
                        return Err(end);
                    }
                    let resolution = self.wait_for_resolution().await;
                    if !matches!(resolution, StreamResolution::Pending) {
                        if let Some(end) = self.write_end() {
                            return Err(end);
                        }
                    } else if error.kind() == io::ErrorKind::Unsupported {
                        return Err(error);
                    }
                }
            }
        }
    }

    fn end(&self) -> Option<GuestStreamEnd> {
        match self.resolution() {
            StreamResolution::Pending => None,
            StreamResolution::OwnerAbort
            | StreamResolution::Record(ForwardOutcome::Aborted { .. }) => {
                Some(GuestStreamEnd::Abort)
            }
            StreamResolution::Record(
                ForwardOutcome::GuestHalfClose | ForwardOutcome::Completed,
            ) => Some(GuestStreamEnd::HalfClose),
        }
    }
}

fn connection_aborted() -> io::Error {
    io::Error::from(io::ErrorKind::ConnectionAborted)
}

fn connection_reset() -> io::Error {
    io::Error::from(io::ErrorKind::ConnectionReset)
}
