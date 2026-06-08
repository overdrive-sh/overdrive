//! Invariant A (ADR-0062 § Decision (3); feature-delta sub-decision 4)
//! — steady-state `ServiceMapHydrator` hydration pays ZERO intent-store
//! scans: the per-listener `(port, protocol)` fact is sourced from the
//! in-memory keyed [`ListenerFactStore`], NOT from a cluster-wide
//! `scan_prefix(b"workloads/")` over the `IntentStore` (the deleted
//! `gather_service_listener_facts` per-tick path).
//!
//! # Why "source moved" rather than a `scan_prefix` counter
//!
//! `AppState.store` is a concrete `Arc<LocalIntentStore>`, not an
//! `Arc<dyn IntentStore>`, so a counting `IntentStore` decorator cannot
//! be injected at the seam the hydrate path actually reads (the
//! decorator approach would require widening `AppState.store` to a trait
//! object — a production change outside this step's boundary). Instead
//! this gate proves the property *behaviorally* and more strongly: it
//! removes the intent record from the store entirely after the keyed
//! fact + the `service_backends` row are in place, then runs N hydrate
//! ticks. The OLD per-tick-scan path (`gather_service_listener_facts`)
//! would scan the now-empty `workloads/` prefix, resolve no fact, and
//! skip the service (empty desired). The NEW keyed-read path resolves
//! the fact from the in-memory store and projects a correct desired on
//! every tick. A non-empty, correct desired across N ticks is only
//! reachable if hydrate reads the in-memory fact, not the store scan.
//!
//! Tier 1, default lane — pure in-process `SimObservationStore` +
//! `LocalIntentStore` over a `TempDir`. Property-based over S services ×
//! L listeners × N ticks; seed printed on failure (proptest).

#![allow(clippy::expect_used, clippy::unwrap_used)]

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

const fn proto_str(p: Proto) -> &'static str {
    match p {
        Proto::Tcp => "tcp",
        Proto::Udp => "udp",
    }
}

/// Persist a Service intent with the given listeners, allocate its VIP
/// via the production allocator path, and return the issued VIP.
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

fn backend_for(workload: &str, idx: usize) -> Backend {
    Backend {
        alloc: SpiffeId::from_str(&format!("spiffe://overdrive.local/job/{workload}/alloc/a{idx}"))
            .expect("spiffe"),
        addr: SocketAddr::from_str(&format!("10.{}.{}.{}:8080", idx + 1, idx + 1, idx + 1))
            .expect("addr"),
        weight: 100,
        healthy: true,
    }
}

// Strategy: 1..=8 services, each with 1..=3 listeners, N in 1..=10 ticks.
fn proto_strategy() -> impl Strategy<Value = Proto> {
    prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
}

prop_compose! {
    fn listener_strategy()(port in 1u16..=65535, protocol in proto_strategy()) -> Listener {
        Listener { port: NonZeroU16::new(port).expect("port 1..=65535 is non-zero"), protocol }
    }
}

prop_compose! {
    fn service_listeners_strategy()(
        listeners in prop::collection::vec(listener_strategy(), 1..=3),
    ) -> Vec<Listener> {
        // Dedup by port so multiple listeners derive distinct ServiceIds
        // (a single service cannot declare two listeners on the same
        // port — the derived ServiceId would collide).
        let mut seen = std::collections::BTreeSet::new();
        listeners.into_iter().filter(|l| seen.insert(l.port)).collect()
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    /// Invariant A — proven behaviorally: after the keyed fact + the
    /// `service_backends` row are in place, the intent record is removed
    /// from the store. N subsequent hydrate ticks STILL project a
    /// non-empty, correct desired for every (service, listener) — which
    /// is only reachable if hydrate sources the fact from the in-memory
    /// `ListenerFactStore`, never from an intent-store scan.
    #[test]
    fn hydrator_resolves_proto_from_in_memory_facts_not_store_scan(
        service_count in 1usize..=8,
        services in prop::collection::vec(service_listeners_strategy(), 1..=8),
        ticks in 1usize..=10,
    ) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio rt");
        rt.block_on(async move {
            let services: Vec<Vec<Listener>> =
                services.into_iter().take(service_count).collect();
            prop_assume!(!services.is_empty());

            let tmp = TempDir::new().expect("tmpdir");
            let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 42));
            let state = build_app_state(&tmp, obs.clone() as Arc<dyn ObservationStore>);

            // (target ServiceId, expected port, expected proto, vip)
            let mut expectations: Vec<(ServiceId, NonZeroU16, Proto, ServiceVip)> = Vec::new();

            for (si, listeners) in services.iter().enumerate() {
                let workload = format!("svc-{si}");
                let wid = WorkloadId::new(&workload).expect("workload id");
                let vip = persist_and_allocate(&state, &workload, listeners).await;
                let vip_addr = vip.try_as_ipv4().expect("allocator issues IPv4");

                // Populate the in-memory keyed fact store (the new source).
                {
                    let mut facts = state.listener_facts.lock().await;
                    facts.upsert(wid.clone(), &vip, listeners);
                }

                // Write one service_backends row per listener (keyed on the
                // derived ServiceId, carrying the allocator-issued VIP).
                for (li, listener) in listeners.iter().enumerate() {
                    let sid =
                        ServiceId::derive(&vip, listener.port, listener.protocol, SERVICE_MAP_PURPOSE);
                    let row = ServiceBackendRow {
                        service_id: sid,
                        vip: vip_addr,
                        backends: vec![backend_for(&workload, li)],
                        updated_at: LogicalTimestamp {
                            counter: 1,
                            writer: node_id("writer-1"),
                        },
                    };
                    obs.write(ObservationRow::ServiceBackend(row))
                        .await
                        .expect("write service_backends");
                    expectations.push((sid, listener.port, listener.protocol, vip));
                }

                // Remove the intent record entirely — the OLD per-tick
                // scan path would now resolve NO fact for this service.
                let key = IntentKey::for_workload(&wid);
                state.store.delete(key.as_bytes()).await.expect("delete intent");
            }

            // Run N hydrate ticks; every tick must yield the correct
            // desired from the in-memory facts (intent is gone).
            for _ in 0..ticks {
                for (sid, port, proto, vip) in &expectations {
                    let target =
                        TargetResource::new(&format!("service/{sid}")).expect("target");
                    let hydrated =
                        overdrive_control_plane::reconciler_runtime::hydrate_desired_for_test(
                            &hydrator_reconciler(),
                            &target,
                            &state,
                        )
                        .await
                        .expect("hydrate_desired must succeed");
                    let smh = match hydrated {
                        AnyState::ServiceMapHydrator(s) => s,
                        other => panic!("expected ServiceMapHydrator state, got {other:?}"),
                    };
                    let desired = smh.desired.get(sid).unwrap_or_else(|| {
                        panic!(
                            "service {sid} must project a desired entry from the in-memory \
                             fact even with the intent record deleted (Invariant A)"
                        )
                    });
                    prop_assert_eq!(desired.port, *port, "port from in-memory fact");
                    prop_assert_eq!(desired.proto, *proto, "proto from in-memory fact (C3)");
                    prop_assert_eq!(desired.vip, *vip, "vip matches allocator-issued");
                }
            }
            Ok(())
        })?;
    }
}
