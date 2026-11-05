//! Public-ingress kernel-relay exchange scenarios over the production
//! `UnboundGatewayBuilder` -> `GatewayBuilder` -> `StartedGateway` path, a
//! real redb Route store and `SimGatewayRelay` (S-PIG-77, S-PIG-78, S-PIG-83,
//! 87 through 99, 101 and 118 through 122).
//!
//! Every public client is a real loopback TCP connection to the gateway's own
//! `127/8` address on port 443; the scripted TLS peer behind it (what the
//! client sends, what the origin answers, control records between octets) is
//! the `SimGatewayRelay` script, and the oracle is what the gateway wrote to
//! each leg, read from the Sim's per-leg transcript. The injected `SimClock`
//! is the only clock: a stall is a script `Quiet` span plus a test-driven
//! `tick`, never a sleep.
//!
//! The driven ports other than the relay are test-local doubles pinned to the
//! DESIGN shapes (`overdrive-sim` carries no `GatewayConnectDataplane` or
//! `GatewayClientMtls` Sim yet): `GatewayConnectDataplane` counts
//! `open_upstream` and `cleanup` calls, `GatewayClientMtls` arms the
//! authenticated socket through `SimGatewayRelay::arm_upstream_leg` (the same
//! backend, as the composition roots do), `GatewayIdentityLifecycleControl`
//! answers with current facts. The authenticated arm of the promotion probe is
//! answered with an inert leg and never touches the relay script; the doubles
//! switch to the relay once the Route is promoted.
//!
//! # Bind-address table (R-6)
//!
//! A composition binds its own `127.0.0.0/8` address on TCP/443. nextest gives
//! each test its own process, so every test derives its addresses from its own
//! second octet `T`; `Slot::bind(ordinal)` is `127.T.hi.lo` (`hi = ordinal /
//! 250`, `lo = ordinal % 250 + 1`) and `Slot::vip(ordinal)` is
//! `127.T.(200 + hi).lo` (the Service frontend, with a listening socket on
//! port 8080 so the connector's real `connect(2)` completes). This file owns
//! second octets 68..=96; the sibling integration file owns 41..=62 and the
//! in-crate drivers 63..=67; no other test may reuse them. The request-budget
//! scenarios of R-37 own 130..=132 (rows below).
//!
//! | `T` | test |
//! |---|---|
//! | 68 | two keep-alive exchanges with exact bodies (S-PIG-87) |
//! | 69 | chunked bodies relay chunk by chunk (S-PIG-88) |
//! | 70 | responses without a body (S-PIG-89) |
//! | 71 | interim heads and `Expect` (S-PIG-83) |
//! | 72 | harmless control records (S-PIG-90) |
//! | 73 | fatal control records (S-PIG-90) |
//! | 74 | failed KeyUpdate answers (S-PIG-90) |
//! | 75 | close-delimited response (S-PIG-91) |
//! | 76 | upstream failure projection (S-PIG-92) |
//! | 77 | early answer and lingering close (S-PIG-93); a second composition, ordinal 1, for NoBackend |
//! | 78 | body ceilings (S-PIG-94) |
//! | 79 | relay pipe failure (S-PIG-95) |
//! | 80 | keep-alive and deadlines (S-PIG-96) |
//! | 81 | single upstream attempt (S-PIG-97) |
//! | 82 | upstream arming refused (S-PIG-98) |
//! | 83 | public arming refused (S-PIG-98) |
//! | 84 | relay probe refused at the gateway gate (S-PIG-99); six sequential compositions, ordinal 0 |
//! | 85 | drain (S-PIG-101); four compositions, ordinals 0..=3 |
//! | 86 | response-side limits (S-PIG-119) |
//! | 87 | response-head deadline (S-PIG-120) |
//! | 88 | early answer is not a 408 (S-PIG-121) |
//! | 89 | public half-close (S-PIG-122) |
//! | 90 | ambiguous request framing (S-PIG-77) |
//! | 91 | ambiguous response framing (S-PIG-78) |
//! | 96 | public server configuration (S-PIG-118) |
//! | 130 | response head after a slow upload (S-PIG-120, 118 s arm) |
//! | 131 | stalled request body (S-PIG-121, 408 arms) |
//! | 132 | trickled request body (S-PIG-121, 120 s arm) |

#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::significant_drop_tightening,
    clippy::missing_const_for_fn,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::unused_self,
    clippy::format_push_string,
    clippy::unchecked_time_subtraction,
    reason = "CONTRACT_SHAPE metadata, long scenario bodies and fixture preconditions; wire builders append formatted lines and the deadline arithmetic is over fixed first-slice values"
)]

use std::collections::BTreeMap;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::num::{NonZeroU64, NonZeroUsize};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, ServiceV1, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::gateway_identity::{
    GatewayIdentityEpoch, GatewayIdentityFacts, GatewayIdentityIssueRecord,
};
use overdrive_core::id::{BackendId, CertSerial, ContentHash, NodeId, ServiceVip, SpiffeId};
use overdrive_core::public_ingress::{
    GatewayApplicationStatusRowV1, GatewayApplicationUnavailableCause, GatewayConnectPathStatus,
    GatewayListenerStatus, PublicCertifiedKeyId, PublicHostname, PublicRouteInput, Route,
    ServiceKey, ServiceListenerReferenceInput, SocketCookie,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_gateway::application::{
    GatewayHandle, GatewayRouteCommands, GatewayRuntimeDependencies, GatewayShutdownReport,
    GatewayStatusReads, UnboundGatewayBuilder,
};
use overdrive_gateway::ports::{
    AcceptedPublicLeg, GatewayClientMtls, GatewayClientMtlsError, GatewayConnectCleanup,
    GatewayConnectCleanupSweep, GatewayConnectDataplane, GatewayConnectDataplaneError,
    GatewayConnectIntent, GatewayIdentityLifecycleControl, GatewayIdentityLifecycleError,
    GatewayRelay, GatewaySelectionReceipt, GatewayUpstream, GatewayUpstreamSeal, HeadTerminator,
    KeyUpdateError, KtlsArmError, KtlsLeg, KtlsReadHalf, KtlsWriteHalf, LegReadError,
    LegWriteError, PreparedUpstream, PublicHandshakeError, ReadDeadlines, RelayDeadlines,
    RelayError, RelayPipeError, RelayProbeError, ThroughBytes,
};
use overdrive_gateway::runtime::GatewayLimits;
use overdrive_gateway::{GatewayConfig, ManualCertifiedKeyConfig};
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::gateway::{
    LegRole, LegScript, LegTranscript, PublicLegScript, RelayCall, SimGatewayRelay, SubstrateLie,
};
use overdrive_sim::adapters::kek::SimKek;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::read_ports::SimServiceVipView;
use overdrive_store_local::LocalIntentStore;
use rustls::CipherSuite;
use tempfile::TempDir;
use tokio::sync::watch;

// ---------------------------------------------------------------------------
// Constants and per-test address slots (R-6)
// ---------------------------------------------------------------------------

const SERVICE_PORT: u16 = 8080;
const GRACE: Duration = Duration::from_secs(5);
/// Real-time ceiling for any wait on a gateway task.
const WAIT: Duration = Duration::from_secs(20);
const HOSTNAME: &str = "api.example.com";

/// One test's reserved second octet in `127.0.0.0/8`.
#[derive(Debug, Clone, Copy)]
struct Slot(u8);

impl Slot {
    const fn bind(self, ordinal: u16) -> Ipv4Addr {
        Ipv4Addr::new(127, self.0, (ordinal / 250) as u8, (ordinal % 250) as u8 + 1)
    }

    const fn vip(self, ordinal: u16) -> Ipv4Addr {
        Ipv4Addr::new(127, self.0, 200 + (ordinal / 250) as u8, (ordinal % 250) as u8 + 1)
    }
}

const TWO_KEEP_ALIVE: Slot = Slot(68);
const CHUNKED: Slot = Slot(69);
const BODYLESS: Slot = Slot(70);
const INTERIM: Slot = Slot(71);
const HARMLESS_CONTROL: Slot = Slot(72);
const FATAL_CONTROL: Slot = Slot(73);
const FAILED_ANSWER: Slot = Slot(74);
const CLOSE_DELIMITED: Slot = Slot(75);
const UPSTREAM_FAILURE: Slot = Slot(76);
const LINGERING: Slot = Slot(77);
const CEILINGS: Slot = Slot(78);
const PIPE_FAILURE: Slot = Slot(79);
const DEADLINES: Slot = Slot(80);
const SINGLE_ATTEMPT: Slot = Slot(81);
const UPSTREAM_ARM: Slot = Slot(82);
const PUBLIC_ARM: Slot = Slot(83);
const RELAY_PROBE: Slot = Slot(84);
const DRAIN: Slot = Slot(85);
const RESPONSE_LIMITS: Slot = Slot(86);
const RESPONSE_HEAD_DEADLINE: Slot = Slot(87);
const EARLY_ANSWER: Slot = Slot(88);
const HALF_CLOSE: Slot = Slot(89);
const AMBIGUOUS_REQUEST: Slot = Slot(90);
const AMBIGUOUS_RESPONSE: Slot = Slot(91);
const SERVER_CONFIGURATION: Slot = Slot(96);
const UPLOAD_COMPLETES_LATE: Slot = Slot(130);
const STALLED_REQUEST_BODY: Slot = Slot(131);
const TRICKLED_REQUEST_BODY: Slot = Slot(132);

// ---------------------------------------------------------------------------
// GatewayConnectDataplane double: counts opens and cleanups
// ---------------------------------------------------------------------------

#[derive(Default)]
struct DataplaneState {
    next_cookie: u64,
    next_backend: u32,
    registered: BTreeMap<SocketCookie, (GatewayConnectIntent, BackendId)>,
    pending_receipts: std::collections::BTreeSet<SocketCookie>,
    opens: u64,
    cleanups: u64,
}

/// Mints a cookie and a backend per `open_upstream`; answers `Selected` when a
/// backend is programmed and the ordinary `NoBackend` otherwise.
struct OriginDataplane {
    programmed: bool,
    state: Mutex<DataplaneState>,
}

fn backend_identity(backend_id: BackendId) -> SpiffeId {
    SpiffeId::new(&format!("spiffe://overdrive.local/workload/api/alloc/api-{}", backend_id.get()))
        .expect("backend SPIFFE ID")
}

impl OriginDataplane {
    fn new(programmed: bool) -> Arc<Self> {
        Arc::new(Self { programmed, state: Mutex::new(DataplaneState::default()) })
    }

    fn opens(&self) -> u64 {
        self.state.lock().expect("dataplane state").opens
    }

    fn cleanups(&self) -> u64 {
        self.state.lock().expect("dataplane state").cleanups
    }

    /// Registered intents and untaken receipts: the double's own ground truth.
    fn residue(&self) -> (usize, usize) {
        let state = self.state.lock().expect("dataplane state");
        (state.registered.len(), state.pending_receipts.len())
    }
}

impl GatewayConnectCleanup for OriginDataplane {
    fn cleanup(&self, intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("dataplane state");
        state.cleanups += 1;
        state.registered.remove(&intent.socket_cookie);
        state.pending_receipts.remove(&intent.socket_cookie);
        drop(state);
        Ok(())
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("dataplane state");
        let removed_intents = u32::try_from(state.registered.len()).expect("intent count");
        let removed_receipts = u32::try_from(state.pending_receipts.len()).expect("receipt count");
        state.registered.clear();
        state.pending_receipts.clear();
        drop(state);
        Ok(GatewayConnectCleanupSweep { removed_intents, removed_receipts })
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        let state = self.state.lock().expect("dataplane state");
        Ok(u32::try_from(state.registered.len()).expect("intent count"))
    }

    fn live_receipts(
        &self,
    ) -> Result<BTreeMap<SocketCookie, GatewaySelectionReceipt>, GatewayConnectDataplaneError> {
        let state = self.state.lock().expect("dataplane state");
        Ok(state
            .pending_receipts
            .iter()
            .filter_map(|cookie| {
                let (intent, backend) = state.registered.get(cookie)?;
                Some((
                    *cookie,
                    if self.programmed {
                        GatewaySelectionReceipt::Selected {
                            socket_cookie: intent.socket_cookie,
                            service_key: intent.service_key,
                            backend_id: *backend,
                        }
                    } else {
                        GatewaySelectionReceipt::NoBackend {
                            socket_cookie: intent.socket_cookie,
                            service_key: intent.service_key,
                        }
                    },
                ))
            })
            .collect())
    }
}

#[async_trait]
impl GatewayConnectDataplane for OriginDataplane {
    async fn probe(
        &self,
        _deadline: Instant,
        _clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        Ok(())
    }

    fn open_upstream(
        &self,
        key: ServiceKey,
    ) -> Result<PreparedUpstream, GatewayConnectDataplaneError> {
        let socket = tokio::net::TcpSocket::new_v4()
            .map_err(|error| GatewayConnectDataplaneError::SocketCreate(error.kind()))?;
        let mut state = self.state.lock().expect("dataplane state");
        state.opens += 1;
        state.next_cookie += 1;
        state.next_backend += 1;
        let cookie = SocketCookie::new(state.next_cookie).expect("nonzero cookie");
        let backend = BackendId::new(state.next_backend).expect("backend ID");
        let intent = GatewayConnectIntent { socket_cookie: cookie, service_key: key };
        state.registered.insert(cookie, (intent.clone(), backend));
        state.pending_receipts.insert(cookie);
        drop(state);
        Ok(PreparedUpstream::new(socket, intent))
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("dataplane state");
        let Some((registered, backend)) = state.registered.get(&intent.socket_cookie).cloned()
        else {
            return Err(GatewayConnectDataplaneError::IntentMissing);
        };
        if !state.pending_receipts.remove(&intent.socket_cookie) {
            return Err(GatewayConnectDataplaneError::ReceiptMissing);
        }
        drop(state);
        Ok(if self.programmed {
            GatewaySelectionReceipt::Selected {
                socket_cookie: registered.socket_cookie,
                service_key: registered.service_key,
                backend_id: backend,
            }
        } else {
            GatewaySelectionReceipt::NoBackend {
                socket_cookie: registered.socket_cookie,
                service_key: registered.service_key,
            }
        })
    }

    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        Ok(backend_identity(backend_id))
    }
}

// ---------------------------------------------------------------------------
// GatewayClientMtls double: arms through the same Sim relay
// ---------------------------------------------------------------------------

/// Before `start_serving` the authenticated arm of the promotion probe is an
/// inert leg; afterwards the socket is armed by `SimGatewayRelay`, which plays
/// the next scripted upstream leg or the queued arm refusal.
struct ArmingMtls {
    seal: GatewayUpstreamSeal,
    relay: Arc<SimGatewayRelay>,
    serving: AtomicBool,
    /// The peer presents an identity other than the selected one.
    unequal_peer: AtomicBool,
    peers: Mutex<Vec<SpiffeId>>,
}

impl ArmingMtls {
    fn new(seal: GatewayUpstreamSeal, relay: Arc<SimGatewayRelay>) -> Arc<Self> {
        Arc::new(Self {
            seal,
            relay,
            serving: AtomicBool::new(false),
            unequal_peer: AtomicBool::new(false),
            peers: Mutex::new(Vec::new()),
        })
    }

    fn start_serving(&self) {
        self.serving.store(true, Ordering::SeqCst);
    }

    /// From now on the peer presents a valid identity that is not the one the
    /// receipt selected.
    fn present_an_unequal_peer(&self) {
        self.unequal_peer.store(true, Ordering::SeqCst);
    }

    /// The `expected_peer` of every request-path `authenticate`, in call order.
    fn request_peers(&self) -> Vec<SpiffeId> {
        self.peers.lock().expect("peers").clone()
    }
}

#[async_trait]
impl GatewayClientMtls for ArmingMtls {
    async fn authenticate(
        &self,
        connected: OwnedFd,
        expected_peer: SpiffeId,
        _deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
        if !self.serving.load(Ordering::SeqCst) {
            drop(connected);
            return Ok(self
                .seal
                .seal_authenticated(KtlsLeg::from_armed(Box::new(()), Box::new(()))));
        }
        self.peers.lock().expect("peers").push(expected_peer.clone());
        if self.unequal_peer.load(Ordering::SeqCst) {
            drop(connected);
            return Err(GatewayClientMtlsError::PeerSpiffeMismatch {
                expected: expected_peer,
                actual: SpiffeId::new("spiffe://overdrive.local/workload/other/alloc/other-1")
                    .expect("other SPIFFE ID"),
            });
        }
        match self.relay.arm_upstream_leg(connected, &expected_peer) {
            Ok(leg) => Ok(self.seal.seal_authenticated(leg)),
            Err(kind) => Err(GatewayClientMtlsError::Arm(kind)),
        }
    }
}

// ---------------------------------------------------------------------------
// GatewayIdentityLifecycleControl double: current facts, synchronous clear
// ---------------------------------------------------------------------------

struct CurrentIdentity {
    facts: GatewayIdentityFacts,
    sender: watch::Sender<Option<GatewayIdentityFacts>>,
}

impl CurrentIdentity {
    fn new() -> Arc<Self> {
        let facts = GatewayIdentityFacts {
            epoch: GatewayIdentityEpoch::first(),
            spiffe_id: SpiffeId::new("spiffe://overdrive.local/gateway/gateway-node")
                .expect("gateway SPIFFE ID"),
            serial: CertSerial::new("01").expect("serial"),
            not_after: overdrive_core::UnixInstant::from_unix_duration(Duration::from_secs(
                4_000_000_000,
            )),
        };
        let (sender, _) = watch::channel(Some(facts.clone()));
        Arc::new(Self { facts, sender })
    }
}

#[async_trait]
impl GatewayIdentityLifecycleControl for CurrentIdentity {
    async fn ensure_current(
        &self,
        _deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError> {
        Ok(self.facts.clone())
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        self.sender.borrow().clone()
    }

    fn issue_record(&self) -> Option<GatewayIdentityIssueRecord> {
        None
    }

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.sender.subscribe()
    }

    fn disable_after_drain(&self) {
        self.sender.send_replace(None);
    }
}

// ---------------------------------------------------------------------------
// GatewayRelay decorator: keeps the public server configuration it was given
// ---------------------------------------------------------------------------

/// Forwards every call to the Sim relay and keeps the `ServerConfig` each
/// `accept_public` received, so a test can read the configuration the
/// production listener passed (S-PIG-118).
struct ConfigurationCapture {
    inner: Arc<SimGatewayRelay>,
    seen: Mutex<Vec<Arc<rustls::ServerConfig>>>,
}

impl ConfigurationCapture {
    fn new(inner: Arc<SimGatewayRelay>) -> Arc<Self> {
        Arc::new(Self { inner, seen: Mutex::new(Vec::new()) })
    }

    fn configurations(&self) -> Vec<Arc<rustls::ServerConfig>> {
        self.seen.lock().expect("configurations").clone()
    }
}

#[async_trait]
impl GatewayRelay for ConfigurationCapture {
    async fn probe(&self, deadline: Instant) -> Result<(), RelayProbeError> {
        self.inner.probe(deadline).await
    }

    async fn accept_public(
        &self,
        accepted: OwnedFd,
        tls: Arc<rustls::ServerConfig>,
        deadline: Instant,
    ) -> Result<AcceptedPublicLeg, PublicHandshakeError> {
        self.seen.lock().expect("configurations").push(Arc::clone(&tls));
        self.inner.accept_public(accepted, tls, deadline).await
    }

    async fn read_through(
        &self,
        from: &mut KtlsReadHalf,
        terminator: HeadTerminator,
        limit: NonZeroUsize,
        deadlines: ReadDeadlines,
    ) -> Result<ThroughBytes, LegReadError> {
        self.inner.read_through(from, terminator, limit, deadlines).await
    }

    async fn write_all(
        &self,
        to: &mut KtlsWriteHalf,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<(), LegWriteError> {
        self.inner.write_all(to, bytes, deadline).await
    }

    async fn relay_exact(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        len: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<(), RelayError> {
        self.inner.relay_exact(from, to, len, deadlines).await
    }

    async fn relay_to_end(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<u64, RelayError> {
        self.inner.relay_to_end(from, to, max, deadlines).await
    }

    async fn close_write(
        &self,
        to: &mut KtlsWriteHalf,
        deadline: Instant,
    ) -> Result<(), LegWriteError> {
        self.inner.close_write(to, deadline).await
    }

    async fn discard_remaining(
        &self,
        from: &mut KtlsReadHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<(), LegReadError> {
        self.inner.discard_remaining(from, max, deadlines).await
    }
}

// ---------------------------------------------------------------------------
// Site: certificate, key and the redb Route store of one gateway
// ---------------------------------------------------------------------------

struct Site {
    _dir: TempDir,
    chain_path: PathBuf,
    key_path: PathBuf,
    intent_path: PathBuf,
}

impl Site {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("isolated gateway root");
        let chain_path = dir.path().join("api-origin-chain.pem");
        let key_path = dir.path().join("api-origin-key.pem");
        let intent_path = dir.path().join("intent.redb");
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("test key");
        let mut params =
            rcgen::CertificateParams::new(vec![HOSTNAME.to_owned()]).expect("certificate params");
        params.is_ca = rcgen::IsCa::NoCa;
        params.not_before = rcgen::date_time_ymd(2020, 1, 1);
        params.not_after = rcgen::date_time_ymd(2100, 1, 1);
        let cert = params.self_signed(&key).expect("test certificate");
        std::fs::write(&chain_path, cert.pem()).expect("write certificate");
        std::fs::write(&key_path, key.serialize_pem()).expect("write private key");
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .expect("private-key mode");
        Self { _dir: dir, chain_path, key_path, intent_path }
    }

    fn config(&self, bind: Ipv4Addr) -> GatewayConfig {
        GatewayConfig::new(
            bind,
            ManualCertifiedKeyConfig::new(
                PublicCertifiedKeyId::new("api-origin").expect("key ID"),
                self.chain_path.clone(),
                self.key_path.clone(),
            ),
        )
    }
}

fn custody_kek() -> Arc<SimKek> {
    Arc::new(SimKek::new().with(
        &KekId::new("overdrive-public-certified-key").expect("custody KEK ID"),
        [0x5a; KEK_LEN],
    ))
}

fn route(path: &str) -> Route {
    Route::from_submit(PublicRouteInput {
        id: "public-api".to_owned(),
        hostname: HOSTNAME.to_owned(),
        path: path.to_owned(),
        path_match: "segment_prefix".to_owned(),
        certified_key: "api-origin".to_owned(),
        target: ServiceListenerReferenceInput {
            service: "api".to_owned(),
            port: SERVICE_PORT,
            protocol: "tcp".to_owned(),
        },
    })
    .expect("valid Route")
}

/// Persist the `api` Service through the real IntentStore and return the VIP
/// view that assigns it `vip`.
async fn persist_api_service(intent: &LocalIntentStore, vip: Ipv4Addr) -> Arc<SimServiceVipView> {
    let service = ServiceV1::from_submit(ServiceSpecInput {
        id: "api".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 10, memory_bytes: 16 * 1024 * 1024 },
        driver: DriverInput::Vm(VmInput {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: SERVICE_PORT, protocol: "tcp".to_owned() }],
        startup_probes: Vec::new(),
        readiness_probes: Vec::new(),
        liveness_probes: Vec::new(),
    })
    .expect("valid api Service input");
    let workload_id = service.id.clone();
    let service_intent = WorkloadIntent::Service(service);
    let digest = service_intent.spec_digest().expect("api Service digest");
    let archived = service_intent.archive_for_store().expect("archive api Service intent");
    intent
        .put(IntentKey::for_workload(&workload_id).as_bytes(), archived.as_ref())
        .await
        .expect("persist api Service");
    intent
        .put(
            IntentKey::for_workload_kind(&workload_id).as_bytes(),
            &[WorkloadKind::Service.discriminator_byte()],
        )
        .await
        .expect("persist api Service kind");
    Arc::new(SimServiceVipView::new(BTreeMap::<ContentHash, ServiceVip>::from([(
        digest,
        ServiceVip::new(vip.into()).expect("Service VIP"),
    )])))
}

// ---------------------------------------------------------------------------
// World: one production composition over the Sim relay
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Spec {
    /// A backend is programmed: requests are `Selected`; otherwise `NoBackend`.
    programmed: bool,
    /// The Route's path prefix.
    route_path: &'static str,
    /// The composition's address ordinal within its slot.
    ordinal: u16,
}

impl Default for Spec {
    fn default() -> Self {
        Self { programmed: true, route_path: "/", ordinal: 0 }
    }
}

struct World {
    clock: Arc<SimClock>,
    _routes: GatewayRouteCommands,
    _status: GatewayStatusReads,
    handle: Option<GatewayHandle>,
    relay: Arc<SimGatewayRelay>,
    dataplane: Arc<OriginDataplane>,
    mtls: Arc<ArmingMtls>,
    bind: SocketAddrV4,
    /// Opens and cleanups already counted when the Route was promoted.
    baseline_opens: u64,
    baseline_cleanups: u64,
    _site: Site,
    _intent: Arc<LocalIntentStore>,
    /// Keeps the Service frontend's TCP listener alive so the connector's real
    /// `connect(2)` completes.
    _frontend_listener: std::net::TcpListener,
}

impl World {
    async fn start(slot: Slot, spec: Spec) -> Self {
        Self::start_with(slot, spec, |relay| relay as Arc<dyn GatewayRelay>).await
    }

    /// Start with the relay the gateway sees chosen by `wrap` (the Sim relay
    /// itself, or a decorator around it).
    async fn start_with(
        slot: Slot,
        spec: Spec,
        wrap: impl FnOnce(Arc<SimGatewayRelay>) -> Arc<dyn GatewayRelay>,
    ) -> Self {
        let site = Site::new();
        let clock = Arc::new(SimClock::new());
        let intent =
            Arc::new(LocalIntentStore::open(&site.intent_path).expect("real intent store"));
        let node = NodeId::new("gateway-node").expect("node ID");
        let observations = Arc::new(SimObservationStore::single_peer(node.clone(), 54));
        let frontend_listener = std::net::TcpListener::bind((slot.vip(spec.ordinal), SERVICE_PORT))
            .expect("Service frontend listener");
        frontend_listener.set_nonblocking(true).expect("nonblocking frontend listener");
        let vip_view = persist_api_service(&intent, slot.vip(spec.ordinal)).await;
        let intent_port: Arc<dyn IntentStore> = intent.clone();
        let observation_port: Arc<dyn ObservationStore> = observations;
        let clock_port: Arc<dyn Clock> = clock.clone();
        let builder = UnboundGatewayBuilder::new(
            node,
            site.config(slot.bind(spec.ordinal)),
            intent_port,
            observation_port,
            vip_view,
            Arc::new(PublicCertifiedKeyAeadCodec::new(custody_kek())),
            clock_port.clone(),
        )
        .await
        .expect("gateway builder");
        let (builder, seal) = builder.into_builder();

        let relay = Arc::new(SimGatewayRelay::new(clock_port));
        let dataplane = OriginDataplane::new(spec.programmed);
        let mtls = ArmingMtls::new(seal, Arc::clone(&relay));
        let dependencies = GatewayRuntimeDependencies::new(
            GatewayLimits::first_slice(),
            dataplane.clone(),
            mtls.clone(),
            CurrentIdentity::new(),
            wrap(Arc::clone(&relay)),
        );
        let (handle, routes, status) = builder.start(dependencies).await.into_parts();
        routes.routes().apply(route(spec.route_path)).await.expect("declare the Route");
        wait_promoted(&status).await;
        mtls.start_serving();
        Self {
            baseline_opens: dataplane.opens(),
            baseline_cleanups: dataplane.cleanups(),
            clock,
            _routes: routes,
            _status: status,
            handle: Some(handle),
            relay,
            dataplane,
            mtls,
            bind: SocketAddrV4::new(slot.bind(spec.ordinal), 443),
            _site: site,
            _intent: intent,
            _frontend_listener: frontend_listener,
        }
    }

    /// `open_upstream` calls made since the Route was promoted.
    fn opens(&self) -> u64 {
        self.dataplane.opens() - self.baseline_opens
    }

    /// `cleanup` calls made since the Route was promoted.
    fn cleanups(&self) -> u64 {
        self.dataplane.cleanups() - self.baseline_cleanups
    }

    fn limits(&self) -> GatewayLimits {
        GatewayLimits::first_slice()
    }

    /// Queue one public TLS connection (SNI `api.example.com`) and the origin
    /// legs the exchange will mint, then connect a real client socket.
    fn start_exchange(&self, public: LegScript, upstreams: Vec<LegScript>) -> Running {
        self.start_exchange_for(HOSTNAME, public, upstreams)
    }

    fn start_exchange_for(
        &self,
        sni: &str,
        public: LegScript,
        upstreams: Vec<LegScript>,
    ) -> Running {
        let first = self.relay.leg_transcripts().len();
        self.relay.script_public_connection(PublicLegScript {
            sni: PublicHostname::new(sni).expect("SNI hostname"),
            inbound: public,
        });
        for upstream in upstreams {
            self.relay.script_upstream_leg(upstream);
        }
        let client = TcpStream::connect(self.bind).expect("connect to the gateway listener");
        Running { first, client }
    }

    /// Wait until the gateway accepted the public connection of `running`: the
    /// handshake completed, so the first request's clock has started.
    async fn accepted(&self, running: &Running) {
        wait_for("the public connection to be accepted", || {
            self.legs(running).public.is_some().then_some(())
        })
        .await;
    }

    /// Legs minted since `running` started.
    fn legs(&self, running: &Running) -> Legs {
        let transcripts: Vec<LegTranscript> = self
            .relay
            .leg_transcripts()
            .into_iter()
            .filter(|leg| leg.id.0 >= running.first as u64)
            .collect();
        Legs {
            public: transcripts.iter().find(|leg| leg.role == LegRole::Public).cloned(),
            upstream: transcripts.into_iter().filter(|leg| leg.role == LegRole::Upstream).collect(),
        }
    }

    /// Run an exchange to its end: the public leg and every upstream leg
    /// closed.
    async fn run(&self, public: LegScript, upstreams: Vec<LegScript>) -> Legs {
        let running = self.start_exchange(public, upstreams);
        self.finish(&running).await
    }

    async fn finish(&self, running: &Running) -> Legs {
        wait_for("the exchange to end with every leg closed", || {
            let legs = self.legs(running);
            let public_closed = legs.public.as_ref().is_some_and(|leg| leg.closed);
            (public_closed && legs.upstream.iter().all(|leg| leg.closed)).then_some(legs)
        })
        .await
    }

    /// Advance the injected clock by `total` in `step` increments, letting the
    /// gateway tasks run between increments.
    async fn advance(&self, total: Duration, step: Duration) {
        let mut left = total;
        while !left.is_zero() {
            let now = step.min(left);
            self.clock.tick(now);
            left -= now;
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    /// Graceful shutdown; consumes the handle.
    async fn shutdown_with_clock(&mut self, step: Duration) -> GatewayShutdownReport {
        let handle = self.handle.take().expect("gateway handle");
        let clock = Arc::clone(&self.clock);
        drive_clock(&clock, step, handle.shutdown(GRACE)).await
    }
}

/// One in-flight exchange: where its legs start, and the real client socket.
struct Running {
    first: usize,
    client: TcpStream,
}

/// The legs of one exchange.
#[derive(Debug, Clone)]
struct Legs {
    public: Option<LegTranscript>,
    upstream: Vec<LegTranscript>,
}

impl Legs {
    fn public(&self) -> &LegTranscript {
        self.public.as_ref().expect("the public leg was accepted")
    }

    fn public_messages(&self) -> Vec<Wire> {
        parse_wire(&self.public().written, Kind::Response).0
    }

    fn public_statuses(&self) -> Vec<u16> {
        self.public_messages().iter().map(Wire::status).collect()
    }

    fn upstream_requests(&self, leg: usize) -> Vec<Wire> {
        parse_wire(&self.upstream[leg].written, Kind::Request).0
    }
}

/// Drive `future` to completion while advancing the injected clock by `step`
/// every few real milliseconds.
async fn drive_clock<T>(clock: &SimClock, step: Duration, future: impl Future<Output = T>) -> T {
    tokio::pin!(future);
    loop {
        tokio::select! {
            result = &mut future => return result,
            () = tokio::time::sleep(Duration::from_millis(3)) => clock.tick(step),
        }
    }
}

/// Start a gateway whose relay shows `lie` (or, for `None`, the honest
/// substrate) to the gate's probe. Behind a lie nothing is ever promoted: the
/// caller waits for the refusal; behind the honest substrate it waits for the
/// promotion.
async fn start_behind_substrate(
    lie: Option<SubstrateLie>,
) -> (Arc<SimClock>, GatewayStatusReads, GatewayHandle, SocketAddrV4, Site) {
    let slot = RELAY_PROBE;
    let site = Site::new();
    let clock = Arc::new(SimClock::new());
    let intent = Arc::new(LocalIntentStore::open(&site.intent_path).expect("real intent store"));
    let node = NodeId::new("gateway-node").expect("node ID");
    let observations = Arc::new(SimObservationStore::single_peer(node.clone(), 54));
    let vip_view = persist_api_service(&intent, slot.vip(0)).await;
    let intent_port: Arc<dyn IntentStore> = intent;
    let observation_port: Arc<dyn ObservationStore> = observations;
    let clock_port: Arc<dyn Clock> = clock.clone();
    let builder = UnboundGatewayBuilder::new(
        node,
        site.config(slot.bind(0)),
        intent_port,
        observation_port,
        vip_view,
        Arc::new(PublicCertifiedKeyAeadCodec::new(custody_kek())),
        clock_port.clone(),
    )
    .await
    .expect("gateway builder");
    let (builder, seal) = builder.into_builder();
    let relay = Arc::new(SimGatewayRelay::new(clock_port));
    relay.present_substrate(lie);
    let dependencies = GatewayRuntimeDependencies::new(
        GatewayLimits::first_slice(),
        OriginDataplane::new(true),
        ArmingMtls::new(seal, Arc::clone(&relay)),
        CurrentIdentity::new(),
        relay as Arc<dyn GatewayRelay>,
    );
    let (handle, routes, status) = builder.start(dependencies).await.into_parts();
    routes.routes().apply(route("/")).await.expect("declare the Route");
    (clock, status, handle, SocketAddrV4::new(slot.bind(0), 443), site)
}

async fn wait_refused(status: &GatewayStatusReads) -> GatewayApplicationStatusRowV1 {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let row = status.application_status().await.expect("application status read");
        if let Some(row) = row
            && row.unavailable == Some(GatewayApplicationUnavailableCause::ConnectPathUnavailable)
        {
            return row;
        }
        assert!(std::time::Instant::now() < deadline, "timed out waiting for the gate to refuse");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_promoted(status: &GatewayStatusReads) -> GatewayApplicationStatusRowV1 {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let row = status.application_status().await.expect("application status read");
        if let Some(row) = row
            && row.current.is_some()
            && row.unavailable.is_none()
            && matches!(row.listener, GatewayListenerStatus::Bound { .. })
        {
            return row;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for a promoted Current generation"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

// ---------------------------------------------------------------------------
// Wire builders and a small HTTP/1.1 reader over the transcripts
// ---------------------------------------------------------------------------

/// A request head: request line, `Host: api.example.com` unless `headers`
/// names a Host, the given headers, and the blank line.
fn request_head(method: &str, target: &str, headers: &[(&str, &str)]) -> Vec<u8> {
    let mut head = format!("{method} {target} HTTP/1.1\r\n");
    if !headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("host")) {
        head.push_str(&format!("Host: {HOSTNAME}\r\n"));
    }
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    head.into_bytes()
}

fn get(target: &str) -> Vec<u8> {
    request_head("GET", target, &[])
}

/// A request with a `Content-Length` body.
fn post(target: &str, body: &[u8]) -> Vec<u8> {
    let mut wire = request_head("POST", target, &[("Content-Length", &body.len().to_string())]);
    wire.extend_from_slice(body);
    wire
}

/// A response head with the given status line remainder and headers.
fn response_head(status: u16, reason: &str, headers: &[(&str, &str)]) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    head.into_bytes()
}

/// A complete `Content-Length` response.
fn ok_response(body: &[u8]) -> Vec<u8> {
    let mut wire = response_head(200, "OK", &[("Content-Length", &body.len().to_string())]);
    wire.extend_from_slice(body);
    wire
}

/// A complete `Content-Length` response with an arbitrary status.
fn response(status: u16, body: &[u8]) -> Vec<u8> {
    let mut wire = response_head(status, "Status", &[("Content-Length", &body.len().to_string())]);
    wire.extend_from_slice(body);
    wire
}

/// Octets whose values cycle so that a shifted or duplicated span is visible.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Request,
    Response,
}

/// One HTTP message read back from a transcript.
#[derive(Debug, Clone)]
struct Wire {
    start_line: String,
    headers: Vec<(String, String)>,
    /// The body with any chunk framing removed.
    body: Vec<u8>,
    /// The body octets exactly as written (chunk framing included).
    raw_body: Vec<u8>,
}

impl Wire {
    fn status(&self) -> u16 {
        self.start_line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or_else(|| panic!("not a status line: {:?}", self.start_line))
    }

    fn method(&self) -> &str {
        self.start_line.split_whitespace().next().unwrap_or_default()
    }

    fn target(&self) -> &str {
        self.start_line.split_whitespace().nth(1).unwrap_or_default()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

/// Read every complete message in `bytes`; returns the messages and the number
/// of octets they consumed (anything after is a partial message).
/// `head_replies[i]` marks response `i` as the answer to a `HEAD` request: it
/// carries no body whatever it declares.
fn parse_wire_for(bytes: &[u8], kind: Kind, head_replies: &[bool]) -> (Vec<Wire>, usize) {
    let mut messages: Vec<Wire> = Vec::new();
    let mut at = 0;
    loop {
        let Some(end) = find(bytes, b"\r\n\r\n", at) else { return (messages, at) };
        let head = String::from_utf8_lossy(&bytes[at..end]).into_owned();
        let mut lines = head.split("\r\n");
        let start_line = lines.next().unwrap_or_default().to_owned();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
            .collect();
        let body_at = end + 4;
        let find_header = |name: &str| -> Option<String> {
            headers
                .iter()
                .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.clone())
        };
        let status = start_line.split_whitespace().nth(1).and_then(|code| code.parse::<u16>().ok());
        let bodyless_status = kind == Kind::Response
            && status.is_some_and(|code| {
                (100..200).contains(&code) || code == 204 || code == 205 || code == 304
            });
        let reply_to_head = kind == Kind::Response
            && head_replies.get(messages.iter().filter(|m| m.status() >= 200).count())
                == Some(&true);
        let (body, raw_body, next) = if bodyless_status || reply_to_head {
            (Vec::new(), Vec::new(), body_at)
        } else if find_header("transfer-encoding")
            .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
        {
            let mut cursor = body_at;
            let mut decoded = Vec::new();
            loop {
                let Some(line_end) = find(bytes, b"\r\n", cursor) else {
                    return (messages, at);
                };
                let size_text = String::from_utf8_lossy(&bytes[cursor..line_end]).into_owned();
                let size_text = size_text.split(';').next().unwrap_or_default().trim().to_owned();
                let size = usize::from_str_radix(&size_text, 16).unwrap_or(0);
                cursor = line_end + 2;
                if size == 0 {
                    // Trailer lines, then the blank line.
                    loop {
                        let Some(trailer_end) = find(bytes, b"\r\n", cursor) else {
                            return (messages, at);
                        };
                        let empty = trailer_end == cursor;
                        cursor = trailer_end + 2;
                        if empty {
                            break;
                        }
                    }
                    break;
                }
                if bytes.len() < cursor + size + 2 {
                    return (messages, at);
                }
                decoded.extend_from_slice(&bytes[cursor..cursor + size]);
                cursor += size + 2;
            }
            (decoded, bytes[body_at..cursor].to_vec(), cursor)
        } else if let Some(length) =
            find_header("content-length").and_then(|value| value.parse::<usize>().ok())
        {
            if bytes.len() < body_at + length {
                return (messages, at);
            }
            let body = bytes[body_at..body_at + length].to_vec();
            (body.clone(), body, body_at + length)
        } else if kind == Kind::Response {
            // Close-delimited: the rest of the transcript.
            (bytes[body_at..].to_vec(), bytes[body_at..].to_vec(), bytes.len())
        } else {
            (Vec::new(), Vec::new(), body_at)
        };
        messages.push(Wire { start_line, headers, body, raw_body });
        at = next;
        if at >= bytes.len() {
            return (messages, at);
        }
    }
}

fn parse_wire(bytes: &[u8], kind: Kind) -> (Vec<Wire>, usize) {
    parse_wire_for(bytes, kind, &[])
}

fn calls_of(leg: &LegTranscript, call: RelayCall) -> usize {
    leg.calls.iter().filter(|seen| **seen == call).count()
}

/// The position of the first `call` in `leg`'s port-call order.
fn call_index(leg: &LegTranscript, call: RelayCall) -> Option<usize> {
    leg.calls.iter().position(|seen| *seen == call)
}

/// A public client script that sends `request` then closes cleanly.
fn client_sends(request: &[u8]) -> LegScript {
    LegScript::new().octets(request.to_vec()).close_notify()
}

/// An origin that answers with `response` and stays open.
fn origin_answers(response: &[u8]) -> LegScript {
    LegScript::new().octets(response.to_vec())
}

fn is_closed_by_gateway(client: &mut TcpStream) -> bool {
    use std::io::Read as _;
    client.set_nonblocking(true).expect("nonblocking client");
    let mut byte = [0_u8; 1];
    match client.read(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(error) => error.kind() == std::io::ErrorKind::ConnectionReset,
    }
}

fn contains(bytes: &[u8], needle: &str) -> bool {
    find(bytes, needle.as_bytes(), 0).is_some()
}

/// A complete chunked response with one data chunk.
fn chunked_response(data: &[u8]) -> Vec<u8> {
    concat(&[
        &response_head(200, "OK", &[("Transfer-Encoding", "chunked")]),
        format!("{:x}\r\n", data.len()).as_bytes(),
        data,
        b"\r\n0\r\n\r\n",
    ])
}

// ---------------------------------------------------------------------------
// S-PIG-87: keep-alive exchanges with exact bodies
// ---------------------------------------------------------------------------

/// S-PIG-87: two requests on one public connection, the first with a
/// `Content-Length` body of 0, 1, 65 535, 65 536 and 65 537 octets (the
/// relay pipe capacity is 65 536): each body arrives octet-exact at the origin
/// and at the client, nothing past the declared length is read, the second
/// request head is untouched, one upstream leg serves each exchange and none
/// is reused. Full duplex: an origin response head reaches the client before
/// the request body ends, and the exchange still ends reusable.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn keep_alive_exchanges_deliver_every_body_octet_exactly_and_leave_the_next_head_untouched() {
    let world = World::start(TWO_KEEP_ALIVE, Spec::default()).await;
    let mut expected_opens = 0;
    for size in [0_usize, 1, 65_535, 65_536, 65_537] {
        let body = pattern(size);
        let public =
            LegScript::new().octets(post("/first", &body)).octets(get("/second")).close_notify();
        let legs = world
            .run(
                public,
                vec![origin_answers(&ok_response(&body)), origin_answers(&ok_response(b"two"))],
            )
            .await;
        expected_opens += 2;

        assert_eq!(legs.upstream.len(), 2, "one upstream leg per exchange ({size} octets)");
        assert_eq!(world.opens(), expected_opens, "one registration per exchange");
        let first = legs.upstream_requests(0);
        assert_eq!(first.len(), 1, "exactly one request reaches the first origin");
        assert_eq!(first[0].method(), "POST");
        assert_eq!(first[0].target(), "/first");
        assert_eq!(first[0].body, body, "request body octet-exact at the origin ({size})");
        assert_eq!(first[0].header("connection"), Some("close"), "one request per upstream leg");
        let second = legs.upstream_requests(1);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].method(), "GET");
        assert_eq!(second[0].target(), "/second", "the next head is untouched");

        let responses = legs.public_messages();
        assert_eq!(responses.len(), 2, "both responses reach the client ({size})");
        assert_eq!(responses[0].body, body, "response body octet-exact at the client ({size})");
        assert_eq!(responses[1].body, b"two");
        assert!(legs.upstream.iter().all(|leg| leg.closed), "no upstream leg is reused");
    }

    // Full duplex: the response head overtakes the request body.
    let head = request_head("POST", "/duplex", &[("Content-Length", "4")]);
    let running = world.start_exchange(
        LegScript::new().octets(head).quiet(Duration::from_secs(10)).octets("abcd").close_notify(),
        vec![
            LegScript::new()
                .octets(response_head(200, "OK", &[("Content-Length", "2")]))
                .quiet(Duration::from_secs(20))
                .octets("ok"),
        ],
    );
    wait_for("the response head to reach the client before the request body", || {
        let legs = world.legs(&running);
        legs.public.as_ref().is_some_and(|leg| contains(&leg.written, "HTTP/1.1 200")).then_some(())
    })
    .await;
    let early = world.legs(&running);
    assert!(
        !contains(&early.upstream[0].written, "abcd"),
        "the request body has not been sent yet when the response head is already out"
    );
    world.advance(Duration::from_secs(20), Duration::from_secs(1)).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.upstream_requests(0)[0].body, b"abcd");
    let responses = legs.public_messages();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].body, b"ok");
    assert_eq!(world.opens(), expected_opens + 1, "still one upstream leg for the exchange");
}

// ---------------------------------------------------------------------------
// S-PIG-88: chunk by chunk
// ---------------------------------------------------------------------------

/// S-PIG-88: a chunked request and a chunked response relay chunk by chunk:
/// the gateway writes each size line, one exact chunk is spliced, extensions
/// and trailers are not forwarded, the gateway writes the last chunk, and a
/// size line split across two records is read whole. An open chunked request
/// body is unfinished at an early response: the connection is not reused.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn chunked_bodies_relay_chunk_by_chunk_without_extensions_or_trailers() {
    let world = World::start(CHUNKED, Spec::default()).await;
    let head = request_head("POST", "/chunks", &[("Transfer-Encoding", "chunked")]);
    let public = LegScript::new()
        .octets(head.clone())
        .octets("5;na")
        .octets("me=value\r\nhello\r\n3\r\nabc\r\n0\r\nX-Trailer: t\r\n\r\n")
        .close_notify();
    let origin = origin_answers(&concat(&[
        &response_head(200, "OK", &[("Transfer-Encoding", "chunked")]),
        b"4;e=1\r\nwxyz\r\n0\r\nX-Resp-Trailer: v\r\n\r\n",
    ]));
    let legs = world.run(public, vec![origin]).await;

    let requests = legs.upstream_requests(0);
    assert_eq!(requests.len(), 1);
    assert!(requests[0].header("transfer-encoding").is_some_and(|v| v.contains("chunked")));
    assert_eq!(requests[0].body, b"helloabc", "request chunks relayed exactly");
    assert_eq!(
        requests[0].raw_body, b"5\r\nhello\r\n3\r\nabc\r\n0\r\n\r\n",
        "the gateway writes each size line and the last chunk; no extension, no trailer"
    );
    assert!(!contains(&legs.upstream[0].written, "name=value"));
    assert!(!contains(&legs.upstream[0].written, "X-Trailer"));
    let (_, consumed) = parse_wire(&legs.upstream[0].written, Kind::Request);
    assert_eq!(consumed, legs.upstream[0].written.len(), "nothing follows the last chunk");

    let responses = legs.public_messages();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].body, b"wxyz", "response chunks relayed exactly");
    assert_eq!(responses[0].raw_body, b"4\r\nwxyz\r\n0\r\n\r\n");
    assert!(!contains(&legs.public().written, "X-Resp-Trailer"));
    assert!(!contains(&legs.public().written, ";e=1"));

    // An early response while the chunked request body is still open.
    let running = world.start_exchange(
        LegScript::new()
            .octets(head)
            .octets("5\r\nhello\r\n")
            .quiet(Duration::from_secs(3600))
            .octets(get("/never")),
        vec![origin_answers(&ok_response(b"early"))],
    );
    wait_for("the early response to reach the client", || {
        let legs = world.legs(&running);
        legs.public.as_ref().is_some_and(|leg| contains(&leg.written, "early")).then_some(())
    })
    .await;
    world.advance(Duration::from_secs(31), Duration::from_secs(1)).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![200]);
    assert_eq!(legs.upstream.len(), 1, "the unfinished chunked body is never reused");
    assert!(legs.public().close_notify_sent, "the early response ends in a lingering close");
}

// ---------------------------------------------------------------------------
// S-PIG-89: responses without a body
// ---------------------------------------------------------------------------

/// S-PIG-89: responses to `HEAD`, and `204`, `205`, `304` responses (some
/// declaring a length), relay nothing for a body and leave the connection
/// reusable; an interim `1xx` before one of them keeps the same rules.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn bodyless_responses_relay_no_body_and_leave_the_connection_reusable() {
    let world = World::start(BODYLESS, Spec::default()).await;
    let early_hints = response_head(103, "Early Hints", &[("Link", "</s.css>; rel=preload")]);
    let cases: Vec<(&str, &str, Vec<u8>)> = vec![
        (
            "HEAD declaring a length",
            "HEAD",
            response_head(200, "OK", &[("Content-Length", "1000")]),
        ),
        ("204", "GET", response_head(204, "No Content", &[])),
        (
            "205 declaring zero",
            "GET",
            response_head(205, "Reset Content", &[("Content-Length", "0")]),
        ),
        (
            "304 declaring a length",
            "GET",
            response_head(304, "Not Modified", &[("Content-Length", "1234")]),
        ),
        (
            "204 after an interim head",
            "GET",
            concat(&[&early_hints, &response_head(204, "No Content", &[])]),
        ),
    ];
    for (name, method, answer) in cases {
        let public = LegScript::new()
            .octets(request_head(method, "/a", &[]))
            .octets(get("/b"))
            .close_notify();
        let legs = world
            .run(public, vec![origin_answers(&answer), origin_answers(&ok_response(b"second"))])
            .await;
        let (responses, consumed) =
            parse_wire_for(&legs.public().written, Kind::Response, &[method == "HEAD", false]);
        assert_eq!(consumed, legs.public().written.len(), "{name}: no stray octets");
        let statuses: Vec<u16> = responses.iter().map(Wire::status).collect();
        let first_status = if name.starts_with("HEAD") { 200 } else { answer_status(&answer) };
        assert_eq!(statuses, vec![first_status, 200], "{name}: both exchanges answered");
        assert!(responses[0].body.is_empty(), "{name}: no body relayed");
        assert_eq!(responses[1].body, b"second", "{name}: the connection was reused");
        assert!(!contains(&legs.public().written, "HTTP/1.1 103"), "{name}: interim head dropped");
        assert_eq!(calls_of(&legs.upstream[0], RelayCall::RelayExact), 0, "{name}");
        assert_eq!(calls_of(&legs.upstream[0], RelayCall::RelayToEnd), 0, "{name}");
        assert_eq!(legs.upstream.len(), 2, "{name}: one upstream leg per exchange");
    }
}

/// The status of the last head in `answer`.
fn answer_status(answer: &[u8]) -> u16 {
    parse_wire(answer, Kind::Response).0.last().map(Wire::status).expect("a response head")
}

// ---------------------------------------------------------------------------
// S-PIG-83: interim heads and Expect
// ---------------------------------------------------------------------------

/// S-PIG-83: an origin `100` reaches the client as a body-less rewritten head,
/// every other interim `1xx` except `101` is read and dropped, `101` is a 502,
/// and `Expect` is forwarded unchanged: the gateway decides nothing, and the
/// origin's `417` is relayed as the origin's own response.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn interim_heads_are_forwarded_or_dropped_and_expect_reaches_the_origin_unchanged() {
    let world = World::start(INTERIM, Spec::default()).await;

    let origin = origin_answers(&concat(&[
        &response_head(103, "Early Hints", &[("Link", "</s.css>; rel=preload")]),
        &response_head(100, "Continue", &[]),
        &ok_response(b"ok"),
    ]));
    let legs = world.run(client_sends(&get("/a")), vec![origin]).await;
    assert_eq!(legs.public_statuses(), vec![100, 200], "the 100 is forwarded, the 103 is not");
    assert!(legs.public_messages()[0].body.is_empty());
    assert!(!contains(&legs.public().written, "103"));
    assert_eq!(legs.public_messages()[1].body, b"ok");

    let switching =
        response_head(101, "Switching Protocols", &[("Upgrade", "foo"), ("Connection", "Upgrade")]);
    let legs = world.run(client_sends(&get("/b")), vec![origin_answers(&switching)]).await;
    assert_eq!(legs.public_statuses(), vec![502], "101 is a 502 before the public head");
    assert!(!contains(&legs.public().written, "101"));

    let expect_request =
        request_head("POST", "/c", &[("Expect", "100-continue"), ("Content-Length", "3")]);
    let origin =
        origin_answers(&concat(&[&response_head(100, "Continue", &[]), &ok_response(b"fine")]));
    let legs = world
        .run(LegScript::new().octets(expect_request).octets("abc").close_notify(), vec![origin])
        .await;
    assert_eq!(
        legs.upstream_requests(0)[0].header("expect"),
        Some("100-continue"),
        "Expect is forwarded unchanged"
    );
    assert_eq!(legs.upstream_requests(0)[0].body, b"abc");
    assert_eq!(legs.public_statuses(), vec![100, 200]);

    let odd_expect =
        request_head("POST", "/d", &[("Expect", "x-unknown"), ("Content-Length", "0")]);
    let legs = world
        .run(client_sends(&odd_expect), vec![origin_answers(&response(417, b"origin says no"))])
        .await;
    assert_eq!(legs.upstream_requests(0)[0].header("expect"), Some("x-unknown"));
    assert_eq!(legs.public_statuses(), vec![417], "the gateway decides nothing about Expect");
    assert_eq!(legs.public_messages()[0].body, b"origin says no");
}

// ---------------------------------------------------------------------------
// S-PIG-90: control records never reach the other leg
// ---------------------------------------------------------------------------

/// S-PIG-90: a peer KeyUpdate (requesting an answer or not), a
/// NewSessionTicket and a close_notify met inside a head, inside a chunk-size
/// line, at a message boundary and inside a body leave the relayed octets
/// exact on both legs; no control event is visible on the other leg and the
/// exchange ends cleanly.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-sim-adapter"]
async fn control_records_met_anywhere_in_a_message_leave_the_relayed_octets_exact() {
    let world = World::start(HARMLESS_CONTROL, Spec::default()).await;

    // Inside a head, at a boundary and inside a body, on both legs.
    let public = LegScript::new()
        .octets("POST /v1/c HTTP/1.1\r\nHost: api.exam")
        .key_update(true)
        .octets("ple.com\r\nContent-Length: 10\r\n\r\n01234")
        .new_session_ticket()
        .octets("56789")
        .key_update(false)
        .close_notify();
    let origin = LegScript::new()
        .octets("HTTP/1.1 200 OK\r\nContent-Le")
        .key_update(false)
        .octets("ngth: 8\r\n\r\nabcd")
        .new_session_ticket()
        .octets("efgh")
        .key_update(true);
    let legs = world.run(public, vec![origin]).await;
    let requests = legs.upstream_requests(0);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, b"0123456789", "request octets exact across the events");
    assert_eq!(legs.public_statuses(), vec![200]);
    assert_eq!(legs.public_messages()[0].body, b"abcdefgh", "response octets exact");
    assert_eq!(legs.public_messages().len(), 1);

    // Inside a chunk-size line, both directions.
    let public = LegScript::new()
        .octets(request_head("POST", "/v1/d", &[("Transfer-Encoding", "chunked")]))
        .octets("5\r")
        .key_update(true)
        .octets("\nhello\r\n")
        .new_session_ticket()
        .octets("0\r\n\r\n")
        .close_notify();
    let origin = LegScript::new()
        .octets(response_head(200, "OK", &[("Transfer-Encoding", "chunked")]))
        .octets("3")
        .key_update(false)
        .octets("\r\nxyz\r\n0\r\n\r\n");
    let legs = world.run(public, vec![origin]).await;
    assert_eq!(legs.upstream_requests(0)[0].body, b"hello");
    assert_eq!(legs.public_messages()[0].body, b"xyz");
    assert_eq!(legs.public_statuses(), vec![200]);

    // A control record is not application octets, so this check only catches
    // control payload leaked onto the other leg as application data: every
    // octet a leg was given must be a whole application message with nothing
    // stray after it. That no control event reaches the other leg needs a
    // control-event field on the Sim leg transcript, which is not pinned yet.
    for leg in world.relay.leg_transcripts() {
        let kind = match leg.role {
            LegRole::Public => Kind::Response,
            LegRole::Upstream => Kind::Request,
        };
        let (_, consumed) = parse_wire(&leg.written, kind);
        assert_eq!(
            consumed,
            leg.written.len(),
            "leg {:?}: only application messages were written",
            leg.id
        );
    }
}

/// S-PIG-90: an alert or an unexpected record closes both legs. On the public
/// leg mid-request the origin gets no octet beyond the failure and the client
/// no response; on the upstream leg before the public response head the client
/// gets exactly one 502; after the head both legs close without a further
/// response and the body never ends cleanly.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-sim-adapter"]
async fn an_alert_or_an_unexpected_record_closes_both_legs() {
    let world = World::start(FATAL_CONTROL, Spec::default()).await;
    let head = request_head("POST", "/v1/e", &[("Content-Length", "10")]);

    for (name, is_alert) in [("alert", true), ("unexpected record", false)] {
        let fatal = |script: LegScript| {
            if is_alert { script.alert(40) } else { script.unexpected_record(24) }
        };
        // Public leg, mid-request body.
        let public = fatal(LegScript::new().octets(head.clone()).octets("0123"));
        let legs = world.run(public, vec![LegScript::new()]).await;
        assert!(legs.public().written.is_empty(), "{name}: no response after a public failure");
        assert!(
            !contains(&legs.upstream[0].written, "456789"),
            "{name}: nothing is relayed past the failure"
        );
        assert!(legs.upstream[0].closed && legs.public().closed, "{name}: both legs closed");

        // Upstream leg, before the public response head.
        let legs = world.run(client_sends(&get("/v1/f")), vec![fatal(LegScript::new())]).await;
        assert_eq!(legs.public_statuses(), vec![502], "{name}: exactly one 502 before the head");
        assert!(legs.upstream[0].closed, "{name}");

        // Upstream leg, after the public response head.
        let origin = fatal(
            LegScript::new()
                .octets(response_head(200, "OK", &[("Content-Length", "100")]))
                .octets(pattern(10)),
        );
        let legs = world.run(client_sends(&get("/v1/g")), vec![origin]).await;
        assert_eq!(
            legs.public_statuses(),
            Vec::<u16>::new(),
            "{name}: the incomplete body is not a message"
        );
        assert!(contains(&legs.public().written, "HTTP/1.1 200"), "{name}: the head went out");
        assert!(!contains(&legs.public().written, "HTTP/1.1 502"), "{name}: no further response");
        assert!(!legs.public().close_notify_sent, "{name}: the body never ends cleanly");
        assert!(legs.upstream[0].closed && legs.public().closed, "{name}: both legs closed");
    }
}

/// S-PIG-90: a KeyUpdate whose re-arm is refused (`RustlsRefused`, `RearmRx`,
/// `RearmTx`, `Io`) on either leg closes both legs and relays nothing past the
/// failure: on the upstream leg before the public response head it is exactly
/// one 502, after the head a close with no further response, and on the public
/// leg mid-request a close with no response.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-sim-adapter"]
async fn a_key_update_whose_answer_fails_closes_both_legs_and_relays_nothing_past_it() {
    let world = World::start(FAILED_ANSWER, Spec::default()).await;
    let failures = [
        KeyUpdateError::RustlsRefused,
        KeyUpdateError::RearmRx { errno: 22 },
        KeyUpdateError::RearmTx { errno: 22 },
        KeyUpdateError::Io { kind: std::io::ErrorKind::BrokenPipe },
    ];
    for failure in failures {
        // Upstream leg, before the public head.
        let origin = LegScript::new().key_update_whose_answer_fails(true, failure);
        let legs = world.run(client_sends(&get("/v1/a")), vec![origin]).await;
        assert_eq!(legs.public_statuses(), vec![502], "{failure:?}: exactly one 502");
        assert!(legs.upstream[0].closed, "{failure:?}");

        // Upstream leg, after the public head.
        let origin = LegScript::new()
            .octets(response_head(200, "OK", &[("Content-Length", "100")]))
            .octets(pattern(10))
            .key_update_whose_answer_fails(true, failure)
            .octets(pattern(50));
        let legs = world.run(client_sends(&get("/v1/b")), vec![origin]).await;
        assert!(contains(&legs.public().written, "HTTP/1.1 200"), "{failure:?}");
        assert!(
            !contains(&legs.public().written, "HTTP/1.1 502"),
            "{failure:?}: no further response"
        );
        let body_octets = legs.public().written.len()
            - find(&legs.public().written, b"\r\n\r\n", 0).map_or(0, |at| at + 4);
        assert!(
            body_octets <= 10,
            "{failure:?}: nothing is relayed past the failure ({body_octets})"
        );
        assert!(!legs.public().close_notify_sent, "{failure:?}: never a clean end");
        assert!(legs.upstream[0].closed && legs.public().closed, "{failure:?}");

        // Public leg, mid-request.
        let public = LegScript::new()
            .octets(request_head("POST", "/v1/c", &[("Content-Length", "10")]))
            .octets("0123")
            .key_update_whose_answer_fails(true, failure)
            .octets("456789");
        let legs = world.run(public, vec![LegScript::new()]).await;
        assert!(
            legs.public().written.is_empty(),
            "{failure:?}: no response to a failed public leg"
        );
        assert!(!contains(&legs.upstream[0].written, "456789"), "{failure:?}");
        assert!(legs.upstream[0].closed && legs.public().closed, "{failure:?}");
    }
}

// ---------------------------------------------------------------------------
// S-PIG-91: close-delimited responses
// ---------------------------------------------------------------------------

/// S-PIG-91: a close-delimited response (no length, no coding) is forwarded
/// with `Connection: close`, ends cleanly on the origin's close_notify, is not
/// reused (the second request on the connection is never parsed), and closes
/// both legs when it exceeds `max_response_body_bytes`.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn a_close_delimited_response_ends_cleanly_on_close_notify_and_is_never_reused() {
    let world = World::start(CLOSE_DELIMITED, Spec::default()).await;
    let body = pattern(5000);
    let origin = LegScript::new()
        .octets(response_head(200, "OK", &[("Content-Type", "application/octet-stream")]))
        .octets(body.clone())
        .close_notify();
    let public = LegScript::new().octets(get("/v1/a")).octets(get("/v1/never")).close_notify();
    let legs = world.run(public, vec![origin]).await;
    let responses = legs.public_messages();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].header("connection"), Some("close"));
    assert_eq!(responses[0].body, body, "the body is every octet up to the origin's close_notify");
    assert!(legs.public().close_notify_sent, "a clean end on the origin's close_notify");
    assert_eq!(legs.upstream.len(), 1, "the second request was never parsed");
    assert_eq!(world.opens(), 1);

    let ceiling = usize::try_from(world.limits().max_response_body_bytes().get()).expect("ceiling");
    let origin = LegScript::new()
        .octets(response_head(200, "OK", &[]))
        .octets(pattern(ceiling + 1))
        .close_notify();
    let legs = world.run(client_sends(&get("/v1/over")), vec![origin]).await;
    let written = &legs.public().written;
    let body_octets = written.len() - find(written, b"\r\n\r\n", 0).map_or(0, |at| at + 4);
    assert!(body_octets <= ceiling, "no octet beyond the ceiling is relayed ({body_octets})");
    assert!(legs.upstream[0].closed && legs.public().closed, "both legs closed");
}

// ---------------------------------------------------------------------------
// S-PIG-92: upstream failure projection
// ---------------------------------------------------------------------------

/// S-PIG-92: an origin close_notify or reset before the response head is a 502
/// exactly once; a close or reset mid-body, and a close_notify before the
/// declared end (a `Content-Length` body and a chunked body), close both legs
/// after the head without a further response and never end cleanly.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn an_upstream_failure_is_one_502_before_the_head_and_an_unclean_close_after_it() {
    let world = World::start(UPSTREAM_FAILURE, Spec::default()).await;

    for (name, origin) in
        [("close_notify", LegScript::new().close_notify()), ("reset", LegScript::new().reset())]
    {
        let legs = world.run(client_sends(&get("/v1/a")), vec![origin]).await;
        assert_eq!(legs.public_statuses(), vec![502], "{name} before the head: exactly one 502");
        assert!(legs.upstream[0].closed, "{name}");
    }

    let length_head = response_head(200, "OK", &[("Content-Length", "100")]);
    let chunked_head = response_head(200, "OK", &[("Transfer-Encoding", "chunked")]);
    let cases = [
        (
            "reset mid length body",
            LegScript::new().octets(length_head.clone()).octets(pattern(40)).reset(),
        ),
        (
            "close_notify before the declared end",
            LegScript::new().octets(length_head).octets(pattern(40)).close_notify(),
        ),
        (
            "close_notify mid chunk",
            LegScript::new()
                .octets(chunked_head.clone())
                .octets("64\r\n")
                .octets(pattern(40))
                .close_notify(),
        ),
        (
            "close_notify before the last chunk",
            LegScript::new().octets(chunked_head).octets("4\r\nabcd\r\n").close_notify(),
        ),
    ];
    for (name, origin) in cases {
        let legs = world.run(client_sends(&get("/v1/b")), vec![origin]).await;
        let written = &legs.public().written;
        assert!(contains(written, "HTTP/1.1 200"), "{name}: the head went out");
        assert!(!contains(written, "HTTP/1.1 502"), "{name}: no further response after the head");
        assert!(!legs.public().close_notify_sent, "{name}: never a clean end");
        let (complete, _) = parse_wire(written, Kind::Response);
        assert!(complete.is_empty(), "{name}: the message never completes");
        assert!(legs.upstream[0].closed && legs.public().closed, "{name}: both legs closed");
    }
}

// ---------------------------------------------------------------------------
// S-PIG-93: early answer and lingering close
// ---------------------------------------------------------------------------

struct Linger {
    name: &'static str,
    public: LegScript,
    upstreams: Vec<LegScript>,
    arm_refusal: Option<KtlsArmError>,
    status: u16,
}

/// A public client that sends `head`, keeps sending body octets and then
/// closes cleanly.
fn client_keeps_sending(head: Vec<u8>) -> LegScript {
    LegScript::new().octets(head).octets(vec![b'x'; 70_000]).close_notify()
}

fn assert_lingering_close(legs: &Legs, name: &str) {
    let public = legs.public();
    let write = call_index(public, RelayCall::WriteAll);
    let close = call_index(public, RelayCall::CloseWrite);
    let discard = call_index(public, RelayCall::DiscardRemaining);
    assert!(
        matches!((write, close, discard), (Some(w), Some(c), Some(d)) if w < c && c < d),
        "{name}: the final response, then close_notify, then the bounded discard: {:?}",
        public.calls
    );
    assert!(public.close_notify_sent, "{name}: the client was half-closed");
    assert!(public.closed, "{name}: the socket is closed on every outcome");
}

/// S-PIG-93: an origin answering before the request body is read (including
/// 401 and 413) and each gateway rejection sent before a body is read (413 on
/// declared length, 400, 404, 421, 431, 502, 503, 505) deliver the complete
/// final response, half-close, discard the remaining request octets until the
/// peer's close_notify, and close; a 408 is not lingered.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER relay-lingering-close"]
async fn an_early_or_gateway_authored_response_ends_with_a_half_close_and_a_bounded_discard() {
    let world = World::start(LINGERING, Spec { route_path: "/api", ..Spec::default() }).await;
    let limits = world.limits();
    let body_ceiling = limits.max_request_body_bytes().get();
    let big = |target: &str| request_head("POST", target, &[("Content-Length", "100000")]);

    let cases = vec![
        Linger {
            name: "origin answers 401 early",
            public: client_keeps_sending(big("/api/x")),
            upstreams: vec![origin_answers(&response(401, b"no"))],
            arm_refusal: None,
            status: 401,
        },
        Linger {
            name: "origin answers 413 early",
            public: client_keeps_sending(big("/api/x")),
            upstreams: vec![origin_answers(&response(413, b"too large"))],
            arm_refusal: None,
            status: 413,
        },
        Linger {
            name: "declared length above the ceiling",
            public: client_keeps_sending(request_head(
                "POST",
                "/api/x",
                &[("Content-Length", &(body_ceiling + 1).to_string())],
            )),
            upstreams: vec![],
            arm_refusal: None,
            status: 413,
        },
        Linger {
            name: "ambiguous framing",
            public: client_keeps_sending(request_head(
                "POST",
                "/api/x",
                &[("Content-Length", "100000"), ("Transfer-Encoding", "chunked")],
            )),
            upstreams: vec![],
            arm_refusal: None,
            status: 400,
        },
        Linger {
            name: "route miss",
            public: client_keeps_sending(big("/other")),
            upstreams: vec![],
            arm_refusal: None,
            status: 404,
        },
        Linger {
            name: "Host differs from SNI",
            public: client_keeps_sending(request_head(
                "POST",
                "/api/x",
                &[("Host", "other.example.com"), ("Content-Length", "100000")],
            )),
            upstreams: vec![],
            arm_refusal: None,
            status: 421,
        },
        Linger {
            name: "request head above the ceiling",
            public: client_keeps_sending(request_head(
                "POST",
                "/api/x",
                &[
                    ("Content-Length", "100000"),
                    ("X-Pad", &"a".repeat(limits.max_request_head_bytes().get())),
                ],
            )),
            upstreams: vec![],
            arm_refusal: None,
            status: 431,
        },
        Linger {
            name: "upstream arming refused",
            public: client_keeps_sending(big("/api/x")),
            upstreams: vec![],
            arm_refusal: Some(KtlsArmError::UlpInstall { errno: 2 }),
            status: 502,
        },
        Linger {
            name: "HTTP/1.0 request",
            public: client_keeps_sending(
                b"POST /api/x HTTP/1.0\r\nHost: api.example.com\r\nContent-Length: 100000\r\n\r\n"
                    .to_vec(),
            ),
            upstreams: vec![],
            arm_refusal: None,
            status: 505,
        },
    ];
    for case in cases {
        if let Some(kind) = case.arm_refusal {
            world.relay.fail_next_upstream_arm(kind);
        }
        let legs = world.run(case.public, case.upstreams).await;
        let statuses = legs.public_statuses();
        assert_eq!(statuses.first(), Some(&case.status), "{}: status", case.name);
        assert_eq!(statuses.len(), 1, "{}: exactly one final response", case.name);
        assert_lingering_close(&legs, case.name);
    }

    // NoBackend is a 503, also lingered.
    let none =
        World::start(LINGERING, Spec { programmed: false, route_path: "/api", ordinal: 1 }).await;
    let legs = none.run(client_keeps_sending(big("/api/x")), vec![]).await;
    assert_eq!(legs.public_statuses(), vec![503], "NoBackend");
    assert_lingering_close(&legs, "NoBackend");

    // 408 is not lingered: the peer stalled, nothing is in flight to destroy.
    let running = world.start_exchange(
        LegScript::new().octets("POST /api/x HTTP/1.1\r\nHo").quiet(Duration::from_secs(3600)),
        vec![],
    );
    world.accepted(&running).await;
    world
        .advance(limits.request_head_deadline() + Duration::from_secs(1), Duration::from_secs(1))
        .await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![408]);
    assert_eq!(calls_of(legs.public(), RelayCall::DiscardRemaining), 0, "a 408 is not lingered");
}

// ---------------------------------------------------------------------------
// S-PIG-94: body ceilings
// ---------------------------------------------------------------------------

/// S-PIG-94: a declared request length above the ceiling is a 413 before any
/// upstream octet and the ceiling itself passes; a chunked total above the
/// ceiling is a 413 if no public response octet was sent and otherwise closes
/// both legs; a response body that outgrows its ceiling closes both legs
/// after the head; one octet under each ceiling passes.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn request_and_response_bodies_are_bounded_by_their_ceilings() {
    let world = World::start(CEILINGS, Spec::default()).await;
    let request_ceiling =
        usize::try_from(world.limits().max_request_body_bytes().get()).expect("ceiling");
    let response_ceiling =
        usize::try_from(world.limits().max_response_body_bytes().get()).expect("ceiling");

    // Declared length one above: 413 before any upstream octet.
    let head =
        request_head("POST", "/a", &[("Content-Length", &(request_ceiling + 1).to_string())]);
    let legs = world.run(client_sends(&head), vec![]).await;
    assert_eq!(legs.public_statuses(), vec![413]);
    assert!(legs.upstream.is_empty(), "no upstream leg, so no upstream octet");
    assert_eq!(world.opens(), 0);

    // Declared length at the ceiling passes; the origin answers after the body.
    let head = request_head("POST", "/b", &[("Content-Length", &request_ceiling.to_string())]);
    let public = LegScript::new().octets(head).octets(vec![b'x'; request_ceiling]).close_notify();
    let origin = LegScript::new().quiet(Duration::from_secs(1)).octets(ok_response(b"done"));
    let running = world.start_exchange(public, vec![origin]);
    wait_for("the whole body to reach the origin", || {
        let legs = world.legs(&running);
        legs.upstream.first().is_some_and(|leg| leg.written.len() > request_ceiling).then_some(())
    })
    .await;
    world.advance(Duration::from_secs(1), Duration::from_secs(1)).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![200]);
    assert_eq!(legs.upstream_requests(0)[0].body.len(), request_ceiling);

    // Chunked total one under the ceiling passes.
    let chunk =
        |len: usize| concat(&[format!("{len:x}\r\n").as_bytes(), &vec![b'x'; len], b"\r\n"]);
    let head = request_head("POST", "/c", &[("Transfer-Encoding", "chunked")]);
    let public = LegScript::new()
        .octets(head.clone())
        .octets(chunk(request_ceiling - 1))
        .octets("0\r\n\r\n")
        .close_notify();
    let origin = LegScript::new().quiet(Duration::from_secs(1)).octets(ok_response(b"done"));
    let running = world.start_exchange(public, vec![origin]);
    wait_for("the chunked body to reach the origin", || {
        let legs = world.legs(&running);
        legs.upstream.first().is_some_and(|leg| leg.written.len() >= request_ceiling).then_some(())
    })
    .await;
    world.advance(Duration::from_secs(1), Duration::from_secs(1)).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![200]);

    // Chunked total above the ceiling, no public octet sent yet: 413.
    let public = LegScript::new()
        .octets(head.clone())
        .octets(chunk(request_ceiling))
        .octets(chunk(1))
        .close_notify();
    let legs = world.run(public, vec![LegScript::new()]).await;
    assert_eq!(legs.public_statuses(), vec![413], "chunked overflow before any public octet");
    assert!(legs.upstream[0].closed);

    // Chunked total above the ceiling after the response head went out: both
    // legs close, no 413.
    let public = LegScript::new()
        .octets(head)
        .octets(chunk(request_ceiling))
        .octets(chunk(1))
        .close_notify();
    let origin = LegScript::new()
        .octets(response_head(200, "OK", &[("Transfer-Encoding", "chunked")]))
        .octets("1\r\nz\r\n");
    let legs = world.run(public, vec![origin]).await;
    assert!(contains(&legs.public().written, "HTTP/1.1 200"));
    assert!(!contains(&legs.public().written, "HTTP/1.1 413"), "a response octet was already sent");
    assert!(legs.upstream[0].closed && legs.public().closed);

    // A chunked response one under its ceiling passes.
    let body = vec![b'y'; response_ceiling - 1];
    let origin = LegScript::new()
        .octets(response_head(200, "OK", &[("Transfer-Encoding", "chunked")]))
        .octets(chunk(response_ceiling - 1))
        .octets("0\r\n\r\n");
    let legs = world.run(client_sends(&get("/d")), vec![origin]).await;
    assert_eq!(legs.public_messages()[0].body, body);

    // A chunked response that outgrows its ceiling after the head: both legs
    // close, never a clean end.
    let origin = LegScript::new()
        .octets(response_head(200, "OK", &[("Transfer-Encoding", "chunked")]))
        .octets(chunk(response_ceiling))
        .octets(chunk(1));
    let legs = world.run(client_sends(&get("/e")), vec![origin]).await;
    assert!(contains(&legs.public().written, "HTTP/1.1 200"));
    assert!(!contains(&legs.public().written, "HTTP/1.1 502"));
    assert!(
        !legs.public().written.ends_with(b"0\r\n\r\n"),
        "the overflowing body never ends cleanly"
    );
    assert!(legs.upstream[0].closed && legs.public().closed);
}

// ---------------------------------------------------------------------------
// S-PIG-95: relay pipe failure
// ---------------------------------------------------------------------------

/// S-PIG-95: a pipe that cannot be created, or that reads back below the
/// floor, is a 502 before the response head and moves no octet; after the
/// response head it closes both legs.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-sim-adapter"]
async fn a_relay_pipe_failure_is_a_502_before_the_head_and_a_close_after_it() {
    let world = World::start(PIPE_FAILURE, Spec::default()).await;
    let failures = [
        RelayPipeError::Create { kind: std::io::ErrorKind::Other },
        RelayPipeError::CapacityBelowFloor { got: 4096 },
    ];
    for failure in failures {
        // The request body's relay fails before any response octet.
        let public = LegScript::new()
            .octets(post("/a", &pattern(1000)))
            .close_notify()
            .with_pipe_failure(failure.clone());
        let legs = world.run(public, vec![LegScript::new()]).await;
        assert_eq!(legs.public_statuses(), vec![502], "{failure:?}: 502 before the head");
        assert!(legs.upstream[0].written.len() < 1000, "{failure:?}: no body octet moved");
        assert!(legs.upstream[0].closed, "{failure:?}");

        // The response body's relay fails after the response head went out.
        let origin = LegScript::new()
            .octets(response_head(200, "OK", &[("Content-Length", "1000")]))
            .octets(pattern(1000))
            .with_pipe_failure(failure.clone());
        let legs = world.run(client_sends(&get("/b")), vec![origin]).await;
        assert!(contains(&legs.public().written, "HTTP/1.1 200"), "{failure:?}: the head went out");
        assert!(
            !contains(&legs.public().written, "HTTP/1.1 502"),
            "{failure:?}: no second response"
        );
        assert!(legs.public().written.len() < 1000, "{failure:?}: no body octet moved");
        assert!(legs.upstream[0].closed && legs.public().closed, "{failure:?}");
    }
}

// ---------------------------------------------------------------------------
// S-PIG-96: keep-alive and deadlines on the injected clock
// ---------------------------------------------------------------------------

/// S-PIG-96: idle expiry before the first octet closes silently (not a 408);
/// no complete head in time is a 408 (first request counted from handshake
/// completion, a later request from its first octet); the hundredth response
/// carries `Connection: close` and a 101st request is never parsed.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn idle_and_head_deadlines_follow_the_injected_clock_and_the_hundredth_response_closes() {
    let world = World::start(DEADLINES, Spec::default()).await;
    let limits = world.limits();
    let head_deadline = limits.request_head_deadline();
    let idle = limits.keep_alive_idle_deadline();
    let second = Duration::from_secs(1);

    // Idle expiry: one request answered, then silence for the idle deadline.
    let running = world.start_exchange(
        LegScript::new().octets(get("/a")).quiet(Duration::from_secs(3600)),
        vec![origin_answers(&ok_response(b"one"))],
    );
    wait_for("the first response", || {
        let legs = world.legs(&running);
        legs.public.as_ref().is_some_and(|leg| contains(&leg.written, "one")).then_some(())
    })
    .await;
    world.advance(idle - second * 2, second).await;
    assert!(!world.legs(&running).public().closed, "still open before the idle deadline");
    world.advance(second * 3, second).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![200], "idle expiry is silent, not a 408");

    // First request: no complete head within the head deadline from the
    // handshake: a 408 exactly then, and not before.
    let running = world.start_exchange(
        LegScript::new()
            .octets("GET /b HTTP/1.1\r\nHost: api.example.com\r\n")
            .quiet(Duration::from_secs(3600)),
        vec![],
    );
    world.accepted(&running).await;
    world.advance(head_deadline - second * 2, second).await;
    assert!(world.legs(&running).public().written.is_empty(), "no 408 before the head deadline");
    world.advance(second * 3, second).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![408], "408 at the head deadline");
    assert!(legs.upstream.is_empty());

    // Later request: counted from its first octet, not from the previous
    // response.
    let running = world.start_exchange(
        LegScript::new()
            .octets(get("/c"))
            .quiet(Duration::from_secs(20))
            .octets("GET /d HTTP/1.1\r\nHost: api.exam")
            .quiet(Duration::from_secs(3600)),
        vec![origin_answers(&ok_response(b"c"))],
    );
    wait_for("the first response", || {
        let legs = world.legs(&running);
        legs.public.as_ref().is_some_and(|leg| contains(&leg.written, "HTTP/1.1 200")).then_some(())
    })
    .await;
    world.advance(Duration::from_secs(20), second).await;
    world.advance(head_deadline - second * 2, second).await;
    assert!(
        !contains(&world.legs(&running).public().written, "HTTP/1.1 408"),
        "the second head still has time left"
    );
    world.advance(second * 3, second).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![200, 408]);

    // The hundredth response closes the connection; the 101st request is never
    // parsed.
    let allowed = usize::try_from(limits.max_requests_per_connection().get()).expect("request cap");
    let mut public = LegScript::new();
    let mut origins = Vec::new();
    for index in 0..=allowed {
        public = public.octets(get(&format!("/n{index}")));
        origins.push(origin_answers(&ok_response(format!("r{index}").as_bytes())));
    }
    let opens_before = world.opens();
    let legs = world.run(public.close_notify(), origins.into_iter().take(allowed).collect()).await;
    let responses = legs.public_messages();
    assert_eq!(responses.len(), allowed, "exactly {allowed} responses");
    assert_eq!(responses[allowed - 1].header("connection"), Some("close"));
    assert!(
        responses[..allowed - 1].iter().all(|r| r.header("connection") != Some("close")),
        "only the last response closes"
    );
    assert_eq!(world.opens() - opens_before, allowed as u64, "the 101st request opened nothing");
}

// ---------------------------------------------------------------------------
// S-PIG-97: single attempt
// ---------------------------------------------------------------------------

/// S-PIG-97: on every failure of S-PIG-92, S-PIG-95 and S-PIG-98 there is no
/// retry, no second backend and no replay: exactly one upstream open and one
/// cleanup per request, the registered intent is gone, and the upstream
/// request head carries `Connection: close`.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn every_failure_is_a_single_attempt_without_retry_or_second_backend() {
    let world = World::start(SINGLE_ATTEMPT, Spec::default()).await;

    struct Failure {
        name: &'static str,
        public: LegScript,
        origin: Option<LegScript>,
        arm_refusal: Option<KtlsArmError>,
    }
    let failures = vec![
        Failure {
            name: "origin closes before the head",
            public: client_sends(&get("/a")),
            origin: Some(LegScript::new().close_notify()),
            arm_refusal: None,
        },
        Failure {
            name: "origin resets mid body",
            public: client_sends(&get("/b")),
            origin: Some(
                LegScript::new()
                    .octets(response_head(200, "OK", &[("Content-Length", "100")]))
                    .octets(pattern(10))
                    .reset(),
            ),
            arm_refusal: None,
        },
        Failure {
            name: "relay pipe cannot be created",
            public: LegScript::new()
                .octets(post("/c", &pattern(100)))
                .close_notify()
                .with_pipe_failure(RelayPipeError::Create { kind: std::io::ErrorKind::Other }),
            origin: Some(LegScript::new()),
            arm_refusal: None,
        },
        Failure {
            name: "arming refused after equality",
            public: client_sends(&get("/d")),
            origin: None,
            arm_refusal: Some(KtlsArmError::CipherNotSupported),
        },
    ];
    for failure in failures {
        if let Some(kind) = failure.arm_refusal {
            world.relay.fail_next_upstream_arm(kind);
        }
        let opens_before = world.opens();
        let cleanups_before = world.cleanups();
        let legs = world.run(failure.public, failure.origin.into_iter().collect()).await;
        assert_eq!(
            world.opens() - opens_before,
            1,
            "{}: one open_upstream, no second dial",
            failure.name
        );
        assert_eq!(world.cleanups() - cleanups_before, 1, "{}: one cleanup", failure.name);
        assert!(legs.upstream.len() <= 1, "{}: at most one upstream leg", failure.name);
        assert_eq!(world.dataplane.residue(), (0, 0), "{}: no registration left", failure.name);
        for request in legs
            .upstream
            .first()
            .map(|leg| parse_wire(&leg.written, Kind::Request).0)
            .unwrap_or_default()
        {
            assert_eq!(request.header("connection"), Some("close"), "{}", failure.name);
        }
    }
    assert_eq!(
        world.mtls.request_peers().len() as u64,
        world.opens(),
        "one authenticate per request: no second attempt on another peer"
    );
}

// ---------------------------------------------------------------------------
// S-PIG-98: arming only after the equality held
// ---------------------------------------------------------------------------

/// S-PIG-98 (upstream): arming refused for each kind
/// (`UlpInstall`, `CryptoInfo`, `CipherNotSupported`, `KernelConnection`) after
/// the exact-peer equality held: no octet reaches the peer, cleanup runs once,
/// and the public request is a 502; an unequal peer arms nothing either.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER exact-peer-client-mtls"]
async fn an_upstream_arm_refusal_or_unequal_peer_sends_nothing_and_is_a_502() {
    let world = World::start(UPSTREAM_ARM, Spec::default()).await;
    let kinds = [
        KtlsArmError::UlpInstall { errno: 2 },
        KtlsArmError::CryptoInfo { errno: 22 },
        KtlsArmError::CipherNotSupported,
        KtlsArmError::KernelConnection,
    ];
    for kind in kinds {
        world.relay.fail_next_upstream_arm(kind);
        let opens_before = world.opens();
        let cleanups_before = world.cleanups();
        let legs = world.run(client_sends(&get("/a")), vec![]).await;
        assert_eq!(legs.public_statuses(), vec![502], "{kind:?}: the public request is a 502");
        assert!(
            legs.upstream.iter().all(|leg| leg.written.is_empty()),
            "{kind:?}: no octet reaches the peer"
        );
        assert_eq!(world.opens() - opens_before, 1, "{kind:?}");
        assert_eq!(world.cleanups() - cleanups_before, 1, "{kind:?}: cleanup runs once");
        assert_eq!(world.dataplane.residue(), (0, 0), "{kind:?}");
    }

    world.mtls.present_an_unequal_peer();
    let cleanups_before = world.cleanups();
    let legs = world.run(client_sends(&get("/b")), vec![]).await;
    assert_eq!(legs.public_statuses(), vec![502], "an unequal peer is a 502");
    assert!(legs.upstream.is_empty(), "a failed equality arms nothing");
    assert_eq!(world.cleanups() - cleanups_before, 1);
}

/// S-PIG-98 (public): the same four kinds refused at the public
/// `accept_public` arm close the public socket without reading any HTTP octet
/// and open no upstream leg.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER exact-peer-client-mtls"]
async fn a_public_arm_refusal_closes_the_socket_without_reading_any_http_octet() {
    let world = World::start(PUBLIC_ARM, Spec::default()).await;
    let kinds = [
        KtlsArmError::UlpInstall { errno: 2 },
        KtlsArmError::CryptoInfo { errno: 22 },
        KtlsArmError::CipherNotSupported,
        KtlsArmError::KernelConnection,
    ];
    for kind in kinds {
        world.relay.fail_next_public_arm(kind);
        let opens_before = world.opens();
        let first = world.relay.leg_transcripts().len();
        let mut running = world.start_exchange(client_sends(&get("/a")), vec![]);
        wait_for("the gateway to close the refused socket", || {
            is_closed_by_gateway(&mut running.client).then_some(())
        })
        .await;
        let minted = world.relay.leg_transcripts().len() - first;
        let read_calls: usize = world
            .relay
            .leg_transcripts()
            .iter()
            .skip(first)
            .map(|leg| calls_of(leg, RelayCall::ReadThrough))
            .sum();
        assert_eq!(read_calls, 0, "{kind:?}: no HTTP octet is read from the refused socket");
        assert_eq!(world.opens() - opens_before, 0, "{kind:?}: no upstream leg opens");
        assert_eq!(
            world
                .relay
                .leg_transcripts()
                .iter()
                .skip(first)
                .filter(|leg| leg.role == LegRole::Upstream)
                .count(),
            0,
            "{kind:?}"
        );
        assert!(minted <= 1, "{kind:?}: at most the refused public leg exists");
        assert_eq!(world.relay.live_leg_count(), 0, "{kind:?}: nothing stays armed");
    }
}

// ---------------------------------------------------------------------------
// S-PIG-99: relay probe gold test
// ---------------------------------------------------------------------------

/// S-PIG-99: the honest Sim substrate passes the Earned-Trust probe; each
/// substrate lie (`tls` ULP absent, cipher refused, splice losing octets, pipe
/// clamped below 64 KiB, a probe timeout, an I/O error) is refused with its
/// own typed `RelayProbeError` variant (the Sim gold test), and a gateway
/// booted behind each lie runs its production gate, refuses it, leaves the
/// public listener unbound and reports the closed `ConnectPathUnavailable`
/// cause with nothing promoted; the honest substrate is promoted.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER relay-earned-trust-probe"]
async fn the_relay_probe_refuses_every_substrate_lie_with_its_own_typed_error() {
    let clock = Arc::new(SimClock::new());
    let relay = SimGatewayRelay::new(clock);
    let deadline = || Instant::now() + Duration::from_secs(5);

    assert_eq!(relay.probe(deadline()).await, Ok(()), "the honest substrate passes");

    let lies: Vec<(SubstrateLie, fn(&RelayProbeError) -> bool)> = vec![
        (SubstrateLie::UlpAbsent, |error| {
            matches!(error, RelayProbeError::Arm(KtlsArmError::UlpInstall { .. }))
        }),
        (SubstrateLie::CipherRefused, |error| {
            matches!(error, RelayProbeError::Arm(KtlsArmError::CipherNotSupported))
        }),
        (SubstrateLie::SpliceInexact { moved: 65_536, expected: 65_537 }, |error| {
            matches!(error, RelayProbeError::SpliceExactness { moved: 65_536, expected: 65_537 })
        }),
        (SubstrateLie::PipeClamped { got: 4096 }, |error| {
            matches!(error, RelayProbeError::Pipe(RelayPipeError::CapacityBelowFloor { got: 4096 }))
        }),
        (SubstrateLie::ProbeTimeout, |error| matches!(error, RelayProbeError::Timeout)),
        (SubstrateLie::ProbeIo { kind: std::io::ErrorKind::PermissionDenied }, |error| {
            matches!(error, RelayProbeError::Io { kind: std::io::ErrorKind::PermissionDenied })
        }),
    ];
    let mut refused = Vec::new();
    for (lie, is_matching) in lies {
        relay.present_substrate(Some(lie.clone()));
        let error = relay.probe(deadline()).await.expect_err("a lying substrate is refused");
        assert!(is_matching(&error), "{lie:?} is refused with its own variant, got {error:?}");
        refused.push(std::mem::discriminant(&error));
    }
    for (index, variant) in refused.iter().enumerate() {
        for other in &refused[index + 1..] {
            assert_ne!(variant, other, "no probe variant stands for two lies");
        }
    }

    relay.present_substrate(None);
    assert_eq!(relay.probe(deadline()).await, Ok(()), "the honest substrate passes again");
    assert_eq!(relay.probe_calls(), 8, "one probe call per probe");

    // The production gate refuses each lie before it binds anything.
    let gate_lies = [
        SubstrateLie::UlpAbsent,
        SubstrateLie::CipherRefused,
        SubstrateLie::SpliceInexact { moved: 65_536, expected: 65_537 },
        SubstrateLie::PipeClamped { got: 4096 },
        SubstrateLie::ProbeTimeout,
        SubstrateLie::ProbeIo { kind: std::io::ErrorKind::PermissionDenied },
    ];

    // Positive control: behind the honest substrate the same gate promotes.
    let (clock, status, handle, _bind, _site) = start_behind_substrate(None).await;
    let promoted = wait_promoted(&status).await;
    assert!(promoted.current.is_some(), "honest substrate: a Current generation is promoted");
    assert!(
        matches!(promoted.listener, GatewayListenerStatus::Bound { .. }),
        "honest substrate: the public listener is bound"
    );
    assert!(promoted.unavailable.is_none(), "honest substrate: nothing is unavailable");
    let _report = drive_clock(&clock, Duration::from_millis(50), handle.shutdown(GRACE)).await;

    for lie in gate_lies {
        let (clock, status, handle, bind, _site) = start_behind_substrate(Some(lie.clone())).await;
        let refused = wait_refused(&status).await;
        assert_eq!(
            refused.unavailable,
            Some(GatewayApplicationUnavailableCause::ConnectPathUnavailable),
            "{lie:?}: the closed cause",
        );
        assert_eq!(refused.connect_path, GatewayConnectPathStatus::Unavailable, "{lie:?}");
        assert!(refused.current.is_none(), "{lie:?}: nothing is promoted");
        assert_eq!(refused.listener, GatewayListenerStatus::Unbound, "{lie:?}");
        assert!(TcpStream::connect(bind).is_err(), "{lie:?}: TCP/443 stays unbound");
        let _report = drive_clock(&clock, Duration::from_millis(50), handle.shutdown(GRACE)).await;
    }
}

// ---------------------------------------------------------------------------
// S-PIG-101: drain
// ---------------------------------------------------------------------------

/// S-PIG-101: an idle keep-alive connection closes at once; an in-flight
/// exchange runs to completion within the one drain deadline and no later
/// request is parsed; an exchange that cannot finish is force-aborted at the
/// drain deadline; a lingering discard in progress ends at the drain deadline.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn drain_closes_idle_connections_at_once_and_ends_inflight_work_at_the_one_deadline() {
    let second = Duration::from_secs(1);

    // An idle keep-alive connection closes at once: the clock never moves.
    let mut world = World::start(DRAIN, Spec { ordinal: 0, ..Spec::default() }).await;
    let running = world.start_exchange(
        LegScript::new().octets(get("/idle")).quiet(Duration::from_secs(3600)),
        vec![origin_answers(&ok_response(b"one"))],
    );
    wait_for("the first response", || {
        let legs = world.legs(&running);
        legs.public.as_ref().is_some_and(|leg| contains(&leg.written, "one")).then_some(())
    })
    .await;
    let handle = world.handle.take().expect("gateway handle");
    let report = tokio::time::timeout(WAIT, handle.shutdown(GRACE))
        .await
        .expect("an idle connection closes at once, without the clock moving");
    assert_eq!(report.forced_connections, 0, "an idle connection is closed, not forced");
    assert!(world.legs(&running).public().closed);

    // An in-flight exchange completes within the drain; the queued second
    // request is never parsed.
    let mut world = World::start(DRAIN, Spec { ordinal: 1, ..Spec::default() }).await;
    let slow = LegScript::new()
        .octets(response_head(200, "OK", &[("Content-Length", "4")]))
        .quiet(Duration::from_secs(2))
        .octets("done");
    let running = world.start_exchange(
        LegScript::new().octets(get("/slow")).octets(get("/after")).close_notify(),
        vec![slow],
    );
    wait_for("the response head", || {
        let legs = world.legs(&running);
        legs.public.as_ref().is_some_and(|leg| contains(&leg.written, "HTTP/1.1 200")).then_some(())
    })
    .await;
    let report = world.shutdown_with_clock(second).await;
    let legs = world.finish(&running).await;
    assert_eq!(report.forced_connections, 0, "the in-flight exchange finished within the drain");
    assert_eq!(legs.public_statuses(), vec![200]);
    assert_eq!(legs.public_messages()[0].body, b"done");
    assert_eq!(legs.upstream.len(), 1, "no later request is parsed after the drain began");
    assert_eq!(world.opens(), 1);

    // An exchange that cannot finish is force-aborted at the drain deadline.
    let mut world = World::start(DRAIN, Spec { ordinal: 2, ..Spec::default() }).await;
    let stuck = LegScript::new()
        .octets(response_head(200, "OK", &[("Content-Length", "4")]))
        .quiet(Duration::from_secs(3600))
        .octets("done");
    let running = world.start_exchange(client_sends(&get("/stuck")), vec![stuck]);
    wait_for("the response head", || {
        let legs = world.legs(&running);
        legs.public.as_ref().is_some_and(|leg| contains(&leg.written, "HTTP/1.1 200")).then_some(())
    })
    .await;
    let report = world.shutdown_with_clock(second).await;
    let legs = world.finish(&running).await;
    assert_eq!(report.forced_connections, 1, "force-aborted at the one drain deadline");
    assert!(!contains(&legs.public().written, "done"), "the body never completed");
    assert!(!legs.public().close_notify_sent);
    assert_eq!(world.relay.live_leg_count(), 0, "no residue");

    // A discard in progress ends at the drain deadline.
    let mut world = World::start(DRAIN, Spec { ordinal: 3, ..Spec::default() }).await;
    let running = world.start_exchange(
        LegScript::new()
            .octets(request_head("POST", "/a", &[("Content-Length", "100000")]))
            .octets(vec![b'x'; 1000])
            .quiet(Duration::from_secs(3600)),
        vec![origin_answers(&response(401, b"no"))],
    );
    wait_for("the lingering discard to begin", || {
        let legs = world.legs(&running);
        legs.public
            .as_ref()
            .is_some_and(|leg| calls_of(leg, RelayCall::DiscardRemaining) > 0)
            .then_some(())
    })
    .await;
    let _report = world.shutdown_with_clock(second).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![401]);
    assert!(legs.public().closed, "the discard ended at the drain deadline and the socket closed");
}

// ---------------------------------------------------------------------------
// S-PIG-119: response-side size limits
// ---------------------------------------------------------------------------

/// A head of exactly `total` octets (terminator included) built by padding one
/// header.
fn head_of_exact_len(status_line: &str, with_length: bool, total: usize) -> Vec<u8> {
    let prefix = format!(
        "{status_line}\r\n{}X-Pad: ",
        if with_length { "Content-Length: 0\r\n" } else { "" }
    );
    let pad = total - prefix.len() - 4;
    let mut head = prefix.into_bytes();
    head.extend(std::iter::repeat_n(b'a', pad));
    head.extend_from_slice(b"\r\n\r\n");
    assert_eq!(head.len(), total);
    head
}

/// A `200` head holding exactly `count` header fields.
fn head_with_fields(count: usize) -> Vec<u8> {
    let mut head = String::from("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n");
    for index in 1..count {
        head.push_str(&format!("X-F{index}: v\r\n"));
    }
    head.push_str("\r\n");
    head.into_bytes()
}

/// S-PIG-119: a declared response `Content-Length` of `max_response_body_bytes`
/// plus one, a response head of `max_response_head_bytes` plus one octet, 101
/// header fields (`max_response_headers` is 100) and interim heads whose
/// cumulative size crosses the head limit are each exactly one 502 before any
/// public response octet, with the upstream leg closed and one registration;
/// at the ceiling, at the head limit or at 100 fields the response is relayed
/// octet-exact.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn response_limits_are_a_single_502_one_octet_over_and_a_relay_at_the_limit() {
    let world = World::start(RESPONSE_LIMITS, Spec::default()).await;
    let limits = world.limits();
    let body_ceiling = limits.max_response_body_bytes().get();
    let head_ceiling = limits.max_response_head_bytes().get();
    let field_ceiling = limits.max_response_headers().get();
    assert_eq!(body_ceiling, 16 * 1024 * 1024, "the ceilings the scenario states");
    assert_eq!(head_ceiling, 32 * 1024);
    assert_eq!(field_ceiling, 100);

    let over: Vec<(&str, Vec<u8>)> = vec![
        (
            "declared length one above the ceiling",
            response_head(200, "OK", &[("Content-Length", &(body_ceiling + 1).to_string())]),
        ),
        ("head one octet over", head_of_exact_len("HTTP/1.1 200 OK", true, head_ceiling + 1)),
        ("101 header fields", head_with_fields(field_ceiling + 1)),
        (
            "interim heads crossing the cumulative limit",
            concat(&[
                &head_of_exact_len("HTTP/1.1 103 Early Hints", false, head_ceiling / 3),
                &head_of_exact_len("HTTP/1.1 103 Early Hints", false, head_ceiling / 3),
                &head_of_exact_len("HTTP/1.1 103 Early Hints", false, head_ceiling / 3),
                &head_of_exact_len("HTTP/1.1 103 Early Hints", false, head_ceiling / 3),
                &ok_response(b"late"),
            ]),
        ),
    ];
    for (name, origin) in over {
        let opens_before = world.opens();
        let legs = world.run(client_sends(&get("/a")), vec![origin_answers(&origin)]).await;
        assert_eq!(legs.public_statuses(), vec![502], "{name}: exactly one 502");
        let (_, consumed) = parse_wire(&legs.public().written, Kind::Response);
        assert_eq!(consumed, legs.public().written.len(), "{name}: nothing but the 502");
        assert!(legs.upstream[0].closed, "{name}: the upstream leg is closed");
        assert_eq!(world.opens() - opens_before, 1, "{name}: one registration, no retry");
    }

    let at_limit: Vec<(&str, Vec<u8>, Vec<u8>)> = vec![
        (
            "declared length at the ceiling",
            concat(&[
                &response_head(200, "OK", &[("Content-Length", &body_ceiling.to_string())]),
                &vec![b'z'; usize::try_from(body_ceiling).expect("ceiling")],
            ]),
            vec![b'z'; usize::try_from(body_ceiling).expect("ceiling")],
        ),
        ("head at the limit", head_of_exact_len("HTTP/1.1 200 OK", true, head_ceiling), Vec::new()),
        ("100 header fields", head_with_fields(field_ceiling), Vec::new()),
        (
            "interim heads within the cumulative limit",
            concat(&[
                &head_of_exact_len("HTTP/1.1 103 Early Hints", false, head_ceiling / 4),
                &head_of_exact_len("HTTP/1.1 103 Early Hints", false, head_ceiling / 4),
                &ok_response(b"fine"),
            ]),
            b"fine".to_vec(),
        ),
    ];
    for (name, origin, body) in at_limit {
        let legs = world.run(client_sends(&get("/b")), vec![origin_answers(&origin)]).await;
        assert_eq!(legs.public_statuses(), vec![200], "{name}: relayed");
        assert_eq!(legs.public_messages()[0].body, body, "{name}: octet-exact");
    }
}

// ---------------------------------------------------------------------------
// S-PIG-120: the response-head deadline
// ---------------------------------------------------------------------------

/// Start an exchange for a bodyless `GET` and return once the upstream request
/// head has been written: that instant is t0, and the injected clock has not
/// moved since.
async fn exchange_at_t0(world: &World, origin: LegScript) -> Running {
    let running = world.start_exchange(client_sends(&get("/a")), vec![origin]);
    wait_for("the upstream request head to be written", || {
        let legs = world.legs(&running);
        legs.upstream.first().is_some_and(|leg| !leg.written.is_empty()).then_some(())
    })
    .await;
    running
}

/// S-PIG-120: with the origin silent after the upstream request-head write
/// (t0), the exchange waits until exactly t0 + `response_head_deadline` (5 s),
/// then answers one 502 and closes the upstream leg; dropped interim heads
/// (a `103` at t0 + 2 s and another at t0 + 4 s) share that one budget and
/// never restart it, so an absent or partly received final head at t0 + 5 s is
/// the same single 502, while a final head completing at t0 + 4.9 s is
/// relayed; no retry; the deadline does not apply once the public head is
/// sent.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn the_response_head_has_one_budget_from_t0_and_interim_heads_never_restart_it() {
    let world = World::start(RESPONSE_HEAD_DEADLINE, Spec::default()).await;
    let budget = world.limits().response_head_deadline();
    assert_eq!(budget, Duration::from_secs(5), "the budget the scenario states");
    let tick = Duration::from_millis(1);
    let hundred_ms = Duration::from_millis(100);
    let interim = response_head(103, "Early Hints", &[("Link", "</s.css>; rel=preload")]);

    struct Case {
        name: &'static str,
        origin: LegScript,
        /// The final head completes before the budget ends.
        relayed: bool,
    }
    let cases = vec![
        Case { name: "silent origin", origin: LegScript::new(), relayed: false },
        Case {
            name: "interim heads at 2 s and 4 s, final withheld",
            origin: LegScript::new()
                .quiet(Duration::from_secs(2))
                .octets(interim.clone())
                .quiet(Duration::from_secs(2))
                .octets(interim.clone()),
            relayed: false,
        },
        Case {
            name: "final head completing at 4.9 s",
            origin: LegScript::new()
                .quiet(Duration::from_secs(2))
                .octets(interim.clone())
                .quiet(Duration::from_secs(2))
                .octets(interim.clone())
                .quiet(Duration::from_millis(900))
                .octets(ok_response(b"fine")),
            relayed: true,
        },
        Case {
            name: "final head partly received at 4.9 s",
            origin: LegScript::new()
                .quiet(Duration::from_secs(2))
                .octets(interim.clone())
                .quiet(Duration::from_secs(2))
                .octets(interim)
                .quiet(Duration::from_millis(900))
                .octets("HTTP/1.1 200 OK\r\nContent-Le"),
            relayed: false,
        },
    ];
    let mut expected_opens = 0;
    for case in cases {
        let running = exchange_at_t0(&world, case.origin).await;
        expected_opens += 1;
        world.advance(budget - tick, hundred_ms).await;
        let now = world.legs(&running);
        if case.relayed {
            assert!(
                contains(&now.public().written, "HTTP/1.1 200"),
                "{}: the head completing at 4.9 s is relayed",
                case.name
            );
        } else {
            assert!(
                now.public().written.is_empty(),
                "{}: still waiting one millisecond before the budget ends",
                case.name
            );
            assert!(!now.upstream[0].closed, "{}", case.name);
            world.advance(tick, tick).await;
        }
        let legs = world.finish(&running).await;
        if case.relayed {
            assert_eq!(legs.public_statuses(), vec![200], "{}", case.name);
            assert_eq!(legs.public_messages()[0].body, b"fine");
        } else {
            assert_eq!(legs.public_statuses(), vec![502], "{}: exactly one 502", case.name);
        }
        assert!(
            !contains(&legs.public().written, "HTTP/1.1 103"),
            "{}: no interim head leaks",
            case.name
        );
        assert!(legs.upstream[0].closed, "{}: the upstream leg is closed once", case.name);
        assert_eq!(world.opens(), expected_opens, "{}: no retry", case.name);
    }

    // Once the public head is sent the deadline no longer applies.
    let origin = LegScript::new()
        .octets(response_head(200, "OK", &[("Content-Length", "2")]))
        .quiet(Duration::from_secs(20))
        .octets("ok");
    let running = exchange_at_t0(&world, origin).await;
    world.advance(Duration::from_secs(20), Duration::from_secs(1)).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![200], "the body may take longer than the head budget");
    assert_eq!(legs.public_messages()[0].body, b"ok");
}

// ---------------------------------------------------------------------------
// S-PIG-121: an early answer is never followed by a 408
// ---------------------------------------------------------------------------

/// S-PIG-121 (early-answer arm): once any response octet has been written (an
/// early origin answer) a public client that then stalls gets no 408 at all;
/// the lingering close governs, ends at its no-progress bound, and closes the
/// socket.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER relay-lingering-close"]
async fn a_stalled_client_after_an_early_answer_gets_no_408_only_the_lingering_close() {
    let world = World::start(EARLY_ANSWER, Spec::default()).await;
    let no_progress = world.limits().body_no_progress_deadline();
    let running = world.start_exchange(
        LegScript::new()
            .octets(request_head("POST", "/a", &[("Content-Length", "100000")]))
            .quiet(Duration::from_secs(3600)),
        vec![origin_answers(&response(401, b"no"))],
    );
    wait_for("the early answer", || {
        let legs = world.legs(&running);
        legs.public.as_ref().is_some_and(|leg| contains(&leg.written, "HTTP/1.1 401")).then_some(())
    })
    .await;
    world.advance(no_progress + Duration::from_secs(1), Duration::from_secs(1)).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![401], "no 408 follows the early answer");
    assert_lingering_close(&legs, "stalled client after an early answer");
}

// ---------------------------------------------------------------------------
// S-PIG-120 (118 s arm) and S-PIG-121 (408 and 120 s arms): the request budget
// ---------------------------------------------------------------------------

/// The octets of `needle` that the origin has been sent (head and body written
/// to the first upstream leg), true once `legs` holds them at the end.
fn upstream_ends_with(legs: &Legs, needle: &[u8]) -> bool {
    legs.upstream.first().is_some_and(|leg| leg.written.ends_with(needle))
}

/// S-PIG-120 (118 s arm, R-37): a request whose body upload completes at 118 of
/// its 120 s (every piece moves inside the 30 s no-progress window, so the
/// upload is never cut) has its response-head budget start at that instant
/// `t0` and end at the earlier of `t0 + response_head_deadline` (5 s) and the
/// whole-request deadline: the silent origin is waited for exactly 2 s, then
/// answered with one 502, and never for the 5 s the head budget alone would
/// allow; one open, no retry.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn a_response_head_after_a_slow_upload_waits_only_the_remaining_request_time() {
    let world = World::start(UPLOAD_COMPLETES_LATE, Spec::default()).await;
    let limits = world.limits();
    let whole = limits.whole_request_deadline();
    assert_eq!(whole, Duration::from_secs(120), "the budget the scenario states");
    let piece_every = Duration::from_millis(29_500);
    assert!(piece_every < limits.body_no_progress_deadline(), "every piece is inside a window");
    let tick = Duration::from_millis(1);
    let half_second = Duration::from_millis(500);

    let mut public =
        LegScript::new().octets(request_head("POST", "/a", &[("Content-Length", "4")]));
    for piece in [b"a", b"b", b"c", b"d"] {
        public = public.quiet(piece_every).octets(piece.to_vec());
    }
    let running = world.start_exchange(public, vec![LegScript::new()]);
    world.accepted(&running).await;

    // The upload completes at 4 x 29.5 s = 118 s; that is t0.
    world.advance(piece_every * 4, half_second).await;
    wait_for("the whole upload to reach the origin", || {
        upstream_ends_with(&world.legs(&running), b"abcd").then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        world.legs(&running).public().written.is_empty(),
        "no response octet at t0: the origin is still being waited for",
    );

    // 2 s remain of the whole-request budget: one millisecond short of it the
    // exchange still waits (the 5 s head budget alone would allow 3 s more).
    world.advance(whole - piece_every * 4 - tick, half_second).await;
    let now = world.legs(&running);
    assert!(now.public().written.is_empty(), "still waiting one millisecond before the deadline");
    assert!(!now.upstream[0].closed, "the origin leg is still open");

    world.advance(tick, tick).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![502], "exactly one 502 at the whole-request deadline");
    assert!(legs.upstream[0].closed, "the origin leg is closed once");
    assert_eq!(world.opens(), 1, "no retry");
}

/// One stalled-request-body case: the public script and the point, observed at
/// the origin, from which the gateway is waiting on the stalled octet (the
/// injected clock does not move meanwhile, so the wait started at this
/// instant).
struct StalledBody {
    name: &'static str,
    public: LegScript,
    waiting: fn(&Legs) -> bool,
}

fn head_reached_the_origin(legs: &Legs) -> bool {
    legs.upstream.first().is_some_and(|leg| !leg.written.is_empty())
}

/// S-PIG-121 (408 arms, R-37): after a complete head a public client that moves
/// no body octet for `body_no_progress_deadline` (30 s) gets exactly one 408
/// before any response octet, and not a millisecond earlier; both legs close,
/// the connection is not reused and no lingering discard follows the 408. The
/// silent origin is never the cause: the stall is bounded by the no-progress
/// window, not by `response_head_deadline`. Arms: a `Content-Length` body, a
/// stalled chunk-size line and a stall in the middle of a chunk.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn a_request_body_that_stops_moving_gets_one_408_at_the_no_progress_window() {
    let mut world = World::start(STALLED_REQUEST_BODY, Spec::default()).await;
    let no_progress = world.limits().body_no_progress_deadline();
    assert_eq!(no_progress, Duration::from_secs(30), "the window the scenario states");
    let tick = Duration::from_millis(1);
    let second = Duration::from_secs(1);
    let forever = Duration::from_secs(3600);
    let chunked_head = request_head("POST", "/a", &[("Transfer-Encoding", "chunked")]);

    let cases = vec![
        StalledBody {
            name: "Content-Length body, no octet",
            public: LegScript::new()
                .octets(request_head("POST", "/a", &[("Content-Length", "1024")]))
                .quiet(forever),
            waiting: head_reached_the_origin,
        },
        StalledBody {
            name: "chunked body, stalled chunk-size line",
            public: LegScript::new().octets(chunked_head.clone()).quiet(forever),
            waiting: head_reached_the_origin,
        },
        StalledBody {
            name: "chunked body, stalled in the middle of a chunk",
            public: LegScript::new()
                .octets(chunked_head)
                .octets("a\r\n")
                .octets("abc")
                .quiet(forever),
            waiting: |legs| upstream_ends_with(legs, b"abc"),
        },
    ];

    let mut expected_opens = 0;
    for case in cases {
        let mut running = world.start_exchange(case.public, vec![LegScript::new()]);
        expected_opens += 1;
        wait_for("the gateway to wait on the stalled body", || {
            (case.waiting)(&world.legs(&running)).then_some(())
        })
        .await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        world.advance(no_progress - tick, second).await;
        let before = world.legs(&running);
        assert!(
            before.public().written.is_empty(),
            "{}: no 408 before the window has elapsed",
            case.name,
        );
        assert!(!before.public().closed, "{}: the connection is still open", case.name);

        world.advance(tick, tick).await;
        let legs = world.finish(&running).await;
        assert_eq!(legs.public_statuses(), vec![408], "{}: exactly one 408", case.name);
        assert_eq!(
            calls_of(legs.public(), RelayCall::DiscardRemaining),
            0,
            "{}: no lingering discard follows a 408",
            case.name,
        );
        assert!(legs.public().closed, "{}: the public leg is closed", case.name);
        assert!(legs.upstream[0].closed, "{}: the origin leg is closed", case.name);
        assert!(
            is_closed_by_gateway(&mut running.client),
            "{}: the client socket is closed, the connection is not reused",
            case.name,
        );
        assert_eq!(world.opens(), expected_opens, "{}: no retry", case.name);
    }
    world.shutdown_with_clock(second).await;
}

/// S-PIG-121 (120 s arm, R-37): a body that moves inside every 30 s window (one
/// octet every 29 s) is never cut by the no-progress rule: no 408 is written
/// and the origin sees every octet; a request still unfinished at
/// `whole_request_deadline` (120 s) with no response octet sent is one 502, at
/// exactly that deadline, with no lingering discard.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn a_trickled_request_body_is_cut_only_at_the_whole_request_deadline_with_a_502() {
    let world = World::start(TRICKLED_REQUEST_BODY, Spec::default()).await;
    let limits = world.limits();
    let whole = limits.whole_request_deadline();
    assert_eq!(whole, Duration::from_secs(120), "the budget the scenario states");
    let every = Duration::from_secs(29);
    assert!(every < limits.body_no_progress_deadline(), "each octet is inside a window");
    let tick = Duration::from_millis(1);
    let second = Duration::from_secs(1);

    let mut public =
        LegScript::new().octets(request_head("POST", "/a", &[("Content-Length", "1000")]));
    for _ in 0..4 {
        public = public.quiet(every).octets(vec![b'x']);
    }
    let public = public.quiet(Duration::from_secs(3600));
    let running = world.start_exchange(public, vec![LegScript::new()]);
    wait_for("the request head to reach the origin", || {
        head_reached_the_origin(&world.legs(&running)).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(20)).await;

    // The last octet moves at 4 x 29 s = 116 s; nothing is written before 120 s.
    world.advance(whole - tick, second).await;
    let before = world.legs(&running);
    assert!(
        before.public().written.is_empty(),
        "no 408 and no 502 while the body keeps moving and the request has time left",
    );
    assert!(upstream_ends_with(&before, b"xxxx"), "the origin saw every trickled octet");
    assert!(!before.upstream[0].closed);

    world.advance(tick, tick).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![502], "one 502 at the whole-request deadline");
    assert_eq!(calls_of(legs.public(), RelayCall::DiscardRemaining), 0, "no discard after it");
    assert!(legs.upstream[0].closed, "the origin leg is closed");
    assert_eq!(world.opens(), 1, "no retry");
}

// ---------------------------------------------------------------------------
// S-PIG-122: public half-close
// ---------------------------------------------------------------------------

/// S-PIG-122: a public peer that sends close_notify once the request is
/// complete (a relayed `Content-Length` body, or a bodyless request) still
/// receives the whole response, `Content-Length` or chunked, and then both legs
/// close, the upstream leg exactly once; close_notify while idle closes
/// silently and opens no upstream leg; close_notify or a reset mid-request-body
/// aborts both legs with no response and no discard.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER kernel-relay-exchange-driver"]
async fn a_public_half_close_delivers_the_response_only_after_the_request_was_fully_read() {
    let world = World::start(HALF_CLOSE, Spec::default()).await;
    let second = Duration::from_secs(1);

    // Content-Length body fully read, then close_notify; the response follows.
    let running = world.start_exchange(
        LegScript::new().octets(post("/a", b"abc")).close_notify(),
        vec![LegScript::new().quiet(second).octets(ok_response(b"done"))],
    );
    wait_for("the request body to reach the origin", || {
        let legs = world.legs(&running);
        legs.upstream.first().is_some_and(|leg| contains(&leg.written, "abc")).then_some(())
    })
    .await;
    world.advance(second, second).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_statuses(), vec![200]);
    assert_eq!(legs.public_messages()[0].body, b"done", "the whole response is delivered");
    assert_eq!(legs.upstream.len(), 1);
    assert!(legs.upstream[0].closed && legs.public().closed, "both legs closed, not reused");

    // Bodyless request, chunked response.
    let running = world.start_exchange(
        client_sends(&get("/b")),
        vec![LegScript::new().quiet(second).octets(chunked_response(b"chunky"))],
    );
    wait_for("the request head to reach the origin", || {
        let legs = world.legs(&running);
        legs.upstream.first().is_some_and(|leg| !leg.written.is_empty()).then_some(())
    })
    .await;
    world.advance(second, second).await;
    let legs = world.finish(&running).await;
    assert_eq!(legs.public_messages()[0].body, b"chunky");
    assert_eq!(world.opens(), 2, "the upstream leg was opened exactly once per request");

    // close_notify while idle: silent close, no upstream leg.
    let opens_before = world.opens();
    let legs = world.run(LegScript::new().close_notify(), vec![]).await;
    assert!(legs.public().written.is_empty(), "closes silently");
    assert!(legs.upstream.is_empty());
    assert_eq!(world.opens(), opens_before, "no upstream leg opened");

    // close_notify and reset mid-request-body: abort, no response, no discard.
    for (name, public) in [
        (
            "close_notify",
            LegScript::new()
                .octets(request_head("POST", "/c", &[("Content-Length", "10")]))
                .octets("abc")
                .close_notify(),
        ),
        (
            "reset",
            LegScript::new()
                .octets(request_head("POST", "/c", &[("Content-Length", "10")]))
                .octets("abc")
                .reset(),
        ),
    ] {
        let legs = world.run(public, vec![LegScript::new()]).await;
        assert!(legs.public().written.is_empty(), "{name}: no response to a half-received request");
        assert_eq!(
            calls_of(legs.public(), RelayCall::DiscardRemaining),
            0,
            "{name}: no lingering discard"
        );
        assert!(legs.upstream[0].closed && legs.public().closed, "{name}: both legs aborted");
    }
}

// ---------------------------------------------------------------------------
// Head and framing policy at the public listener (S-PIG-77 and S-PIG-78)
// ---------------------------------------------------------------------------

/// One public connection that sends `request`, trailing request octets and a
/// close_notify; no origin script is queued, so the request must be refused
/// before any upstream leg exists.
async fn assert_refused_before_upstream(world: &World, request: Vec<u8>, status: u16, what: &str) {
    let opens_before = world.opens();
    let public = LegScript::new().octets(request).octets(vec![b'x'; 64]).close_notify();
    let legs = world.run(public, vec![]).await;
    assert_eq!(legs.public_statuses(), vec![status], "{what}: status");
    assert!(legs.upstream.is_empty(), "{what}: no upstream octet, no upstream leg");
    assert_eq!(world.opens(), opens_before, "{what}: nothing was opened");
}

/// S-PIG-77: ambiguous request framing is refused before any upstream octet:
/// both `Content-Length` and `Transfer-Encoding`; repeated, non-decimal or
/// `u64`-overflowing `Content-Length`; repeated `Transfer-Encoding`,
/// `chunked, chunked`, a final coding other than `chunked`, any other coding:
/// 400; an `HTTP/1.0` request: 505.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER http1-head-validation-and-framing"]
async fn ambiguous_request_framing_is_refused_before_any_upstream_octet() {
    let world = World::start(AMBIGUOUS_REQUEST, Spec::default()).await;
    let with = |headers: &[(&str, &str)]| request_head("POST", "/a", headers);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "Content-Length and Transfer-Encoding",
            with(&[("Content-Length", "3"), ("Transfer-Encoding", "chunked")]),
        ),
        ("repeated Content-Length", with(&[("Content-Length", "3"), ("Content-Length", "3")])),
        ("conflicting Content-Length", with(&[("Content-Length", "3"), ("Content-Length", "4")])),
        ("non-decimal Content-Length", with(&[("Content-Length", "abc")])),
        ("signed Content-Length", with(&[("Content-Length", "+5")])),
        ("negative Content-Length", with(&[("Content-Length", "-5")])),
        ("spaced Content-Length", with(&[("Content-Length", "1 2")])),
        ("hex Content-Length", with(&[("Content-Length", "0x10")])),
        ("empty Content-Length", with(&[("Content-Length", "")])),
        ("overflowing Content-Length", with(&[("Content-Length", "18446744073709551616")])),
        (
            "repeated Transfer-Encoding",
            with(&[("Transfer-Encoding", "chunked"), ("Transfer-Encoding", "chunked")]),
        ),
        ("chunked, chunked", with(&[("Transfer-Encoding", "chunked, chunked")])),
        ("gzip then chunked", with(&[("Transfer-Encoding", "gzip, chunked")])),
        ("chunked then gzip", with(&[("Transfer-Encoding", "chunked, gzip")])),
        ("gzip only", with(&[("Transfer-Encoding", "gzip")])),
        ("identity only", with(&[("Transfer-Encoding", "identity")])),
    ];
    for (what, request) in cases {
        assert_refused_before_upstream(&world, request, 400, what).await;
    }
    assert_refused_before_upstream(
        &world,
        b"GET /a HTTP/1.0\r\nHost: api.example.com\r\n\r\n".to_vec(),
        505,
        "an HTTP/1.0 request",
    )
    .await;
}

/// S-PIG-78: the same ambiguity on an origin response (both lengths, repeated
/// or overflowing length, repeated or unsupported coding, a status line that is
/// not `HTTP/1.1`, `101`) is a 502 before the public response head; an
/// `HTTP/1.0` response with an explicit length is accepted.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER http1-head-validation-and-framing"]
async fn ambiguous_origin_framing_is_a_502_before_the_public_head() {
    let world = World::start(AMBIGUOUS_RESPONSE, Spec::default()).await;
    let with = |headers: &[(&str, &str)]| response_head(200, "OK", headers);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "Content-Length and Transfer-Encoding",
            with(&[("Content-Length", "2"), ("Transfer-Encoding", "chunked")]),
        ),
        ("repeated Content-Length", with(&[("Content-Length", "2"), ("Content-Length", "2")])),
        ("overflowing Content-Length", with(&[("Content-Length", "18446744073709551616")])),
        ("non-decimal Content-Length", with(&[("Content-Length", "two")])),
        (
            "repeated Transfer-Encoding",
            with(&[("Transfer-Encoding", "chunked"), ("Transfer-Encoding", "chunked")]),
        ),
        ("chunked, chunked", with(&[("Transfer-Encoding", "chunked, chunked")])),
        ("gzip then chunked", with(&[("Transfer-Encoding", "gzip, chunked")])),
        ("gzip only", with(&[("Transfer-Encoding", "gzip")])),
        ("HTTP/2.0 status line", b"HTTP/2.0 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec()),
        ("HTTP/0.9-style status line", b"HTTP/0.9 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec()),
        (
            "101 Switching Protocols",
            response_head(
                101,
                "Switching Protocols",
                &[("Upgrade", "foo"), ("Connection", "Upgrade")],
            ),
        ),
    ];
    for (what, origin) in cases {
        let opens_before = world.opens();
        let legs = world.run(client_sends(&get("/a")), vec![origin_answers(&origin)]).await;
        assert_eq!(legs.public_statuses(), vec![502], "{what}: exactly one 502");
        let (_, consumed) = parse_wire(&legs.public().written, Kind::Response);
        assert_eq!(
            consumed,
            legs.public().written.len(),
            "{what}: no octet of the origin's head leaked"
        );
        assert_eq!(world.opens() - opens_before, 1, "{what}: no retry");
    }

    let legs = world
        .run(
            client_sends(&get("/b")),
            vec![origin_answers(b"HTTP/1.0 200 OK\r\nContent-Length: 3\r\n\r\nabc")],
        )
        .await;
    assert_eq!(legs.public_statuses(), vec![200], "an HTTP/1.0 response with an explicit length");
    assert_eq!(legs.public_messages()[0].body, b"abc");
}

// ---------------------------------------------------------------------------
// S-PIG-118: the public server configuration
// ---------------------------------------------------------------------------

/// S-PIG-118 (server configuration): the configuration the production listener
/// hands to `accept_public` is the custody snapshot's single builder: TLS 1.3
/// only, exactly the two AES-GCM suites (AES-128-GCM and AES-256-GCM, no
/// ChaCha20-Poly1305), ALPN `http/1.1` alone, no tickets, no 0-RTT, secret
/// extraction enabled.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER certified-key-custody"]
async fn the_public_server_configuration_lists_exactly_the_two_aes_gcm_suites() {
    let capture = std::cell::RefCell::new(None);
    let world = World::start_with(SERVER_CONFIGURATION, Spec::default(), |relay| {
        let wrapper = ConfigurationCapture::new(relay);
        *capture.borrow_mut() = Some(Arc::clone(&wrapper));
        wrapper as Arc<dyn GatewayRelay>
    })
    .await;
    let capture = capture.into_inner().expect("the relay was wrapped");
    let legs = world.run(client_sends(&get("/a")), vec![origin_answers(&ok_response(b"ok"))]).await;
    assert_eq!(legs.public_statuses(), vec![200]);

    let configurations = capture.configurations();
    assert_eq!(configurations.len(), 1, "one public handshake, one configuration");
    let config = &configurations[0];
    let suites: Vec<CipherSuite> = config
        .crypto_provider()
        .cipher_suites
        .iter()
        .map(rustls::SupportedCipherSuite::suite)
        .collect();
    assert_eq!(suites.len(), 2, "exactly two suites: {suites:?}");
    assert!(suites.contains(&CipherSuite::TLS13_AES_128_GCM_SHA256));
    assert!(suites.contains(&CipherSuite::TLS13_AES_256_GCM_SHA384));
    assert!(
        config
            .crypto_provider()
            .cipher_suites
            .iter()
            .all(|suite| matches!(suite, rustls::SupportedCipherSuite::Tls13(_))),
        "TLS 1.3 only: no TLS 1.2 suite is offered"
    );
    assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()], "ALPN http/1.1 alone");
    assert!(config.enable_secret_extraction, "secret extraction is enabled for kernel arming");
    assert_eq!(config.send_tls13_tickets, 0, "no session tickets");
    assert_eq!(config.max_early_data_size, 0, "no 0-RTT");
}
