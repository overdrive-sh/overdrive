//! P5-S10 — the mTLS forwarded-outbound port on the REAL `MtlsInterceptWorker`
//! (D9, D14). Tier-3 real-I/O adapter integration, `host-kernel-shared`.
//!
//! DISTILL (netns-density-295 vsock replacement), piece P5, scenario P5-S10;
//! gate G-V6. Spec:
//! `docs/feature/netns-density-295/distill/test-scenarios-p5-egress-unframe-hostinternal.md`.
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] mTLS forwarded-outbound port — `overdrive-worker::MtlsInterceptWorker`
//! (D9, D14)".
//!
//! ## What is driven, and what plays which role
//!
//! The system under test is the production `MtlsInterceptWorker`, composed
//! exactly as the sibling Tier-3 worker suites compose it: the real
//! `HostMtlsIntercept` (node-shared transparent leg-F/leg-C listeners + the
//! constant `overdrive-mtls` nft program, published by `start_shared_owner`),
//! the real `HostMtlsEnforcement` (kTLS mTLS substrate) reading a held
//! identity, and an allocation made intercept-live through the production
//! `start_alloc`. The only test-local port implementation is the `MtlsResolve`
//! double, [`CountingResolve`], which answers `MeshUnreachable` for every
//! address and counts its calls: if leg-F resolved a registered flow again, the
//! flow would fail closed and the counter would move.
//!
//! The guest-flow owner lives in `overdrive-control-plane`, which this crate
//! cannot depend on, so the test plays the owner's role ONLY at the port
//! boundary: it registers through `register_forwarded_outbound`, opens the
//! forwarder-side loopback socket bound to the registered `forwarder_local`,
//! and connects it to `leg_f_addr()`. The full `overdrive serve` +
//! `overdrive deploy` mesh cell is P6's walking skeleton (V-6).
//!
//! ## Egress-dialer discipline (CLAUDE.md § "East-west mTLS tests") — LOAD-BEARING
//! The forwarder socket stands in for the identity-unaware guest dialer: it
//! speaks PLAINTEXT toward leg-F and never presents client TLS (a client TLS
//! session toward leg-F is a peerless session the agent never terminates —
//! the handshake stalls). The mTLS proof is observed on the INTER-AGENT leg
//! (leg-F → remote mesh peer): the peer records the raw bytes it reads off
//! its TCP socket before TLS processing, and those carry TLS 1.3
//! `application_data` (`0x17`) records with zero cleartext. The remote peer
//! holds a real SVID chain from the test CA and requires a client certificate
//! chaining to the same CA. REQUEST and RESPONSE are byte-distinct so the
//! round-trip proves the real server→client reply pipe.
//!
//! ## H-5 (remote-side linger)
//! "An `overdrive serve` exit resets the remote peer instead of ending its
//! stream cleanly." A process exit is modelled faithfully by a CHILD PROCESS
//! (this test binary re-executed for the same test with
//! [`SERVE_CHILD_ENV`] set) that hosts the composition, carries a flow
//! mid-stream, and is then `SIGKILL`ed: the kernel closes every descriptor the
//! worker held, exactly as on a serve exit.
//!
//! ## Allocation transport (DG-2 rows 4, 5)
//! [`intercept_live_spec`] registers the allocation through
//! `AllocationSpec.transport` (`GuestTransportAssignment`), the vsock
//! attachment cut's replacement for the deleted per-TAP network assignment
//! (feature-delta DG-2 rows 4 and 5).
//!
//! Requires root + CAP_NET_ADMIN (IP_TRANSPARENT, nft). Run via
//! `cargo xtask lima run -- cargo nextest run -p overdrive-worker --features
//! integration-tests`. The whole worker `integration` binary is serialized in
//! the `host-kernel-shared` nextest group, and every test holds the shared
//! cross-process `KernelStateLock`.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::missing_panics_doc,
    clippy::similar_names,
    clippy::struct_field_names,
    clippy::missing_const_for_fn,
    clippy::result_large_err,
    reason = "Tier-3 adapter-integration test bodies: fixture preconditions panic with informative messages; evidence goes to stderr; leg F/C are the ADR-0069 contract vocabulary; the PEM fields share their encoding suffix; const-fn-ability of test helpers is not load-bearing; the install error's size is the production type's"
)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GuestCid, GuestCidClaim as _, GuestTransportAssignment, LaunchToken,
};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::driver::{AllocationSpec, DriverPayload, Resources, VmPayload};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::mtls_resolve::{
    MtlsResolution, MtlsResolve, MtlsResolveError, ResolvedBackend,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, CertSerial, SpiffeId};
use overdrive_dataplane::mtls::HostMtlsEnforcement;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::guest_cid_claim::SimGuestCidClaim;
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_worker::mtls_intercept_port::HostMtlsIntercept;
use overdrive_worker::mtls_intercept_worker::{
    ForwardedOutbound, MtlsInterceptInstallError, MtlsInterceptWorker, MtlsSharedOwnerError,
};
use parking_lot::Mutex;
use rcgen::string::Ia5String;
use rcgen::{CertificateParams, Issuer, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpSocket, TcpStream as TokioTcpStream};
use x509_parser::prelude::FromDer;

use super::inbound_tproxy_harness::{KernelStateLock, clean_shared_infra, is_root, record_uname};

/// Byte-distinct litmus: the peer must receive REQUEST and the forwarder must
/// read RESPONSE, so the oracle proves the real server→client reply pipe.
const REQUEST: &[u8] = b"P5-S10-FORWARDED-OUTBOUND-REQUEST-guest-plaintext-to-mesh-peer";
const RESPONSE: &[u8] = b"P5-S10-FORWARDED-OUTBOUND-RESPONSE-mesh-peer-reply-over-leg-f";

/// The DNS SAN the remote mesh peer presents — the fixed SNI the enforcement
/// adapter's leg-B client handshake uses (`mtls::outbound`).
const PEER_SNI: &str = "peer.overdrive.local";
const CLIENT_SPIFFE: &str = "spiffe://overdrive.local/ns/default/sa/p5-forwarded-client";
const PEER_SPIFFE: &str = "spiffe://overdrive.local/ns/default/sa/p5-forwarded-peer";

/// The service port the guest dialled. The registered `original_destination`
/// is `(lease network address, this port)`: nothing listens there, so a leg-F
/// that dialled the original destination instead of the registered backend
/// would never complete the round-trip.
const DIALLED_SERVICE_PORT: u16 = 443;

/// Set in the re-executed child process of the H-5 serve-exit cell; its value
/// is the directory holding the material the child composes from.
const SERVE_CHILD_ENV: &str = "OVERDRIVE_P5_S10_SERVE_CHILD_DIR";

const ROUND_TRIP_DEADLINE: Duration = Duration::from_secs(15);
const CLOSURE_DEADLINE: Duration = Duration::from_secs(5);
const PEER_TERMINATION_DEADLINE: Duration = Duration::from_secs(10);

// ============================================================================
// Identity material (real root → intermediate → leaf chain)
// ============================================================================

/// The PEM material the agent's held identity and the peer's trust anchor are
/// built from. Serializable to a directory so the H-5 child composes from the
/// exact same chain the parent's peer trusts.
#[derive(Clone)]
struct IdentityMaterial {
    ca_pem: String,
    intermediate_pem: String,
    client_cert_pem: String,
    client_key_pem: String,
}

impl IdentityMaterial {
    const FILES: [&'static str; 4] =
        ["ca.pem", "intermediate.pem", "client_cert.pem", "client_key.pem"];

    fn write_to(&self, dir: &Path) {
        for (name, body) in Self::FILES.iter().zip([
            &self.ca_pem,
            &self.intermediate_pem,
            &self.client_cert_pem,
            &self.client_key_pem,
        ]) {
            fs::write(dir.join(name), body).expect("write identity material for the serve child");
        }
    }

    fn read_from(dir: &Path) -> Self {
        let read = |name: &str| {
            fs::read_to_string(dir.join(name)).expect("serve child reads its identity material")
        };
        Self {
            ca_pem: read(Self::FILES[0]),
            intermediate_pem: read(Self::FILES[1]),
            client_cert_pem: read(Self::FILES[2]),
            client_key_pem: read(Self::FILES[3]),
        }
    }

    fn client_svid(&self) -> SvidMaterial {
        let cert_der = first_cert_der(&self.client_cert_pem);
        let not_after = UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800));
        SvidMaterial::new(
            CaCertPem::new(self.client_cert_pem.clone()),
            CaCertDer::new(cert_der.as_ref().to_vec()),
            CertSerial::new("0a0b0c0d").expect("valid serial"),
            client_spiffe(),
            CaKeyPem::new(self.client_key_pem.clone()),
            not_after,
        )
    }

    fn trust_bundle(&self) -> TrustBundle {
        TrustBundle::new(
            CaCertPem::new(self.ca_pem.clone()),
            Some(CaCertPem::new(self.intermediate_pem.clone())),
        )
    }
}

fn client_spiffe() -> SpiffeId {
    CLIENT_SPIFFE.parse().expect("valid client spiffe id")
}

fn first_cert_der(pem: &str) -> CertificateDer<'static> {
    let mut reader = std::io::BufReader::new(pem.as_bytes());
    rustls_pemfile::certs(&mut reader)
        .next()
        .expect("PEM holds a certificate")
        .expect("certificate PEM parses")
}

/// The test PKI: the agent-side identity material plus the remote peer's
/// server leaf (DNS SAN = the fixed leg-B SNI).
struct TestPki {
    identity: IdentityMaterial,
    intermediate_der: CertificateDer<'static>,
    peer_cert_der: CertificateDer<'static>,
    peer_key_der: PrivateKeyDer<'static>,
}

impl TestPki {
    fn mint() -> Self {
        let root = MintedCa::root("overdrive-p5-s10-ROOT-CA");
        let intermediate = root.intermediate("overdrive-p5-s10-INTERMEDIATE-CA");
        let (client_cert_pem, client_key_pem, _, _) = intermediate.leaf(CLIENT_SPIFFE, None, true);
        let (_, _, peer_cert_der, peer_key_der) =
            intermediate.leaf(PEER_SPIFFE, Some(PEER_SNI), false);
        Self {
            identity: IdentityMaterial {
                ca_pem: root.cert_pem,
                intermediate_pem: intermediate.cert_pem.clone(),
                client_cert_pem,
                client_key_pem,
            },
            intermediate_der: CertificateDer::from(intermediate.cert_der.clone()),
            peer_cert_der,
            peer_key_der,
        }
    }
}

struct MintedCa {
    params: CertificateParams,
    key: KeyPair,
    cert_pem: String,
    cert_der: Vec<u8>,
}

impl MintedCa {
    fn root(cn: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("root params");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name.push(rcgen::DnType::CommonName, cn);
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("root key");
        let cert = params.self_signed(&key).expect("self-sign root");
        Self { cert_pem: cert.pem(), cert_der: cert.der().to_vec(), params, key }
    }

    fn intermediate(&self, cn: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("intermediate params");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        params.distinguished_name.push(rcgen::DnType::CommonName, cn);
        params.use_authority_key_identifier_extension = true;
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("intermediate key");
        let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&key, &issuer).expect("sign intermediate");
        Self { cert_pem: cert.pem(), cert_der: cert.der().to_vec(), params, key }
    }

    fn leaf(
        &self,
        spiffe: &str,
        dns_san: Option<&str>,
        client_auth: bool,
    ) -> (String, String, CertificateDer<'static>, PrivateKeyDer<'static>) {
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("leaf params");
        let mut sans = vec![SanType::URI(Ia5String::try_from(spiffe).expect("spiffe URI is IA5"))];
        if let Some(dns) = dns_san {
            sans.push(SanType::DnsName(Ia5String::try_from(dns).expect("dns SAN is IA5")));
        }
        params.subject_alt_names = sans;
        params.distinguished_name.push(rcgen::DnType::CommonName, spiffe);
        params.use_authority_key_identifier_extension = true;
        params.extended_key_usages = if client_auth {
            vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth]
        } else {
            vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth]
        };
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("leaf key");
        let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&key, &issuer).expect("sign leaf");
        (
            cert.pem(),
            key.serialize_pem(),
            CertificateDer::from(cert.der().to_vec()),
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
    }
}

/// The agent's held identity store: the ONLY holder of SVID material
/// (workloads hold nothing; the kernel does mTLS).
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

// ============================================================================
// The MtlsResolve port double
// ============================================================================

/// Answers `MeshUnreachable` for every address and counts its calls. A
/// registered flow carries the owner's single resolution, so leg-F must never
/// call this port for it; a call would also fail the flow closed.
#[derive(Default)]
struct CountingResolve {
    calls: AtomicU64,
}

impl CountingResolve {
    fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl MtlsResolve for CountingResolve {
    async fn probe(&self) -> Result<(), MtlsResolveError> {
        Ok(())
    }

    async fn resolve(&self, _orig_dst: SocketAddrV4) -> Result<MtlsResolution, MtlsResolveError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(MtlsResolution::MeshUnreachable)
    }
}

// ============================================================================
// Production composition of the worker
// ============================================================================

/// The guest addressing the allocation is registered under, drawn from the
/// caller's `TestCidrLease` (never a CIDR literal).
#[derive(Clone, Copy)]
struct GuestAddressing {
    address: Ipv4Addr,
    gateway: Ipv4Addr,
    prefix: u8,
    network: Ipv4Addr,
}

impl GuestAddressing {
    fn from_lease(lease: &TestCidrLease) -> Self {
        Self {
            address: lease.workload_addr(),
            gateway: lease.host_gateway(),
            prefix: lease.prefix_len(),
            network: lease.network(),
        }
    }

    /// The destination the guest dialled (see [`DIALLED_SERVICE_PORT`]).
    fn original_destination(self) -> SocketAddrV4 {
        SocketAddrV4::new(self.network, DIALLED_SERVICE_PORT)
    }

    fn encode(self) -> String {
        format!("{} {} {} {}", self.address, self.gateway, self.prefix, self.network)
    }

    fn decode(raw: &str) -> Self {
        let fields: Vec<&str> = raw.split_whitespace().collect();
        assert_eq!(fields.len(), 4, "guest addressing record has four fields: {raw:?}");
        Self {
            address: fields[0].parse().expect("guest address"),
            gateway: fields[1].parse().expect("guest gateway"),
            prefix: fields[2].parse().expect("guest prefix"),
            network: fields[3].parse().expect("guest network"),
        }
    }
}

fn intercept_live_spec(alloc: &AllocationId, guest: GuestAddressing) -> AllocationSpec {
    let [_, _, high, low] = guest.address.octets();
    let cid = GuestCid::new(GUEST_CID_BASE + u32::from(u16::from_be_bytes([high, low])))
        .expect("an in-range guest CID");
    let claim = SimGuestCidClaim::new().claim(cid).expect("claim the allocation's guest CID");
    let launch = LaunchToken::from_bytes([0x5a; 16]).expect("a non-zero launch token");
    AllocationSpec {
        alloc: alloc.clone(),
        identity: client_spiffe(),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: vec![],
            kernel: "/nonexistent/kernel".into(),
            rootfs: "/nonexistent/rootfs".into(),
        }),
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        transport: Some(GuestTransportAssignment::new(guest.address, guest.gateway, launch, claim)),
        service_ports: Vec::new(),
    }
}

/// Compose the production worker over the real intercept and enforcement
/// adapters, publish the node-shared owner, and make every allocation in
/// `live` intercept-live through `start_alloc`.
async fn compose_worker(
    identity: &IdentityMaterial,
    resolve: Arc<CountingResolve>,
    live: &[AllocationId],
    guest: GuestAddressing,
) -> Arc<MtlsInterceptWorker> {
    let svid = identity.client_svid();
    let held = HeldIdentities {
        svids: live.iter().map(|alloc| (alloc.clone(), svid.clone())).collect(),
        bundle: identity.trust_bundle(),
    };
    let enforcement = Arc::new(HostMtlsEnforcement::new(Arc::new(held), MtlsLimits::default()));
    enforcement.probe().await.expect("real mTLS enforcement probe passes before any flow");
    let worker = Arc::new(MtlsInterceptWorker::new(
        enforcement as Arc<dyn MtlsEnforcement>,
        resolve as Arc<dyn MtlsResolve>,
        Arc::new(SimClock::new()),
        Arc::new(HostMtlsIntercept::new()),
    ));
    worker.start_shared_owner().await.expect("publish the node-shared leg-F/leg-C owner");
    for alloc in live {
        worker
            .start_alloc(&intercept_live_spec(alloc, guest))
            .await
            .expect("start_alloc makes the allocation intercept-live");
    }
    worker
}

/// Scrubs the node-global `overdrive-mtls` state on every exit path. Declared
/// after the `KernelStateLock`, so it drops while the lock is still held.
struct SharedInfraCleanup;

impl Drop for SharedInfraCleanup {
    fn drop(&mut self) {
        clean_shared_infra();
    }
}

/// One composed node + a remote mesh peer. Field order is drop order: the
/// worker and peer drop first, the infra scrub next, the kernel lock last.
struct Rig {
    worker: Arc<MtlsInterceptWorker>,
    resolve: Arc<CountingResolve>,
    peer: MeshPeer,
    alloc: AllocationId,
    guest: GuestAddressing,
    _lease: TestCidrLease,
    _cleanup: SharedInfraCleanup,
    _kernel_lock: KernelStateLock,
}

impl Rig {
    async fn start(tag: &str) -> Self {
        Self::start_with(tag, PeerMode::FinishOnFin).await
    }

    async fn start_with(tag: &str, peer_mode: PeerMode) -> Self {
        assert!(
            is_root(),
            "{tag}: Tier-3 forwarded-outbound evidence requires root + CAP_NET_ADMIN"
        );
        record_uname(tag);
        let _ = rustls::crypto::ring::default_provider().install_default();
        let kernel_lock = KernelStateLock::acquire();
        clean_shared_infra();
        let cleanup = SharedInfraCleanup;
        let lease = TestCidrLease::acquire(&format!("worker-p5s10-{tag}"))
            .expect("acquire the forwarded-outbound guest CIDR lease");
        let guest = GuestAddressing::from_lease(&lease);
        let pki = TestPki::mint();
        let peer = MeshPeer::spawn_with(&pki, peer_mode);
        let resolve = Arc::new(CountingResolve::default());
        let alloc = AllocationId::new(&format!("p5s10-{tag}")).expect("valid allocation id");
        let worker = compose_worker(
            &pki.identity,
            Arc::clone(&resolve),
            std::slice::from_ref(&alloc),
            guest,
        )
        .await;
        Self {
            worker,
            resolve,
            peer,
            alloc,
            guest,
            _lease: lease,
            _cleanup: cleanup,
            _kernel_lock: kernel_lock,
        }
    }

    /// The owner's single resolution: the remote mesh peer.
    fn backend(&self) -> ResolvedBackend {
        ResolvedBackend { addr: self.peer.addr(), expected_svid: None }
    }

    fn register(
        &self,
        forwarder_local: SocketAddrV4,
    ) -> Result<ForwardedOutbound, MtlsInterceptInstallError> {
        self.worker.register_forwarded_outbound(
            &self.alloc,
            forwarder_local,
            self.guest.original_destination(),
            self.backend(),
        )
    }

    fn leg_f(&self) -> SocketAddrV4 {
        self.worker.leg_f_addr().expect("leg-F is published while the shared owner is live")
    }

    async fn finish(self) {
        self.worker.shutdown_owner().await.expect("the node-shared owner shuts down cleanly");
        self.peer.finish();
    }
}

// ============================================================================
// The forwarder side (the owner's role at the port boundary)
// ============================================================================

/// A fresh forwarder-side loopback socket, bound before registration so the
/// owner knows `forwarder_local`.
fn fresh_forwarder() -> (TcpSocket, SocketAddrV4) {
    let socket =
        forwarder_socket(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("bind a forwarder");
    let local = match socket.local_addr().expect("forwarder local address") {
        SocketAddr::V4(v4) => v4,
        SocketAddr::V6(v6) => panic!("forwarder bound to an IPv4 loopback reported {v6}"),
    };
    (socket, local)
}

fn forwarder_socket(local: SocketAddrV4) -> std::io::Result<TcpSocket> {
    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    socket.bind(local.into())?;
    Ok(socket)
}

async fn connect_to_leg_f(socket: TcpSocket, leg_f: SocketAddrV4) -> TokioTcpStream {
    tokio::time::timeout(CLOSURE_DEADLINE, socket.connect(leg_f.into()))
        .await
        .expect("leg-F's transparent listener completes the forwarder's handshake in time")
        .expect("the forwarder connects to leg-F")
}

/// Plaintext REQUEST out, plaintext RESPONSE back (the egress dialer never
/// speaks TLS).
async fn exchange(stream: &mut TokioTcpStream) -> Vec<u8> {
    stream.write_all(REQUEST).await.expect("forwarder writes the guest's plaintext request");
    let mut response = vec![0_u8; RESPONSE.len()];
    tokio::time::timeout(ROUND_TRIP_DEADLINE, stream.read_exact(&mut response))
        .await
        .expect("the mesh peer's response arrives within the round-trip deadline")
        .expect("forwarder reads the response");
    response
}

/// How the forwarder's side of a connection ended.
#[derive(Debug, PartialEq, Eq)]
enum Closure {
    Eof,
    Reset,
    Bytes(Vec<u8>),
    StillOpen,
}

impl Closure {
    fn is_closed_without_bytes(&self) -> bool {
        matches!(self, Self::Eof | Self::Reset)
    }
}

fn is_reset(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
    )
}

/// Offer the guest's request on a connection leg-F must close, then observe
/// how it ends. A connection leg-F carried would return bytes or stay open.
async fn offer_and_observe_closure(stream: &mut TokioTcpStream) -> Closure {
    match stream.write_all(REQUEST).await {
        Ok(()) => {}
        Err(error) if is_reset(&error) => return Closure::Reset,
        Err(error) => panic!("forwarder write failed for a reason other than closure: {error}"),
    }
    observe_closure(stream).await
}

async fn observe_closure(stream: &mut TokioTcpStream) -> Closure {
    let mut buf = vec![0_u8; 4096];
    match tokio::time::timeout(CLOSURE_DEADLINE, stream.read(&mut buf)).await {
        Err(_elapsed) => Closure::StillOpen,
        Ok(Ok(0)) => Closure::Eof,
        Ok(Ok(n)) => Closure::Bytes(buf[..n].to_vec()),
        Ok(Err(error)) if is_reset(&error) => Closure::Reset,
        Ok(Err(error)) => panic!("forwarder read failed for a reason other than closure: {error}"),
    }
}

// ============================================================================
// The remote mesh peer (holds a real SVID chain; records its raw wire)
// ============================================================================

/// How the remote peer's TCP stream from leg-F ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Termination {
    #[default]
    Open,
    /// A FIN arrived. `reset_after_fin` records whether leg-F then aborted the
    /// connection instead of finishing the close.
    Fin {
        reset_after_fin: bool,
    },
    Reset,
    /// The TLS handshake or the request read failed.
    Failed,
}

/// What the remote peer does with its own direction once leg-F's FIN arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PeerMode {
    /// Finish its own direction (the peer half-closes on FIN), so a forwarder
    /// FIN ends both directions of the loopback leg.
    FinishOnFin,
    /// Keep its own direction open after the FIN, so the loopback leg stays
    /// half-ended (the forwarder's direction FIN'd, leg-F's toward it open).
    HoldOpenOnFin,
}

#[derive(Clone, Debug, Default)]
struct PeerSession {
    handshake_ok: bool,
    client_spiffe: Option<String>,
    request: Vec<u8>,
    /// Every byte the peer read off its TCP socket before TLS processing —
    /// the inter-agent wire as delivered from leg-F.
    wire_in: Vec<u8>,
    /// Leg-F's FIN reached the peer (recorded as it arrives, before the
    /// session's termination is known).
    saw_fin: bool,
    termination: Termination,
}

type Sessions = Arc<Mutex<Vec<PeerSession>>>;

/// A real rustls TLS 1.3 server on a loopback ephemeral port presenting the
/// peer SVID and REQUIRING a client certificate chaining to the test CA.
struct MeshPeer {
    addr: SocketAddrV4,
    sessions: Sessions,
    stop: Arc<AtomicBool>,
    acceptor: Option<JoinHandle<Vec<JoinHandle<()>>>>,
}

impl MeshPeer {
    fn spawn(pki: &TestPki) -> Self {
        Self::spawn_with(pki, PeerMode::FinishOnFin)
    }

    fn spawn_with(pki: &TestPki, mode: PeerMode) -> Self {
        use rustls::server::WebPkiClientVerifier;
        let mut roots = rustls::RootCertStore::empty();
        roots.add(first_cert_der(&pki.identity.ca_pem)).expect("trust the test root");
        let verifier =
            WebPkiClientVerifier::builder(Arc::new(roots)).build().expect("client verifier");
        let mut config = rustls::ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![pki.peer_cert_der.clone(), pki.intermediate_der.clone()],
                pki.peer_key_der.clone_key(),
            )
            .expect("peer server config");
        // No NewSessionTicket: leg-F's kTLS-RX is armed straight after the
        // handshake and a raw post-handshake ticket record would hit EIO.
        config.send_tls13_tickets = 0;
        let config = Arc::new(config);

        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("bind the mesh peer on loopback");
        listener.set_nonblocking(true).expect("mesh peer listener non-blocking");
        let addr = match listener.local_addr().expect("mesh peer address") {
            SocketAddr::V4(v4) => v4,
            SocketAddr::V6(v6) => panic!("mesh peer bound to IPv4 loopback reported {v6}"),
        };
        let sessions: Sessions = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let acceptor = {
            let sessions = Arc::clone(&sessions);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut handles = Vec::new();
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((tcp, _)) => {
                            tcp.set_nonblocking(false).expect("session socket blocking");
                            let index = {
                                let mut all = sessions.lock();
                                all.push(PeerSession::default());
                                all.len() - 1
                            };
                            let config = Arc::clone(&config);
                            let sessions = Arc::clone(&sessions);
                            let stop = Arc::clone(&stop);
                            handles.push(std::thread::spawn(move || {
                                run_peer_session(tcp, &config, index, &sessions, &stop, mode);
                            }));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("mesh peer accept failed: {error}"),
                    }
                }
                handles
            })
        };
        Self { addr, sessions, stop, acceptor: Some(acceptor) }
    }

    const fn addr(&self) -> SocketAddrV4 {
        self.addr
    }

    fn sessions(&self) -> Vec<PeerSession> {
        self.sessions.lock().clone()
    }

    /// Poll the recorded sessions until `predicate` holds or `deadline` passes.
    async fn wait_until(
        &self,
        deadline: Duration,
        what: &str,
        predicate: impl Fn(&[PeerSession]) -> bool,
    ) -> Vec<PeerSession> {
        let until = Instant::now() + deadline;
        loop {
            let sessions = self.sessions();
            if predicate(&sessions) {
                return sessions;
            }
            assert!(
                Instant::now() < until,
                "mesh peer: timed out waiting for {what}: {sessions:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn finish(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(acceptor) = self.acceptor.take() {
            for session in acceptor.join().expect("mesh peer acceptor joins") {
                session.join().expect("mesh peer session joins");
            }
        }
    }
}

impl Drop for MeshPeer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// The peer's TCP socket, recording every byte read off it.
struct RecordingTcp {
    tcp: TcpStream,
    wire_in: Vec<u8>,
}

impl Read for RecordingTcp {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.tcp.read(buf)?;
        self.wire_in.extend_from_slice(&buf[..n]);
        Ok(n)
    }
}

impl Write for RecordingTcp {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.tcp.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.tcp.flush()
    }
}

fn run_peer_session(
    tcp: TcpStream,
    config: &Arc<rustls::ServerConfig>,
    index: usize,
    sessions: &Sessions,
    stop: &AtomicBool,
    mode: PeerMode,
) {
    let record = |update: &dyn Fn(&mut PeerSession)| update(&mut sessions.lock()[index]);
    tcp.set_nodelay(true).expect("peer nodelay");
    tcp.set_read_timeout(Some(Duration::from_secs(10))).expect("peer handshake read timeout");
    let mut rec = RecordingTcp { tcp, wire_in: Vec::new() };
    let mut conn = rustls::ServerConnection::new(Arc::clone(config)).expect("server connection");

    let mut handshake_ok = true;
    while conn.is_handshaking() {
        if conn.complete_io(&mut rec).is_err() {
            handshake_ok = false;
            break;
        }
    }
    if !handshake_ok {
        let wire = rec.wire_in.clone();
        record(&|s| {
            s.wire_in.clone_from(&wire);
            s.termination = Termination::Failed;
        });
        return;
    }
    let client_spiffe = conn
        .peer_certificates()
        .and_then(|chain| chain.first())
        .and_then(|leaf| spiffe_uri_san(leaf.as_ref()));

    let mut request = Vec::new();
    let mut buf = vec![0_u8; 4096];
    while request.len() < REQUEST.len() {
        match rustls::Stream::new(&mut conn, &mut rec).read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let request_complete = request == REQUEST;
    {
        let wire = rec.wire_in.clone();
        let request = request.clone();
        record(&|s| {
            s.handshake_ok = true;
            s.client_spiffe.clone_from(&client_spiffe);
            s.request.clone_from(&request);
            s.wire_in.clone_from(&wire);
        });
    }
    if !request_complete {
        record(&|s| s.termination = Termination::Failed);
        return;
    }
    let wrote = {
        let mut tls = rustls::Stream::new(&mut conn, &mut rec);
        tls.write_all(RESPONSE).and_then(|()| tls.flush())
    };
    if wrote.is_err() {
        record(&|s| s.termination = Termination::Failed);
        return;
    }

    let termination = watch_termination(&mut rec, stop, mode, &|| record(&|s| s.saw_fin = true));
    let wire = rec.wire_in.clone();
    record(&|s| {
        s.wire_in.clone_from(&wire);
        s.termination = termination;
    });
}

/// Raw TCP reads after the exchange: a FIN (`Ok(0)`) or a reset ends the
/// stream. After a FIN the peer (in [`PeerMode::FinishOnFin`]) finishes its
/// own direction and watches for an abortive close from leg-F; in
/// [`PeerMode::HoldOpenOnFin`] it keeps its own direction open and watches
/// for a reset until the test stops it.
fn watch_termination(
    rec: &mut RecordingTcp,
    stop: &AtomicBool,
    mode: PeerMode,
    on_fin: &dyn Fn(),
) -> Termination {
    rec.tcp.set_read_timeout(Some(Duration::from_millis(100))).expect("peer watch timeout");
    let give_up = Instant::now() + Duration::from_secs(60);
    let mut buf = vec![0_u8; 4096];
    loop {
        if stop.load(Ordering::SeqCst) || Instant::now() >= give_up {
            return Termination::Open;
        }
        match rec.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) if is_timeout(&error) => {}
            Err(error) if is_reset(&error) => return Termination::Reset,
            Err(error) => panic!("mesh peer raw read failed unexpectedly: {error}"),
        }
    }
    on_fin();
    if mode == PeerMode::HoldOpenOnFin {
        loop {
            if stop.load(Ordering::SeqCst) || Instant::now() >= give_up {
                return Termination::Fin { reset_after_fin: false };
            }
            match rec.read(&mut buf) {
                Ok(_) => {}
                Err(error) if is_timeout(&error) => {}
                Err(error) if is_reset(&error) => {
                    return Termination::Fin { reset_after_fin: true };
                }
                Err(error) => panic!("mesh peer post-FIN read failed unexpectedly: {error}"),
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    rec.tcp
        .shutdown(std::net::Shutdown::Write)
        .expect("peer finishes its own direction after leg-F's FIN");
    let settle = Instant::now() + Duration::from_secs(2);
    while Instant::now() < settle {
        match rec.read(&mut buf) {
            Ok(_) => {}
            Err(error) if is_timeout(&error) => {}
            Err(error) if is_reset(&error) => return Termination::Fin { reset_after_fin: true },
            Err(error) => panic!("mesh peer post-FIN read failed unexpectedly: {error}"),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Termination::Fin { reset_after_fin: false }
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
}

fn spiffe_uri_san(der: &[u8]) -> Option<String> {
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(der).ok()?;
    let san = cert.subject_alternative_name().ok()??;
    san.value.general_names.iter().find_map(|name| match name {
        x509_parser::extensions::GeneralName::URI(uri) => Some((*uri).to_owned()),
        _ => None,
    })
}

const TLS_APPLICATION_DATA: u8 = 0x17;

/// Walk TLS record framing from the start of the recorded stream and count
/// `application_data` records.
fn count_tls_application_data_records(stream: &[u8]) -> u64 {
    let mut count = 0;
    let mut i = 0;
    while let Some(header) = stream.get(i..i + 5) {
        if header[1] != 0x03 || !matches!(header[2], 0x01 | 0x03) {
            break;
        }
        if header[0] == TLS_APPLICATION_DATA {
            count += 1;
        }
        i += 5 + usize::from(u16::from_be_bytes([header[3], header[4]]));
    }
    count
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

// ============================================================================
// (a) the registered flow is enforced to the registered backend; leg-F does
//     not resolve it again
// ============================================================================

/// P5-S10 happy path. `@contract-shape:unbounded-preservation`.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn registered_mesh_flow_reaches_its_registered_backend_over_mtls_without_leg_f_resolving_again()
 {
    let rig = Rig::start("happy").await;

    // GIVEN the owner resolved the guest's destination once, to the mesh peer,
    // and registered the forwarded flow from its forwarder socket.
    let (socket, forwarder_local) = fresh_forwarder();
    let registration = rig.register(forwarder_local).expect("register the forwarded flow");
    let resolutions_before = rig.resolve.calls();

    // WHEN the forwarder carries the guest's plaintext request to leg-F.
    let mut stream = connect_to_leg_f(socket, rig.leg_f()).await;
    let response = exchange(&mut stream).await;

    // THEN the guest reads the mesh peer's reply byte-exact…
    assert_eq!(response, RESPONSE, "the plaintext round-trip completes through leg-F");
    let sessions = rig
        .peer
        .wait_until(PEER_TERMINATION_DEADLINE, "one served session", |s| {
            s.len() == 1 && s[0].request == REQUEST
        })
        .await;
    let session = &sessions[0];
    // …the registered backend (not the dialled original destination) received
    // the request byte-exact, after a handshake in which leg-F presented the
    // allocation's SVID chaining to the test CA…
    assert!(session.handshake_ok, "the peer completed mTLS with leg-F: {session:?}");
    assert_eq!(
        session.client_spiffe.as_deref(),
        Some(CLIENT_SPIFFE),
        "leg-F presented the registered allocation's identity to the backend"
    );
    // …the inter-agent wire carried TLS 1.3 application data and no cleartext…
    assert!(
        count_tls_application_data_records(&session.wire_in) > 0,
        "the leg-F → peer wire carries 0x17 application_data records"
    );
    assert!(
        !contains(&session.wire_in, REQUEST),
        "the guest's plaintext never appears on the inter-agent wire"
    );
    // …and leg-F never resolved the registered flow again.
    assert_eq!(
        rig.resolve.calls(),
        resolutions_before,
        "leg-F does not re-resolve a registered flow (ADR-0123)"
    );
    assert_eq!(rig.resolve.calls(), 0, "the resolve port is untouched by the forwarded flow");

    drop(stream);
    drop(registration);
    rig.finish().await;
}

// ============================================================================
// (b) claim by exact 4-tuple: a replayed peer and an unregistered peer are
//     closed without resolution
// ============================================================================

/// P5-S10 sad paths. `@contract-shape:bounded-change` — universe: peer
/// sessions, resolve calls, the forwarder connections' outcome.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn a_replayed_peer_and_an_unregistered_peer_are_closed_by_leg_f_without_resolution() {
    let rig = Rig::start("claim").await;

    // GIVEN a registered forwarded flow that leg-F has claimed and carried.
    let (socket, forwarder_local) = fresh_forwarder();
    let registration = rig.register(forwarder_local).expect("register the forwarded flow");
    // Abortive close so the exact 4-tuple is free for the replay (no TIME_WAIT).
    socket.set_zero_linger().expect("forwarder zero linger");
    let leg_f = rig.leg_f();
    let mut first = connect_to_leg_f(socket, leg_f).await;
    assert_eq!(exchange(&mut first).await, RESPONSE, "the registered flow is carried once");
    drop(first);

    // WHEN a second connection arrives from the same forwarder address — the
    // same exact 4-tuple — while the registration is still held.
    let replay_socket =
        forwarder_socket(forwarder_local).expect("rebind the exact forwarder address");
    let mut replay = connect_to_leg_f(replay_socket, leg_f).await;
    let replay_closure = offer_and_observe_closure(&mut replay).await;

    // THEN leg-F finds no registration for it and closes it.
    assert!(
        replay_closure.is_closed_without_bytes(),
        "a second accept from the same peer 4-tuple is closed, got {replay_closure:?}"
    );

    // WHEN a connection arrives from a forwarder address nobody registered.
    let (orphan_socket, _unregistered) = fresh_forwarder();
    let mut orphan = connect_to_leg_f(orphan_socket, leg_f).await;
    let orphan_closure = offer_and_observe_closure(&mut orphan).await;

    // THEN leg-F closes it too, without resolving it.
    assert!(
        orphan_closure.is_closed_without_bytes(),
        "a leg-F accept with no registration is closed, got {orphan_closure:?}"
    );
    assert_eq!(rig.resolve.calls(), 0, "neither closed accept was resolved");
    let sessions = rig.peer.sessions();
    assert_eq!(sessions.len(), 1, "only the registered flow reached the backend: {sessions:?}");

    drop(replay);
    drop(orphan);
    drop(registration);
    rig.finish().await;
}

// ============================================================================
// (c) a duplicate registration for the same forwarder address is refused
// ============================================================================

/// `@contract-shape:bounded-change` — universe: the registration set (observed
/// through the first registration still carrying its flow), peer sessions.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn a_second_registration_for_the_same_forwarder_address_is_refused() {
    let rig = Rig::start("duplicate").await;

    // GIVEN a held registration for a forwarder address.
    let (socket, forwarder_local) = fresh_forwarder();
    let registration = rig.register(forwarder_local).expect("first registration");

    // WHEN the owner registers the same forwarder address again.
    let duplicate = rig.register(forwarder_local);

    // THEN it is refused, naming the address.
    let Err(error) = duplicate else {
        panic!("a duplicate forwarded registration must be refused")
    };
    assert!(
        matches!(
            error,
            MtlsInterceptInstallError::DuplicateForwardedRegistration { forwarder_local: refused }
                if refused == forwarder_local
        ),
        "expected DuplicateForwardedRegistration naming {forwarder_local}, got {error:?}"
    );
    assert_eq!(
        error.to_string(),
        format!("a forwarded-outbound registration for {forwarder_local} already exists"),
    );

    // AND the first registration is untouched: its flow is still carried.
    let mut stream = connect_to_leg_f(socket, rig.leg_f()).await;
    assert_eq!(exchange(&mut stream).await, RESPONSE, "the original registration still carries");
    assert_eq!(rig.peer.sessions().len(), 1, "exactly one flow reached the backend");
    assert_eq!(rig.resolve.calls(), 0);

    drop(stream);
    drop(registration);
    rig.finish().await;
}

// ============================================================================
// (d) an allocation that is not intercept-live cannot register
// ============================================================================

/// `@contract-shape:bounded-change` — universe: registration outcome per
/// allocation; the live allocation and the stopped one differ only in their
/// capability.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn an_allocation_without_a_live_intercept_cannot_register_a_forwarded_flow() {
    let rig = Rig::start("not-live").await;

    // GIVEN an allocation that was never started on this node.
    let never_started = AllocationId::new("p5s10-never-started").expect("valid allocation id");
    let (_socket, forwarder_local) = fresh_forwarder();

    // WHEN the owner registers a forwarded flow for it, THEN it is refused.
    let refused = rig.worker.register_forwarded_outbound(
        &never_started,
        forwarder_local,
        rig.guest.original_destination(),
        rig.backend(),
    );
    assert_not_intercept_live(refused, &never_started);

    // GIVEN the live allocation registers successfully…
    let (_live_socket, live_local) = fresh_forwarder();
    let live = rig.register(live_local).expect("an intercept-live allocation registers");
    drop(live);

    // …WHEN its capability is retired, THEN the same allocation is refused.
    rig.worker.stop_alloc(&rig.alloc).await.expect("retire the allocation's capability");
    let (_stopped_socket, stopped_local) = fresh_forwarder();
    assert_not_intercept_live(rig.register(stopped_local), &rig.alloc);

    assert!(rig.peer.sessions().is_empty(), "no refused registration reached the backend");
    assert_eq!(rig.resolve.calls(), 0);
    rig.finish().await;
}

fn assert_not_intercept_live(
    outcome: Result<ForwardedOutbound, MtlsInterceptInstallError>,
    expected: &AllocationId,
) {
    let Err(error) = outcome else {
        panic!("{expected} is not intercept-live; registration must be refused")
    };
    assert!(
        matches!(
            &error,
            MtlsInterceptInstallError::NotInterceptLive { alloc } if alloc == expected
        ),
        "expected NotInterceptLive naming {expected}, got {error:?}"
    );
    assert_eq!(
        error.to_string(),
        format!("allocation {expected} is not intercept-live; forwarded outbound refused"),
    );
}

// ============================================================================
// (e) dropping ForwardedOutbound closes leg-F's side, claimed or not
// ============================================================================

/// Unclaimed: the registration is dropped before leg-F ever accepted the flow.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn dropping_an_unclaimed_registration_closes_the_flow_before_it_reaches_the_backend() {
    let rig = Rig::start("drop-unclaimed").await;

    // GIVEN a registration leg-F has not claimed.
    let (socket, forwarder_local) = fresh_forwarder();
    let registration = rig.register(forwarder_local).expect("register the forwarded flow");
    let leg_f = rig.leg_f();

    // WHEN the owner drops it, and the forwarder then reaches leg-F.
    drop(registration);
    let mut stream = connect_to_leg_f(socket, leg_f).await;
    let closure = offer_and_observe_closure(&mut stream).await;

    // THEN leg-F closes its side; nothing reaches the backend or the resolver.
    assert!(
        closure.is_closed_without_bytes(),
        "a dropped, unclaimed registration closes the flow, got {closure:?}"
    );
    assert!(rig.peer.sessions().is_empty(), "the backend never sees the dropped flow");
    assert_eq!(rig.resolve.calls(), 0);

    drop(stream);
    rig.finish().await;
}

/// Claimed (DG-10 reading 2): the registration is dropped while leg-F is
/// carrying the flow and the loopback leg is still open in both directions,
/// so leg-F aborts both of the flow's sockets (`SO_LINGER{1,0}`): the
/// forwarder side and the remote peer each see a reset, never a clean FIN.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn dropping_a_claimed_registration_with_the_loopback_leg_open_resets_both_sides() {
    let rig = Rig::start("drop-claimed").await;

    // GIVEN a registered flow leg-F has claimed and is carrying, neither
    // direction of the loopback leg ended.
    let (socket, forwarder_local) = fresh_forwarder();
    let registration = rig.register(forwarder_local).expect("register the forwarded flow");
    let mut stream = connect_to_leg_f(socket, rig.leg_f()).await;
    assert_eq!(exchange(&mut stream).await, RESPONSE, "the flow is live before the drop");

    // WHEN the owner drops the registration mid-flow.
    drop(registration);

    // THEN leg-F aborts its side toward the forwarder…
    let closure = observe_closure(&mut stream).await;
    assert_eq!(
        closure,
        Closure::Reset,
        "dropping a claimed registration with the loopback leg open aborts leg-F's side"
    );
    // …and its connection to the backend: the remote peer sees a reset.
    let sessions = rig
        .peer
        .wait_until(PEER_TERMINATION_DEADLINE, "the backend connection to end", |s| {
            s.len() == 1 && s[0].termination != Termination::Open
        })
        .await;
    assert_eq!(
        sessions[0].termination,
        Termination::Reset,
        "an unclean end of the loopback leg resets the remote peer (DG-10): {sessions:?}"
    );
    assert_eq!(rig.resolve.calls(), 0);

    drop(stream);
    rig.finish().await;
}

/// Claimed, after a clean close (DG-10 readings 2 and 3): both directions of
/// the loopback leg ended with FIN while the registration was held, so leg-F
/// cleared the remote socket's linger and closed it cleanly; dropping the
/// registration afterwards releases it without resetting the finished peer.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn dropping_a_claimed_registration_after_a_clean_close_keeps_the_remote_fin() {
    let rig = Rig::start("drop-after-clean").await;

    let (socket, forwarder_local) = fresh_forwarder();
    let registration = rig.register(forwarder_local).expect("register the forwarded flow");
    let mut stream = connect_to_leg_f(socket, rig.leg_f()).await;
    assert_eq!(exchange(&mut stream).await, RESPONSE);
    stream.shutdown().await.expect("forwarder finishes the guest's direction");
    assert_eq!(observe_closure(&mut stream).await, Closure::Eof, "the peer's FIN came back");
    drop(stream);
    let before = rig
        .peer
        .wait_until(PEER_TERMINATION_DEADLINE, "the backend stream to end", |s| {
            s.len() == 1 && s[0].termination != Termination::Open
        })
        .await;
    assert_eq!(before[0].termination, Termination::Fin { reset_after_fin: false });

    // WHEN the owner drops the registration after the clean close.
    drop(registration);

    // THEN the remote peer's record is unchanged: FIN, no reset after it.
    tokio::time::sleep(CLOSURE_DEADLINE).await;
    let after = rig.peer.sessions();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].termination, Termination::Fin { reset_after_fin: false });
    rig.finish().await;
}

/// Claimed, with the loopback leg half-ended (DG-10 reading 2): the guest's
/// direction ended with FIN, but the remote peer keeps its own direction open,
/// so leg-F's direction toward the forwarder never ends. Dropping the
/// registration then is not a clean close: leg-F aborts both sockets and the
/// remote peer sees a reset after the FIN it already received.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn dropping_a_claimed_registration_with_the_loopback_leg_half_ended_resets_the_remote_peer() {
    let rig = Rig::start_with("drop-half-ended", PeerMode::HoldOpenOnFin).await;

    // GIVEN a claimed flow whose guest direction has ended with FIN while the
    // peer's direction is still open.
    let (socket, forwarder_local) = fresh_forwarder();
    let registration = rig.register(forwarder_local).expect("register the forwarded flow");
    let mut stream = connect_to_leg_f(socket, rig.leg_f()).await;
    assert_eq!(exchange(&mut stream).await, RESPONSE, "the flow is live before the FIN");
    stream.shutdown().await.expect("forwarder finishes the guest's direction");
    let half_ended = rig
        .peer
        .wait_until(PEER_TERMINATION_DEADLINE, "leg-F to carry the guest's FIN", |s| {
            s.len() == 1 && s[0].saw_fin
        })
        .await;
    assert_eq!(half_ended[0].termination, Termination::Open, "the peer holds its direction open");
    assert_eq!(
        observe_closure(&mut stream).await,
        Closure::StillOpen,
        "leg-F's direction toward the forwarder has not ended: the loopback leg is half-ended",
    );

    // WHEN the owner drops the registration.
    drop(registration);

    // THEN leg-F aborts the remote socket: the peer sees a reset after the FIN.
    let sessions = rig
        .peer
        .wait_until(PEER_TERMINATION_DEADLINE, "the backend connection to end", |s| {
            s.len() == 1 && s[0].termination != Termination::Open
        })
        .await;
    assert_eq!(
        sessions[0].termination,
        Termination::Fin { reset_after_fin: true },
        "a half-ended loopback leg is not a clean close: the remote peer is reset (DG-10): {sessions:?}"
    );
    // …and its side toward the forwarder.
    assert_eq!(observe_closure(&mut stream).await, Closure::Reset, "leg-F aborts its own side");
    assert_eq!(rig.resolve.calls(), 0);

    drop(stream);
    rig.finish().await;
}

/// Unclaimed drop removes the registration (DG-10 reading 2): the same
/// forwarder address registers again afterwards (no
/// `DuplicateForwardedRegistration`), and the new registration carries a flow.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn an_unclaimed_drop_removes_the_registration_so_the_address_registers_again() {
    let rig = Rig::start("drop-unclaimed-reregister").await;

    // GIVEN a registration leg-F never claimed, then dropped.
    let (socket, forwarder_local) = fresh_forwarder();
    let first = rig.register(forwarder_local).expect("first registration");
    drop(first);

    // WHEN the owner registers the same forwarder address again.
    let again = rig.register(forwarder_local);

    // THEN it succeeds: the dropped registration was removed, not left behind.
    let again = match again {
        Ok(registration) => registration,
        Err(error) => panic!("the address must register again after an unclaimed drop: {error:?}"),
    };
    // AND the new registration carries the flow to the backend.
    let mut stream = connect_to_leg_f(socket, rig.leg_f()).await;
    assert_eq!(exchange(&mut stream).await, RESPONSE, "the re-registered flow is carried");
    assert_eq!(rig.peer.sessions().len(), 1, "exactly the re-registered flow reached the backend");
    assert_eq!(rig.resolve.calls(), 0);

    drop(stream);
    drop(again);
    rig.finish().await;
}

// ============================================================================
// (e') refusal precedence (DG-10 reading 4)
// ============================================================================

/// DG-10 reading 4: worker shutdown → `OwnerShutdown`; shared owner not
/// published → `SharedOwner { NotStarted }`; then no active capability →
/// `NotInterceptLive`; then an existing registration →
/// `DuplicateForwardedRegistration`. The cells are layered: each refusal is
/// asserted with its own condition held together with every LATER condition
/// the worker's state can carry, so only the pinned order produces the
/// asserted variant:
///
/// | Cell | shut down | unpublished | no capability | duplicate | → |
/// |---|---|---|---|---|---|
/// | 4 | – | – | – | yes | `DuplicateForwardedRegistration` |
/// | 3 | – | – | yes | yes | `NotInterceptLive` |
/// | 2 | – | yes | yes | (unrepresentable) | `SharedOwner { NotStarted }` |
/// | 1 | yes | (shut down) | yes | yes | `OwnerShutdown` |
///
/// Cell 2 cannot carry a duplicate: a worker that never published its shared
/// owner refuses every registration, so it never holds one.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn forwarded_registration_refusals_follow_the_pinned_precedence() {
    let rig = Rig::start("precedence").await;
    let never_started = AllocationId::new("p5s10-precedence-other").expect("valid allocation id");

    // Cell 4 — duplicate alone: the live allocation holds `taken` and asks
    // for it again.
    let (_s1, taken) = fresh_forwarder();
    let held = rig.register(taken).expect("the live allocation registers");
    let Err(refused) = rig.register(taken) else {
        panic!("a second registration of a held address is refused")
    };
    assert!(
        matches!(
            refused,
            MtlsInterceptInstallError::DuplicateForwardedRegistration { forwarder_local }
                if forwarder_local == taken
        ),
        "expected DuplicateForwardedRegistration naming {taken}, got {refused:?}"
    );

    // Cell 3 — no capability AND duplicate: a never-started allocation asking
    // for the held address is refused for its capability, not the address.
    let refused = rig.worker.register_forwarded_outbound(
        &never_started,
        taken,
        rig.guest.original_destination(),
        rig.backend(),
    );
    assert_not_intercept_live(refused, &never_started);

    // Cell 2 — unpublished AND no capability: a worker that never published
    // its shared owner refuses a never-started allocation with
    // SharedOwner { NotStarted }, not NotInterceptLive.
    let unpublished = Arc::new(MtlsInterceptWorker::new(
        Arc::new(HostMtlsEnforcement::new(
            Arc::new(HeldIdentities {
                svids: BTreeMap::new(),
                bundle: TestPki::mint().identity.trust_bundle(),
            }),
            MtlsLimits::default(),
        )) as Arc<dyn MtlsEnforcement>,
        Arc::new(CountingResolve::default()) as Arc<dyn MtlsResolve>,
        Arc::new(SimClock::new()),
        Arc::new(HostMtlsIntercept::new()),
    ));
    let Err(refused) = unpublished.register_forwarded_outbound(
        &never_started,
        taken,
        rig.guest.original_destination(),
        rig.backend(),
    ) else {
        panic!("an unpublished shared owner refuses every registration")
    };
    assert!(
        matches!(
            refused,
            MtlsInterceptInstallError::SharedOwner { source: MtlsSharedOwnerError::NotStarted }
        ),
        "expected SharedOwner {{ NotStarted }}, got {refused:?}"
    );

    // Cell 1 — shut down AND no capability AND duplicate: with `taken` still
    // held, the worker shuts down; a never-started allocation asking for the
    // held address is refused OwnerShutdown.
    rig.worker.shutdown_owner().await.expect("the node-shared owner shuts down cleanly");
    let Err(refused) = rig.worker.register_forwarded_outbound(
        &never_started,
        taken,
        rig.guest.original_destination(),
        rig.backend(),
    ) else {
        panic!("a shut-down worker refuses every registration")
    };
    assert!(
        matches!(refused, MtlsInterceptInstallError::OwnerShutdown),
        "expected OwnerShutdown, got {refused:?}"
    );
    drop(held);
    assert!(rig.peer.sessions().is_empty(), "no refused registration reached the backend");
    assert_eq!(rig.resolve.calls(), 0);
    let Rig { peer, .. } = rig;
    peer.finish();
}

// ============================================================================
// (f) H-5 remote-side linger
// ============================================================================

/// A clean close of both directions ends the remote stream with FIN, never a
/// reset. `@contract-shape:unbounded-preservation`.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn a_clean_close_of_both_directions_ends_the_remote_stream_with_fin() {
    let rig = Rig::start("clean-close").await;

    // GIVEN a registered flow carrying traffic.
    let (socket, forwarder_local) = fresh_forwarder();
    let registration = rig.register(forwarder_local).expect("register the forwarded flow");
    let mut stream = connect_to_leg_f(socket, rig.leg_f()).await;
    assert_eq!(exchange(&mut stream).await, RESPONSE);

    // WHEN the guest finishes its direction, the peer finishes its own (the
    // peer half-closes on FIN), and the forwarder sees the peer's FIN.
    stream.shutdown().await.expect("forwarder finishes the guest's direction");
    let closure = observe_closure(&mut stream).await;
    assert_eq!(closure, Closure::Eof, "the peer's FIN is carried back through leg-F");
    drop(stream);

    // THEN the remote peer saw a FIN, and no reset followed it.
    let sessions = rig
        .peer
        .wait_until(PEER_TERMINATION_DEADLINE, "the backend stream to end", |s| {
            s.len() == 1 && s[0].termination != Termination::Open
        })
        .await;
    assert_eq!(
        sessions[0].termination,
        Termination::Fin { reset_after_fin: false },
        "a clean close clears leg-F's abortive linger before closing (H-5)"
    );

    drop(registration);
    rig.finish().await;
}

/// An `overdrive serve` exit mid-flow resets the remote peer instead of ending
/// its stream cleanly (H-5, ADR-0160). `@contract-shape:unbounded-preservation`.
///
/// The composition runs in a child process (this test, re-executed with
/// [`SERVE_CHILD_ENV`]) that is `SIGKILL`ed mid-flow, so the kernel closes
/// every descriptor the worker held exactly as on a serve exit.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)"]
async fn a_serve_exit_mid_flow_resets_the_remote_peer() {
    if let Some(dir) = std::env::var_os(SERVE_CHILD_ENV) {
        serve_child(Path::new(&dir)).await;
        return;
    }

    assert!(is_root(), "H-5: Tier-3 forwarded-outbound evidence requires root + CAP_NET_ADMIN");
    record_uname("serve-exit");
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();
    let _cleanup = SharedInfraCleanup;
    let lease = TestCidrLease::acquire("worker-p5s10-serve-exit")
        .expect("acquire the forwarded-outbound guest CIDR lease");
    let pki = TestPki::mint();
    let peer = MeshPeer::spawn(&pki);

    // GIVEN a serve process carrying a registered flow mid-stream.
    let dir = tempfile::tempdir().expect("serve child material directory");
    pki.identity.write_to(dir.path());
    fs::write(dir.path().join("peer"), peer.addr().to_string()).expect("write peer address");
    fs::write(dir.path().join("guest"), GuestAddressing::from_lease(&lease).encode())
        .expect("write guest addressing");
    let mut reap = ReapChild(spawn_serve_child(dir.path()));
    let ready = dir.path().join("ready");
    let until = Instant::now() + Duration::from_secs(90);
    while !ready.exists() {
        if let Some(status) = reap.0.try_wait().expect("poll the serve child") {
            panic!(
                "the serve child exited {status} before carrying the flow:\n{}",
                fs::read_to_string(dir.path().join("child.stderr")).unwrap_or_default()
            );
        }
        assert!(Instant::now() < until, "the serve child did not reach mid-flow in time");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let sessions = peer.sessions();
    assert!(
        sessions.len() == 1
            && sessions[0].request == REQUEST
            && sessions[0].termination == Termination::Open,
        "the flow is live at the peer before the serve exit: {sessions:?}"
    );

    // WHEN the serve process exits.
    reap.0.kill().expect("SIGKILL the serve child");
    reap.0.wait().expect("reap the serve child");

    // THEN the remote peer sees a reset, not a clean end of stream.
    let sessions = peer
        .wait_until(PEER_TERMINATION_DEADLINE, "the backend stream to end", |s| {
            s.first().is_some_and(|session| session.termination != Termination::Open)
        })
        .await;
    assert_eq!(
        sessions[0].termination,
        Termination::Reset,
        "leg-F's remote socket carries SO_LINGER{{1,0}} so a serve exit resets the peer (H-5)"
    );
    peer.finish();
}

/// Kills and reaps the serve child on every exit path.
struct ReapChild(Child);

impl Drop for ReapChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            if let Err(error) = self.0.kill() {
                eprintln!("serve child kill on unwind failed: {error}");
            }
            if let Err(error) = self.0.wait() {
                eprintln!("serve child reap on unwind failed: {error}");
            }
        }
    }
}

fn spawn_serve_child(dir: &Path) -> Child {
    let module = module_path!();
    let test_path = module.split_once("::").map_or(module, |(_crate, rest)| rest);
    let filter = format!("{test_path}::a_serve_exit_mid_flow_resets_the_remote_peer");
    let stderr = fs::File::create(dir.join("child.stderr")).expect("serve child stderr file");
    let stdout = fs::File::create(dir.join("child.stdout")).expect("serve child stdout file");
    Command::new(std::env::current_exe().expect("this test binary"))
        .args([filter.as_str(), "--exact", "--include-ignored", "--nocapture", "--test-threads=1"])
        .env(SERVE_CHILD_ENV, dir)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .expect("spawn the serve child")
}

/// The child process: compose the node, carry one registered flow to
/// mid-stream, signal readiness, then wait to be killed. It holds no kernel
/// lock of its own — the parent holds it for the whole cell.
async fn serve_child(dir: &Path) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let identity = IdentityMaterial::read_from(dir);
    let peer: SocketAddrV4 = fs::read_to_string(dir.join("peer"))
        .expect("serve child reads the peer address")
        .trim()
        .parse()
        .expect("peer address parses");
    let guest = GuestAddressing::decode(
        &fs::read_to_string(dir.join("guest")).expect("serve child reads guest addressing"),
    );
    let alloc = AllocationId::new("p5s10-serve-exit").expect("valid allocation id");
    let resolve = Arc::new(CountingResolve::default());
    let worker = compose_worker(&identity, resolve, std::slice::from_ref(&alloc), guest).await;

    let (socket, forwarder_local) = fresh_forwarder();
    let _registration = worker
        .register_forwarded_outbound(
            &alloc,
            forwarder_local,
            guest.original_destination(),
            ResolvedBackend { addr: peer, expected_svid: None },
        )
        .expect("register the forwarded flow");
    let leg_f = worker.leg_f_addr().expect("leg-F is published");
    let mut stream = connect_to_leg_f(socket, leg_f).await;
    assert_eq!(exchange(&mut stream).await, RESPONSE, "the flow is live in the serve child");
    fs::write(dir.join("ready"), b"mid-flow").expect("signal mid-flow readiness");
    tokio::time::sleep(Duration::from_secs(120)).await;
    panic!("the serve child was not killed by its parent within 120s");
}
