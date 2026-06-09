//! Behavior-equivalence pins for the `ServiceMapHydrator` read-path
//! switch (ADR-0062 § Decision (3); feature-delta sub-decisions 3-5).
//!
//! The hydrator's source of the per-listener `(port, protocol)` fact
//! moves from the per-tick cluster scan
//! (`gather_service_listener_facts`, deleted in step 01-04) to the
//! in-memory keyed [`ListenerFactStore`]. These tests pin the OBSERVABLE
//! behavior across the switch so the source change is provably
//! semantics-preserving:
//!
//! * BE-1 — a `service_backends` row WHOSE `ServiceId` has a keyed fact
//!   projects a desired carrying the right `(port, protocol)`, identical
//!   to the pre-change projection.
//! * BE-2 — a `service_backends` row WHOSE `ServiceId` has NO keyed fact
//!   is skipped: NO `ServiceDesired` is produced, and crucially no
//!   silently-defaulted `Proto::Tcp` entry leaks (ADR-0060 C3 verbatim).
//! * BE-3 — distinct VIPs derive distinct `ServiceId`s with no
//!   collision: the keyed store's primary-entry count equals the total
//!   listener count across all services.
//!
//! Tier 1, default lane — pure in-process `SimObservationStore` +
//! `LocalIntentStore` over a `TempDir`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::num::NonZeroU16;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_control_plane::AppState;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::SpiffeId;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Listener, ResourcesInput, ServiceV1, WorkloadIntent,
    WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::reconcilers::{AnyReconciler, AnyState, ServiceMapHydrator, TargetResource};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationRow, ObservationStore, ServiceBackendRow,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use proptest::prelude::*;
use tempfile::TempDir;

const SERVICE_MAP_PURPOSE: &str = "service-map";

fn node_id(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid NodeId")
}

fn hydrator_reconciler() -> AnyReconciler {
    AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical(
        std::net::Ipv4Addr::UNSPECIFIED,
    ))
}

const fn proto_str(p: Proto) -> &'static str {
    match p {
        Proto::Tcp => "tcp",
        Proto::Udp => "udp",
    }
}

fn build_app_state(tmp: &TempDir, obs: Arc<dyn ObservationStore>) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        node_id("writer-1"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

async fn persist_and_allocate(
    state: &AppState,
    workload: &str,
    listeners: &[Listener],
) -> ServiceVip {
    let listener_inputs: Vec<ListenerInput> = listeners
        .iter()
        .map(|l| ListenerInput { port: l.port.get(), protocol: proto_str(l.protocol).to_string() })
        .collect();
    let svc = ServiceV1::from_submit(ServiceSpecInput {
        id: workload.to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/serve".to_string(), args: vec![] }),
        listeners: listener_inputs,
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    })
    .expect("valid service spec");
    let intent = WorkloadIntent::Service(svc.clone());
    let key = IntentKey::for_workload(&svc.id);
    let archived = intent.archive_for_store().expect("rkyv archive");
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put intent");
    let kind_key = IntentKey::for_workload_kind(&svc.id);
    state
        .store
        .put(kind_key.as_bytes(), &[WorkloadKind::Service.discriminator_byte()])
        .await
        .expect("put kind");

    let digest = intent.spec_digest().expect("spec_digest");
    let bytes: [u8; 32] = *digest.as_bytes();
    let mut guard = state.allocator.lock().await;
    let vip = guard.allocate(bytes).await.expect("allocate vip");
    drop(guard);
    vip
}

fn one_backend(workload: &str) -> Backend {
    Backend {
        alloc: SpiffeId::from_str(&format!("spiffe://overdrive.local/job/{workload}/alloc/a1"))
            .expect("spiffe"),
        addr: SocketAddr::from_str("10.9.9.9:8080").expect("addr"),
        weight: 100,
        healthy: true,
    }
}

fn proto_strategy() -> impl Strategy<Value = Proto> {
    prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
}

prop_compose! {
    fn listener_strategy()(port in 1u16..=65535, protocol in proto_strategy()) -> Listener {
        Listener { port: NonZeroU16::new(port).expect("port 1..=65535 is non-zero"), protocol }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// BE-1 — a `service_backends` row whose `ServiceId` has a keyed
    /// fact projects a desired carrying exactly that `(port, protocol)`.
    /// This is the post-switch equivalent of the pre-change projection
    /// (which sourced the same `(port, protocol)` from the cluster
    /// scan's `ListenerRow`).
    #[test]
    fn hydrate_desired_with_fact_matches_pre_change_projection(listener in listener_strategy()) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all().build().expect("rt");
        rt.block_on(async move {
            let tmp = TempDir::new().expect("tmpdir");
            let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 42));
            let state = build_app_state(&tmp, obs.clone() as Arc<dyn ObservationStore>);

            let workload = "be1";
            let wid = WorkloadId::new(workload).expect("wid");
            let listeners = vec![listener];
            let vip = persist_and_allocate(&state, workload, &listeners).await;
            let vip_addr = vip.try_as_ipv4().expect("ipv4");

            {
                let mut facts = state.listener_facts.lock().await;
                facts.upsert(wid.clone(), &vip, &listeners);
            }

            let sid = ServiceId::derive(&vip, listener.port, listener.protocol, SERVICE_MAP_PURPOSE);
            let backends = vec![one_backend(workload)];
            let row = ServiceBackendRow {
                service_id: sid,
                vip: vip_addr,
                backends: backends.clone(),
                updated_at: LogicalTimestamp { counter: 1, writer: node_id("writer-1") },
            };
            obs.write(ObservationRow::ServiceBackend(row)).await.expect("write row");

            let target = TargetResource::new(&format!("service/{sid}")).expect("target");
            let hydrated =
                overdrive_control_plane::reconciler_runtime::hydrate_desired_for_test(
                    &hydrator_reconciler(), &target, &state,
                ).await.expect("hydrate ok");
            let smh = match hydrated {
                AnyState::ServiceMapHydrator(s) => s,
                other => panic!("expected ServiceMapHydrator, got {other:?}"),
            };

            prop_assert_eq!(smh.desired.len(), 1, "exactly one service projected");
            let desired = smh.desired.get(&sid).expect("desired entry");
            prop_assert_eq!(desired.port, listener.port, "port from keyed fact");
            prop_assert_eq!(desired.proto, listener.protocol, "proto from keyed fact (C3)");
            prop_assert_eq!(desired.vip, vip, "vip matches allocator-issued");
            prop_assert_eq!(&desired.backends, &backends, "backends from the obs row");
            let expected_fp =
                overdrive_core::dataplane::fingerprint::fingerprint(&vip, &backends);
            prop_assert_eq!(desired.fingerprint, expected_fp, "fingerprint canonical");
            Ok(())
        })?;
    }
}

/// BE-2 — a `service_backends` row whose `ServiceId` has NO keyed fact
/// is SKIPPED: hydrate produces an EMPTY desired. Crucially no
/// silently-defaulted `Proto::Tcp` entry leaks — the C3 guard is
/// preserved verbatim across the read-path switch. Single-example
/// (the contract is the absence of an entry, not a quantified range).
#[tokio::test]
async fn hydrate_desired_unresolvable_proto_skips_and_emits_no_tcp_default() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 42));
    let state = build_app_state(&tmp, obs.clone() as Arc<dyn ObservationStore>);

    // Allocate a VIP + derive the ServiceId, write the service_backends
    // row — but DO NOT populate the listener_facts store. The keyed read
    // resolves None ⇒ the service must be skipped (no Tcp default).
    let workload = "be2";
    let listeners =
        vec![Listener { port: NonZeroU16::new(443).expect("nz"), protocol: Proto::Udp }];
    let vip = persist_and_allocate(&state, workload, &listeners).await;
    let vip_addr = vip.try_as_ipv4().expect("ipv4");
    let sid =
        ServiceId::derive(&vip, listeners[0].port, listeners[0].protocol, SERVICE_MAP_PURPOSE);
    let row = ServiceBackendRow {
        service_id: sid,
        vip: vip_addr,
        backends: vec![one_backend(workload)],
        updated_at: LogicalTimestamp { counter: 1, writer: node_id("writer-1") },
    };
    obs.write(ObservationRow::ServiceBackend(row)).await.expect("write row");

    let target = TargetResource::new(&format!("service/{sid}")).expect("target");
    let hydrated = overdrive_control_plane::reconciler_runtime::hydrate_desired_for_test(
        &hydrator_reconciler(),
        &target,
        &state,
    )
    .await
    .expect("hydrate ok");
    let smh = match hydrated {
        AnyState::ServiceMapHydrator(s) => s,
        other => panic!("expected ServiceMapHydrator, got {other:?}"),
    };

    assert!(
        smh.desired.is_empty(),
        "a row with no keyed listener fact must be skipped — never defaulted to Tcp (C3)"
    );
}

prop_compose! {
    fn listeners_strategy()(
        listeners in prop::collection::vec(listener_strategy(), 1..=3),
    ) -> Vec<Listener> {
        let mut seen = BTreeSet::new();
        listeners.into_iter().filter(|l| seen.insert(l.port)).collect()
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    /// BE-3 — distinct service VIPs derive distinct `ServiceId`s with no
    /// collision: after upserting S services (each with its own
    /// allocator-issued VIP + L listeners), the keyed store's primary
    /// entry count equals the total listener count across all services.
    /// A `ServiceId` collision (two listeners deriving the same id)
    /// would shrink the primary count below the listener total.
    #[test]
    fn distinct_service_vips_derive_distinct_service_ids_no_collision(
        services in prop::collection::vec(listeners_strategy(), 1..=6),
    ) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all().build().expect("rt");
        rt.block_on(async move {
            let tmp = TempDir::new().expect("tmpdir");
            let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 42));
            let state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);

            let mut total_listeners = 0usize;
            let mut all_ids: BTreeSet<ServiceId> = BTreeSet::new();
            for (si, listeners) in services.iter().enumerate() {
                let workload = format!("be3-{si}");
                let wid = WorkloadId::new(&workload).expect("wid");
                let vip = persist_and_allocate(&state, &workload, listeners).await;
                {
                    let mut facts = state.listener_facts.lock().await;
                    facts.upsert(wid, &vip, listeners);
                }
                for l in listeners {
                    all_ids.insert(ServiceId::derive(&vip, l.port, l.protocol, SERVICE_MAP_PURPOSE));
                    total_listeners += 1;
                }
            }

            // The allocator issues a distinct VIP per service, so every
            // (vip, port) pair is unique ⇒ no ServiceId collision.
            prop_assert_eq!(
                all_ids.len(),
                total_listeners,
                "distinct VIPs ⇒ distinct ServiceIds (no collision)"
            );

            // The keyed store holds exactly one primary entry per listener.
            let primary_count = {
                let facts = state.listener_facts.lock().await;
                let mut n = 0usize;
                for id in &all_ids {
                    if facts.fact_for(*id).is_some() {
                        n += 1;
                    }
                }
                n
            };
            prop_assert_eq!(
                primary_count,
                total_listeners,
                "primary entry count == total listener count"
            );
            Ok(())
        })?;
    }
}
