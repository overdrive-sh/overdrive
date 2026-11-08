//! Registered gateway upstream connector.
//!
//! The prepare-then-dial sequence: ledger reservation, `open_upstream`, dial,
//! receipt validation, identity resolution, mTLS and cleanup with failure
//! retention. The source-local acceptance bodies implement S-PIG-28 through
//! S-PIG-30, S-PIG-37 and S-PIG-53.

use std::net::{SocketAddr, SocketAddrV4};
use std::num::NonZeroU32;
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Instant;

use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::id::{BackendId, SpiffeId};
use overdrive_core::public_ingress::ServiceKey;
use overdrive_core::traits::clock::Clock;

use crate::application::{
    GatewayAdmissionBudget, GatewayAdmissionError, GatewayCleanupLedger, GatewayCleanupReservation,
    GatewayConnectPermit,
};
use crate::ports::{
    GatewayClientMtls, GatewayClientMtlsError, GatewayConnectDataplane,
    GatewayConnectDataplaneError, GatewayConnectIntent, GatewaySelectionReceipt, GatewayUpstream,
};
use crate::runtime::GatewayLimits;

pub(crate) struct GatewayUpstreamConnector {
    dataplane: Arc<dyn GatewayConnectDataplane>,
    client_mtls: Arc<dyn GatewayClientMtls>,
    limits: GatewayLimits,
    clock: Arc<dyn Clock>,
    budget: Arc<GatewayAdmissionBudget>,
    cleanup_ledger: Arc<GatewayCleanupLedger>,
}

impl GatewayUpstreamConnector {
    pub(crate) fn new(
        dataplane: Arc<dyn GatewayConnectDataplane>,
        client_mtls: Arc<dyn GatewayClientMtls>,
        limits: GatewayLimits,
        clock: Arc<dyn Clock>,
        budget: Arc<GatewayAdmissionBudget>,
        cleanup_ledger: Arc<GatewayCleanupLedger>,
    ) -> Self {
        Self { dataplane, client_mtls, limits, clock, budget, cleanup_ledger }
    }

    pub(crate) async fn connect(
        &self,
        frontend: ServiceFrontend,
        deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayUpstreamConnectError> {
        let selected = self.dial(frontend, deadline).await?;
        let peer = self
            .dataplane
            .selected_backend_identity(selected.backend_id)
            .map_err(GatewayUpstreamConnectError::SelectedIdentity)?;
        emit_selected_receipt(&selected.intent, selected.backend_id, &peer);
        let std_stream =
            selected.stream.into_std().map_err(|_| GatewayUpstreamConnectError::Connect)?;
        let connected: OwnedFd = std_stream.into();
        self.client_mtls
            .authenticate(connected, peer, deadline)
            .await
            .map_err(GatewayUpstreamConnectError::Mtls)
    }

    /// The prepare-then-dial sequence, up to a validated `Selected` receipt.
    /// The returned value keeps the connect permit
    /// and the intent cleanup guard alive until the caller finishes with it.
    async fn dial(
        &self,
        frontend: ServiceFrontend,
        deadline: Instant,
    ) -> Result<SelectedDial<'_>, GatewayUpstreamConnectError> {
        let permit = self.budget.try_connect().map_err(GatewayUpstreamConnectError::Capacity)?;
        let mut reservation = self.cleanup_ledger.reserve().map_err(|full| {
            GatewayUpstreamConnectError::CleanupLedgerFull { capacity: full.capacity }
        })?;
        let prepared = self
            .dataplane
            .open_upstream(ServiceKey::from_frontend(frontend))
            .map_err(GatewayUpstreamConnectError::Prepare)?;
        let (socket, intent) = prepared.into_parts();
        reservation.bind(intent.clone());
        let mut cleanup = IntentCleanupGuard::new(&*self.dataplane, intent.clone(), reservation);
        let address = SocketAddr::V4(SocketAddrV4::new(frontend.vip_v4(), frontend.port().get()));
        let connect_deadline =
            deadline.min(self.clock.now() + self.limits.upstream_connect_deadline());
        let connected = tokio::select! {
            connected = socket.connect(address) => connected,
            () = self.clock.sleep(connect_deadline.saturating_duration_since(self.clock.now())) => {
                return Err(GatewayUpstreamConnectError::ConnectTimeout);
            }
        };
        // The receipt is the hook's verdict, so it is taken once after
        // `connect(2)` completes, `Ok` or `Err`, and the outcome is classified
        // by the pair (receipt, connect result), never by errno.
        let receipt = self.dataplane.take_receipt(&intent);
        if receipt.is_ok() {
            cleanup.receipt_taken();
        }
        match (receipt, connected) {
            (Ok(receipt), connected) => {
                Self::classify_receipt(&receipt, intent, connected, cleanup, permit)
            }
            (Err(GatewayConnectDataplaneError::ReceiptMissing), Err(_)) => {
                Err(GatewayUpstreamConnectError::Connect)
            }
            (Err(error), _) => Err(GatewayUpstreamConnectError::Receipt(error)),
        }
    }

    /// Classify a taken receipt against the connect result.
    fn classify_receipt<'a>(
        receipt: &GatewaySelectionReceipt,
        intent: GatewayConnectIntent,
        connected: std::io::Result<tokio::net::TcpStream>,
        cleanup: IntentCleanupGuard<'a>,
        permit: GatewayConnectPermit,
    ) -> Result<SelectedDial<'a>, GatewayUpstreamConnectError> {
        match *receipt {
            GatewaySelectionReceipt::Selected { socket_cookie, service_key, backend_id }
                if socket_cookie == intent.socket_cookie && service_key == intent.service_key =>
            {
                let stream = connected.map_err(|_| GatewayUpstreamConnectError::Connect)?;
                Ok(SelectedDial { stream, _cleanup: cleanup, _permit: permit, intent, backend_id })
            }
            GatewaySelectionReceipt::NoBackend { socket_cookie, service_key }
                if socket_cookie == intent.socket_cookie && service_key == intent.service_key =>
            {
                emit_no_backend_receipt(&intent);
                Err(GatewayUpstreamConnectError::NoBackend)
            }
            GatewaySelectionReceipt::Selected { .. }
            | GatewaySelectionReceipt::NoBackend { .. } => {
                Err(GatewayUpstreamConnectError::ReceiptMismatch)
            }
        }
    }
}

/// A dialed upstream whose `Selected` receipt validated. Field order is drop
/// order: the socket closes, then the registered intent is cleaned, then the
/// connect permit is released.
struct SelectedDial<'a> {
    stream: tokio::net::TcpStream,
    _cleanup: IntentCleanupGuard<'a>,
    _permit: GatewayConnectPermit,
    intent: GatewayConnectIntent,
    backend_id: BackendId,
}

/// The connector's one cleanup guard. It owns the bound ledger reservation
/// and cleans the exact registered intent when it drops, so every result,
/// including a connect future dropped at an await, cleans up. Success
/// completes the ledger record; a failed cleanup marks the failing key on the
/// record and leaves it retained for the ledger's next sweep.
struct IntentCleanupGuard<'a> {
    dataplane: &'a dyn GatewayConnectDataplane,
    intent: GatewayConnectIntent,
    reservation: Option<GatewayCleanupReservation>,
    receipt: ReceiptState,
}

impl<'a> IntentCleanupGuard<'a> {
    fn new(
        dataplane: &'a dyn GatewayConnectDataplane,
        intent: GatewayConnectIntent,
        reservation: GatewayCleanupReservation,
    ) -> Self {
        Self {
            dataplane,
            intent,
            reservation: Some(reservation),
            receipt: ReceiptState::Unconsumed,
        }
    }

    /// The selection receipt has been taken; a later cleanup failure is an
    /// intent failure.
    const fn receipt_taken(&mut self) {
        self.receipt = ReceiptState::Consumed;
    }
}

impl Drop for IntentCleanupGuard<'_> {
    fn drop(&mut self) {
        let Some(reservation) = self.reservation.take() else {
            return;
        };
        if self.dataplane.cleanup(&self.intent).is_ok() {
            reservation.complete();
            return;
        }
        match self.receipt {
            ReceiptState::Unconsumed => reservation.record_receipt_failure(),
            ReceiptState::Consumed => reservation.record_intent_failure(),
        }
    }
}

/// Whether the connector took the selection receipt before cleanup ran.
#[derive(Clone, Copy)]
enum ReceiptState {
    Unconsumed,
    Consumed,
}

/// The `Selected` receipt occurrence: after exact receipt validation and
/// selected-identity resolution, before the mTLS handshake.
fn emit_selected_receipt(
    intent: &GatewayConnectIntent,
    backend_id: BackendId,
    expected_peer: &SpiffeId,
) {
    tracing::debug!(
        name: "gateway.upstream.receipt.validated",
        target: "overdrive::gateway_connector",
        event = "gateway.upstream.receipt.validated",
        socket_cookie = intent.socket_cookie.get().get(),
        service_key_vip = %intent.service_key.vip(),
        service_key_port = intent.service_key.port().get(),
        service_key_protocol = intent.service_key.protocol().as_str(),
        receipt_outcome = "selected",
        backend_id = backend_id.get(),
        expected_peer_spiffe_id = %expected_peer,
        "validated gateway upstream BPF selection receipt",
    );
}

/// The `NoBackend` receipt occurrence: after exact receipt validation.
fn emit_no_backend_receipt(intent: &GatewayConnectIntent) {
    tracing::debug!(
        name: "gateway.upstream.receipt.validated",
        target: "overdrive::gateway_connector",
        event = "gateway.upstream.receipt.validated",
        socket_cookie = intent.socket_cookie.get().get(),
        service_key_vip = %intent.service_key.vip(),
        service_key_port = intent.service_key.port().get(),
        service_key_protocol = intent.service_key.protocol().as_str(),
        receipt_outcome = "no_backend",
        "validated gateway upstream BPF selection receipt",
    );
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GatewayUpstreamConnectError {
    #[error("gateway admission capacity exhausted")]
    Capacity(GatewayAdmissionError),
    #[error("gateway cleanup ledger is full")]
    CleanupLedgerFull { capacity: NonZeroU32 },
    /// `open_upstream` failed (ADR-0243).
    #[error("gateway upstream socket preparation failed: {0}")]
    Prepare(GatewayConnectDataplaneError),
    #[error("gateway upstream connect timed out")]
    ConnectTimeout,
    #[error("gateway upstream connect failed")]
    Connect,
    #[error("gateway selection receipt failed: {0}")]
    Receipt(GatewayConnectDataplaneError),
    #[error("gateway selection receipt mismatch")]
    ReceiptMismatch,
    #[error("gateway has no selectable backend")]
    NoBackend,
    #[error("selected backend identity failed: {0}")]
    SelectedIdentity(GatewayConnectDataplaneError),
    #[error("gateway-client mTLS failed: {0}")]
    Mtls(GatewayClientMtlsError),
}

#[cfg(all(test, feature = "integration-tests"))]
mod acceptance {
    //! S-PIG-28..30, S-PIG-37 (connector arm), S-PIG-53 and S-PIG-69 (I half).
    //!
    //! The connector issues one real loopback `connect(2)` to the Service
    //! frontend, so every fixture binds a real `TcpListener` on `127.0.0.1:0`
    //! as the frontend address. The dataplane and the gateway-client mTLS
    //! port are test-local doubles: the dataplane double mints a nonzero
    //! cookie per `open_upstream`, records the registered intent and returns
    //! `PreparedUpstream::new(TcpSocket::new_v4()?, intent)`; the mTLS double
    //! fails with a scripted `GatewayClientMtlsError` after dropping the
    //! connected fd.
    #![allow(
        clippy::doc_markdown,
        clippy::too_many_lines,
        reason = "CONTRACT_SHAPE metadata and exhaustive closed-error matrices"
    )]

    use std::collections::{BTreeMap, BTreeSet};
    use std::os::fd::OwnedFd;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use overdrive_core::dataplane::Proto;
    use overdrive_core::id::{BackendId, ServiceVip, SpiffeId};
    use overdrive_core::public_ingress::{ServiceKey, SocketCookie};
    use overdrive_sim::adapters::clock::SimClock;
    use tokio::io::AsyncReadExt;
    use tokio::sync::Notify;

    use super::*;
    use crate::application::{GatewayCleanupFailure, GatewayCleanupReservation};
    use crate::ports::{
        GatewayConnectCleanup, GatewayConnectCleanupSweep, GatewayConnectIntent,
        GatewaySelectionReceipt, KtlsArmError, PreparedUpstream,
    };

    const SELECTED_PEER: &str = "spiffe://overdrive.local/workload/api/alloc/api-0";
    const OTHER_PEER: &str = "spiffe://overdrive.local/workload/other/alloc/other-0";

    #[derive(Clone, Copy)]
    enum ReceiptMode {
        /// The hook wrote no receipt for the registered intent.
        Missing,
        /// The hook wrote a receipt carrying another Service key.
        OtherServiceKey,
        Selected,
        NoBackend,
    }

    /// Test-local `GatewayConnectDataplane` double.
    struct ConnectDataplane {
        receipt_mode: ReceiptMode,
        ledger: Arc<GatewayCleanupLedger>,
        next_cookie: AtomicU64,
        open_failure: Mutex<Option<GatewayConnectDataplaneError>>,
        identity_missing: AtomicBool,
        cleanup_failing: AtomicBool,
        opens: AtomicU32,
        ledger_len_during_open: Mutex<Vec<u32>>,
        registered: Mutex<BTreeSet<GatewayConnectIntent>>,
        receipt_live: Mutex<BTreeSet<SocketCookie>>,
        receipts_taken: AtomicU32,
        identity_lookups: Mutex<Vec<BackendId>>,
        cleanup_intents: Mutex<Vec<GatewayConnectIntent>>,
    }

    impl ConnectDataplane {
        /// The receipt the hook records for `intent` under this double's mode.
        fn receipt_for(&self, intent: &GatewayConnectIntent) -> GatewaySelectionReceipt {
            let backend_id = BackendId::new(7).expect("BackendId");
            match self.receipt_mode {
                ReceiptMode::Missing => unreachable!("a missing receipt is never made live"),
                ReceiptMode::OtherServiceKey => {
                    let other = ServiceFrontend::new(
                        ServiceVip::new("10.96.0.2".parse().expect("IP")).expect("VIP"),
                        std::num::NonZeroU16::new(8081).expect("port"),
                        Proto::Tcp,
                    )
                    .expect("frontend");
                    GatewaySelectionReceipt::Selected {
                        socket_cookie: intent.socket_cookie,
                        service_key: ServiceKey::from_frontend(other),
                        backend_id,
                    }
                }
                ReceiptMode::Selected => GatewaySelectionReceipt::Selected {
                    socket_cookie: intent.socket_cookie,
                    service_key: intent.service_key,
                    backend_id,
                },
                ReceiptMode::NoBackend => GatewaySelectionReceipt::NoBackend {
                    socket_cookie: intent.socket_cookie,
                    service_key: intent.service_key,
                },
            }
        }
    }

    impl GatewayConnectCleanup for ConnectDataplane {
        fn cleanup(
            &self,
            intent: &GatewayConnectIntent,
        ) -> Result<(), GatewayConnectDataplaneError> {
            self.cleanup_intents.lock().expect("cleanup log").push(intent.clone());
            if self.cleanup_failing.load(Ordering::SeqCst) {
                return Err(GatewayConnectDataplaneError::Cleanup);
            }
            self.registered.lock().expect("registry").remove(intent);
            self.receipt_live.lock().expect("receipts").remove(&intent.socket_cookie);
            Ok(())
        }

        fn cleanup_all_gateway_intents(
            &self,
        ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
            let removed_intents = u32::try_from(
                std::mem::take(&mut *self.registered.lock().expect("registry")).len(),
            )
            .expect("small count");
            let removed_receipts = u32::try_from(
                std::mem::take(&mut *self.receipt_live.lock().expect("receipts")).len(),
            )
            .expect("small count");
            Ok(GatewayConnectCleanupSweep { removed_intents, removed_receipts })
        }

        fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
            Ok(u32::try_from(self.registered.lock().expect("registry").len()).expect("count"))
        }

        fn live_receipts(
            &self,
        ) -> Result<BTreeMap<SocketCookie, GatewaySelectionReceipt>, GatewayConnectDataplaneError>
        {
            let live = self.receipt_live.lock().expect("receipts");
            let registered = self.registered.lock().expect("registry");
            Ok(registered
                .iter()
                .filter(|intent| live.contains(&intent.socket_cookie))
                .map(|intent| (intent.socket_cookie, self.receipt_for(intent)))
                .collect())
        }
    }

    #[async_trait]
    impl GatewayConnectDataplane for ConnectDataplane {
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
            self.opens.fetch_add(1, Ordering::SeqCst);
            self.ledger_len_during_open.lock().expect("open log").push(self.ledger.len());
            let scripted_failure = self.open_failure.lock().expect("open failure").take();
            if let Some(failure) = scripted_failure {
                return Err(failure);
            }
            let cookie = SocketCookie::new(self.next_cookie.fetch_add(1, Ordering::SeqCst))
                .expect("nonzero cookie");
            let socket = tokio::net::TcpSocket::new_v4()
                .map_err(|error| GatewayConnectDataplaneError::SocketCreate(error.kind()))?;
            let intent = GatewayConnectIntent { socket_cookie: cookie, service_key: key };
            self.registered.lock().expect("registry").insert(intent.clone());
            if !matches!(self.receipt_mode, ReceiptMode::Missing) {
                self.receipt_live.lock().expect("receipts").insert(cookie);
            }
            Ok(PreparedUpstream::new(socket, intent))
        }

        fn take_receipt(
            &self,
            intent: &GatewayConnectIntent,
        ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
            self.receipts_taken.fetch_add(1, Ordering::SeqCst);
            if !self.receipt_live.lock().expect("receipts").remove(&intent.socket_cookie) {
                return Err(GatewayConnectDataplaneError::ReceiptMissing);
            }
            Ok(self.receipt_for(intent))
        }

        fn selected_backend_identity(
            &self,
            backend_id: BackendId,
        ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
            self.identity_lookups.lock().expect("identity log").push(backend_id);
            if self.identity_missing.load(Ordering::SeqCst) {
                return Err(GatewayConnectDataplaneError::BackendIdentityMissing { backend_id });
            }
            Ok(SpiffeId::new(SELECTED_PEER).expect("selected peer"))
        }
    }

    /// Test-local `GatewayClientMtls` double: fails with the scripted error
    /// after dropping the connected fd; with `hold` it parks inside
    /// `authenticate` until released.
    struct ScriptedMtls {
        hold: bool,
        attempts: AtomicU32,
        expected_peers: Mutex<Vec<SpiffeId>>,
        error: Mutex<Option<GatewayClientMtlsError>>,
        parked: Notify,
        release: Notify,
    }

    #[async_trait]
    impl GatewayClientMtls for ScriptedMtls {
        async fn authenticate(
            &self,
            connected: OwnedFd,
            expected_peer: SpiffeId,
            _deadline: Instant,
        ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            self.expected_peers.lock().expect("peer log").push(expected_peer.clone());
            if self.hold {
                self.parked.notify_one();
                self.release.notified().await;
            }
            drop(connected);
            let error =
                self.error.lock().expect("mTLS fault").take().expect("one originating mTLS fault");
            if let GatewayClientMtlsError::PeerSpiffeMismatch { actual, .. } = error {
                return Err(GatewayClientMtlsError::PeerSpiffeMismatch {
                    expected: expected_peer,
                    actual,
                });
            }
            Err(error)
        }
    }

    struct Fixture {
        connector: GatewayUpstreamConnector,
        frontend: ServiceFrontend,
        /// Accepts the one connector connection and returns how many octets
        /// the connector wrote before closing it.
        accept: tokio::task::JoinHandle<usize>,
        dataplane: Arc<ConnectDataplane>,
        mtls: Arc<ScriptedMtls>,
        ledger: Arc<GatewayCleanupLedger>,
        clock: Arc<SimClock>,
    }

    fn wrong_valid_peer_mismatch() -> GatewayClientMtlsError {
        GatewayClientMtlsError::PeerSpiffeMismatch {
            expected: SpiffeId::new(SELECTED_PEER).expect("selected peer"),
            actual: SpiffeId::new(OTHER_PEER).expect("wrong-valid peer"),
        }
    }

    async fn fixture_with(
        receipt_mode: ReceiptMode,
        mtls_error: Option<GatewayClientMtlsError>,
        hold: bool,
    ) -> Fixture {
        let listener =
            tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("real loopback listener");
        let addr = listener.local_addr().expect("listener address");
        let accept = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("connector reached listener");
            let mut received = Vec::new();
            stream.read_to_end(&mut received).await.expect("read until the connector closes");
            received.len()
        });
        let frontend = ServiceFrontend::new(
            ServiceVip::new(addr.ip()).expect("VIP"),
            std::num::NonZeroU16::new(addr.port()).expect("port"),
            Proto::Tcp,
        )
        .expect("frontend");
        let limits = GatewayLimits::first_slice();
        let budget = GatewayAdmissionBudget::new(Arc::new(limits.clone()));
        let ledger = GatewayCleanupLedger::new(limits.max_cleanup_ledger_entries());
        let dataplane = Arc::new(ConnectDataplane {
            receipt_mode,
            ledger: ledger.clone(),
            next_cookie: AtomicU64::new(1),
            open_failure: Mutex::new(None),
            identity_missing: AtomicBool::new(false),
            cleanup_failing: AtomicBool::new(false),
            opens: AtomicU32::new(0),
            ledger_len_during_open: Mutex::new(Vec::new()),
            registered: Mutex::new(BTreeSet::new()),
            receipt_live: Mutex::new(BTreeSet::new()),
            receipts_taken: AtomicU32::new(0),
            identity_lookups: Mutex::new(Vec::new()),
            cleanup_intents: Mutex::new(Vec::new()),
        });
        let mtls = Arc::new(ScriptedMtls {
            hold,
            attempts: AtomicU32::new(0),
            expected_peers: Mutex::new(Vec::new()),
            error: Mutex::new(mtls_error),
            parked: Notify::new(),
            release: Notify::new(),
        });
        let clock = Arc::new(SimClock::new());
        let connector = GatewayUpstreamConnector::new(
            dataplane.clone(),
            mtls.clone(),
            limits,
            clock.clone(),
            budget,
            ledger.clone(),
        );
        Fixture { connector, frontend, accept, dataplane, mtls, ledger, clock }
    }

    /// The connector call has completed, so a dial would already sit in the
    /// listener's backlog (the kernel completes the handshake) and the accept
    /// task would finish at once: it must still be waiting after a grace period
    /// that is long compared with a loopback accept.
    async fn assert_nothing_was_dialed(accept: &mut tokio::task::JoinHandle<usize>) {
        let finished = tokio::time::timeout(Duration::from_millis(250), &mut *accept).await;
        assert!(finished.is_err(), "nothing was dialed, but a connection reached the frontend");
        accept.abort();
        assert!(
            accept.await.expect_err("the accept task never completed").is_cancelled(),
            "the accept task ended only because it was cancelled"
        );
    }

    /// Closes the frontend port: the listener lives in the accept task, so
    /// cancelling the task closes it and the kernel refuses every later dial.
    async fn close_the_frontend(fx: &mut Fixture) {
        fx.accept.abort();
        assert!(
            (&mut fx.accept).await.expect_err("the accept task never completed").is_cancelled(),
            "the listener is closed, so the frontend port refuses every dial",
        );
    }

    async fn fixture(receipt_mode: ReceiptMode) -> Fixture {
        fixture_with(receipt_mode, Some(wrong_valid_peer_mismatch()), false).await
    }

    fn deadline(fx: &Fixture) -> Instant {
        fx.clock.now() + Duration::from_secs(5)
    }

    async fn connect_error(fx: &Fixture) -> GatewayUpstreamConnectError {
        match fx.connector.connect(fx.frontend, deadline(fx)).await {
            Ok(_) => panic!("the originating fault cannot yield an authenticated upstream"),
            Err(error) => error,
        }
    }

    fn reserve_one(ledger: &Arc<GatewayCleanupLedger>) -> GatewayCleanupReservation {
        match ledger.reserve() {
            Ok(reservation) => reservation,
            Err(full) => panic!("ledger refused below capacity {}", full.capacity),
        }
    }

    fn count(value: &AtomicU32) -> u32 {
        value.load(Ordering::SeqCst)
    }

    /// The dataplane state delta every fail-closed connect must leave: no
    /// registered intent, no live receipt, an empty ledger, and exactly one
    /// cleanup of the exact intent that was registered.
    fn assert_fail_closed_residue_is_empty(fx: &Fixture) {
        assert_eq!(fx.dataplane.live_intent_count().expect("intent count"), 0);
        assert_eq!(fx.dataplane.live_receipts().expect("receipt count").len(), 0);
        assert_eq!(fx.ledger.len(), 0);
        assert_eq!(count(&fx.dataplane.opens), 1, "exactly one upstream was prepared");
        assert_eq!(
            fx.dataplane.cleanup_intents.lock().expect("cleanup log").len(),
            1,
            "the registered intent is cleaned up exactly once",
        );
    }

    /// S-PIG-28: a missing selection receipt fails closed with the typed
    /// receipt error before identity resolution or mTLS.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn missing_selection_receipt_fails_closed_before_identity_and_mtls() {
        let fx = fixture(ReceiptMode::Missing).await;
        let error = connect_error(&fx).await;
        assert!(matches!(
            error,
            GatewayUpstreamConnectError::Receipt(GatewayConnectDataplaneError::ReceiptMissing)
        ));
        assert_fail_closed_residue_is_empty(&fx);
        assert_eq!(count(&fx.mtls.attempts), 0);
        assert!(fx.dataplane.identity_lookups.lock().expect("identity log").is_empty());
        assert_eq!(fx.accept.await.expect("accept task"), 0, "no byte sent upstream");
    }

    /// S-PIG-29: a receipt carrying another `ServiceKey` is a
    /// `ReceiptMismatch`; no identity is resolved and no mTLS starts.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn receipt_for_another_service_key_fails_closed_before_identity_and_mtls() {
        let fx = fixture(ReceiptMode::OtherServiceKey).await;
        let error = connect_error(&fx).await;
        assert!(matches!(error, GatewayUpstreamConnectError::ReceiptMismatch));
        assert_fail_closed_residue_is_empty(&fx);
        assert_eq!(count(&fx.dataplane.receipts_taken), 1, "one receipt consumed, no retry");
        assert_eq!(count(&fx.mtls.attempts), 0);
        assert!(fx.dataplane.identity_lookups.lock().expect("identity log").is_empty());
        assert_eq!(fx.accept.await.expect("accept task"), 0, "no byte sent upstream");
    }

    /// S-PIG-30: a valid peer other than the selected one fails the exact
    /// peer match; one attempt, one backend, nothing else is dialed.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn wrong_valid_selected_peer_fails_closed_without_an_alternate_backend() {
        let fx = fixture(ReceiptMode::Selected).await;
        let error = connect_error(&fx).await;
        let GatewayUpstreamConnectError::Mtls(GatewayClientMtlsError::PeerSpiffeMismatch {
            expected,
            actual,
        }) = error
        else {
            panic!("the wrong-valid peer must surface as the exact-peer mismatch");
        };
        assert_eq!(expected, SpiffeId::new(SELECTED_PEER).expect("selected peer"));
        assert_eq!(actual, SpiffeId::new(OTHER_PEER).expect("wrong-valid peer"));
        assert_fail_closed_residue_is_empty(&fx);
        assert_eq!(count(&fx.mtls.attempts), 1, "single upstream attempt");
        assert_eq!(
            *fx.dataplane.identity_lookups.lock().expect("identity log"),
            vec![BackendId::new(7).expect("BackendId")],
            "only the receipt's backend is resolved; no second backend",
        );
        assert_eq!(count(&fx.dataplane.receipts_taken), 1);
        assert_eq!(fx.accept.await.expect("accept task"), 0, "no byte sent upstream");
    }

    /// S-PIG-28..30 family: every `GatewayClientMtlsError` the port can return
    /// is mapped to `Mtls(<same error>)` and leaves no intent or receipt.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn every_gateway_client_mtls_fault_is_mapped_and_cleans_intent_state() {
        let faults = [
            GatewayClientMtlsError::IdentityAbsent,
            GatewayClientMtlsError::IdentityExpired,
            GatewayClientMtlsError::PeerAddress,
            GatewayClientMtlsError::HandshakeTimeout,
            GatewayClientMtlsError::Handshake,
            GatewayClientMtlsError::PeerCertificateMissing,
            GatewayClientMtlsError::PeerSpiffeShape,
            wrong_valid_peer_mismatch(),
            GatewayClientMtlsError::Arm(KtlsArmError::UlpInstall { errno: 92 }),
        ];
        assert_eq!(faults.len(), 9, "the closed GatewayClientMtlsError has nine variants");
        for originating in faults {
            let expected = originating.clone();
            let fx = fixture_with(ReceiptMode::Selected, Some(originating), false).await;
            let error = connect_error(&fx).await;
            assert!(
                matches!(&error, GatewayUpstreamConnectError::Mtls(observed) if *observed == expected),
                "{expected:?} must surface unchanged, got {error:?}",
            );
            assert_fail_closed_residue_is_empty(&fx);
            assert_eq!(count(&fx.mtls.attempts), 1, "{expected:?}: one attempt");
            assert_eq!(fx.accept.await.expect("accept task"), 0, "{expected:?}: no byte sent");
        }
    }

    /// S-PIG-28..30 family: `NoBackend` resolves no identity, starts no mTLS
    /// and is not retried.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn no_backend_receipt_does_not_resolve_identity_start_mtls_or_retry() {
        let fx = fixture(ReceiptMode::NoBackend).await;
        let error = connect_error(&fx).await;
        assert!(matches!(error, GatewayUpstreamConnectError::NoBackend));
        assert_fail_closed_residue_is_empty(&fx);
        assert_eq!(count(&fx.mtls.attempts), 0);
        assert_eq!(count(&fx.dataplane.receipts_taken), 1);
        assert!(fx.dataplane.identity_lookups.lock().expect("identity log").is_empty());
        assert_eq!(fx.accept.await.expect("accept task"), 0, "no byte sent upstream");
    }

    /// S-PIG-28..30 family: a selected backend whose applied identity is
    /// missing fails before mTLS and cleans the exact intent.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn missing_applied_identity_fails_before_mtls_and_cleans_the_exact_intent() {
        let fx = fixture(ReceiptMode::Selected).await;
        fx.dataplane.identity_missing.store(true, Ordering::SeqCst);
        let backend_id = BackendId::new(7).expect("BackendId");
        let error = connect_error(&fx).await;
        assert!(matches!(
            error,
            GatewayUpstreamConnectError::SelectedIdentity(
                GatewayConnectDataplaneError::BackendIdentityMissing { backend_id: observed }
            ) if observed == backend_id
        ));
        assert_fail_closed_residue_is_empty(&fx);
        assert_eq!(count(&fx.mtls.attempts), 0);
        assert_eq!(fx.accept.await.expect("accept task"), 0, "no byte sent upstream");
    }

    /// S-PIG-37 (connector arm, R-5/DG-11): with the ledger at its 128-slot
    /// capacity the connector refuses before `open_upstream`: no socket is
    /// prepared, nothing is registered and nothing is dialed.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn full_cleanup_ledger_refuses_before_any_socket_is_prepared() {
        let mut fx = fixture(ReceiptMode::Selected).await;
        let held: Vec<GatewayCleanupReservation> =
            (0..128).map(|_| reserve_one(&fx.ledger)).collect();
        assert_eq!(fx.ledger.len(), 128);

        let error = connect_error(&fx).await;
        assert!(matches!(
            error,
            GatewayUpstreamConnectError::CleanupLedgerFull { capacity } if capacity.get() == 128
        ));
        assert_eq!(count(&fx.dataplane.opens), 0, "open_upstream is never called");
        assert_eq!(fx.dataplane.live_intent_count().expect("intent count"), 0);
        assert_eq!(fx.dataplane.live_receipts().expect("receipt count").len(), 0);
        assert_eq!(count(&fx.mtls.attempts), 0);
        assert!(fx.dataplane.cleanup_intents.lock().expect("cleanup log").is_empty());
        assert_eq!(fx.ledger.len(), 128, "the refusal consumed no slot");
        assert_eq!(
            fx.ledger.retry_all(fx.dataplane.as_ref()),
            vec![GatewayCleanupFailure::LedgerSaturated { attempts: 1, capacity: 128 }],
            "the refusal is counted as one saturation, with no retained cleanup",
        );

        assert_nothing_was_dialed(&mut fx.accept).await;
        drop(held);
        assert_eq!(fx.ledger.len(), 0, "dropping unbound reservations releases every slot");
    }

    /// S-PIG-37 (connector arm): an `open_upstream` failure releases the slot
    /// the connector reserved before the call.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn failing_open_upstream_releases_its_reserved_ledger_slot() {
        let mut fx = fixture(ReceiptMode::Selected).await;
        let held: Vec<GatewayCleanupReservation> =
            (0..3).map(|_| reserve_one(&fx.ledger)).collect();
        *fx.dataplane.open_failure.lock().expect("open failure") =
            Some(GatewayConnectDataplaneError::SocketBind(std::io::ErrorKind::AddrInUse));

        let error = connect_error(&fx).await;
        assert!(matches!(
            error,
            GatewayUpstreamConnectError::Prepare(GatewayConnectDataplaneError::SocketBind(
                std::io::ErrorKind::AddrInUse
            ))
        ));
        assert_eq!(count(&fx.dataplane.opens), 1);
        assert_eq!(
            *fx.dataplane.ledger_len_during_open.lock().expect("open log"),
            vec![4],
            "the slot was reserved before open_upstream ran",
        );
        assert_eq!(fx.ledger.len(), 3, "the failed open released its slot");
        assert_eq!(fx.dataplane.live_intent_count().expect("intent count"), 0);
        assert!(fx.dataplane.cleanup_intents.lock().expect("cleanup log").is_empty());
        assert_eq!(count(&fx.mtls.attempts), 0);
        assert!(
            fx.ledger.retry_all(fx.dataplane.as_ref()).is_empty(),
            "no saturation, no retention"
        );

        assert_nothing_was_dialed(&mut fx.accept).await;
        drop(held);
        assert_eq!(fx.ledger.len(), 0);
    }

    /// S-PIG-133: every refused upstream dial releases its ledger slot and its
    /// registered intent, so more refused dials than the ledger holds never
    /// make a later request fail with `CleanupLedgerFull`.
    ///
    /// The frontend is a real loopback port that was bound and then closed, so
    /// the kernel refuses each `connect(2)`. The double runs in
    /// `ReceiptMode::Missing`: the refusal carries no hook verdict. The
    /// connector takes the receipt once after the refused connect, finds it
    /// absent (`ReceiptMissing`) and classifies the pair (absent, refused) as
    /// `Connect`. The conversion-to-std failure arm of the connector is not
    /// reachable through a real socket and is covered by inspection.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn refused_upstream_dials_release_their_ledger_slot_and_intent() {
        /// More attempts than the 128-slot ledger holds.
        const ATTEMPTS: u32 = 130;
        let mut fx = fixture(ReceiptMode::Missing).await;
        close_the_frontend(&mut fx).await;

        for attempt in 1..=ATTEMPTS {
            let error = connect_error(&fx).await;
            assert!(
                matches!(error, GatewayUpstreamConnectError::Connect),
                "attempt {attempt}: a refused dial surfaces as Connect, got {error:?}",
            );
            assert_eq!(fx.ledger.len(), 0, "attempt {attempt}: the refused dial released its slot");
            assert_eq!(
                fx.dataplane.live_intent_count().expect("intent count"),
                0,
                "attempt {attempt}: the refused dial cleaned its registered intent",
            );
        }

        assert_eq!(count(&fx.dataplane.opens), ATTEMPTS, "every attempt prepared one upstream");
        assert_eq!(
            fx.dataplane.cleanup_intents.lock().expect("cleanup log").len(),
            ATTEMPTS as usize,
            "each registered intent is cleaned up exactly once",
        );
        assert_eq!(fx.dataplane.live_receipts().expect("receipt count").len(), 0);
        assert_eq!(
            count(&fx.dataplane.receipts_taken),
            ATTEMPTS,
            "the receipt is taken once after each refused connect, whatever its result",
        );
        assert_eq!(count(&fx.mtls.attempts), 0, "no handshake follows a refused dial");
        assert!(
            fx.ledger.retry_all(fx.dataplane.as_ref()).is_empty(),
            "no saturation and no retained cleanup",
        );
    }

    /// S-PIG-138: a refused dial whose hook selected a backend is a dial
    /// failure. The pair (`Selected`, refused) is `Connect` (502): the receipt
    /// is taken once, no identity is resolved, no handshake starts and no
    /// byte is sent.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn a_selected_backend_whose_dial_is_refused_is_a_connect_failure() {
        let mut fx = fixture(ReceiptMode::Selected).await;
        close_the_frontend(&mut fx).await;

        let error = connect_error(&fx).await;

        assert!(
            matches!(error, GatewayUpstreamConnectError::Connect),
            "(Selected, refused) is Connect, got {error:?}",
        );
        assert_fail_closed_residue_is_empty(&fx);
        assert_eq!(count(&fx.dataplane.receipts_taken), 1, "the receipt is taken exactly once");
        assert!(fx.dataplane.identity_lookups.lock().expect("identity log").is_empty());
        assert_eq!(count(&fx.mtls.attempts), 0, "no handshake follows a refused dial");
    }

    /// S-PIG-139: a refused dial whose hook denied the connect is
    /// `NoBackend` (503), never `Connect`: the receipt is the verdict, not the
    /// refusal. The connector takes the receipt after the refused connect,
    /// resolves no identity, starts no handshake and never waits on the clock.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn a_dial_the_hook_denied_is_no_backend_not_a_connect_failure() {
        let mut fx = fixture(ReceiptMode::NoBackend).await;
        close_the_frontend(&mut fx).await;
        let before = fx.clock.now();

        let error = connect_error(&fx).await;

        assert!(
            matches!(error, GatewayUpstreamConnectError::NoBackend),
            "(NoBackend, refused) is NoBackend, got {error:?}",
        );
        assert_fail_closed_residue_is_empty(&fx);
        assert_eq!(count(&fx.dataplane.receipts_taken), 1, "the receipt is taken exactly once");
        assert!(fx.dataplane.identity_lookups.lock().expect("identity log").is_empty());
        assert_eq!(count(&fx.mtls.attempts), 0, "no handshake follows a denied dial");
        assert_eq!(fx.clock.now(), before, "the denied dial never waited");
    }

    /// S-PIG-134: a connect future dropped while it is parked inside the
    /// gateway-client mTLS handshake still cleans its registered intent and
    /// completes its ledger record (R-63), so repeated cancellations never
    /// fill the ledger.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn a_connect_dropped_mid_handshake_releases_its_ledger_slot_and_intent() {
        let fx = fixture_with(ReceiptMode::Selected, Some(GatewayClientMtlsError::Handshake), true)
            .await;
        {
            let connect = fx.connector.connect(fx.frontend, deadline(&fx));
            tokio::pin!(connect);
            let parked = tokio::time::timeout(Duration::from_secs(5), fx.mtls.parked.notified());
            tokio::select! {
                _ = &mut connect => panic!("the held handshake cannot complete before release"),
                reached = parked => reached.expect("the connect reached the held authenticate"),
            }
            assert_eq!(fx.ledger.len(), 1, "the in-flight connect holds its reservation");
            assert_eq!(
                fx.dataplane.live_intent_count().expect("intent count"),
                1,
                "the in-flight connect holds its registered intent",
            );
            // The pinned future is dropped here, still suspended in `authenticate`.
        }

        assert_eq!(
            count(&fx.mtls.attempts),
            1,
            "the handshake was started once and never finished"
        );
        assert_eq!(fx.ledger.len(), 0, "the dropped connect released its ledger slot");
        assert_eq!(
            fx.dataplane.live_intent_count().expect("intent count"),
            0,
            "the dropped connect cleaned its registered intent",
        );
        assert_eq!(fx.dataplane.live_receipts().expect("receipt count").len(), 0);
        assert_eq!(count(&fx.dataplane.opens), 1, "exactly one upstream was prepared");
        assert_eq!(
            fx.dataplane.cleanup_intents.lock().expect("cleanup log").len(),
            1,
            "the registered intent is cleaned up exactly once",
        );
        assert!(
            fx.ledger.retry_all(fx.dataplane.as_ref()).is_empty(),
            "no saturation and no retained cleanup",
        );
        let received = tokio::time::timeout(Duration::from_secs(5), fx.accept)
            .await
            .expect("the dropped connect closed its upstream connection")
            .expect("accept task");
        assert_eq!(received, 0, "no byte sent upstream");
    }

    /// S-PIG-53: a failed cleanup is retained in the ledger under the record
    /// created at the pre-reservation and bound after `open_upstream`; the
    /// retry cleans the same intent without re-registering, reconnecting or
    /// re-authenticating.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn failed_cleanup_is_retained_and_retried_without_reconnecting_or_reauthenticating() {
        let fx = fixture(ReceiptMode::Selected).await;
        fx.dataplane.cleanup_failing.store(true, Ordering::SeqCst);

        let error = connect_error(&fx).await;
        assert!(matches!(
            error,
            GatewayUpstreamConnectError::Mtls(GatewayClientMtlsError::PeerSpiffeMismatch { .. })
        ));
        assert_eq!(
            *fx.dataplane.ledger_len_during_open.lock().expect("open log"),
            vec![1],
            "the record exists from the pre-reservation, before the intent is registered",
        );
        assert_eq!(fx.ledger.len(), 1, "failed cleanup is retained exactly once");
        assert_eq!(count(&fx.dataplane.opens), 1);
        assert_eq!(fx.dataplane.cleanup_intents.lock().expect("cleanup log").len(), 1);
        assert_eq!(count(&fx.mtls.attempts), 1);
        assert_eq!(fx.dataplane.live_intent_count().expect("intent count"), 1, "still registered");

        fx.dataplane.cleanup_failing.store(false, Ordering::SeqCst);
        assert!(fx.ledger.retry_all(fx.dataplane.as_ref()).is_empty());
        assert_eq!(fx.ledger.len(), 0);
        assert_eq!(count(&fx.dataplane.opens), 1, "no re-registration");
        assert_eq!(count(&fx.mtls.attempts), 1, "no re-authentication");
        assert_eq!(count(&fx.dataplane.receipts_taken), 1);
        let cleaned = fx.dataplane.cleanup_intents.lock().expect("cleanup log").clone();
        assert_eq!(cleaned.len(), 2, "one failed attempt and one retry");
        assert_eq!(cleaned[0], cleaned[1], "the retry cleans the exact retained intent");
        assert_eq!(fx.dataplane.live_intent_count().expect("intent count"), 0);
        assert_eq!(fx.dataplane.live_receipts().expect("receipt count").len(), 0);
        assert_eq!(fx.accept.await.expect("accept task"), 0, "no byte sent upstream");
    }

    /// S-PIG-69 (I half, connector level): while a request is in flight the
    /// guest-flow owner restarts or fail-stops; the mTLS leg dies with a
    /// `Handshake` failure. The connector surfaces that typed error, leaves
    /// no intent or receipt, cleans up once and does not retry.
    ///
    /// The public 502 projection of this error is the exhaustive connector
    /// status mapping asserted by the relay rows; the public-listener arm
    /// needs `SimGatewayRelay`.
    ///
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER public-request-end-to-end"]
    async fn owner_restart_mid_request_surfaces_the_typed_error_and_leaves_no_residue() {
        let fx = fixture_with(ReceiptMode::Selected, Some(GatewayClientMtlsError::Handshake), true)
            .await;
        let start = fx.clock.now();
        let drive = async {
            tokio::time::timeout(Duration::from_secs(5), fx.mtls.parked.notified())
                .await
                .expect("the request reached the held authenticate");
            // The owner restarts or fails-stop here; the request is in flight.
            fx.clock.tick(Duration::from_secs(1));
            fx.mtls.release.notify_one();
        };
        let (result, ()) = tokio::join!(fx.connector.connect(fx.frontend, deadline(&fx)), drive);
        let Err(error) = result else {
            panic!("an owner failure cannot yield an authenticated upstream");
        };
        assert!(matches!(
            error,
            GatewayUpstreamConnectError::Mtls(GatewayClientMtlsError::Handshake)
        ));
        assert_fail_closed_residue_is_empty(&fx);
        assert_eq!(count(&fx.mtls.attempts), 1, "no retry");
        assert_eq!(count(&fx.dataplane.receipts_taken), 1);
        assert!(fx.clock.now().duration_since(start) <= Duration::from_secs(5));
        assert_eq!(fx.accept.await.expect("accept task"), 0, "no byte sent upstream");
    }
}
