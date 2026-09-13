//! Intent/VIP-backed Service Frontend resolver.
//!
//! SCAFFOLD: true. Exact private constructor and public driven port compile;
//! DELIVER replaces the marked reads and validation.

#![expect(clippy::todo, reason = "public-ingress frontend resolver RED scaffold")]
#![allow(dead_code, reason = "activated by UnboundGatewayBuilder in DELIVER")]

use std::sync::Arc;

use async_trait::async_trait;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::public_ingress::ServiceListenerReference;
use overdrive_core::traits::{IntentStore, ServiceVipView};

use crate::ports::{ServiceFrontendResolve, ServiceFrontendResolveError};

pub(crate) struct IntentServiceFrontendResolver {
    _intent: Arc<dyn IntentStore>,
    _vip_view: Arc<dyn ServiceVipView>,
}

impl IntentServiceFrontendResolver {
    pub(crate) fn new(intent: Arc<dyn IntentStore>, vip_view: Arc<dyn ServiceVipView>) -> Self {
        Self { _intent: intent, _vip_view: vip_view }
    }
}

#[async_trait]
impl ServiceFrontendResolve for IntentServiceFrontendResolver {
    async fn probe(&self) -> Result<(), ServiceFrontendResolveError> {
        todo!("SCAFFOLD: IntentServiceFrontendResolver::probe")
    }
    async fn resolve(
        &self,
        _reference: &ServiceListenerReference,
    ) -> Result<ServiceFrontend, ServiceFrontendResolveError> {
        todo!("SCAFFOLD: IntentServiceFrontendResolver::resolve")
    }
}

#[cfg(test)]
mod acceptance {
    #![allow(clippy::doc_markdown, reason = "Contract Shape metadata")]

    use std::collections::BTreeMap;
    use std::num::NonZeroU16;
    use std::num::NonZeroU32;
    use std::sync::Arc;

    use bytes::Bytes;
    use futures::Stream;
    use overdrive_core::aggregate::{
        CronExpr, Exec, IntentKey, Job, Listener, Schedule, ServiceV2, WorkloadDriver,
        WorkloadIntent,
    };
    use overdrive_core::dataplane::Proto;
    use overdrive_core::id::{ContentHash, IdParseError, ServiceVip, WorkloadId};
    use overdrive_core::traits::driver::Resources;
    use overdrive_core::traits::intent_store::{
        IntentStore, IntentStoreError, IntentSubscriptionEvent, PutOutcome, StateSnapshot, TxnOp,
        TxnOutcome,
    };

    use super::*;

    struct ScriptedResolver {
        error: std::sync::Mutex<Option<ServiceFrontendResolveError>>,
    }
    type ErrorMatcher = fn(&ServiceFrontendResolveError) -> bool;

    struct BusyIntentStore;

    #[async_trait]
    impl IntentStore for BusyIntentStore {
        async fn get(&self, _key: &[u8]) -> Result<Option<Bytes>, IntentStoreError> {
            Err(IntentStoreError::Busy)
        }
        async fn put(&self, _key: &[u8], _value: &[u8]) -> Result<(), IntentStoreError> {
            Err(IntentStoreError::Busy)
        }
        async fn put_if_absent(
            &self,
            _key: &[u8],
            _value: &[u8],
        ) -> Result<PutOutcome, IntentStoreError> {
            Err(IntentStoreError::Busy)
        }
        async fn delete(&self, _key: &[u8]) -> Result<(), IntentStoreError> {
            Err(IntentStoreError::Busy)
        }
        async fn txn(&self, _ops: Vec<TxnOp>) -> Result<TxnOutcome, IntentStoreError> {
            Err(IntentStoreError::Busy)
        }
        async fn watch(
            &self,
            _prefix: &[u8],
        ) -> Result<Box<dyn Stream<Item = IntentSubscriptionEvent> + Send + Unpin>, IntentStoreError>
        {
            Err(IntentStoreError::Busy)
        }
        async fn scan_prefix(
            &self,
            _prefix: &[u8],
        ) -> Result<Vec<(Bytes, Bytes)>, IntentStoreError> {
            Err(IntentStoreError::Busy)
        }
        async fn export_snapshot(&self) -> Result<StateSnapshot, IntentStoreError> {
            Err(IntentStoreError::Busy)
        }
        async fn bootstrap_from(&self, _snapshot: StateSnapshot) -> Result<(), IntentStoreError> {
            Err(IntentStoreError::Busy)
        }
    }
    #[async_trait]
    impl ServiceFrontendResolve for ScriptedResolver {
        async fn probe(&self) -> Result<(), ServiceFrontendResolveError> {
            Ok(())
        }
        async fn resolve(
            &self,
            _reference: &ServiceListenerReference,
        ) -> Result<ServiceFrontend, ServiceFrontendResolveError> {
            Err(self.error.lock().expect("script lock").take().expect("one scripted result"))
        }
    }

    /// CONTRACT_SHAPE: unbounded-preservation.
    #[tokio::test]
    #[ignore = "pending DELIVER IntentServiceFrontendResolver closed error matrix"]
    async fn every_frontend_resolution_error_is_cause_distinct_and_jobs_are_rejected() {
        let service = WorkloadId::new("api").expect("Service ID");
        let reference = ServiceListenerReference::new(
            service.clone(),
            NonZeroU16::new(8080).expect("port"),
            Proto::Tcp,
        )
        .expect("reference");
        let cases: Vec<(ServiceFrontendResolveError, ErrorMatcher)> = vec![
            (ServiceFrontendResolveError::ServiceAbsent { service: service.clone() }, |error| {
                matches!(error, ServiceFrontendResolveError::ServiceAbsent { .. })
            }),
            (ServiceFrontendResolveError::ServiceIntent(IntentStoreError::Busy), |error| {
                matches!(error, ServiceFrontendResolveError::ServiceIntent(IntentStoreError::Busy))
            }),
            (ServiceFrontendResolveError::ServiceIntentDecode, |error| {
                matches!(error, ServiceFrontendResolveError::ServiceIntentDecode)
            }),
            (ServiceFrontendResolveError::TargetNotService { service: service.clone() }, |error| {
                matches!(error, ServiceFrontendResolveError::TargetNotService { .. })
            }),
            (
                ServiceFrontendResolveError::ListenerAbsent {
                    port: NonZeroU16::new(8080).expect("port"),
                    protocol: Proto::Tcp,
                },
                |error| matches!(error, ServiceFrontendResolveError::ListenerAbsent { .. }),
            ),
            (ServiceFrontendResolveError::ServiceVipUnavailable, |error| {
                matches!(error, ServiceFrontendResolveError::ServiceVipUnavailable)
            }),
            (
                ServiceFrontendResolveError::InvalidServiceFrontend(IdParseError::Empty {
                    kind: "ServiceFrontend",
                }),
                |error| matches!(error, ServiceFrontendResolveError::InvalidServiceFrontend(_)),
            ),
        ];
        for (scripted, expected) in cases {
            let resolver = ScriptedResolver { error: std::sync::Mutex::new(Some(scripted)) };
            let error = resolver.resolve(&reference).await.expect_err("closed error");
            assert!(expected(&error), "wrong frontend error partition: {error:?}");
        }
    }

    fn job(id: &WorkloadId) -> Job {
        Job {
            id: id.clone(),
            replicas: NonZeroU32::MIN,
            resources: Resources { cpu_milli: 10, memory_bytes: 16 * 1024 * 1024 },
            driver: WorkloadDriver::Exec(Exec {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
            }),
        }
    }

    fn service(id: &WorkloadId, listeners: Vec<Listener>) -> WorkloadIntent {
        WorkloadIntent::Service(ServiceV2 {
            id: id.clone(),
            replicas: NonZeroU32::MIN,
            resources: Resources { cpu_milli: 10, memory_bytes: 16 * 1024 * 1024 },
            driver: WorkloadDriver::Exec(Exec {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
            }),
            listeners,
            startup_probes: Vec::new(),
            readiness_probes: Vec::new(),
            liveness_probes: Vec::new(),
        })
    }

    fn reference(id: &WorkloadId) -> ServiceListenerReference {
        ServiceListenerReference::new(id.clone(), NonZeroU16::new(8080).expect("port"), Proto::Tcp)
            .expect("reference")
    }

    /// CONTRACT_SHAPE: unbounded-preservation.
    #[tokio::test]
    #[ignore = "pending DELIVER IntentServiceFrontendResolver real-intent adapter"]
    #[expect(clippy::too_many_lines, reason = "one exhaustive closed adapter-error matrix")]
    async fn every_frontend_error_is_driven_through_the_production_resolver_without_mutation() {
        let root = tempfile::tempdir().expect("isolated resolver store");
        let store = Arc::new(
            overdrive_store_local::LocalIntentStore::open(root.path().join("intent.redb"))
                .expect("real intent store"),
        );
        let empty_vips = || {
            Arc::new(overdrive_sim::adapters::read_ports::SimServiceVipView::new(BTreeMap::new()))
        };
        let resolver = IntentServiceFrontendResolver::new(store.clone(), empty_vips());
        let absent_id = WorkloadId::new("absent-target").expect("absent ID");
        assert!(matches!(
            resolver.resolve(&reference(&absent_id)).await,
            Err(ServiceFrontendResolveError::ServiceAbsent { service }) if service == absent_id
        ));

        let busy_resolver =
            IntentServiceFrontendResolver::new(Arc::new(BusyIntentStore), empty_vips());
        assert!(matches!(
            busy_resolver.resolve(&reference(&absent_id)).await,
            Err(ServiceFrontendResolveError::ServiceIntent(IntentStoreError::Busy))
        ));

        let malformed_id = WorkloadId::new("malformed-target").expect("malformed ID");
        store
            .put(IntentKey::for_workload(&malformed_id).as_bytes(), b"not-an-envelope")
            .await
            .expect("seed malformed intent");
        assert!(matches!(
            resolver.resolve(&reference(&malformed_id)).await,
            Err(ServiceFrontendResolveError::ServiceIntentDecode)
        ));

        let job_id = WorkloadId::new("job-target").expect("Job ID");
        let schedule_id = WorkloadId::new("schedule-target").expect("Schedule ID");
        let schedule_job = job(&schedule_id);
        let cases = [
            (job_id.clone(), WorkloadIntent::Job(job(&job_id))),
            (
                schedule_id.clone(),
                WorkloadIntent::Schedule(Schedule {
                    id: schedule_id.clone(),
                    job: schedule_job,
                    cron_expr: CronExpr::new("0 * * * *").expect("cron"),
                }),
            ),
        ];
        for (id, intent) in &cases {
            store
                .put(
                    IntentKey::for_workload(id).as_bytes(),
                    intent.archive_for_store().expect("archive intent").as_ref(),
                )
                .await
                .expect("seed non-Service intent");
        }
        let before_nonservice = store.scan_prefix(b"workloads/").await.expect("before snapshot");
        for (id, _) in cases {
            assert!(matches!(
                resolver.resolve(&reference(&id)).await,
                Err(ServiceFrontendResolveError::TargetNotService { service }) if service == id
            ));
        }
        assert_eq!(
            store.scan_prefix(b"workloads/").await.expect("after snapshot"),
            before_nonservice,
            "non-Service rejection is read-only over the intent universe",
        );

        let missing_listener_id = WorkloadId::new("missing-listener").expect("Service ID");
        let missing_listener = service(&missing_listener_id, Vec::new());
        store
            .put(
                IntentKey::for_workload(&missing_listener_id).as_bytes(),
                missing_listener.archive_for_store().expect("archive Service").as_ref(),
            )
            .await
            .expect("seed listener-absent Service");
        assert!(matches!(
            resolver.resolve(&reference(&missing_listener_id)).await,
            Err(ServiceFrontendResolveError::ListenerAbsent { port, protocol })
                if port.get() == 8080 && protocol == Proto::Tcp
        ));

        let listener =
            Listener { port: NonZeroU16::new(8080).expect("port"), protocol: Proto::Tcp };
        let no_vip_id = WorkloadId::new("missing-vip").expect("Service ID");
        let no_vip = service(&no_vip_id, vec![listener]);
        store
            .put(
                IntentKey::for_workload(&no_vip_id).as_bytes(),
                no_vip.archive_for_store().expect("archive Service").as_ref(),
            )
            .await
            .expect("seed VIP-absent Service");
        assert!(matches!(
            resolver.resolve(&reference(&no_vip_id)).await,
            Err(ServiceFrontendResolveError::ServiceVipUnavailable)
        ));

        let ipv6_id = WorkloadId::new("ipv6-vip").expect("Service ID");
        let ipv6 = service(&ipv6_id, vec![listener]);
        let ipv6_digest: ContentHash = ipv6.spec_digest().expect("Service digest");
        store
            .put(
                IntentKey::for_workload(&ipv6_id).as_bytes(),
                ipv6.archive_for_store().expect("archive Service").as_ref(),
            )
            .await
            .expect("seed IPv6 Service");
        let before_ipv6 = store.scan_prefix(b"workloads/").await.expect("pre-resolve snapshot");
        let ipv6_resolver = IntentServiceFrontendResolver::new(
            store.clone(),
            Arc::new(overdrive_sim::adapters::read_ports::SimServiceVipView::new(BTreeMap::from(
                [(
                    ipv6_digest,
                    ServiceVip::new("2001:db8::8".parse().expect("IPv6")).expect("Service VIP"),
                )],
            ))),
        );
        assert!(matches!(
            ipv6_resolver.resolve(&reference(&ipv6_id)).await,
            Err(ServiceFrontendResolveError::InvalidServiceFrontend(IdParseError::InvalidFormat {
                kind: "ServiceFrontend",
                ..
            }))
        ));

        assert_eq!(
            store.scan_prefix(b"workloads/").await.expect("final snapshot"),
            before_ipv6,
            "every frontend error preserves the complete intent universe",
        );
    }
}
