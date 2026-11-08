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
//! # Held-gate capability (DG-9, R-65)
//!
//! Route activation does not depend on backend health or backend
//! authentication (R-65): the per-candidate gate the owner awaits checks only
//! gateway-local facts (custody, candidate validity, the relay probe, the
//! connect dataplane's sentinel `probe` and the Gateway Identity). It makes no
//! connection to the Route's Service and calls no `open_upstream`,
//! `take_receipt`, `selected_backend_identity` or
//! `GatewayClientMtls::authenticate`. The gate therefore parks at an awaited
//! operation it owns, never at `authenticate`. A gate parks at exactly one
//! point ([`ParkPoint`]):
//!
//! - the first gate of a boot parks inside `GatewayConnectDataplane::probe`,
//!   the once-per-boot proof the owner latches. That wait is not cancellable:
//!   a gate the owner has superseded stays parked there until it is released,
//!   and then completes a real late result that the gate itself discards;
//! - every later gate parks at `GatewayIdentityLifecycleControl::ensure_current`,
//!   the one await every staged gate performs. The owner cancels that wait
//!   when it supersedes the gate, so only a live staged gate stays parked.
//!
//! A gate released usable at the sentinel probe passes its own identity wait at
//! once (the double recognises the gate's task), so one gate is one entry and
//! one verdict:
//!
//! 1. each staged gate parks as its own entry ([`HeldProbes::park`]);
//! 2. one entry is released by the [`HeldGate`] it was given when it parked,
//!    never by arrival position ([`HeldProbes::release`]);
//! 3. a release carries a typed outcome: [`ProbeOutcome::Usable`] lets the gate
//!    go on to its next gateway-local check, [`ProbeOutcome::Unusable`] returns the
//!    port's typed unavailable error (`GatewayConnectDataplaneError::Unavailable`
//!    at the connect probe, `GatewayIdentityLifecycleError::ReconcilerUnavailable`
//!    at the identity wait), which the gate reports as a gateway-local failure;
//! 4. the number of entries held ([`HeldProbes::held_count`]) and the number of
//!    connect probes that have returned ([`HeldProbes::connect_returned`]) are
//!    observable.
//!
//! An entry is removed when it is released and when its waiter is dropped (the
//! owner cancels a superseded gate's identity wait), so releasing an entry
//! whose waiter is gone or one already released returns
//! [`ReleaseOutcome::Absent`], a no-op.
//!
//! The client double answers `authenticate` with the port's typed unavailable
//! error: the gate does not reach it under R-65, and no request is driven on
//! these paths.
//!
//! # Bind-address table (R-6)
//!
//! Every composition that reaches promotion binds its own `127.0.0.0/8`
//! address on TCP/443: `127.<octet>.<hi>.<lo>` for ordinal `n` with
//! `hi = n / 250`, `lo = n % 250 + 1`; the Service frontend address (the
//! Service's own address, with a listening socket on port 8080; the gate makes
//! no connection to it, R-65) is `127.<octet>.<200 + hi>.<lo>`. The integration binary owns second octets
//! 41..=62.
//!
//! | octet | owner |
//! |---|---|
//! | 63 | QC-2 `replace_valid` (`quint_gateway_application`), one ordinal per composition |
//! | 64 | S-PIG-73 (`probe_completion_acceptance`) |
//! | 65 | QC-2 `route_withdraw_drain` |
//! | 66 | QC-2 `late_probe_after_supersession` |
//! | 67 | QC-2 randomized traces |
//! | 68..=71 | admission refusals over leases (`admission_acceptance`), one per test |
//! | 98 | QC-2 `failed_candidate_not_restaged` |
#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    reason = "CONTRACT_SHAPE metadata, fixture preconditions and the address arithmetic of the table above"
)]

use std::collections::{BTreeMap, BTreeSet};
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
use overdrive_core::gateway_identity::{
    GatewayIdentityEpoch, GatewayIdentityFacts, GatewayIdentityIssueRecord,
};
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
    GatewayAdmission, GatewayAdmissionBudget, GatewayHandle, GatewayRouteCommands,
    GatewayRuntimeDependencies, GatewayShutdownReport, GatewayStatusReads, UnboundGatewayBuilder,
};
use crate::ports::{
    AcceptedPublicLeg, GatewayClientMtls, GatewayClientMtlsError, GatewayConnectCleanup,
    GatewayConnectCleanupSweep, GatewayConnectDataplane, GatewayConnectDataplaneError,
    GatewayConnectIntent, GatewayIdentityLifecycleControl, GatewayIdentityLifecycleError,
    GatewayRelay, GatewaySelectionReceipt, GatewayUpstream, HeadTerminator, KtlsReadHalf,
    KtlsWriteHalf, LegReadError, LegWriteError, PreparedUpstream, PublicHandshakeError,
    ReadDeadlines, RelayDeadlines, RelayError, RelayProbeError, ThroughBytes,
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

/// The typed outcome a release delivers to a parked gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProbeOutcome {
    /// `ensure_current` returns the current facts: the gate goes on to its
    /// next gateway-local check.
    Usable,
    /// `ensure_current` returns the typed `ReconcilerUnavailable`: the gate
    /// reports `Identity(..)`, a gateway-local failure.
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

/// The identity of one parked gate, assigned when it parks. A held gate is
/// addressed by this value, never by its position among the parked entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct HeldGate(u64);

/// Where a gate parks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParkPoint {
    /// Inside `GatewayConnectDataplane::probe`: not cancellable.
    ConnectProbe,
    /// At `GatewayIdentityLifecycleControl::ensure_current`: cancelled by the
    /// owner when it supersedes the gate.
    IdentityWait,
}

struct HeldEntry {
    sender: oneshot::Sender<ProbeOutcome>,
    point: ParkPoint,
}

#[derive(Default)]
struct HeldState {
    next_gate: u64,
    entries: BTreeMap<HeldGate, HeldEntry>,
    /// Gate tasks released usable at the connect probe: their identity wait
    /// passes at once.
    passed: BTreeSet<tokio::task::Id>,
    /// Connect probes that returned after having been parked.
    connect_returned: u32,
}

/// Gates parked at the awaited operations they own, each addressed by the
/// [`HeldGate`] it was given.
#[derive(Default)]
pub(super) struct HeldProbes {
    state: Mutex<HeldState>,
}

/// Removes its entry when the parked waiter is dropped.
struct ParkGuard {
    probes: Arc<HeldProbes>,
    gate: HeldGate,
}

impl Drop for ParkGuard {
    fn drop(&mut self) {
        self.probes.state.lock().expect("held probes").entries.remove(&self.gate);
    }
}

impl HeldProbes {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn park(self: &Arc<Self>, point: ParkPoint) -> (ParkGuard, oneshot::Receiver<ProbeOutcome>) {
        let (sender, receiver) = oneshot::channel();
        let mut state = self.state.lock().expect("held probes");
        state.next_gate += 1;
        let gate = HeldGate(state.next_gate);
        state.entries.insert(gate, HeldEntry { sender, point });
        drop(state);
        (ParkGuard { probes: Arc::clone(self), gate }, receiver)
    }

    /// Number of entries currently parked.
    pub(super) fn held_count(&self) -> usize {
        self.state.lock().expect("held probes").entries.len()
    }

    /// Parked gates in the order they parked (oldest first).
    pub(super) fn held_gates(&self) -> Vec<HeldGate> {
        self.state.lock().expect("held probes").entries.keys().copied().collect()
    }

    /// Where `gate` is parked, or `None` once it is released or its waiter is
    /// gone.
    pub(super) fn park_point(&self, gate: HeldGate) -> Option<ParkPoint> {
        self.state.lock().expect("held probes").entries.get(&gate).map(|entry| entry.point)
    }

    /// Connect probes that have returned after having been parked.
    pub(super) fn connect_returned(&self) -> u32 {
        self.state.lock().expect("held probes").connect_returned
    }

    /// Complete the entry of `gate` with `outcome`.
    pub(super) fn release(&self, gate: HeldGate, outcome: ProbeOutcome) -> ReleaseOutcome {
        let removed = self.state.lock().expect("held probes").entries.remove(&gate);
        match removed.map(|entry| entry.sender.send(outcome)) {
            Some(Ok(())) => ReleaseOutcome::Released,
            Some(Err(_)) | None => ReleaseOutcome::Absent,
        }
    }

    /// The parked connect probe returned `outcome` to the current gate task.
    fn connect_probe_returned(&self, usable: bool) {
        let mut state = self.state.lock().expect("held probes");
        state.connect_returned += 1;
        if usable && let Some(task) = tokio::task::try_id() {
            state.passed.insert(task);
        }
    }

    /// Whether the current gate task was released usable at the connect probe.
    fn identity_already_passed(&self) -> bool {
        tokio::task::try_id()
            .is_some_and(|task| self.state.lock().expect("held probes").passed.contains(&task))
    }
}

// ---------------------------------------------------------------------------
// GatewayClientMtls double
// ---------------------------------------------------------------------------

/// Answers every `authenticate` with the port's typed unavailable error. Under
/// R-65 the gate never calls it, and the request path is not driven on the QC-2
/// path.
pub(super) struct UnavailableClientMtls;

#[async_trait]
impl GatewayClientMtls for UnavailableClientMtls {
    async fn authenticate(
        &self,
        connected: OwnedFd,
        _expected_peer: SpiffeId,
        _deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
        drop(connected);
        Err(GatewayClientMtlsError::IdentityAbsent)
    }
}

// ---------------------------------------------------------------------------
// GatewayConnectDataplane double
// ---------------------------------------------------------------------------

/// The one backend every receipt of the double names: the gate performs no
/// backend resolution (R-65), so per-socket backends are not needed.
fn selected_backend() -> BackendId {
    BackendId::new(1).expect("backend ID")
}

#[derive(Default)]
struct ConnectState {
    next_cookie: u64,
    probe_calls: u32,
    registered: BTreeMap<SocketCookie, GatewayConnectIntent>,
    pending_receipts: BTreeSet<SocketCookie>,
}

/// Answers every `open_upstream` with a fresh unconnected socket and a freshly
/// minted nonzero cookie; `take_receipt` answers `Selected`. The gate calls
/// neither (R-65): they exist because the port requires them, for a request
/// path these compositions do not drive.
/// The first `probe` call of the boot parks (the owner latches the proof, so no
/// later candidate calls it again) until its entry is released.
pub(super) struct HeldConnectDouble {
    probes: Arc<HeldProbes>,
    state: Mutex<ConnectState>,
}

impl HeldConnectDouble {
    pub(super) fn new(probes: Arc<HeldProbes>) -> Arc<Self> {
        Arc::new(Self { probes, state: Mutex::new(ConnectState::default()) })
    }
}

impl GatewayConnectCleanup for HeldConnectDouble {
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

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        let state = self.state.lock().expect("connect state");
        Ok(u32::try_from(state.registered.len()).expect("intent count"))
    }

    fn live_receipts(
        &self,
    ) -> Result<BTreeMap<SocketCookie, GatewaySelectionReceipt>, GatewayConnectDataplaneError> {
        let state = self.state.lock().expect("connect state");
        Ok(state
            .pending_receipts
            .iter()
            .filter_map(|cookie| {
                let intent = state.registered.get(cookie)?;
                Some((
                    *cookie,
                    GatewaySelectionReceipt::Selected {
                        socket_cookie: intent.socket_cookie,
                        service_key: intent.service_key,
                        backend_id: selected_backend(),
                    },
                ))
            })
            .collect())
    }
}

#[async_trait]
impl GatewayConnectDataplane for HeldConnectDouble {
    async fn probe(
        &self,
        _deadline: Instant,
        _clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        let first = {
            let mut state = self.state.lock().expect("connect state");
            state.probe_calls += 1;
            state.probe_calls == 1
        };
        if !first {
            return Ok(());
        }
        let (_parked, outcome) = self.probes.park(ParkPoint::ConnectProbe);
        let outcome = outcome.await;
        let usable = matches!(outcome, Ok(ProbeOutcome::Usable));
        self.probes.connect_probe_returned(usable);
        if usable { Ok(()) } else { Err(GatewayConnectDataplaneError::Unavailable) }
    }

    fn open_upstream(
        &self,
        key: ServiceKey,
    ) -> Result<PreparedUpstream, GatewayConnectDataplaneError> {
        let socket = tokio::net::TcpSocket::new_v4()
            .map_err(|error| GatewayConnectDataplaneError::SocketCreate(error.kind()))?;
        let mut state = self.state.lock().expect("connect state");
        state.next_cookie += 1;
        let cookie = SocketCookie::new(state.next_cookie).expect("nonzero cookie");
        let intent = GatewayConnectIntent { socket_cookie: cookie, service_key: key };
        state.registered.insert(cookie, intent.clone());
        state.pending_receipts.insert(cookie);
        drop(state);
        Ok(PreparedUpstream::new(socket, intent))
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("connect state");
        let Some(registered) = state.registered.get(&intent.socket_cookie).cloned() else {
            return Err(GatewayConnectDataplaneError::IntentMissing);
        };
        if !state.pending_receipts.remove(&intent.socket_cookie) {
            return Err(GatewayConnectDataplaneError::ReceiptMissing);
        }
        drop(state);
        Ok(GatewaySelectionReceipt::Selected {
            socket_cookie: registered.socket_cookie,
            service_key: registered.service_key,
            backend_id: selected_backend(),
        })
    }

    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        Err(GatewayConnectDataplaneError::BackendIdentityMissing { backend_id })
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

/// Parks every `ensure_current` of a gate that did not already pass the connect
/// probe as its own entry (the gate's identity wait, DG-9) and answers it with
/// the current facts or the typed `ReconcilerUnavailable` when released; `current` and `subscribe` answer
/// immediately with current facts; `disable_after_drain` is a synchronous clear
/// (R-4).
pub(super) struct HeldIdentity {
    facts: GatewayIdentityFacts,
    sender: watch::Sender<Option<GatewayIdentityFacts>>,
    probes: Arc<HeldProbes>,
}

impl HeldIdentity {
    pub(super) fn new(probes: Arc<HeldProbes>) -> Arc<Self> {
        let facts = gateway_identity_facts();
        let (sender, _) = watch::channel(Some(facts.clone()));
        Arc::new(Self { facts, sender, probes })
    }
}

#[async_trait]
impl GatewayIdentityLifecycleControl for HeldIdentity {
    async fn ensure_current(
        &self,
        _deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError> {
        if self.probes.identity_already_passed() {
            return Ok(self.facts.clone());
        }
        let (_parked, outcome) = self.probes.park(ParkPoint::IdentityWait);
        match outcome.await {
            Ok(ProbeOutcome::Usable) => Ok(self.facts.clone()),
            Ok(ProbeOutcome::Unusable) | Err(_) => {
                Err(GatewayIdentityLifecycleError::ReconcilerUnavailable)
            }
        }
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
    ) -> Result<(), LegReadError> {
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
    pub(super) routes: GatewayRouteCommands,
    pub(super) status: GatewayStatusReads,
    pub(super) handle: GatewayHandle,
    pub(super) admission: Arc<GatewayAdmission>,
    pub(super) probes: Arc<HeldProbes>,
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
        let node = NodeId::new("gateway-node").expect("node ID");
        let observations = Arc::new(SimObservationStore::single_peer(node.clone(), 54));
        let clock = Arc::new(SimClock::new());
        let intent_port: Arc<dyn IntentStore> = intent.clone();
        let observation_port: Arc<dyn ObservationStore> = observations;
        let clock_port: Arc<dyn Clock> = clock.clone();
        let unbound = UnboundGatewayBuilder::new(
            node,
            site.config(bind_address(octet, ordinal)),
            intent_port,
            observation_port,
            vip_view,
            Arc::new(PublicCertifiedKeyAeadCodec::new(custody_kek())),
            clock_port,
        )
        .await
        .expect("gateway builder");
        let routes = unbound.route_commands();
        let status = unbound.status_reads();
        let (builder, _seal) = unbound.into_builder();

        let limits = GatewayLimits::first_slice();
        let budget = GatewayAdmissionBudget::new(Arc::new(limits.clone()));
        let admission = GatewayAdmission::new(budget);
        let probes = HeldProbes::new();
        let connect = HeldConnectDouble::new(Arc::clone(&probes));
        let dependencies = GatewayRuntimeDependencies::new(
            limits,
            connect.clone(),
            Arc::new(UnavailableClientMtls),
            HeldIdentity::new(Arc::clone(&probes)),
            Arc::new(UnavailableRelay),
        );
        let handle = builder.start_with(dependencies, Arc::clone(&admission)).await.into_parts();
        Self {
            routes,
            status,
            handle,
            admission,
            probes,
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

async fn read_status(status: &GatewayStatusReads) -> Option<GatewayApplicationStatusRowV1> {
    status.application_status().await.expect("Application status read")
}

/// Yield until `application_status()` has not changed for `QUIET_POLLS`
/// consecutive reads, and return it.
pub(super) async fn settle(
    status: &GatewayStatusReads,
) -> Result<Option<GatewayApplicationStatusRowV1>, String> {
    let mut last = read_status(status).await;
    let mut quiet = 0;
    for _ in 0..POLL_CAP {
        pause().await;
        let now = read_status(status).await;
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
    status: &GatewayStatusReads,
    probes: &HeldProbes,
    what: &str,
    predicate: impl Fn(Option<&GatewayApplicationStatusRowV1>, &HeldProbes) -> bool,
) -> Result<Option<GatewayApplicationStatusRowV1>, String> {
    let mut last = None;
    for _ in 0..POLL_CAP {
        let observed = read_status(status).await;
        if predicate(observed.as_ref(), probes) {
            return Ok(observed);
        }
        last = observed;
        pause().await;
    }
    Err(format!("timed out waiting for {what}; last status {last:?}"))
}
