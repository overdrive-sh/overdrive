//! Gateway Application integration scenarios over the real redb `LocalIntentStore`
//! (S-PIG-21 through S-PIG-27, 33, 46, 50, 51, 54, 66 and 68).
//!
//! Every composition goes through the production `UnboundGatewayBuilder` ->
//! `GatewayBuilder` -> `StartedGateway` path. The driven ports are test-local
//! doubles pinned to the DESIGN shapes: `GatewayConnectDataplane`
//! (`open_upstream` mints the cookie, registers the intent and returns a real
//! unconnected `TcpSocket`), `GatewayClientMtls` (parks by minted peer identity,
//! DG-9), `GatewayIdentityLifecycleControl` (disable is a synchronous clear, R-4)
//! and `GatewayRelay` (the probe succeeds; every other method is a typed
//! unavailable error, because no public client socket is driven in this file).
//!
//! # Bind-address table (R-6)
//!
//! A composition that reaches promotion binds its own `127.0.0.0/8` address on
//! TCP/443. nextest gives each test its own process, so every test derives its
//! addresses from its own second octet `T` below; `Slot::bind(ordinal)` is
//! `127.T.hi.lo` (`hi = ordinal / 250`, `lo = ordinal % 250 + 1`) and
//! `Slot::vip(ordinal)` is `127.T.(200 + hi).lo` (the Service frontend, with a
//! listening socket on port 8080 so the connector's real `connect(2)` completes).
//! This file owns second octets 41..=62 and 180..=190; no other test file may reuse
//! them.
//!
//! | `T` | test |
//! |---|---|
//! | 41 | Route declare / replay / replace / withdraw (S-PIG-21) |
//! | 42 | competing Route ID (S-PIG-22) |
//! | 43 | withdraw with a different Route ID (S-PIG-23) |
//! | 44 | two concurrent claims, 256 rounds (S-PIG-24) |
//! | 45 | status redaction (S-PIG-25) |
//! | 46 | shutdown joins the admitted user first (S-PIG-26) |
//! | 47 | identity error retained by shutdown (S-PIG-26) |
//! | 48 | manual key refusals (S-PIG-27) |
//! | 49 | withdraw from Empty (S-PIG-33) |
//! | 50 | cardinality zero / one / many (S-PIG-33) |
//! | 51 | install, replay, replace (S-PIG-46) |
//! | 52 | mismatched replacement (S-PIG-46) |
//! | 53 | expired replacement (S-PIG-46) |
//! | 54 | expiry and reopen (S-PIG-46) |
//! | 55 | dataplane error crosses its owner operation (S-PIG-50) |
//! | 56 | dataplane error collapses in status (S-PIG-50) |
//! | 57 | identity error crosses at the owner (S-PIG-51) |
//! | 58 | identity error collapses in status (S-PIG-51) |
//! | 59 | custody replay and restart (S-PIG-54) |
//! | 60 | identical Route re-apply (S-PIG-54) |
//! | 61 | watch recovery after a lag (S-PIG-66) |
//! | 62 | superseded probe (S-PIG-68) |
//! | 180 | identity that never issues, then recovers (O-SIF-G1, G2) |
//! | 181 | seeded failure / success / restart sequence (O-SIF-G1) |
//! | 182 | restart continues the count (O-SIF-G3) |
//! | 183 | prior-row read error is retried (O-SIF-G3) |
//! | 184 | held identity past expiry (O-SIF-G5) |
//! | 185 | boot identity timeout (O-SIF-G5) |
//! | 186 | failing status write loses nothing (O-SIF-G6) |
//! | 187 | restart over an undecodable prior status row (S-PIG-129) |
//! | 188 | no status row before a Route exists (S-PIG-130) |
//! | 189 | first status row after a Route carries what was held (S-PIG-130) |
//! | 190 | prior status row carried with no Route (S-PIG-130) |

#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::significant_drop_tightening,
    clippy::missing_const_for_fn,
    clippy::print_stderr,
    reason = "CONTRACT_SHAPE metadata, long scenario bodies and fixture preconditions"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::{NonZeroU64, NonZeroUsize};
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt as _;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, ServiceV1, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::ca::issue_failure::{SvidIssueFailure, SvidIssueFailureCause};
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::gateway_identity::{
    GatewayIdentityEpoch, GatewayIdentityFacts, GatewayIdentityIssueRecord,
};
use overdrive_core::id::{BackendId, CertSerial, ContentHash, NodeId, ServiceVip, SpiffeId};
use overdrive_core::public_ingress::{
    CertifiedKeyFailure, CertifiedKeyUsabilityFailure, GatewayApplicationStatusRowV1,
    GatewayApplicationUnavailableCause, GatewayConnectPathStatus, GatewayIdentityStatus,
    GatewayListenerStatus, PublicCertifiedKeyId, PublicCertifiedKeyStatusRowV1,
    PublicCertifiedKeyStatusState, PublicRouteInput, PublicRouteSetV1, Route, RouteApplyOutcome,
    RouteId, RouteWithdrawOutcome, ServiceKey, ServiceListenerReferenceInput, SocketCookie,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::intent_store::{IntentStore, IntentSubscriptionEvent, TxnOp};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_gateway::application::{
    GatewayBootError, GatewayCleanupFailure, GatewayConnectSweepFailure, GatewayHandle,
    GatewayRouteCommands, GatewayRuntimeDependencies, GatewayShutdownReport, GatewayStatusReads,
    UnboundGatewayBuilder,
};
use overdrive_gateway::certified_key::PublicCertifiedKeyError;
use overdrive_gateway::ports::{
    AcceptedPublicLeg, GatewayClientMtls, GatewayClientMtlsError, GatewayConnectCleanup,
    GatewayConnectCleanupSweep, GatewayConnectDataplane, GatewayConnectDataplaneError,
    GatewayConnectIntent, GatewayIdentityLifecycleControl, GatewayIdentityLifecycleError,
    GatewayRelay, GatewaySelectionReceipt, GatewayUpstream, GatewayUpstreamSeal, HeadTerminator,
    KtlsLeg, KtlsReadHalf, KtlsWriteHalf, LegReadError, LegWriteError, PreparedUpstream,
    PublicHandshakeError, ReadDeadlines, RelayDeadlines, RelayError, RelayProbeError, ThroughBytes,
};
use overdrive_gateway::route_set::PublicRouteSetError;
use overdrive_gateway::runtime::GatewayLimits;
use overdrive_gateway::{GatewayConfig, ManualCertifiedKeyConfig};
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::kek::SimKek;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::read_ports::SimServiceVipView;
use overdrive_store_local::{LocalIntentStore, LocalObservationStore};
use tempfile::TempDir;
use tokio::sync::{Barrier, Notify, watch};

// ---------------------------------------------------------------------------
// Constants and per-test address slots (R-6)
// ---------------------------------------------------------------------------

/// Key of the singleton Route Set record.
const ROUTE_SET_KEY: &str = "public-ingress/route-set";
/// Prefix of the protected certified-key custody records.
const CUSTODY_PREFIX: &[u8] = b"public-ingress/certified-key/";
const SERVICE_PORT: u16 = 8080;
const GRACE: Duration = Duration::from_secs(5);
/// Real-time ceiling for any wait on an owner task.
const WAIT: Duration = Duration::from_secs(10);
/// Simulated-time search bound for the manual source's private maintenance
/// cadence. The cadence itself is not pinned by the DESIGN, so the tests advance
/// the injected clock in steps until the awaited transition appears.
const MAINTENANCE_STEP: Duration = Duration::from_secs(30);
const MAINTENANCE_STEPS: u32 = 240;

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

const ROUTE_LIFECYCLE: Slot = Slot(41);
const COMPETING_ROUTE: Slot = Slot(42);
const WRONG_ID_WITHDRAW: Slot = Slot(43);
const CONCURRENT_CLAIMS: Slot = Slot(44);
const STATUS_REDACTION: Slot = Slot(45);
const SHUTDOWN_JOIN: Slot = Slot(46);
const KEY_REFUSALS: Slot = Slot(48);
const WITHDRAW_FROM_EMPTY: Slot = Slot(49);
const CARDINALITY: Slot = Slot(50);
const KEY_INSTALL_REPLAY_REPLACE: Slot = Slot(51);
const KEY_MISMATCH: Slot = Slot(52);
const KEY_EXPIRED_REPLACEMENT: Slot = Slot(53);
const KEY_EXPIRY_REOPEN: Slot = Slot(54);
const DATAPLANE_CROSSES: Slot = Slot(55);
const DATAPLANE_COLLAPSES: Slot = Slot(56);
const IDENTITY_CROSSES: Slot = Slot(57);
const IDENTITY_COLLAPSES: Slot = Slot(58);
const CUSTODY_REPLAY: Slot = Slot(59);
const ROUTE_REAPPLY: Slot = Slot(60);
const WATCH_RECOVERY: Slot = Slot(61);
const SUPERSEDED_PROBE: Slot = Slot(62);
const ISSUE_ABSENT_THEN_RECOVERS: Slot = Slot(180);
const ISSUE_SEQUENCE: Slot = Slot(181);
const ISSUE_RESTART: Slot = Slot(182);
const ISSUE_READ_RETRY: Slot = Slot(183);
const ISSUE_EXPIRY: Slot = Slot(184);
const ISSUE_BOOT_TIMEOUT: Slot = Slot(185);
const ISSUE_WRITE_RETRY: Slot = Slot(186);
const ISSUE_UNDECODABLE_PRIOR: Slot = Slot(187);
const NO_ROW_BEFORE_ROUTE: Slot = Slot(188);
const FIRST_ROW_AFTER_ROUTE: Slot = Slot(189);
const PRIOR_ROW_WITHOUT_ROUTE: Slot = Slot(190);

// ---------------------------------------------------------------------------
// Event log shared by the doubles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    AcceptParked,
    AcceptReleased,
    IdentityCleared,
}

#[derive(Default)]
struct EventLog(Mutex<Vec<Event>>);

impl EventLog {
    fn record(&self, event: Event) {
        self.0.lock().expect("event log").push(event);
    }

    fn snapshot(&self) -> Vec<Event> {
        self.0.lock().expect("event log").clone()
    }

    fn position(&self, event: Event) -> Option<usize> {
        self.snapshot().iter().position(|seen| *seen == event)
    }
}

// ---------------------------------------------------------------------------
// GatewayConnectDataplane double
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DataplaneOp {
    Probe,
    OpenUpstream,
    TakeReceipt,
    SelectedBackendIdentity,
    Cleanup,
    CleanupAll,
    LiveIntentCount,
    LiveReceipts,
}

/// Whether the shared Service state holds a programmed backend: a programmed
/// double answers `Selected` with a minted `BackendId`, an unprogrammed one
/// answers the ordinary `NoBackend` of "no outer entry".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backends {
    NoneProgrammed,
    Programmed,
}

#[derive(Default)]
struct DataplaneState {
    next_cookie: u64,
    next_backend: u32,
    registered: BTreeMap<SocketCookie, (GatewayConnectIntent, BackendId)>,
    pending_receipts: BTreeSet<SocketCookie>,
    calls: Vec<DataplaneOp>,
    injected: Vec<GatewayConnectDataplaneError>,
    opened: Vec<GatewayConnectIntent>,
}

struct ScriptedDataplane {
    backends: Backends,
    fault: Option<(DataplaneOp, GatewayConnectDataplaneError)>,
    state: Mutex<DataplaneState>,
}

fn backend_identity(backend_id: BackendId) -> SpiffeId {
    SpiffeId::new(&format!("spiffe://overdrive.local/workload/api/alloc/api-{}", backend_id.get()))
        .expect("backend SPIFFE ID")
}

impl ScriptedDataplane {
    fn new(
        backends: Backends,
        fault: Option<(DataplaneOp, GatewayConnectDataplaneError)>,
    ) -> Arc<Self> {
        Arc::new(Self { backends, fault, state: Mutex::new(DataplaneState::default()) })
    }

    fn fault_at(
        &self,
        state: &mut DataplaneState,
        op: DataplaneOp,
    ) -> Option<GatewayConnectDataplaneError> {
        state.calls.push(op);
        let (fault_op, error) = self.fault.as_ref()?;
        if *fault_op != op {
            return None;
        }
        state.injected.push(error.clone());
        Some(error.clone())
    }

    fn calls(&self) -> Vec<DataplaneOp> {
        self.state.lock().expect("dataplane state").calls.clone()
    }

    fn injected(&self) -> Vec<GatewayConnectDataplaneError> {
        self.state.lock().expect("dataplane state").injected.clone()
    }

    fn opened(&self) -> Vec<GatewayConnectIntent> {
        self.state.lock().expect("dataplane state").opened.clone()
    }

    /// The double's own ground truth: registered intents and untaken receipts.
    fn residue(&self) -> (usize, usize) {
        let state = self.state.lock().expect("dataplane state");
        (state.registered.len(), state.pending_receipts.len())
    }
}

impl GatewayConnectCleanup for ScriptedDataplane {
    fn cleanup(&self, intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("dataplane state");
        if let Some(error) = self.fault_at(&mut state, DataplaneOp::Cleanup) {
            return Err(error);
        }
        state.registered.remove(&intent.socket_cookie);
        state.pending_receipts.remove(&intent.socket_cookie);
        Ok(())
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("dataplane state");
        if let Some(error) = self.fault_at(&mut state, DataplaneOp::CleanupAll) {
            return Err(error);
        }
        let removed_intents = u32::try_from(state.registered.len()).expect("intent count");
        let removed_receipts = u32::try_from(state.pending_receipts.len()).expect("receipt count");
        state.registered.clear();
        state.pending_receipts.clear();
        Ok(GatewayConnectCleanupSweep { removed_intents, removed_receipts })
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("dataplane state");
        if let Some(error) = self.fault_at(&mut state, DataplaneOp::LiveIntentCount) {
            return Err(error);
        }
        Ok(u32::try_from(state.registered.len()).expect("intent count"))
    }

    fn live_receipts(
        &self,
    ) -> Result<BTreeMap<SocketCookie, GatewaySelectionReceipt>, GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("dataplane state");
        if let Some(error) = self.fault_at(&mut state, DataplaneOp::LiveReceipts) {
            return Err(error);
        }
        Ok(state
            .pending_receipts
            .iter()
            .filter_map(|cookie| {
                let (intent, backend) = state.registered.get(cookie)?;
                Some((
                    *cookie,
                    match self.backends {
                        Backends::Programmed => GatewaySelectionReceipt::Selected {
                            socket_cookie: intent.socket_cookie,
                            service_key: intent.service_key,
                            backend_id: *backend,
                        },
                        Backends::NoneProgrammed => GatewaySelectionReceipt::NoBackend {
                            socket_cookie: intent.socket_cookie,
                            service_key: intent.service_key,
                        },
                    },
                ))
            })
            .collect())
    }
}

#[async_trait]
impl GatewayConnectDataplane for ScriptedDataplane {
    async fn probe(
        &self,
        _deadline: Instant,
        _clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("dataplane state");
        self.fault_at(&mut state, DataplaneOp::Probe).map_or(Ok(()), Err)
    }

    fn open_upstream(
        &self,
        key: ServiceKey,
    ) -> Result<PreparedUpstream, GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("dataplane state");
        if let Some(error) = self.fault_at(&mut state, DataplaneOp::OpenUpstream) {
            return Err(error);
        }
        let socket = tokio::net::TcpSocket::new_v4()
            .map_err(|error| GatewayConnectDataplaneError::SocketCreate(error.kind()))?;
        state.next_cookie += 1;
        state.next_backend += 1;
        let cookie = SocketCookie::new(state.next_cookie).expect("nonzero cookie");
        let backend = BackendId::new(state.next_backend).expect("backend ID");
        let intent = GatewayConnectIntent { socket_cookie: cookie, service_key: key };
        state.registered.insert(cookie, (intent.clone(), backend));
        state.pending_receipts.insert(cookie);
        state.opened.push(intent.clone());
        Ok(PreparedUpstream::new(socket, intent))
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("dataplane state");
        if let Some(error) = self.fault_at(&mut state, DataplaneOp::TakeReceipt) {
            return Err(error);
        }
        let Some((registered, backend)) = state.registered.get(&intent.socket_cookie).cloned()
        else {
            return Err(GatewayConnectDataplaneError::IntentMissing);
        };
        if !state.pending_receipts.remove(&intent.socket_cookie) {
            return Err(GatewayConnectDataplaneError::ReceiptMissing);
        }
        Ok(match self.backends {
            Backends::Programmed => GatewaySelectionReceipt::Selected {
                socket_cookie: registered.socket_cookie,
                service_key: registered.service_key,
                backend_id: backend,
            },
            Backends::NoneProgrammed => GatewaySelectionReceipt::NoBackend {
                socket_cookie: registered.socket_cookie,
                service_key: registered.service_key,
            },
        })
    }

    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        let mut state = self.state.lock().expect("dataplane state");
        if let Some(error) = self.fault_at(&mut state, DataplaneOp::SelectedBackendIdentity) {
            return Err(error);
        }
        Ok(backend_identity(backend_id))
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

#[derive(Default)]
struct IdentityState {
    ensure_calls: u32,
    observed: Vec<GatewayIdentityLifecycleError>,
}

struct ScriptedIdentity {
    facts: GatewayIdentityFacts,
    sender: watch::Sender<Option<GatewayIdentityFacts>>,
    ensure_error: Option<GatewayIdentityLifecycleError>,
    /// When set, `ensure_current` waits for the held facts instead of answering
    /// at once, as the production control does while the reconciler issues.
    wait_for_held: bool,
    state: Mutex<IdentityState>,
    /// The process-local issue-failure record the slot would hold.
    record: Mutex<Option<GatewayIdentityIssueRecord>>,
    events: Arc<EventLog>,
}

impl ScriptedIdentity {
    fn scripted(
        ensure_error: Option<GatewayIdentityLifecycleError>,
        absent_until_held: bool,
        events: Arc<EventLog>,
    ) -> Arc<Self> {
        let facts = gateway_identity_facts();
        let initial = (ensure_error.is_none() && !absent_until_held).then(|| facts.clone());
        let (sender, _) = watch::channel(initial);
        Arc::new(Self {
            facts,
            sender,
            ensure_error,
            wait_for_held: absent_until_held,
            state: Mutex::new(IdentityState::default()),
            record: Mutex::new(None),
            events,
        })
    }

    /// One confirmed failed issuance, exactly as the slot's
    /// `record_issue_failure` applies it: one more on the count, the latest
    /// failure replaces `last`, the held identity is untouched, and every
    /// `subscribe` receiver is woken with an unchanged item.
    fn record_failure(&self, failure: SvidIssueFailure) {
        {
            let mut record = self.record.lock().expect("issue record");
            let count = record.map_or(NonZeroU64::MIN, |prior| prior.count.saturating_add(1));
            *record = Some(GatewayIdentityIssueRecord { count, last: failure });
        }
        self.sender.send_modify(|_| {});
    }

    /// A successful audited hold of `facts`; the issue record is untouched.
    fn hold(&self, facts: GatewayIdentityFacts) {
        self.sender.send_replace(Some(facts));
    }

    fn ensure_calls(&self) -> u32 {
        self.state.lock().expect("identity state").ensure_calls
    }

    fn observed(&self) -> Vec<GatewayIdentityLifecycleError> {
        self.state.lock().expect("identity state").observed.clone()
    }
}

#[async_trait]
impl GatewayIdentityLifecycleControl for ScriptedIdentity {
    async fn ensure_current(
        &self,
        _deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError> {
        {
            let mut state = self.state.lock().expect("identity state");
            state.ensure_calls += 1;
            if let Some(error) = &self.ensure_error {
                state.observed.push(error.clone());
                return Err(error.clone());
            }
        }
        if self.wait_for_held {
            let mut receiver = self.sender.subscribe();
            loop {
                let held = receiver.borrow_and_update().clone();
                if let Some(held) = held {
                    return Ok(held);
                }
                if receiver.changed().await.is_err() {
                    return Err(GatewayIdentityLifecycleError::Closed);
                }
            }
        }
        Ok(self.facts.clone())
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        self.sender.borrow().clone()
    }

    fn issue_record(&self) -> Option<GatewayIdentityIssueRecord> {
        *self.record.lock().expect("issue record")
    }

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.sender.subscribe()
    }

    fn disable_after_drain(&self) {
        self.sender.send_replace(None);
        self.events.record(Event::IdentityCleared);
    }
}

// ---------------------------------------------------------------------------
// GatewayClientMtls double: parks by minted peer identity (DG-9)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct HoldState {
    /// Peers whose `authenticate` is currently parked, with their release latch.
    parked: BTreeMap<SpiffeId, Arc<Notify>>,
    /// Every `expected_peer` `authenticate` was called with, in call order.
    calls: Vec<SpiffeId>,
    /// Peers whose `authenticate` returned, with their outcome.
    returned: Vec<(SpiffeId, bool)>,
}

struct HoldRegistry {
    hold_next: AtomicU32,
    state: Mutex<HoldState>,
}

impl HoldRegistry {
    fn new(hold_first_calls: u32) -> Arc<Self> {
        Arc::new(Self {
            hold_next: AtomicU32::new(hold_first_calls),
            state: Mutex::new(HoldState::default()),
        })
    }

    fn parked_peers(&self) -> Vec<SpiffeId> {
        self.state.lock().expect("hold state").parked.keys().cloned().collect()
    }

    fn call_count(&self) -> usize {
        self.state.lock().expect("hold state").calls.len()
    }

    fn returned_count(&self) -> usize {
        self.state.lock().expect("hold state").returned.len()
    }

    /// Release the parked entry identified by `peer`; no-op for an entry whose
    /// waiter is already gone.
    fn release(&self, peer: &SpiffeId) {
        let latch = self.state.lock().expect("hold state").parked.get(peer).cloned();
        if let Some(latch) = latch {
            latch.notify_one();
        }
    }
}

struct ParkingMtls {
    seal: GatewayUpstreamSeal,
    registry: Arc<HoldRegistry>,
}

#[async_trait]
impl GatewayClientMtls for ParkingMtls {
    async fn authenticate(
        &self,
        connected: OwnedFd,
        expected_peer: SpiffeId,
        _deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
        drop(connected);
        let latch = {
            let mut state = self.registry.state.lock().expect("hold state");
            state.calls.push(expected_peer.clone());
            let must_park = self
                .registry
                .hold_next
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
                .is_ok();
            must_park.then(|| {
                let latch = Arc::new(Notify::new());
                state.parked.insert(expected_peer.clone(), Arc::clone(&latch));
                latch
            })
        };
        if let Some(latch) = latch {
            latch.notified().await;
            self.registry.state.lock().expect("hold state").parked.remove(&expected_peer);
        }
        self.registry.state.lock().expect("hold state").returned.push((expected_peer, true));
        Ok(self.seal.seal_authenticated(KtlsLeg::from_armed(Box::new(()), Box::new(()))))
    }
}

// ---------------------------------------------------------------------------
// GatewayRelay double
// ---------------------------------------------------------------------------

struct ScriptedRelay {
    events: Arc<EventLog>,
    park_accept: bool,
    accept_latch: Notify,
}

impl ScriptedRelay {
    fn new(events: Arc<EventLog>, park_accept: bool) -> Arc<Self> {
        Arc::new(Self { events, park_accept, accept_latch: Notify::new() })
    }

    fn release_accepts(&self) {
        self.accept_latch.notify_waiters();
        self.accept_latch.notify_one();
    }
}

fn unsupported() -> std::io::ErrorKind {
    std::io::ErrorKind::Unsupported
}

#[async_trait]
impl GatewayRelay for ScriptedRelay {
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
        if self.park_accept {
            self.events.record(Event::AcceptParked);
            self.accept_latch.notified().await;
            self.events.record(Event::AcceptReleased);
        }
        Err(PublicHandshakeError::Timeout)
    }

    async fn read_through(
        &self,
        _from: &mut KtlsReadHalf,
        _terminator: HeadTerminator,
        _limit: NonZeroUsize,
        _deadlines: ReadDeadlines,
    ) -> Result<ThroughBytes, LegReadError> {
        Err(LegReadError::Io { kind: unsupported() })
    }

    async fn write_all(
        &self,
        _to: &mut KtlsWriteHalf,
        _bytes: &[u8],
        _deadline: Instant,
    ) -> Result<(), LegWriteError> {
        Err(LegWriteError::Io { kind: unsupported() })
    }

    async fn relay_exact(
        &self,
        _from: &mut KtlsReadHalf,
        _to: &mut KtlsWriteHalf,
        _len: NonZeroU64,
        _deadlines: RelayDeadlines,
    ) -> Result<(), RelayError> {
        Err(RelayError::Write(LegWriteError::Io { kind: unsupported() }))
    }

    async fn relay_to_end(
        &self,
        _from: &mut KtlsReadHalf,
        _to: &mut KtlsWriteHalf,
        _max: NonZeroU64,
        _deadlines: RelayDeadlines,
    ) -> Result<u64, RelayError> {
        Err(RelayError::Read(LegReadError::Io { kind: unsupported() }))
    }

    async fn close_write(
        &self,
        _to: &mut KtlsWriteHalf,
        _deadline: Instant,
    ) -> Result<(), LegWriteError> {
        Err(LegWriteError::Io { kind: unsupported() })
    }

    async fn discard_remaining(
        &self,
        _from: &mut KtlsReadHalf,
        _max: NonZeroU64,
        _deadlines: RelayDeadlines,
    ) -> Result<(), LegReadError> {
        Err(LegReadError::Io { kind: unsupported() })
    }
}

// ---------------------------------------------------------------------------
// Fixtures: the four doubles of one composition
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FixtureSpec {
    programmed: bool,
    dataplane_fault: Option<(DataplaneOp, GatewayConnectDataplaneError)>,
    ensure_error: Option<GatewayIdentityLifecycleError>,
    /// The identity is not held until the test holds it.
    absent_until_held: bool,
    park_accept: bool,
    hold_authenticate_calls: u32,
}

struct Fixtures {
    dataplane: Arc<ScriptedDataplane>,
    identity: Arc<ScriptedIdentity>,
    holds: Arc<HoldRegistry>,
    relay: Arc<ScriptedRelay>,
    events: Arc<EventLog>,
}

impl Fixtures {
    fn new(spec: FixtureSpec) -> Self {
        let events = Arc::new(EventLog::default());
        Self {
            dataplane: ScriptedDataplane::new(
                if spec.programmed { Backends::Programmed } else { Backends::NoneProgrammed },
                spec.dataplane_fault,
            ),
            identity: ScriptedIdentity::scripted(
                spec.ensure_error,
                spec.absent_until_held,
                Arc::clone(&events),
            ),
            holds: HoldRegistry::new(spec.hold_authenticate_calls),
            relay: ScriptedRelay::new(Arc::clone(&events), spec.park_accept),
            events,
        }
    }

    fn healthy() -> Self {
        Self::new(FixtureSpec::default())
    }
}

// ---------------------------------------------------------------------------
// Key material and the on-disk site of one gateway
// ---------------------------------------------------------------------------

struct KeyMaterial {
    chain_pem: String,
    key_pem: String,
}

impl KeyMaterial {
    /// One base64 line of the private key: a probe for key-byte leaks.
    fn key_body_probe(&self) -> String {
        self.key_pem
            .lines()
            .find(|line| !line.starts_with("-----") && !line.is_empty())
            .expect("PEM body line")
            .to_owned()
    }
}

/// A self-signed leaf for `hostname`, valid from January 1 of `from_year` to
/// January 1 of `until_year`.
fn mint_material(hostname: &str, from_year: i32, until_year: i32) -> KeyMaterial {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("test key");
    let mut params =
        rcgen::CertificateParams::new(vec![hostname.to_owned()]).expect("certificate params");
    params.is_ca = rcgen::IsCa::NoCa;
    params.not_before = rcgen::date_time_ymd(from_year, 1, 1);
    params.not_after = rcgen::date_time_ymd(until_year, 1, 1);
    let cert = params.self_signed(&key).expect("test certificate");
    KeyMaterial { chain_pem: cert.pem(), key_pem: key.serialize_pem() }
}

fn write_private(path: &Path, contents: &str, mode: u32) {
    let staging = path.with_extension("staging");
    std::fs::write(&staging, contents).expect("write staging file");
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(mode)).expect("file mode");
    std::fs::rename(&staging, path).expect("atomic file replacement");
}

fn write_material(chain: &Path, key: &Path, material: &KeyMaterial) {
    write_private(chain, &material.chain_pem, 0o644);
    write_private(key, &material.key_pem, 0o600);
}

struct Site {
    dir: TempDir,
    chain_path: PathBuf,
    key_path: PathBuf,
    intent_path: PathBuf,
    /// Every key body ever written to this site, for leak probes.
    key_probes: Mutex<Vec<String>>,
}

impl Site {
    fn new() -> Self {
        Self::with_material(&mint_material("api.example.com", 2020, 2100))
    }

    fn with_material(material: &KeyMaterial) -> Self {
        let dir = tempfile::tempdir().expect("isolated gateway root");
        let chain_path = dir.path().join("api-origin-chain.pem");
        let key_path = dir.path().join("api-origin-key.pem");
        let intent_path = dir.path().join("intent.redb");
        write_material(&chain_path, &key_path, material);
        let site = Self { dir, chain_path, key_path, intent_path, key_probes: Mutex::new(vec![]) };
        site.remember(material);
        site
    }

    fn remember(&self, material: &KeyMaterial) {
        self.key_probes.lock().expect("probes").push(material.key_body_probe());
    }

    fn replace_material(&self, material: &KeyMaterial) {
        write_material(&self.chain_path, &self.key_path, material);
        self.remember(material);
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

    /// Fail when `rendered` carries a path, file name, PEM armour or key byte.
    fn assert_no_secret_material(&self, rendered: &str, context: &str) {
        let mut forbidden = vec![
            self.dir.path().display().to_string(),
            "api-origin-chain.pem".to_owned(),
            "api-origin-key.pem".to_owned(),
            "PRIVATE KEY".to_owned(),
            "BEGIN CERTIFICATE".to_owned(),
        ];
        forbidden.extend(self.key_probes.lock().expect("probes").iter().cloned());
        for needle in forbidden {
            assert!(!rendered.contains(&needle), "{context} leaked {needle:?}: {rendered}");
        }
    }
}

fn custody_kek() -> Arc<SimKek> {
    Arc::new(SimKek::new().with(
        &KekId::new("overdrive-public-certified-key").expect("custody KEK ID"),
        [0x5a; KEK_LEN],
    ))
}

// ---------------------------------------------------------------------------
// Route and Service builders
// ---------------------------------------------------------------------------

fn route(id: &str, path: &str) -> Route {
    Route::from_submit(PublicRouteInput {
        id: id.to_owned(),
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

fn route_id(raw: &str) -> RouteId {
    RouteId::new(raw).expect("Route ID")
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
// World: one production composition
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Plan {
    slot: Slot,
    ordinal: u16,
    with_service: bool,
}

impl Plan {
    const fn with_service(slot: Slot) -> Self {
        Self { slot, ordinal: 0, with_service: true }
    }

    const fn route_only(slot: Slot) -> Self {
        Self { slot, ordinal: 0, with_service: false }
    }

    const fn ordinal(mut self, ordinal: u16) -> Self {
        self.ordinal = ordinal;
        self
    }
}

fn node_id() -> NodeId {
    NodeId::new("gateway-node").expect("node ID")
}

/// The pre-start half of a composition: only `UnboundGatewayBuilder::new` ran.
async fn unbound(
    site: &Site,
    plan: Plan,
    intent: &Arc<LocalIntentStore>,
    observations: &Arc<dyn ObservationStore>,
    clock: &Arc<SimClock>,
) -> Result<UnboundGatewayBuilder, GatewayBootError> {
    let vip_view = if plan.with_service {
        persist_api_service(intent, plan.slot.vip(plan.ordinal)).await
    } else {
        Arc::new(SimServiceVipView::new(BTreeMap::new()))
    };
    let intent_port: Arc<dyn IntentStore> = intent.clone();
    let observation_port: Arc<dyn ObservationStore> = Arc::clone(observations);
    let clock_port: Arc<dyn Clock> = clock.clone();
    UnboundGatewayBuilder::new(
        node_id(),
        site.config(plan.slot.bind(plan.ordinal)),
        intent_port,
        observation_port,
        vip_view,
        Arc::new(PublicCertifiedKeyAeadCodec::new(custody_kek())),
        clock_port,
    )
    .await
}

struct World {
    intent: Arc<LocalIntentStore>,
    clock: Arc<SimClock>,
    routes: GatewayRouteCommands,
    status: GatewayStatusReads,
    handle: Option<GatewayHandle>,
    fx: Fixtures,
    bind: SocketAddrV4,
    /// Keeps the Service frontend's TCP listener alive so the connector's real
    /// `connect(2)` completes.
    _frontend_listener: Option<std::net::TcpListener>,
}

impl World {
    async fn start(site: &Site, plan: Plan, fx: Fixtures) -> Result<Self, GatewayBootError> {
        Self::start_on(site, plan, Arc::new(SimClock::new()), fx).await
    }

    /// Start on an existing clock so a reopened composition continues the same
    /// simulated time.
    async fn start_on(
        site: &Site,
        plan: Plan,
        clock: Arc<SimClock>,
        fx: Fixtures,
    ) -> Result<Self, GatewayBootError> {
        let observations: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(node_id(), 54));
        Self::start_over(site, plan, clock, fx, observations).await
    }

    /// Start over an existing observation store, so a restarted composition
    /// reads the row its predecessor left (the durable-store half of a restart).
    async fn start_over(
        site: &Site,
        plan: Plan,
        clock: Arc<SimClock>,
        fx: Fixtures,
        observations: Arc<dyn ObservationStore>,
    ) -> Result<Self, GatewayBootError> {
        let intent =
            Arc::new(LocalIntentStore::open(&site.intent_path).expect("real intent store"));
        let frontend_listener = plan.with_service.then(|| {
            let listener = std::net::TcpListener::bind((plan.slot.vip(plan.ordinal), SERVICE_PORT))
                .expect("Service frontend listener");
            listener.set_nonblocking(true).expect("nonblocking frontend listener");
            listener
        });
        let builder = unbound(site, plan, &intent, &observations, &clock).await?;
        let (builder, seal) = builder.into_builder();
        let dependencies = GatewayRuntimeDependencies::new(
            GatewayLimits::first_slice(),
            fx.dataplane.clone(),
            Arc::new(ParkingMtls { seal, registry: Arc::clone(&fx.holds) }),
            fx.identity.clone(),
            fx.relay.clone(),
        );
        let (handle, routes, status) = builder.start(dependencies).await.into_parts();
        Ok(Self {
            intent,
            clock,
            routes,
            status,
            handle: Some(handle),
            fx,
            bind: SocketAddrV4::new(plan.slot.bind(plan.ordinal), 443),
            _frontend_listener: frontend_listener,
        })
    }

    async fn declare(&self, route: Route) -> Result<RouteApplyOutcome, PublicRouteSetError> {
        self.routes.routes().apply(route).await
    }

    /// Graceful shutdown; consumes the world so the redb file is released.
    async fn stop(mut self) -> GatewayShutdownReport {
        self.handle.take().expect("gateway handle").shutdown(GRACE).await
    }
}

// ---------------------------------------------------------------------------
// Waiting and reading helpers
// ---------------------------------------------------------------------------

async fn wait_status(
    status: &GatewayStatusReads,
    what: &str,
    predicate: impl Fn(&GatewayApplicationStatusRowV1) -> bool,
) -> GatewayApplicationStatusRowV1 {
    let deadline = Instant::now() + WAIT;
    let mut last = None;
    loop {
        let row = status.application_status().await.expect("application status read");
        if let Some(row) = row {
            if predicate(&row) {
                return row;
            }
            last = Some(row);
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}; last status {last:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_promoted(status: &GatewayStatusReads) -> GatewayApplicationStatusRowV1 {
    wait_status(status, "a promoted Current generation", |row| {
        row.current.is_some()
            && row.unavailable.is_none()
            && matches!(row.listener, GatewayListenerStatus::Bound { .. })
    })
    .await
}

async fn wait_key_status(
    status: &GatewayStatusReads,
    what: &str,
    predicate: impl Fn(&PublicCertifiedKeyStatusRowV1) -> bool,
) -> PublicCertifiedKeyStatusRowV1 {
    let deadline = Instant::now() + WAIT;
    let mut last = None;
    loop {
        let row = status.certified_key_status().await.expect("custody status read");
        if let Some(row) = row {
            if predicate(&row) {
                return row;
            }
            last = Some(row);
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}; last status {last:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Advance the injected clock in `MAINTENANCE_STEP` increments until the custody
/// status satisfies `predicate`, giving the manual source's private maintenance
/// cadence every chance to run.
async fn advance_until_key_status(
    world: &World,
    what: &str,
    predicate: impl Fn(&PublicCertifiedKeyStatusRowV1) -> bool,
) -> PublicCertifiedKeyStatusRowV1 {
    let mut last = None;
    for _ in 0..MAINTENANCE_STEPS {
        if let Some(row) = world.status.certified_key_status().await.expect("custody status read") {
            if predicate(&row) {
                return row;
            }
            last = Some(row);
        }
        world.clock.tick(MAINTENANCE_STEP);
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("the maintenance cadence never produced {what}; last custody status {last:?}");
}

/// Advance the injected clock over the whole search bound; for the empty-delta
/// arms where nothing is awaited.
async fn advance_through_maintenance(world: &World) {
    for _ in 0..MAINTENANCE_STEPS {
        world.clock.tick(MAINTENANCE_STEP);
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn wait_for<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn custody_records(intent: &LocalIntentStore) -> Vec<(Bytes, Bytes)> {
    intent.scan_prefix(CUSTODY_PREFIX).await.expect("scan custody records")
}

/// The whole intent universe: Route Set, Service, custody records.
async fn intent_universe(intent: &LocalIntentStore) -> Vec<(Bytes, Bytes)> {
    intent.scan_prefix(b"").await.expect("scan the whole intent store")
}

/// `None`: key absent. `Some(None)`: persisted empty set. `Some(Some(_))`: one Route.
async fn route_set(intent: &LocalIntentStore, redb_path: &Path) -> Option<Option<Route>> {
    let bytes = intent.get(ROUTE_SET_KEY.as_bytes()).await.expect("read Route Set")?;
    let set = PublicRouteSetV1::from_store_bytes(&bytes, redb_path, Some(ROUTE_SET_KEY))
        .expect("decode the persisted Route Set");
    Some(set.route)
}

async fn persisted_route(intent: &LocalIntentStore, redb_path: &Path) -> Option<Route> {
    route_set(intent, redb_path).await.expect("the Route Set key is persisted")
}

fn certified_key_generation(
    row: &PublicCertifiedKeyStatusRowV1,
) -> overdrive_core::public_ingress::CertifiedKeyGeneration {
    match &row.state {
        PublicCertifiedKeyStatusState::Usable { generation, .. }
        | PublicCertifiedKeyStatusState::Unusable { generation, .. } => *generation,
        PublicCertifiedKeyStatusState::Absent => panic!("custody status is Absent: {row:?}"),
    }
}

fn is_usable(row: &PublicCertifiedKeyStatusRowV1) -> bool {
    matches!(row.state, PublicCertifiedKeyStatusState::Usable { .. })
}

// ---------------------------------------------------------------------------
// S-PIG-21: Route declare / exact replay / replace / withdraw
// ---------------------------------------------------------------------------

/// S-PIG-21: the four Route commands return exactly `Declared`, `Unchanged`,
/// `Replaced` and `Withdrawn`; the persisted singleton slot holds the one Route;
/// the emptied slot is persisted, not deleted, and survives reopen. A replay has
/// an empty intent delta and no command touches the custody records.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn route_declare_replay_replace_and_withdraw_return_exact_outcomes_over_real_redb() {
    let site = Site::new();
    let world = World::start(&site, Plan::route_only(ROUTE_LIFECYCLE), Fixtures::healthy())
        .await
        .expect("gateway starts");
    let initially_empty = world
        .intent
        .get(ROUTE_SET_KEY.as_bytes())
        .await
        .expect("read initial set")
        .expect("the empty Route Set is persisted at owner start");
    let custody_at_start = custody_records(&world.intent).await;

    let v1 = route("public-api", "/");
    assert_eq!(world.declare(v1.clone()).await.expect("declare"), RouteApplyOutcome::Declared);
    assert_eq!(
        persisted_route(&world.intent, &site.intent_path).await,
        Some(v1.clone()),
        "the singleton slot holds the declared Route",
    );

    let after_declare = intent_universe(&world.intent).await;
    assert_eq!(world.declare(v1.clone()).await.expect("replay"), RouteApplyOutcome::Unchanged);
    assert_eq!(
        intent_universe(&world.intent).await,
        after_declare,
        "an exact replay has an empty intent delta",
    );

    let v2 = route("public-api", "/v2");
    assert_ne!(v1.route_generation(), v2.route_generation(), "the replacement is a new generation");
    assert_eq!(world.declare(v2.clone()).await.expect("replace"), RouteApplyOutcome::Replaced);
    assert_eq!(persisted_route(&world.intent, &site.intent_path).await, Some(v2));

    assert_eq!(
        world.routes.routes().withdraw(route_id("public-api")).await.expect("withdraw"),
        RouteWithdrawOutcome::Withdrawn,
    );
    let after_withdrawal = world
        .intent
        .get(ROUTE_SET_KEY.as_bytes())
        .await
        .expect("read withdrawn set")
        .expect("withdrawal persists the empty slot instead of deleting the key");
    assert_eq!(after_withdrawal, initially_empty, "withdrawal restores the canonical empty slot");
    assert_eq!(
        custody_records(&world.intent).await,
        custody_at_start,
        "no Route command mutates the custody records",
    );

    let report = world.stop().await;
    assert!(report.is_clean(), "Route owner shutdown releases the real store: {report:?}");
    let reopened = LocalIntentStore::open(&site.intent_path).expect("reopen the same redb");
    assert_eq!(
        reopened.get(ROUTE_SET_KEY.as_bytes()).await.expect("read after reopen"),
        Some(initially_empty),
        "the persisted empty slot survives reopen",
    );
}

/// S-PIG-22: a different Route ID while the slot is occupied returns the typed
/// slot conflict; the whole intent universe is unchanged, the redacted status
/// does not echo the typed error text, and the original Route still replays as
/// `Unchanged`.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_different_route_id_is_rejected_with_the_typed_conflict_and_an_unchanged_universe() {
    let site = Site::new();
    let world = World::start(&site, Plan::route_only(COMPETING_ROUTE), Fixtures::healthy())
        .await
        .expect("gateway starts");
    let first = route("public-api", "/");
    world.declare(first.clone()).await.expect("first Route");
    let before = intent_universe(&world.intent).await;
    let identity_calls = world.fx.identity.ensure_calls();

    let error = world.declare(route("other-api", "/")).await.expect_err("slot conflict");
    let message = error.to_string();
    assert!(matches!(
        &error,
        PublicRouteSetError::OccupiedByDifferentRoute { current, attempted }
            if current.as_str() == "public-api" && attempted.as_str() == "other-api"
    ));

    assert_eq!(
        intent_universe(&world.intent).await,
        before,
        "empty Route Set / Service / key delta"
    );
    assert_eq!(world.fx.identity.ensure_calls(), identity_calls, "no identity effect");
    assert_eq!(world.fx.dataplane.residue(), (0, 0), "no connect-path effect");
    let status = world.status.application_status().await.expect("status read");
    assert!(
        !format!("{status:?}").contains(&message),
        "the typed conflict stays apart from the redacted status projection",
    );
    assert_eq!(
        persisted_route(&world.intent, &site.intent_path).await,
        Some(first.clone()),
        "the original Route remains in the slot",
    );
    assert_eq!(world.declare(first).await.expect("original replay"), RouteApplyOutcome::Unchanged);
}

/// S-PIG-23: withdrawing with a different Route ID returns `WithdrawIdMismatch`
/// and has an empty delta over the whole universe; the occupant is untouched.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn withdrawal_by_a_different_route_id_has_an_empty_delta() {
    let site = Site::new();
    let world = World::start(&site, Plan::route_only(WRONG_ID_WITHDRAW), Fixtures::healthy())
        .await
        .expect("gateway starts");
    let first = route("public-api", "/");
    world.declare(first.clone()).await.expect("first Route");
    let before = intent_universe(&world.intent).await;

    let error = world
        .routes
        .routes()
        .withdraw(route_id("other-api"))
        .await
        .expect_err("mismatched withdrawal");
    assert!(matches!(
        &error,
        PublicRouteSetError::WithdrawIdMismatch { current, attempted }
            if current.as_str() == "public-api" && attempted.as_str() == "other-api"
    ));
    assert_eq!(intent_universe(&world.intent).await, before, "empty state delta");
    assert_eq!(persisted_route(&world.intent, &site.intent_path).await, Some(first.clone()));
    assert_eq!(world.declare(first).await.expect("original replay"), RouteApplyOutcome::Unchanged);
}

// ---------------------------------------------------------------------------
// S-PIG-24: two simultaneous declares
// ---------------------------------------------------------------------------

/// SplitMix64: a seeded generator so a failing round is reproducible.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

async fn claim_round(round: u16, seed: u64, rng: &mut SplitMix64) {
    let site = Site::new();
    let world = World::start(
        &site,
        Plan::route_only(CONCURRENT_CLAIMS).ordinal(round),
        Fixtures::healthy(),
    )
    .await
    .expect("gateway starts");
    let barrier = Arc::new(Barrier::new(2));
    let spin_a = rng.next() % 8;
    let spin_b = rng.next() % 8;
    let swap = rng.next().is_multiple_of(2);
    let claim = |id: &'static str, path: &'static str, spin: u64| {
        let routes = world.routes.routes().clone();
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            for _ in 0..spin {
                tokio::task::yield_now().await;
            }
            routes.apply(route(id, path)).await
        })
    };
    let (a, b) = if swap {
        let b = claim("route-b", "/b", spin_b);
        let a = claim("route-a", "/a", spin_a);
        (a, b)
    } else {
        let a = claim("route-a", "/a", spin_a);
        let b = claim("route-b", "/b", spin_b);
        (a, b)
    };
    let outcomes = [a.await.expect("join A"), b.await.expect("join B")];
    let declared: Vec<usize> = outcomes
        .iter()
        .enumerate()
        .filter_map(|(index, outcome)| {
            matches!(outcome, Ok(RouteApplyOutcome::Declared)).then_some(index)
        })
        .collect();
    let conflicts = outcomes
        .iter()
        .filter(|outcome| {
            matches!(outcome, Err(PublicRouteSetError::OccupiedByDifferentRoute { .. }))
        })
        .count();
    assert_eq!(
        declared.len(),
        1,
        "exactly one Declared (seed {seed}, round {round}): {outcomes:?}"
    );
    assert_eq!(
        conflicts, 1,
        "exactly one slot conflict (seed {seed}, round {round}): {outcomes:?}"
    );

    let winner = if declared[0] == 0 { route("route-a", "/a") } else { route("route-b", "/b") };
    if let Err(PublicRouteSetError::OccupiedByDifferentRoute { current, .. }) =
        outcomes.iter().find(|outcome| outcome.is_err()).expect("the loser")
    {
        assert_eq!(
            current,
            winner.route_id(),
            "the conflict names the winner (seed {seed}, round {round})"
        );
    }
    assert_eq!(
        persisted_route(&world.intent, &site.intent_path).await,
        Some(winner),
        "the store holds exactly the winner's Route, no partial Route (seed {seed}, round {round})",
    );
    let report = world.stop().await;
    assert!(report.is_clean(), "seed {seed}, round {round}: {report:?}");
}

/// S-PIG-24: two simultaneous declares with distinct Route IDs, released
/// together on a multi-thread runtime, 256 rounds each on a fresh store. Exactly
/// one `Declared`, the other the typed slot conflict, the store holds exactly the
/// winner. The seed (override with `PIG_S24_SEED`) is printed so a failing round
/// reproduces; each round is capped at 5 s.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_simultaneous_route_claims_have_exactly_one_winner_in_every_round() {
    let seed = std::env::var("PIG_S24_SEED")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(0x5eed_0024);
    eprintln!("S-PIG-24 seed={seed}");
    let mut rng = SplitMix64(seed);
    for round in 0..256u16 {
        tokio::time::timeout(Duration::from_secs(5), claim_round(round, seed, &mut rng))
            .await
            .unwrap_or_else(|_| panic!("round {round} exceeded 5 s (seed {seed})"));
    }
}

// ---------------------------------------------------------------------------
// S-PIG-25: status redaction
// ---------------------------------------------------------------------------

/// S-PIG-25: the rendered operator status carries no manual path, key bytes,
/// ciphertext, nonce or salt, for the Absent-key moment before the gate and the
/// Current moment after promotion.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
async fn operator_status_carries_no_manual_path_or_credential_material() {
    let site = Site::new();
    let world = World::start(&site, Plan::with_service(STATUS_REDACTION), Fixtures::healthy())
        .await
        .expect("gateway starts");
    let before = format!(
        "{:?}{:?}",
        world.status.application_status().await.expect("application status"),
        world.status.certified_key_status().await.expect("key status"),
    );
    site.assert_no_secret_material(&before, "status before a Route");

    world.declare(route("public-api", "/")).await.expect("declare");
    wait_promoted(&world.status).await;
    let after = format!(
        "{:?}{:?}",
        world.status.application_status().await.expect("application status"),
        world.status.certified_key_status().await.expect("key status"),
    );
    site.assert_no_secret_material(&after, "status after promotion");
    for forbidden in ["ciphertext", "nonce", "salt", "kek"] {
        assert!(!after.to_lowercase().contains(forbidden), "status leaked {forbidden}: {after}");
    }
    let key_row = world
        .status
        .certified_key_status()
        .await
        .expect("key status")
        .expect("custody status after boot");
    assert!(is_usable(&key_row), "boot installed the manual key: {key_row:?}");
}

// ---------------------------------------------------------------------------
// S-PIG-26: shutdown
// ---------------------------------------------------------------------------

/// S-PIG-26: with one admitted connection user parked in the relay, shutdown does
/// not clear the identity slot until that user has joined; then the report is
/// clean, the connect path holds no intent or receipt, the identity is empty,
/// and `elapsed` is the injected clock's advance (3 s), not wall time.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER gateway-boot-and-shutdown"]
async fn identity_slot_clears_only_after_the_admitted_connection_user_joined() {
    let site = Site::new();
    let fx = Fixtures::new(FixtureSpec { park_accept: true, ..FixtureSpec::default() });
    let mut world =
        World::start(&site, Plan::with_service(SHUTDOWN_JOIN), fx).await.expect("gateway starts");
    world.declare(route("public-api", "/")).await.expect("declare");
    wait_promoted(&world.status).await;

    let _client = tokio::net::TcpStream::connect(world.bind)
        .await
        .expect("connect to the bound gateway listener");
    let events = Arc::clone(&world.fx.events);
    wait_for("the admitted connection to park in the relay", || {
        events.position(Event::AcceptParked)
    })
    .await;

    let handle = world.handle.take().expect("gateway handle");
    let shutdown = tokio::spawn(async move { handle.shutdown(GRACE).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        world.fx.events.position(Event::IdentityCleared).is_none(),
        "the identity slot must not clear while an admitted user is live: {:?}",
        world.fx.events.snapshot(),
    );
    assert!(world.fx.identity.current().is_some(), "the slot still holds the identity");

    world.clock.tick(Duration::from_secs(3));
    world.fx.relay.release_accepts();
    let report = shutdown.await.expect("join shutdown");

    let released = world.fx.events.position(Event::AcceptReleased).expect("user released");
    let cleared = world.fx.events.position(Event::IdentityCleared).expect("slot cleared");
    assert!(released < cleared, "user joined before the clear: {:?}", world.fx.events.snapshot());
    assert!(report.is_clean(), "{report:?}");
    assert!(report.identity_empty);
    assert!(world.fx.identity.current().is_none());
    assert_eq!(world.fx.dataplane.residue(), (0, 0), "live intent and receipt counts are 0");
    assert_eq!(report.forced_connections, 0, "the user joined, it was not forced");
    assert_eq!(report.elapsed, Duration::from_secs(3), "elapsed is measured on the injected clock");
}

// ---------------------------------------------------------------------------
// S-PIG-27: manual key refusals
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum KeyFault {
    /// The private key carries group/other mode bits.
    PermissiveKeyMode,
    /// The certificate chain file does not exist.
    MissingChain,
    /// The private-key file is not PEM.
    NotPem,
    /// A well-formed PEM key that does not match the certificate.
    MismatchedKey,
}

impl KeyFault {
    const fn expected(self) -> CertifiedKeyFailure {
        match self {
            Self::PermissiveKeyMode => CertifiedKeyFailure::SourcePermissions,
            Self::MissingChain => CertifiedKeyFailure::SourceOpen,
            Self::NotPem => CertifiedKeyFailure::PemShape,
            Self::MismatchedKey => CertifiedKeyFailure::KeyMismatch,
        }
    }

    fn apply(self, site: &Site) {
        match self {
            Self::PermissiveKeyMode => {
                std::fs::set_permissions(&site.key_path, std::fs::Permissions::from_mode(0o644))
                    .expect("loosen the key mode");
            }
            Self::MissingChain => std::fs::remove_file(&site.chain_path).expect("remove chain"),
            Self::NotPem => write_private(&site.key_path, "this is not a PEM private key\n", 0o600),
            Self::MismatchedKey => {
                let other = mint_material("api.example.com", 2020, 2100);
                write_private(&site.key_path, &other.key_pem, 0o600);
                site.remember(&other);
            }
        }
    }
}

/// S-PIG-27: unreadable (permission, missing) and malformed (not PEM,
/// mismatched key) manual key files each refuse boot with their own
/// cause-distinct `GatewayBootError`; the Route Set, custody records and status
/// rows equal their pre-boot state, and neither the error text nor its Debug
/// carries a path, file name, PEM armour or key byte. The root-independent
/// "unreadable" case is the permission policy (a mode-0644 key), because root
/// reads a mode-000 file.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
async fn unreadable_and_malformed_manual_keys_refuse_boot_with_distinct_causes_and_no_mutation() {
    let mut causes = BTreeSet::new();
    for (index, fault) in [
        KeyFault::PermissiveKeyMode,
        KeyFault::MissingChain,
        KeyFault::NotPem,
        KeyFault::MismatchedKey,
    ]
    .into_iter()
    .enumerate()
    {
        let site = Site::new();
        fault.apply(&site);
        let intent =
            Arc::new(LocalIntentStore::open(&site.intent_path).expect("real intent store"));
        let observations: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(node_id(), 55));
        let clock = Arc::new(SimClock::new());
        let custody_before = custody_records(&intent).await;
        let route_before = route_set(&intent, &site.intent_path).await;
        assert!(matches!(route_before, None | Some(None)), "no Route before boot");

        let plan = Plan::route_only(KEY_REFUSALS).ordinal(u16::try_from(index).expect("index"));
        let Err(error) = unbound(&site, plan, &intent, &observations, &clock).await else {
            panic!("{fault:?} must refuse boot");
        };
        assert!(
            matches!(
                &error,
                GatewayBootError::ManualCertifiedKey(PublicCertifiedKeyError::Source(found))
                    if *found == fault.expected()
            ),
            "{fault:?} must refuse with {:?}, got {error:?}",
            fault.expected(),
        );
        causes.insert(format!("{:?}", fault.expected()));

        assert_eq!(
            custody_records(&intent).await,
            custody_before,
            "no custody record for {fault:?}"
        );
        let route_after = route_set(&intent, &site.intent_path).await;
        assert!(matches!(route_after, None | Some(None)), "no Route mutation for {fault:?}");
        assert!(
            observations
                .public_certified_key_status_row(
                    &PublicCertifiedKeyId::new("api-origin").expect("key ID")
                )
                .await
                .expect("custody status read")
                .is_none(),
            "no custody status row for {fault:?}",
        );
        assert!(
            observations
                .gateway_application_status_row(&node_id())
                .await
                .expect("application status read")
                .is_none(),
            "no Application status row for {fault:?}",
        );
        site.assert_no_secret_material(&format!("{error} {error:?}"), "the refusal");
    }
    assert_eq!(causes.len(), 4, "the four refusals are cause-distinct: {causes:?}");
}

// ---------------------------------------------------------------------------
// S-PIG-33: withdraw from Empty and cardinality
// ---------------------------------------------------------------------------

/// S-PIG-33: withdrawing from the persisted Empty set is `Unchanged` with an
/// empty delta, and the empty set survives reopen.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn withdraw_from_empty_is_unchanged_and_the_empty_set_survives_reopen() {
    let site = Site::new();
    let world = World::start(&site, Plan::route_only(WITHDRAW_FROM_EMPTY), Fixtures::healthy())
        .await
        .expect("gateway starts");
    let before = world
        .intent
        .get(ROUTE_SET_KEY.as_bytes())
        .await
        .expect("read empty set")
        .expect("the owner persists the empty set at start");
    let universe = intent_universe(&world.intent).await;

    assert_eq!(
        world.routes.routes().withdraw(route_id("public-api")).await.expect("inverse from Empty"),
        RouteWithdrawOutcome::Unchanged,
    );
    assert_eq!(intent_universe(&world.intent).await, universe, "empty delta");
    let report = world.stop().await;
    assert!(report.is_clean(), "{report:?}");

    let reopened = LocalIntentStore::open(&site.intent_path).expect("reopen");
    assert_eq!(
        reopened.get(ROUTE_SET_KEY.as_bytes()).await.expect("read after reopen"),
        Some(before),
        "the empty key remains present with the same canonical bytes",
    );
}

/// S-PIG-33: Route-set cardinality zero / one / many is closed at one: a
/// withdrawal from zero is `Unchanged`, the first Route is `Declared`, every
/// further distinct ID is the typed conflict naming the occupant, and the
/// occupant still replays.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn route_set_cardinality_zero_one_and_many_is_closed_at_one() {
    let site = Site::new();
    let world = World::start(&site, Plan::route_only(CARDINALITY), Fixtures::healthy())
        .await
        .expect("gateway starts");
    assert_eq!(
        world.routes.routes().withdraw(route_id("public-api")).await.expect("zero"),
        RouteWithdrawOutcome::Unchanged,
    );
    assert_eq!(
        world.declare(route("public-api", "/")).await.expect("one"),
        RouteApplyOutcome::Declared
    );
    for attempted in ["second-api", "third-api", "fourth-api"] {
        assert!(matches!(
            world.declare(route(attempted, "/")).await,
            Err(PublicRouteSetError::OccupiedByDifferentRoute { current, attempted: rejected })
                if current.as_str() == "public-api" && rejected.as_str() == attempted
        ));
    }
    assert_eq!(
        world.declare(route("public-api", "/")).await.expect("occupant"),
        RouteApplyOutcome::Unchanged,
    );
    assert_eq!(
        persisted_route(&world.intent, &site.intent_path).await,
        Some(route("public-api", "/")),
        "many never grows the slot past one",
    );
}

// ---------------------------------------------------------------------------
// S-PIG-46: certified-key install, replay, replacement, invalid, expired
// ---------------------------------------------------------------------------

/// S-PIG-46: the boot install produces the first generation (one protected
/// record, a Usable Manual status with no failure); identical files replayed
/// across the maintenance cadence leave the record and the generation unchanged;
/// a valid replacement produces a new generation under the same record key; a
/// reopen yields that same current generation with an unchanged record.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn certified_key_install_replay_and_valid_replacement_move_the_generation_exactly() {
    let site = Site::new();
    let plan = Plan::route_only(KEY_INSTALL_REPLAY_REPLACE);
    let world = World::start(&site, plan, Fixtures::healthy()).await.expect("gateway starts");
    let installed = wait_key_status(&world.status, "the installed key", is_usable).await;
    assert!(installed.last_install_failure.is_none());
    let first_generation = certified_key_generation(&installed);
    let record_after_install = custody_records(&world.intent).await;
    assert_eq!(record_after_install.len(), 1, "one protected record per certified-key ID");

    advance_through_maintenance(&world).await;
    assert_eq!(
        custody_records(&world.intent).await,
        record_after_install,
        "the identical files replayed have an empty durable delta (Unchanged)",
    );
    let replayed = world.status.certified_key_status().await.expect("status").expect("row");
    assert_eq!(certified_key_generation(&replayed), first_generation);

    let replacement = mint_material("api.example.com", 2020, 2101);
    site.replace_material(&replacement);
    let swapped_in = advance_until_key_status(&world, "the replacement generation", |row| {
        is_usable(row) && certified_key_generation(row) != first_generation
    })
    .await;
    assert!(swapped_in.last_install_failure.is_none(), "a successful install clears the failure");
    let record_after_replacement = custody_records(&world.intent).await;
    assert_eq!(record_after_replacement.len(), 1);
    assert_ne!(record_after_replacement, record_after_install, "Replaced rewrites the record");
    let second_generation = certified_key_generation(&swapped_in);

    let clock = Arc::clone(&world.clock);
    let report = world.stop().await;
    assert!(report.is_clean(), "{report:?}");
    let reopened = World::start_on(&site, plan, clock, Fixtures::healthy())
        .await
        .expect("reopened gateway starts");
    let adopted = wait_key_status(&reopened.status, "the adopted key", is_usable).await;
    assert_eq!(
        certified_key_generation(&adopted),
        second_generation,
        "reopen keeps the generation"
    );
    assert_eq!(
        custody_records(&reopened.intent).await,
        record_after_replacement,
        "adopting the durable record writes nothing",
    );
}

/// S-PIG-46: an invalid replacement (a key that does not match the certificate)
/// leaves the current generation and protected record intact and surfaces only
/// the cause-distinct redacted failure category, no key byte.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_mismatched_replacement_key_preserves_the_current_generation_and_records_its_cause() {
    let site = Site::new();
    let world = World::start(&site, Plan::route_only(KEY_MISMATCH), Fixtures::healthy())
        .await
        .expect("gateway starts");
    let installed = wait_key_status(&world.status, "the installed key", is_usable).await;
    let generation = certified_key_generation(&installed);
    let record = custody_records(&world.intent).await;

    let wrong = mint_material("api.example.com", 2020, 2100);
    write_private(&site.key_path, &wrong.key_pem, 0o600);
    site.remember(&wrong);
    let failed = advance_until_key_status(&world, "the recorded install failure", |row| {
        row.last_install_failure.is_some()
    })
    .await;
    assert_eq!(failed.last_install_failure, Some(CertifiedKeyFailure::KeyMismatch));
    assert!(is_usable(&failed), "the prior generation stays Current: {failed:?}");
    assert_eq!(certified_key_generation(&failed), generation);
    assert_eq!(custody_records(&world.intent).await, record, "empty custody delta");
    site.assert_no_secret_material(&format!("{failed:?}"), "the install-failure status");
}

/// S-PIG-46: an expired replacement is refused the same way: the current
/// generation and record stay intact, the redacted failure category is present,
/// and it is distinct from the key-mismatch category.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn an_expired_replacement_preserves_the_current_generation_and_records_a_distinct_cause() {
    let site = Site::new();
    let world = World::start(&site, Plan::route_only(KEY_EXPIRED_REPLACEMENT), Fixtures::healthy())
        .await
        .expect("gateway starts");
    let installed = wait_key_status(&world.status, "the installed key", is_usable).await;
    let generation = certified_key_generation(&installed);
    let record = custody_records(&world.intent).await;

    site.replace_material(&mint_material("api.example.com", 2019, 2021));
    let failed = advance_until_key_status(&world, "the recorded install failure", |row| {
        row.last_install_failure.is_some()
    })
    .await;
    assert_ne!(
        failed.last_install_failure,
        Some(CertifiedKeyFailure::KeyMismatch),
        "an expired candidate is a different cause from a mismatched key",
    );
    assert!(is_usable(&failed));
    assert_eq!(certified_key_generation(&failed), generation);
    assert_eq!(
        custody_records(&world.intent).await,
        record,
        "an expired candidate is never sealed"
    );
    site.assert_no_secret_material(&format!("{failed:?}"), "the install-failure status");
}

/// S-PIG-46: expiry is derived from the persisted validity plus the injected
/// clock: past `not_after` the status is `Unusable { Expired }` with the same
/// generation, the protected record is retained byte-for-byte, and a reopen at
/// the same simulated time keeps it Unusable instead of replacing it.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn expiry_publishes_unusable_retains_the_record_and_survives_reopen() {
    let site = Site::new();
    let plan = Plan::route_only(KEY_EXPIRY_REOPEN);
    let world = World::start(&site, plan, Fixtures::healthy()).await.expect("gateway starts");
    let installed = wait_key_status(&world.status, "the installed key", is_usable).await;
    let generation = certified_key_generation(&installed);
    let record = custody_records(&world.intent).await;

    world.clock.tick(Duration::from_secs(101 * 365 * 24 * 60 * 60));
    let expired =
        advance_until_key_status(&world, "the Unusable status", |row| !is_usable(row)).await;
    assert!(matches!(
        expired.state,
        PublicCertifiedKeyStatusState::Unusable {
            cause: CertifiedKeyUsabilityFailure::Expired,
            ..
        }
    ));
    assert_eq!(certified_key_generation(&expired), generation);
    assert_eq!(custody_records(&world.intent).await, record, "the protected record is retained");

    let clock = Arc::clone(&world.clock);
    world.stop().await;
    // The files on disk are expired at this simulated time. Boot may refuse the
    // expired material or adopt the retained record as Unusable; either way the
    // durable record is never silently replaced.
    match World::start_on(&site, plan, clock, Fixtures::healthy()).await {
        Ok(reopened) => {
            assert_eq!(custody_records(&reopened.intent).await, record, "adopted, not replaced");
            reopened.stop().await;
        }
        Err(error) => {
            assert!(matches!(error, GatewayBootError::ManualCertifiedKey(_)), "{error:?}");
            let intent = LocalIntentStore::open(&site.intent_path).expect("reopen intent store");
            assert_eq!(custody_records(&intent).await, record, "a refused boot keeps the record");
        }
    }
}

// ---------------------------------------------------------------------------
// S-PIG-50: every GatewayConnectDataplaneError crosses its real owner operation
// ---------------------------------------------------------------------------

/// The exhaustive compile guard of the closed error set: the owner operation
/// that can originate each variant. A new variant fails this match until its
/// owner operation and both evidence layers (crossing, status) are updated.
const fn owning_operation(error: &GatewayConnectDataplaneError) -> DataplaneOp {
    use GatewayConnectDataplaneError as Error;
    match error {
        Error::Unavailable | Error::ProbeTimeout | Error::Kernel => DataplaneOp::Probe,
        Error::SocketCreate(_)
        | Error::SocketOption(_)
        | Error::SocketBind(_)
        | Error::CookieRead(_)
        | Error::IntentRegistryFull
        | Error::IntentAlreadyRegistered => DataplaneOp::OpenUpstream,
        Error::IntentMissing | Error::ReceiptMissing | Error::ReceiptMismatch => {
            DataplaneOp::TakeReceipt
        }
        Error::BackendIdentityMissing { .. } => DataplaneOp::SelectedBackendIdentity,
        Error::Cleanup => DataplaneOp::Cleanup,
    }
}

/// The recognisable `BackendId` the identity-missing case carries.
const LEAK_PROBE_BACKEND: u32 = 907;

fn dataplane_cases() -> Vec<(DataplaneOp, GatewayConnectDataplaneError)> {
    use GatewayConnectDataplaneError as Error;
    use std::io::ErrorKind;
    let primary = vec![
        Error::Unavailable,
        Error::ProbeTimeout,
        Error::SocketCreate(ErrorKind::OutOfMemory),
        Error::SocketOption(ErrorKind::PermissionDenied),
        Error::SocketBind(ErrorKind::AddrInUse),
        Error::CookieRead(ErrorKind::Unsupported),
        Error::IntentRegistryFull,
        Error::IntentAlreadyRegistered,
        Error::IntentMissing,
        Error::ReceiptMissing,
        Error::ReceiptMismatch,
        Error::BackendIdentityMissing {
            backend_id: BackendId::new(LEAK_PROBE_BACKEND).expect("BackendId"),
        },
        Error::Kernel,
        Error::Cleanup,
    ];
    let mut cases: Vec<_> =
        primary.into_iter().map(|error| (owning_operation(&error), error)).collect();
    // The same variants also surface from the sweep and count operations.
    cases.push((DataplaneOp::CleanupAll, Error::Cleanup));
    cases.push((DataplaneOp::LiveIntentCount, Error::Kernel));
    cases.push((DataplaneOp::LiveReceipts, Error::Kernel));
    cases
}

/// Operations the actual-frontend gate drives: they fail the gate.
const fn is_gate_operation(op: DataplaneOp) -> bool {
    matches!(
        op,
        DataplaneOp::Probe
            | DataplaneOp::OpenUpstream
            | DataplaneOp::TakeReceipt
            | DataplaneOp::SelectedBackendIdentity
    )
}

async fn start_dataplane_fault(
    slot: Slot,
    case_index: usize,
    op: DataplaneOp,
    error: &GatewayConnectDataplaneError,
) -> (Site, World) {
    let site = Site::new();
    let fx = Fixtures::new(FixtureSpec {
        programmed: true,
        dataplane_fault: Some((op, error.clone())),
        ..FixtureSpec::default()
    });
    let plan = Plan::with_service(slot).ordinal(u16::try_from(case_index).expect("case index"));
    let world = World::start(&site, plan, fx).await.expect("composition accepts the fault double");
    world.declare(route("public-api", "/")).await.expect("Route command reaches the owner");
    (site, world)
}

/// Wait until the gate settled: promoted, failed with a cause, or (for the
/// post-gate operations) the faulting operation has been driven.
async fn settle_gate(world: &World, op: DataplaneOp) {
    if op == DataplaneOp::Cleanup {
        let dataplane = Arc::clone(&world.fx.dataplane);
        wait_for("the owner to drive cleanup", || dataplane.calls().contains(&op).then_some(()))
            .await;
    } else {
        wait_status(&world.status, "the gate to settle", |row| {
            row.current.is_some() || row.unavailable.is_some()
        })
        .await;
    }
}

/// S-PIG-50: each `GatewayConnectDataplaneError` variant is injected at its real
/// owner operation (probe, open_upstream, take_receipt,
/// selected_backend_identity, cleanup, cleanup_all_gateway_intents, live counts)
/// and the owner receives that exact typed error. Gate operations leave no
/// residue and a clean shutdown; sweep operations surface typed in the shutdown
/// report.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER upstream-socket-preparation"]
async fn every_dataplane_error_crosses_its_real_owner_operation() {
    for (index, (op, error)) in dataplane_cases().into_iter().enumerate() {
        let (_site, world) = start_dataplane_fault(DATAPLANE_CROSSES, index, op, &error).await;
        settle_gate(&world, op).await;
        let dataplane = Arc::clone(&world.fx.dataplane);
        let report = world.stop().await;

        assert!(dataplane.calls().contains(&op), "{error:?}: the owner never drove {op:?}");
        assert!(
            dataplane.injected().contains(&error),
            "{error:?} at {op:?}: the owner did not receive the exact injected error; got {:?}",
            dataplane.injected(),
        );
        match op {
            DataplaneOp::CleanupAll => {
                let residual = report.residual_connect.as_ref().expect("typed sweep residual");
                assert!(
                    matches!(residual.sweep_failures.as_slice(),
                        [GatewayConnectSweepFailure::CleanupAll(found)] if *found == error),
                    "{error:?}: {report:?}",
                );
            }
            DataplaneOp::LiveIntentCount => {
                let residual = report.residual_connect.as_ref().expect("typed count residual");
                assert!(
                    matches!(residual.sweep_failures.as_slice(),
                        [GatewayConnectSweepFailure::IntentCount(found)] if *found == error),
                    "{error:?}: {report:?}",
                );
                assert_eq!(residual.live_intents, None, "the unreadable count is not a zero");
            }
            DataplaneOp::LiveReceipts => {
                let residual = report.residual_connect.as_ref().expect("typed receipt residual");
                assert_eq!(residual.live_receipts, None, "the unreadable snapshot is not a zero");
                assert!(residual.sweep_failures.is_empty(), "{error:?}: {report:?}");
                assert!(!report.is_clean(), "{error:?}: an unreadable snapshot is not clean");
            }
            DataplaneOp::Cleanup => {
                assert!(
                    report.cleanup_failures.iter().any(
                        |failure| matches!(failure, GatewayCleanupFailure::ConnectIntent { count } if *count >= 1)
                    ),
                    "{error:?}: a failed cleanup stays retained and typed: {report:?}",
                );
            }
            _ => {
                assert!(report.is_clean(), "{error:?} at {op:?} left residue: {report:?}");
                assert_eq!(dataplane.residue(), (0, 0), "{error:?}: live counts return to 0");
            }
        }
    }
}

/// S-PIG-50: the owner collapses each gate-time variant to the closed redacted
/// `ConnectPathUnavailable` (connect path Unavailable, nothing Current, listener
/// unbound), and no variant text, `BackendId`, socket cookie or SPIFFE ID leaks
/// into the status; the post-gate sweep variants leave the status redacted too.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER upstream-socket-preparation"]
async fn every_dataplane_error_collapses_to_the_redacted_connect_path_cause() {
    for (index, (op, error)) in dataplane_cases().into_iter().enumerate() {
        let (site, world) = start_dataplane_fault(DATAPLANE_COLLAPSES, index, op, &error).await;
        settle_gate(&world, op).await;
        let status = world.status.clone();
        let dataplane = Arc::clone(&world.fx.dataplane);

        if is_gate_operation(op) {
            let row = status
                .application_status()
                .await
                .expect("status read")
                .expect("status row for the fault");
            assert_eq!(
                row.unavailable,
                Some(GatewayApplicationUnavailableCause::ConnectPathUnavailable),
                "{error:?} at {op:?}",
            );
            assert_eq!(row.connect_path, GatewayConnectPathStatus::Unavailable, "{error:?}");
            assert!(row.current.is_none(), "{error:?}: nothing is admitted");
            assert_eq!(row.listener, GatewayListenerStatus::Unbound, "{error:?}");
        }
        world.stop().await;

        let row = status.application_status().await.expect("status read");
        let rendered = format!("{row:?}");
        assert!(!rendered.contains(&error.to_string()), "{error:?}: status leaked the error text");
        if !matches!(
            error,
            GatewayConnectDataplaneError::Unavailable
                | GatewayConnectDataplaneError::Kernel
                | GatewayConnectDataplaneError::Cleanup
                | GatewayConnectDataplaneError::ProbeTimeout
        ) {
            let variant = format!("{error:?}");
            assert!(
                !rendered.contains(&variant),
                "{error:?}: status leaked the variant: {rendered}"
            );
        }
        for forbidden in [
            "gateway connect dataplane",
            "gateway intent",
            "gateway receipt",
            "gateway kernel operation",
            "spiffe://overdrive.local/workload",
        ] {
            assert!(!rendered.contains(forbidden), "{error:?}: status leaked {forbidden}");
        }
        for intent in dataplane.opened() {
            let cookie = intent.socket_cookie.get().get().to_string();
            assert!(
                !rendered.contains(&format!("cookie: {cookie}")),
                "{error:?}: status leaked socket cookie {cookie}",
            );
        }
        site.assert_no_secret_material(&rendered, "the connect-path status");
    }
}

// ---------------------------------------------------------------------------
// S-PIG-51: identity lifecycle errors
// ---------------------------------------------------------------------------

/// Exhaustive compile guard of the closed lifecycle error set: every variant is
/// returned by `ensure_current` at the gate (the only fallible operation of the
/// port: `disable_after_drain` is infallible, R-30). A new variant fails this
/// match until it is updated.
const fn originates_at_ensure_current(error: &GatewayIdentityLifecycleError) -> bool {
    match error {
        GatewayIdentityLifecycleError::ReconcilerUnavailable
        | GatewayIdentityLifecycleError::Timeout
        | GatewayIdentityLifecycleError::Closed => true,
    }
}

fn identity_errors() -> Vec<GatewayIdentityLifecycleError> {
    vec![
        GatewayIdentityLifecycleError::ReconcilerUnavailable,
        GatewayIdentityLifecycleError::Timeout,
        GatewayIdentityLifecycleError::Closed,
    ]
}

/// S-PIG-51: at the gate each lifecycle error reaches the owner exactly as the
/// port returned it (cause-distinct), nothing is promoted, and the listener
/// stays unbound.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn every_identity_lifecycle_error_reaches_the_owner_cause_distinct() {
    for (index, error) in identity_errors().into_iter().enumerate() {
        assert!(originates_at_ensure_current(&error));

        let site = Site::new();
        let fx = Fixtures::new(FixtureSpec {
            ensure_error: Some(error.clone()),
            ..FixtureSpec::default()
        });
        let plan =
            Plan::with_service(IDENTITY_CROSSES).ordinal(u16::try_from(index).expect("index"));
        let world = World::start(&site, plan, fx).await.expect("composition starts");
        world.declare(route("public-api", "/")).await.expect("Route command");
        let row =
            wait_status(&world.status, "the identity failure", |row| row.unavailable.is_some())
                .await;
        assert!(row.current.is_none(), "{error:?}: nothing is admitted");
        assert_eq!(row.listener, GatewayListenerStatus::Unbound);
        assert!(
            world.fx.identity.observed().iter().all(|seen| *seen == error)
                && !world.fx.identity.observed().is_empty(),
            "{error:?}: the owner saw {:?}",
            world.fx.identity.observed(),
        );
        world.stop().await;
    }
}

/// S-PIG-51: every lifecycle error collapses to `GatewayIdentityUnusable` in
/// the operator status, with no variant name, error text, SPIFFE ID
/// or path in the rendered row.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn every_identity_lifecycle_error_collapses_to_gateway_identity_unusable() {
    for (index, error) in identity_errors().into_iter().enumerate() {
        let site = Site::new();
        let fx = Fixtures::new(FixtureSpec {
            ensure_error: Some(error.clone()),
            ..FixtureSpec::default()
        });
        let plan =
            Plan::with_service(IDENTITY_COLLAPSES).ordinal(u16::try_from(index).expect("index"));
        let world = World::start(&site, plan, fx).await.expect("composition starts");
        world.declare(route("public-api", "/")).await.expect("Route command");
        let row =
            wait_status(&world.status, "the identity cause", |row| row.unavailable.is_some()).await;
        assert_eq!(
            row.unavailable,
            Some(GatewayApplicationUnavailableCause::GatewayIdentityUnusable),
            "{error:?}",
        );
        let rendered = format!("{row:?}");
        for forbidden in [
            error.to_string(),
            format!("{error:?}"),
            "gateway identity".to_owned(),
            "gateway SVID".to_owned(),
            "spiffe://".to_owned(),
        ] {
            assert!(
                !rendered.contains(&forbidden),
                "{error:?}: status leaked {forbidden:?}: {rendered}"
            );
        }
        site.assert_no_secret_material(&rendered, "the identity status");
        world.stop().await;
    }
}

// ---------------------------------------------------------------------------
// SVID issuance failure occurrence: the owner's projection (R-45, R-48, R-52)
// ---------------------------------------------------------------------------

/// The failure the executor would record at the Sim clock's current instant.
fn issue_failure(cause: SvidIssueFailureCause, clock: &SimClock) -> SvidIssueFailure {
    SvidIssueFailure { cause, at: overdrive_core::UnixInstant::from_clock(clock) }
}

/// Every status field except the occurrence fields and the LWW stamp, so a
/// test can assert that recording a failure changed nothing else.
fn outside_the_occurrence(row: &GatewayApplicationStatusRowV1) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        row.node_id,
        row.listener,
        row.staged,
        row.current,
        row.draining,
        row.unavailable,
        row.connect_path,
    )
}

fn absent_identity() -> Fixtures {
    Fixtures::new(FixtureSpec { absent_until_held: true, ..FixtureSpec::default() })
}

/// O-SIF-G1, O-SIF-G2: a gateway whose identity has never issued reads
/// `Absent` with the failure count and the latest cause (distinguishable from
/// the fresh `Absent`, `0`, `None`); a `Ca` failure then an `Audit` failure give
/// `2` and the latest cause and Sim-clock instant; recording changes nothing
/// else in the status; a later success reads `Current` with the count and last
/// failure retained.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER gateway-svid-issue-failure-occurrence"]
async fn a_gateway_whose_identity_never_issues_reads_absent_with_the_failure_count_and_cause() {
    let site = Site::new();
    let world =
        World::start(&site, Plan::with_service(ISSUE_ABSENT_THEN_RECOVERS), absent_identity())
            .await
            .expect("composition starts");
    world.declare(route("public-api", "/")).await.expect("Route command");

    let fresh = wait_status(&world.status, "the first projection", |_| true).await;
    assert_eq!(fresh.gateway_identity, GatewayIdentityStatus::Absent);
    assert_eq!(fresh.issue_failures, 0);
    assert_eq!(fresh.last_issue_failure, None);

    let first = issue_failure(SvidIssueFailureCause::Ca, &world.clock);
    world.fx.identity.record_failure(first);
    let after_first =
        wait_status(&world.status, "the first failure", |row| row.issue_failures == 1).await;
    assert_eq!(after_first.last_issue_failure, Some(first));
    assert_eq!(
        after_first.gateway_identity,
        GatewayIdentityStatus::Absent,
        "a never-succeeded issuance is Absent with a count, not a fresh Absent",
    );
    assert_eq!(outside_the_occurrence(&after_first), outside_the_occurrence(&fresh));

    world.clock.tick(Duration::from_secs(30));
    let second = issue_failure(SvidIssueFailureCause::Audit, &world.clock);
    assert!(second.at > first.at, "the Sim clock advanced between the two failures");
    world.fx.identity.record_failure(second);
    let after_second =
        wait_status(&world.status, "the second failure", |row| row.issue_failures == 2).await;
    assert_eq!(after_second.last_issue_failure, Some(second));
    assert_eq!(after_second.gateway_identity, GatewayIdentityStatus::Absent);
    assert_eq!(outside_the_occurrence(&after_second), outside_the_occurrence(&fresh));

    let held = gateway_identity_facts();
    world.fx.identity.hold(held.clone());
    let recovered = wait_status(&world.status, "the recovered identity", |row| {
        matches!(row.gateway_identity, GatewayIdentityStatus::Current { .. })
    })
    .await;
    assert_eq!(
        recovered.gateway_identity,
        GatewayIdentityStatus::Current {
            epoch: held.epoch,
            spiffe_id: held.spiffe_id,
            serial: held.serial,
            not_after: held.not_after,
        },
    );
    assert_eq!(recovered.issue_failures, 2, "a success never clears the count");
    assert_eq!(recovered.last_issue_failure, Some(second), "nor the latest failure");
    world.stop().await;
}

/// O-SIF-G1 (property over sequences, seeded): across a seeded interleaving of
/// failed issuances, successful holds and restarts over one durable
/// observation store, the projected count equals the number of failures ever
/// recorded, never decreases, and `last` is the latest failure.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER gateway-svid-issue-failure-occurrence"]
async fn the_failure_count_never_decreases_across_failures_successes_and_restarts() {
    const SEED: u64 = 0x5A1F_0A11;
    let mut rng = SplitMix64(SEED);
    let site = Site::new();
    let clock = Arc::new(SimClock::new());
    let observations: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(node_id(), 181));
    let plan = Plan::with_service(ISSUE_SEQUENCE);

    let mut world =
        World::start_over(&site, plan, clock.clone(), absent_identity(), observations.clone())
            .await
            .expect("composition starts");
    world.declare(route("public-api", "/")).await.expect("Route command");

    let mut model_count: u64 = 0;
    let mut model_last: Option<SvidIssueFailure> = None;
    let mut previous_seen: u64 = 0;
    for step in 0..24_u32 {
        let draw = rng.next();
        let operation = match draw % 4 {
            0 | 1 => {
                clock.tick(Duration::from_secs(1));
                let cause = if draw & 0x10 == 0 {
                    SvidIssueFailureCause::Ca
                } else {
                    SvidIssueFailureCause::Audit
                };
                let failure = issue_failure(cause, &clock);
                world.fx.identity.record_failure(failure);
                model_count += 1;
                model_last = Some(failure);
                "failure"
            }
            2 => {
                world.fx.identity.hold(gateway_identity_facts());
                "success"
            }
            _ => {
                world.stop().await;
                world = World::start_over(
                    &site,
                    plan,
                    clock.clone(),
                    absent_identity(),
                    observations.clone(),
                )
                .await
                .expect("restarted composition starts");
                world.declare(route("public-api", "/")).await.expect("Route command");
                "restart"
            }
        };
        let row = wait_status(&world.status, "the modelled occurrence", |row| {
            row.issue_failures == model_count && row.last_issue_failure == model_last
        })
        .await;
        assert!(
            row.issue_failures >= previous_seen,
            "seed {SEED:#x}, step {step} ({operation}): the count fell from {previous_seen} to {}",
            row.issue_failures,
        );
        previous_seen = row.issue_failures;
    }
    world.stop().await;
}

/// O-SIF-G3: re-composing over the same durable store continues the count
/// (`base + new`): a restarted owner first projects the prior row's count and
/// last failure, and one more failure makes it `base + 1` with the new `last`.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER gateway-svid-issue-failure-occurrence"]
async fn a_restarted_gateway_continues_the_failure_count_from_the_prior_row() {
    let site = Site::new();
    let clock = Arc::new(SimClock::new());
    let observations: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(node_id(), 182));
    let plan = Plan::with_service(ISSUE_RESTART);

    let first =
        World::start_over(&site, plan, clock.clone(), absent_identity(), observations.clone())
            .await
            .expect("first composition starts");
    first.declare(route("public-api", "/")).await.expect("Route command");
    let mut last = None;
    for cause in [SvidIssueFailureCause::Ca, SvidIssueFailureCause::Audit] {
        clock.tick(Duration::from_secs(5));
        let failure = issue_failure(cause, &clock);
        first.fx.identity.record_failure(failure);
        last = Some(failure);
    }
    wait_status(&first.status, "two recorded failures", |row| row.issue_failures == 2).await;
    first.stop().await;

    let second =
        World::start_over(&site, plan, clock.clone(), absent_identity(), observations.clone())
            .await
            .expect("second composition starts");
    second.declare(route("public-api", "/")).await.expect("Route command");
    let carried =
        wait_status(&second.status, "the carried base", |row| row.issue_failures == 2).await;
    assert_eq!(carried.last_issue_failure, last, "the prior last failure is retained");

    clock.tick(Duration::from_secs(5));
    let third = issue_failure(SvidIssueFailureCause::Audit, &clock);
    second.fx.identity.record_failure(third);
    let continued =
        wait_status(&second.status, "base plus the new failure", |row| row.issue_failures == 3)
            .await;
    assert_eq!(continued.last_issue_failure, Some(third));
    second.stop().await;
}

/// O-SIF-G3: a prior-row read error is retried and never defaulted to zero: no
/// status row is written while the read keeps failing, and once it succeeds
/// the row carries `base + process` (the facts stayed in the slot).
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER gateway-svid-issue-failure-occurrence"]
async fn a_prior_row_read_error_is_retried_and_never_defaulted_to_zero() {
    let site = Site::new();
    let clock = Arc::new(SimClock::new());
    let inner: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(node_id(), 183));
    let plan = Plan::with_service(ISSUE_READ_RETRY);

    let before = World::start_over(&site, plan, clock.clone(), absent_identity(), inner.clone())
        .await
        .expect("first composition starts");
    before.declare(route("public-api", "/")).await.expect("Route command");
    for _ in 0..5 {
        clock.tick(Duration::from_secs(1));
        before.fx.identity.record_failure(issue_failure(SvidIssueFailureCause::Audit, &clock));
    }
    wait_status(&before.status, "five recorded failures", |row| row.issue_failures == 5).await;
    before.stop().await;

    let faulty = super::issue_fault_store::IssueFaultStore::over(inner.clone());
    faulty.fail_gateway_status_reads.store(true, Ordering::SeqCst);
    let faulty_port: Arc<dyn ObservationStore> = faulty.clone();
    let after = World::start_over(&site, plan, clock.clone(), absent_identity(), faulty_port)
        .await
        .expect("restarted composition starts");
    after.declare(route("public-api", "/")).await.expect("Route command");
    clock.tick(Duration::from_secs(1));
    after.fx.identity.record_failure(issue_failure(SvidIssueFailureCause::Ca, &clock));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        faulty.offered_gateway_status_rows().is_empty(),
        "no row may be written from a defaulted zero while the prior row cannot be read: {:?}",
        faulty.offered_gateway_status_rows(),
    );

    faulty.fail_gateway_status_reads.store(false, Ordering::SeqCst);
    clock.tick(Duration::from_secs(1));
    let latest = issue_failure(SvidIssueFailureCause::Audit, &clock);
    after.fx.identity.record_failure(latest);
    let row = wait_status(&after.status, "base plus both process failures", |row| {
        row.issue_failures == 7
    })
    .await;
    assert_eq!(row.last_issue_failure, Some(latest));
    assert!(
        faulty.offered_gateway_status_rows().iter().all(|written| written.issue_failures >= 5),
        "no written row ever carried a count below the prior base",
    );
    after.stop().await;
}

/// S-PIG-129 (gap 9a, O-SIF-G3 residual): a gateway owner restarted over an
/// undecodable prior status row counts from zero. Garbage bytes under the pinned
/// status table at the node-id key are written through the store-local raw
/// back-door before the production `LocalObservationStore` holds the file; the
/// gateway composition boots over it (no boot refusal), one recorded failure
/// projects `issue_failures == 1` with that failure as the latest (so the prior
/// base was zero), and a restart over the same store carries 1 (the garbage row
/// was replaced by a decodable one). The single skipped-row warning is asserted
/// at the store by the store-local test, not here.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER gateway-svid-issue-failure-occurrence"]
async fn a_restarted_gateway_over_an_undecodable_prior_status_row_counts_from_zero() {
    let site = Site::new();
    let clock = Arc::new(SimClock::new());
    let plan = Plan::with_service(ISSUE_UNDECODABLE_PRIOR);
    let observation_path = site.dir.path().join("observation.redb");

    // The raw write runs first: it opens its own `Database`, which must not
    // overlap the store that will hold the file.
    super::envelope_helpers::write_raw_bytes_to_gateway_application_status_table(
        &observation_path,
        &node_id(),
        b"not an archived gateway status row",
    );
    let observations: Arc<dyn ObservationStore> = Arc::new(
        LocalObservationStore::open(&observation_path).expect("production observation store"),
    );

    let first =
        World::start_over(&site, plan, clock.clone(), absent_identity(), observations.clone())
            .await
            .expect("the owner boots over an undecodable prior row");
    first.declare(route("public-api", "/")).await.expect("Route command");

    clock.tick(Duration::from_secs(5));
    let one = issue_failure(SvidIssueFailureCause::Ca, &clock);
    first.fx.identity.record_failure(one);
    let row =
        wait_status(&first.status, "one failure from a zero base", |row| row.issue_failures == 1)
            .await;
    assert_eq!(row.last_issue_failure, Some(one));
    first.stop().await;

    let second =
        World::start_over(&site, plan, clock.clone(), absent_identity(), observations.clone())
            .await
            .expect("the second composition starts");
    second.declare(route("public-api", "/")).await.expect("Route command");
    let carried =
        wait_status(&second.status, "the carried base", |row| row.issue_failures == 1).await;
    assert_eq!(carried.last_issue_failure, Some(one), "the replaced row decodes and is carried");
    second.stop().await;
}

/// How long a "nothing is written" claim is watched: the owner handles each
/// wake in well under this, so a row that would be written has appeared by then.
const NO_ROW_WATCH: Duration = Duration::from_millis(400);

/// Fail when the gateway status row appears within `NO_ROW_WATCH`.
async fn assert_no_gateway_status_row(world: &World, context: &str) {
    let deadline = Instant::now() + NO_ROW_WATCH;
    while Instant::now() < deadline {
        let row = world.status.application_status().await.expect("application status read");
        assert_eq!(row, None, "{context}: the gateway status row must be absent");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// An identity wake (the identity is held) and two recorded issue failures on a
/// gateway that has no Route; returns the latest failure.
fn identity_and_failures_before_any_route(world: &World) -> SvidIssueFailure {
    world.fx.identity.hold(gateway_identity_facts());
    world.clock.tick(Duration::from_secs(5));
    world.fx.identity.record_failure(issue_failure(SvidIssueFailureCause::Ca, &world.clock));
    world.clock.tick(Duration::from_secs(5));
    let latest = issue_failure(SvidIssueFailureCause::Audit, &world.clock);
    world.fx.identity.record_failure(latest);
    latest
}

/// S-PIG-130 (R-58): a gateway with no Route and no row from an earlier boot
/// writes no status row, however many identity wakes and recorded issue failures
/// arrive. The status of an unconfigured gateway is the absence of a row.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn no_gateway_status_row_is_written_before_a_route_exists() {
    let site = Site::new();
    let world = World::start(&site, Plan::with_service(NO_ROW_BEFORE_ROUTE), absent_identity())
        .await
        .expect("composition starts");
    assert_no_gateway_status_row(&world, "after boot").await;

    identity_and_failures_before_any_route(&world);
    assert_no_gateway_status_row(&world, "after an identity wake and two issue failures").await;
    world.stop().await;
}

/// S-PIG-130 (R-58): the identity facts and the failure count held while no
/// Route existed reach the first row, which is written when a Route is declared.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_first_gateway_status_row_after_a_route_carries_what_was_held_before_it() {
    let site = Site::new();
    let world = World::start(&site, Plan::with_service(FIRST_ROW_AFTER_ROUTE), absent_identity())
        .await
        .expect("composition starts");
    // The owner has subscribed to the identity by the time the boot watch ends,
    // so the wakes below reach it.
    assert_no_gateway_status_row(&world, "after boot").await;
    let latest = identity_and_failures_before_any_route(&world);
    assert_no_gateway_status_row(&world, "before the Route is declared").await;

    world.declare(route("public-api", "/")).await.expect("Route command");
    let held = gateway_identity_facts();
    let first = wait_status(&world.status, "the first row after the Route", |_| true).await;
    assert_eq!(first.issue_failures, 2);
    assert_eq!(first.last_issue_failure, Some(latest));
    assert_eq!(
        first.gateway_identity,
        GatewayIdentityStatus::Current {
            epoch: held.epoch,
            spiffe_id: held.spiffe_id,
            serial: held.serial,
            not_after: held.not_after,
        },
    );
    world.stop().await;
}

/// S-PIG-130 (R-58, R-52): a row left by an earlier boot is still carried when
/// the new boot has no Route: the owner writes, over the prior base, with the
/// cause `RouteAbsent`.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_status_row_from_an_earlier_boot_is_still_carried_when_no_route_exists() {
    let clock = Arc::new(SimClock::new());
    let observations: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(node_id(), 190));
    let plan = Plan::with_service(PRIOR_ROW_WITHOUT_ROUTE);

    let earlier_site = Site::new();
    let earlier = World::start_over(
        &earlier_site,
        plan,
        clock.clone(),
        absent_identity(),
        observations.clone(),
    )
    .await
    .expect("earlier composition starts");
    earlier.declare(route("public-api", "/")).await.expect("Route command");
    let latest = identity_and_failures_before_any_route(&earlier);
    wait_status(&earlier.status, "the earlier boot's row", |row| row.issue_failures == 2).await;
    earlier.stop().await;
    let prior = observations
        .gateway_application_status_row(&node_id())
        .await
        .expect("prior row read")
        .expect("the earlier boot left a row");

    // A new boot over a fresh intent store: no Route is declared.
    let site = Site::new();
    let world = World::start_over(&site, plan, clock, absent_identity(), observations.clone())
        .await
        .expect("new composition starts");
    let row = wait_status(&world.status, "a row over the prior base without a Route", |row| {
        row.unavailable == Some(GatewayApplicationUnavailableCause::RouteAbsent)
            && row.updated_at != prior.updated_at
    })
    .await;
    assert_eq!(row.issue_failures, 2, "the prior base is carried");
    assert_eq!(row.last_issue_failure, Some(latest));
    world.stop().await;
}

/// O-SIF-G5: held material whose `not_after` is not after the owner's clock
/// projects as `Unusable` at the next projection (the failure wake of a failing
/// rotation), with the count above zero; before that wake the row may still read
/// `Current`.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER gateway-svid-issue-failure-occurrence"]
async fn held_identity_past_its_expiry_projects_unusable_at_the_failure_wake() {
    // Both the expiry instant itself (`not_after == now`, not after the clock)
    // and an instant well past it.
    for advance in [100, 200] {
        let site = Site::new();
        let world = World::start(&site, Plan::with_service(ISSUE_EXPIRY), absent_identity())
            .await
            .expect("composition starts");
        world.declare(route("public-api", "/")).await.expect("Route command");

        let mut short_lived = gateway_identity_facts();
        short_lived.not_after = overdrive_core::UnixInstant::from_unix_duration(
            world.clock.unix_now() + Duration::from_secs(100),
        );
        world.fx.identity.hold(short_lived);
        wait_status(&world.status, "the short-lived identity", |row| {
            matches!(row.gateway_identity, GatewayIdentityStatus::Current { .. })
        })
        .await;

        world.clock.tick(Duration::from_secs(advance));
        world.fx.identity.record_failure(issue_failure(SvidIssueFailureCause::Audit, &world.clock));
        let row =
            wait_status(&world.status, "the expired identity", |row| row.issue_failures == 1).await;
        assert_eq!(
            row.gateway_identity,
            GatewayIdentityStatus::Unusable,
            "{advance} s after a 100 s identity was held",
        );
        world.stop().await;
    }
}

/// O-SIF-G5: a boot `ensure_current` timeout maps to neither identity value: it
/// sets the application cause `GatewayIdentityUnusable` and `gateway_identity`
/// stays what the slot says (`Absent` when empty, `Current` once held), with no
/// failure recorded.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER gateway-svid-issue-failure-occurrence"]
async fn a_boot_identity_timeout_sets_the_application_cause_and_leaves_the_identity_as_the_slot_says()
 {
    let site = Site::new();
    let fx = Fixtures::new(FixtureSpec {
        ensure_error: Some(GatewayIdentityLifecycleError::Timeout),
        ..FixtureSpec::default()
    });
    let world = World::start(&site, Plan::with_service(ISSUE_BOOT_TIMEOUT), fx)
        .await
        .expect("composition starts");
    world.declare(route("public-api", "/")).await.expect("Route command");

    let timed_out =
        wait_status(&world.status, "the timeout cause", |row| row.unavailable.is_some()).await;
    assert_eq!(
        timed_out.unavailable,
        Some(GatewayApplicationUnavailableCause::GatewayIdentityUnusable),
    );
    assert_eq!(timed_out.gateway_identity, GatewayIdentityStatus::Absent, "the slot is empty");
    assert_eq!(timed_out.issue_failures, 0);
    assert_eq!(timed_out.last_issue_failure, None);

    world.fx.identity.hold(gateway_identity_facts());
    let held = wait_status(&world.status, "the slot's held identity", |row| {
        matches!(row.gateway_identity, GatewayIdentityStatus::Current { .. })
    })
    .await;
    assert_eq!(held.issue_failures, 0);
    world.stop().await;
}

/// O-SIF-G6: a failing owner status write loses nothing: the record stays in
/// the slot, and the next successful write carries the full count and the latest
/// failure.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER gateway-svid-issue-failure-occurrence"]
async fn a_failing_status_write_loses_nothing_the_next_write_carries_the_full_count() {
    let site = Site::new();
    let clock = Arc::new(SimClock::new());
    let inner: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(node_id(), 186));
    let faulty = super::issue_fault_store::IssueFaultStore::over(inner);
    let port: Arc<dyn ObservationStore> = faulty.clone();
    let world = World::start_over(
        &site,
        Plan::with_service(ISSUE_WRITE_RETRY),
        clock.clone(),
        absent_identity(),
        port,
    )
    .await
    .expect("composition starts");
    world.declare(route("public-api", "/")).await.expect("Route command");
    wait_status(&world.status, "the first projection", |_| true).await;

    faulty.fail_gateway_status_writes.store(true, Ordering::SeqCst);
    for cause in [SvidIssueFailureCause::Ca, SvidIssueFailureCause::Audit] {
        clock.tick(Duration::from_secs(2));
        world.fx.identity.record_failure(issue_failure(cause, &clock));
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let stalled = world.status.application_status().await.expect("status read").expect("a row");
    assert_eq!(stalled.issue_failures, 0, "no failing write became visible");

    faulty.fail_gateway_status_writes.store(false, Ordering::SeqCst);
    clock.tick(Duration::from_secs(2));
    let third = issue_failure(SvidIssueFailureCause::Audit, &clock);
    world.fx.identity.record_failure(third);
    let row = wait_status(&world.status, "the full count", |row| row.issue_failures == 3).await;
    assert_eq!(row.last_issue_failure, Some(third));
    world.stop().await;
}

// ---------------------------------------------------------------------------
// S-PIG-54: the gateway half of C4a
// ---------------------------------------------------------------------------

/// S-PIG-54: custody replay and identity hold repeat keep coverage. With no
/// backend programmed, a gateway rebooted over the same redb adopts the same
/// protected record (empty custody delta), keeps the same certified-key
/// generation, and promotes the same Application generation (the hash of Route
/// generation, frontend and key generation); the persisted Route Set is
/// identical. The connector-level two-request arm is recorded in the B2a gaps
/// file: no public client socket exists in this batch.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER consuming-port-unavailable-outcome"]
async fn custody_replay_across_a_restart_keeps_the_generation_and_the_application_identity() {
    let site = Site::new();
    let plan = Plan::with_service(CUSTODY_REPLAY);
    let first = World::start(&site, plan, Fixtures::healthy()).await.expect("gateway starts");
    first.declare(route("public-api", "/")).await.expect("declare");
    let promoted = wait_promoted(&first.status).await;
    let key_before = first.status.certified_key_status().await.expect("status").expect("row");
    let custody_before = custody_records(&first.intent).await;
    let route_before = persisted_route(&first.intent, &site.intent_path).await;
    let clock = Arc::clone(&first.clock);
    let report = first.stop().await;
    assert!(report.is_clean(), "{report:?}");

    let second = World::start_on(&site, plan, clock, Fixtures::healthy())
        .await
        .expect("restarted gateway starts");
    let repromoted = wait_promoted(&second.status).await;
    let key_after = second.status.certified_key_status().await.expect("status").expect("row");
    assert_eq!(
        certified_key_generation(&key_after),
        certified_key_generation(&key_before),
        "the replayed custody install keeps the generation",
    );
    assert_eq!(custody_records(&second.intent).await, custody_before, "empty custody delta");
    assert_eq!(persisted_route(&second.intent, &site.intent_path).await, route_before);
    assert_eq!(
        repromoted.current.as_ref().map(|current| current.application_generation),
        promoted.current.as_ref().map(|current| current.application_generation),
        "the Application generation is a pure function of Route, frontend and key generations",
    );
    assert_eq!(second.fx.dataplane.residue(), (0, 0), "no backend, no residue");
}

/// S-PIG-54: applying the identical Route to a promoted gateway with no backend
/// has an empty second delta: `Unchanged`, the status row byte-equal (no new
/// write), no further identity ensure, no further connect-path call, no live
/// intent or receipt, and the injected clock unmoved.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER consuming-port-unavailable-outcome"]
async fn applying_the_identical_route_twice_has_an_empty_second_delta() {
    let site = Site::new();
    let world = World::start(&site, Plan::with_service(ROUTE_REAPPLY), Fixtures::healthy())
        .await
        .expect("gateway starts");
    let route = route("public-api", "/");
    world.declare(route.clone()).await.expect("declare");
    let promoted = wait_promoted(&world.status).await;
    let now = world.clock.now();
    let identity_calls = world.fx.identity.ensure_calls();
    let dataplane_calls = world.fx.dataplane.calls();
    let universe = intent_universe(&world.intent).await;

    assert_eq!(world.declare(route).await.expect("second apply"), RouteApplyOutcome::Unchanged);
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(world.status.application_status().await.expect("status"), Some(promoted));
    assert_eq!(world.fx.identity.ensure_calls(), identity_calls, "no further identity ensure");
    assert_eq!(world.fx.dataplane.calls(), dataplane_calls, "no further connect-path call");
    assert_eq!(world.fx.dataplane.residue(), (0, 0));
    assert_eq!(intent_universe(&world.intent).await, universe);
    assert_eq!(world.clock.now(), now, "the injected clock did not move (no sleep or poll)");
}

// ---------------------------------------------------------------------------
// S-PIG-66: watch recovery
// ---------------------------------------------------------------------------

/// S-PIG-66: a burst of committed events overflowing the watch buffer is never
/// silently lost. A witness subscription that is not drained sees `Lagged` and
/// its delivered `Changed` count plus the reported skipped count equals the
/// number of commits (no silent loss). The gateway's Route, custody and
/// Application watchers recover: status is never derived from an empty store
/// (never `RouteAbsent`/`RouteUnreadable` while a Route is declared), the custody
/// generation is unchanged, and a Route replaced after the burst is promoted.
///
/// The burst is aimed at the gateway by construction: the intent store has one
/// change channel that every watcher receives from before it filters by prefix,
/// so a burst under any prefix lags the gateway's own subscriptions. The test
/// therefore requires at least one sampled status row reporting `WatchGap`;
/// the status is sampled by polling, so a design that clears the gap faster
/// than the poll would be a testability boundary to raise, not to weaken here.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER lag-aware-intent-subscription"]
async fn a_watch_lag_is_reported_and_the_gateway_relists_instead_of_reading_an_empty_store() {
    const BURST: usize = 3_000;
    let site = Site::new();
    let world = World::start(&site, Plan::with_service(WATCH_RECOVERY), Fixtures::healthy())
        .await
        .expect("gateway starts");
    world.declare(route("public-api", "/")).await.expect("declare");
    wait_promoted(&world.status).await;
    let key_before = world.status.certified_key_status().await.expect("status").expect("row");

    let mut witness = world.intent.watch(b"noise/").await.expect("subscribe the witness");

    let sampling = Arc::new(AtomicBool::new(true));
    let sampler = {
        let status = world.status.clone();
        let sampling = Arc::clone(&sampling);
        tokio::spawn(async move {
            let mut rows = Vec::new();
            while sampling.load(Ordering::SeqCst) {
                if let Some(row) = status.application_status().await.expect("status read") {
                    rows.push(row);
                }
                tokio::task::yield_now().await;
            }
            rows
        })
    };

    let ops: Vec<TxnOp> = (0..BURST)
        .map(|n| TxnOp::Put {
            key: Bytes::from(format!("noise/{n:05}")),
            value: Bytes::from_static(b"x"),
        })
        .collect();
    world.intent.txn(ops).await.expect("commit the burst in one transaction");

    let mut delivered = 0usize;
    let mut skipped = 0u64;
    let mut lagged = false;
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(250), witness.next()).await
    {
        match event {
            IntentSubscriptionEvent::Changed { .. } => delivered += 1,
            IntentSubscriptionEvent::Lagged { skipped: count } => {
                lagged = true;
                skipped += count.get();
            }
        }
    }
    assert!(lagged, "a burst of {BURST} commits overflows the watch buffer");
    assert_eq!(
        delivered as u64 + skipped,
        BURST as u64,
        "every commit is either delivered or counted as skipped: delivered {delivered}, skipped {skipped}",
    );

    let replacement = route("public-api", "/v2");
    assert_eq!(
        world.declare(replacement.clone()).await.expect("replace"),
        RouteApplyOutcome::Replaced
    );
    let recovered = wait_status(&world.status, "the replacement to be promoted", |row| {
        row.current
            .as_ref()
            .is_some_and(|current| current.route_generation == replacement.route_generation())
            && row.unavailable.is_none()
    })
    .await;
    assert_eq!(
        recovered.current.as_ref().map(|current| current.route_id.as_str()),
        Some("public-api")
    );

    sampling.store(false, Ordering::SeqCst);
    let status_rows = sampler.await.expect("join sampler");
    assert!(
        status_rows
            .iter()
            .any(|row| row.unavailable == Some(GatewayApplicationUnavailableCause::WatchGap)),
        "the burst overflowed the store's single change channel, which every watcher receives \
         from regardless of prefix, so the gateway must have lagged and reported WatchGap in at \
         least one sampled status row; sampled {} rows",
        status_rows.len(),
    );
    for row in status_rows {
        assert!(
            !matches!(
                row.unavailable,
                Some(
                    GatewayApplicationUnavailableCause::RouteAbsent
                        | GatewayApplicationUnavailableCause::RouteUnreadable
                )
            ),
            "a lag must close admission with WatchGap, never read as an empty store: {row:?}",
        );
    }
    let key_after = world.status.certified_key_status().await.expect("status").expect("row");
    assert_eq!(
        certified_key_generation(&key_after),
        certified_key_generation(&key_before),
        "the custody watcher relisted the same generation",
    );
    assert_eq!(
        persisted_route(&world.intent, &site.intent_path).await,
        Some(replacement),
        "the Route Set is untouched by the unrelated burst",
    );
}

// ---------------------------------------------------------------------------
// S-PIG-68: a late probe result after supersession
// ---------------------------------------------------------------------------

/// S-PIG-68: a backend is programmed (the double answers `Selected` with a
/// minted `BackendId`), so the first generation's actual-frontend probe parks
/// inside `GatewayClientMtls::authenticate`, identified by the peer identity it
/// was given. Replacing the Route (same ID, new generation) promotes the second
/// generation. Releasing the held probe afterwards is never delivered: the
/// status row is byte-equal before and after, the superseded generation appears
/// nowhere in it, and the held probe's intent and receipt are cleaned up.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_late_probe_result_after_supersession_is_never_delivered() {
    let site = Site::new();
    let fx = Fixtures::new(FixtureSpec {
        programmed: true,
        hold_authenticate_calls: 1,
        ..FixtureSpec::default()
    });
    let world = World::start(&site, Plan::with_service(SUPERSEDED_PROBE), fx)
        .await
        .expect("gateway starts");
    let first = route("public-api", "/");
    world.declare(first.clone()).await.expect("declare the first generation");

    let holds = Arc::clone(&world.fx.holds);
    let parked = wait_for("the first probe to park in authenticate", || {
        let peers = holds.parked_peers();
        (peers.len() == 1).then(|| peers[0].clone())
    })
    .await;
    let staged =
        wait_status(&world.status, "the first generation staged", |row| row.staged.is_some()).await;
    let superseded = staged.staged.as_ref().expect("staged generation").application_generation;
    assert!(staged.current.is_none(), "a parked probe promotes nothing");
    assert_eq!(staged.listener, GatewayListenerStatus::Unbound);

    let second = route("public-api", "/v2");
    assert_eq!(
        world.declare(second.clone()).await.expect("supersede"),
        RouteApplyOutcome::Replaced
    );
    let promoted = wait_status(&world.status, "the second generation to be promoted", |row| {
        row.current
            .as_ref()
            .is_some_and(|current| current.route_generation == second.route_generation())
            && row.unavailable.is_none()
    })
    .await;
    let current = promoted.current.as_ref().expect("current generation").application_generation;
    assert_ne!(current, superseded, "the replacement is a new Application generation");
    assert!(matches!(promoted.listener, GatewayListenerStatus::Bound { .. }));
    assert_eq!(holds.parked_peers(), vec![parked.clone()], "the first probe is still held");
    assert!(holds.call_count() >= 2, "the second generation authenticated its own probe");

    let now = world.clock.now();
    let before = world.status.application_status().await.expect("status read");
    let returned_before = holds.returned_count();
    holds.release(&parked);
    wait_for("the held probe to return", || {
        (holds.returned_count() > returned_before).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let after = world.status.application_status().await.expect("status read");
    assert_eq!(after, before, "the late result produced no status write");
    let row = after.expect("status row");
    assert_eq!(row.current.as_ref().map(|c| c.application_generation), Some(current));
    assert!(
        row.staged.is_none() && row.draining.is_empty(),
        "no old admission object returned: {row:?}"
    );
    for generation in row.current.iter().chain(row.staged.iter()).chain(row.draining.iter()) {
        assert_ne!(
            generation.application_generation, superseded,
            "the superseded generation reappeared"
        );
    }
    let dataplane = Arc::clone(&world.fx.dataplane);
    wait_for("the held probe's intent and receipt to be cleaned up", || {
        (dataplane.residue() == (0, 0)).then_some(())
    })
    .await;
    assert_eq!(world.clock.now(), now, "the injected clock did not move");
    assert_eq!(
        persisted_route(&world.intent, &site.intent_path).await,
        Some(second),
        "the persisted Route is the replacement",
    );
}
