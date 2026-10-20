//! R5-54 Tier-3 half (LEGS-DIAL, D-14, approved 2026-10-19; GH #295):
//! the real `HostMtlsEnforcement`'s inbound path to a VM backend — a target
//! inside the node guest prefix — on the real kernel.
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md` § *leg-S over
//! vsock (temporary bridge until #303)*, "Who dials leg S and relays it":
//! for `Routed::Inbound { orig_dst }` with `orig_dst` inside `guest_prefix`
//! the path is (1) the blocking rustls server handshake with its
//! `handshake_deadline`, the peer-verification guard and the kTLS TX+RX arm,
//! returning the armed leg C and the 0.5-RTT plaintext (`early_deliver`);
//! (2) the awaited `guest_dial.dial(orig_dst, DialPurpose::Relay)`, bounded by
//! the owner's `GUEST_DIAL_PAIRING_DEADLINE`, not by `handshake_deadline` —
//! the two deadlines run in sequence; (3) the async relay, `early_deliver`
//! written first. A dial error resets leg C and returns
//! `MtlsEnforcementError::GuestDial`. Leg S ends only through the
//! `GuestStream`'s drop, and `teardown` awaits the relay's end.
//!
//! Driving port: `MtlsEnforcement::{probe, enforce, liveness, teardown}` on
//! the production `HostMtlsEnforcement` (real rustls server handshake, real
//! kTLS on a real loopback TCP leg C). Driven port: `GuestStreamDial`, the
//! constructor argument the boot fills with its `OwnerDial`. Here it is a
//! test-local scripted dial whose `GuestStream` carries a `GuestStreamIo`
//! over an in-memory pipe to a test "guest application" — the owner's vsock
//! adapter is not this file's subject (the owner's `dial` has its own cells
//! and the through-`serve` halves are routed in the DISTILL blockers); the
//! subject is what the enforcement does with the dial: when it dials, what it
//! writes first, what it does on each end of each leg, and how it ends leg S.
//!
//! What the pipe models of the production stream (`GuestStreamIo`'s
//! contract): a guest half-close reads `Ok(0)` and reports
//! `end() == Some(HalfClose)`; a guest reset reads `Err(ConnectionReset)` and
//! reports `Some(Abort)`; `shutdown_write` is the host's 0x2; the stream's
//! drop with the host→guest direction open is the guest's 0x3. The pipe
//! records each of those as an event the cells read.
//!
//! Tier 3 (kTLS needs the real kernel): `cargo xtask lima run -- cargo
//! nextest run -p overdrive-dataplane --features integration-tests -E
//! 'test(mtls_inbound_guest_relay)'`. No netns, no TPROXY: the leg-C capture
//! is the worker's (step 05-01) and the enforcement takes `orig_dst` from
//! `Routed::Inbound` as given, so a loopback listener stands in for the
//! worker's accept.

#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "test fixtures assert their preconditions; leg names are contract vocabulary"
)]

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::AllocationId;
use overdrive_core::guest_transport::GUEST_NODE_PREFIX;
use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial, GuestStreamEnd, GuestStreamIo,
};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{
    InterceptedConnection, MtlsEnforcement, MtlsEnforcementError, MtlsLimits, PumpLiveness, Routed,
};
use overdrive_dataplane::mtls::HostMtlsEnforcement;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::helpers::mtls_pki::TestPki;

/// Which certificate the client presents.
#[derive(Debug, Clone, Copy)]
enum Presents {
    /// The test PKI's client leaf: chains to the node's bundle.
    TrustedClient,
    /// A leaf from another CA: fails peer verification.
    RogueClient,
}

/// The VM backend the client's connection was destined for: inside
/// `GUEST_NODE_PREFIX` (100.95.0.0/16), so the enforcement takes the guest
/// path, never the TCP leg-S dial.
const GUEST_BACKEND: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 7), 8080);
/// Request bytes, written in two client writes: the first immediately after
/// the handshake (it rides in the handshake's tail or while the dial is in
/// flight, so it is `early_deliver` or kernel-buffered), the second after.
const REQUEST_HEAD: &[u8] = b"GET /r5-54 HTTP/1.1\r\nhost: vm\r\n";
const REQUEST_TAIL: &[u8] = b"x-later: after-the-dial\r\n\r\n";
const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nguest";
/// How long every wait in this file may take.
const WAIT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

struct HeldIdentities {
    server: (AllocationId, SvidMaterial),
    bundle: TrustBundle,
}

impl IdentityRead for HeldIdentities {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        (alloc == &self.server.0).then(|| self.server.1.clone())
    }

    fn current_bundle(&self) -> Option<TrustBundle> {
        Some(self.bundle.clone())
    }
}

// ---------------------------------------------------------------------------
// The guest end: a GuestStreamIo over an in-memory pipe
// ---------------------------------------------------------------------------

/// What happened to the host end of leg S, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegSEvent {
    /// `shutdown_write`: the host's clean end of its direction (0x2).
    HostHalfClose,
    /// The stream was dropped; `host_direction_open` is the guest's 0x3.
    Dropped { host_direction_open: bool },
}

#[derive(Debug, Default)]
struct LegSRecord {
    events: Vec<LegSEvent>,
    /// The guest application reset its connection (the next host read and
    /// every later one fail `ConnectionReset`).
    guest_reset: bool,
}

#[derive(Debug)]
struct PipeIo {
    pipe: DuplexStream,
    end: Option<GuestStreamEnd>,
    shut: bool,
    record: Arc<Mutex<LegSRecord>>,
}

impl PipeIo {
    fn record(&self) -> &Mutex<LegSRecord> {
        &self.record
    }
}

#[async_trait::async_trait]
impl GuestStreamIo for PipeIo {
    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.end == Some(GuestStreamEnd::Abort) {
            return Err(ErrorKind::ConnectionReset.into());
        }
        let n = self.pipe.read(buf).await?;
        if n == 0 {
            if self.record().lock().guest_reset {
                self.end = Some(GuestStreamEnd::Abort);
                return Err(ErrorKind::ConnectionReset.into());
            }
            self.end = Some(GuestStreamEnd::HalfClose);
        }
        Ok(n)
    }

    async fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.shut {
            return Err(ErrorKind::BrokenPipe.into());
        }
        if self.record().lock().guest_reset {
            return Err(ErrorKind::ConnectionReset.into());
        }
        self.pipe.write(buf).await
    }

    async fn shutdown_write(&mut self) -> std::io::Result<()> {
        if self.shut {
            return Ok(());
        }
        self.pipe.shutdown().await?;
        self.shut = true;
        self.record().lock().events.push(LegSEvent::HostHalfClose);
        Ok(())
    }

    fn end(&self) -> Option<GuestStreamEnd> {
        self.end
    }
}

impl Drop for PipeIo {
    fn drop(&mut self) {
        let host_direction_open = !self.shut;
        self.record().lock().events.push(LegSEvent::Dropped { host_direction_open });
    }
}

// ---------------------------------------------------------------------------
// The scripted dial
// ---------------------------------------------------------------------------

/// What the dial does when called.
enum DialScript {
    /// After `delay`, pair: return a `GuestStream` whose guest end is handed
    /// to the cell.
    Pair { delay: Duration },
    /// After `delay`, refuse with `error`.
    Refuse { delay: Duration, error: GuestDialError },
}

struct ScriptedDial {
    script: DialScript,
    calls: Mutex<Vec<(SocketAddrV4, DialPurpose, Instant)>>,
    guest: Mutex<Option<DuplexStream>>,
    record: Arc<Mutex<LegSRecord>>,
}

impl ScriptedDial {
    fn new(script: DialScript) -> Arc<Self> {
        Arc::new(Self {
            script,
            calls: Mutex::new(Vec::new()),
            guest: Mutex::new(None),
            record: Arc::new(Mutex::new(LegSRecord::default())),
        })
    }

    fn calls(&self) -> Vec<(SocketAddrV4, DialPurpose)> {
        self.calls.lock().iter().map(|(t, p, _)| (*t, *p)).collect()
    }

    fn events(&self) -> Vec<LegSEvent> {
        self.record.lock().events.clone()
    }

    /// The guest application's end of leg S, once the dial paired.
    async fn guest_app(&self) -> DuplexStream {
        let deadline = Instant::now() + WAIT;
        loop {
            let paired = self.guest.lock().take();
            if let Some(g) = paired {
                return g;
            }
            assert!(Instant::now() < deadline, "the enforcement never dialled the guest");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Waits until the stream was dropped.
    async fn dropped(&self) -> bool {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if self.events().iter().any(|e| matches!(e, LegSEvent::Dropped { .. })) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        false
    }
}

#[async_trait::async_trait]
impl GuestStreamDial for ScriptedDial {
    async fn dial(
        &self,
        target: SocketAddrV4,
        purpose: DialPurpose,
    ) -> Result<GuestStream, GuestDialError> {
        self.calls.lock().push((target, purpose, Instant::now()));
        match &self.script {
            DialScript::Refuse { delay, error } => {
                tokio::time::sleep(*delay).await;
                Err(*error)
            }
            DialScript::Pair { delay } => {
                tokio::time::sleep(*delay).await;
                let (host, guest) = tokio::io::duplex(64 * 1024);
                *self.guest.lock() = Some(guest);
                let io =
                    PipeIo { pipe: host, end: None, shut: false, record: Arc::clone(&self.record) };
                Ok(GuestStream::new(target, Box::new(io)))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The client (a blocking rustls TLS 1.3 client on its own thread)
// ---------------------------------------------------------------------------

struct Client {
    conn: rustls::ClientConnection,
    tcp: TcpStream,
}

impl Client {
    /// Connects to `addr` and completes the handshake presenting `leaf`.
    /// `Err` with the handshake's I/O or TLS error.
    fn connect(
        addr: std::net::SocketAddr,
        pki: &TestPki,
        presents: Presents,
    ) -> std::io::Result<Self> {
        let rogue;
        let leaf = match presents {
            Presents::TrustedClient => &pki.client_leaf,
            Presents::RogueClient => {
                rogue = pki.untrusted_client_leaf();
                &rogue
            }
        };
        let mut roots = rustls::RootCertStore::empty();
        let mut rd = std::io::BufReader::new(pki.ca_cert_pem().as_bytes());
        for c in rustls_pemfile::certs(&mut rd) {
            roots.add(c.expect("ca cert")).expect("add ca cert");
        }
        let cfg = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_client_auth_cert(
                vec![leaf.cert_der.clone(), pki.intermediate_cert_der()],
                leaf.key_der.clone_key(),
            )
            .expect("client config");
        let sni = rustls::pki_types::ServerName::try_from(TestPki::SERVER_SNI.to_owned())
            .expect("server SNI");
        let mut conn = rustls::ClientConnection::new(Arc::new(cfg), sni).expect("ClientConnection");
        let mut tcp = TcpStream::connect(addr)?;
        tcp.set_nodelay(true)?;
        tcp.set_read_timeout(Some(WAIT))?;
        while conn.is_handshaking() {
            while conn.wants_write() {
                conn.write_tls(&mut tcp)?;
            }
            if !conn.is_handshaking() {
                break;
            }
            if conn.read_tls(&mut tcp)? == 0 {
                return Err(ErrorKind::UnexpectedEof.into());
            }
            conn.process_new_packets().map_err(std::io::Error::other)?;
        }
        Ok(Self { conn, tcp })
    }

    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let mut tls = rustls::Stream::new(&mut self.conn, &mut self.tcp);
        tls.write_all(bytes)?;
        tls.flush()
    }

    /// Reads until `len` bytes, end of stream or an error.
    fn read_up_to(&mut self, len: usize) -> (Vec<u8>, std::io::Result<()>) {
        let mut got = Vec::new();
        let mut buf = [0_u8; 4096];
        while got.len() < len {
            let mut tls = rustls::Stream::new(&mut self.conn, &mut self.tcp);
            match tls.read(&mut buf) {
                Ok(0) => return (got, Ok(())),
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(e) => return (got, Err(e)),
            }
        }
        (got, Ok(()))
    }

    /// Reads until the server's end: `Ok(())` for a clean end (close_notify
    /// or FIN), the error otherwise.
    fn read_to_end(&mut self) -> std::io::Result<Vec<u8>> {
        let mut got = Vec::new();
        let mut buf = [0_u8; 4096];
        loop {
            let mut tls = rustls::Stream::new(&mut self.conn, &mut self.tcp);
            match tls.read(&mut buf) {
                Ok(0) => return Ok(got),
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(got),
                Err(e) => return Err(e),
            }
        }
    }

    /// The client's clean half-close: close_notify, then FIN.
    fn half_close(&mut self) -> std::io::Result<()> {
        self.conn.send_close_notify();
        while self.conn.wants_write() {
            self.conn.write_tls(&mut self.tcp)?;
        }
        self.tcp.shutdown(Shutdown::Write)
    }

    /// The client's reset: `SO_LINGER{1,0}` and close.
    fn reset(self) {
        let linger = libc::linger { l_onoff: 1, l_linger: 0 };
        // SAFETY: setsockopt on an owned, open descriptor with a correctly sized value.
        let rc = unsafe {
            libc::setsockopt(
                std::os::fd::AsRawFd::as_raw_fd(&self.tcp),
                libc::SOL_SOCKET,
                libc::SO_LINGER,
                std::ptr::from_ref(&linger).cast(),
                libc::socklen_t::try_from(std::mem::size_of::<libc::linger>()).expect("fits"),
            )
        };
        assert_eq!(rc, 0, "SO_LINGER: {}", std::io::Error::last_os_error());
        drop(self);
    }
}

// ---------------------------------------------------------------------------
// The rig: the production enforcement over the scripted dial
// ---------------------------------------------------------------------------

struct Rig {
    pki: Arc<TestPki>,
    enforcement: Arc<HostMtlsEnforcement>,
    listener: TcpListener,
}

impl Rig {
    async fn new(dial: Arc<ScriptedDial>, limits: MtlsLimits) -> Self {
        let pki = Arc::new(TestPki::mint());
        let identity: Arc<dyn IdentityRead> = Arc::new(HeldIdentities {
            server: (pki.server_alloc.clone(), pki.server_svid_material()),
            bundle: pki.trust_bundle(),
        });
        let enforcement = Arc::new(HostMtlsEnforcement::new(
            identity,
            limits,
            GUEST_NODE_PREFIX,
            dial as Arc<dyn GuestStreamDial>,
        ));
        enforcement.probe().await.expect("the Earned-Trust probe passes on the real kernel");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the leg-C stand-in listener");
        Self { pki, enforcement, listener }
    }

    /// Spawns a client thread presenting `leaf` that runs `body` after its
    /// handshake, accepts leg C, and returns the intercepted connection (the
    /// worker's hand-off) and the client thread.
    fn intercept<F, T>(
        &self,
        presents: Presents,
        body: F,
    ) -> (InterceptedConnection, std::thread::JoinHandle<T>)
    where
        F: FnOnce(std::io::Result<Client>) -> T + Send + 'static,
        T: Send + 'static,
    {
        let addr = self.listener.local_addr().expect("listener addr");
        let pki = Arc::clone(&self.pki);
        let client = std::thread::spawn(move || body(Client::connect(addr, &pki, presents)));
        let (leg, _) = self.listener.accept().expect("accept leg C");
        let conn = InterceptedConnection {
            leg: OwnedFd::from(leg),
            routed: Routed::Inbound { orig_dst: GUEST_BACKEND },
            alloc: self.pki.server_alloc.clone(),
            expected_peer: None,
        };
        (conn, client)
    }
}

fn join<T>(handle: std::thread::JoinHandle<T>) -> T {
    handle.join().expect("the client thread did not panic")
}

// ---------------------------------------------------------------------------
// Cells
// ---------------------------------------------------------------------------

/// R5-54: an authenticated client's connection to a VM backend is dialled
/// once, as `Relay`, to the original destination; the request — its first
/// bytes sent before the dial paired — reaches the guest application
/// byte-exact and in order (the 0.5-RTT plaintext first), the reply reaches
/// the client byte-exact; the client's clean half-close is the guest's 0x2;
/// the guest application's half-close half-closes leg C; with both
/// directions ended the relay ends and leg S ends by the stream's drop with
/// the host direction already closed (no 0x3).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER attachment-cut step (9): LEGS-DIAL establish split"]
async fn an_authenticated_client_is_relayed_to_the_guest_application_byte_exact() {
    let dial = ScriptedDial::new(DialScript::Pair { delay: Duration::from_millis(300) });
    let rig = Rig::new(Arc::clone(&dial), MtlsLimits::default()).await;
    let (conn, client) = rig.intercept(Presents::TrustedClient, |client| {
        let mut client = client.expect("the client's handshake completes");
        // Sent at once: lands before the dial pairs (it is the handshake
        // tail's 0.5-RTT plaintext or kernel-buffered on leg C).
        client.write(REQUEST_HEAD).expect("first request bytes");
        std::thread::sleep(Duration::from_millis(600));
        client.write(REQUEST_TAIL).expect("later request bytes");
        let (reply, read) = client.read_up_to(RESPONSE.len());
        client.half_close().expect("client half-close");
        let rest = client.read_to_end();
        (reply, read.map_err(|e| e.kind()), rest.map_err(|e| e.kind()))
    });

    let handle = rig.enforcement.enforce(conn).await.expect("enforce reaches the relay");
    assert_eq!(
        dial.calls(),
        vec![(GUEST_BACKEND, DialPurpose::Relay)],
        "one dial, to the original destination, counted as a relay"
    );
    assert_eq!(rig.enforcement.liveness(&handle), PumpLiveness::Running, "the relay runs");

    let mut guest = dial.guest_app().await;
    let mut request = vec![0_u8; REQUEST_HEAD.len() + REQUEST_TAIL.len()];
    tokio::time::timeout(WAIT, guest.read_exact(&mut request))
        .await
        .expect("the request reaches the guest within the bound")
        .expect("the guest reads the whole request");
    assert_eq!(request, [REQUEST_HEAD, REQUEST_TAIL].concat(), "byte-exact, early bytes first");
    guest.write_all(RESPONSE).await.expect("the guest replies");

    // The client's clean half-close reaches the guest as end of stream (0x2).
    let mut after = Vec::new();
    tokio::time::timeout(WAIT, guest.read_to_end(&mut after))
        .await
        .expect("the client's half-close reaches the guest within the bound")
        .expect("a clean end, not an error");
    assert!(after.is_empty(), "nothing after the request");
    assert_eq!(dial.events(), vec![LegSEvent::HostHalfClose], "the host sent 0x2, nothing else");

    // The guest application half-closes: leg C half-closes; both ended.
    guest.shutdown().await.expect("guest half-close");
    let (reply, read, rest) = join(client);
    assert_eq!(reply, RESPONSE, "the reply reaches the client byte-exact");
    assert_eq!(read, Ok(()));
    assert_eq!(rest, Ok(Vec::new()), "leg C ends cleanly after the guest's half-close");
    assert!(dial.dropped().await, "the relay ended and dropped leg S");
    assert_eq!(
        dial.events(),
        vec![LegSEvent::HostHalfClose, LegSEvent::Dropped { host_direction_open: false }],
        "leg S ended by the stream's drop, after the clean 0x2 (no 0x3)"
    );
    let deadline = Instant::now() + WAIT;
    while rig.enforcement.liveness(&handle) != PumpLiveness::Gone {
        assert!(Instant::now() < deadline, "the relay's end makes the connection Gone");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// R5-54 / LEGS-DIAL: the dial is awaited after the handshake and bounded by
/// the owner's pairing deadline, not by `handshake_deadline`: a dial that
/// takes longer than the configured handshake deadline (and well inside
/// `GUEST_DIAL_PAIRING_DEADLINE`) still relays.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER attachment-cut step (9): LEGS-DIAL establish split"]
async fn a_dial_slower_than_the_handshake_deadline_still_relays() {
    let limits =
        MtlsLimits { handshake_deadline: Duration::from_millis(400), ..MtlsLimits::default() };
    let dial = ScriptedDial::new(DialScript::Pair { delay: Duration::from_millis(1_500) });
    assert!(
        Duration::from_millis(1_500)
            < overdrive_core::guest_transport::wire::GUEST_DIAL_PAIRING_DEADLINE
    );
    let rig = Rig::new(Arc::clone(&dial), limits).await;
    let (conn, client) = rig.intercept(Presents::TrustedClient, |client| {
        let mut client = client.expect("the handshake completes inside its own deadline");
        client.write(REQUEST_HEAD).expect("request");
        let (reply, read) = client.read_up_to(RESPONSE.len());
        (reply, read.map_err(|e| e.kind()))
    });
    let started = Instant::now();
    let handle = rig.enforcement.enforce(conn).await.expect("the slow dial is not cut short");
    assert!(
        started.elapsed() >= Duration::from_millis(1_400),
        "enforce awaited the dial (the dial took 1.5 s)"
    );
    let mut guest = dial.guest_app().await;
    let mut request = vec![0_u8; REQUEST_HEAD.len()];
    tokio::time::timeout(WAIT, guest.read_exact(&mut request))
        .await
        .expect("bounded")
        .expect("request");
    assert_eq!(request, REQUEST_HEAD);
    guest.write_all(RESPONSE).await.expect("reply");
    let (reply, read) = join(client);
    assert_eq!((reply.as_slice(), read), (RESPONSE, Ok(())));
    rig.enforcement.teardown(handle).await.expect("teardown");
}

/// R5-54 / LEGS-DIAL: a client whose certificate does not chain to the trust
/// bundle fails peer verification in the handshake, and no dial is made — the
/// guest application never sees a connection and no flow quota is taken.
/// (The seeded-sim half's `AbsentBundle` is a node with no bundle; this is a
/// real bad client against a real bundle.)
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER attachment-cut step (9): LEGS-DIAL establish split"]
async fn a_client_failing_peer_verification_causes_no_dial() {
    let dial = ScriptedDial::new(DialScript::Pair { delay: Duration::ZERO });
    let rig = Rig::new(Arc::clone(&dial), MtlsLimits::default()).await;
    let (conn, client) = rig.intercept(Presents::RogueClient, |client| client.is_ok());
    let result = rig.enforcement.enforce(conn).await;
    assert!(
        matches!(result, Err(MtlsEnforcementError::PeerVerificationFailed { .. })),
        "a client certificate that does not chain refuses the connection; got {result:?}"
    );
    assert!(!join(client), "the client's handshake fails");
    assert!(dial.calls().is_empty(), "no dial was made for an unauthenticated client");
    assert!(dial.events().is_empty(), "no leg S existed");
}

/// R5-54 / LEGS-DIAL: a dial that ends in an error (here the owner's
/// `PairingDeadline`, after the delay a real pairing deadline takes) resets
/// leg C — the client sees a reset, not a clean end — and `enforce` returns
/// `MtlsEnforcementError::GuestDial` naming the dial's outcome.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER attachment-cut step (9): LEGS-DIAL establish split"]
async fn a_dial_error_resets_leg_c_and_is_typed() {
    let dial = ScriptedDial::new(DialScript::Refuse {
        delay: Duration::from_millis(500),
        error: GuestDialError::PairingDeadline,
    });
    let rig = Rig::new(Arc::clone(&dial), MtlsLimits::default()).await;
    let (conn, client) = rig.intercept(Presents::TrustedClient, |client| {
        let mut client = client.expect("the handshake completes");
        let _ = client.write(REQUEST_HEAD);
        client.read_to_end().map_err(|e| e.kind())
    });
    let result = rig.enforcement.enforce(conn).await;
    assert!(
        matches!(
            result,
            Err(MtlsEnforcementError::GuestDial { source: GuestDialError::PairingDeadline })
        ),
        "the dial's error is returned typed; got {result:?}"
    );
    assert_eq!(dial.calls(), vec![(GUEST_BACKEND, DialPurpose::Relay)]);
    assert_eq!(join(client), Err(ErrorKind::ConnectionReset), "leg C is reset (SO_LINGER{{1,0}})");
}

/// R5-54: the TLS client resets leg C mid-relay: the relay ends and drops leg
/// S with the host→guest direction open, which is the guest's 0x3 (the guest
/// application sees its connection end; it never sees a clean 0x2).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER attachment-cut step (9): LEGS-DIAL establish split"]
async fn a_client_reset_drops_leg_s_with_its_direction_open() {
    let dial = ScriptedDial::new(DialScript::Pair { delay: Duration::ZERO });
    let rig = Rig::new(Arc::clone(&dial), MtlsLimits::default()).await;
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let (conn, client) = rig.intercept(Presents::TrustedClient, move |client| {
        let mut client = client.expect("the handshake completes");
        client.write(REQUEST_HEAD).expect("request");
        go_rx.recv().expect("the cell says when");
        client.reset();
    });
    let _handle = rig.enforcement.enforce(conn).await.expect("relay");
    let mut guest = dial.guest_app().await;
    let mut request = vec![0_u8; REQUEST_HEAD.len()];
    tokio::time::timeout(WAIT, guest.read_exact(&mut request))
        .await
        .expect("bounded")
        .expect("request");
    go_tx.send(()).expect("client alive");
    join(client);
    assert!(dial.dropped().await, "the relay ended and dropped leg S");
    assert_eq!(
        dial.events(),
        vec![LegSEvent::Dropped { host_direction_open: true }],
        "a client reset is the guest's 0x3: dropped with the host direction open, no 0x2"
    );
}

/// R5-54: the guest application resets its connection: the stream reads
/// `ConnectionReset` (`end() == Abort`) and the relay resets leg C — the
/// client sees a reset, not a clean end.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER attachment-cut step (9): LEGS-DIAL establish split"]
async fn a_guest_application_reset_resets_leg_c() {
    let dial = ScriptedDial::new(DialScript::Pair { delay: Duration::ZERO });
    let rig = Rig::new(Arc::clone(&dial), MtlsLimits::default()).await;
    let (conn, client) = rig.intercept(Presents::TrustedClient, |client| {
        let mut client = client.expect("the handshake completes");
        client.write(REQUEST_HEAD).expect("request");
        client.read_to_end().map_err(|e| e.kind())
    });
    let _handle = rig.enforcement.enforce(conn).await.expect("relay");
    let guest = dial.guest_app().await;
    dial.record.lock().guest_reset = true;
    drop(guest);
    assert_eq!(join(client), Err(ErrorKind::ConnectionReset), "leg C is reset");
    assert!(dial.dropped().await, "the relay ended and dropped leg S");
}

/// R5-54 / D-14: `teardown` of a relayed connection stops the relay and
/// awaits its end: when `teardown` returns, leg S has already been dropped
/// (its watch aborted — the guest's 0x3 — so the owner can release the
/// flow's quota), leg C is closed and the connection is `Gone`. A teardown
/// that returned while the drop were only queued would fail the first
/// assertion after it.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER attachment-cut step (9): LEGS-DIAL establish split"]
async fn teardown_returns_only_after_the_relay_dropped_leg_s() {
    let dial = ScriptedDial::new(DialScript::Pair { delay: Duration::ZERO });
    let rig = Rig::new(Arc::clone(&dial), MtlsLimits::default()).await;
    let (conn, client) = rig.intercept(Presents::TrustedClient, |client| {
        let mut client = client.expect("the handshake completes");
        client.write(REQUEST_HEAD).expect("request");
        client.read_to_end().map_err(|e| e.kind())
    });
    let handle = rig.enforcement.enforce(conn).await.expect("relay");
    let mut guest = dial.guest_app().await;
    let mut request = vec![0_u8; REQUEST_HEAD.len()];
    tokio::time::timeout(WAIT, guest.read_exact(&mut request))
        .await
        .expect("bounded")
        .expect("request");
    assert!(dial.events().is_empty(), "precondition: leg S is live");

    rig.enforcement.teardown(handle.clone()).await.expect("teardown");
    // No wait: the drop must already have happened.
    assert_eq!(
        dial.events(),
        vec![LegSEvent::Dropped { host_direction_open: true }],
        "teardown returned only after the relay dropped leg S (0x3)"
    );
    assert_eq!(rig.enforcement.liveness(&handle), PumpLiveness::Gone);
    let ended = join(client);
    assert!(ended.is_err() || ended == Ok(Vec::new()), "leg C is closed: {ended:?}");
}
