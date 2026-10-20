//! LEGB-DIAL — leg-B of host mTLS enforcement marks and source-binds its dial
//! to a same-node VM (a peer inside the guest prefix), and only then.
//! Tier-3, Lima + root, `host-kernel-shared`.
//!
//! netns-density-295 vsock attachment replacement. Contract:
//! `docs/feature/netns-density-295/feature-delta.md` § "[REF] Component — host
//! mTLS enforcement leg-B dial (LEGB-DIAL)" (ADR-0190): the constructor
//! `HostMtlsEnforcement::new(identity, limits, guest_prefix)`; per leg-B dial
//! to a peer inside `guest_prefix`, `SO_MARK = MTLS_TPROXY_MARK`,
//! `IP_BIND_ADDRESS_NO_PORT`, `bind(127.0.0.1:0)` before `connect()`; to any
//! other peer, none of them. Obligation R5-43 (the leg-B half; the constant
//! rules' half is `vsock_constant_guest_prefix_rules`). Evidence V-LEGB-MARK
//! (`spike/findings-v33-legb-mark.md`).
//!
//! # Production path
//!
//! The node is the production `MtlsInterceptWorker` over the real
//! `HostMtlsIntercept` (shared leg-F / leg-C listeners, the constant nft
//! program, the `fwmark 0x1 → table 100` rule and `local default dev lo
//! table 100` route, published by `start_shared_owner`) and the real
//! `HostMtlsEnforcement`. A server allocation is made intercept-live through
//! `start_alloc` with a workload address inside `GUEST_NODE_PREFIX` and one
//! declared service port, so production registers its `inbound_destinations`
//! tuple. Leg-B is driven through the `MtlsEnforcement` driving port
//! (`enforce` with `Routed::Outbound`) on the same `HostMtlsEnforcement`.
//! No body installs a rule, route or address production does not install;
//! no route covers the guest prefix (INB-1).
//!
//! Observables: `enforce`'s result (an `Ok` means leg-C completed the mTLS
//! handshake, presenting the server allocation's SVID — leg-C handshakes
//! before it dials leg-S), the kernel's socket table (`ss -tne`: leg-C's
//! accepted peer and leg-B's `fwmark`), and what a far peer accepts.
//!
//! # Not here
//!
//! The no-default-route cells of V-LEGB-MARK (S1, S4) need a node with no
//! default route; the dev VM has one, so these bodies are the with-route
//! cells (S2, S3, S5). The single-rule-dropped rows (mark only, bind only)
//! are V-LEGB-MARK spike facts with no production path that produces them.
//!
//! Ignored until DELIVER step 13 (the leg-B dial, `HostMtlsEnforcement::new`
//! with `guest_prefix`); the unregistered-tuple cell also needs step 12's
//! constant guest-prefix rules, which precede it.

#![allow(
    clippy::drop_non_drop,
    clippy::missing_const_for_fn,
    clippy::redundant_clone,
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    reason = "Tier-3 bodies: evidence goes to stderr; fixture preconditions panic with informative messages"
)]

use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use overdrive_core::AllocationId;
use overdrive_core::guest_transport::GUEST_NODE_PREFIX;
use overdrive_core::traits::mtls_enforcement::{
    InterceptedConnection, MtlsEnforcement, MtlsEnforcementError, Routed,
};

use self::node::{Node, PeerNetns, established_sockets};

/// A guest-prefix address the node treats as a same-node VM's workload
/// address (no route, no host address: INB-1).
const VM_ADDR: Ipv4Addr = Ipv4Addr::new(100, 95, 255, 200);
/// The VM's declared service port.
const SERVICE_PORT: u16 = 18_443;
/// A port of the VM that no allocation declares.
const UNDECLARED_PORT: u16 = 18_444;
/// The bound within which a refused dial must fail (no hang).
const VERDICT_BOUND: Duration = Duration::from_secs(1);

/// A connected loopback TCP pair: the far end plays the intercepted
/// workload, the near end is leg F handed to `enforce`.
fn leg_f_pair() -> (TcpStream, OwnedFd) {
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("precondition: a loopback listener for the leg-F pair");
    let workload = TcpStream::connect(listener.local_addr().expect("listener address"))
        .expect("precondition: the workload end connects");
    let (leg_f, _) = listener.accept().expect("precondition: leg F accepted");
    (workload, OwnedFd::from(leg_f))
}

fn outbound(leg: OwnedFd, client: &AllocationId, peer: SocketAddrV4) -> InterceptedConnection {
    InterceptedConnection {
        leg,
        routed: Routed::Outbound { peer },
        alloc: client.clone(),
        expected_peer: None,
    }
}

/// A same-node mesh dial (leg-B to a peer inside the guest prefix) is marked
/// and bound to `127.0.0.1:0`, matches the existing `fwmark 0x1` rule, is
/// delivered on `lo` and diverted to leg-C by the server allocation's
/// registered tuple: leg-C completes the mTLS handshake (leg-B's `enforce`
/// returns `Ok`), leg-C sees the peer as `127.0.0.1:<ephemeral>`, and the
/// leg-B socket carries `fwmark 0x1`.
///
/// CONTRACT_SHAPE: bounded-change. Universe: `enforce`'s result, the
/// established sockets on the service port; expected: `Ok`, leg-C's accepted
/// socket has local `VM_ADDR:SERVICE_PORT` and a `127.0.0.1` peer, leg-B's
/// socket has `fwmark:0x1`.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER step 13"]
async fn a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c() {
    let node = Node::start("legb-same-node").await;
    let server = node.serve_vm("legb-server", VM_ADDR, SERVICE_PORT).await;
    let (_workload, leg_f) = leg_f_pair();

    let started = Instant::now();
    let enforced = node
        .enforcement()
        .enforce(outbound(leg_f, &node.client_alloc(), SocketAddrV4::new(VM_ADDR, SERVICE_PORT)))
        .await;
    node.record(
        "enforce",
        format_args!("{:?} after {:?}", enforced.as_ref().err(), started.elapsed()),
    );
    let _enforced =
        enforced.expect("leg-C completed the mTLS handshake with the marked, bound leg-B");

    let sockets = established_sockets(SERVICE_PORT);
    node.record("ss", format_args!("{sockets:?}"));
    let leg_c = sockets
        .iter()
        .find(|s| s.local == SocketAddrV4::new(VM_ADDR, SERVICE_PORT))
        .expect("leg-C's accepted socket is local to the VM's registered tuple");
    assert_eq!(*leg_c.peer.ip(), Ipv4Addr::LOCALHOST, "leg-C sees a same-node peer as 127.0.0.1");
    let leg_b = sockets
        .iter()
        .find(|s| s.peer == SocketAddrV4::new(VM_ADDR, SERVICE_PORT))
        .expect("leg-B's socket is connected to the VM's address");
    assert_eq!(*leg_b.local.ip(), Ipv4Addr::LOCALHOST, "leg-B is bound to 127.0.0.1");
    assert_eq!(leg_b.fwmark, Some(0x1), "leg-B carries MTLS_TPROXY_MARK");
    drop(server);
    node.finish().await;
}

/// A leg-B dial to a peer outside the guest prefix sets neither option: the
/// far peer (behind a veth in its own namespace) accepts a connection whose
/// source is the host's address on that link, not `127.0.0.1` (no bind), and
/// the connection reached the namespace at all (a marked dial would have been
/// delivered locally on `lo` by table 100).
///
/// CONTRACT_SHAPE: bounded-change. Universe: the far peer's accepted
/// connections; expected: exactly one, from the host's link address.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER step 13"]
async fn a_dial_to_a_peer_outside_the_guest_prefix_is_neither_marked_nor_bound() {
    let node = Node::start("legb-outside").await;
    let peer = PeerNetns::create("legb-outside");
    let accepted = peer.accept_one(SERVICE_PORT);
    let target = SocketAddrV4::new(peer.peer_addr(), SERVICE_PORT);
    assert!(
        !GUEST_NODE_PREFIX.contains(target.ip()),
        "precondition: the peer is outside the prefix"
    );
    let (_workload, leg_f) = leg_f_pair();

    // The far peer is a plain listener, so leg-B's handshake does not
    // complete; the dial's path is what this cell observes.
    let result = node.enforcement().enforce(outbound(leg_f, &node.client_alloc(), target)).await;
    node.record("enforce", format_args!("{:?}", result.as_ref().err()));

    let source = accepted.join().expect("peer thread").expect("the far peer accepted the dial");
    assert_eq!(*source.ip(), peer.host_addr(), "the dial left from the host's link address");
    assert_ne!(
        *source.ip(),
        Ipv4Addr::LOCALHOST,
        "an outside-prefix dial is not bound to 127.0.0.1"
    );
    node.finish().await;
}

/// A same-node dial to a guest-prefix tuple no allocation registered is
/// refused at once — TCP `ECONNREFUSED` from the constant guest-prefix output
/// reject (ADR-0172) — and reaches no host socket: a wildcard listener on the
/// same port accepts nothing (V-LEGB-MARK S3).
///
/// CONTRACT_SHAPE: bounded-change. Universe: `enforce`'s result and its
/// latency, the wildcard listener's accepts; expected: `Io(ConnectionRefused)`
/// within the bound, no accept.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER step 13"]
async fn a_marked_dial_to_an_unregistered_tuple_is_refused_and_reaches_no_host_socket() {
    let node = Node::start("legb-unregistered").await;
    let _server = node.serve_vm("legb-server", VM_ADDR, SERVICE_PORT).await;
    let wildcard = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, UNDECLARED_PORT))
        .expect("precondition: a wildcard host listener on the undeclared port");
    wildcard.set_nonblocking(true).expect("non-blocking wildcard listener");
    let (_workload, leg_f) = leg_f_pair();

    let started = Instant::now();
    let refused = node
        .enforcement()
        .enforce(outbound(leg_f, &node.client_alloc(), SocketAddrV4::new(VM_ADDR, UNDECLARED_PORT)))
        .await
        .expect_err("an unregistered guest-prefix tuple is refused");
    let elapsed = started.elapsed();
    node.record("enforce", format_args!("{refused:?} after {elapsed:?}"));

    assert!(
        matches!(&refused, MtlsEnforcementError::Io(e) if e.kind() == std::io::ErrorKind::ConnectionRefused),
        "the constant output reject answers ECONNREFUSED, got {refused:?}"
    );
    assert!(elapsed < VERDICT_BOUND, "refused at once, not after a timeout: {elapsed:?}");
    match wildcard.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Ok((_stream, from)) => panic!("the refused dial reached a host socket from {from}"),
        Err(e) => panic!("wildcard accept failed: {e}"),
    }
    node.finish().await;
}

// ---------------------------------------------------------------------------
// Shared node fixture (also used by `vsock_constant_guest_prefix_rules`)
// ---------------------------------------------------------------------------

/// The production node composition of these Tier-3 files: the real
/// `MtlsInterceptWorker` over `HostMtlsIntercept` and `HostMtlsEnforcement`,
/// a test PKI, and RAII scrubbing of the node-global intercept state.
/// DISTILL test support, not production API.
pub(super) mod node {
    use std::collections::BTreeMap;
    use std::fmt::Display;
    use std::io::{self, Read as _};
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
    use std::num::NonZeroU16;
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use overdrive_core::guest_transport::{
        GUEST_CID_BASE, GUEST_FLOW_CONTROL_PORT, GUEST_FLOW_STREAM_PORT, GUEST_NODE_GATEWAY,
        GUEST_NODE_PREFIX, GuestCid, GuestFlowPorts, GuestTransportAssignment, LaunchToken,
    };
    use overdrive_core::traits::IdentityRead;
    use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
    use overdrive_core::traits::driver::{AllocationSpec, DriverPayload, Resources, VmPayload};
    use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
    use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
    use overdrive_core::wall_clock::UnixInstant;
    use overdrive_core::{AllocationId, CertSerial, SpiffeId};
    use overdrive_dataplane::mtls::HostMtlsEnforcement;
    use overdrive_sim::adapters::SimMtlsResolve;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_testing::cidr_lease::TestCidrLease;
    use overdrive_worker::mtls_intercept_port::HostMtlsIntercept;
    use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    use super::super::inbound_tproxy_harness::{
        KernelStateLock, clean_shared_infra, is_root, record_uname,
    };
    use super::super::vsock_mesh_egress_mtls::{
        HeldIdentities, MintedCa, PEER_SNI, first_cert_der,
    };

    const CLIENT_SPIFFE: &str = "spiffe://overdrive.local/ns/default/sa/legb-client";
    const SERVER_SPIFFE: &str = "spiffe://overdrive.local/ns/default/sa/legb-server";

    /// Scrubs the node-global intercept state on drop (declared after the
    /// kernel lock in [`Node`], so it drops while the lock is held).
    struct SharedInfraCleanup;

    impl Drop for SharedInfraCleanup {
        fn drop(&mut self) {
            clean_shared_infra();
        }
    }

    /// The test PKI: one root → intermediate chain; a client leaf
    /// (`ClientAuth`) for the dialling allocation and a server leaf
    /// (`ServerAuth`, DNS SAN = leg-B's fixed SNI) for the VM allocation.
    pub struct Pki {
        /// The held identities, keyed by allocation (both allocations).
        pub held: Arc<HeldIdentities>,
        /// The client leaf (DER) and key, for a remote peer playing a
        /// client of the VM's service.
        pub client_chain: Vec<CertificateDer<'static>>,
        /// The client leaf's private key.
        pub client_key: PrivateKeyDer<'static>,
        /// The root certificate, the trust anchor of every handshake.
        pub root: CertificateDer<'static>,
    }

    fn svid(cert_pem: String, key_pem: String, spiffe: &str) -> SvidMaterial {
        let der = first_cert_der(&cert_pem);
        SvidMaterial::new(
            CaCertPem::new(cert_pem),
            CaCertDer::new(der.as_ref().to_vec()),
            CertSerial::new("0b0c0d0e").expect("valid serial"),
            spiffe.parse::<SpiffeId>().expect("valid spiffe id"),
            CaKeyPem::new(key_pem),
            UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800)),
        )
    }

    impl Pki {
        fn mint(client: &AllocationId, server: &AllocationId) -> Self {
            let root = MintedCa::root("overdrive-legb-ROOT-CA");
            let intermediate = root.intermediate("overdrive-legb-INTERMEDIATE-CA");
            let (client_pem, client_key_pem, client_der, client_key) =
                intermediate.leaf(CLIENT_SPIFFE, None, true);
            let (server_pem, server_key_pem, _, _) =
                intermediate.leaf(SERVER_SPIFFE, Some(PEER_SNI), false);
            let bundle = TrustBundle::new(
                CaCertPem::new(root.cert_pem.clone()),
                Some(CaCertPem::new(intermediate.cert_pem.clone())),
            );
            let mut svids = BTreeMap::new();
            svids.insert(client.clone(), svid(client_pem, client_key_pem, CLIENT_SPIFFE));
            svids.insert(server.clone(), svid(server_pem, server_key_pem, SERVER_SPIFFE));
            Self {
                held: Arc::new(HeldIdentities { svids, bundle }),
                client_chain: vec![client_der, CertificateDer::from(intermediate.cert_der.clone())],
                client_key,
                root: CertificateDer::from(root.cert_der.clone()),
            }
        }
    }

    /// One production node: the mTLS worker over the real intercept and
    /// enforcement adapters, with the node-shared owner published. Field
    /// order is drop order: the worker first, the scrub next, the lock last.
    pub struct Node {
        worker: Arc<MtlsInterceptWorker>,
        enforcement: Arc<HostMtlsEnforcement>,
        client: AllocationId,
        server: AllocationId,
        /// The test PKI.
        pub pki: Pki,
        tag: &'static str,
        started: Instant,
        _cleanup: SharedInfraCleanup,
        _kernel_lock: KernelStateLock,
    }

    impl Node {
        /// Compose and publish the node. `HostMtlsEnforcement::new` takes
        /// the node's guest prefix (`GUEST_NODE_PREFIX`, as
        /// `run_server_with_obs_and_drivers` passes it).
        pub async fn start(tag: &'static str) -> Self {
            assert!(is_root(), "{tag}: Tier-3 evidence requires root and CAP_NET_ADMIN");
            record_uname(tag);
            if rustls::crypto::ring::default_provider().install_default().is_err() {
                // An earlier body in this process installed the provider.
            }
            let kernel_lock = KernelStateLock::acquire();
            clean_shared_infra();
            let cleanup = SharedInfraCleanup;
            let client = AllocationId::new(&format!("{tag}-client")).expect("allocation id");
            let server = AllocationId::new(&format!("{tag}-server")).expect("allocation id");
            let pki = Pki::mint(&client, &server);
            let enforcement = Arc::new(HostMtlsEnforcement::new(
                Arc::clone(&pki.held) as Arc<dyn IdentityRead>,
                MtlsLimits::default(),
                GUEST_NODE_PREFIX,
                Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
            ));
            enforcement.probe().await.expect("the enforcement probe passes before any flow");
            let resolve: Arc<dyn MtlsResolve> =
                Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
            let worker = Arc::new(MtlsInterceptWorker::new(
                Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
                resolve,
                Arc::new(SimClock::new()),
                Arc::new(HostMtlsIntercept::new()),
            ));
            worker.start_shared_owner().await.expect("publish the node-shared owner");
            Self {
                worker,
                enforcement,
                client,
                server,
                pki,
                tag,
                started: Instant::now(),
                _cleanup: cleanup,
                _kernel_lock: kernel_lock,
            }
        }

        /// The leg-B enforcement driving port.
        pub fn enforcement(&self) -> &HostMtlsEnforcement {
            &self.enforcement
        }

        /// The dialling allocation (its SVID is held).
        pub fn client_alloc(&self) -> AllocationId {
            self.client.clone()
        }

        /// Make the VM allocation intercept-live with `addr` and one declared
        /// service port: production registers `(addr, port)` in
        /// `inbound_destinations` and installs its divert to leg-C.
        pub async fn serve_vm(&self, label: &str, addr: Ipv4Addr, port: u16) -> ServedVm {
            let spec = vm_spec(&self.server, addr, port);
            self.worker.start_alloc(&spec).await.expect("start_alloc makes the VM intercept-live");
            self.record("serve_vm", format_args!("{label} {addr}:{port}"));
            ServedVm { _addr: SocketAddrV4::new(addr, port) }
        }

        /// One append-only evidence line.
        pub fn record(&self, phase: &str, detail: impl Display) {
            eprintln!(
                "[{}][{phase}][+{}ms] {detail}",
                self.tag,
                self.started.elapsed().as_millis()
            );
        }

        /// Shut the node-shared owner down.
        pub async fn finish(self) {
            self.worker.shutdown_owner().await.expect("the node-shared owner shuts down");
        }
    }

    /// A VM allocation made intercept-live (held for the cell's life).
    pub struct ServedVm {
        _addr: SocketAddrV4,
    }

    /// The `AllocationSpec` of a VM allocation at `addr` declaring `port`.
    pub fn vm_spec(alloc: &AllocationId, addr: Ipv4Addr, port: u16) -> AllocationSpec {
        let [_, _, high, low] = addr.octets();
        let cid = GuestCid::new(GUEST_CID_BASE + u32::from(u16::from_be_bytes([high, low])))
            .expect("an in-range guest CID");
        let launch = LaunchToken::from_bytes([0x6b; 16]).expect("a non-zero launch token");
        AllocationSpec {
            alloc: alloc.clone(),
            identity: SERVER_SPIFFE.parse().expect("valid spiffe id"),
            driver: DriverPayload::Vm(VmPayload {
                command: "/bin/true".to_owned(),
                args: vec![],
                kernel: "/nonexistent/kernel".into(),
                rootfs: "/nonexistent/rootfs".into(),
            }),
            resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
            probe_descriptors: Vec::new(),
            transport: Some(GuestTransportAssignment::new(
                addr,
                GUEST_NODE_GATEWAY,
                cid,
                launch,
                GuestFlowPorts::new(GUEST_FLOW_STREAM_PORT, GUEST_FLOW_CONTROL_PORT)
                    .expect("the default guest-flow listener ports"),
            )),
            service_ports: vec![NonZeroU16::new(port).expect("a non-zero service port")],
        }
    }

    /// One established TCP socket as `ss -Htne` reports it.
    #[derive(Debug, Clone)]
    pub struct SocketRow {
        /// Local address.
        pub local: SocketAddrV4,
        /// Peer address.
        pub peer: SocketAddrV4,
        /// `fwmark`, when the socket carries one.
        pub fwmark: Option<u32>,
    }

    /// Every established IPv4 TCP socket whose local or peer port is `port`.
    pub fn established_sockets(port: u16) -> Vec<SocketRow> {
        let filter = format!("( sport = :{port} or dport = :{port} )");
        let out = Command::new("ss")
            .args(["-H", "-4", "-t", "-n", "-e", "state", "established", &filter])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("precondition: ss is runnable");
        assert!(out.status.success(), "ss failed: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                // Recv-Q Send-Q Local Peer [extended fields…]
                let local = fields.get(2)?.parse().ok()?;
                let peer = fields.get(3)?.parse().ok()?;
                let fwmark = fields.iter().find_map(|f| {
                    f.strip_prefix("fwmark:0x").and_then(|hex| u32::from_str_radix(hex, 16).ok())
                });
                Some(SocketRow { local, peer, fwmark })
            })
            .collect()
    }

    /// A far peer behind a veth pair in its own network namespace, addressed
    /// from a [`TestCidrLease`] `/24` (route-disjoint from production). The
    /// host end holds the lease gateway; the namespace end the lease
    /// workload address, with a route to the guest prefix via the host.
    pub struct PeerNetns {
        name: String,
        host_if: String,
        lease: TestCidrLease,
    }

    fn ip(args: &[&str]) {
        let out = Command::new("ip").args(args).output().expect("precondition: ip is runnable");
        assert!(
            out.status.success(),
            "precondition: ip {} succeeds: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    fn ip_cleanup(args: &[&str]) {
        match Command::new("ip").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status() {
            Ok(status) if status.success() => {}
            Ok(status) => eprintln!("cleanup: ip {} exited {status}", args.join(" ")),
            Err(e) => eprintln!("cleanup: ip {} could not run: {e}", args.join(" ")),
        }
    }

    impl PeerNetns {
        /// Create the namespace, the veth pair and the addresses.
        pub fn create(label: &str) -> Self {
            let lease = TestCidrLease::acquire(&format!("worker-{label}"))
                .expect("acquire the peer namespace CIDR lease");
            let tag = format!("{:05x}", std::process::id() & 0xf_ffff);
            let name = format!("ovt-{label}-{tag}");
            let host_if = format!("ovtH{tag}");
            let ns_if = format!("ovtN{tag}");
            ip_cleanup(&["netns", "del", &name]);
            ip_cleanup(&["link", "del", &host_if]);
            ip(&["netns", "add", &name]);
            ip(&["link", "add", &host_if, "type", "veth", "peer", "name", &ns_if]);
            ip(&["link", "set", &ns_if, "netns", &name]);
            let gw = format!("{}/{}", lease.host_gateway(), lease.prefix_len());
            let wl = format!("{}/{}", lease.workload_addr(), lease.prefix_len());
            ip(&["addr", "add", &gw, "dev", &host_if]);
            ip(&["link", "set", &host_if, "up"]);
            ip(&["-n", &name, "addr", "add", &wl, "dev", &ns_if]);
            ip(&["-n", &name, "link", "set", &ns_if, "up"]);
            ip(&["-n", &name, "link", "set", "lo", "up"]);
            let prefix = GUEST_NODE_PREFIX.to_string();
            let via = lease.host_gateway().to_string();
            ip(&["-n", &name, "route", "add", &prefix, "via", &via]);
            Self { name, host_if, lease }
        }

        /// The namespace end's address.
        pub fn peer_addr(&self) -> Ipv4Addr {
            self.lease.workload_addr()
        }

        /// The host end's address.
        pub fn host_addr(&self) -> Ipv4Addr {
            self.lease.host_gateway()
        }

        /// Run `f` on a thread inside the namespace.
        pub fn spawn_in<T: Send + 'static>(
            &self,
            f: impl FnOnce() -> T + Send + 'static,
        ) -> JoinHandle<T> {
            let path = format!("/var/run/netns/{}", self.name);
            std::thread::spawn(move || {
                use std::os::fd::AsRawFd as _;
                let file = std::fs::File::open(&path)
                    .unwrap_or_else(|e| panic!("precondition: netns {path} opens: {e}"));
                // SAFETY: a valid netns descriptor; CLONE_NEWNET moves only
                // this thread.
                let rc = unsafe { libc::setns(file.as_raw_fd(), libc::CLONE_NEWNET) };
                assert_eq!(rc, 0, "precondition: setns {path}: {}", io::Error::last_os_error());
                f()
            })
        }

        /// Accept one TCP connection on `port` inside the namespace (bounded)
        /// and return its source, or `None` if none arrived.
        pub fn accept_one(&self, port: u16) -> JoinHandle<Option<SocketAddrV4>> {
            let addr = SocketAddrV4::new(self.peer_addr(), port);
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let handle = self.spawn_in(move || {
                let listener = TcpListener::bind(addr).expect("the far peer binds");
                listener.set_nonblocking(true).expect("non-blocking listener");
                ready_tx.send(()).expect("signal readiness");
                let deadline = Instant::now() + Duration::from_secs(5);
                while Instant::now() < deadline {
                    match listener.accept() {
                        Ok((mut stream, std::net::SocketAddr::V4(from))) => {
                            let mut sink = [0_u8; 256];
                            if stream.set_read_timeout(Some(Duration::from_millis(200))).is_ok() {
                                // Drain whatever leg-B sent (its ClientHello);
                                // the content is not the observation.
                                match stream.read(&mut sink) {
                                    Ok(_) | Err(_) => {}
                                }
                            }
                            return Some(from);
                        }
                        Ok((_, from)) => panic!("an IPv4 listener accepted {from}"),
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => panic!("far peer accept failed: {e}"),
                    }
                }
                None
            });
            ready_rx.recv().expect("the far peer is listening");
            handle
        }

        /// A TCP connect from inside the namespace to `target`, bounded by
        /// `timeout`: the connected stream, or the connect's error.
        pub fn connect_from(
            &self,
            target: SocketAddrV4,
            timeout: Duration,
        ) -> JoinHandle<io::Result<std::net::TcpStream>> {
            self.spawn_in(move || {
                std::net::TcpStream::connect_timeout(&std::net::SocketAddr::V4(target), timeout)
            })
        }
    }

    impl Drop for PeerNetns {
        fn drop(&mut self) {
            ip_cleanup(&["link", "del", &self.host_if]);
            ip_cleanup(&["netns", "del", &self.name]);
        }
    }
}
