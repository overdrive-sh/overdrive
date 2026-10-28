//! Tier-3 shared-capability mTLS enforcement on the real kernel: two shared
//! allocations' TLS handles are independent (stopping one keeps the other's
//! live), and the shared owner's shutdown closes admission, waits for the one
//! claim in flight and drains every shared handle.
//!
//! The TAP-era OUTBOUND enforce-substrate splice scenario that used to live here
//! drove workload egress through the per-allocation egress nft-TPROXY capture,
//! which the vsock attachment cut removes (netns-density-295 D14; the forwarded
//! outbound port replaces it — see `vsock_mesh_egress_mtls`). It was deleted
//! with its strace oracle, wire scan and topology helpers.

#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unnecessary_cast,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::match_wildcard_for_single_variants,
    clippy::missing_panics_doc,
    clippy::missing_const_for_fn,
    clippy::format_collect,
    reason = "Tier-3 outbound-substrate test body; the bidirectional-splice narrative in the module docstring is prose; skip messages + strace diagnostics go to stderr; failures must panic with informative messages; the libc FFI casts are width conversions on compile-time constants (ETH_P_ALL.to_be() as i32 mirrors traffic.rs); leg F/B are the ADR-0069 contract vocabulary; the single composed Tier-3 scenario drives the round-trip under one strace attach; the SocketAddr wildcard arm is the V6 case a v4-only fixture cannot hit; the per-byte \\xNN python-literal fold reads clearer than a write! accumulator in a test fixture; const-fn-ability on test constructors is not load-bearing"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::os::fd::AsRawFd as _;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GUEST_FLOW_CONTROL_PORT, GUEST_FLOW_STREAM_PORT, GuestCid, GuestFlowPorts,
    GuestTransportAssignment, LaunchToken,
};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::driver::{AllocationSpec, Resources};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, CertSerial};
use overdrive_dataplane::mtls::HostMtlsEnforcement;
use overdrive_netlink::nft::{SharedIpInterceptIdentity, SharedIpInterceptState};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::mtls_intercept::{InterceptPostcondition, Result as InterceptResult};
use overdrive_worker::mtls_intercept_port::{
    HostMtlsIntercept, InterceptGuard, InterceptListener, MtlsIntercept,
};
use overdrive_worker::mtls_intercept_worker::{ForwardedOutbound, MtlsInterceptWorker};

use async_trait::async_trait;
use overdrive_core::traits::mtls_resolve::{
    MtlsResolution, MtlsResolve, MtlsResolveError, ResolvedBackend,
};
use rcgen::string::Ia5String;
use rcgen::{CertificateParams, Issuer, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use super::leg_listener::LegListener;

// ============================================================================
// topology constants (mirror the increment-b egress spike + the 05-01/05-02 harness)
// ============================================================================

const NS_W: &str = "nsW-asym0503";
const VETH_H: &str = "vethH-asym05";

/// The mesh backend the OUTBOUND workload dials — a host-side lo-bound address it
/// routes to via the gateway, so its egress genuinely INGRESSES vethH and hits
/// PREROUTING. This is the dialed `orig_dst` the resolve consumer classifies
/// `Mesh`, and the address the real mesh mTLS server (leg-B's peer) binds.
const MESH_BACKEND_IP: &str = "10.200.0.1";
const MESH_BACKEND_PORT: u16 = 18831;

// ============================================================================
// Cross-process kernel-state exclusion (shared path with the sibling suites)
// ============================================================================

/// Cross-PROCESS exclusion for the shared host-netns kernel state. The
/// `overdrive-mtls` nft table, the fwmark ip-rule, and the table-100 local route
/// are NODE-GLOBAL. nextest runs each `#[test]` in a SEPARATE PROCESS, so an
/// in-process lock cannot serialise them — an `flock(2)` on the fixed path (shared
/// with `egress_tproxy_capture.rs` / `bidirectional_walking_skeleton.rs`) spans
/// processes.
struct KernelStateLock {
    fd: std::os::fd::OwnedFd,
}

impl KernelStateLock {
    fn acquire() -> Self {
        use std::os::fd::FromRawFd as _;
        let path = c"/tmp/overdrive-mtls-kernel-state.lock";
        // SAFETY: open with O_CREAT|O_RDWR on a fixed path; the returned fd is
        // adopted by OwnedFd. flock blocks until the exclusive lock is held.
        let fd = unsafe {
            let raw = libc::open(path.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
            assert!(raw >= 0, "open kernel-state lock file: {}", std::io::Error::last_os_error());
            let rc = libc::flock(raw, libc::LOCK_EX);
            assert!(rc == 0, "flock LOCK_EX: {}", std::io::Error::last_os_error());
            std::os::fd::OwnedFd::from_raw_fd(raw)
        };
        Self { fd }
    }
}

impl Drop for KernelStateLock {
    fn drop(&mut self) {
        // SAFETY: fd is the live lock fd; LOCK_UN releases the advisory lock.
        unsafe {
            libc::flock(self.fd.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// True iff this process is uid 0 (root). IP_TRANSPARENT, nft, `ip netns`, and
/// `ip rule` all need root + CAP_NET_ADMIN/CAP_SYS_ADMIN; a non-root run cannot
/// stand up the fixture, so we SKIP rather than fail.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

// ============================================================================
// command shims (mirror egress_tproxy_capture.rs / bidirectional_walking_skeleton.rs)
// ============================================================================

fn ip(args: &[&str]) {
    let out = Command::new("ip")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn ip");
    assert!(
        out.status.success(),
        "ip {args:?} exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
}

fn ip_quiet(args: &[&str]) {
    let _ = Command::new("ip").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Scrub ALL `overdrive-mtls` nft state + the shared fwmark rule/route so a
/// clean-kernel ground-truth run is reproducible. Run at test START (tolerate
/// pre-existing) AND END. Best-effort: every failure is "nothing to clean".
fn clean_shared_infra() {
    for _ in 0..64 {
        let ok = Command::new("ip")
            .args(["rule", "del", "fwmark", "0x1", "lookup", "100"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            break;
        }
    }
    ip_quiet(&["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"]);
    let _ = Command::new("nft")
        .args(["delete", "table", "ip", "overdrive-mtls"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Tear down the per-test netns + veth pair + the lo-bound mesh backend addr. The
/// shared `overdrive-mtls` infra is handled by `clean_shared_infra`.
fn teardown_topology() {
    ip_quiet(&["link", "del", VETH_H]);
    ip_quiet(&["netns", "del", NS_W]);
    ip_quiet(&["addr", "del", &format!("{MESH_BACKEND_IP}/32"), "dev", "lo"]);
}

// ============================================================================
// Fresh focused PKI (re-authored — replicates the dataplane `mtls_pki.rs` reference
// + the 05-01 walking-skeleton: a real root → intermediate → leaf chain)
// ============================================================================

/// A minted leaf — the PEM cert + key + the SPIFFE SAN, plus the DER forms.
struct Leaf {
    cert_pem: String,
    key_pem: String,
    cert_der: CertificateDer<'static>,
    key_der: PrivateKeyDer<'static>,
    spiffe: overdrive_core::SpiffeId,
    serial: CertSerial,
}

/// The shared test PKI: root self-signs; intermediate signed by root; every leaf
/// signed by the intermediate (production issuance shape).
struct TestPki {
    ca_cert_pem: String,
    intermediate_cert_pem: String,
    intermediate_cert_der: CertificateDer<'static>,
    /// The OUTBOUND client SVID (workload-as-client; the agent presents on leg-B).
    client_leaf: Leaf,
    /// The OUTBOUND real mesh peer leaf: a SERVER cert with a DNS SAN matching the
    /// fixed leg-B SNI (`peer.overdrive.local`, per mtls/outbound.rs) so the agent's
    /// leg-B client handshake verifies the mesh server's cert.
    peer_leaf: Leaf,
    client_alloc: AllocationId,
}

impl TestPki {
    /// The DNS SAN the OUTBOUND mesh peer presents (matches the FIXED SNI the
    /// adapter's leg-B client handshake uses in `mtls::outbound::client_handshake` —
    /// `peer.overdrive.local`).
    const PEER_SNI: &'static str = "peer.overdrive.local";

    fn mint() -> Self {
        let root = MintedCa::mint_root("overdrive-mtls-05-03-ROOT-CA");
        let intermediate = root.mint_intermediate("overdrive-mtls-05-03-INTERMEDIATE-CA");

        let client_spiffe = "spiffe://overdrive.local/ns/default/sa/client";
        let client_leaf = intermediate.mint_leaf(client_spiffe, None, true);
        let peer_leaf = intermediate.mint_leaf(
            "spiffe://overdrive.local/ns/default/sa/peer",
            Some(Self::PEER_SNI),
            false,
        );

        Self {
            ca_cert_pem: root.cert_pem,
            intermediate_cert_pem: intermediate.cert_pem.clone(),
            intermediate_cert_der: CertificateDer::from(intermediate.cert_der),
            client_leaf,
            peer_leaf,
            client_alloc: AllocationId::new("alloc-asym-client").expect("valid alloc"),
        }
    }

    fn ca_cert_pem(&self) -> &str {
        &self.ca_cert_pem
    }

    fn intermediate_cert_der(&self) -> CertificateDer<'static> {
        self.intermediate_cert_der.clone()
    }

    /// The shared trust bundle: root anchor = the ROOT; intermediate chain material
    /// = the INTERMEDIATE (the agent reads this via `IdentityRead`).
    fn trust_bundle(&self) -> TrustBundle {
        TrustBundle::new(
            CaCertPem::new(self.ca_cert_pem.clone()),
            Some(CaCertPem::new(self.intermediate_cert_pem.clone())),
        )
    }

    fn client_svid_material(&self) -> SvidMaterial {
        svid_from_leaf(&self.client_leaf)
    }
}

/// A minted signing authority (root OR intermediate) retaining its
/// `CertificateParams` + `KeyPair` so it can build a reusable rcgen 0.14 `Issuer`.
struct MintedCa {
    params: CertificateParams,
    key: KeyPair,
    cert_pem: String,
    cert_der: Vec<u8>,
}

impl MintedCa {
    fn mint_root(cn: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name.push(rcgen::DnType::CommonName, cn);
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = params.self_signed(&key).unwrap();
        let cert_pem = cert.pem();
        let cert_der = cert.der().to_vec();
        Self { params, key, cert_pem, cert_der }
    }

    fn mint_intermediate(&self, cn: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        params.distinguished_name.push(rcgen::DnType::CommonName, cn);
        params.use_authority_key_identifier_extension = true;
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let root_issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&key, &root_issuer).unwrap();
        let cert_pem = cert.pem();
        let cert_der = cert.der().to_vec();
        Self { params, key, cert_pem, cert_der }
    }

    fn mint_leaf(&self, spiffe: &str, dns_san: Option<&str>, client_auth: bool) -> Leaf {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        let uri = Ia5String::try_from(spiffe).expect("spiffe URI is a valid IA5 string");
        let mut sans = vec![SanType::URI(uri)];
        if let Some(dns) = dns_san {
            let dns_ia5 = Ia5String::try_from(dns).expect("dns SAN is a valid IA5 string");
            sans.push(SanType::DnsName(dns_ia5));
        }
        params.subject_alt_names = sans;
        params.distinguished_name.push(rcgen::DnType::CommonName, spiffe);
        params.use_authority_key_identifier_extension = true;
        params.extended_key_usages = if client_auth {
            vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth]
        } else {
            vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth]
        };
        let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&leaf_key, &issuer).unwrap();
        let cert_pem = cert.pem();
        let key_pem = leaf_key.serialize_pem();
        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        Leaf {
            cert_pem,
            key_pem,
            cert_der,
            key_der,
            spiffe: spiffe.parse().expect("valid spiffe id"),
            serial: CertSerial::new("0a0b0c0d").expect("valid serial"),
        }
    }
}

/// Assemble `SvidMaterial` from a minted leaf (cert PEM/DER + leaf key PEM +
/// far-future `not_after`).
fn svid_from_leaf(leaf: &Leaf) -> SvidMaterial {
    let not_after = UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800)); // 2100
    SvidMaterial::new(
        CaCertPem::new(leaf.cert_pem.clone()),
        CaCertDer::new(leaf.cert_der.as_ref().to_vec()),
        leaf.serial.clone(),
        leaf.spiffe.clone(),
        CaKeyPem::new(leaf.key_pem.clone()),
        not_after,
    )
}

/// The agent's held-identity store — the ONLY holder of SVID material (workloads
/// hold nothing; the agent reads through THIS `IdentityRead` port and NEVER mints,
/// #26 is a reader). `None` is explicit absence.
struct HeldIdentities {
    svids: BTreeMap<AllocationId, SvidMaterial>,
    bundle: TrustBundle,
}

impl IdentityRead for HeldIdentities {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        self.svids.get(alloc).cloned()
    }

    fn current_bundle(&self) -> Option<TrustBundle> {
        Some(self.bundle.clone())
    }
}

fn held_identities_for(pki: &TestPki, allocations: &[AllocationId]) -> HeldIdentities {
    let material = pki.client_svid_material();
    let svids =
        allocations.iter().cloned().map(|allocation| (allocation, material.clone())).collect();
    HeldIdentities { svids, bundle: pki.trust_bundle() }
}

/// The `AllocationSpec` the OUTBOUND production `start_alloc` consumes: keyed on the
/// CLIENT alloc id (so production's `enforce` selects the held client SVID for the
/// leg-B handshake) with `host_veth = Some(VETH_H)` (the channel the action-shim C3
/// provision seam sets in production — drives the egress nft-TPROXY install matching
/// `iifname VETH_H`).
fn build_client_spec(pki: &TestPki, _host_veth: Option<String>) -> AllocationSpec {
    AllocationSpec {
        alloc: pki.client_alloc.clone(),
        identity: pki.client_leaf.spiffe.clone(),
        driver: overdrive_core::traits::driver::DriverPayload::Vm(
            overdrive_core::traits::driver::VmPayload {
                command: "/bin/true".to_owned(),
                args: vec![],
                kernel: "/nonexistent/kernel".into(),
                rootfs: "/nonexistent/rootfs".into(),
            },
        ),
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        transport: None,
        service_ports: Vec::new(),
    }
}

// ---- shared TLS + socket helpers (re-authored from the 05-01 skeleton) ----

fn ca_root_store(ca_cert_pem: &str) -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    let mut rd = std::io::BufReader::new(ca_cert_pem.as_bytes());
    for c in rustls_pemfile::certs(&mut rd) {
        roots.add(c.expect("ca cert")).expect("add ca cert");
    }
    roots
}

fn drive_server_handshake(
    conn: &mut rustls::ServerConnection,
    tcp: &mut std::net::TcpStream,
) -> bool {
    use std::io::ErrorKind;
    loop {
        while conn.wants_write() {
            if conn.write_tls(tcp).is_err() {
                return false;
            }
        }
        if !conn.is_handshaking() {
            while conn.wants_write() {
                if conn.write_tls(tcp).is_err() {
                    return false;
                }
            }
            return true;
        }
        match conn.read_tls(tcp) {
            Ok(0) => return false,
            Ok(_) => {
                if conn.process_new_packets().is_err() {
                    return false;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => return false,
        }
    }
}

/// Accept one connection within `timeout` by polling a non-blocking accept.
fn accept_with_timeout(
    listener: &TcpListener,
    timeout: Duration,
) -> std::io::Result<(std::net::TcpStream, std::net::SocketAddr)> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    let result = loop {
        match listener.accept() {
            Ok(pair) => {
                pair.0.set_nonblocking(false).ok();
                break Ok(pair);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    break Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "no connection within timeout",
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => break Err(e),
        }
    };
    listener.set_nonblocking(false).ok();
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MetalSharedOwnerSurface {
    binds: usize,
    converges: usize,
    observes: usize,
    addresses: Vec<SocketAddrV4>,
    node_guard_drops: usize,
    allocation_guard_drops: usize,
}

struct MetalOwnedGuard {
    inner: Option<Box<dyn InterceptGuard>>,
    drops: Arc<AtomicUsize>,
}

impl InterceptGuard for MetalOwnedGuard {}

impl Drop for MetalOwnedGuard {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        drop(self.inner.take());
    }
}

struct MetalSharedIntercept {
    inner: HostMtlsIntercept,
    binds: AtomicUsize,
    converges: AtomicUsize,
    observes: AtomicUsize,
    addresses: parking_lot::Mutex<Vec<SocketAddrV4>>,
    node_guard_drops: Arc<AtomicUsize>,
    allocation_guard_drops: Arc<AtomicUsize>,
}

impl MetalSharedIntercept {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: HostMtlsIntercept::new(),
            binds: AtomicUsize::new(0),
            converges: AtomicUsize::new(0),
            observes: AtomicUsize::new(0),
            addresses: parking_lot::Mutex::new(Vec::new()),
            node_guard_drops: Arc::new(AtomicUsize::new(0)),
            allocation_guard_drops: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn surface(&self) -> MetalSharedOwnerSurface {
        MetalSharedOwnerSurface {
            binds: self.binds.load(Ordering::SeqCst),
            converges: self.converges.load(Ordering::SeqCst),
            observes: self.observes.load(Ordering::SeqCst),
            addresses: self.addresses.lock().clone(),
            node_guard_drops: self.node_guard_drops.load(Ordering::SeqCst),
            allocation_guard_drops: self.allocation_guard_drops.load(Ordering::SeqCst),
        }
    }

    fn wrap(guard: Box<dyn InterceptGuard>, drops: &Arc<AtomicUsize>) -> Box<dyn InterceptGuard> {
        Box::new(MetalOwnedGuard { inner: Some(guard), drops: Arc::clone(drops) })
    }
}

/// Listener type returned by the test-local intercept double.
type BoundListener = Arc<dyn InterceptListener>;

impl MtlsIntercept for MetalSharedIntercept {
    fn bind_transparent(&self, address: SocketAddrV4) -> InterceptResult<BoundListener> {
        let listener = self.inner.bind_transparent(address)?;
        let bound = listener.bound_v4().expect("bound shared-listener IPv4 address");
        self.binds.fetch_add(1, Ordering::SeqCst);
        self.addresses.lock().push(bound);
        Ok(listener)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.converges.fetch_add(1, Ordering::SeqCst);
        self.inner
            .converge_shared(prior, leg_f, leg_c)
            .map(|guard| Self::wrap(guard, &self.node_guard_drops))
    }

    fn observe_shared(&self) -> InterceptResult<Option<InterceptPostcondition>> {
        self.observes.fetch_add(1, Ordering::SeqCst);
        self.inner.observe_shared()
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        leg_c_port: u16,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.inner
            .install_inbound(virt, leg_c_port)
            .map(|guard| Self::wrap(guard, &self.allocation_guard_drops))
    }

    fn observe_shared_state(
        &self,
    ) -> InterceptResult<Option<overdrive_worker::mtls_intercept_port::InterceptState>> {
        self.inner.observe_shared_state()
    }

    fn converge_allocation_elements(
        &self,
        expected: &overdrive_worker::mtls_intercept_port::InterceptMembers,
    ) -> InterceptResult<Option<overdrive_worker::mtls_intercept_port::InterceptState>> {
        self.inner.converge_allocation_elements(expected)
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> InterceptResult<overdrive_worker::mtls_intercept_port::InterceptState> {
        self.inner.remove_allocation_elements(source_addr, destinations)
    }
}

struct AllMeshResolve {
    peer: SocketAddrV4,
}

#[async_trait]
impl MtlsResolve for AllMeshResolve {
    async fn probe(&self) -> Result<(), MtlsResolveError> {
        Ok(())
    }

    async fn resolve(&self, _orig_dst: SocketAddrV4) -> Result<MtlsResolution, MtlsResolveError> {
        Ok(MtlsResolution::Mesh(ResolvedBackend { addr: self.peer, expected_svid: None }))
    }
}

fn shared_metal_spec(
    pki: &TestPki,
    allocation: AllocationId,
    address: std::net::Ipv4Addr,
) -> AllocationSpec {
    let mut spec = build_client_spec(pki, None);
    spec.alloc = allocation;
    let [_, _, high, low] = address.octets();
    let cid = GuestCid::new(GUEST_CID_BASE + u32::from(u16::from_be_bytes([high, low])))
        .expect("an in-range guest CID");
    let launch = LaunchToken::from_bytes([0x5a; 16]).expect("a non-zero launch token");
    spec.transport = Some(GuestTransportAssignment::new(
        address,
        std::net::Ipv4Addr::LOCALHOST,
        cid,
        launch,
        GuestFlowPorts::new(GUEST_FLOW_STREAM_PORT, GUEST_FLOW_CONTROL_PORT)
            .expect("the default guest-flow listener ports"),
    ));
    spec.service_ports = vec![std::num::NonZeroU16::new(8443).expect("non-zero port")];
    spec
}

fn shared_peer_server_config(pki: &TestPki) -> Arc<rustls::ServerConfig> {
    use rustls::server::WebPkiClientVerifier;
    let roots = Arc::new(ca_root_store(pki.ca_cert_pem()));
    let verifier = WebPkiClientVerifier::builder(roots).build().expect("client verifier");
    let mut config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![pki.peer_leaf.cert_der.clone(), pki.intermediate_cert_der()],
            pki.peer_leaf.key_der.clone_key(),
        )
        .expect("shared peer server config");
    config.send_tls13_tickets = 0;
    Arc::new(config)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SharedPeerEnd {
    AbortRequired,
    EofOrAbort,
}

fn is_abortive_reset(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
    )
}

fn shared_peer_connection(
    mut tcp: std::net::TcpStream,
    config: Arc<rustls::ServerConfig>,
    expected_end: SharedPeerEnd,
) -> bool {
    tcp.set_read_timeout(Some(Duration::from_secs(15))).ok();
    tcp.set_nodelay(true).ok();
    let mut connection = rustls::ServerConnection::new(config).expect("server connection");
    if !drive_server_handshake(&mut connection, &mut tcp) {
        return false;
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut buffer = [0_u8; 4096];
    while Instant::now() < deadline {
        let mut stream = rustls::Stream::new(&mut connection, &mut tcp);
        match stream.read(&mut buffer) {
            Ok(0) => return expected_end == SharedPeerEnd::EofOrAbort,
            Ok(read) => {
                if stream.write_all(&buffer[..read]).and_then(|()| stream.flush()).is_err() {
                    return false;
                }
            }
            Err(error) if is_abortive_reset(&error) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                return expected_end == SharedPeerEnd::EofOrAbort;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return false,
        }
    }
    false
}

struct SharedPeerOwner {
    join: Option<std::thread::JoinHandle<bool>>,
    ready: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    third_entered: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    third_release: std::sync::mpsc::Sender<()>,
}

impl SharedPeerOwner {
    fn spawn(pki: &TestPki, expected_ends: Vec<SharedPeerEnd>, block_third: bool) -> Self {
        let bind = SocketAddrV4::new(MESH_BACKEND_IP.parse().expect("mesh ip"), MESH_BACKEND_PORT);
        let config = shared_peer_server_config(pki);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let join = std::thread::spawn(move || {
            let listener = TcpListener::bind(bind).expect("bind shared mesh peer");
            listener.set_nonblocking(true).expect("nonblocking shared mesh peer");
            ready_tx.send(()).expect("publish shared peer readiness");
            let mut children = Vec::new();
            for (index, expected_end) in expected_ends.into_iter().enumerate() {
                let (tcp, _) = accept_with_timeout(&listener, Duration::from_secs(15))
                    .expect("accept shared mesh connection");
                if block_third && index == 2 {
                    entered_tx.send(()).expect("publish third-peer barrier");
                    release_rx.recv().expect("release third-peer barrier");
                }
                let config = Arc::clone(&config);
                children.push(std::thread::spawn(move || {
                    shared_peer_connection(tcp, config, expected_end)
                }));
            }
            children.into_iter().all(|child| child.join().is_ok_and(|result| result))
        });
        Self {
            join: Some(join),
            ready: std::sync::Mutex::new(ready_rx),
            third_entered: std::sync::Mutex::new(entered_rx),
            third_release: release_tx,
        }
    }

    async fn wait_ready(&self) {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let polled = self.ready.lock().expect("peer ready lock").try_recv();
                match polled {
                    Ok(()) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => tokio::task::yield_now().await,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        panic!("shared peer ended before publishing readiness")
                    }
                }
            }
        })
        .await
        .expect("shared peer binds before direct production exchange");
    }

    async fn wait_third(&self) {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let received = self.third_entered.lock().expect("third barrier lock").try_recv();
                match received {
                    Ok(()) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => tokio::task::yield_now().await,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        panic!("shared peer ended before third connection")
                    }
                }
            }
        })
        .await
        .expect("third real enforcement connection reaches the peer barrier");
    }

    fn release_third(&self) {
        self.third_release.send(()).expect("release third connection");
    }

    fn finish(mut self) {
        let _ = self.third_release.send(());
        assert!(self.join.take().expect("peer join").join().expect("peer thread joins"));
    }
}

impl Drop for SharedPeerOwner {
    fn drop(&mut self) {
        let _ = self.third_release.send(());
    }
}

async fn registered_shared_client(
    worker: &MtlsInterceptWorker,
    spec: &AllocationSpec,
    source: std::net::Ipv4Addr,
    target: SocketAddrV4,
    backend: SocketAddrV4,
) -> (ForwardedOutbound, tokio::net::TcpStream) {
    let socket = tokio::net::TcpSocket::new_v4().expect("create shared metal client");
    socket.bind(SocketAddrV4::new(source, 0).into()).expect("bind exact source capability");
    let forwarder_local = match socket.local_addr().expect("read registered forwarder address") {
        std::net::SocketAddr::V4(address) => address,
        std::net::SocketAddr::V6(address) => {
            unreachable!("an IPv4 registered forwarder reported {address}")
        }
    };
    let registration = worker
        .register_forwarded_outbound(
            &spec.alloc,
            forwarder_local,
            target,
            ResolvedBackend { addr: backend, expected_svid: None },
        )
        .expect("register the exact forwarded flow before connecting to leg-F");
    let stream = socket.connect(target.into()).await.expect("connect shared leg F");
    (registration, stream)
}

async fn assert_shared_exchange(stream: &mut tokio::net::TcpStream, marker: &[u8]) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    stream.write_all(marker).await.expect("send shared plaintext marker");
    let mut echoed = vec![0_u8; marker.len()];
    tokio::time::timeout(Duration::from_secs(15), stream.read_exact(&mut echoed))
        .await
        .expect("real enforcement returns the byte-distinct response")
        .expect("read shared response");
    assert_eq!(echoed, marker);
}

fn setup_shared_peer_address() {
    ip_quiet(&["addr", "del", &format!("{MESH_BACKEND_IP}/32"), "dev", "lo"]);
    ip(&["addr", "add", &format!("{MESH_BACKEND_IP}/32"), "dev", "lo"]);
}

fn observe_shared_intercept_state() -> SharedIpInterceptState {
    overdrive_netlink::nft::observe_shared_ip_intercept_state()
        .expect("typed shared-IP observation succeeds")
        .expect("the node-owned constant shared-IP program is present")
}

fn assert_shared_intercept_state(
    state: &SharedIpInterceptState,
    identity: &SharedIpInterceptIdentity,
    specs: &[&AllocationSpec],
) {
    let inbound = specs
        .iter()
        .flat_map(|spec| {
            let address =
                spec.transport.as_ref().expect("shared allocation transport").workload_addr();
            spec.service_ports.iter().map(move |port| SocketAddrV4::new(address, port.get()))
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        state.identity(),
        identity,
        "allocation changes preserve the exact constant program"
    );
    assert_eq!(state.inbound_destinations(), &inbound, "destination membership is exact");
    assert_eq!(
        state.inbound_destinations().len(),
        specs.iter().map(|spec| spec.service_ports.len()).sum::<usize>(),
        "the complete dynamic universe contains exactly P elements per allocation"
    );
}

struct SharedMetalCleanup;

impl Drop for SharedMetalCleanup {
    fn drop(&mut self) {
        clean_shared_infra();
        teardown_topology();
    }
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn two_real_shared_capabilities_keep_the_unrelated_tls_handle_live_after_one_stops() {
    assert!(is_root(), "S-ND295-25 native evidence requires root and CAP_NET_ADMIN");
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();
    setup_shared_peer_address();
    let _cleanup = SharedMetalCleanup;
    let pki = TestPki::mint();
    let first = AllocationId::new("metal-shared-first").expect("allocation id");
    let second = AllocationId::new("metal-shared-second").expect("allocation id");
    let identity: Arc<dyn IdentityRead> =
        Arc::new(held_identities_for(&pki, &[first.clone(), second.clone()]));
    let enforcement = Arc::new(HostMtlsEnforcement::new(
        identity,
        MtlsLimits::default(),
        overdrive_core::guest_transport::GUEST_NODE_PREFIX,
        Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
    ));
    enforcement.probe().await.expect("real enforcement probe");
    let intercept = MetalSharedIntercept::new();
    let peer = SharedPeerOwner::spawn(
        &pki,
        vec![SharedPeerEnd::AbortRequired, SharedPeerEnd::EofOrAbort],
        false,
    );
    peer.wait_ready().await;
    let backend = SocketAddrV4::new(MESH_BACKEND_IP.parse().expect("mesh ip"), MESH_BACKEND_PORT);
    let resolve: Arc<dyn MtlsResolve> = Arc::new(AllMeshResolve { peer: backend });
    let worker = Arc::new(MtlsInterceptWorker::new(
        Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
        resolve,
        Arc::new(SimClock::new()),
        Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
    ));
    worker.start_shared_owner().await.expect("publish real shared owner");
    let owner_before = intercept.surface();
    let leg_f = owner_before.addresses[0];
    let constant_identity = SharedIpInterceptIdentity::for_listener_ports(
        owner_before.addresses[0].port(),
        owner_before.addresses[1].port(),
    )
    .expect("non-zero shared listener ports form the exact constant program");
    assert_shared_intercept_state(&observe_shared_intercept_state(), &constant_identity, &[]);
    let first_spec = shared_metal_spec(&pki, first.clone(), "127.0.0.2".parse().unwrap());
    let second_spec = shared_metal_spec(&pki, second.clone(), "127.0.0.3".parse().unwrap());
    worker.start_alloc(&first_spec).await.expect("publish first real capability");
    worker.start_alloc(&second_spec).await.expect("publish second real capability");
    assert_shared_intercept_state(
        &observe_shared_intercept_state(),
        &constant_identity,
        &[&first_spec, &second_spec],
    );
    let (_first_registration, mut first_client) = registered_shared_client(
        &worker,
        &first_spec,
        "127.0.0.2".parse().unwrap(),
        leg_f,
        backend,
    )
    .await;
    let (_second_registration, mut second_client) = registered_shared_client(
        &worker,
        &second_spec,
        "127.0.0.3".parse().unwrap(),
        leg_f,
        backend,
    )
    .await;
    assert_shared_exchange(&mut first_client, b"metal-shared-first-before-stop").await;
    assert_shared_exchange(&mut second_client, b"metal-shared-second-before-stop").await;

    worker.stop_alloc(&first).await.expect("first real handle drains");
    assert_shared_intercept_state(
        &observe_shared_intercept_state(),
        &constant_identity,
        &[&second_spec],
    );
    assert_shared_exchange(&mut second_client, b"metal-shared-second-after-first-stop").await;
    worker.audit_shared_owner().await.expect("both shared listener tasks remain live");
    let owner_after = intercept.surface();
    assert_eq!(owner_after.binds, owner_before.binds);
    assert_eq!(owner_after.converges, owner_before.converges);
    assert_eq!(owner_after.addresses, owner_before.addresses);
    assert_eq!(owner_after.node_guard_drops, 0);
    assert_eq!(owner_after.allocation_guard_drops, 1);

    drop(first_client);
    drop(second_client);
    worker.stop_alloc(&second).await.expect("second real handle drains");
    assert_shared_intercept_state(&observe_shared_intercept_state(), &constant_identity, &[]);
    worker.shutdown_owner().await.expect("real shared owner joins");
    assert_eq!(intercept.surface().allocation_guard_drops, 2);
    assert_eq!(intercept.surface().node_guard_drops, 0);
    assert_shared_intercept_state(&observe_shared_intercept_state(), &constant_identity, &[]);
    peer.finish();
    clean_shared_infra();
    teardown_topology();
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn real_owner_shutdown_closes_admission_waits_one_claim_and_drains_every_shared_handle() {
    assert!(is_root(), "S-ND295-26 native evidence requires root and CAP_NET_ADMIN");
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();
    setup_shared_peer_address();
    let _cleanup = SharedMetalCleanup;
    let pki = TestPki::mint();
    let first = AllocationId::new("metal-shutdown-first").expect("allocation id");
    let second = AllocationId::new("metal-shutdown-second").expect("allocation id");
    let identity: Arc<dyn IdentityRead> =
        Arc::new(held_identities_for(&pki, &[first.clone(), second.clone()]));
    let enforcement = Arc::new(HostMtlsEnforcement::new(
        identity,
        MtlsLimits::default(),
        overdrive_core::guest_transport::GUEST_NODE_PREFIX,
        Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
    ));
    enforcement.probe().await.expect("real enforcement probe");
    let intercept = MetalSharedIntercept::new();
    let peer = SharedPeerOwner::spawn(&pki, vec![SharedPeerEnd::AbortRequired; 3], true);
    peer.wait_ready().await;
    let backend = SocketAddrV4::new(MESH_BACKEND_IP.parse().expect("mesh ip"), MESH_BACKEND_PORT);
    let resolve: Arc<dyn MtlsResolve> = Arc::new(AllMeshResolve { peer: backend });
    let worker = Arc::new(MtlsInterceptWorker::new(
        Arc::clone(&enforcement) as Arc<dyn MtlsEnforcement>,
        resolve,
        Arc::new(SimClock::new()),
        Arc::clone(&intercept) as Arc<dyn MtlsIntercept>,
    ));
    worker.start_shared_owner().await.expect("publish real shared owner");
    let addresses = intercept.surface().addresses;
    let constant_identity =
        SharedIpInterceptIdentity::for_listener_ports(addresses[0].port(), addresses[1].port())
            .expect("non-zero shared listener ports form the exact constant program");
    assert_shared_intercept_state(&observe_shared_intercept_state(), &constant_identity, &[]);
    let first_spec = shared_metal_spec(&pki, first, "127.0.0.4".parse().unwrap());
    let second_spec = shared_metal_spec(&pki, second, "127.0.0.5".parse().unwrap());
    worker.start_alloc(&first_spec).await.expect("publish first capability");
    worker.start_alloc(&second_spec).await.expect("publish second capability");
    assert_shared_intercept_state(
        &observe_shared_intercept_state(),
        &constant_identity,
        &[&first_spec, &second_spec],
    );
    let (_first_registration, mut first_client) = registered_shared_client(
        &worker,
        &first_spec,
        "127.0.0.4".parse().unwrap(),
        addresses[0],
        backend,
    )
    .await;
    let (_second_registration, mut second_client) = registered_shared_client(
        &worker,
        &second_spec,
        "127.0.0.5".parse().unwrap(),
        addresses[0],
        backend,
    )
    .await;
    assert_shared_exchange(&mut first_client, b"metal-shutdown-first-live").await;
    assert_shared_exchange(&mut second_client, b"metal-shutdown-second-live").await;
    let (_third_registration, third_client) = registered_shared_client(
        &worker,
        &second_spec,
        "127.0.0.5".parse().unwrap(),
        addresses[0],
        backend,
    )
    .await;
    peer.wait_third().await;

    let mut shutdown = tokio::spawn({
        let worker = Arc::clone(&worker);
        async move { worker.shutdown_owner().await }
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut shutdown).await.is_err(),
        "real owner waits for the enforcement-held claim"
    );
    peer.release_third();
    shutdown.await.expect("shutdown task joins").expect("all real handles and claims drain");
    drop(first_client);
    drop(second_client);
    drop(third_client);
    for address in addresses {
        drop(TcpListener::bind(address).expect("shutdown closes each exact shared socket"));
    }
    assert_eq!(intercept.surface().allocation_guard_drops, 2);
    assert_eq!(intercept.surface().node_guard_drops, 0);
    assert_shared_intercept_state(&observe_shared_intercept_state(), &constant_identity, &[]);
    peer.finish();
    clean_shared_infra();
    teardown_topology();
}
