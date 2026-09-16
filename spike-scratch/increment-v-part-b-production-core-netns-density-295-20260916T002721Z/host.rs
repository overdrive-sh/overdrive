// Throwaway GH #295 Part-B host assembly. Missing shared-TAP interception is scratch-only;
// the TLS/kTLS/splice, CA/SVID hold, resolver, and cgroup operations are production code.
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::mtls_resolve_adapter::ServiceBackendsResolve;
use overdrive_core::traits::ca::{Ca, SvidRequest};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::mtls_enforcement::{
    InterceptedConnection, MtlsEnforcement, MtlsLimits, Routed,
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

const CLIENT_ALLOC: &str = "gh295b-client";
const SERVER_ALLOC: &str = "gh295b-server";
const PEER: &str = "10.95.0.3:9000";

fn check(rc: i32, label: &str) {
    assert!(rc >= 0, "{label}: {}", std::io::Error::last_os_error());
}

fn sockaddr(ip: Ipv4Addr, port: u16) -> libc::sockaddr_in {
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    addr.sin_family = libc::AF_INET as u16;
    addr.sin_port = port.to_be();
    addr.sin_addr.s_addr = u32::from_ne_bytes(ip.octets());
    addr
}

fn transparent_listener(port: u16) -> TcpListener {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    check(fd, "socket");
    for (level, name, value) in [
        (libc::SOL_IP, libc::IP_TRANSPARENT, 1i32),
        (libc::SOL_SOCKET, libc::SO_REUSEADDR, 1i32),
    ] {
        check(
            unsafe {
                libc::setsockopt(
                    fd,
                    level,
                    name,
                    std::ptr::from_ref(&value).cast(),
                    std::mem::size_of::<i32>() as libc::socklen_t,
                )
            },
            "setsockopt listener",
        );
    }
    let addr = sockaddr(Ipv4Addr::LOCALHOST, port);
    check(
        unsafe {
            libc::bind(
                fd,
                std::ptr::from_ref(&addr).cast(),
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        },
        "bind transparent listener",
    );
    check(unsafe { libc::listen(fd, 32) }, "listen");
    unsafe { TcpListener::from_raw_fd(fd) }
}

fn original_destination(stream: &TcpStream, leg: &str) -> SocketAddrV4 {
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    check(
        unsafe {
            libc::getsockname(
                stream.as_raw_fd(),
                std::ptr::from_mut(&mut addr).cast(),
                std::ptr::from_mut(&mut len),
            )
        },
        "getsockname",
    );
    let original = SocketAddrV4::new(
        Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes()),
        u16::from_be(addr.sin_port),
    );
    println!("{leg} GETSOCKNAME_ORIGINAL={original} PEER={}", stream.peer_addr().unwrap());
    assert_eq!(original, PEER.parse::<SocketAddrV4>().unwrap());
    original
}

fn start_dns() {
    let socket = UdpSocket::bind("10.95.0.1:53").unwrap();
    std::thread::spawn(move || loop {
        let mut query = [0u8; 512];
        let (n, from) = socket.recv_from(&mut query).unwrap();
        let query = &query[..n];
        let mut end = 12;
        let mut labels = Vec::new();
        while query[end] != 0 {
            let count = query[end] as usize;
            labels.push(std::str::from_utf8(&query[end + 1..end + 1 + count]).unwrap());
            end += 1 + count;
        }
        let qtype = u16::from_be_bytes([query[end + 1], query[end + 2]]);
        let name = labels.join(".");
        println!("SCRATCH_SHARED_BRIDGE_DNS query_from={from} name={name} qtype={qtype}");
        assert_eq!(name, "peer.mesh");
        let answer = qtype == 1;
        let mut response = query[..end + 5].to_vec();
        response[2..4].copy_from_slice(&0x8180u16.to_be_bytes());
        response[6..8].copy_from_slice(&u16::from(answer).to_be_bytes());
        response[8..12].fill(0);
        if answer {
            response.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 1, 0, 4, 10, 95, 0, 3]);
        }
        socket.send_to(&response, from).unwrap();
    });
}

async fn cgroup_manager() -> CgroupManager {
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(PathBuf::from("/sys/fs/cgroup"), fs)
}

fn allocation(raw: &str) -> AllocationId {
    AllocationId::new(raw).unwrap()
}

async fn cgroup_init() {
    let manager = cgroup_manager().await;
    manager.create_workloads_slice_with_controllers().await.unwrap();
    for raw in [CLIENT_ALLOC, SERVER_ALLOC] {
        let scope = CgroupPath::for_alloc(&allocation(raw));
        manager.create_workload_scope(&scope).await.unwrap();
        manager
            .write_resource_limits(
                &scope,
                &Resources { cpu_milli: 1_000, memory_bytes: 768 * 1024 * 1024 },
            )
            .await
            .unwrap();
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
                Err(error) if error.raw_os_error() == Some(libc::EBUSY) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => panic!("remove {scope}: {error}"),
            }
        }
        println!("PRODUCTION_CGROUP_REMOVED alloc={raw} scope={scope}");
    }
}

async fn build_production_components() -> (
    Arc<HostMtlsEnforcement>,
    Arc<ServiceBackendsResolve>,
    AllocationId,
    AllocationId,
) {
    let provider = rustls::crypto::ring::default_provider();
    let _ = provider.install_default();

    let client_alloc = allocation(CLIENT_ALLOC);
    let server_alloc = allocation(SERVER_ALLOC);
    let client_workload = WorkloadId::new("gh295b-client-workload").unwrap();
    let server_workload = WorkloadId::new("peer").unwrap();
    let client_spiffe = SpiffeId::for_allocation(&client_workload, &client_alloc);
    let server_spiffe = SpiffeId::for_allocation(&server_workload, &server_alloc);
    let ca = RcgenCa::new(
        Arc::new(OsEntropy),
        SpiffeId::new("spiffe://overdrive.local/overdrive/ca").unwrap(),
    );
    ca.issue_intermediate(&NodeId::new("gh295b-node").unwrap()).unwrap();
    let now = SystemClock.unix_now();
    let not_before = UnixInstant::from_unix_duration(now.saturating_sub(Duration::from_secs(60)));
    let not_after = UnixInstant::from_unix_duration(now + Duration::from_secs(3_600));
    let client_svid = ca
        .issue_svid(&SvidRequest::new(client_spiffe.clone(), not_before, not_after))
        .unwrap();
    let server_svid = ca
        .issue_svid(&SvidRequest::new(server_spiffe.clone(), not_before, not_after))
        .unwrap();
    let identity = Arc::new(IdentityMgr::new(Some(ca.trust_bundle().unwrap())));
    identity.hold(client_alloc.clone(), client_svid);
    identity.hold(server_alloc.clone(), server_svid);
    println!(
        "PRODUCTION_IDENTITY_HELD client={} server={} held_count={}",
        client_spiffe,
        server_spiffe,
        identity.held_snapshot().len()
    );

    let node = NodeId::new("gh295b-node").unwrap();
    let store = Arc::new(SimObservationStore::single_peer(node.clone(), 295));
    let peer: SocketAddrV4 = PEER.parse().unwrap();
    store
        .write(ObservationWrite::ServiceBackend(ServiceBackendRow {
            service_id: ServiceId::new(295).unwrap(),
            vip: *peer.ip(),
            backends: vec![Backend {
                alloc: server_spiffe,
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
    println!("PRODUCTION_RESOLVER_PROBE_OK");

    let identity_read: Arc<dyn overdrive_core::traits::IdentityRead> = identity;
    let enforcement = Arc::new(HostMtlsEnforcement::new(identity_read, MtlsLimits::default()));
    enforcement.probe().await.unwrap();
    println!("PRODUCTION_MTLS_KTLS_SPLICE_PROBE_OK");
    (enforcement, resolver, client_alloc, server_alloc)
}

async fn run_host() {
    let (enforcement, resolver, client_alloc, server_alloc) = build_production_components().await;
    start_dns();
    let leg_f_listener = transparent_listener(15294);
    let leg_c_listener = transparent_listener(15295);
    println!("HOST READY legF=127.0.0.1:15294 legC=127.0.0.1:15295 dns=10.95.0.1:53");

    let inbound_enforcement = Arc::clone(&enforcement);
    let inbound = tokio::spawn(async move {
        let (leg_c, _) = tokio::task::spawn_blocking(move || leg_c_listener.accept().unwrap())
            .await
            .unwrap();
        let original = original_destination(&leg_c, "LEG_C owner=tap295b");
        let leg: OwnedFd = leg_c.into();
        let handle = inbound_enforcement
            .enforce(InterceptedConnection {
                leg,
                routed: Routed::Inbound { orig_dst: original },
                alloc: server_alloc,
                expected_peer: None,
            })
            .await
            .unwrap();
        println!("PRODUCTION_INBOUND_KTLS_SPLICE_ESTABLISHED id={}", handle.id());
        handle
    });

    let (leg_f, _) = tokio::task::spawn_blocking(move || leg_f_listener.accept().unwrap())
        .await
        .unwrap();
    let original = original_destination(&leg_f, "LEG_F owner=tap295a");
    let resolution = resolver.resolve(original).await.unwrap();
    let MtlsResolution::Mesh(backend) = resolution else {
        panic!("production resolver did not classify {original} as Mesh");
    };
    println!(
        "PRODUCTION_RESOLVER_MESH orig_dst={original} backend={} expected_svid={:?}",
        backend.addr, backend.expected_svid
    );
    let leg: OwnedFd = leg_f.into();
    let outbound = enforcement
        .enforce(InterceptedConnection {
            leg,
            routed: Routed::Outbound { peer: backend.addr },
            alloc: client_alloc,
            expected_peer: backend.expected_svid,
        })
        .await
        .unwrap();
    println!("PRODUCTION_OUTBOUND_KTLS_SPLICE_ESTABLISHED id={}", outbound.id());
    let inbound = inbound.await.unwrap();
    println!("PRODUCTION_MTLS_BOTH_ESTABLISHED");
    tokio::time::sleep(Duration::from_secs(6)).await;
    enforcement.teardown(outbound).await.unwrap();
    enforcement.teardown(inbound).await.unwrap();
    println!("PRODUCTION_MTLS_TEARDOWN_COMPLETE");
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some("cgroup-init") => cgroup_init().await,
        Some("cgroup-place") => {
            let alloc = args.get(2).unwrap();
            let pid = args.get(3).unwrap().parse().unwrap();
            cgroup_place(alloc, pid).await;
        }
        Some("cgroup-clean") => cgroup_clean().await,
        None => run_host().await,
        other => panic!("unknown mode: {other:?}"),
    }
}
