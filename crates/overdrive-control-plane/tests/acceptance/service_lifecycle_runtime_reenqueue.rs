//! GAP-9 end-to-end runtime witness — the `service-lifecycle`
//! reconciler stays alive across convergence cadences (Shape B
//! self-re-enqueue) until it observes the `ProbeRunner`'s Pass row,
//! then emits `Stable` and goes quiet.
//!
//! Pre-patch the reconciler was registered at boot but had ZERO
//! production enqueue paths. After the first broker drain it was never
//! re-ticked, so its Stable / EarlyExit / StartupProbeFailed branches
//! were structurally unreachable: a Running-but-not-yet-Pass alloc
//! emits no actions, the §18 action-emitted self-re-enqueue gate stays
//! false, and the broker drains empty.
//!
//! This AT drives a REAL `ReconcilerRuntime` convergence loop with Sim
//! adapters (NOT a hand-mirrored `hydrate_actual`, which the sibling
//! `service_lifecycle_probe_to_stable.rs` already covers). The property
//! pinned is: "the runtime keeps the reconciler alive across ticks
//! (Shape B) until it observes the Pass row, then converges."
//!
//! The convergence loop shape mirrors
//! `service_workload_convergence_no_panic.rs`: drain the broker, run
//! `run_convergence_tick` per pending eval (which self-re-enqueues via
//! `has_work` / `view_has_backoff_pending`), repeat.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, service_lifecycle, workload_lifecycle};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, ResourcesInput, ServiceV1, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::eval_broker::Evaluation;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

const SERVICE_LIFECYCLE: &str = "service-lifecycle";

fn nid(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}
fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}
fn wid(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}
fn service_target(w: &WorkloadId) -> TargetResource {
    TargetResource::new(&format!("job/{w}")).expect("valid target")
}
fn service_reconciler_name() -> ReconcilerName {
    ReconcilerName::new(SERVICE_LIFECYCLE).expect("valid reconciler name")
}

async fn build_state(
    tmp: &TempDir,
    clock: Arc<SimClock>,
    obs: Arc<dyn ObservationStore>,
) -> AppState {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(workload_lifecycle()).await.expect("register job-lifecycle");
    runtime.register(service_lifecycle()).await.expect("register service-lifecycle");
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
        clock,
        Arc::new(SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        nid("writer-1"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

fn startup_probe(port: u16) -> ProbeDescriptor {
    ProbeDescriptor {
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_string(), port },
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

async fn persist_service(state: &AppState, svc: &ServiceV1) {
    let w = svc.id.clone();
    let intent = WorkloadIntent::Service(svc.clone());
    let archived = intent.archive_for_store().expect("rkyv archive");
    state
        .store
        .put(IntentKey::for_workload(&w).as_bytes(), archived.as_ref())
        .await
        .expect("put service intent");
    state
        .store
        .put(
            IntentKey::for_workload_kind(&w).as_bytes(),
            &[WorkloadKind::Service.discriminator_byte()],
        )
        .await
        .expect("put workload kind");
}

async fn write_running_alloc(
    state: &AppState,
    w: &WorkloadId,
    a: &AllocationId,
    started_secs: u64,
) {
    let row = AllocStatusRow {
        alloc_id: a.clone(),
        workload_id: w.clone(),
        node_id: nid("local"),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: nid("writer-1") },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(started_secs))),
        // Host-netns fixture — no canonical workload address (AllocStatusRowV2 additive field, GH #241).
        workload_addr: None,
    };
    state.obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write alloc row");
}

async fn write_pass_probe(obs: &Arc<dyn ObservationStore>, a: &AllocationId, observed_at_ms: u64) {
    obs.write_probe_result(ProbeResultRow {
        alloc_id: a.clone(),
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status: ProbeStatus::Pass,
        last_observed_at_unix_ms: observed_at_ms,
        inferred: false,
    })
    .await
    .expect("write pass probe row");
}

/// Drain the broker and run one convergence tick per pending eval.
/// Returns whether a `service-lifecycle` eval was among the drained
/// set (i.e. the runtime had it pending at the start of this tick).
async fn run_one_cadence(state: &AppState, tick_n: u64) -> bool {
    let now = state.clock.now();
    let deadline = now + Duration::from_millis(100);
    let pending = {
        let mut broker = state.runtime.broker();
        broker.drain_pending()
    };
    let had_service = pending.iter().any(|e| e.reconciler.as_str() == SERVICE_LIFECYCLE);
    for eval in pending {
        run_convergence_tick(state, &eval.reconciler, &eval.target, now, tick_n, deadline)
            .await
            .expect("convergence tick must not panic");
    }
    had_service
}

/// Is a `service-lifecycle` eval currently pending in the broker
/// (without draining it)? Implemented by draining and re-submitting —
/// the broker is LWW so re-submit is idempotent at the same key.
fn service_eval_pending(state: &AppState) -> bool {
    let mut broker = state.runtime.broker();
    let drained = broker.drain_pending();
    let present = drained.iter().any(|e| e.reconciler.as_str() == SERVICE_LIFECYCLE);
    for e in drained {
        broker.submit(e);
    }
    present
}

/// GAP-9 — the runtime self-re-enqueues `service-lifecycle` across
/// cadences while the alloc is mid-startup-window (Shape B), then emits
/// `Stable` once it observes the Pass row, then goes quiet.
#[tokio::test]
async fn service_lifecycle_reenqueues_until_pass_then_emits_stable() {
    let tmp = TempDir::new().expect("tmpdir");
    let clock = Arc::new(SimClock::new());
    let obs =
        Arc::new(SimObservationStore::single_peer(nid("local"), 0)) as Arc<dyn ObservationStore>;
    let state = build_state(&tmp, Arc::clone(&clock), Arc::clone(&obs)).await;

    let workload = wid("payments");
    let alloc = aid("payments-0");
    let target = service_target(&workload);

    // Service intent with a NON-EMPTY startup probe — load-bearing: an
    // empty `startup_probes` triggers the first-Running-IS-Stable
    // opt-out (`spec_facts_for_service` → `startup_probes_empty=true`),
    // which would emit Stable on tick 1 without ever needing a Pass row
    // and defeat the Shape B re-enqueue property under test. With a
    // declared probe the reconciler waits for the Pass row.
    let svc = ServiceV1::from_submit(ServiceSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/serve".to_string(), args: vec![] }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_string() }],
        startup_probes: vec![startup_probe(8080)],
        readiness_probes: vec![],
        liveness_probes: vec![],
    })
    .expect("valid service spec");
    persist_service(&state, &svc).await;
    write_running_alloc(&state, &workload, &alloc, 1_700_000_000).await;

    // Seed the FIRST enqueue (Shape C's job is to do this in
    // production; here we submit directly to isolate Shape B — the
    // self-re-enqueue is the property under test).
    state
        .runtime
        .broker()
        .submit(Evaluation { reconciler: service_reconciler_name(), target: target.clone() });

    // -----------------------------------------------------------------
    // Cadence 1 — Running, no Pass row → reconciler observes the alloc
    // mid-startup-window, emits NO action, and the runtime MUST
    // self-re-enqueue via view_has_backoff_pending (Shape B).
    // -----------------------------------------------------------------
    let ran_1 = run_one_cadence(&state, 0).await;
    assert!(ran_1, "cadence 1 must have run the seeded service-lifecycle eval");
    assert!(
        service_eval_pending(&state),
        "Shape B: after a mid-startup-window tick the runtime MUST re-enqueue \
         service-lifecycle (pre-patch the broker drained empty here)"
    );

    // A few more cadences with still no Pass — the reconciler must
    // stay alive every cadence, never draining to empty.
    for tick_n in 1..4 {
        let ran = run_one_cadence(&state, tick_n).await;
        assert!(ran, "cadence {tick_n}: service-lifecycle must still be pending (Shape B)");
        assert!(
            service_eval_pending(&state),
            "cadence {tick_n}: runtime must keep re-enqueueing while mid-startup-window"
        );
    }

    // -----------------------------------------------------------------
    // The ProbeRunner writes a Pass row. The next cadence's reconcile
    // observes it → emits Stable → records stable_announced → the
    // mid-startup-window predicate flips false → the runtime stops
    // re-enqueueing.
    // -----------------------------------------------------------------
    write_pass_probe(&obs, &alloc, 5_000).await;

    let ran_pass = run_one_cadence(&state, 4).await;
    assert!(ran_pass, "the Pass-observing cadence must run the pending service-lifecycle eval");

    // The persisted view must now record Stable for the alloc.
    let views = state
        .runtime
        .loaded_service_lifecycle_views_for_test(&service_reconciler_name())
        .expect("service-lifecycle view map present");
    let view = views.get(&target).expect("view for target present");
    assert!(
        view.stable_announced.contains(&alloc),
        "after observing the Pass row the reconciler must announce Stable for the alloc; \
         view = {view:?}"
    );

    // The action-emitted Stable re-enqueues once more; that final
    // cadence sees the alloc already in stable_announced (dedup →
    // 0 actions) AND the predicate is false (alloc is terminal), so the
    // runtime drains to empty — the reconciler goes quiet.
    let _ = run_one_cadence(&state, 5).await;
    assert!(
        !service_eval_pending(&state),
        "after Stable the reconciler must go quiet — no busy-loop on a converged alloc"
    );
}
