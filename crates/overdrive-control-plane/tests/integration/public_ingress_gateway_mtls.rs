//! Public-ingress gateway — exact-peer client mTLS against a real rustls peer
//! (S-PIG-38, 43, 52 and the matching-identity wire evidence of S-PIG-64).
//!
//! Relocated here from `overdrive-dataplane` because a `GatewayUpstreamSeal` is
//! minted only by `UnboundGatewayBuilder::into_builder`, whose constructor takes
//! the host `PublicCertifiedKeyAeadCodec`: this crate is the one test tree with
//! both the gateway and the host edges. The client under test is the
//! production `HostGatewayClientMtlsFactory::new(Arc<HostGatewayRelay>)`
//! output, built from deps the production builder minted; no fault seam, map,
//! identity or socket is installed that production does not install.
//!
//! | Body | Scenario |
//! |---|---|
//! | `a_valid_peer_that_is_not_the_selected_peer_is_rejected_before_any_body_and_nothing_is_armed` | S-PIG-38 |
//! | `an_unusable_gateway_identity_fails_the_dial_before_any_body` | S-PIG-43 |
//! | `a_descriptor_that_is_not_a_connected_socket_has_no_peer_address` | S-PIG-43 |
//! | `a_peer_that_never_answers_the_handshake_times_out_before_the_deadline_passes` | S-PIG-43 |
//! | `a_peer_certificate_without_a_spiffe_uri_san_is_rejected_by_shape` | S-PIG-52 |
//! | `the_matching_peer_is_dialed_over_tls_13_records_with_no_cleartext_and_the_leg_is_armed` | S-PIG-64 |
//!
//! Every peer is a test-owned blocking rustls server on an ephemeral loopback
//! port (no `127/8` public bind, no netns, no kernel table). The independent
//! oracles are the peer's own observations (the client SVID it saw, the
//! application octets it received), a recording TCP proxy between the dial and
//! the peer (the bytes on the leg-B/leg-C segment) and `getsockopt(TCP_ULP)` on
//! a duplicate of the dialed descriptor (whether the kernel TLS upper-layer
//! protocol is installed on the socket).

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::cast_possible_truncation,
    reason = "CONTRACT_SHAPE metadata; test-owned peers assert their fixture preconditions"
)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use overdrive_core::CertSerial;
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::id::{NodeId, SpiffeId};
use overdrive_core::public_ingress::PublicCertifiedKeyId;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_dataplane::gateway_relay::HostGatewayRelay;
use overdrive_dataplane::mtls::gateway_client::HostGatewayClientMtlsFactory;
use overdrive_gateway::application::UnboundGatewayBuilder;
use overdrive_gateway::ports::{
    GatewayClientIdentityAccess, GatewayClientIdentitySource, GatewayClientMtls,
    GatewayClientMtlsDeps, GatewayClientMtlsError, GatewayClientMtlsFactory, GatewayRelay,
};
use overdrive_gateway::runtime::GatewayLimits;
use overdrive_gateway::{GatewayConfig, ManualCertifiedKeyConfig};
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use overdrive_sim::adapters::kek::SimKek;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::read_ports::SimServiceVipView;
use overdrive_store_local::LocalIntentStore;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};
use tempfile::TempDir;
use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};

use super::mtls_pki::{Leaf, TestPki};

/// The recognisable request-body octets written through the authenticated leg.
const BODY_MARKER: &[u8] =
    b"PIG-CLEARTEXT-MARKER:the-request-body-must-never-be-visible-on-the-wire:";
const HANDSHAKE_BUDGET: Duration = Duration::from_secs(5);
const PEER_IDLE: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// The wall clock the host relay evaluates its deadlines on
// ---------------------------------------------------------------------------

/// Test-local wall clock: a real socket only makes progress in real time, and
/// no production binding crate is a dev-dependency.
struct WallClock;

#[async_trait]
impl Clock for WallClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn unix_now(&self) -> Duration {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
    }
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

// ---------------------------------------------------------------------------
// The held gateway identity source
// ---------------------------------------------------------------------------

type HeldMaterial = Result<(SvidMaterial, TrustBundle), GatewayClientMtlsError>;

/// A source over a closure: the identity the gateway holds at the call.
struct HeldSource {
    held: Box<dyn Fn() -> HeldMaterial + Send + Sync>,
}

impl GatewayClientIdentitySource for HeldSource {
    fn with_current(
        &self,
        use_material: &mut dyn FnMut(
            &SvidMaterial,
            &TrustBundle,
        ) -> Result<(), GatewayClientMtlsError>,
    ) -> Result<(), GatewayClientMtlsError> {
        let (material, bundle) = (self.held)()?;
        use_material(&material, &bundle)
    }
}

fn source(
    held: impl Fn() -> HeldMaterial + Send + Sync + 'static,
) -> Arc<dyn GatewayClientIdentitySource> {
    Arc::new(HeldSource { held: Box::new(held) })
}

fn far_future() -> UnixInstant {
    UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800))
}

fn svid_from_leaf(leaf: &Leaf, not_after: UnixInstant) -> SvidMaterial {
    SvidMaterial::new(
        CaCertPem::new(leaf.cert_pem.clone()),
        CaCertDer::new(leaf.cert_der.as_ref().to_vec()),
        leaf.serial.clone(),
        leaf.spiffe.clone(),
        CaKeyPem::new(leaf.key_pem.clone()),
        not_after,
    )
}

// ---------------------------------------------------------------------------
// The client under test, built through the production composition
// ---------------------------------------------------------------------------

/// The gateway's own custody inputs: a self-signed manual key for the builder
/// (the builder validates the configuration; the client never uses it).
struct Site {
    _dir: TempDir,
    config: GatewayConfig,
    intent_path: PathBuf,
}

impl Site {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("isolated gateway root");
        let chain_path = dir.path().join("api-origin-chain.pem");
        let key_path = dir.path().join("api-origin-key.pem");
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("test key");
        let mut params = rcgen::CertificateParams::new(vec!["api.example.com".to_owned()])
            .expect("certificate params");
        params.is_ca = rcgen::IsCa::NoCa;
        params.not_before = rcgen::date_time_ymd(2020, 1, 1);
        params.not_after = rcgen::date_time_ymd(2100, 1, 1);
        let cert = params.self_signed(&key).expect("test certificate");
        std::fs::write(&chain_path, cert.pem()).expect("write certificate");
        std::fs::write(&key_path, key.serialize_pem()).expect("write private key");
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .expect("private-key mode");
        let config = GatewayConfig::new(
            std::net::Ipv4Addr::new(127, 151, 0, 1),
            ManualCertifiedKeyConfig::new(
                PublicCertifiedKeyId::new("api-origin").expect("key ID"),
                chain_path,
                key_path,
            ),
        );
        let intent_path = dir.path().join("intent.redb");
        Self { _dir: dir, config, intent_path }
    }
}

fn custody_kek() -> Arc<SimKek> {
    Arc::new(SimKek::new().with(
        &KekId::new("overdrive-public-certified-key").expect("custody KEK ID"),
        [0x5a; KEK_LEN],
    ))
}

/// The production exact-peer client over `identity`, and the host relay that
/// writes and closes the authenticated leg. The seal comes from
/// `UnboundGatewayBuilder::into_builder`, as in the boot composition.
async fn production_client(
    site: &Site,
    identity: Arc<dyn GatewayClientIdentitySource>,
) -> (Arc<dyn GatewayClientMtls>, Arc<HostGatewayRelay>) {
    let intent: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(&site.intent_path).expect("real intent store"));
    let observations: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("gateway-node").expect("node ID"),
        64,
    ));
    let clock: Arc<dyn Clock> = Arc::new(WallClock);
    let builder = UnboundGatewayBuilder::new(
        site.config.clone(),
        intent,
        observations,
        Arc::new(SimServiceVipView::new(BTreeMap::new())),
        Arc::new(PublicCertifiedKeyAeadCodec::new(custody_kek())),
        Arc::clone(&clock),
    )
    .await
    .expect("gateway builder");
    let (_builder, seal) = builder.into_builder();
    let relay = Arc::new(HostGatewayRelay::new(clock));
    let client =
        HostGatewayClientMtlsFactory::new(Arc::clone(&relay)).client_mtls(GatewayClientMtlsDeps {
            identity: GatewayClientIdentityAccess::new(identity),
            seal,
            limits: GatewayLimits::first_slice(),
        });
    (client, relay)
}

// ---------------------------------------------------------------------------
// The peer: a blocking rustls server that records what it observed
// ---------------------------------------------------------------------------

/// What the peer observed on the one connection it served.
#[derive(Debug, Clone, Default)]
struct PeerObservation {
    handshake_completed: bool,
    client_spiffe: Option<String>,
    application_octets: Vec<u8>,
}

struct PeerSpec {
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    client_roots_pem: String,
    /// `true`: complete no handshake at all (accept, then stay silent).
    silent: bool,
}

struct Peer {
    addr: SocketAddr,
    observed: Arc<Mutex<PeerObservation>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Peer {
    fn start(spec: PeerSpec) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("peer listener");
        let addr = listener.local_addr().expect("peer address");
        let observed = Arc::new(Mutex::new(PeerObservation::default()));
        let shared = Arc::clone(&observed);
        let join = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("peer accept");
            stream.set_read_timeout(Some(PEER_IDLE)).expect("peer read timeout");
            if spec.silent {
                std::thread::sleep(PEER_IDLE);
                return;
            }
            let mut roots = RootCertStore::empty();
            for cert in rustls_pemfile::certs(&mut spec.client_roots_pem.as_bytes()) {
                roots.add(cert.expect("client root PEM")).expect("client root");
            }
            let verifier =
                WebPkiClientVerifier::builder(Arc::new(roots)).build().expect("client verifier");
            let config = ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_client_cert_verifier(verifier)
                .with_single_cert(spec.chain, spec.key)
                .expect("peer server config");
            let connection = ServerConnection::new(Arc::new(config)).expect("peer connection");
            let mut tls = StreamOwned::new(connection, stream);
            let mut buf = [0u8; 4096];
            // Drive the handshake with the first read; a failed client yields an
            // error here and the observation stays `handshake_completed = false`.
            let first = tls.read(&mut buf);
            let handshaken = !tls.conn.is_handshaking();
            {
                let mut seen = shared.lock().expect("peer observation lock");
                seen.handshake_completed = handshaken;
                if let Some(certs) = tls.conn.peer_certificates() {
                    seen.client_spiffe = certs.first().and_then(|cert| spiffe_uri(cert));
                }
                if let Ok(n) = first {
                    seen.application_octets.extend_from_slice(&buf[..n]);
                }
            }
            if handshaken {
                loop {
                    match tls.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => shared
                            .lock()
                            .expect("peer observation lock")
                            .application_octets
                            .extend_from_slice(&buf[..n]),
                    }
                }
            }
        });
        Self { addr, observed, join: Some(join) }
    }

    /// Wait for the peer thread, then return what it observed.
    fn finish(mut self) -> PeerObservation {
        if let Some(join) = self.join.take() {
            join.join().expect("the peer thread joins");
        }
        self.observed.lock().expect("peer observation lock").clone()
    }
}

/// The first URI SAN of `cert`, if any.
fn spiffe_uri(cert: &CertificateDer<'_>) -> Option<String> {
    let (_, parsed) = X509Certificate::from_der(cert.as_ref()).ok()?;
    parsed.subject_alternative_name().ok().flatten().and_then(|san| {
        san.value.general_names.iter().find_map(|name| match name {
            GeneralName::URI(uri) => Some((*uri).to_owned()),
            _ => None,
        })
    })
}

fn peer_spec_from(pki: &TestPki, leaf: &Leaf) -> PeerSpec {
    PeerSpec {
        chain: vec![leaf.cert_der.clone(), pki.intermediate_cert_der()],
        key: leaf.key_der.clone_key(),
        client_roots_pem: pki.ca_cert_pem().to_owned(),
        silent: false,
    }
}

// ---------------------------------------------------------------------------
// A recording proxy: the bytes on the leg-B/leg-C segment
// ---------------------------------------------------------------------------

/// Every byte that crossed the segment, per direction, in order.
#[derive(Debug, Clone, Default)]
struct Wire {
    client_to_peer: Vec<u8>,
    peer_to_client: Vec<u8>,
}

struct Proxy {
    addr: SocketAddr,
    wire: Arc<Mutex<Wire>>,
    joins: Vec<std::thread::JoinHandle<()>>,
}

impl Proxy {
    fn start(peer: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("proxy listener");
        let addr = listener.local_addr().expect("proxy address");
        let wire = Arc::new(Mutex::new(Wire::default()));
        let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
        let accept = std::thread::spawn(move || {
            let (client, _) = listener.accept().expect("proxy accept");
            accepted_tx.send(client).expect("hand the client over");
        });
        let wire_for_pump = Arc::clone(&wire);
        let pump = std::thread::spawn(move || {
            let client = accepted_rx.recv().expect("the proxied client");
            let upstream = TcpStream::connect(peer).expect("proxy dial to the peer");
            let mut to_peer =
                (client.try_clone().expect("clone"), upstream.try_clone().expect("clone"));
            let mut to_client = (upstream, client);
            let up_wire = Arc::clone(&wire_for_pump);
            let up = std::thread::spawn(move || {
                copy_recording(&mut to_peer.0, &mut to_peer.1, |bytes| {
                    up_wire.lock().expect("wire lock").client_to_peer.extend_from_slice(bytes);
                });
            });
            copy_recording(&mut to_client.0, &mut to_client.1, |bytes| {
                wire_for_pump.lock().expect("wire lock").peer_to_client.extend_from_slice(bytes);
            });
            up.join().expect("the upstream pump joins");
        });
        Self { addr, wire, joins: vec![accept, pump] }
    }

    fn finish(mut self) -> Wire {
        for join in self.joins.drain(..) {
            join.join().expect("the proxy thread joins");
        }
        self.wire.lock().expect("wire lock").clone()
    }
}

fn copy_recording(from: &mut TcpStream, to: &mut TcpStream, mut record: impl FnMut(&[u8])) {
    from.set_read_timeout(Some(PEER_IDLE)).expect("pump read timeout");
    let mut buf = [0u8; 8192];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                record(&buf[..n]);
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = to.shutdown(std::net::Shutdown::Write);
}

/// The TLS record content types of `stream`, in order (`0x16` handshake,
/// `0x14` change-cipher-spec, `0x17` application data, `0x15` alert).
fn record_types(stream: &[u8]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut at = 0;
    while at + 5 <= stream.len() {
        types.push(stream[at]);
        let len = usize::from(u16::from_be_bytes([stream[at + 3], stream[at + 4]]));
        at += 5 + len;
    }
    types
}

// ---------------------------------------------------------------------------
// Socket oracles
// ---------------------------------------------------------------------------

/// Whether the kernel TLS upper-layer protocol is installed on `fd`
/// (`getsockopt(SOL_TCP, TCP_ULP)` names `tls`).
fn tls_ulp_installed(fd: &impl std::os::fd::AsRawFd) -> bool {
    let mut name = [0u8; 16];
    let mut len = name.len() as libc::socklen_t;
    // SAFETY: `fd` is a live descriptor owned by the caller, `name` and `len`
    // describe a writable buffer of the stated size.
    let status = unsafe {
        libc::getsockopt(
            fd.as_raw_fd(),
            libc::SOL_TCP,
            libc::TCP_ULP,
            name.as_mut_ptr().cast(),
            &raw mut len,
        )
    };
    status == 0 && name.starts_with(b"tls")
}

/// Dial `to` and return the connected descriptor plus a duplicate the test
/// keeps to inspect the socket after the client consumed the original.
fn dial(to: SocketAddr) -> (OwnedFd, TcpStream) {
    let stream = TcpStream::connect(to).expect("connect to the selected peer");
    let observer = stream.try_clone().expect("duplicate the dialed descriptor");
    (OwnedFd::from(stream), observer)
}

fn deadline_in(span: Duration) -> Instant {
    Instant::now() + span
}

fn assert_nothing_reached_the_peer(seen: &PeerObservation, context: &str) {
    assert!(
        seen.application_octets.is_empty(),
        "{context}: no body octet reaches the peer, saw {} octet(s)",
        seen.application_octets.len()
    );
}

// ---------------------------------------------------------------------------
// S-PIG-38
// ---------------------------------------------------------------------------

/// S-PIG-38 — the Service Frontend selected peer A, but the socket reached a
/// peer B that presents a perfectly valid SVID of the same trust domain. The
/// gateway rejects it with the exact identity mismatch (expected A, actual B)
/// before any body octet is written, and the dialed socket carries no kernel
/// TLS ULP afterwards.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pending DELIVER exact-peer-client-mtls"]
async fn a_valid_peer_that_is_not_the_selected_peer_is_rejected_before_any_body_and_nothing_is_armed()
 {
    let pki = Arc::new(TestPki::mint());
    let site = Site::new();
    let held = Arc::clone(&pki);
    let (client, relay) = production_client(
        &site,
        source(move || Ok((held.client_svid_material(), held.trust_bundle()))),
    )
    .await;
    // Peer B (the server leaf) answers; the selected peer was A (the peer leaf).
    let peer_b = Peer::start(peer_spec_from(&pki, &pki.server_leaf));
    let selected = pki.peer_leaf.spiffe.clone();
    let (connected, observer) = dial(peer_b.addr);

    let result =
        client.authenticate(connected, selected.clone(), deadline_in(HANDSHAKE_BUDGET)).await;

    match result {
        Err(GatewayClientMtlsError::PeerSpiffeMismatch { expected, actual }) => {
            assert_eq!(expected, selected, "the expected identity is the selected peer's");
            assert_eq!(actual, pki.server_leaf.spiffe, "the actual identity is the presenter's");
        }
        Err(other) => panic!("expected PeerSpiffeMismatch, got {other:?}"),
        Ok(_) => panic!("a peer other than the selected one must not authenticate"),
    }
    assert!(!tls_ulp_installed(&observer), "a failed equality arms nothing on the socket");
    drop(observer);
    let _ = relay;
    assert_nothing_reached_the_peer(&peer_b.finish(), "wrong valid peer");
}

// ---------------------------------------------------------------------------
// S-PIG-43
// ---------------------------------------------------------------------------

/// S-PIG-43 (identity arms) — the held gateway identity is absent, expired or
/// malformed. The dial fails with the typed error before any body octet and
/// arms nothing. Absent and expired are the source's typed results and cross
/// the client unchanged; a malformed key never authenticates.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pending DELIVER exact-peer-client-mtls"]
async fn an_unusable_gateway_identity_fails_the_dial_before_any_body() {
    enum Held {
        Absent,
        Expired,
        MalformedKey,
    }
    for (name, held) in
        [("absent", Held::Absent), ("expired", Held::Expired), ("malformed", Held::MalformedKey)]
    {
        let pki = Arc::new(TestPki::mint());
        let site = Site::new();
        let for_source = Arc::clone(&pki);
        let (client, _relay) = production_client(
            &site,
            match held {
                Held::Absent => source(|| Err(GatewayClientMtlsError::IdentityAbsent)),
                Held::Expired => source(|| Err(GatewayClientMtlsError::IdentityExpired)),
                Held::MalformedKey => source(move || {
                    let leaf_material = SvidMaterial::new(
                        CaCertPem::new(for_source.client_leaf.cert_pem.clone()),
                        CaCertDer::new(for_source.client_leaf.cert_der.as_ref().to_vec()),
                        CertSerial::new("0a0b0c0d").expect("serial"),
                        for_source.client_leaf.spiffe.clone(),
                        CaKeyPem::new("not a PEM private key".to_owned()),
                        far_future(),
                    );
                    Ok((leaf_material, for_source.trust_bundle()))
                }),
            },
        )
        .await;
        let peer = Peer::start(peer_spec_from(&pki, &pki.peer_leaf));
        let (connected, observer) = dial(peer.addr);

        let result = client
            .authenticate(connected, pki.peer_leaf.spiffe.clone(), deadline_in(HANDSHAKE_BUDGET))
            .await;

        match (name, result) {
            ("absent", Err(GatewayClientMtlsError::IdentityAbsent))
            | ("expired", Err(GatewayClientMtlsError::IdentityExpired)) => {}
            ("malformed", Err(error)) => {
                assert!(
                    !matches!(error, GatewayClientMtlsError::Arm(_)),
                    "{name}: a malformed identity fails before any arming, got {error:?}"
                );
            }
            (_, other) => panic!("{name}: unexpected outcome {:?}", other.map(|_| "an armed leg")),
        }
        assert!(!tls_ulp_installed(&observer), "{name}: nothing is armed");
        drop(observer);
        assert_nothing_reached_the_peer(&peer.finish(), name);
    }
}

/// S-PIG-43 (descriptor arm) — a descriptor that is not a connected socket has
/// no peer address; the dial fails with `PeerAddress` and never handshakes.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pending DELIVER exact-peer-client-mtls"]
async fn a_descriptor_that_is_not_a_connected_socket_has_no_peer_address() {
    let pki = Arc::new(TestPki::mint());
    let site = Site::new();
    let held = Arc::clone(&pki);
    let (client, _relay) = production_client(
        &site,
        source(move || Ok((held.client_svid_material(), held.trust_bundle()))),
    )
    .await;
    let not_a_socket = OwnedFd::from(std::fs::File::open("/dev/null").expect("open /dev/null"));

    let result = client
        .authenticate(not_a_socket, pki.peer_leaf.spiffe.clone(), deadline_in(HANDSHAKE_BUDGET))
        .await;

    assert!(
        matches!(result, Err(GatewayClientMtlsError::PeerAddress)),
        "a non-socket descriptor has no selected peer address, got {:?}",
        result.map(|_| "an armed leg")
    );
}

/// S-PIG-43 (deadline arm) — a peer that accepts and never speaks: the
/// handshake ends with `HandshakeTimeout` at the caller's deadline, not before
/// it, and arms nothing.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pending DELIVER exact-peer-client-mtls"]
async fn a_peer_that_never_answers_the_handshake_times_out_before_the_deadline_passes() {
    const BUDGET: Duration = Duration::from_millis(400);
    let pki = Arc::new(TestPki::mint());
    let site = Site::new();
    let held = Arc::clone(&pki);
    let (client, _relay) = production_client(
        &site,
        source(move || Ok((held.client_svid_material(), held.trust_bundle()))),
    )
    .await;
    let mut spec = peer_spec_from(&pki, &pki.peer_leaf);
    spec.silent = true;
    let silent = Peer::start(spec);
    let (connected, observer) = dial(silent.addr);

    let started = Instant::now();
    let result =
        client.authenticate(connected, pki.peer_leaf.spiffe.clone(), deadline_in(BUDGET)).await;
    let elapsed = started.elapsed();

    assert!(
        matches!(result, Err(GatewayClientMtlsError::HandshakeTimeout)),
        "a silent peer ends in HandshakeTimeout, got {:?}",
        result.map(|_| "an armed leg")
    );
    assert!(elapsed >= BUDGET, "the timeout fires at the deadline, not earlier: {elapsed:?}");
    assert!(elapsed < BUDGET * 5, "the timeout is prompt once the deadline passes: {elapsed:?}");
    assert!(!tls_ulp_installed(&observer), "a timed-out handshake arms nothing");
    drop(observer);
    assert_nothing_reached_the_peer(&silent.finish(), "silent peer");
}

// ---------------------------------------------------------------------------
// S-PIG-52
// ---------------------------------------------------------------------------

/// A second, self-contained trust domain whose peer certificate carries a URI
/// SAN that is not a SPIFFE identity: the one shape the shared test PKI cannot
/// mint.
struct ShapePki {
    root_pem: String,
    gateway: Leaf,
    peer_chain: Vec<CertificateDer<'static>>,
    peer_key: PrivateKeyDer<'static>,
}

impl ShapePki {
    fn mint() -> Self {
        use rcgen::{
            BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer,
            KeyPair, SanType, string::Ia5String,
        };
        let root_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("root key");
        let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("root params");
        root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        root_params.distinguished_name.push(DnType::CommonName, "shape-root");
        let root = root_params.self_signed(&root_key).expect("root certificate");
        let issuer = Issuer::from_params(&root_params, &root_key);

        let leaf = |uri: &str, client: bool| {
            let mut params = CertificateParams::new(Vec::<String>::new()).expect("leaf params");
            params.subject_alt_names =
                vec![SanType::URI(Ia5String::try_from(uri.to_owned()).expect("IA5 URI"))];
            params.distinguished_name.push(DnType::CommonName, uri);
            params.extended_key_usages = vec![if client {
                ExtendedKeyUsagePurpose::ClientAuth
            } else {
                ExtendedKeyUsagePurpose::ServerAuth
            }];
            let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("leaf key");
            let cert = params.signed_by(&key, &issuer).expect("leaf certificate");
            (cert, key)
        };
        let gateway_uri = "spiffe://overdrive.local/gateway/shape-node";
        let (gateway_cert, gateway_key) = leaf(gateway_uri, true);
        let (peer_cert, peer_key) = leaf("https://peer.example.test/not-a-spiffe-id", false);
        Self {
            root_pem: root.pem(),
            gateway: Leaf {
                cert_pem: gateway_cert.pem(),
                key_pem: gateway_key.serialize_pem(),
                cert_der: CertificateDer::from(gateway_cert.der().to_vec()),
                key_der: PrivateKeyDer::from(PrivatePkcs8KeyDer::from(gateway_key.serialize_der())),
                spiffe: gateway_uri.parse().expect("gateway SPIFFE ID"),
                serial: CertSerial::new("0a0b0c0d").expect("serial"),
            },
            peer_chain: vec![CertificateDer::from(peer_cert.der().to_vec())],
            peer_key: PrivateKeyDer::from(PrivatePkcs8KeyDer::from(peer_key.serialize_der())),
        }
    }
}

/// S-PIG-52 — the peer's certificate chains to the gateway's trust bundle but
/// its URI SAN is not a SPIFFE identity. The dial fails with
/// `PeerSpiffeShape` before any body octet, and arms nothing.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pending DELIVER exact-peer-client-mtls"]
async fn a_peer_certificate_without_a_spiffe_uri_san_is_rejected_by_shape() {
    let shape = Arc::new(ShapePki::mint());
    let site = Site::new();
    let held = Arc::clone(&shape);
    let (client, _relay) = production_client(
        &site,
        source(move || {
            Ok((
                svid_from_leaf(&held.gateway, far_future()),
                TrustBundle::new(CaCertPem::new(held.root_pem.clone()), None),
            ))
        }),
    )
    .await;
    let peer = Peer::start(PeerSpec {
        chain: shape.peer_chain.clone(),
        key: shape.peer_key.clone_key(),
        client_roots_pem: shape.root_pem.clone(),
        silent: false,
    });
    let (connected, observer) = dial(peer.addr);
    let selected: SpiffeId = "spiffe://overdrive.local/workload/api/alloc/api-0"
        .parse()
        .expect("selected peer SPIFFE ID");

    let result = client.authenticate(connected, selected, deadline_in(HANDSHAKE_BUDGET)).await;

    assert!(
        matches!(result, Err(GatewayClientMtlsError::PeerSpiffeShape)),
        "a non-SPIFFE URI SAN is rejected by shape, got {:?}",
        result.map(|_| "an armed leg")
    );
    assert!(!tls_ulp_installed(&observer), "a shape failure arms nothing");
    drop(observer);
    assert_nothing_reached_the_peer(&peer.finish(), "wrong URI-SAN shape");
}

// ---------------------------------------------------------------------------
// S-PIG-64
// ---------------------------------------------------------------------------

/// S-PIG-64 — the matching-identity dial. The peer is the selected peer; the
/// gateway authenticates it, the kernel TLS ULP is installed on the dialed
/// socket, a request body written through the authenticated leg reaches the
/// peer intact, and the recording proxy on the segment shows only TLS 1.3
/// records after the hello flight (application data, never the body marker in
/// the clear). The peer observes the gateway's dedicated SVID.
///
/// The wrong-valid, expired and absent negatives are S-PIG-38/43/52 and are
/// not repeated on the wire.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pending DELIVER exact-peer-client-mtls"]
async fn the_matching_peer_is_dialed_over_tls_13_records_with_no_cleartext_and_the_leg_is_armed() {
    let pki = Arc::new(TestPki::mint());
    let site = Site::new();
    let held = Arc::clone(&pki);
    let (client, relay) = production_client(
        &site,
        source(move || Ok((held.client_svid_material(), held.trust_bundle()))),
    )
    .await;
    let peer = Peer::start(peer_spec_from(&pki, &pki.peer_leaf));
    let proxy = Proxy::start(peer.addr);
    let (connected, observer) = dial(proxy.addr);

    let upstream = client
        .authenticate(connected, pki.peer_leaf.spiffe.clone(), deadline_in(HANDSHAKE_BUDGET))
        .await
        .unwrap_or_else(|error| panic!("the matching peer authenticates: {error:?}"));
    assert!(tls_ulp_installed(&observer), "the kernel TLS ULP is installed on the dialed socket");

    let (_read, mut write) = upstream.into_halves();
    relay
        .write_all(&mut write, BODY_MARKER, deadline_in(HANDSHAKE_BUDGET))
        .await
        .expect("the request body is written through the armed leg");
    relay
        .close_write(&mut write, deadline_in(HANDSHAKE_BUDGET))
        .await
        .expect("the write half is closed with a close_notify");
    drop(write);
    drop(observer);

    let seen = peer.finish();
    let wire = proxy.finish();
    assert!(seen.handshake_completed, "the peer completed the mutual handshake");
    assert_eq!(
        seen.client_spiffe.as_deref(),
        Some(pki.client_leaf.spiffe.as_str()),
        "the peer observes the dedicated gateway SVID"
    );
    assert_eq!(seen.application_octets, BODY_MARKER, "the body arrives intact");

    for (direction, bytes) in
        [("gateway to peer", &wire.client_to_peer), ("peer to gateway", &wire.peer_to_client)]
    {
        assert!(
            !bytes.windows(BODY_MARKER.len()).any(|window| window == BODY_MARKER),
            "{direction}: the request body never appears in the clear on the segment"
        );
        let types = record_types(bytes);
        assert_eq!(
            types.first(),
            Some(&0x16),
            "{direction}: the flight opens with a handshake record"
        );
        assert!(
            types[1..].iter().all(|kind| matches!(kind, 0x14 | 0x17)),
            "{direction}: after the hello every record is TLS 1.3 application data (or a compatibility CCS): {types:?}"
        );
    }
    let encrypted_records =
        record_types(&wire.client_to_peer).into_iter().filter(|kind| *kind == 0x17).count();
    assert!(
        encrypted_records >= 2,
        "the gateway sent encrypted records after the hello (finished and the body)"
    );
}
