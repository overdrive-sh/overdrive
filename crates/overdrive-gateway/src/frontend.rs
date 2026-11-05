//! Intent/VIP-backed Service Frontend resolver.
//!
//! Reads only the Service intent and the VIP view; nothing is written.

use std::sync::Arc;

use async_trait::async_trait;
use overdrive_core::aggregate::{IntentKey, WorkloadIntent, WorkloadIntentEnvelope};
use overdrive_core::codec::decode_envelope_bytes;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::public_ingress::ServiceListenerReference;
use overdrive_core::traits::{IntentStore, ServiceVipView};

use crate::ports::{ServiceFrontendResolve, ServiceFrontendResolveError};

pub(crate) struct IntentServiceFrontendResolver {
    intent: Arc<dyn IntentStore>,
    vip_view: Arc<dyn ServiceVipView>,
}

impl IntentServiceFrontendResolver {
    pub(crate) fn new(intent: Arc<dyn IntentStore>, vip_view: Arc<dyn ServiceVipView>) -> Self {
        Self { intent, vip_view }
    }
}

#[async_trait]
impl ServiceFrontendResolve for IntentServiceFrontendResolver {
    async fn probe(&self) -> Result<(), ServiceFrontendResolveError> {
        self.intent
            .scan_prefix(IntentKey::workload_prefix())
            .await
            .map(|_| ())
            .map_err(ServiceFrontendResolveError::ServiceIntent)
    }

    async fn resolve(
        &self,
        reference: &ServiceListenerReference,
    ) -> Result<ServiceFrontend, ServiceFrontendResolveError> {
        let service = reference.service();
        let bytes = self
            .intent
            .get(IntentKey::for_workload(service).as_bytes())
            .await
            .map_err(ServiceFrontendResolveError::ServiceIntent)?
            .ok_or_else(|| ServiceFrontendResolveError::ServiceAbsent {
                service: service.clone(),
            })?;
        let intent = decode_envelope_bytes::<WorkloadIntentEnvelope>(&bytes)
            .map_err(|_| ServiceFrontendResolveError::ServiceIntentDecode)?;
        let WorkloadIntent::Service(declared) = &intent else {
            return Err(ServiceFrontendResolveError::TargetNotService { service: service.clone() });
        };
        let (port, protocol) = (reference.port(), reference.protocol());
        if !declared.listeners.iter().any(|l| l.port == port && l.protocol == protocol) {
            return Err(ServiceFrontendResolveError::ListenerAbsent { port, protocol });
        }
        let digest =
            intent.spec_digest().map_err(|_| ServiceFrontendResolveError::ServiceIntentDecode)?;
        let vip = self
            .vip_view
            .assigned_vip(&digest)
            .await
            .ok_or(ServiceFrontendResolveError::ServiceVipUnavailable)?;
        ServiceFrontend::new(vip, port, protocol)
            .map_err(ServiceFrontendResolveError::InvalidServiceFrontend)
    }
}

#[cfg(test)]
mod acceptance {
    //! S-PIG-49: Service-frontend resolution closed errors.
    #![allow(
        clippy::doc_markdown,
        clippy::too_many_lines,
        reason = "CONTRACT_SHAPE metadata and one exhaustive closed adapter-error matrix"
    )]

    use std::collections::{BTreeMap, BTreeSet};
    use std::num::{NonZeroU16, NonZeroU32};

    use bytes::Bytes;
    use futures::Stream;
    use overdrive_core::aggregate::{
        CronExpr, IntentKey, Job, Listener, Schedule, ServiceV1, Vm, WorkloadDriver, WorkloadIntent,
    };
    use overdrive_core::dataplane::Proto;
    use overdrive_core::id::{ContentHash, IdParseError, ServiceVip, WorkloadId};
    use overdrive_core::traits::driver::Resources;
    use overdrive_core::traits::intent_store::{
        IntentStoreError, IntentSubscriptionEvent, PutOutcome, StateSnapshot, TxnOp, TxnOutcome,
    };

    use super::*;

    /// An `IntentStore` whose every operation reports `Busy`.
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

    /// A resolver double that returns one scripted closed error.
    struct ScriptedResolver {
        error: std::sync::Mutex<Option<ServiceFrontendResolveError>>,
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

    fn vm() -> WorkloadDriver {
        WorkloadDriver::Vm(Vm {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        })
    }

    fn resources() -> Resources {
        Resources { cpu_milli: 10, memory_bytes: 16 * 1024 * 1024 }
    }

    fn job(id: &WorkloadId) -> Job {
        Job { id: id.clone(), replicas: NonZeroU32::MIN, resources: resources(), driver: vm() }
    }

    fn service(id: &WorkloadId, listeners: Vec<Listener>) -> WorkloadIntent {
        WorkloadIntent::Service(ServiceV1 {
            id: id.clone(),
            replicas: NonZeroU32::MIN,
            resources: resources(),
            driver: vm(),
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

    async fn universe(store: &dyn IntentStore) -> Vec<(Bytes, Bytes)> {
        store.scan_prefix(IntentKey::workload_prefix()).await.expect("intent universe scan")
    }

    async fn seed(store: &dyn IntentStore, id: &WorkloadId, intent: &WorkloadIntent) {
        store
            .put(
                IntentKey::for_workload(id).as_bytes(),
                intent.archive_for_store().expect("archive intent").as_ref(),
            )
            .await
            .expect("seed intent");
    }

    /// S-PIG-49: the seven closed resolution errors are cause-distinct: no
    /// two share a rendered cause, and each keeps its own typed payload.
    ///
    /// CONTRACT_SHAPE: unbounded-preservation.
    #[tokio::test]
    async fn frontend_resolution_errors_are_cause_distinct() {
        let service_id = WorkloadId::new("api").expect("Service ID");
        let listener_reference = reference(&service_id);
        let scripted = vec![
            ServiceFrontendResolveError::ServiceAbsent { service: service_id.clone() },
            ServiceFrontendResolveError::ServiceIntent(IntentStoreError::Busy),
            ServiceFrontendResolveError::ServiceIntentDecode,
            ServiceFrontendResolveError::TargetNotService { service: service_id.clone() },
            ServiceFrontendResolveError::ListenerAbsent {
                port: NonZeroU16::new(8080).expect("port"),
                protocol: Proto::Tcp,
            },
            ServiceFrontendResolveError::ServiceVipUnavailable,
            ServiceFrontendResolveError::InvalidServiceFrontend(IdParseError::Empty {
                kind: "ServiceFrontend",
            }),
        ];
        let mut causes = BTreeSet::new();
        for error in scripted {
            let resolver = ScriptedResolver { error: std::sync::Mutex::new(Some(error)) };
            let observed = resolver.resolve(&listener_reference).await.expect_err("closed error");
            assert!(
                causes.insert(observed.to_string()),
                "cause {observed:?} renders the same as an earlier closed error",
            );
        }
        assert_eq!(causes.len(), 7, "seven closed causes, each with its own rendering");
    }

    /// S-PIG-49: the production resolver over a real redb intent store maps
    /// each failure to its own closed error, rejects a real persisted Job and
    /// Schedule as `TargetNotService`, and no error mutates the intent
    /// universe.
    ///
    /// CONTRACT_SHAPE: unbounded-preservation.
    #[tokio::test]
    async fn production_resolver_rejects_every_failure_cause_distinctly_without_mutation() {
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
        assert!(universe(store.as_ref()).await.is_empty(), "absent target writes nothing");

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
        let non_services = [
            (job_id.clone(), WorkloadIntent::Job(job(&job_id))),
            (
                schedule_id.clone(),
                WorkloadIntent::Schedule(Schedule {
                    id: schedule_id.clone(),
                    job: job(&schedule_id),
                    cron_expr: CronExpr::new("0 * * * *").expect("cron"),
                }),
            ),
        ];
        for (id, intent) in &non_services {
            seed(store.as_ref(), id, intent).await;
        }
        let before_non_services = universe(store.as_ref()).await;
        for (id, _) in &non_services {
            assert!(matches!(
                resolver.resolve(&reference(id)).await,
                Err(ServiceFrontendResolveError::TargetNotService { service }) if service == *id
            ));
        }
        assert_eq!(
            universe(store.as_ref()).await,
            before_non_services,
            "non-Service rejection is read-only over the intent universe",
        );

        let listener =
            Listener { port: NonZeroU16::new(8080).expect("port"), protocol: Proto::Tcp };
        let missing_listener_id = WorkloadId::new("missing-listener").expect("Service ID");
        seed(store.as_ref(), &missing_listener_id, &service(&missing_listener_id, Vec::new()))
            .await;
        assert!(matches!(
            resolver.resolve(&reference(&missing_listener_id)).await,
            Err(ServiceFrontendResolveError::ListenerAbsent { port, protocol })
                if port.get() == 8080 && protocol == Proto::Tcp
        ));

        let no_vip_id = WorkloadId::new("missing-vip").expect("Service ID");
        seed(store.as_ref(), &no_vip_id, &service(&no_vip_id, vec![listener])).await;
        assert!(matches!(
            resolver.resolve(&reference(&no_vip_id)).await,
            Err(ServiceFrontendResolveError::ServiceVipUnavailable)
        ));

        let ipv6_id = WorkloadId::new("ipv6-vip").expect("Service ID");
        let ipv6 = service(&ipv6_id, vec![listener]);
        let ipv6_digest: ContentHash = ipv6.spec_digest().expect("Service digest");
        seed(store.as_ref(), &ipv6_id, &ipv6).await;
        let before_ipv6 = universe(store.as_ref()).await;
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
            universe(store.as_ref()).await,
            before_ipv6,
            "every frontend error preserves the complete intent universe",
        );
    }
}
