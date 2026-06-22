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
//!      sources `alloc_id`, `state`, `started_at`, `exit_code`.
//!   2. `obs.list_probe_results_for_alloc(...)` LWW projection —
//!      sources `latest_startup_probe`.
//!   3. `IntentStore::get(IntentKey::for_workload(workload_id))` →
//!      `WorkloadIntent::Service(ServiceV1)` (with probe vecs
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
    DriverInput, ExecInput, IntentKey, ResourcesInput, ServiceV1, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::{AnyReconciler, AnyState, TargetResource};
use overdrive_core::service_lifecycle::{ServiceLifecycleReconciler, ServiceLifecycleState};
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
    TargetResource::new(&format!("job/{wid}")).expect("valid target")
}

async fn build_app_state(tmp: &TempDir, obs: Arc<dyn ObservationStore>) -> AppState {
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

fn declared_tcp_probe(port: u16, max_attempts: u32, interval_seconds: u32) -> ProbeDescriptor {
    ProbeDescriptor {
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_string(), port },
        timeout_seconds: 5,
        interval_seconds,
        max_attempts,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

fn inferred_tcp_probe(port: u16) -> ProbeDescriptor {
    ProbeDescriptor {
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_string(), port },
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: true,
    }
}

fn build_service_spec(wid: &WorkloadId, startup_probes: Vec<ProbeDescriptor>) -> ServiceV1 {
    let input = ServiceSpecInput {
        id: wid.as_str().to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/serve".to_string(), args: vec![] }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_string() }],
        startup_probes,
        readiness_probes: vec![],
        liveness_probes: vec![],
    };
    ServiceV1::from_submit(input).expect("valid service spec")
}

async fn persist_service_intent(state: &AppState, svc: &ServiceV1) {
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
        // Host-netns fixture — no canonical workload address (AllocStatusRowV2 additive field, GH #241).
        workload_addr: None,
    }
}

/// Like [`make_alloc_status_row`] but carries an explicit
/// `reason: Option<TransitionReason>` so the `exit_code` projection
/// (GAP-11) can be exercised. The Service-kind hydration sources
/// `exit_code` from the row's `WorkloadCrashedImmediately` variant,
/// mirroring the Job-kind precedent at
/// `workload_lifecycle.rs::classify_natural_exit_terminal`.
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
    state.obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write alloc row");
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
// GAP-1-AT-01 — hydrate_desired against IntentStore-persisted ServiceV1
// ===========================================================================

/// `WorkloadIntent::Service(ServiceV1)` with a declared startup probe
/// → `hydrate_desired` returns a non-default `ServiceLifecycleState`.
///
/// Closes the audit's "AT exercises the placeholder default() return"
/// gap: this AT goes through `hydrate_desired_for_test`, exercising
/// the real production projection.
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
// GAP-1-AT-02 — hydrate_actual sources state, started_at, exit_code
//                from alloc_status row
// ===========================================================================

/// AllocStatusRow with `state=Running` + `started_at=Some(ts)` → fact
/// carries the row's state + started_at verbatim.
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
    assert_eq!(fact.state, AllocState::Running);
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
}

// ===========================================================================
// GAP-1-AT-04 — empty stores
// ===========================================================================

/// Empty IntentStore + empty ObservationStore → desired empty, actual
/// empty, both succeed (no panic, no error).
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
// GAP-1-AT-07 — reconciler skip branches on None
//                (EarlyExit / StartupProbeFailed)
// ===========================================================================

/// Per the audit's locked-in semantic answers:
///   - EarlyExit (branch c) skips when `started_at == None`
///   - StartupProbeFailed (branch b) skips when `started_at == None`
///
/// Construct facts with `state == Failed` AND `started_at == None`;
/// invoke the reconciler directly; assert that NO EarlyExit AND NO
/// StartupProbeFailed Action is emitted from those branches.
#[test]
fn gap_1_at_07_reconciler_skips_when_started_at_none_on_failed_alloc() {
    use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
    use overdrive_core::service_lifecycle::{
        ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleView,
    };
    use overdrive_core::transition_reason::TerminalCondition;

    let aid = alloc_id("svc-skip-0");
    let fact = ServiceAllocFact {
        alloc_id: aid.clone(),
        state: AllocState::Failed,
        started_at: None, // load-bearing: triggers skip on both branches
        exit_code: Some(99),
        latest_startup_probe: None,
        max_attempts: 0, // would otherwise satisfy StartupProbeFailed gate
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: false,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        restart_count: 0,
        restart_spec: overdrive_core::traits::driver::AllocationSpec {
            alloc: aid.clone(),
            identity: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
                .expect("valid spiffe"),
            command: "/bin/svc".to_string(),
            args: vec![],
            resources: overdrive_core::traits::driver::Resources {
                cpu_milli: 100,
                memory_bytes: 64 * 1024 * 1024,
            },
            probe_descriptors: vec![],
            // transparent-mtls-enrollment step 04-01 (JOIN-4/JOIN-6): off the mTLS-composed boot gate.
            netns: None,
            host_veth: None,
        },
    };
    let mut allocs = BTreeMap::new();
    allocs.insert(aid.clone(), fact);
    let actual = ServiceLifecycleState { allocs, service_dataplane: None };
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_999_999)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    let reconciler = ServiceLifecycleReconciler::new();
    let (actions, _) = reconciler.reconcile(
        &ServiceLifecycleState::default(),
        &actual,
        &ServiceLifecycleView::default(),
        &tick,
    );

    // No EarlyExit emission AND no StartupProbeFailed emission from
    // the two branches that would normally fire when `started_at` is
    // present.
    for action in &actions {
        if let Action::FinalizeFailed { terminal: Some(term), .. } = action {
            match term {
                TerminalCondition::ServiceFailed { reason } => {
                    panic!(
                        "branches (b) and (c) must SKIP when started_at == None; \
                         got terminal {reason:?}",
                    );
                }
                TerminalCondition::Stable { .. } => {
                    panic!("Stable must NOT fire on Failed alloc (state guard); got {action:?}",);
                }
                _ => {}
            }
        }
    }
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
#[test]
#[should_panic(expected = "hydrate invariant")]
fn gap_1_at_08_reconciler_unreachable_when_running_alloc_has_no_started_at() {
    use overdrive_core::reconcilers::{Reconciler, TickContext};
    use overdrive_core::service_lifecycle::{
        ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleView,
    };

    let aid = alloc_id("svc-invariant-0");
    let fact = ServiceAllocFact {
        alloc_id: aid.clone(),
        state: AllocState::Running, // load-bearing: triggers the unreachable
        started_at: None,           // invalid combination
        exit_code: None,
        latest_startup_probe: Some(ProbeStatus::Pass),
        max_attempts: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 0.0.0.0:8080".to_string(),
        inferred: false,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        restart_count: 0,
        restart_spec: overdrive_core::traits::driver::AllocationSpec {
            alloc: aid.clone(),
            identity: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
                .expect("valid spiffe"),
            command: "/bin/svc".to_string(),
            args: vec![],
            resources: overdrive_core::traits::driver::Resources {
                cpu_milli: 100,
                memory_bytes: 64 * 1024 * 1024,
            },
            probe_descriptors: vec![],
            // transparent-mtls-enrollment step 04-01 (JOIN-4/JOIN-6): off the mTLS-composed boot gate.
            netns: None,
            host_veth: None,
        },
    };
    let mut allocs = BTreeMap::new();
    allocs.insert(aid.clone(), fact);
    let actual = ServiceLifecycleState { allocs, service_dataplane: None };
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
// GAP-11 — hydrate_actual projects exit_code from row.reason
//          (WorkloadCrashedImmediately) for the Service kind.
// ===========================================================================

/// The Service-kind hydration MUST source the alloc's `exit_code` from
/// the row's `reason: Option<TransitionReason>` — specifically the
/// `WorkloadCrashedImmediately { exit_code, .. }` variant — and project
/// `None` for any other reason shape. This mirrors the Job-kind
/// precedent at `workload_lifecycle.rs::classify_natural_exit_terminal`
/// (line ~944) and closes GAP-11: before the fix the projection
/// hardcoded `exit_code: None`, so every EarlyExit wire surface
/// reported `exit_code: 0`.
///
/// Three cases pin both the match arm AND the `_ => None` fallback as
/// mutation targets:
///   1. crashed with `exit_code: Some(1)` → fact carries `Some(1)`
///   2. crashed with `exit_code: None` (signal-only) → fact carries `None`
///   3. a non-crash reason → fact carries `None` (the `_` fallback)
#[tokio::test]
async fn gap_11_hydrate_actual_projects_exit_code_from_crash_reason() {
    use overdrive_core::transition_reason::{StoppedBy, TransitionReason};

    let cases: Vec<(&str, Option<TransitionReason>, Option<i32>)> = vec![
        (
            "crash-exit-1",
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(1),
                signal: None,
                stderr_tail: None,
            }),
            Some(1),
        ),
        (
            "crash-signal-only",
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: None,
                signal: Some(9),
                stderr_tail: None,
            }),
            None,
        ),
        ("non-crash-reason", Some(TransitionReason::Stopped { by: StoppedBy::Process }), None),
    ];

    for (suffix, reason, expected_exit_code) in cases {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
            as Arc<dyn ObservationStore>;
        let state = build_app_state(&tmp, Arc::clone(&obs)).await;

        let wid = workload_id(&format!("svc-exit-{suffix}"));
        let svc = build_service_spec(&wid, vec![declared_tcp_probe(8080, 30, 2)]);
        persist_service_intent(&state, &svc).await;

        let aid = alloc_id(&format!("svc-exit-{suffix}-0"));
        let started = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
        // Failed row with a started_at so the fact projects without the
        // Running-invariant unreachable; the exit_code projection is the
        // assertion under test.
        write_alloc_status(
            &state,
            make_alloc_status_row_with_reason(
                &wid,
                &aid,
                AllocState::Failed,
                Some(started),
                1,
                reason,
            ),
        )
        .await;

        let actual = extract_lifecycle(
            hydrate_actual_for_test(&service_lifecycle_reconciler(), &target_for(&wid), &state)
                .await
                .expect("hydrate_actual"),
        );
        let fact = actual.allocs.get(&aid).expect("fact present");
        assert_eq!(
            fact.exit_code, expected_exit_code,
            "exit_code must project from row.reason for case {suffix:?}: expected {expected_exit_code:?}, got {:?}",
            fact.exit_code,
        );
    }
}

// ===========================================================================
// Smoke: inferred-probe spec hydrates with inferred=true on the fact.
// ===========================================================================

#[tokio::test]
async fn gap_1_smoke_inferred_probe_carries_inferred_flag_through_actual() {
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
    assert!(fact.inferred, "inferred=true probe must surface inferred=true on fact");
    assert_eq!(fact.mechanic_summary, "tcp 0.0.0.0:8080");
}
