//! Test-local doubles and fixtures shared by the Gateway Application QC-2
//! conformance driver and the S-PIG-73 duplicate-completion acceptance body.
//!
//! The doubles stand at the gateway-crate driven ports the Application owner
//! composes over (`GatewayConnectDataplane`, `GatewayClientMtls`,
//! `GatewayIdentityLifecycleControl`, `GatewayRelay`). They cannot be the
//! `overdrive-sim` gateway adapters: those implement the other copy of the
//! crate-local traits that `overdrive-sim` links (feature-delta, "Conformance
//! placement and test-local doubles (DG-7)"). Only core-trait Sims
//! (`SimClock`, `SimObservationStore`, `SimServiceVipView`, `SimKek`) come
//! from `overdrive-sim`. The module is crate-private and compiles only under
//! `integration-tests`.
//!
//! # Held-probe capability (DG-9)
//!
//! The per-candidate probe the owner awaits is
//! `GatewayUpstreamConnector::probe`: `open_upstream`, a real `connect(2)` to
//! the Service frontend, `take_receipt`, `selected_backend_identity`, then
//! `GatewayClientMtls::authenticate`. The park point is `authenticate`:
//!
//! 1. each probe attempt parks as its own entry ([`HeldProbes::park`]);
//! 2. one entry is released by the identity the connect double minted for it
//!    ([`HeldProbes::release`]), never by arrival position;
//! 3. a release carries a typed outcome: [`ProbeOutcome::Usable`] returns
//!    `Ok(GatewayUpstream)` sealed through the real `GatewayUpstreamSeal`
//!    over an inert in-memory leg, [`ProbeOutcome::Unusable`] returns
//!    `Err(GatewayClientMtlsError::Handshake)`; no half of the leg is used;
//! 4. the number of entries held is observable ([`HeldProbes::held_count`]).
//!
//! An entry is removed when it is released and when its waiter is dropped, so
//! releasing an entry whose waiter is gone (a probe the owner discarded) or
//! one already released returns [`ReleaseOutcome::Absent`], a no-op.
//!
//! # Bind-address table (R-6)
//!
//! Every composition that reaches promotion binds its own `127.0.0.0/8`
//! address on TCP/443: `127.<octet>.<hi>.<lo>` for ordinal `n` with
//! `hi = n / 250`, `lo = n % 250 + 1`; the Service frontend (with a listening
//! socket on port 8080, so the connector's real `connect(2)` completes) is
//! `127.<octet>.<200 + hi>.<lo>`. The integration binary owns second octets
//! 41..=62.
//!
//! | octet | owner |
//! |---|---|
//! | 63 | QC-2 `replace_valid` (`quint_gateway_application`), one ordinal per composition |
//! | 64 | S-PIG-73 (`probe_completion_acceptance`) |
//! | 65 | QC-2 `route_withdraw_drain` |
//! | 66 | QC-2 `late_probe_after_supersession` |
//! | 67 | QC-2 randomized traces |
#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    reason = "CONTRACT_SHAPE metadata, fixture preconditions and the address arithmetic of the table above"
)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, TcpListener};
use std::num::{NonZeroU64, NonZeroUsize};
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, ServiceV1, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::gateway_identity::{GatewayIdentityEpoch, GatewayIdentityFacts};
use overdrive_core::id::{BackendId, CertSerial, ContentHash, NodeId, ServiceVip, SpiffeId};
use overdrive_core::public_ingress::{
    GatewayApplicationStatusRowV1, PublicCertifiedKeyId, PublicRouteInput, Route, ServiceKey,
    ServiceListenerReferenceInput, SocketCookie,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::kek::SimKek;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::read_ports::SimServiceVipView;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;
use tokio::sync::{oneshot, watch};

use super::{
    GatewayAdmission, GatewayAdmissionBudget, GatewayControl, GatewayHandle,
    GatewayRuntimeDependencies, GatewayShutdownReport, UnboundGatewayBuilder,
};
use crate::ports::{
    AcceptedPublicLeg, GatewayClientMtls, GatewayClientMtlsError, GatewayConnectCleanupSweep,
    GatewayConnectDataplane, GatewayConnectDataplaneError, GatewayConnectIntent,
    GatewayIdentityLifecycleControl, GatewayIdentityLifecycleError, GatewayRelay,
    GatewaySelectionReceipt, GatewayUpstream, GatewayUpstreamSeal, HeadTerminator, KtlsLeg,
    KtlsReadHalf, KtlsWriteHalf, LegReadError, LegWriteError, PreparedUpstream,
    PublicHandshakeError, ReadDeadlines, RelayDeadlines, RelayError, RelayProbeError, ThroughBytes,
};
use crate::runtime::GatewayLimits;
use crate::{GatewayConfig, ManualCertifiedKeyConfig};

/// The port of the Service listener the Route targets.
pub(super) const SERVICE_PORT: u16 = 8080;
/// Grace given to `GatewayHandle::shutdown`.
pub(super) const GRACE: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Held probes: the registry shared by the client double and its driver
// ---------------------------------------------------------------------------

/// The typed outcome a release delivers to a parked `authenticate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProbeOutcome {
    /// `Ok(GatewayUpstream)`: the connector reports `SelectedPeerAuthenticated`.
    Usable,
    /// `Err(GatewayClientMtlsError::Handshake)`: the connector reports `Mtls(..)`.
    Unusable,
}

/// What a release found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReleaseOutcome {
    /// The entry was parked and its waiter received the outcome.
    Released,
    /// No such entry: never parked, already released, or its waiter is gone.
    Absent,
}

struct HeldEntry {
    sequence: u64,
    outcome: oneshot::Sender<ProbeOutcome>,
}

#[derive(Default)]
struct HeldState {
    next_sequence: u64,
    entries: BTreeMap<SpiffeId, HeldEntry>,
}

/// Entries parked at `GatewayClientMtls::authenticate`, addressed by the
/// peer identity the connect double minted for each.
#[derive(Default)]
pub(super) struct HeldProbes {
    state: Mutex<HeldState>,
}

/// Removes its entry when the parked waiter is dropped.
struct ParkGuard {
    probes: Arc<HeldProbes>,
    peer: SpiffeId,
    sequence: u64,
}

impl Drop for ParkGuard {
    fn drop(&mut self) {
        let mut state = self.probes.state.lock().expect("held probes");
        if state.entries.get(&self.peer).is_some_and(|entry| entry.sequence == self.sequence) {
            state.entries.remove(&self.peer);
        }
    }
}

impl HeldProbes {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn park(self: &Arc<Self>, peer: SpiffeId) -> (ParkGuard, oneshot::Receiver<ProbeOutcome>) {
        let (sender, receiver) = oneshot::channel();
        let mut state = self.state.lock().expect("held probes");
        state.next_sequence += 1;
        let sequence = state.next_sequence;
        state.entries.insert(peer.clone(), HeldEntry { sequence, outcome: sender });
        drop(state);
        (ParkGuard { probes: Arc::clone(self), peer, sequence }, receiver)
    }

    /// Number of entries currently parked.
    pub(super) fn held_count(&self) -> usize {
        self.state.lock().expect("held probes").entries.len()
    }

    /// Parked peers in the order they parked (oldest first).
    pub(super) fn held_peers(&self) -> Vec<SpiffeId> {
        let state = self.state.lock().expect("held probes");
        let mut ordered: Vec<(u64, SpiffeId)> =
            state.entries.iter().map(|(peer, entry)| (entry.sequence, peer.clone())).collect();
        drop(state);
        ordered.sort();
        ordered.into_iter().map(|(_, peer)| peer).collect()
    }

    /// Complete the entry minted for `peer` with `outcome`.
    pub(super) fn release(&self, peer: &SpiffeId, outcome: ProbeOutcome) -> ReleaseOutcome {
        let removed = self.state.lock().expect("held probes").entries.remove(peer);
        let entry = removed;
        match entry.map(|entry| entry.outcome.send(outcome)) {
            Some(Ok(())) => ReleaseOutcome::Released,
            Some(Err(_)) | None => ReleaseOutcome::Absent,
        }
    }
}

// ---------------------------------------------------------------------------
// GatewayClientMtls double
// ---------------------------------------------------------------------------

/// Parks every `authenticate` as its own entry keyed by `expected_peer`.
pub(super) struct HeldClientMtls {
    seal: GatewayUpstreamSeal,
    probes: Arc<HeldProbes>,
}

impl HeldClientMtls {
    pub(super) fn new(seal: GatewayUpstreamSeal, probes: Arc<HeldProbes>) -> Self {
        Self { seal, probes }
    }
}

#[async_trait]
impl GatewayClientMtls for HeldClientMtls {
    async fn authenticate(
        &self,
        connected: OwnedFd,
        expected_peer: SpiffeId,
        _deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
        drop(connected);
        let (_parked, outcome) = self.probes.park(expected_peer);
        match outcome.await {
            Ok(ProbeOutcome::Usable) => {
                Ok(self.seal.seal_authenticated(KtlsLeg::from_armed(Box::new(()), Box::new(()))))
            }
            Ok(ProbeOutcome::Unusable) | Err(_) => Err(GatewayClientMtlsError::Handshake),
        }
    }
}

// ---------------------------------------------------------------------------
// GatewayConnectDataplane double
// ---------------------------------------------------------------------------

/// The peer identity the double answers for one minted backend.
pub(super) fn backend_identity(backend_id: BackendId) -> SpiffeId {
    SpiffeId::new(&format!("spiffe://overdrive.local/workload/api/alloc/api-{}", backend_id.get()))
        .expect("backend SPIFFE ID")
}

#[derive(Default)]
struct ConnectState {
    next_cookie: u64,
    next_backend: u32,
    registered: BTreeMap<SocketCookie, (GatewayConnectIntent, BackendId)>,
    pending_receipts: BTreeMap<SocketCookie, BackendId>,
}

/// Answers every `open_upstream` with a fresh unconnected socket, a freshly
/// minted nonzero cookie and a distinct minted `BackendId`; `take_receipt`
/// answers `Selected` carrying that backend (a held probe cannot be a
/// `NoBackend` probe).
#[derive(Default)]
pub(super) struct HeldConnectDouble {
    state: Mutex<ConnectState>,
}

impl HeldConnectDouble {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Number of upstream sockets opened so far.
    pub(super) fn opened_count(&self) -> u64 {
        self.state.lock().expect("connect state").next_cookie
    }
}

#[async_trait]
impl GatewayConnectDataplane for HeldConnectDouble {
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
        let mut state = self.state.lock().expect("connect state");
        state.next_cookie += 1;
        state.next_backend += 1;
        let cookie = SocketCookie::new(state.next_cookie).expect("nonzero cookie");
        let backend = BackendId::new(state.next_backend).expect("backend ID");
        let intent = GatewayConnectIntent { socket_cookie: cookie, service_key: key };
        state.registered.insert(cookie, (intent.clone(), backend));
        state.pending_receipts.insert(cookie, backend);
        drop(state);
        Ok(PreparedUpstream::new(socket, intent))
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("connect state");
        let Some((registered, _)) = state.registered.get(&intent.socket_cookie).cloned() else {
            return Err(GatewayConnectDataplaneError::IntentMissing);
        };
        let Some(backend_id) = state.pending_receipts.remove(&intent.socket_cookie) else {
            return Err(GatewayConnectDataplaneError::ReceiptMissing);
        };
        drop(state);
        Ok(GatewaySelectionReceipt::Selected {
            socket_cookie: registered.socket_cookie,
            service_key: registered.service_key,
            backend_id,
        })
    }

    fn cleanup(&self, intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("connect state");
        state.registered.remove(&intent.socket_cookie);
        state.pending_receipts.remove(&intent.socket_cookie);
        drop(state);
        Ok(())
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("connect state");
        let removed_intents = u32::try_from(state.registered.len()).expect("intent count");
        let removed_receipts = u32::try_from(state.pending_receipts.len()).expect("receipt count");
        state.registered.clear();
        state.pending_receipts.clear();
        drop(state);
        Ok(GatewayConnectCleanupSweep { removed_intents, removed_receipts })
    }

    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        Ok(backend_identity(backend_id))
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        let state = self.state.lock().expect("connect state");
        Ok(u32::try_from(state.registered.len()).expect("intent count"))
    }

    fn live_receipt_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        let state = self.state.lock().expect("connect state");
        Ok(u32::try_from(state.pending_receipts.len()).expect("receipt count"))
    }
}

// ---------------------------------------------------------------------------
// GatewayIdentityLifecycleControl double
// ---------------------------------------------------------------------------

fn gateway_identity_facts() -> GatewayIdentityFacts {
    GatewayIdentityFacts {
        epoch: GatewayIdentityEpoch::first(),
        spiffe_id: SpiffeId::new("spiffe://overdrive.local/gateway/gateway-node")
            .expect("gateway SPIFFE ID"),
        serial: CertSerial::new("01").expect("serial"),
        not_after: overdrive_core::UnixInstant::from_unix_duration(Duration::from_secs(
            4_000_000_000,
        )),
    }
}

/// Answers `ensure_current`, `current` and `subscribe` immediately with
/// current facts; `disable_after_drain` is a synchronous clear (R-4).
pub(super) struct CurrentIdentity {
    facts: GatewayIdentityFacts,
    sender: watch::Sender<Option<GatewayIdentityFacts>>,
}

impl CurrentIdentity {
    pub(super) fn new() -> Arc<Self> {
        let facts = gateway_identity_facts();
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

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.sender.subscribe()
    }

    async fn disable_after_drain(
        &self,
        _deadline: Instant,
    ) -> Result<(), GatewayIdentityLifecycleError> {
        self.sender.send_replace(None);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// GatewayRelay double
// ---------------------------------------------------------------------------

/// `probe` succeeds. No public client socket is driven on the QC-2 path, so
/// every other method returns the port's typed I/O error with
/// `ErrorKind::Unsupported` (the closest typed "unavailable" the port
/// defines: `PublicHandshakeError::Io`, `LegReadError::Io`,
/// `LegWriteError::Io`, `RelayError::{Read, Write}`).
pub(super) struct UnavailableRelay;

#[async_trait]
impl GatewayRelay for UnavailableRelay {
    async fn probe(&self, _deadline: Instant) -> Result<(), RelayProbeError> {
        Ok(())
    }

    async fn accept_public(
        &self,
        accepted: OwnedFd,
        _tls: Arc<rustls::ServerConfig>,
        _deadline: Instant,
    ) -> Result<AcceptedPublicLeg, PublicHandshakeError> {
        drop(accepted);
        Err(PublicHandshakeError::Io { kind: std::io::ErrorKind::Unsupported })
    }

    async fn read_through(
        &self,
        _from: &mut KtlsReadHalf,
        _terminator: HeadTerminator,
        _limit: NonZeroUsize,
        _deadlines: ReadDeadlines,
    ) -> Result<ThroughBytes, LegReadError> {
        Err(LegReadError::Io { kind: std::io::ErrorKind::Unsupported })
    }

    async fn write_all(
        &self,
        _to: &mut KtlsWriteHalf,
        _bytes: &[u8],
        _deadline: Instant,
    ) -> Result<(), LegWriteError> {
        Err(LegWriteError::Io { kind: std::io::ErrorKind::Unsupported })
    }

    async fn relay_exact(
        &self,
        _from: &mut KtlsReadHalf,
        _to: &mut KtlsWriteHalf,
        _len: NonZeroU64,
        _deadlines: RelayDeadlines,
    ) -> Result<(), RelayError> {
        Err(RelayError::Write(LegWriteError::Io { kind: std::io::ErrorKind::Unsupported }))
    }

    async fn relay_to_end(
        &self,
        _from: &mut KtlsReadHalf,
        _to: &mut KtlsWriteHalf,
        _max: NonZeroU64,
        _deadlines: RelayDeadlines,
    ) -> Result<u64, RelayError> {
        Err(RelayError::Read(LegReadError::Io { kind: std::io::ErrorKind::Unsupported }))
    }

    async fn close_write(
        &self,
        _to: &mut KtlsWriteHalf,
        _deadline: Instant,
    ) -> Result<(), LegWriteError> {
        Err(LegWriteError::Io { kind: std::io::ErrorKind::Unsupported })
    }

    async fn discard_remaining(
        &self,
        _from: &mut KtlsReadHalf,
        _max: NonZeroU64,
        _deadlines: RelayDeadlines,
    ) -> Result<u64, LegReadError> {
        Err(LegReadError::Io { kind: std::io::ErrorKind::Unsupported })
    }
}

// ---------------------------------------------------------------------------
// Fixtures: key material, Service intent, Route
// ---------------------------------------------------------------------------

/// The on-disk site of one gateway: certificate and key files (key mode
/// 0600) and the redb Route store path, in one tempdir.
struct Site {
    _dir: TempDir,
    chain_path: PathBuf,
    key_path: PathBuf,
    intent_path: PathBuf,
}

impl Site {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("isolated gateway root");
        let chain_path = dir.path().join("api-origin-chain.pem");
        let key_path = dir.path().join("api-origin-key.pem");
        let intent_path = dir.path().join("intent.redb");
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

/// The custody KEK the `PublicCertifiedKeyAeadCodec` resolves.
fn custody_kek() -> Arc<SimKek> {
    Arc::new(SimKek::new().with(
        &KekId::new("overdrive-public-certified-key").expect("custody KEK ID"),
        [0x5a; KEK_LEN],
    ))
}

/// Persist the `api` Service through the real IntentStore and return the VIP
/// view that maps its digest to `vip`.
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

/// The one Route ID the QC-2 and S-PIG-73 compositions declare.
pub(super) const ROUTE_ID: &str = "public-api";

/// The Route `public-api` for `path`; distinct paths are distinct Route
/// generations.
pub(super) fn route(path: &str) -> Route {
    Route::from_submit(PublicRouteInput {
        id: ROUTE_ID.to_owned(),
        hostname: "api.example.com".to_owned(),
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

/// The gateway bind address of ordinal `ordinal` in second octet `octet`.
pub(super) const fn bind_address(octet: u8, ordinal: u16) -> Ipv4Addr {
    Ipv4Addr::new(127, octet, (ordinal / 250) as u8, (ordinal % 250) as u8 + 1)
}

/// The Service frontend address of ordinal `ordinal` in second octet `octet`.
pub(super) const fn frontend_address(octet: u8, ordinal: u16) -> Ipv4Addr {
    Ipv4Addr::new(127, octet, 200 + (ordinal / 250) as u8, (ordinal % 250) as u8 + 1)
}

// ---------------------------------------------------------------------------
// One production composition
// ---------------------------------------------------------------------------

/// The real owner composition: `UnboundGatewayBuilder::new` ->
/// `into_builder` -> `GatewayBuilder::start_with(deps, admission)` over a real
/// redb Route store, `SimClock` and `SimObservationStore`, with the four
/// driver-local port doubles and the injected `Arc<GatewayAdmission>`.
pub(super) struct Composition {
    pub(super) control: GatewayControl,
    pub(super) handle: GatewayHandle,
    pub(super) admission: Arc<GatewayAdmission>,
    pub(super) probes: Arc<HeldProbes>,
    pub(super) connect: Arc<HeldConnectDouble>,
    pub(super) clock: Arc<SimClock>,
    _intent: Arc<LocalIntentStore>,
    _site: Site,
    _frontend: TcpListener,
}

impl Composition {
    /// Compose a fresh gateway on `127.<octet>.<hi>.<lo>:443` for `ordinal`.
    pub(super) async fn start(octet: u8, ordinal: u16) -> Self {
        let site = Site::new();
        let intent = Arc::new(LocalIntentStore::open(&site.intent_path).expect("redb Route store"));
        let frontend_ip = frontend_address(octet, ordinal);
        let vip_view = persist_api_service(&intent, frontend_ip).await;
        let frontend = TcpListener::bind((frontend_ip, SERVICE_PORT)).expect("frontend listener");
        frontend.set_nonblocking(true).expect("nonblocking frontend listener");
        let observations = Arc::new(SimObservationStore::single_peer(
            NodeId::new("gateway-node").expect("node ID"),
            54,
        ));
        let clock = Arc::new(SimClock::new());
        let intent_port: Arc<dyn IntentStore> = intent.clone();
        let observation_port: Arc<dyn ObservationStore> = observations;
        let clock_port: Arc<dyn Clock> = clock.clone();
        let unbound = UnboundGatewayBuilder::new(
            site.config(bind_address(octet, ordinal)),
            intent_port,
            observation_port,
            vip_view,
            Arc::new(PublicCertifiedKeyAeadCodec::new(custody_kek())),
            clock_port,
        )
        .await
        .expect("gateway builder");
        let (builder, seal) = unbound.into_builder();

        let limits = GatewayLimits::first_slice();
        let budget = GatewayAdmissionBudget::new(Arc::new(limits.clone()));
        let admission = GatewayAdmission::new(budget);
        let probes = HeldProbes::new();
        let connect = HeldConnectDouble::new();
        let dependencies = GatewayRuntimeDependencies::new(
            limits,
            connect.clone(),
            Arc::new(HeldClientMtls::new(seal, Arc::clone(&probes))),
            CurrentIdentity::new(),
            Arc::new(UnavailableRelay),
        );
        let (handle, control) =
            builder.start_with(dependencies, Arc::clone(&admission)).await.into_parts();
        Self {
            control,
            handle,
            admission,
            probes,
            connect,
            clock,
            _intent: intent,
            _site: site,
            _frontend: frontend,
        }
    }

    /// Graceful shutdown; releases the redb file and the bound address.
    pub(super) async fn stop(self) -> GatewayShutdownReport {
        self.handle.shutdown(GRACE).await
    }
}

/// A process-wide ordinal source, so two compositions of one process never
/// share a bind address.
pub(super) fn next_ordinal(counter: &AtomicU64) -> u16 {
    u16::try_from(counter.fetch_add(1, Ordering::SeqCst)).expect("ordinal space")
}

// ---------------------------------------------------------------------------
// Waiting on the owner (paused single-thread runtime)
// ---------------------------------------------------------------------------

/// Consecutive identical status reads that mean "the owner is idle".
const QUIET_POLLS: u32 = 16;
/// Polls before a wait gives up.
const POLL_CAP: u32 = 4_000;

/// Let every other task of the single-thread runtime run. The owner reads
/// time from the injected `SimClock`, never from tokio, so the paused tokio
/// clock only paces this wait: the 1 ms sleep returns once the runtime is
/// idle, which also gives the I/O driver and the blocking pool a turn.
async fn pause() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(1)).await;
}

async fn read_status(control: &GatewayControl) -> Option<GatewayApplicationStatusRowV1> {
    control.application_status().await.expect("Application status read")
}

/// Yield until `application_status()` has not changed for `QUIET_POLLS`
/// consecutive reads, and return it.
pub(super) async fn settle(
    control: &GatewayControl,
) -> Result<Option<GatewayApplicationStatusRowV1>, String> {
    let mut last = read_status(control).await;
    let mut quiet = 0;
    for _ in 0..POLL_CAP {
        pause().await;
        let now = read_status(control).await;
        if now == last {
            quiet += 1;
            if quiet >= QUIET_POLLS {
                return Ok(now);
            }
        } else {
            quiet = 0;
            last = now;
        }
    }
    Err(format!("the owner never went idle; last status {last:?}"))
}

/// Yield until `predicate` holds for the status read and `probes`, and return
/// that status.
pub(super) async fn wait_until(
    control: &GatewayControl,
    probes: &HeldProbes,
    what: &str,
    predicate: impl Fn(Option<&GatewayApplicationStatusRowV1>, &HeldProbes) -> bool,
) -> Result<Option<GatewayApplicationStatusRowV1>, String> {
    let mut last = None;
    for _ in 0..POLL_CAP {
        let status = read_status(control).await;
        if predicate(status.as_ref(), probes) {
            return Ok(status);
        }
        last = status;
        pause().await;
    }
    Err(format!("timed out waiting for {what}; last status {last:?}"))
}
