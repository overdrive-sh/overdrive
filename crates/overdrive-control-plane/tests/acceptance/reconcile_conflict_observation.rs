//! Fix C — `run_convergence_tick` writes a queryable `reconcile_conflict`
//! observation row on a genuine same-slot reconcile-output violation,
//! alongside (not replacing) the `reconciler.output.invariant_violation`
//! tracing event. Surface-then-continue: dispatch is skipped, the View
//! still persists, the tick does NOT error/stop. See
//! `docs/feature/fix-mixed-backend-dispatch-spin/deliver/bugfix-rca.md`
//! § Fix C + § Posture, and `.claude/rules/reconcilers.md` self-heal
//! posture (no `TerminalError`; the appliance OS self-heals).
//!
//! Tier 1 DST: drives the production `run_convergence_tick` against the
//! sim adapters (`SimObservationStore`, `SimClock`, `SimDriver`,
//! `SimDataplane`, `LocalIntentStore` on a `TempDir`). Default unit lane —
//! no real infra, no `integration-tests` gate.
//!
//! Conflict shape: after Fix A1 (step 01-01) the cross-route dual-path
//! is NOT a conflict; the surviving violation class is same-route
//! same-slot. We drive the real `ServiceMapHydrator` with a service
//! whose backend set carries TWO distinct *local* backends (both
//! matching `host_ipv4`). `push_register_local_backend_actions` emits
//! one `Action::RegisterLocalBackend` per local backend, all carrying
//! the SAME service-level `(vip, vip_port, proto)` slot — so two local
//! backends produce two cgroup writes to one slot → a genuine
//! cgroup-vs-cgroup same-slot conflict reaching the validator `Err`
//! arm.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::AppState;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_core::SpiffeId;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, ResourcesInput, ServiceV1, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{NodeId, ServiceId, ServiceVip};
use overdrive_core::reconcilers::{
    AnyReconciler, ReconcilerName, ServiceMapHydrator, TargetResource,
};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    ConflictRoute, LogicalTimestamp, ObservationRow, ObservationStore, ServiceBackendRow,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

/// The host IPv4 the hydrator classifier compares backends against. Two
/// backends with THIS ip are both classified Local → both emit a cgroup
/// `RegisterLocalBackend` under the same service slot.
const HOST_IPV4: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 5);

fn node_id(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid NodeId")
}

/// Build an `AppState` whose runtime carries the `service-map-hydrator`
/// reconciler classifying against `HOST_IPV4`.
async fn build_state(tmp: &TempDir, obs: Arc<dyn ObservationStore>) -> AppState {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime
        .register(AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical(HOST_IPV4)))
        .await
        .expect("register service-map-hydrator");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(SimDataplane::new()),
        node_id("writer-1"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

/// Persist a single-listener Service intent + allocate its VIP, then
/// populate the in-memory `ListenerFactStore` (the read-path source for
/// the hydrator's `(port, proto)` fact). Mirrors the submit-edge upsert
/// the production handler performs. Returns the allocator VIP + the
/// listener port + derived `ServiceId`.
async fn persist_service_and_allocate_vip(
    state: &AppState,
    listener_port: u16,
    protocol: &str,
) -> (ServiceVip, u16, ServiceId) {
    let svc = ServiceV1::from_submit(ServiceSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/serve".to_string(), args: vec![] }),
        listeners: vec![ListenerInput { port: listener_port, protocol: protocol.to_string() }],
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
    let vip = {
        let mut guard = state.allocator.lock().await;
        guard.allocate(bytes).await.expect("allocate vip")
    };

    {
        let mut facts = state.listener_facts.lock().await;
        facts.upsert(svc.id.clone(), &vip, &svc.listeners);
    }

    let sid = ServiceId::derive(
        &vip,
        std::num::NonZeroU16::new(listener_port).expect("non-zero"),
        svc.listeners[0].protocol,
        "service-map",
    );
    (vip, listener_port, sid)
}

fn local_backend(addr: &str, alloc_suffix: &str) -> Backend {
    Backend {
        alloc: SpiffeId::from_str(&format!(
            "spiffe://overdrive.local/job/payments/alloc/{alloc_suffix}"
        ))
        .expect("spiffe"),
        addr: SocketAddr::from_str(addr).expect("addr"),
        weight: 100,
        healthy: true,
    }
}

/// GIVEN a service whose backend set has two distinct LOCAL backends
/// (both at `HOST_IPV4`), the hydrator emits two `RegisterLocalBackend`
/// actions to the SAME `(vip, vip_port, proto)` cgroup slot — a genuine
/// same-slot conflict. WHEN `run_convergence_tick` runs, THEN:
///   (a) a queryable `reconcile_conflict` observation row is written and
///       readable via `state.obs.reconcile_conflict_rows(&sid)` with the
///       conflicting slot + cgroup routes;
///   (b) dispatch is skipped — no service-hydration row landed for the
///       VIP this tick;
///   (c) the tick returns `Ok(())` (no error/stop; surface-then-continue).
#[tokio::test]
async fn genuine_same_slot_conflict_produces_queryable_observation_row() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 7));
    let state = build_state(&tmp, obs.clone() as Arc<dyn ObservationStore>).await;

    // Persist the Service (UDP:53) + allocate its VIP — the
    // listener-bearing protocol fact source. UDP (NOT the `Tcp`
    // fallback) is deliberate: the row's `proto` field MUST be
    // recovered from the correctly-matched conflicting action via
    // `conflicting_slot_proto`. Asserting `proto == Udp` below (rather
    // than the default `Tcp`) makes any mutation of that recovery
    // helper — return `None`, delete a match arm, flip a `==` / `&&` —
    // surface as `Tcp` and FAIL the assertion, killing the mutant.
    let (vip, port, sid) = persist_service_and_allocate_vip(&state, 53, "udp").await;
    let vip_v4 = vip.try_as_ipv4().expect("allocator issues IPv4");

    // Two distinct LOCAL backends — both at HOST_IPV4, different ports.
    // The hydrator classifies both as Local and emits two
    // RegisterLocalBackend to the SAME (vip, vip_port=port, proto=udp)
    // slot → genuine cgroup-vs-cgroup same-slot conflict.
    let backends = vec![
        local_backend(&format!("{HOST_IPV4}:9090"), "a1"),
        local_backend(&format!("{HOST_IPV4}:9091"), "a2"),
    ];
    let row = ServiceBackendRow {
        service_id: sid,
        vip: vip_v4,
        backends: backends.clone(),
        updated_at: LogicalTimestamp { counter: 1, writer: node_id("writer-1") },
    };
    obs.write(ObservationRow::ServiceBackend(row)).await.expect("write service_backends");

    // Drive one convergence tick against the hydrator for this service.
    let target = TargetResource::new(&format!("service/{sid}")).expect("target");
    let reconciler_name = ReconcilerName::new("service-map-hydrator").expect("name");
    let now = std::time::Instant::now();
    let tick_n = 3_u64;
    let deadline = now + Duration::from_millis(100);

    // --- Assertion (c): the tick returns Ok — no stop/error on the
    //     genuine conflict (surface-then-continue posture).
    run_convergence_tick(&state, &reconciler_name, &target, now, tick_n, deadline)
        .await
        .expect("convergence tick must NOT error/stop on a genuine conflict");

    // --- Assertion (a): the conflict row is queryable, with the
    //     conflicting slot + cgroup-vs-cgroup routes.
    let conflicts = obs.reconcile_conflict_rows(&sid).await.expect("read conflict rows");
    assert_eq!(conflicts.len(), 1, "exactly one reconcile_conflict row for the service");
    let conflict = &conflicts[0];
    assert_eq!(conflict.service_id, sid, "row carries the conflicting service identity");
    assert_eq!(conflict.vip, vip_v4, "row carries the conflicting slot VIP");
    assert_eq!(conflict.port, port, "row carries the conflicting slot port (the service VIP port)");
    assert_eq!(
        conflict.proto,
        Proto::Udp,
        "row's proto MUST be recovered from the matching conflicting action \
         (the listener declared udp:53), never the Tcp fallback — this is the \
         assertion that kills every `conflicting_slot_proto` mutation"
    );
    assert_eq!(
        conflict.first_route,
        ConflictRoute::Cgroup,
        "the surviving conflict class is cgroup-vs-cgroup"
    );
    assert_eq!(conflict.second_route, ConflictRoute::Cgroup, "both routes are cgroup");
    // LWW timestamp follows the action-shim convention: counter = tick+1.
    assert_eq!(
        conflict.updated_at.counter,
        tick_n + 1,
        "row's LWW counter follows the action-shim convention (tick.tick + 1)"
    );
    assert_eq!(conflict.updated_at.writer, node_id("writer-1"), "row's LWW writer is the node id");

    // --- Assertion (b): dispatch was skipped — NO service-hydration row
    //     was written for the service this tick (the action shim never
    //     ran). The hydration row is the observable side effect of a
    //     successful DataplaneUpdateService dispatch.
    let hydration = obs.service_hydration_results_rows(&sid).await.expect("read hydration rows");
    assert!(
        hydration.is_empty(),
        "dispatch must be skipped on a genuine conflict; no service-hydration row should land, \
         got {hydration:?}"
    );
}
