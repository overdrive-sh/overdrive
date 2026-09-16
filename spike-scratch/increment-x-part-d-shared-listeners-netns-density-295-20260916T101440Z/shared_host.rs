use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::mtls_resolve_adapter::ServiceBackendsResolve;
use overdrive_core::traits::ca::{Ca, SvidRequest};
use overdrive_core::traits::clock::Clock as _;
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::identity_read::IdentityRead as _;
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, InterceptedConnection, MtlsEnforcement, MtlsLimits, Routed,
};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationStore, ObservationWrite, ServiceBackendRow,
};
use overdrive_core::traits::{CgroupFs, driver::Resources};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::id::ServiceId;
use overdrive_core::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_dataplane::mtls::HostMtlsEnforcement;
use overdrive_host::{OsEntropy, RcgenCa, RealCgroupFs, SystemClock};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_worker::{CgroupManager, CgroupPath};

const CLIENT_ALLOC: &str = "gh295d-client";
const SERVER_ALLOC: &str = "gh295d-server";
const SUCCESSOR_ALLOC: &str = "gh295d-successor";
const PEER: &str = "10.95.0.3:9000";

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Capability {
    alloc: AllocationId,
    generation: u64,
    identity: SpiffeId,
}

struct Registry {
    source: RwLock<BTreeMap<Ipv4Addr, Capability>>,
    destination: RwLock<BTreeMap<SocketAddrV4, Capability>>,
}

impl Registry {
    fn new() -> Self {
        Self { source: RwLock::new(BTreeMap::new()), destination: RwLock::new(BTreeMap::new()) }
    }

    fn source(&self, ip: Ipv4Addr) -> Option<Capability> {
        self.source.read().unwrap().get(&ip).cloned()
    }

    fn destination(&self, addr: SocketAddrV4) -> Option<Capability> {
        self.destination.read().unwrap().get(&addr).cloned()
    }

    fn assign_source(&self, ip: Ipv4Addr, cap: Capability) {
        self.source.write().unwrap().insert(ip, cap);
    }

    fn assign_destination(&self, addr: SocketAddrV4, cap: Capability) {
        self.destination.write().unwrap().insert(addr, cap);
    }

    fn remove_source(&self, ip: Ipv4Addr, expected: &Capability) {
        let mut map = self.source.write().unwrap();
        if map.get(&ip) == Some(expected) {
            map.remove(&ip);
        }
    }

    fn remove_destination(&self, addr: SocketAddrV4, expected: &Capability) {
        let mut map = self.destination.write().unwrap();
        if map.get(&addr) == Some(expected) {
            map.remove(&addr);
        }
    }
}

struct OwnerState {
    active: BTreeSet<Capability>,
    handles: BTreeMap<Capability, Vec<EnforcedConnection>>,
}

struct ConnectionOwner {
    state: Mutex<OwnerState>,
    enforcement: Arc<HostMtlsEnforcement>,
}

impl ConnectionOwner {
    fn new(enforcement: Arc<HostMtlsEnforcement>) -> Self {
        Self {
            state: Mutex::new(OwnerState { active: BTreeSet::new(), handles: BTreeMap::new() }),
            enforcement,
        }
    }

    fn activate(&self, cap: Capability) {
        self.state.lock().unwrap().active.insert(cap);
    }

    fn claim(&self, cap: &Capability) -> bool {
        self.state.lock().unwrap().active.contains(cap)
    }

    async fn publish(&self, cap: Capability, handle: EnforcedConnection) -> bool {
        let accepted = {
            let mut state = self.state.lock().unwrap();
            if state.active.contains(&cap) {
                state.handles.entry(cap.clone()).or_default().push(handle.clone());
                true
            } else {
                false
            }
        };
        if !accepted {
            self.enforcement.teardown(handle).await.unwrap();
        }
        accepted
    }

    async fn stop(&self, cap: &Capability) -> usize {
        let handles = {
            let mut state = self.state.lock().unwrap();
            state.active.remove(cap);
            state.handles.remove(cap).unwrap_or_default()
        };
        let count = handles.len();
        for handle in handles {
            self.enforcement.teardown(handle).await.unwrap();
        }
        count
    }

    fn handle_count(&self, cap: &Capability) -> usize {
        self.state.lock().unwrap().handles.get(cap).map_or(0, Vec::len)
    }
}

fn allocation(raw: &str) -> AllocationId {
    AllocationId::new(raw).unwrap()
}

fn sockaddr(ip: Ipv4Addr, port: u16) -> libc::sockaddr_in {
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    addr.sin_family = libc::AF_INET as u16;
    addr.sin_port = port.to_be();
    addr.sin_addr.s_addr = u32::from_ne_bytes(ip.octets());
    addr
}

fn transparent_listener_at(ip: Ipv4Addr, port: u16) -> TcpListener {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    assert!(fd >= 0, "socket: {}", std::io::Error::last_os_error());
    for (level, name, value) in [
        (libc::SOL_IP, libc::IP_TRANSPARENT, 1i32),
        (libc::SOL_SOCKET, libc::SO_REUSEADDR, 1i32),
    ] {
        let rc = unsafe {
            libc::setsockopt(
                fd,
                level,
                name,
                std::ptr::from_ref(&value).cast(),
                std::mem::size_of::<i32>() as libc::socklen_t,
            )
        };
        assert!(rc == 0, "setsockopt: {}", std::io::Error::last_os_error());
    }
    let addr = sockaddr(ip, port);
    let rc = unsafe {
        libc::bind(
            fd,
            std::ptr::from_ref(&addr).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    assert!(rc == 0, "bind {ip}:{port}: {}", std::io::Error::last_os_error());
    assert!(unsafe { libc::listen(fd, 128) } == 0);
    unsafe { TcpListener::from_raw_fd(fd) }
}

fn original_destination(stream: &TcpStream) -> SocketAddrV4 {
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    assert!(unsafe {
        libc::getsockname(
            stream.as_raw_fd(),
            std::ptr::from_mut(&mut addr).cast(),
            std::ptr::from_mut(&mut len),
        )
    } == 0);
    SocketAddrV4::new(
        Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes()),
        u16::from_be(addr.sin_port),
    )
}

fn start_dns() {
    let socket = UdpSocket::bind("10.95.0.1:53").unwrap();
    std::thread::spawn(move || loop {
        let mut query = [0u8; 512];
        let (n, from) = socket.recv_from(&mut query).unwrap();
        let query = &query[..n];
        let mut end = 12;
        while query[end] != 0 {
            end += 1 + usize::from(query[end]);
        }
        let qtype = u16::from_be_bytes([query[end + 1], query[end + 2]]);
        let mut response = query[..end + 5].to_vec();
        response[2..4].copy_from_slice(&0x8180u16.to_be_bytes());
        response[6..8].copy_from_slice(&u16::from(qtype == 1).to_be_bytes());
        response[8..12].fill(0);
        if qtype == 1 {
            response.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 1, 0, 4, 10, 95, 0, 3]);
        }
        socket.send_to(&response, from).unwrap();
    });
}

fn capability(workload: &str, alloc: &str, generation: u64) -> Capability {
    let alloc = allocation(alloc);
    let workload = WorkloadId::new(workload).unwrap();
    Capability { identity: SpiffeId::for_allocation(&workload, &alloc), alloc, generation }
}

async fn build_components() -> (
    Arc<HostMtlsEnforcement>,
    Arc<ServiceBackendsResolve>,
    Arc<IdentityMgr>,
    Capability,
    Capability,
    Capability,
) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = capability("gh295d-client-workload", CLIENT_ALLOC, 1);
    let server = capability("peer", SERVER_ALLOC, 1);
    let successor = capability("gh295d-successor-workload", SUCCESSOR_ALLOC, 2);
    let ca = RcgenCa::new(
        Arc::new(OsEntropy),
        SpiffeId::new("spiffe://overdrive.local/overdrive/ca").unwrap(),
    );
    ca.issue_intermediate(&NodeId::new("gh295d-node").unwrap()).unwrap();
    let now = SystemClock.unix_now();
    let before = UnixInstant::from_unix_duration(now.saturating_sub(Duration::from_secs(60)));
    let after = UnixInstant::from_unix_duration(now + Duration::from_secs(3600));
    let identity = Arc::new(IdentityMgr::new(Some(ca.trust_bundle().unwrap())));
    for cap in [&client, &server, &successor] {
        identity.hold(
            cap.alloc.clone(),
            ca.issue_svid(&SvidRequest::new(cap.identity.clone(), before, after)).unwrap(),
        );
    }
    let node = NodeId::new("gh295d-node").unwrap();
    let store = Arc::new(SimObservationStore::single_peer(node.clone(), 295));
    let peer: SocketAddrV4 = PEER.parse().unwrap();
    store
        .write(ObservationWrite::ServiceBackend(ServiceBackendRow {
            service_id: ServiceId::new(295).unwrap(),
            vip: *peer.ip(),
            backends: vec![Backend {
                alloc: server.identity.clone(),
                addr: SocketAddr::V4(peer),
                weight: 1,
                healthy: true,
            }],
            updated_at: LogicalTimestamp::dominating(0, node, None),
        }))
        .await
        .unwrap();
    let store: Arc<dyn ObservationStore> = store;
    let resolver = Arc::new(ServiceBackendsResolve::new(store, FrontendAddrAllocator::new()));
    resolver.probe().await.unwrap();
    let read: Arc<dyn overdrive_core::traits::IdentityRead> = identity.clone();
    let enforcement = Arc::new(HostMtlsEnforcement::new(read, MtlsLimits::default()));
    enforcement.probe().await.unwrap();
    (enforcement, resolver, identity, client, server, successor)
}

fn assert_eof(mut client: TcpStream) {
    client.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut byte = [0u8; 1];
    assert_eq!(client.read(&mut byte).unwrap(), 0);
}

fn registry_reuse_selftest(
    registry: &Registry,
    owner: &ConnectionOwner,
    identity: &IdentityMgr,
    predecessor: &Capability,
    successor: &Capability,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();

    let unknown = TcpStream::connect(address).unwrap();
    let (accepted, peer) = listener.accept().unwrap();
    assert!(registry.source(peer.ip().to_string().parse().unwrap()).is_none());
    drop(accepted);
    assert_eof(unknown);
    println!("SHARED_NEGATIVE unknown_source=FAIL_CLOSED");

    let source = Ipv4Addr::LOCALHOST;
    registry.assign_source(source, predecessor.clone());
    owner.activate(predecessor.clone());
    let inflight = TcpStream::connect(address).unwrap();
    let (accepted, peer) = listener.accept().unwrap();
    let captured = registry.source(peer.ip().to_string().parse().unwrap()).unwrap();
    assert_eq!(&captured, predecessor);
    assert!(owner.claim(&captured));
    owner.state.lock().unwrap().active.remove(predecessor);
    registry.remove_source(source, predecessor);
    registry.assign_source(source, successor.clone());
    owner.activate(successor.clone());
    assert!(!owner.claim(&captured));
    assert_eq!(registry.source(source).as_ref(), Some(successor));
    assert_eq!(identity.svid_for(&captured.alloc).unwrap().spiffe_id(), &predecessor.identity);
    assert_eq!(identity.svid_for(&successor.alloc).unwrap().spiffe_id(), &successor.identity);
    drop(accepted);
    assert_eof(inflight);
    println!(
        "SHARED_REUSE accepted_alloc={} accepted_generation={} successor_alloc={} successor_generation={} publish=TEARDOWN_NOT_REATTRIBUTE",
        captured.alloc, captured.generation, successor.alloc, successor.generation
    );

    let new_client = TcpStream::connect(address).unwrap();
    let (accepted, peer) = listener.accept().unwrap();
    let new_cap = registry.source(peer.ip().to_string().parse().unwrap()).unwrap();
    assert_eq!(&new_cap, successor);
    drop(accepted);
    assert_eof(new_client);
    println!("SHARED_SUCCESSOR_NEW_CONNECT alloc={} generation={} identity={}", new_cap.alloc, new_cap.generation, new_cap.identity);

    registry.remove_source(source, successor);
    owner.state.lock().unwrap().active.remove(successor);
    let removed = TcpStream::connect(address).unwrap();
    let (accepted, peer) = listener.accept().unwrap();
    assert!(registry.source(peer.ip().to_string().parse().unwrap()).is_none());
    drop(accepted);
    assert_eof(removed);
    println!("SHARED_NEGATIVE post_removal_new_connect=FAIL_CLOSED");

    let unknown_destination = TcpStream::connect(address).unwrap();
    let (accepted, _) = listener.accept().unwrap();
    let local = match accepted.local_addr().unwrap() { SocketAddr::V4(v4) => v4, _ => unreachable!() };
    assert!(registry.destination(local).is_none());
    drop(accepted);
    assert_eof(unknown_destination);
    println!("SHARED_NEGATIVE unknown_destination=FAIL_CLOSED");
}

async fn run_shared() {
    let (enforcement, resolver, identity, client, server, successor) = build_components().await;
    let registry = Arc::new(Registry::new());
    let owner = Arc::new(ConnectionOwner::new(enforcement.clone()));
    registry_reuse_selftest(&registry, &owner, &identity, &client, &successor);

    let client_ip = Ipv4Addr::new(10, 95, 0, 2);
    let peer: SocketAddrV4 = PEER.parse().unwrap();
    registry.assign_source(client_ip, client.clone());
    registry.assign_destination(peer, server.clone());
    owner.activate(client.clone());
    owner.activate(server.clone());
    start_dns();

    let leg_f_std = transparent_listener_at(Ipv4Addr::LOCALHOST, 15294);
    let leg_c_std = transparent_listener_at(Ipv4Addr::LOCALHOST, 15295);
    leg_f_std.set_nonblocking(true).unwrap();
    leg_c_std.set_nonblocking(true).unwrap();
    let leg_f = tokio::net::TcpListener::from_std(leg_f_std).unwrap();
    let leg_c = tokio::net::TcpListener::from_std(leg_c_std).unwrap();
    let (ready_tx, mut ready_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let f_registry = registry.clone();
    let f_owner = owner.clone();
    let f_resolver = resolver.clone();
    let f_tx = ready_tx.clone();
    let leg_f_task = tokio::spawn(async move {
        loop {
            let (stream, peer_addr) = leg_f.accept().await.unwrap();
            let source = match peer_addr { SocketAddr::V4(v4) => *v4.ip(), _ => { drop(stream); continue; } };
            let Some(cap) = f_registry.source(source) else {
                println!("SHARED_LEG_F_REJECT unknown_source={source}");
                drop(stream);
                continue;
            };
            if !f_owner.claim(&cap) {
                println!("SHARED_LEG_F_REJECT stale_capability alloc={} generation={}", cap.alloc, cap.generation);
                drop(stream);
                continue;
            }
            let std_stream = stream.into_std().unwrap();
            let original = original_destination(&std_stream);
            let resolution = f_resolver.resolve(original).await.unwrap();
            let MtlsResolution::Mesh(backend) = resolution else { drop(std_stream); continue; };
            println!("SHARED_LEG_F_CAPABILITY source={source} orig_dst={original} alloc={} generation={} identity={}", cap.alloc, cap.generation, cap.identity);
            let leg: OwnedFd = std_stream.into();
            let handle = f_owner.enforcement.enforce(InterceptedConnection {
                leg,
                routed: Routed::Outbound { peer: backend.addr },
                alloc: cap.alloc.clone(),
                expected_peer: backend.expected_svid,
            }).await.unwrap();
            if f_owner.publish(cap.clone(), handle).await {
                let _ = f_tx.send(format!("F:{}:{}", cap.alloc, cap.generation));
            }
        }
    });

    let c_registry = registry.clone();
    let c_owner = owner.clone();
    let c_tx = ready_tx;
    let leg_c_task = tokio::spawn(async move {
        loop {
            let (stream, _) = leg_c.accept().await.unwrap();
            let std_stream = stream.into_std().unwrap();
            let original = original_destination(&std_stream);
            let Some(cap) = c_registry.destination(original) else {
                println!("SHARED_LEG_C_REJECT unknown_destination={original}");
                drop(std_stream);
                continue;
            };
            if !c_owner.claim(&cap) {
                println!("SHARED_LEG_C_REJECT stale_capability alloc={} generation={}", cap.alloc, cap.generation);
                drop(std_stream);
                continue;
            }
            println!("SHARED_LEG_C_CAPABILITY orig_dst={original} alloc={} generation={} identity={}", cap.alloc, cap.generation, cap.identity);
            let leg: OwnedFd = std_stream.into();
            let handle = c_owner.enforcement.enforce(InterceptedConnection {
                leg,
                routed: Routed::Inbound { orig_dst: original },
                alloc: cap.alloc.clone(),
                expected_peer: None,
            }).await.unwrap();
            if c_owner.publish(cap.clone(), handle).await {
                let _ = c_tx.send(format!("C:{}:{}", cap.alloc, cap.generation));
            }
        }
    });

    println!("SHARED_LISTENERS_READY leg_f=127.0.0.1:15294 leg_c=127.0.0.1:15295 listener_count=2 accept_task_count=2");
    let first = ready_rx.recv().await.unwrap();
    let second = ready_rx.recv().await.unwrap();
    println!("SHARED_CONNECTIONS_ESTABLISHED first={first} second={second}");
    tokio::time::sleep(Duration::from_secs(6)).await;

    let server_before = owner.handle_count(&server);
    let client_drained = owner.stop(&client).await;
    println!(
        "SHARED_STOP_ISOLATION stopped_alloc={} generation={} drained_handles={} server_handles_before={} leg_f_listener_alive={} leg_c_listener_alive={}",
        client.alloc,
        client.generation,
        client_drained,
        server_before,
        !leg_f_task.is_finished(),
        !leg_c_task.is_finished()
    );
    assert!(!leg_f_task.is_finished() && !leg_c_task.is_finished());
    assert!(server_before > 0);
    let server_drained = owner.stop(&server).await;
    println!("SHARED_SERVER_STOP drained_handles={server_drained}");
    registry.remove_source(client_ip, &client);
    registry.remove_destination(peer, &server);
    leg_f_task.abort();
    leg_c_task.abort();
    println!("SHARED_LISTENER_OWNER_SHUTDOWN_COMPLETE");
}

async fn cgroup_manager() -> CgroupManager {
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), fs)
}

async fn cgroup_init() {
    let manager = cgroup_manager().await;
    manager.create_workloads_slice_with_controllers().await.unwrap();
    for raw in [CLIENT_ALLOC, SERVER_ALLOC] {
        let scope = CgroupPath::for_alloc(&allocation(raw));
        manager.create_workload_scope(&scope).await.unwrap();
        manager.write_resource_limits(&scope, &Resources { cpu_milli: 1000, memory_bytes: 768 * 1024 * 1024 }).await.unwrap();
        println!("PRODUCTION_CGROUP_CREATED alloc={raw} scope={scope}");
    }
}

async fn cgroup_place(raw: &str, pid: u32) {
    let manager = cgroup_manager().await;
    let scope = CgroupPath::for_alloc(&allocation(raw));
    manager.place_pid_in_scope(&scope, pid).await.unwrap();
    println!("PRODUCTION_CGROUP_PID_PLACED alloc={raw} scope={scope} pid={pid}");
}

async fn cgroup_clean() {
    let manager = cgroup_manager().await;
    for raw in [CLIENT_ALLOC, SERVER_ALLOC] {
        let scope = CgroupPath::for_alloc(&allocation(raw));
        manager.cgroup_kill(&scope).await.unwrap();
        for _ in 0..100 {
            match manager.remove_workload_scope(&scope).await {
                Ok(()) => break,
                Err(error) if error.raw_os_error() == Some(libc::EBUSY) => tokio::time::sleep(Duration::from_millis(20)).await,
                Err(error) => panic!("remove {scope}: {error}"),
            }
        }
        println!("PRODUCTION_CGROUP_REMOVED alloc={raw} scope={scope}");
    }
}

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap()
}

fn fd_count() -> usize {
    std::fs::read_dir("/proc/self/fd").unwrap().count()
}

async fn resource_compare(mode: &str, n: usize) {
    let start_rss = rss_kib();
    let mut source = BTreeMap::new();
    let mut destination = BTreeMap::new();
    for index in 0..n {
        let cap = (index as u64, 1u64);
        let ip = Ipv4Addr::new(10, 200, (index >> 8) as u8, index as u8);
        source.insert(ip, cap);
        destination.insert(SocketAddrV4::new(ip, 9000), cap);
    }
    let registry_rss = rss_kib();
    let before_fd = fd_count();
    let before_rss = rss_kib();
    let started = Instant::now();
    let listener_count = if mode == "shared" { 2 } else { 2 * n };
    let mut listeners = Vec::with_capacity(listener_count);
    let mut tasks = Vec::with_capacity(listener_count);
    for index in 0..listener_count {
        let ip = if mode == "shared" {
            Ipv4Addr::LOCALHOST
        } else {
            let lane = if index < n { 64 } else { 65 };
            let ordinal = index % n;
            Ipv4Addr::new(127, lane, (ordinal >> 8) as u8, ordinal as u8)
        };
        let listener = transparent_listener_at(ip, 0);
        listener.set_nonblocking(true).unwrap();
        let listener = Arc::new(tokio::net::TcpListener::from_std(listener).unwrap());
        let task_listener = listener.clone();
        tasks.push(tokio::spawn(async move { let _ = task_listener.accept().await; }));
        listeners.push(listener);
    }
    tokio::task::yield_now().await;
    let setup = started.elapsed();
    let after_fd = fd_count();
    let after_rss = rss_kib();
    println!(
        "RESOURCE_RESULT mode={mode} n={n} registry_entries={} listeners={} idle_accept_tasks={} fd_before={} fd_after={} fd_delta={} rss_start_kib={} rss_after_registry_kib={} registry_rss_delta_kib={} rss_after_listeners_kib={} listener_task_rss_delta_kib={} setup_ms={:.3} kernel_socket_memory=not_process_attributable",
        source.len() + destination.len(),
        listener_count,
        tasks.len(),
        before_fd,
        after_fd,
        after_fd - before_fd,
        start_rss,
        registry_rss,
        registry_rss.saturating_sub(start_rss),
        after_rss,
        after_rss.saturating_sub(before_rss),
        setup.as_secs_f64() * 1000.0
    );
    let teardown = Instant::now();
    for task in tasks { task.abort(); }
    drop(listeners);
    tokio::time::sleep(Duration::from_millis(100)).await;
    println!(
        "RESOURCE_TEARDOWN mode={mode} fd_after={} rss_after_kib={} teardown_ms={:.3}",
        fd_count(),
        rss_kib(),
        teardown.elapsed().as_secs_f64() * 1000.0
    );
    drop((source, destination));
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some("cgroup-init") => cgroup_init().await,
        Some("cgroup-place") => cgroup_place(args.get(2).unwrap(), args.get(3).unwrap().parse().unwrap()).await,
        Some("cgroup-clean") => cgroup_clean().await,
        Some("resource-compare") => resource_compare(args.get(2).unwrap(), args.get(3).unwrap().parse().unwrap()).await,
        None => run_shared().await,
        other => panic!("unknown mode {other:?}"),
    }
}
