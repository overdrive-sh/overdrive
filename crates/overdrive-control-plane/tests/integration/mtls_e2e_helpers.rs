//! Focused control-plane-local mTLS e2e helpers for the Tier-3
//! production-activation gate (transparent-mtls-host-socket, GH #26;
//! step 06-03 criteria[1]).
//!
//! These productionise — through the REAL `run_server` boot — the same
//! observables the dataplane `mtls_composed_walking_skeleton`
//! demonstrates, but composed entirely in the control-plane test tree to
//! avoid the high-risk cross-crate `traffic.rs` promotion (per the
//! 06-03 dispatch § HELPERS). They are deliberately self-contained:
//!
//!   - [`TestPki`] — a shared `root → intermediate → leaf` PKI (matching
//!     production issuance). Mints the agent's client SVID (the leg-B
//!     identity the override hands the booted worker) and the
//!     `OutboundPeer` server cert (DNS SAN `peer.overdrive.local`, so the
//!     agent's leg-B SNI + root-anchor verification accepts it).
//!   - [`HeldIdentities`] — the `IdentityRead` double the PKI-SEAM
//!     injects into the boot (`ServerConfig.mtls_identity_override`), so
//!     the agent's leg-B SVID + `TrustBundle` both root on this `TestPki`.
//!   - [`OutboundPeer`] — a real synchronous rustls + raw-kTLS TLS 1.3
//!     mTLS server (REQUIRE+VERIFY client auth) the agent's leg-B dials,
//!     paired with a REAL `AF_PACKET` capture on `lo` that counts genuine
//!     TLS 1.3 `0x17` application_data records on the peer-facing wire and
//!     confirms the plaintext markers never appear (the confidentiality
//!     oracle — derived from CAPTURED BYTES, never handshake bookkeeping).
//!
//! Lifted from `overdrive-dataplane/tests/integration/helpers/{mtls_pki,
//! mtls_roles,traffic}.rs` (the proven 01-01 composed walking-skeleton
//! harness) and trimmed to the OUTBOUND peer role the criteria[1] gate
//! needs. The control-plane test crate already dev-deps
//! rustls/rcgen/rustls-pemfile/x509-parser/libc.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]
// Raw libc/socket glue (AF_PACKET capture, raw kTLS arm) + FFI-width casts
// on compile-time-constant struct sizes; the unwraps are the standard
// test-fixture idiom (a panic-with-message is the right precondition
// failure for a Tier-3 gate that always runs as root on the real kernel).
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unnecessary_cast,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::missing_panics_doc,
    clippy::doc_markdown,
    clippy::missing_const_for_fn
)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, CertSerial, SpiffeId};
use rcgen::string::Ia5String;
use rcgen::{CertificateParams, Issuer, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::WebPkiClientVerifier;

// ============================================================================
// TestPki — root → intermediate → leaf, the production issuance shape.
// ============================================================================

/// The SNI the agent's leg-B client handshake presents (it dials the real
/// peer and verifies the peer's server cert against the bundle AND that
/// the SNI matches a DNS SAN). The `OutboundPeer` cert carries this SAN.
pub const PEER_SNI: &str = "peer.overdrive.local";

/// A minted leaf — PEM + DER + the SPIFFE-shaped URI SAN.
pub struct Leaf {
    pub cert_pem: String,
    pub key_pem: String,
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
    pub spiffe: SpiffeId,
    pub serial: CertSerial,
}

/// The shared test PKI (root anchor + intermediate chain material + the
/// agent's client leaf + the outbound peer's server leaf).
pub struct TestPki {
    ca_cert_pem: String,
    intermediate_cert_pem: String,
    intermediate_cert_der: CertificateDer<'static>,
    /// The agent's OUTBOUND client SVID — `clientAuth`; presented by the
    /// booted worker's leg-B handshake (read via the injected `IdentityRead`).
    pub client_leaf: Leaf,
    /// The OUTBOUND real-peer server cert — `serverAuth` + DNS SAN `PEER_SNI`.
    pub peer_leaf: Leaf,
    /// The allocation the deployed exec workload runs under (the agent reads
    /// `svid_for(client_alloc)` to present the client SVID on its behalf).
    pub client_alloc: AllocationId,
}

impl TestPki {
    /// Mint the CA chain + the agent's client leaf + the peer server leaf.
    /// `client_alloc` is the [`AllocationId`] the deployed workload runs
    /// under; the agent reads this alloc's held SVID to present on leg B.
    #[must_use]
    pub fn mint(client_alloc: AllocationId) -> Self {
        let root = MintedCa::mint_root("overdrive-mtls-e2e-ROOT-CA");
        let intermediate = root.mint_intermediate("overdrive-mtls-e2e-INTERMEDIATE-CA");

        let client_leaf =
            intermediate.mint_leaf("spiffe://overdrive.local/ns/default/sa/e2e-client", None, true);
        let peer_leaf = intermediate.mint_leaf(
            "spiffe://overdrive.local/ns/default/sa/e2e-peer",
            Some(PEER_SNI),
            false,
        );

        Self {
            ca_cert_pem: root.cert_pem,
            intermediate_cert_pem: intermediate.cert_pem.clone(),
            intermediate_cert_der: CertificateDer::from(intermediate.cert_der),
            client_leaf,
            peer_leaf,
            client_alloc,
        }
    }

    /// The ROOT cert PEM (the trust anchor the bundle pins).
    #[must_use]
    pub fn ca_cert_pem(&self) -> &str {
        &self.ca_cert_pem
    }

    /// The INTERMEDIATE cert in DER form (chain material every presenting
    /// side appends to its leaf so a root-anchor-only verifier builds the
    /// `leaf → intermediate → root` path).
    #[must_use]
    pub fn intermediate_cert_der(&self) -> CertificateDer<'static> {
        self.intermediate_cert_der.clone()
    }

    /// The shared trust bundle: root anchor + intermediate chain material.
    #[must_use]
    pub fn trust_bundle(&self) -> TrustBundle {
        TrustBundle::new(
            CaCertPem::new(self.ca_cert_pem.clone()),
            Some(CaCertPem::new(self.intermediate_cert_pem.clone())),
        )
    }

    /// Build the `IdentityRead` double the PKI-SEAM injects — holds the
    /// agent's client SVID keyed by `client_alloc`, plus the shared bundle.
    #[must_use]
    pub fn held_identities(&self) -> HeldIdentities {
        let mut svids = BTreeMap::new();
        svids.insert(self.client_alloc.clone(), svid_from_leaf(&self.client_leaf));
        HeldIdentities { svids, bundle: self.trust_bundle() }
    }
}

/// The `IdentityRead` double: holds minted SVIDs keyed by `AllocationId`
/// plus the trust bundle, served as owned clones (a read never issues,
/// never mutates; `None` is explicit absence). The booted agent reads
/// through THIS when the PKI-SEAM override is set.
pub struct HeldIdentities {
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

/// A minted signing authority (root OR intermediate) retaining its params
/// + key so it can build a reusable rcgen `Issuer` for the next level down.
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

/// Assemble `SvidMaterial` from a minted leaf (cert PEM/DER + node-held
/// leaf key PEM + a far-future `not_after`).
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

// ============================================================================
// OutboundPeer — the real mTLS server the agent's leg-B dials + AF_PACKET
// 0x17 wire oracle on `lo`.
// ============================================================================

/// The cleartext request marker the workload sends (must NEVER appear on the
/// encrypted peer-facing wire; the peer must reconstruct it byte-exact after
/// kTLS-RX decrypt).
pub const OUTBOUND_REQUEST: &[u8] =
    b"OVERDRIVE_E2E_OUTBOUND_REQUEST_deployed_workload_speaks_first_must_arrive_TLS13_decrypted_byte_exact_0001";
/// The peer's reply (the return leg). The workload reads it back byte-exact.
pub const OUTBOUND_REPLY: &[u8] =
    b"OVERDRIVE_E2E_OUTBOUND_REPLY_peer_responds_return_leg_splices_back_to_workload_byte_exact_0002";

const LOOPBACK_IFACE: &str = "lo";

/// Confidentiality + activity observations on the peer-facing leg.
pub struct WireObservations {
    /// `0x17` application_data records flowing TOWARD the peer port (the
    /// forward / request direction). > 0 proves real TLS 1.3 on the wire.
    pub records_request_dir: u64,
    /// `0x17` application_data records flowing FROM the peer port (response).
    pub records_response_dir: u64,
    /// Appearances of EITHER cleartext marker on the peer wire (MUST be 0).
    pub plaintext_marker_hits: u64,
}

/// The real mTLS peer the agent's leg-B dials: presents the peer SVID
/// (chaining to `TestPki`, DNS SAN `PEER_SNI`), REQUIRES+VERIFIES the
/// client SVID, arms raw kTLS, reads the workload's request (decrypt proof),
/// replies. A REAL `AF_PACKET` capture on `lo` is the confidentiality oracle.
pub struct OutboundPeer {
    addr: SocketAddrV4,
    handle: Option<std::thread::JoinHandle<PeerOutcome>>,
    wire: parking_lot::Mutex<WireCaptureState>,
    presented_client_spiffe: Arc<parking_lot::Mutex<Option<SpiffeId>>>,
}

enum WireCaptureState {
    Live(WireCapture),
    Scanned(WireScan),
}

struct PeerOutcome {
    request_byte_exact: bool,
}

impl OutboundPeer {
    /// Spawn the peer on an ephemeral loopback addr. The booted agent's
    /// leg-B dials THIS addr (it is the `real_peer` the e2e programs into
    /// `MTLS_REDIRECT_DEST` and routes leg-B to). Starts the wire capture
    /// BEFORE accepting so the first leg-B record is captured.
    #[must_use]
    pub fn spawn(pki: &TestPki) -> Self {
        let cert = pki.peer_leaf.cert_der.clone();
        let key = pki.peer_leaf.key_der.clone_key();
        let intermediate = pki.intermediate_cert_der();
        let client_verifier =
            WebPkiClientVerifier::builder(Arc::new(ca_root_store(pki.ca_cert_pem())))
                .build()
                .expect("peer client-cert verifier");
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("peer bind");
        let addr = match listener.local_addr().expect("peer addr") {
            std::net::SocketAddr::V4(a) => a,
            std::net::SocketAddr::V6(_) => unreachable!("bound on 127.0.0.1"),
        };
        let capture = WireCapture::start(LOOPBACK_IFACE, addr.port());
        let presented_client_spiffe = Arc::new(parking_lot::Mutex::new(None));
        let spiffe_slot = Arc::clone(&presented_client_spiffe);
        let handle = std::thread::spawn(move || {
            outbound_peer_serve(&listener, cert, intermediate, key, client_verifier, &spiffe_slot)
        });
        Self {
            addr,
            handle: Some(handle),
            wire: parking_lot::Mutex::new(WireCaptureState::Live(capture)),
            presented_client_spiffe,
        }
    }

    /// The peer's loopback addr — the `real_peer` the workload dials and the
    /// e2e programs into `MTLS_REDIRECT_DEST`.
    #[must_use]
    pub fn addr(&self) -> SocketAddrV4 {
        self.addr
    }

    /// The CLIENT SPIFFE-id the peer REQUIRED + verified + extracted from the
    /// presented client leaf's URI SAN. `None` until the leg-B handshake
    /// completed; proves the agent presented the held client SVID.
    #[must_use]
    pub fn presented_client_spiffe(&self) -> Option<SpiffeId> {
        self.presented_client_spiffe.lock().clone()
    }

    /// Stop the wire capture (on first call), scan it, and report the oracle.
    /// Called AFTER the round-trip completes so every peer-leg record is on
    /// the captured wire.
    #[must_use]
    pub fn wire_observations(&self) -> WireObservations {
        let scan = stop_scan_cached(&self.wire, OUTBOUND_REQUEST, OUTBOUND_REPLY);
        WireObservations {
            records_request_dir: scan.records_to_wire_port,
            records_response_dir: scan.records_from_wire_port,
            plaintext_marker_hits: scan.plaintext_marker_hits,
        }
    }

    /// Join the serve thread and report whether the workload's request was
    /// reconstructed byte-exact (the decrypt proof). Consumes the handle.
    #[must_use]
    pub fn wait_outcome(&mut self) -> bool {
        self.handle.take().is_some_and(|h| h.join().is_ok_and(|o| o.request_byte_exact))
    }
}

/// Stop+scan the capture on first call, caching the scan for repeat calls.
/// The Mutex guard is not held across the slow `stop_and_scan` I/O.
fn stop_scan_cached(
    state: &parking_lot::Mutex<WireCaptureState>,
    request_marker: &[u8],
    response_marker: &[u8],
) -> WireScan {
    let mut guard = state.lock();
    let prior = std::mem::replace(&mut *guard, WireCaptureState::Scanned(WireScan::default()));
    drop(guard);
    let taken = match prior {
        WireCaptureState::Scanned(s) => {
            *state.lock() = WireCaptureState::Scanned(s);
            return s;
        }
        WireCaptureState::Live(capture) => capture,
    };
    let scan = taken.stop_and_scan(request_marker, response_marker);
    *state.lock() = WireCaptureState::Scanned(scan);
    scan
}

/// The synchronous rustls + raw-kTLS peer serve loop: accept leg B, complete
/// the rustls SERVER handshake (REQUIRE+VERIFY client auth), arm kTLS-TX+RX,
/// read the workload's request (decrypted by kTLS-RX), reply.
///
/// MULTI-ACCEPT (06-03): the deployed workload's retry loop dials the real
/// peer addr from spawn — INCLUDING the dials that land BEFORE the test
/// programs the `MTLS_REDIRECT_DEST` redirect. Those pre-redirect dials reach
/// the peer DIRECTLY as PLAINTEXT (the workload speaks no TLS), so the rustls
/// server handshake fails on them. The peer MUST NOT give up on the first such
/// failure (that would close the listener and make the later genuine leg-B
/// dial — `enforce` → `dial_leg(real_peer)` — hit `ECONNREFUSED`). Instead it
/// loops, discarding each non-TLS/failed-handshake connection, until ONE
/// completes the genuine mTLS leg-B handshake (the agent's `enforce` dial) or
/// an overall wall-clock deadline expires.
fn outbound_peer_serve(
    listener: &TcpListener,
    cert: CertificateDer<'static>,
    intermediate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    client_verifier: Arc<dyn rustls::server::danger::ClientCertVerifier>,
    spiffe_slot: &parking_lot::Mutex<Option<SpiffeId>>,
) -> PeerOutcome {
    let cfg = {
        let mut cfg = rustls::ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(vec![cert, intermediate], key)
            .expect("peer server config");
        cfg.enable_secret_extraction = true;
        cfg.send_tls13_tickets = 0;
        Arc::new(cfg)
    };
    // Overall deadline spanning the pre-redirect plaintext dials + the genuine
    // leg-B handshake. Each accept gets the remaining budget.
    let overall_deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let remaining = overall_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            eprintln!("OUTBOUND-PEER: no genuine leg-B handshake before the 45s deadline");
            return PeerOutcome { request_byte_exact: false };
        }
        let Some(tcp) = accept_with_timeout(listener, remaining) else {
            return PeerOutcome { request_byte_exact: false };
        };
        tcp.set_nodelay(true).ok();
        let fd = tcp.as_raw_fd();
        let mut tcp = tcp;
        tcp.set_read_timeout(Some(Duration::from_secs(10))).ok();
        let mut conn =
            rustls::ServerConnection::new(Arc::clone(&cfg)).expect("peer ServerConnection");
        if drive_server_handshake(&mut conn, &mut tcp) {
            // Genuine leg-B mTLS handshake completed — proceed to the kTLS
            // read/reply path with THIS connection.
            if let Some(spiffe) = peer_presented_leaf_spiffe(conn.peer_certificates()) {
                *spiffe_slot.lock() = Some(spiffe);
            }
            return finish_outbound_peer(conn, tcp, fd);
        }
        // Handshake failed (a pre-redirect plaintext dial, or a RST): drop this
        // connection and accept the next. `tcp` drops here → the socket closes.
        drop(tcp);
    }
}

/// Finish the OUTBOUND peer's serve after a genuine leg-B mTLS handshake:
/// extract secrets, arm raw kTLS, read the workload's request (kTLS-RX
/// decrypted), reply on the return leg. Split out of [`outbound_peer_serve`]
/// so the multi-accept loop body stays the handshake-retry concern only.
fn finish_outbound_peer(
    mut conn: rustls::ServerConnection,
    tcp: TcpStream,
    fd: RawFd,
) -> PeerOutcome {
    let mut got = drain_early_plaintext(&mut conn.reader());
    let secrets = conn.dangerous_extract_secrets().expect("peer extract secrets");
    arm_ktls_raw(fd, &secrets);
    std::mem::forget(tcp);

    let stream = unsafe { TcpStream::from_raw_fd(fd) };
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut buf = vec![0u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(10);
    while got.len() < OUTBOUND_REQUEST.len() && Instant::now() < deadline {
        match (&stream).read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    let request_byte_exact = got == OUTBOUND_REQUEST;
    if !request_byte_exact {
        eprintln!(
            "OUTBOUND-PEER: forward miss — received {} of {} request bytes",
            got.len(),
            OUTBOUND_REQUEST.len()
        );
    }
    if request_byte_exact {
        // Two post-arm writes with a delay > the agent's decrypt-pump poll
        // window, so kTLS-TX frames ≥2 distinct TLS records on the return leg.
        let mid = OUTBOUND_REPLY.len() / 2;
        let _ = (&stream).write_all(&OUTBOUND_REPLY[..mid]);
        let _ = (&stream).flush();
        std::thread::sleep(Duration::from_millis(200));
        let _ = (&stream).write_all(&OUTBOUND_REPLY[mid..]);
        let _ = (&stream).flush();
    }
    std::thread::sleep(Duration::from_millis(700));
    PeerOutcome { request_byte_exact }
}

/// Extract the SPIFFE-id (sole URI SAN) from chain position 0 (the leaf).
fn peer_presented_leaf_spiffe(certs: Option<&[CertificateDer<'_>]>) -> Option<SpiffeId> {
    use x509_parser::prelude::FromDer as _;
    let leaf = certs?.first()?;
    let (_, parsed) = x509_parser::certificate::X509Certificate::from_der(leaf.as_ref()).ok()?;
    let san = parsed.subject_alternative_name().ok()??;
    let uri = san.value.general_names.iter().find_map(|gn| match gn {
        x509_parser::extensions::GeneralName::URI(uri) => Some(*uri),
        _ => None,
    })?;
    uri.parse::<SpiffeId>().ok()
}

fn drain_early_plaintext(reader: &mut dyn Read) -> Vec<u8> {
    let mut early = Vec::new();
    let mut buf = [0u8; 16384];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => early.extend_from_slice(&buf[..n]),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
    early
}

fn drive_server_handshake(conn: &mut rustls::ServerConnection, tcp: &mut TcpStream) -> bool {
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

fn arm_ktls_raw(fd: RawFd, secrets: &rustls::ExtractedSecrets) {
    let ulp = b"tls\0";
    // SAFETY: 3-byte "tls" ULP option on a connected fd.
    let rc = unsafe { libc::setsockopt(fd, libc::SOL_TCP, libc::TCP_ULP, ulp.as_ptr().cast(), 3) };
    assert!(rc == 0, "peer TCP_ULP: {}", std::io::Error::last_os_error());
    set_crypto_info(fd, libc::TLS_TX, &secrets.tx);
    set_crypto_info(fd, libc::TLS_RX, &secrets.rx);
}

fn set_crypto_info(fd: RawFd, dir: libc::c_int, sec: &(u64, rustls::ConnectionTrafficSecrets)) {
    use rustls::ConnectionTrafficSecrets;
    #[repr(C)]
    struct Info {
        version: u16,
        cipher: u16,
        iv: [u8; 8],
        key: [u8; 32],
        salt: [u8; 4],
        rec_seq: [u8; 8],
    }
    let (seq, traffic) = sec;
    let ConnectionTrafficSecrets::Aes256Gcm { key, iv } = traffic else {
        panic!("peer kTLS arm requires AES-256-GCM TLS 1.3");
    };
    let ivb = iv.as_ref();
    let mut info = Info {
        version: 0x0304,
        cipher: 52,
        iv: [0; 8],
        key: [0; 32],
        salt: [0; 4],
        rec_seq: seq.to_be_bytes(),
    };
    info.key.copy_from_slice(key.as_ref());
    info.salt.copy_from_slice(&ivb[0..4]);
    info.iv.copy_from_slice(&ivb[4..12]);
    // SAFETY: `Info` is `#[repr(C)]` matching `tls12_crypto_info_aes_gcm_256`.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_TLS,
            dir,
            std::ptr::from_ref(&info).cast(),
            std::mem::size_of::<Info>() as libc::socklen_t,
        )
    };
    assert!(rc == 0, "peer SOL_TLS dir={dir}: {}", std::io::Error::last_os_error());
}

fn ca_root_store(ca_cert_pem: &str) -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    let mut rd = std::io::BufReader::new(ca_cert_pem.as_bytes());
    for c in rustls_pemfile::certs(&mut rd) {
        roots.add(c.expect("ca cert")).expect("add ca cert");
    }
    roots
}

/// Accept one connection within `timeout`, or None.
fn accept_with_timeout(listener: &TcpListener, timeout: Duration) -> Option<TcpStream> {
    let lfd = listener.as_raw_fd();
    let deadline = Instant::now() + timeout;
    listener.set_nonblocking(true).ok();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let mut pfd = libc::pollfd { fd: lfd, events: libc::POLLIN, revents: 0 };
        let ms = remaining.as_millis().min(200) as libc::c_int;
        let pr = unsafe { libc::poll(std::ptr::from_mut(&mut pfd), 1, ms) };
        if pr <= 0 {
            continue;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).ok();
                return Some(stream);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return None,
        }
    }
}

// ============================================================================
// AF_PACKET wire capture + TLS 0x17 record-counting scan (the oracle).
// ============================================================================

const ETH_P_ALL: std::os::raw::c_int = 0x0003;
const ETH_HDR_LEN: usize = 14;
const IPV4_HDR_LEN: usize = 20;
const TLS_CONTENT_TYPE_APPLICATION_DATA: u8 = 0x17;
const TLS_LEGACY_RECORD_VERSION_TLS12: [u8; 2] = [0x03, 0x03];
const TLS_LEGACY_RECORD_VERSION_TLS10: [u8; 2] = [0x03, 0x01];
const TLS_RECORD_HEADER_LEN: usize = 5;

/// Bucketed record + plaintext counts (see [`WireObservations`]).
#[derive(Debug, Clone, Copy, Default)]
struct WireScan {
    records_to_wire_port: u64,
    records_from_wire_port: u64,
    plaintext_marker_hits: u64,
}

/// A live `AF_PACKET`/`SOCK_RAW` capture on `iface`, draining frames on a
/// background thread until [`WireCapture::stop_and_scan`].
struct WireCapture {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Vec<Vec<u8>>>>,
    wire_port: u16,
}

impl WireCapture {
    #[must_use]
    fn start(iface: &str, wire_port: u16) -> Self {
        let ifindex = if_nametoindex(iface).expect("wire-capture: if_nametoindex");
        // SAFETY: AF_PACKET / SOCK_RAW socket (needs root / CAP_NET_RAW).
        let fd: RawFd =
            unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, ETH_P_ALL.to_be() as i32) };
        assert!(fd >= 0, "wire-capture: socket: {}", std::io::Error::last_os_error());

        let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        sll.sll_family = libc::AF_PACKET as u16;
        sll.sll_protocol = (ETH_P_ALL as u16).to_be();
        sll.sll_ifindex = ifindex as i32;
        // SAFETY: bind an AF_PACKET socket to the resolved ifindex.
        let rc = unsafe {
            libc::bind(
                fd,
                std::ptr::from_ref(&sll).cast(),
                std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        };
        assert!(rc == 0, "wire-capture: bind {iface}: {}", std::io::Error::last_os_error());
        // SAFETY: fcntl on our own fd.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || -> Vec<Vec<u8>> {
            let mut frames: Vec<Vec<u8>> = Vec::new();
            let mut buf = vec![0u8; 65536];
            while !stop_thread.load(Ordering::SeqCst) {
                // SAFETY: recv into our owned buffer on the bound AF_PACKET fd.
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
                if n > 0 {
                    frames.push(buf[..n as usize].to_vec());
                } else {
                    std::thread::sleep(Duration::from_micros(200));
                }
            }
            loop {
                // SAFETY: same bounded recv on our fd.
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
                if n > 0 {
                    frames.push(buf[..n as usize].to_vec());
                } else {
                    break;
                }
            }
            // SAFETY: fd created above; close on capture-thread exit.
            unsafe { libc::close(fd) };
            frames
        });
        Self { stop, handle: Some(handle), wire_port }
    }

    #[must_use]
    fn stop_and_scan(mut self, request_marker: &[u8], response_marker: &[u8]) -> WireScan {
        self.stop.store(true, Ordering::SeqCst);
        let frames = self.handle.take().expect("wire-capture handle").join().expect("capture join");
        scan_frames(&frames, self.wire_port, request_marker, response_marker)
    }
}

fn scan_frames(
    frames: &[Vec<u8>],
    wire_port: u16,
    request_marker: &[u8],
    response_marker: &[u8],
) -> WireScan {
    let mut streams: BTreeMap<(u16, u16), Vec<u8>> = BTreeMap::new();
    for frame in frames {
        let Some((src_port, dst_port, payload)) = parse_tcp_payload(frame) else {
            continue;
        };
        if src_port != wire_port && dst_port != wire_port {
            continue;
        }
        if payload.is_empty() {
            continue;
        }
        streams.entry((src_port, dst_port)).or_default().extend_from_slice(payload);
    }
    let mut records_to_wire_port: u64 = 0;
    let mut records_from_wire_port: u64 = 0;
    let mut plaintext_marker_hits: u64 = 0;
    for (&(src_port, dst_port), stream) in &streams {
        let records = count_tls_app_data_records(stream);
        if dst_port == wire_port {
            records_to_wire_port += records;
        } else if src_port == wire_port {
            records_from_wire_port += records;
        }
        plaintext_marker_hits += count_subslices(stream, request_marker);
        plaintext_marker_hits += count_subslices(stream, response_marker);
    }
    WireScan { records_to_wire_port, records_from_wire_port, plaintext_marker_hits }
}

fn parse_tcp_payload(frame: &[u8]) -> Option<(u16, u16, &[u8])> {
    if frame.len() < ETH_HDR_LEN + IPV4_HDR_LEN {
        return None;
    }
    if frame.get(12).copied()? != 0x08 || frame.get(13).copied()? != 0x00 {
        return None;
    }
    let ip = ETH_HDR_LEN;
    let vihl = frame.get(ip).copied()?;
    if vihl >> 4 != 4 {
        return None;
    }
    let ihl = ((vihl & 0x0f) as usize) * 4;
    if ihl < IPV4_HDR_LEN {
        return None;
    }
    if frame.get(ip + 9).copied()? != 0x06 {
        return None;
    }
    let tcp = ip + ihl;
    if frame.len() < tcp + 20 {
        return None;
    }
    let src_port = u16::from_be_bytes([frame.get(tcp).copied()?, frame.get(tcp + 1).copied()?]);
    let dst_port = u16::from_be_bytes([frame.get(tcp + 2).copied()?, frame.get(tcp + 3).copied()?]);
    let data_off = ((frame.get(tcp + 12).copied()? >> 4) as usize) * 4;
    if data_off < 20 {
        return None;
    }
    let payload_start = tcp + data_off;
    if payload_start > frame.len() {
        return None;
    }
    Some((src_port, dst_port, &frame[payload_start..]))
}

fn count_tls_app_data_records(stream: &[u8]) -> u64 {
    let mut count: u64 = 0;
    let mut i = 0usize;
    while i + TLS_RECORD_HEADER_LEN <= stream.len() {
        let content_type = stream[i];
        let version = [stream[i + 1], stream[i + 2]];
        let length = u16::from_be_bytes([stream[i + 3], stream[i + 4]]) as usize;
        if !is_tls_record_version(version) {
            break;
        }
        if content_type == TLS_CONTENT_TYPE_APPLICATION_DATA {
            count += 1;
        }
        let next = i + TLS_RECORD_HEADER_LEN + length;
        if next <= i {
            break;
        }
        i = next;
    }
    count
}

fn is_tls_record_version(version: [u8; 2]) -> bool {
    version == TLS_LEGACY_RECORD_VERSION_TLS12 || version == TLS_LEGACY_RECORD_VERSION_TLS10
}

fn count_subslices(haystack: &[u8], needle: &[u8]) -> u64 {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    let mut count: u64 = 0;
    let mut i = 0usize;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            count += 1;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    count
}

fn if_nametoindex(iface: &str) -> Result<u32, std::io::Error> {
    let cstr = std::ffi::CString::new(iface).expect("iface name has no NUL");
    // SAFETY: thin syscall wrapper; pointer not retained past call.
    let idx = unsafe { libc::if_nametoindex(cstr.as_ptr()) };
    if idx == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(idx)
}
