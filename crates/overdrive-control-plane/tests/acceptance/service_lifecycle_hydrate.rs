//! GAP-1 acceptance — `hydrate_desired` / `hydrate_actual` for the
//! `ServiceLifecycle` arm of `AnyReconciler::reconcile` per the Phase
//! 01 structural gap audit (`.context/01-03-structural-gap-audit.md`).
//!
//! Closes the `default()` placeholder at
//! `reconciler_runtime.rs:1360-1362` (`hydrate_desired`) and `:1733-1735`
//! (`hydrate_actual`) per Action A of the audit's recommended
//! corrective scope.
//!
//! Three-source join per the audit's recommended fix:
//!
//!   1. `obs.alloc_status_rows()` filtered to the target workload —
//!      sources `alloc_id`, `state`, `started_at`. The crash's typed
//!      `reason` stays on the observation row for `WorkloadLifecycle`.
//!   2. `obs.list_probe_results_for_alloc(...)` LWW projection —
//!      sources `latest_startup_probe`.
//!   3. `IntentStore::get(IntentKey::for_workload(workload_id))` →
//!      `WorkloadIntent::Service(Service)` (with probe vecs
//!      persisted post-GAP-6) — sources `max_attempts`,
//!      `startup_deadline`, `mechanic_summary`, `inferred`,
//!      `startup_probes_empty`.
//!
//! Per the audit's GAP-1-AT-08, the reconciler `unreachable!()` on the
//! Running + None(started_at) invalid combination is asserted via
//! `#[should_panic(expected = "hydrate invariant")]`.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::unnecessary_trailing_comma,
    clippy::unused_async
)]

use overdrive_core::observation::{ProbeRunSeq, SameStatusRuns};
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::AppState;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, hydrate_actual_for_test, hydrate_desired_for_test,
};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, Service, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::TargetResource;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::service_lifecycle::{ServiceLifecycleReconciler, ServiceLifecycleState};
use overdrive_reconcilers::{AnyReconciler, AnyState};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_id(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid NodeId")
}

fn alloc_id(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn workload_id(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}

fn service_lifecycle_reconciler() -> AnyReconciler {
    AnyReconciler::ServiceLifecycle(ServiceLifecycleReconciler::new())
}

fn target_for(wid: &WorkloadId) -> TargetResource {
    TargetResource::new(&format!("workload/{wid}")).expect("valid target")
}

async fn build_app_state(tmp: &TempDir, obs: Arc<dyn ObservationStore>) -> AppState {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(service_lifecycle_reconciler()).await.expect("register service-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
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
        crate::acceptance::serve_ports::worker(),
        crate::acceptance::serve_ports::owner(),
        crate::acceptance::serve_ports::exec_gate(),
        crate::acceptance::serve_ports::pool(),
    )
}

fn declared_tcp_probe(port: u16, max_attempts: u32, interval_seconds: u32) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_string(), port },
        timeout_seconds: 5,
        interval_seconds,
        max_attempts,
        failure_threshold: None,
        success_threshold: None,
    }
}

fn inferred_tcp_probe(port: u16) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_string(), port },
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
    }
}

fn build_service_spec(wid: &WorkloadId, startup_probes: Vec<ProbeDescriptor>) -> Service {
    let input = ServiceSpecInput {
        id: wid.as_str().to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
            command: "/bin/serve".to_string(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_string() }],
        startup_probes,
        readiness_probes: vec![],
        liveness_probes: vec![],
    };
    Service::from_submit(input).expect("valid service spec")
}

async fn persist_service_intent(state: &AppState, svc: &Service) {
    let wid = svc.id.clone();
    let intent = WorkloadIntent::Service(svc.clone());
    let archived = intent.archive_for_store().expect("rkyv archive");
    let key = IntentKey::for_workload(&wid);
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put service intent");

    let kind_key = IntentKey::for_workload_kind(&wid);
    state
        .store
        .put(kind_key.as_bytes(), &[WorkloadKind::Service.discriminator_byte()])
        .await
        .expect("put workload kind");
}

/// The fixture workload address of a Running row, inside the guest prefix the
/// guest-network pool leases from (never the node address).
const WORKLOAD_ADDR_OFFSET: u32 = 2;

/// The `offset`-th address of the guest node prefix.
fn guest_addr(offset: u32) -> std::net::Ipv4Addr {
    std::net::Ipv4Addr::from(
        u32::from(overdrive_core::guest_transport::GUEST_NODE_PREFIX.network()) + offset,
    )
}

fn make_alloc_status_row(
    wid: &WorkloadId,
    aid: &AllocationId,
    state: AllocState,
    started_at: Option<UnixInstant>,
    counter: u64,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid.clone(),
        workload_id: wid.clone(),
        node_id: node_id("local"),
        state,
        updated_at: LogicalTimestamp { counter, writer: node_id("writer-1") },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at,
        // A Running row carries its workload address, as the production Running
        // write does (ADR-0276); a non-Running row legitimately carries none.
        workload_addr: (state == AllocState::Running).then(|| guest_addr(WORKLOAD_ADDR_OFFSET)),
        last_terminated: None,
        restart_count: 0,
    }
}

/// Like [`make_alloc_status_row`] but carries an explicit typed reason so
/// the durable crash cause remains available to `WorkloadLifecycle` after
/// Service-kind hydration (D6; no `ServiceAllocFact.exit_code` projection).
fn make_alloc_status_row_with_reason(
    wid: &WorkloadId,
    aid: &AllocationId,
    state: AllocState,
    started_at: Option<UnixInstant>,
    counter: u64,
    reason: Option<overdrive_core::transition_reason::TransitionReason>,
) -> AllocStatusRow {
    let mut row = make_alloc_status_row(wid, aid, state, started_at, counter);
    row.reason = reason;
    row
}

async fn write_alloc_status(state: &AppState, row: AllocStatusRow) {
    state
        .obs
        .write_alloc_lifecycle(
            row,
            overdrive_core::traits::observation_store::TransitionSource::Reconciler,
        )
        .await
        .expect("write alloc row");
}

async fn write_probe_result(
    obs: &Arc<dyn ObservationStore>,
    aid: &AllocationId,
    probe_idx: u32,
    role: ProbeRole,
    status: ProbeStatus,
    last_observed_at_unix_ms: u64,
) {
    let row = ProbeResultRow {
        alloc_id: aid.clone(),
        probe_idx: ProbeIdx::new(probe_idx),
        role,
        status,
        last_observed_at_unix_ms,
        inferred: false,
        run_seq: ProbeRunSeq::new(u64::max(last_observed_at_unix_ms, 1))
            .unwrap_or(ProbeRunSeq::FIRST),
        same_status_runs: SameStatusRuns::ONE,
    };
    obs.write_probe_result(row).await.expect("write probe result");
}

fn extract_lifecycle(state: AnyState) -> ServiceLifecycleState {
    match state {
        AnyState::ServiceLifecycle(s) => s,
        other => panic!("expected ServiceLifecycle state, got {other:?}"),
    }
}

// ===========================================================================
// GAP-1-AT-01 — hydrate_desired against IntentStore-persisted Service
// ===========================================================================

/// `WorkloadIntent::Service(Service)` with a declared startup probe
/// → `hydrate_desired` returns a non-default `ServiceLifecycleState`.
///
/// Closes the audit's "AT exercises the placeholder default() return"
/// gap: this AT goes through `hydrate_desired_for_test`, exercising
/// the real production projection.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn gap_1_at_01_hydrate_desired_succeeds_when_service_intent_persisted() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
        as Arc<dyn ObservationStore>;
    let state = build_app_state(&tmp, Arc::clone(&obs)).await;

    let wid = workload_id("svc-with-probe");
    let svc = build_service_spec(&wid, vec![declared_tcp_probe(8080, 30, 2)]);
    persist_service_intent(&state, &svc).await;

    let hydrated =
        hydrate_desired_for_test(&service_lifecycle_reconciler(), &target_for(&wid), &state)
            .await
            .expect("hydrate_desired succeeds");

    let desired = extract_lifecycle(hydrated);
    // Phase 1 desired-side projection: spec-derived facts are folded
    // into actual-side facts at reconcile time; desired.allocs stays
    // empty (the spec doesn't enumerate allocations). The key
    // assertion is `Ok(_)` flowed through the real intent-read path —
    // not the `default()` placeholder.
    assert!(
        desired.allocs.is_empty(),
        "desired.allocs is empty (spec describes the workload, not its allocs); got {:?}",
        desired
    );
}

/// Empty `startup_probes` (the opt-out shape) still hydrates cleanly —
/// the spec read succeeds and the spec-derived helper handles the
/// empty case without panicking.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn gap_1_at_01b_hydrate_desired_handles_empty_startup_probes() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
        as Arc<dyn ObservationStore>;
    let state = build_app_state(&tmp, Arc::clone(&obs)).await;

    let wid = workload_id("svc-opt-out");
    let svc = build_service_spec(&wid, vec![]);
    persist_service_intent(&state, &svc).await;

    let hydrated =
        hydrate_desired_for_test(&service_lifecycle_reconciler(), &target_for(&wid), &state)
            .await
            .expect("hydrate_desired succeeds on empty probes");

    let _ = extract_lifecycle(hydrated);
}

// ===========================================================================
// GAP-1-AT-02 — hydrate_actual sources state and started_at
//                from alloc_status row
// ===========================================================================

/// AllocStatusRow with `state=Running` + `started_at=Some(ts)` → fact
/// carries the row's state + started_at verbatim.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn gap_1_at_02_hydrate_actual_projects_row_state_and_started_at_verbatim() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
        as Arc<dyn ObservationStore>;
    let state = build_app_state(&tmp, Arc::clone(&obs)).await;

    let wid = workload_id("svc-actual");
    let svc = build_service_spec(&wid, vec![declared_tcp_probe(8080, 30, 2)]);
    persist_service_intent(&state, &svc).await;

    let aid = alloc_id("svc-actual-0");
    let started = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    let row = make_alloc_status_row(&wid, &aid, AllocState::Running, Some(started), 1);
    write_alloc_status(&state, row).await;

    let hydrated =
        hydrate_actual_for_test(&service_lifecycle_reconciler(), &target_for(&wid), &state)
            .await
            .expect("hydrate_actual succeeds");

    let actual = extract_lifecycle(hydrated);
    assert_eq!(actual.allocs.len(), 1, "exactly one alloc in actual");
    let fact = actual.allocs.get(&aid).expect("fact for alloc");
    assert_eq!(fact.state(), AllocState::Running);
    assert_eq!(
        fact.started_at,
        Some(started),
        "started_at must propagate from row to fact verbatim",
    );
}

// ===========================================================================
// GAP-1-AT-03 — LWW projection for probe_result rows
// ===========================================================================

/// Two probe_result rows at the same `(alloc_id, probe_idx=0,
/// role=startup)` → actual-side picks the row with the dominating
/// `last_observed_at_unix_ms`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn gap_1_at_03_hydrate_actual_picks_lww_winner_for_startup_probe() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
        as Arc<dyn ObservationStore>;
    let state = build_app_state(&tmp, Arc::clone(&obs)).await;

    let wid = workload_id("svc-lww");
    let svc = build_service_spec(&wid, vec![declared_tcp_probe(8080, 30, 2)]);
    persist_service_intent(&state, &svc).await;

    let aid = alloc_id("svc-lww-0");
    let started = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    write_alloc_status(
        &state,
        make_alloc_status_row(&wid, &aid, AllocState::Running, Some(started), 1),
    )
    .await;

    // Older Fail row — should LOSE to the newer Pass.
    write_probe_result(
        &obs,
        &aid,
        0,
        ProbeRole::Startup,
        ProbeStatus::Fail { last_fail_reason: "older".to_string() },
        1000,
    )
    .await;
    // Newer Pass row — should WIN.
    write_probe_result(&obs, &aid, 0, ProbeRole::Startup, ProbeStatus::Pass, 5000).await;

    let hydrated =
        hydrate_actual_for_test(&service_lifecycle_reconciler(), &target_for(&wid), &state)
            .await
            .expect("hydrate_actual succeeds");

    let actual = extract_lifecycle(hydrated);
    let fact = actual.allocs.get(&aid).expect("fact for alloc");
    assert!(
        matches!(fact.latest_startup_probe, Some(ProbeStatus::Pass)),
        "LWW winner must be the Pass row (last_observed_at 5000 > 1000); got {:?}",
        fact.latest_startup_probe,
    );
    assert_eq!(
        fact.latest_startup_probe_observed_at,
        Some(UnixInstant::from_unix_duration(Duration::from_secs(5))),
        "the Startup/index-0 LWW identity must hydrate beside its status",
    );
}

// ===========================================================================
// GAP-1-AT-04 — empty stores
// ===========================================================================

/// Empty IntentStore + empty ObservationStore → desired empty, actual
/// empty, both succeed (no panic, no error).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn gap_1_at_04_empty_stores_yield_empty_state() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
        as Arc<dyn ObservationStore>;
    let state = build_app_state(&tmp, Arc::clone(&obs)).await;

    let wid = workload_id("svc-absent");
    let target = target_for(&wid);

    let desired = hydrate_desired_for_test(&service_lifecycle_reconciler(), &target, &state)
        .await
        .expect("hydrate_desired on empty stores");
    let desired = extract_lifecycle(desired);
    assert!(desired.allocs.is_empty());

    let actual = hydrate_actual_for_test(&service_lifecycle_reconciler(), &target, &state)
        .await
        .expect("hydrate_actual on empty stores");
    let actual = extract_lifecycle(actual);
    assert!(actual.allocs.is_empty());
}

// ===========================================================================
// GAP-1-AT-05 — regression guard against default() collapse
// ===========================================================================

/// For ANY non-empty input (intent + alloc rows), the returned actual
/// state is structurally distinguishable from
/// `ServiceLifecycleState::default()`. Defensive guard against a
/// future regression that re-introduces the placeholder.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn gap_1_at_05_non_empty_input_yields_non_default_actual() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
        as Arc<dyn ObservationStore>;
    let state = build_app_state(&tmp, Arc::clone(&obs)).await;

    let wid = workload_id("svc-non-default");
    let svc = build_service_spec(&wid, vec![declared_tcp_probe(8080, 30, 2)]);
    persist_service_intent(&state, &svc).await;

    let aid = alloc_id("svc-non-default-0");
    let started = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    write_alloc_status(
        &state,
        make_alloc_status_row(&wid, &aid, AllocState::Running, Some(started), 1),
    )
    .await;

    let actual = extract_lifecycle(
        hydrate_actual_for_test(&service_lifecycle_reconciler(), &target_for(&wid), &state)
            .await
            .expect("hydrate_actual"),
    );

    assert_ne!(
        actual,
        ServiceLifecycleState::default(),
        "hydrate_actual must NOT collapse to default() when inputs exist (GAP-1 regression guard)",
    );
}

// ===========================================================================
// GAP-1-AT-06 — started_at Some/None propagation
// ===========================================================================

/// Row with `state=Failed` + `started_at=None` (driver-rejected start)
/// → fact carries `started_at == None` (not unwrapped, not collapsed
/// to zero per the "Distinct failure modes get distinct error
/// variants" rule).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn gap_1_at_06_started_at_none_propagates_as_none() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
        as Arc<dyn ObservationStore>;
    let state = build_app_state(&tmp, Arc::clone(&obs)).await;

    let wid = workload_id("svc-none");
    let svc = build_service_spec(&wid, vec![declared_tcp_probe(8080, 30, 2)]);
    persist_service_intent(&state, &svc).await;

    let aid = alloc_id("svc-none-0");
    // No Running observation — `started_at == None` on a Failed row.
    write_alloc_status(&state, make_alloc_status_row(&wid, &aid, AllocState::Failed, None, 1))
        .await;

    let actual = extract_lifecycle(
        hydrate_actual_for_test(&service_lifecycle_reconciler(), &target_for(&wid), &state)
            .await
            .expect("hydrate_actual"),
    );
    let fact = actual.allocs.get(&aid).expect("fact present");
    assert_eq!(fact.started_at, None, "None must propagate verbatim — not collapsed to zero");
}

// ===========================================================================
// GAP-1-AT-07 — a terminal fact closes the ServiceLifecycle watch
// ===========================================================================

/// D6/DG-12: a `Failed` allocation with no `started_at` emits no
/// action and joins `terminal_announced`. The same outcome now holds
/// for a started terminal allocation (S-56).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn gap_1_at_07_reconciler_skips_when_started_at_none_on_failed_alloc() {
    use overdrive_core::reconcilers::{Reconciler, TickContext};
    use overdrive_reconcilers::service_lifecycle::{
        ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleView,
    };

    let aid = alloc_id("svc-skip-0");
    let fact = ServiceAllocFact {
        stable_committed: false,
        alloc_id: aid.clone(),
        phase: overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::NotRunning(
            overdrive_reconcilers::service_lifecycle::NotRunningState::new(AllocState::Failed)
                .expect("not Running"),
        ),
        started_at: None,
        latest_startup_probe: None,
        latest_startup_probe_observed_at: None,
        startup_attempts_required: 0, // terminal state wins even when the startup gate could fire
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        probe_rows: BTreeMap::new(),
    };
    let mut allocs = BTreeMap::new();
    allocs.insert(aid.clone(), fact);
    let actual = ServiceLifecycleState {
        allocs,
        service_dataplane: BTreeMap::new(),
        observed_backend_rows: BTreeMap::new(),
    };
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_999_999)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    let reconciler = ServiceLifecycleReconciler::new();
    let (actions, next_view) = reconciler.reconcile(
        &ServiceLifecycleState::default(),
        &actual,
        &ServiceLifecycleView::default(),
        &tick,
    );

    assert!(actions.is_empty(), "a terminal fact emits no ServiceLifecycle action: {actions:?}");
    assert!(next_view.terminal_announced.contains(&aid), "the terminal watch closes");
}

// ===========================================================================
// GAP-1-AT-08 — reconciler unreachable when Running + None(started_at)
// ===========================================================================

/// Hydrate invariant: `state == Running` IFF
/// `started_at == Some(_)`. The reconciler's Stable + opt-out-Stable
/// branches assert this via `unreachable!()` per `.claude/rules/
/// development.md` § "Logically unreachable None / Err — use
/// `unreachable!()`".
///
/// This test deliberately constructs the invalid combination and
/// asserts the panic message matches the structural invariant
/// statement.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "hydrate invariant")]
fn gap_1_at_08_reconciler_unreachable_when_running_alloc_has_no_started_at() {
    use overdrive_core::reconcilers::{Reconciler, TickContext};
    use overdrive_reconcilers::service_lifecycle::{
        ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleView,
    };

    let aid = alloc_id("svc-invariant-0");
    let fact = ServiceAllocFact {
        stable_committed: false,
        alloc_id: aid.clone(),
        phase: overdrive_reconcilers::service_lifecycle::ServiceAllocPhase::Running(
            overdrive_reconcilers::service_lifecycle::ServiceBackendIdentity {
                spiffe: overdrive_core::SpiffeId::new(
                    "spiffe://overdrive.local/workload/svc/alloc/x",
                )
                .expect("valid spiffe"),
                ip: std::net::Ipv4Addr::LOCALHOST,
            },
        ), // load-bearing: triggers the unreachable
        started_at: None, // invalid combination
        latest_startup_probe: Some(ProbeStatus::Pass),
        latest_startup_probe_observed_at: Some(UnixInstant::from_unix_duration(
            Duration::from_millis(1),
        )),
        startup_attempts_required: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        probe_rows: BTreeMap::from([(
            (ProbeRole::Startup, ProbeIdx::new(0)),
            ProbeResultRow {
                alloc_id: aid.clone(),
                probe_idx: ProbeIdx::new(0),
                role: ProbeRole::Startup,
                status: ProbeStatus::Pass,
                last_observed_at_unix_ms: 1,
                inferred: false,
                run_seq: ProbeRunSeq::FIRST,
                same_status_runs: SameStatusRuns::ONE,
            },
        )]),
    };
    let mut allocs = BTreeMap::new();
    allocs.insert(aid.clone(), fact);
    let actual = ServiceLifecycleState {
        allocs,
        service_dataplane: BTreeMap::new(),
        observed_backend_rows: BTreeMap::new(),
    };
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_999_999)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    let reconciler = ServiceLifecycleReconciler::new();
    // This call MUST panic with the "hydrate invariant" message.
    let _ = reconciler.reconcile(
        &ServiceLifecycleState::default(),
        &actual,
        &ServiceLifecycleView::default(),
        &tick,
    );
}

// ===========================================================================
// D6 — the crash cause stays on the durable row, not ServiceAllocFact.
// ===========================================================================

/// Service-kind hydration leaves the observed row intact. The restart
/// authority reads its typed cause at give-up (D6): crash with an exit code,
/// signal-only crash, and a non-crash reason retain their distinct payloads.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn crash_reason_remains_on_observation_row_after_service_hydration() {
    use overdrive_core::transition_reason::{StoppedBy, TransitionReason};

    let cases: Vec<(&str, Option<TransitionReason>)> = vec![
        (
            "crash-exit-1",
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(1),
                signal: None,
                stderr_tail: None,
            }),
        ),
        (
            "crash-signal-only",
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: None,
                signal: Some(9),
                stderr_tail: None,
            }),
        ),
        ("non-crash-reason", Some(TransitionReason::Stopped { by: StoppedBy::Process })),
    ];

    for (suffix, reason) in cases {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
            as Arc<dyn ObservationStore>;
        let state = build_app_state(&tmp, Arc::clone(&obs)).await;

        let wid = workload_id(&format!("svc-exit-{suffix}"));
        let svc = build_service_spec(&wid, vec![declared_tcp_probe(8080, 30, 2)]);
        persist_service_intent(&state, &svc).await;

        let aid = alloc_id(&format!("svc-exit-{suffix}-0"));
        let started = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
        // The row carries the typed cause the restart authority will read;
        // ServiceLifecycle hydration projects state and start time only.
        write_alloc_status(
            &state,
            make_alloc_status_row_with_reason(
                &wid,
                &aid,
                AllocState::Failed,
                Some(started),
                1,
                reason.clone(),
            ),
        )
        .await;

        let actual = extract_lifecycle(
            hydrate_actual_for_test(&service_lifecycle_reconciler(), &target_for(&wid), &state)
                .await
                .expect("hydrate_actual"),
        );
        let fact = actual.allocs.get(&aid).expect("fact present");
        assert_eq!(fact.state(), AllocState::Failed, "{suffix}: state is still hydrated");
        assert_eq!(fact.started_at, Some(started), "{suffix}: started_at is still hydrated");
        let observed = state.obs.alloc_status_row(&aid).await.expect("row read").expect("row");
        assert_eq!(observed.reason, reason, "{suffix}: the typed cause remains on its row");
    }
}

// ===========================================================================
// Smoke: the synthesized TCP startup probe supplies the hydration mechanic.
// ===========================================================================

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn default_tcp_startup_probe_supplies_hydration_mechanic() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
        as Arc<dyn ObservationStore>;
    let state = build_app_state(&tmp, Arc::clone(&obs)).await;

    let wid = workload_id("svc-inferred");
    let svc = build_service_spec(&wid, vec![inferred_tcp_probe(8080)]);
    persist_service_intent(&state, &svc).await;

    let aid = alloc_id("svc-inferred-0");
    let started = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    write_alloc_status(
        &state,
        make_alloc_status_row(&wid, &aid, AllocState::Running, Some(started), 1),
    )
    .await;

    let actual = extract_lifecycle(
        hydrate_actual_for_test(&service_lifecycle_reconciler(), &target_for(&wid), &state)
            .await
            .expect("hydrate_actual"),
    );
    let fact = actual.allocs.get(&aid).expect("fact present");
    assert_eq!(fact.mechanic_summary, "tcp 0.0.0.0:8080");
}

// ===========================================================================
// ADR-0276 — a Service backend is the Running allocation's workload address,
// never the node address.
//
// Driven through the production hydration (`hydrate_actual_for_test` over the
// ObservationStore port and the VIP allocator memo) and the production
// reconcile; the oracle is the published `ServiceBackendRow`.
// ===========================================================================

mod backend_address {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use overdrive_control_plane::reconciler_runtime::{ConvergenceError, run_convergence_tick};
    use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
    use overdrive_core::id::ServiceId;
    use overdrive_core::reconcilers::{
        Action, HydrateError, Reconciler, ReconcilerName, TickContext,
    };
    use overdrive_core::traits::observation_store::ServiceBackendRow;
    use overdrive_reconcilers::service_lifecycle::ServiceLifecycleView;

    const LISTENER_PORT: u16 = 8080;

    /// Allocate the Service's VIP through the production allocator path and
    /// return the `ServiceId` of its single TCP listener.
    async fn allocate_service_id(state: &AppState, svc: &Service) -> ServiceId {
        let intent = WorkloadIntent::Service(svc.clone());
        let digest = intent.spec_digest().expect("spec_digest");
        let bytes: [u8; 32] = *digest.as_bytes();
        let vip = {
            let mut guard = state.allocator.lock().await;
            guard.allocate(bytes).await.expect("allocate vip")
        };
        let port = std::num::NonZeroU16::new(LISTENER_PORT).expect("listener port");
        ServiceId::derive(&vip, port, Proto::Tcp, "service-map")
    }

    /// A Service with persisted intent and an allocated VIP.
    async fn deploy_service(state: &AppState, name: &str) -> (WorkloadId, ServiceId) {
        let wid = workload_id(name);
        let svc = build_service_spec(&wid, vec![]);
        persist_service_intent(state, &svc).await;
        let service_id = allocate_service_id(state, &svc).await;
        (wid, service_id)
    }

    /// A Running row of `wid` whose observed workload address is `addr`.
    fn running_row(wid: &WorkloadId, alloc: &str, addr: Ipv4Addr, counter: u64) -> AllocStatusRow {
        let started = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
        AllocStatusRow {
            workload_addr: Some(addr),
            ..make_alloc_status_row(
                wid,
                &alloc_id(alloc),
                AllocState::Running,
                Some(started),
                counter,
            )
        }
    }

    fn tick() -> TickContext {
        let now = Instant::now();
        TickContext {
            now,
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_999_999)),
            tick: 0,
            deadline: now + Duration::from_secs(1),
        }
    }

    /// Hydrate desired and actual for `wid` and reconcile once; return every
    /// `ServiceBackendRow` the reconcile publishes.
    async fn publish_backend_rows(state: &AppState, wid: &WorkloadId) -> Vec<ServiceBackendRow> {
        let reconciler = service_lifecycle_reconciler();
        let target = target_for(wid);
        let desired = extract_lifecycle(
            hydrate_desired_for_test(&reconciler, &target, state).await.expect("hydrate_desired"),
        );
        let actual = extract_lifecycle(
            hydrate_actual_for_test(&reconciler, &target, state).await.expect("hydrate_actual"),
        );
        let (actions, _) = ServiceLifecycleReconciler::new().reconcile(
            &desired,
            &actual,
            &ServiceLifecycleView::default(),
            &tick(),
        );
        actions
            .into_iter()
            .filter_map(|action| match action {
                Action::WriteServiceBackendRow { row, .. } => Some(row),
                _ => None,
            })
            .collect()
    }

    fn keys(row: &ServiceBackendRow) -> std::collections::BTreeSet<BackendKey> {
        row.backends
            .iter()
            .map(|backend| match backend.addr {
                SocketAddr::V4(addr) => BackendKey::new(*addr.ip(), addr.port(), Proto::Tcp),
                SocketAddr::V6(addr) => panic!("a Service backend is IPv4, got {addr}"),
            })
            .collect()
    }

    async fn new_state(tmp: &TempDir) -> AppState {
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
            as Arc<dyn ObservationStore>;
        build_app_state(tmp, obs).await
    }

    /// O1 — a Running allocation with its workload address is published as
    /// the backend `address:listener_port`; the node address (the sim
    /// composer's 127.0.0.1) never appears.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn a_running_allocation_is_published_as_a_backend_at_its_workload_address() {
        let tmp = TempDir::new().expect("tmpdir");
        let state = new_state(&tmp).await;
        let (wid, service_id) = deploy_service(&state, "svc-addr").await;
        let addr = guest_addr(6);
        write_alloc_status(&state, running_row(&wid, "svc-addr-0", addr, 1)).await;

        let rows = publish_backend_rows(&state, &wid).await;

        assert_eq!(rows.len(), 1, "one ServiceBackendRow for the one listener, got {rows:?}");
        let row = &rows[0];
        assert_eq!(row.service_id, service_id);
        assert_eq!(
            row.backends.iter().map(|b| b.addr).collect::<Vec<_>>(),
            vec![SocketAddr::new(IpAddr::V4(addr), LISTENER_PORT)],
            "the backend is the allocation's own workload address and the listener port",
        );
        assert!(
            row.backends.iter().all(|b| b.addr.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST)),
            "no backend carries the node address",
        );
    }

    /// T2 — two Services never share a backend endpoint: each Service's
    /// backend row holds only its own allocations' endpoints and the two rows'
    /// `BackendKey` sets are disjoint.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn two_services_publish_only_their_own_allocations_and_disjoint_backend_keys() {
        let tmp = TempDir::new().expect("tmpdir");
        let state = new_state(&tmp).await;
        let (wid_a, service_a) = deploy_service(&state, "svc-alpha").await;
        let (wid_b, service_b) = deploy_service(&state, "svc-beta").await;
        assert_ne!(service_a, service_b, "two Services have two ServiceIds");

        let alpha = [guest_addr(2), guest_addr(3)];
        let beta = [guest_addr(4), guest_addr(5)];
        for (index, addr) in alpha.iter().enumerate() {
            write_alloc_status(
                &state,
                running_row(&wid_a, &format!("svc-alpha-{index}"), *addr, 1),
            )
            .await;
        }
        for (index, addr) in beta.iter().enumerate() {
            write_alloc_status(&state, running_row(&wid_b, &format!("svc-beta-{index}"), *addr, 1))
                .await;
        }

        let row_a = publish_backend_rows(&state, &wid_a).await.pop().expect("alpha row");
        let row_b = publish_backend_rows(&state, &wid_b).await.pop().expect("beta row");

        let expected = |addrs: &[Ipv4Addr]| {
            addrs
                .iter()
                .map(|addr| BackendKey::new(*addr, LISTENER_PORT, Proto::Tcp))
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(row_a.service_id, service_a);
        assert_eq!(row_b.service_id, service_b);
        assert_eq!(keys(&row_a), expected(&alpha), "alpha publishes only its own allocations");
        assert_eq!(keys(&row_b), expected(&beta), "beta publishes only its own allocations");
        assert!(
            keys(&row_a).is_disjoint(&keys(&row_b)),
            "no BackendKey is in the backend sets of two Services",
        );
    }

    /// O3 — a non-Running allocation hydrates whether or not it carries an
    /// address, and is never published as a backend; a Running allocation
    /// beside it still is.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn non_running_allocations_without_an_address_hydrate_and_are_not_published() {
        let states = [
            AllocState::Pending,
            AllocState::Draining,
            AllocState::Suspended,
            AllocState::Terminated,
            AllocState::Failed,
        ];
        for (index, alloc_state) in states.into_iter().enumerate() {
            let tmp = TempDir::new().expect("tmpdir");
            let state = new_state(&tmp).await;
            let (wid, _) = deploy_service(&state, "svc-idle").await;
            let started = (alloc_state != AllocState::Pending)
                .then(|| UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)));
            let idle =
                make_alloc_status_row(&wid, &alloc_id("svc-idle-0"), alloc_state, started, 1);
            assert_eq!(idle.workload_addr, None, "{alloc_state:?} fixture carries no address");
            write_alloc_status(&state, idle).await;
            let live = guest_addr(7);
            write_alloc_status(&state, running_row(&wid, "svc-idle-1", live, 2)).await;

            let actual = extract_lifecycle(
                hydrate_actual_for_test(&service_lifecycle_reconciler(), &target_for(&wid), &state)
                    .await
                    .unwrap_or_else(|e| {
                        panic!("{alloc_state:?} without an address must hydrate: {e}")
                    }),
            );
            assert!(
                actual.allocs.contains_key(&alloc_id("svc-idle-0")),
                "{alloc_state:?} allocation hydrates (case {index})",
            );

            let rows = publish_backend_rows(&state, &wid).await;
            let row = rows.last().expect("a backend row is published");
            assert_eq!(
                row.backends.iter().map(|b| b.addr).collect::<Vec<_>>(),
                vec![SocketAddr::new(IpAddr::V4(live), LISTENER_PORT)],
                "{alloc_state:?} allocation is not a backend; the Running one is",
            );
        }
    }

    /// Drive one production convergence tick of `service-lifecycle` for `wid`:
    /// hydrate through the runtime, reconcile, and dispatch the actions, so a
    /// `ServiceBackendRow` lands in the ObservationStore only if the tick
    /// completes.
    async fn converge_service(state: &AppState, wid: &WorkloadId) -> Result<(), ConvergenceError> {
        let name = ReconcilerName::new("service-lifecycle").expect("reconciler name");
        let now = Instant::now();
        run_convergence_tick(state, &name, &target_for(wid), now, 0, now + Duration::from_secs(1))
            .await
    }

    /// O2 — a Running allocation without a workload address is refused with a
    /// typed hydration failure naming it; it is neither defaulted to the node
    /// address nor skipped, and no backend row is written for the Service. The
    /// valid Running allocation of the same Service does not rescue the tick.
    /// Once the offending row carries its address the same tick publishes the
    /// Service's backend row, so the absence of a row is caused by the refusal.
    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test]
    async fn a_running_allocation_without_a_workload_address_is_refused_not_defaulted() {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
            as Arc<dyn ObservationStore>;
        let state = build_app_state(&tmp, Arc::clone(&obs)).await;
        let (wid, service_id) = deploy_service(&state, "svc-refuse").await;
        write_alloc_status(&state, running_row(&wid, "svc-refuse-good", guest_addr(2), 1)).await;
        let bad = AllocStatusRow {
            workload_addr: None,
            ..running_row(&wid, "svc-refuse-bad", guest_addr(3), 2)
        };
        write_alloc_status(&state, bad).await;

        let refused = converge_service(&state, &wid)
            .await
            .expect_err("a Running allocation without an address fails the tick");

        let ConvergenceError::Hydrate(HydrateError::RunningAllocationWithoutWorkloadAddr {
            alloc_id: refused_alloc,
        }) = refused
        else {
            panic!("the refusal is RunningAllocationWithoutWorkloadAddr, got {refused:?}");
        };
        assert_eq!(
            refused_alloc,
            alloc_id("svc-refuse-bad"),
            "the refusal names exactly the offending allocation, not its valid sibling",
        );
        assert!(
            obs.service_backends_rows(&service_id).await.expect("read rows").is_empty(),
            "no ServiceBackendRow is written for the Service while the refusal stands",
        );

        // Control: the offending allocation's row is corrected to carry its
        // address; the same tick now completes and publishes the backends.
        write_alloc_status(&state, running_row(&wid, "svc-refuse-bad", guest_addr(3), 3)).await;
        converge_service(&state, &wid).await.expect("the corrected Service converges");
        let rows = obs.service_backends_rows(&service_id).await.expect("read rows");
        assert_eq!(rows.len(), 1, "the corrected Service publishes its backend row: {rows:?}");
        assert_eq!(
            rows[0].backends.iter().map(|b| b.addr).collect::<std::collections::BTreeSet<_>>(),
            [guest_addr(2), guest_addr(3)]
                .into_iter()
                .map(|a| SocketAddr::new(IpAddr::V4(a), LISTENER_PORT))
                .collect(),
            "both Running allocations are backends at their own workload addresses",
        );
    }
}
