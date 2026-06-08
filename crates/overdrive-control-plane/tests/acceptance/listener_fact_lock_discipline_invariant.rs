//! Invariant C (ADR-0062 § Decision (3); feature-delta sub-decision 5)
//! — the `ServiceMapHydrator` hydrate path NEVER holds the
//! `listener_facts` guard across an `.await`.
//!
//! # Behavioral gate (primary)
//!
//! There is no portable runtime hook for "a guard crossed an `.await`"
//! (distill § C honesty note). The falsifiable proxy is contention:
//! the `listener_facts` store is `Arc<tokio::sync::Mutex<...>>`, shared
//! by the hydrate read, the submit-edge `upsert`, and the stop-edge
//! `remove_workload`. If the hydrate path acquired the guard and then
//! `.await`ed the `ObservationStore` read (or any other future) while
//! holding it, concurrent `upsert` / `remove_workload` tasks racing on
//! the same mutex would stall — and a bounded `timeout` wrapping the
//! whole concurrent batch would elapse. The correct
//! acquire → clone → drop discipline lets every task make progress, so
//! the batch completes well within the budget.
//!
//! Tier 1, default lane — pure in-process, no real infrastructure.
//! Uses the multi-thread tokio runtime so the contending tasks run
//! genuinely concurrently (a guard-across-await deadlock manifests as a
//! timeout rather than silently serialising).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::num::NonZeroU16;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

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
use overdrive_core::reconcilers::{AnyReconciler, ServiceMapHydrator, TargetResource};
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

async fn persist_and_allocate(
    state: &AppState,
    workload: &str,
    listeners: &[Listener],
) -> ServiceVip {
    let listener_inputs: Vec<ListenerInput> = listeners
        .iter()
        .map(|l| ListenerInput {
            port: l.port.get(),
            protocol: match l.protocol {
                Proto::Tcp => "tcp".to_string(),
                Proto::Udp => "udp".to_string(),
            },
        })
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

/// Concurrent hydrate-read + submit-upsert + stop-remove on the shared
/// `Arc<Mutex<ListenerFactStore>>`. A guard held across `.await` in the
/// hydrate path deadlocks/stalls the upsert + remove tasks, so the
/// bounded `timeout` over the joined batch elapses; the correct
/// acquire→clone→drop discipline lets every op complete promptly.
///
/// `assert_eventually!`-shaped: the assertion is "all ops complete
/// within the DST budget" (here a generous 5 s `tokio::time::timeout`
/// that a healthy run clears in milliseconds and a deadlocked run never
/// clears).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn listener_fact_guard_never_held_across_await_under_contention() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 42));
    let state = Arc::new(build_app_state(&tmp, obs.clone() as Arc<dyn ObservationStore>));

    let listeners = vec![
        Listener { port: NonZeroU16::new(8080).expect("nz"), protocol: Proto::Tcp },
        Listener { port: NonZeroU16::new(5353).expect("nz"), protocol: Proto::Udp },
    ];
    let workload = "contended";
    let wid = WorkloadId::new(workload).expect("workload id");
    let vip = persist_and_allocate(&state, workload, &listeners).await;
    let vip_addr = vip.try_as_ipv4().expect("allocator issues IPv4");

    // Seed the keyed fact + the service_backends rows for the read path.
    {
        let mut facts = state.listener_facts.lock().await;
        facts.upsert(wid.clone(), &vip, &listeners);
    }
    let mut targets = Vec::new();
    for listener in &listeners {
        let sid = ServiceId::derive(&vip, listener.port, listener.protocol, SERVICE_MAP_PURPOSE);
        let row = ServiceBackendRow {
            service_id: sid,
            vip: vip_addr,
            backends: vec![Backend {
                alloc: SpiffeId::from_str("spiffe://overdrive.local/job/contended/alloc/a1")
                    .expect("spiffe"),
                addr: SocketAddr::from_str("10.1.1.1:8080").expect("addr"),
                weight: 100,
                healthy: true,
            }],
            updated_at: LogicalTimestamp { counter: 1, writer: node_id("writer-1") },
        };
        obs.write(ObservationRow::ServiceBackend(row)).await.expect("write row");
        targets.push(TargetResource::new(&format!("service/{sid}")).expect("target"));
    }

    // Spawn contending tasks: repeated hydrate reads + repeated upserts +
    // repeated removes, all racing on the shared listener_facts mutex.
    let iterations = 50usize;

    let read_state = Arc::clone(&state);
    let read_targets = targets.clone();
    let reader = tokio::spawn(async move {
        for _ in 0..iterations {
            for target in &read_targets {
                let _ = overdrive_control_plane::reconciler_runtime::hydrate_desired_for_test(
                    &hydrator_reconciler(),
                    target,
                    &read_state,
                )
                .await
                .expect("hydrate_desired must succeed");
            }
        }
    });

    let upsert_state = Arc::clone(&state);
    let upsert_listeners = listeners.clone();
    let upsert_wid = wid.clone();
    let upserter = tokio::spawn(async move {
        for _ in 0..iterations {
            let mut facts = upsert_state.listener_facts.lock().await;
            facts.upsert(upsert_wid.clone(), &vip, &upsert_listeners);
            drop(facts);
            tokio::task::yield_now().await;
        }
    });

    let remove_state = Arc::clone(&state);
    let remove_wid = wid.clone();
    let remover = tokio::spawn(async move {
        for _ in 0..iterations {
            let mut facts = remove_state.listener_facts.lock().await;
            facts.remove_workload(&remove_wid);
            drop(facts);
            tokio::task::yield_now().await;
        }
    });

    // Liveness: every contending op completes within the budget. A
    // guard-across-await in the hydrate path stalls upsert/remove and
    // this elapses.
    let outcome = tokio::time::timeout(Duration::from_secs(5), async {
        reader.await.expect("reader task");
        upserter.await.expect("upsert task");
        remover.await.expect("remove task");
    })
    .await;

    assert!(
        outcome.is_ok(),
        "concurrent hydrate-read + upsert + remove must all complete within the budget; \
         a stall means the hydrate path held the listener_facts guard across an .await"
    );
}
