//! Registered gateway upstream connector.
//!
//! SCAFFOLD: true. The exact production entry points remain RED until DELIVER.
//! Source-local integration bodies implement S-PIG-28 through S-PIG-30.

#![expect(clippy::unused_async, reason = "exact async RED scaffold signature")]
#![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

use std::sync::Arc;
use std::time::Instant;

use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::traits::clock::Clock;
use overdrive_host::socket_cookie::{SocketCookieReadError, SocketCookieReader};

use crate::application::{GatewayAdmissionBudget, GatewayAdmissionError, GatewayCleanupLedger};
use crate::ports::{
    GatewayClientMtls, GatewayClientMtlsError, GatewayConnectDataplane,
    GatewayConnectDataplaneError, GatewayUpstream,
};
use crate::runtime::GatewayLimits;

pub(crate) struct GatewayUpstreamConnector;

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "DISTILL RED scaffold is activated by the public runtime")
)]
impl GatewayUpstreamConnector {
    pub(crate) fn new(
        _dataplane: Arc<dyn GatewayConnectDataplane>,
        _cookie_reader: Arc<SocketCookieReader>,
        _client_mtls: Arc<dyn GatewayClientMtls>,
        _limits: GatewayLimits,
        _clock: Arc<dyn Clock>,
        _budget: Arc<GatewayAdmissionBudget>,
        _cleanup_ledger: Arc<GatewayCleanupLedger>,
    ) -> Self {
        todo!("SCAFFOLD: GatewayUpstreamConnector::new")
    }

    pub(crate) async fn connect(
        &self,
        _frontend: ServiceFrontend,
        _deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayUpstreamConnectError> {
        todo!("SCAFFOLD: GatewayUpstreamConnector::connect")
    }

    #[expect(dead_code, reason = "activated by the DELIVER application gate")]
    pub(crate) async fn probe(
        &self,
        _frontend: ServiceFrontend,
        _deadline: Instant,
    ) -> Result<GatewayConnectProbeOutcome, GatewayUpstreamConnectError> {
        todo!("SCAFFOLD: GatewayUpstreamConnector::probe")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(dead_code, reason = "activated by the DELIVER application gate")]
pub(crate) enum GatewayConnectProbeOutcome {
    ReadyNoBackend,
    SelectedPeerAuthenticated,
}

#[derive(Debug, thiserror::Error)]
#[expect(dead_code, reason = "closed RED scaffold variants activate incrementally")]
pub(crate) enum GatewayUpstreamConnectError {
    #[error("gateway admission capacity exhausted")]
    Capacity(GatewayAdmissionError),
    #[error("gateway cleanup ledger is full")]
    CleanupLedgerFull { capacity: std::num::NonZeroU32 },
    #[error("gateway upstream socket create failed")]
    SocketCreate,
    #[error("gateway socket-cookie read failed: {0}")]
    Cookie(SocketCookieReadError),
    #[error("gateway connect intent registration failed: {0}")]
    Register(GatewayConnectDataplaneError),
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
    use std::os::fd::OwnedFd;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use async_trait::async_trait;
    use overdrive_core::dataplane::Proto;
    use overdrive_core::id::{BackendId, ServiceVip, SpiffeId};
    use overdrive_core::public_ingress::{
        GatewayConnectIntent, GatewaySelectionReceipt, ServiceKey,
    };
    use overdrive_sim::adapters::clock::SimClock;

    use super::*;
    use crate::ports::GatewayConnectCleanupSweep;

    #[derive(Clone, Copy)]
    enum ReceiptMode {
        Missing,
        Mismatched,
        Selected,
        NoBackend,
    }

    struct ReceiptDataplane {
        mode: ReceiptMode,
        registered: Mutex<Option<GatewayConnectIntent>>,
        receipt_live: AtomicBool,
        registrations: AtomicU32,
        cleanup_failure: AtomicBool,
        cleanup_calls: AtomicU32,
        selected_identity_missing: AtomicBool,
    }

    #[async_trait]
    impl GatewayConnectDataplane for ReceiptDataplane {
        async fn probe(
            &self,
            _deadline: Instant,
            _clock: &dyn Clock,
        ) -> Result<(), GatewayConnectDataplaneError> {
            Ok(())
        }

        fn register(
            &self,
            intent: GatewayConnectIntent,
        ) -> Result<(), GatewayConnectDataplaneError> {
            self.registered.lock().expect("intent lock").replace(intent);
            self.receipt_live.store(!matches!(self.mode, ReceiptMode::Missing), Ordering::SeqCst);
            self.registrations.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn take_receipt(
            &self,
            intent: &GatewayConnectIntent,
        ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
            let result = match self.mode {
                ReceiptMode::Missing => Err(GatewayConnectDataplaneError::ReceiptMissing),
                ReceiptMode::Mismatched => {
                    let other = ServiceFrontend::new(
                        ServiceVip::new("10.96.0.2".parse().expect("IP")).expect("VIP"),
                        std::num::NonZeroU16::new(8081).expect("port"),
                        Proto::Tcp,
                    )
                    .expect("frontend");
                    Ok(GatewaySelectionReceipt::Selected {
                        socket_cookie: intent.socket_cookie,
                        service_key: ServiceKey::from_frontend(other),
                        backend_id: BackendId::new(7).expect("BackendId"),
                    })
                }
                ReceiptMode::Selected => Ok(GatewaySelectionReceipt::Selected {
                    socket_cookie: intent.socket_cookie,
                    service_key: intent.service_key,
                    backend_id: BackendId::new(7).expect("BackendId"),
                }),
                ReceiptMode::NoBackend => Ok(GatewaySelectionReceipt::NoBackend {
                    socket_cookie: intent.socket_cookie,
                    service_key: intent.service_key,
                }),
            };
            if result.is_ok() {
                self.receipt_live.store(false, Ordering::SeqCst);
            }
            result
        }

        fn cleanup(
            &self,
            _intent: &GatewayConnectIntent,
        ) -> Result<(), GatewayConnectDataplaneError> {
            self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
            if self.cleanup_failure.load(Ordering::SeqCst) {
                return Err(GatewayConnectDataplaneError::Cleanup);
            }
            self.registered.lock().expect("intent lock").take();
            self.receipt_live.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn cleanup_all_gateway_intents(
            &self,
        ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
            let removed_intents =
                u32::from(self.registered.lock().expect("intent lock").take().is_some());
            let removed_receipts = u32::from(self.receipt_live.swap(false, Ordering::SeqCst));
            Ok(GatewayConnectCleanupSweep { removed_intents, removed_receipts })
        }

        fn selected_backend_identity(
            &self,
            backend_id: BackendId,
        ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
            if self.selected_identity_missing.load(Ordering::SeqCst) {
                return Err(GatewayConnectDataplaneError::BackendIdentityMissing { backend_id });
            }
            SpiffeId::new("spiffe://overdrive.local/workload/api/alloc/api-0")
                .map_err(|_| GatewayConnectDataplaneError::Unavailable)
        }

        fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
            Ok(u32::from(self.registered.lock().expect("intent lock").is_some()))
        }
        fn live_receipt_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
            Ok(u32::from(self.receipt_live.load(Ordering::SeqCst)))
        }
    }

    struct FaultingMtls {
        attempts: AtomicU32,
        error: Mutex<Option<GatewayClientMtlsError>>,
    }
    #[async_trait]
    impl GatewayClientMtls for FaultingMtls {
        async fn authenticate(
            &self,
            connected: OwnedFd,
            expected_peer: SpiffeId,
            _deadline: Instant,
        ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            drop(connected);
            let error = self
                .error
                .lock()
                .expect("mTLS fault lock")
                .take()
                .expect("one originating mTLS fault");
            if let GatewayClientMtlsError::PeerSpiffeMismatch { actual, .. } = error {
                return Err(GatewayClientMtlsError::PeerSpiffeMismatch {
                    expected: expected_peer,
                    actual,
                });
            }
            Err(error)
        }
    }

    async fn connector(
        mode: ReceiptMode,
    ) -> (
        GatewayUpstreamConnector,
        ServiceFrontend,
        tokio::task::JoinHandle<()>,
        Arc<ReceiptDataplane>,
        Arc<FaultingMtls>,
        Arc<GatewayCleanupLedger>,
    ) {
        connector_with_mtls_error(
            mode,
            GatewayClientMtlsError::PeerSpiffeMismatch {
                expected: SpiffeId::new("spiffe://overdrive.local/workload/api/alloc/api-0")
                    .expect("selected peer"),
                actual: SpiffeId::new("spiffe://overdrive.local/workload/other/alloc/other-0")
                    .expect("wrong-valid peer"),
            },
        )
        .await
    }

    async fn connector_with_mtls_error(
        mode: ReceiptMode,
        mtls_error: GatewayClientMtlsError,
    ) -> (
        GatewayUpstreamConnector,
        ServiceFrontend,
        tokio::task::JoinHandle<()>,
        Arc<ReceiptDataplane>,
        Arc<FaultingMtls>,
        Arc<GatewayCleanupLedger>,
    ) {
        let listener =
            tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("real loopback listener");
        let addr = listener.local_addr().expect("listener address");
        let accept = tokio::spawn(async move {
            let _ = listener.accept().await.expect("connector reached listener");
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
        let dataplane = Arc::new(ReceiptDataplane {
            mode,
            registered: Mutex::new(None),
            receipt_live: AtomicBool::new(false),
            registrations: AtomicU32::new(0),
            cleanup_failure: AtomicBool::new(false),
            cleanup_calls: AtomicU32::new(0),
            selected_identity_missing: AtomicBool::new(false),
        });
        let mtls = Arc::new(FaultingMtls {
            attempts: AtomicU32::new(0),
            error: Mutex::new(Some(mtls_error)),
        });
        let connector = GatewayUpstreamConnector::new(
            dataplane.clone(),
            Arc::new(SocketCookieReader::new()),
            mtls.clone(),
            limits,
            Arc::new(SimClock::new()),
            budget,
            ledger.clone(),
        );
        (connector, frontend, accept, dataplane, mtls, ledger)
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER GatewayClientMtls caller mapping"]
    async fn every_gateway_client_mtls_fault_is_mapped_by_the_connector_and_cleans_intent_state() {
        let expected_peer =
            SpiffeId::new("spiffe://overdrive.local/workload/api/alloc/api-0").expect("peer A");
        let actual_peer =
            SpiffeId::new("spiffe://overdrive.local/workload/api/alloc/api-b").expect("peer B");
        for originating in [
            GatewayClientMtlsError::IdentityAbsent,
            GatewayClientMtlsError::IdentityExpired,
            GatewayClientMtlsError::PeerAddress,
            GatewayClientMtlsError::HandshakeTimeout,
            GatewayClientMtlsError::Handshake,
            GatewayClientMtlsError::PeerCertificateMissing,
            GatewayClientMtlsError::PeerSpiffeShape,
            GatewayClientMtlsError::PeerSpiffeMismatch {
                expected: expected_peer,
                actual: actual_peer,
            },
        ] {
            let expected = originating.clone();
            let (connector, frontend, accept, dataplane, mtls, ledger) =
                connector_with_mtls_error(ReceiptMode::Selected, originating).await;
            let error = match connector
                .connect(frontend, Instant::now() + std::time::Duration::from_secs(5))
                .await
            {
                Ok(_) => panic!("the originating mTLS fault cannot yield an upstream"),
                Err(error) => error,
            };
            assert!(matches!(
                error,
                GatewayUpstreamConnectError::Mtls(observed) if observed == expected
            ));
            assert_eq!(dataplane.registrations.load(Ordering::SeqCst), 1);
            assert_eq!(dataplane.cleanup_calls.load(Ordering::SeqCst), 1);
            assert_eq!(dataplane.live_intent_count().expect("intent count"), 0);
            assert_eq!(dataplane.live_receipt_count().expect("receipt count"), 0);
            assert_eq!(ledger.len(), 0);
            assert_eq!(mtls.attempts.load(Ordering::SeqCst), 1);
            accept.await.expect("accept task");
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER connector receipt validation step"]
    async fn missing_selection_receipt_fails_closed_and_cleans_the_intent() {
        let (connector, frontend, accept, dataplane, mtls, _ledger) =
            connector(ReceiptMode::Missing).await;
        let error = match connector
            .connect(frontend, Instant::now() + std::time::Duration::from_secs(5))
            .await
        {
            Ok(_) => panic!("missing receipt cannot produce an upstream"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            GatewayUpstreamConnectError::Receipt(GatewayConnectDataplaneError::ReceiptMissing)
        ));
        assert_eq!(dataplane.live_intent_count().expect("intent count"), 0);
        assert_eq!(dataplane.live_receipt_count().expect("receipt count"), 0);
        assert_eq!(dataplane.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(mtls.attempts.load(Ordering::SeqCst), 0);
        accept.await.expect("accept task");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER connector receipt validation step"]
    async fn mismatched_service_key_receipt_fails_closed_before_mtls() {
        let (connector, frontend, accept, dataplane, mtls, _ledger) =
            connector(ReceiptMode::Mismatched).await;
        let error = match connector
            .connect(frontend, Instant::now() + std::time::Duration::from_secs(5))
            .await
        {
            Ok(_) => panic!("mismatched receipt cannot produce an upstream"),
            Err(error) => error,
        };
        assert!(matches!(error, GatewayUpstreamConnectError::ReceiptMismatch));
        assert_eq!(dataplane.live_intent_count().expect("intent count"), 0);
        assert_eq!(dataplane.live_receipt_count().expect("receipt count"), 0);
        assert_eq!(dataplane.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(mtls.attempts.load(Ordering::SeqCst), 0);
        accept.await.expect("accept task");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER gateway-client exact-peer mTLS step"]
    async fn wrong_valid_selected_peer_fails_closed_without_an_alternate_backend() {
        let (connector, frontend, accept, dataplane, mtls, _ledger) =
            connector(ReceiptMode::Selected).await;
        let error = match connector
            .connect(frontend, Instant::now() + std::time::Duration::from_secs(5))
            .await
        {
            Ok(_) => panic!("wrong-valid peer cannot produce an upstream"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            GatewayUpstreamConnectError::Mtls(GatewayClientMtlsError::PeerSpiffeMismatch { .. })
        ));
        assert_eq!(dataplane.live_intent_count().expect("intent count"), 0);
        assert_eq!(dataplane.live_receipt_count().expect("receipt count"), 0);
        assert_eq!(dataplane.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(mtls.attempts.load(Ordering::SeqCst), 1, "single upstream attempt");
        accept.await.expect("accept task");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER connector NoBackend single-attempt mapping"]
    async fn no_backend_receipt_does_not_resolve_identity_start_mtls_or_retry() {
        let (connector, frontend, accept, dataplane, mtls, _ledger) =
            connector(ReceiptMode::NoBackend).await;
        let error = match connector
            .connect(frontend, Instant::now() + std::time::Duration::from_secs(5))
            .await
        {
            Ok(_) => panic!("NoBackend cannot produce an upstream"),
            Err(error) => error,
        };
        assert!(matches!(error, GatewayUpstreamConnectError::NoBackend));
        assert_eq!(dataplane.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(dataplane.cleanup_calls.load(Ordering::SeqCst), 1);
        assert_eq!(dataplane.live_intent_count().expect("intent count"), 0);
        assert_eq!(dataplane.live_receipt_count().expect("receipt count"), 0);
        assert_eq!(mtls.attempts.load(Ordering::SeqCst), 0);
        accept.await.expect("accept task");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER selected BackendId applied-identity lookup"]
    async fn missing_applied_identity_fails_before_mtls_and_cleans_the_exact_intent() {
        let (connector, frontend, accept, dataplane, mtls, _ledger) =
            connector(ReceiptMode::Selected).await;
        dataplane.selected_identity_missing.store(true, Ordering::SeqCst);
        let error = match connector
            .connect(frontend, Instant::now() + std::time::Duration::from_secs(5))
            .await
        {
            Ok(_) => panic!("missing applied identity cannot produce an upstream"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            GatewayUpstreamConnectError::SelectedIdentity(
                GatewayConnectDataplaneError::BackendIdentityMissing { .. }
            )
        ));
        assert_eq!(dataplane.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(dataplane.cleanup_calls.load(Ordering::SeqCst), 1);
        assert_eq!(dataplane.live_intent_count().expect("intent count"), 0);
        assert_eq!(dataplane.live_receipt_count().expect("receipt count"), 0);
        assert_eq!(mtls.attempts.load(Ordering::SeqCst), 0);
        accept.await.expect("accept task");
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER connector cleanup-ledger retry"]
    async fn cleanup_failure_is_retained_and_retried_without_reconnecting_or_reauthenticating() {
        let (connector, frontend, accept, dataplane, mtls, ledger) =
            connector(ReceiptMode::Selected).await;
        dataplane.cleanup_failure.store(true, Ordering::SeqCst);
        let error = match connector
            .connect(frontend, Instant::now() + std::time::Duration::from_secs(5))
            .await
        {
            Ok(_) => panic!("wrong-valid peer remains rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            GatewayUpstreamConnectError::Mtls(GatewayClientMtlsError::PeerSpiffeMismatch { .. })
        ));
        assert_eq!(ledger.len(), 1, "failed cleanup is retained exactly once");
        assert_eq!(dataplane.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(dataplane.cleanup_calls.load(Ordering::SeqCst), 1);
        assert_eq!(mtls.attempts.load(Ordering::SeqCst), 1);

        dataplane.cleanup_failure.store(false, Ordering::SeqCst);
        assert!(ledger.retry_all(dataplane.as_ref()).is_empty());
        assert_eq!(ledger.len(), 0);
        assert_eq!(dataplane.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(dataplane.cleanup_calls.load(Ordering::SeqCst), 2);
        assert_eq!(mtls.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(dataplane.live_intent_count().expect("intent count"), 0);
        assert_eq!(dataplane.live_receipt_count().expect("receipt count"), 0);
        accept.await.expect("accept task");
    }
}
