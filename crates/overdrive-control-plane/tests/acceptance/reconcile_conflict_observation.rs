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
    DriverInput, ExecInput, IntentKey, ResourcesInput, ServiceV2, WorkloadIntent, WorkloadKind,
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
    ConflictRoute, LogicalTimestamp, ObservationRow, ObservationStore, ReconcileConflictRow,
    ServiceBackendRow,
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
        .register(AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical(
            HOST_IPV4,
            overdrive_control_plane::veth_provisioner::WORKLOAD_SUBNET_BASE,
        )))
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
    let svc = ServiceV2::from_submit(ServiceSpecInput {
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
            "spiffe://overdrive.local/workload/payments/alloc/{alloc_suffix}"
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

/// Seeded LWW counter on the prior row at the CONFLICTING slot — the
/// only one the production lookup may consult.
const PRIOR_AT_SLOT: u64 = 100;
/// Seeded counters on the three decoy slots, each one key field away
/// from the conflicting slot. Distinct so the row a corrupted lookup
/// predicate selects is identifiable from the resulting counter alone.
const PRIOR_DECOY_VIP: u64 = 200;
const PRIOR_DECOY_PORT: u64 = 300;
const PRIOR_DECOY_PROTO: u64 = 400;

/// Prior `reconcile_conflict` row at an arbitrary slot, used to seed the
/// LWW floor. Routes are fixed (cgroup-vs-cgroup, the only surviving
/// conflict class); only the slot and the counter vary per seed.
fn seeded_conflict_row(
    service_id: ServiceId,
    vip: Ipv4Addr,
    port: u16,
    proto: Proto,
    counter: u64,
) -> ReconcileConflictRow {
    ReconcileConflictRow {
        service_id,
        vip,
        port,
        proto,
        first_route: ConflictRoute::Cgroup,
        second_route: ConflictRoute::Cgroup,
        updated_at: LogicalTimestamp { counter, writer: node_id("seed-writer") },
    }
}

/// ADR-0077 § D2 site 8: the conflict row's LWW counter derives from the
/// prior row at **this exact `(vip, port, proto)` slot** — never from a
/// neighbouring slot, and never from the tick alone when a prior exists.
///
/// GIVEN four prior `reconcile_conflict` rows for the SAME service — one
/// at the conflicting slot and three decoys differing in exactly one key
/// field each (vip, port, proto), every decoy carrying a distinct counter
/// — WHEN a genuine same-slot conflict is surfaced, THEN the new row's
/// counter is derived from the row at the conflicting slot alone
/// (`max(tick+1, prior.counter+1)`), and every decoy row is left
/// untouched.
///
/// This is the assertion that exercises the prior-row lookup predicate
/// `r.vip == vip && r.port == port && r.proto == proto`. The sibling test
/// above seeds NO prior rows, so `find` runs over an empty iterator and
/// the predicate is never evaluated — every comparison in it could be
/// inverted with no observable effect. Here each decoy is reachable by
/// exactly one corruption of that predicate, and each yields a different
/// counter:
///
/// | predicate corruption      | row selected | resulting counter |
/// |---------------------------|--------------|-------------------|
/// | (none — correct)          | true slot    | 101               |
/// | `vip ==` → `vip !=`       | decoy vip    | 201               |
/// | `port ==` → `port !=`     | decoy port   | 301               |
/// | `proto ==` → `proto !=`   | decoy proto  | 401               |
/// | either `&&` → `\|\|`      | decoy vip    | 201               |
///
/// The two `&&` → `||` cases land on the decoy-vip row because it sorts
/// FIRST in the store's `(service_id, vip, port, proto)` `BTreeMap` order
/// (its vip is one below the real one) and `Iterator::find` short-circuits
/// on the first match — under either disjunction every seeded row matches,
/// so the first one wins.
#[tokio::test]
async fn conflict_lww_counter_derives_from_the_prior_row_at_the_exact_slot() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 7));
    let state = build_state(&tmp, obs.clone() as Arc<dyn ObservationStore>).await;

    // Same conflict shape as the sibling test: a UDP:53 listener whose
    // backend set carries two LOCAL backends → two cgroup writes to one
    // `(vip, 53, udp)` slot.
    let (vip, port, sid) = persist_service_and_allocate_vip(&state, 53, "udp").await;
    let vip_v4 = vip.try_as_ipv4().expect("allocator issues IPv4");

    // Decoy slots, each one key field away from the conflicting slot.
    // `lower_vip` is strictly BELOW the real vip so the decoy-vip row
    // sorts first in the store's key order (see the table above).
    let lower_vip = Ipv4Addr::from(u32::from(vip_v4).saturating_sub(1));
    let other_port = port.saturating_add(1);

    for row in [
        seeded_conflict_row(sid, lower_vip, port, Proto::Udp, PRIOR_DECOY_VIP),
        seeded_conflict_row(sid, vip_v4, port, Proto::Udp, PRIOR_AT_SLOT),
        seeded_conflict_row(sid, vip_v4, other_port, Proto::Udp, PRIOR_DECOY_PORT),
        seeded_conflict_row(sid, vip_v4, port, Proto::Tcp, PRIOR_DECOY_PROTO),
    ] {
        obs.write(ObservationRow::ReconcileConflict(row)).await.expect("seed prior conflict row");
    }

    let backends = vec![
        local_backend(&format!("{HOST_IPV4}:9090"), "a1"),
        local_backend(&format!("{HOST_IPV4}:9091"), "a2"),
    ];
    obs.write(ObservationRow::ServiceBackend(ServiceBackendRow {
        service_id: sid,
        vip: vip_v4,
        backends,
        updated_at: LogicalTimestamp { counter: 1, writer: node_id("writer-1") },
    }))
    .await
    .expect("write service_backends");

    let target = TargetResource::new(&format!("service/{sid}")).expect("target");
    let reconciler_name = ReconcilerName::new("service-map-hydrator").expect("name");
    let now = std::time::Instant::now();
    // Deliberately LOW so `tick + 1` (= 4) never masks a prior-derived
    // counter: every expected outcome below is driven by the prior row,
    // not by the tick floor.
    let tick_n = 3_u64;
    let deadline = now + Duration::from_millis(100);

    run_convergence_tick(&state, &reconciler_name, &target, now, tick_n, deadline)
        .await
        .expect("convergence tick must NOT error/stop on a genuine conflict");

    let conflicts = obs.reconcile_conflict_rows(&sid).await.expect("read conflict rows");
    let slot_of = |r: &ReconcileConflictRow| (r.vip, r.port, r.proto);
    let counter_at = |v: Ipv4Addr, p: u16, pr: Proto| -> u64 {
        conflicts
            .iter()
            .find(|r| slot_of(r) == (v, p, pr))
            .unwrap_or_else(|| panic!("no reconcile_conflict row at slot ({v}, {p}, {pr:?})"))
            .updated_at
            .counter
    };

    // The conflicting slot: counter derived from ITS prior row
    // (`max(tick+1, 100+1)` = 101). Any corruption of the prior-row
    // lookup predicate selects a decoy instead and lands 201 / 301 / 401.
    assert_eq!(
        counter_at(vip_v4, port, Proto::Udp),
        PRIOR_AT_SLOT + 1,
        "the conflict row's LWW counter MUST derive from the prior row at the exact \
         conflicting (vip, port, proto) slot — got a counter that can only come from a \
         neighbouring slot's row, so the prior-row lookup matched the wrong slot"
    );

    // Every decoy is untouched — the tick writes exactly one slot.
    assert_eq!(
        counter_at(lower_vip, port, Proto::Udp),
        PRIOR_DECOY_VIP,
        "the decoy-vip row must be left untouched by the conflict write"
    );
    assert_eq!(
        counter_at(vip_v4, other_port, Proto::Udp),
        PRIOR_DECOY_PORT,
        "the decoy-port row must be left untouched by the conflict write"
    );
    assert_eq!(
        counter_at(vip_v4, port, Proto::Tcp),
        PRIOR_DECOY_PROTO,
        "the decoy-proto row must be left untouched by the conflict write"
    );
}
