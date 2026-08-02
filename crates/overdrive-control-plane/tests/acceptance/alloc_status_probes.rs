//! `GET /v1/allocs` carries the probe surface — the handler-layer half
//! of the probe-observability chain (US-06 / K4; EDD O02).
//!
//! The operator-facing proof lives in
//! `overdrive-cli::integration::workload_describe_probes` (real `serve`
//! → real Service → real render). These tests pin the SAME contract one
//! layer down, at the control-plane port, for two reasons:
//!
//! 1. **The handler's contract is its own.** `alloc_status` is the only
//!    reader of the probe surface; whether it projects the intent-side
//!    declarations and the observation-side rows is a control-plane
//!    fact, independent of any CLI.
//! 2. **A cross-package test cannot defend it.** `cargo xtask mutants
//!    --package overdrive-control-plane` runs this crate's tests, so a
//!    killer living only in `overdrive-cli` leaves
//!    `service_probe_descriptors` / `probe_results_for_rows` as MISSED
//!    mutants — the suite would not actually assert on the thing that
//!    matters (`.claude/rules/testing.md` § "Mutation testing").
//!
//! Default-lane: in-process axum handler call, sim observation store
//! seeded with rows, no real I/O — same shape as
//! `alloc_status_snapshot.rs`.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use overdrive_control_plane::AppState;
use overdrive_control_plane::api::{AllocStatusResponse, ProbeStatusJson};
use overdrive_control_plane::handlers::{AllocStatusQuery, alloc_status};
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput, ServiceV1,
    WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::probe_result_row::{
    ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus,
};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const SERVICE_ID: &str = "payments-svc";
const JOB_ID: &str = "payments-job";

fn node() -> NodeId {
    NodeId::from_str("node-a").expect("valid node id")
}

fn alloc(id: &str) -> AllocationId {
    AllocationId::from_str(id).expect("valid alloc id")
}

fn build_app_state(tmp: &TempDir) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(node(), 0));
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
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        NodeId::new("writer-1").unwrap(),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

/// A probe descriptor with the ADR-0057 defaults applied, as the parser
/// would emit it.
fn descriptor(
    role: ProbeRole,
    idx: u32,
    mechanic: ProbeMechanic,
    inferred: bool,
) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(idx),
        role,
        mechanic,
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: matches!(role, ProbeRole::Liveness).then_some(3),
        success_threshold: matches!(role, ProbeRole::Readiness).then_some(1),
        inferred,
    }
}

/// Persist a `WorkloadIntent::Service` carrying `startup` + `liveness`
/// probes, plus its kind discriminator record.
async fn install_service(state: &AppState) -> ServiceV1 {
    let spec = ServiceSpecInput {
        id: SERVICE_ID.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput {
            command: "/usr/local/bin/payments".to_owned(),
            args: vec![],
        }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
        startup_probes: vec![descriptor(
            ProbeRole::Startup,
            0,
            ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: 8080 },
            /*inferred=*/ true,
        )],
        readiness_probes: vec![],
        liveness_probes: vec![descriptor(
            ProbeRole::Liveness,
            0,
            ProbeMechanic::Http {
                path: "/healthz".to_owned(),
                port: 8080,
                host: Some("127.0.0.1".to_owned()),
            },
            /*inferred=*/ false,
        )],
    };
    let svc = ServiceV1::from_submit(spec).expect("ServiceV1::from_submit");
    let id = WorkloadId::new(SERVICE_ID).expect("valid workload id");
    let archived =
        WorkloadIntent::Service(svc.clone()).archive_for_store().expect("rkyv archive Service");
    state
        .store
        .put(IntentKey::for_workload(&id).as_bytes(), archived.as_ref())
        .await
        .expect("IntentStore put Service");
    state
        .store
        .put(
            IntentKey::for_workload_kind(&id).as_bytes(),
            &[WorkloadKind::Service.discriminator_byte()],
        )
        .await
        .expect("IntentStore put kind");
    svc
}

/// Persist a `WorkloadIntent::Job` — the kind that cannot declare probes.
async fn install_job(state: &AppState) -> Job {
    let job = Job::from_submit(JobSpecInput {
        id: JOB_ID.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_owned(), args: vec![] }),
    })
    .expect("Job::from_submit");
    let archived = WorkloadIntent::Job(job.clone()).archive_for_store().expect("rkyv archive Job");
    state
        .store
        .put(IntentKey::for_workload(&job.id).as_bytes(), archived.as_ref())
        .await
        .expect("IntentStore put Job");
    state
        .store
        .put(
            IntentKey::for_workload_kind(&job.id).as_bytes(),
            &[WorkloadKind::Job.discriminator_byte()],
        )
        .await
        .expect("IntentStore put kind");
    job
}

async fn write_running_alloc(state: &AppState, alloc_id: &AllocationId, workload_id: &WorkloadId) {
    let row = AllocStatusRow {
        alloc_id: alloc_id.clone(),
        workload_id: workload_id.clone(),
        node_id: node(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 2, writer: node() },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };
    state.obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("obs write alloc");
}

async fn write_probe_row(
    state: &AppState,
    alloc_id: &AllocationId,
    role: ProbeRole,
    idx: u32,
    status: ProbeStatus,
    at_ms: u64,
) {
    state
        .obs
        .write_probe_result(ProbeResultRow {
            alloc_id: alloc_id.clone(),
            probe_idx: ProbeIdx::new(idx),
            role,
            status,
            last_observed_at_unix_ms: at_ms,
            inferred: false,
        })
        .await
        .expect("obs write probe result");
}

async fn read_snapshot(state: &AppState, workload_id: &str) -> AllocStatusResponse {
    alloc_status(
        State(state.clone()),
        Query(AllocStatusQuery { job: Some(workload_id.to_owned()) }),
    )
    .await
    .expect("alloc_status returned err")
    .0
}

// ---------------------------------------------------------------------------
// Declaration side — `AllocStatusResponse.probes`
// ---------------------------------------------------------------------------

/// The handler projects the persisted Service aggregate's probe
/// declarations onto the response, flattened `(role, idx)`-ascending.
///
/// The order assertion is load-bearing, not cosmetic: the CLI joins the
/// declaration side against the observation side, and
/// `ObservationStore::list_probe_results_for_alloc` guarantees
/// role-key-byte order with ascending `probe_idx` within a role. If the
/// handler flattened in some other order the operator-visible row order
/// would drift between reads.
#[tokio::test]
async fn alloc_status_projects_service_probe_declarations_in_role_then_idx_order() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    install_service(&state).await;

    let body = read_snapshot(&state, SERVICE_ID).await;

    assert_eq!(
        body.probes.len(),
        2,
        "the Service declares one startup and one liveness probe; the handler must project \
         both. Got: {:?}",
        body.probes,
    );
    assert_eq!(
        body.probes[0].role,
        ProbeRole::Startup,
        "startup precedes liveness — `ProbeRole`'s declaration order IS its `Ord` and its \
         durable key-byte order; got {:?}",
        body.probes.iter().map(|p| p.role).collect::<Vec<_>>(),
    );
    assert_eq!(body.probes[1].role, ProbeRole::Liveness);

    // Per-role indexing (ADR-0080 § D1): the liveness probe is index 0
    // of the LIVENESS array, not index 1 of the flattened vector. A
    // handler that re-derived the index from its position in the
    // flattened projection would report 1 here and the CLI join would
    // then miss every liveness observation.
    assert_eq!(body.probes[0].idx.get(), 0, "startup probe is idx 0 within its own role array");
    assert_eq!(
        body.probes[1].idx.get(),
        0,
        "liveness probe is idx 0 within its OWN role array — NOT 1. `probe_idx` is per-role \
         (ADR-0080 § D1); deriving it from the flat concatenation is the defect that ADR \
         restored `ProbeDescriptor.idx` to prevent",
    );

    // The mechanic and the inferred flag are what the operator render
    // reads out of the declaration side.
    assert!(
        matches!(&body.probes[0].mechanic, ProbeMechanic::Tcp { host, port }
                 if host == "0.0.0.0" && *port == 8080),
        "the declaration's mechanic must survive the projection verbatim; got {:?}",
        body.probes[0].mechanic,
    );
    assert!(body.probes[0].inferred, "the startup probe was declared inferred");
    assert!(!body.probes[1].inferred, "the liveness probe was operator-declared");
}

// ---------------------------------------------------------------------------
// Observation side — `AllocStatusResponse.probe_results`
// ---------------------------------------------------------------------------

/// The handler reads the durable probe rows for every allocation of the
/// workload and projects them onto the response.
///
/// This is the link whose absence made probe state unobservable: the
/// rows were durable all along; nothing read them.
#[tokio::test]
async fn alloc_status_projects_observed_probe_rows_for_every_allocation() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    install_service(&state).await;
    let workload_id = WorkloadId::new(SERVICE_ID).expect("valid workload id");

    let alloc_0 = alloc("alloc-payments-svc-0");
    let alloc_1 = alloc("alloc-payments-svc-1");
    write_running_alloc(&state, &alloc_0, &workload_id).await;
    write_running_alloc(&state, &alloc_1, &workload_id).await;

    // Replica 0: startup passed, liveness now failing. Replica 1:
    // startup passed only — its liveness probe has not ticked.
    write_probe_row(&state, &alloc_0, ProbeRole::Startup, 0, ProbeStatus::Pass, 1_000).await;
    write_probe_row(
        &state,
        &alloc_0,
        ProbeRole::Liveness,
        0,
        ProbeStatus::Fail { last_fail_reason: "HTTP 503".to_owned() },
        2_000,
    )
    .await;
    write_probe_row(&state, &alloc_1, ProbeRole::Startup, 0, ProbeStatus::Pass, 3_000).await;

    let body = read_snapshot(&state, SERVICE_ID).await;

    assert_eq!(
        body.probe_results.len(),
        3,
        "three durable rows were written across two allocations; the handler must project \
         every one — a per-allocation read that stopped at the first alloc would return 2. \
         Got: {:?}",
        body.probe_results,
    );

    let liveness = body
        .probe_results
        .iter()
        .find(|r| r.role == ProbeRole::Liveness.as_str())
        .expect("the observed liveness row must be projected");
    assert_eq!(
        liveness.alloc_id,
        alloc_0.to_string(),
        "the failing liveness row belongs to replica 0 — the durable PK is per-allocation \
         (ADR-0080 § D2), so the projection must carry alloc identity or the operator cannot \
         tell which replica is sick",
    );
    assert_eq!(
        liveness.status,
        ProbeStatusJson::Fail { last_fail_reason: "HTTP 503".to_owned() },
        "the failure REASON must survive the projection — `last=fail` without the reason is \
         the difference between an actionable render and a shrug",
    );
    assert_eq!(liveness.probe_idx, 0, "per-role index survives the projection");

    // Replica 1's liveness probe has no row: absence IS pending per
    // ADR-0054 §5, and the handler must NOT synthesise one.
    assert!(
        !body
            .probe_results
            .iter()
            .any(|r| r.alloc_id == alloc_1.to_string() && r.role == ProbeRole::Liveness.as_str()),
        "a probe that has not ticked has NO row — the handler must not fabricate one; \
         row absence is what the renderer materialises as `pending`. Got: {:?}",
        body.probe_results,
    );
}

/// A Service whose probes have all ticked still carries BOTH halves —
/// the guard against a handler that projects declarations but silently
/// drops the observation read (or vice versa).
#[tokio::test]
async fn alloc_status_carries_both_probe_halves_together() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    install_service(&state).await;
    let workload_id = WorkloadId::new(SERVICE_ID).expect("valid workload id");
    let alloc_0 = alloc("alloc-payments-svc-0");
    write_running_alloc(&state, &alloc_0, &workload_id).await;
    write_probe_row(&state, &alloc_0, ProbeRole::Startup, 0, ProbeStatus::Pass, 1_000).await;

    let body = read_snapshot(&state, SERVICE_ID).await;

    assert!(!body.probes.is_empty(), "declaration side must be populated");
    assert!(!body.probe_results.is_empty(), "observation side must be populated");
    // Every observed row must correspond to an allocation in the same
    // response — an orphan row would mean the read was not filtered to
    // this workload.
    for row in &body.probe_results {
        assert!(
            body.rows.iter().any(|alloc_row| alloc_row.alloc_id == row.alloc_id),
            "probe row for alloc {} has no matching allocation row in the same snapshot; \
             the per-alloc read must be scoped to this workload's allocations",
            row.alloc_id,
        );
    }
}

// ---------------------------------------------------------------------------
// Kind guard — the negative
// ---------------------------------------------------------------------------

/// A Job-kind read carries NEITHER probe field.
///
/// `skip_serializing_if = "Vec::is_empty"` then keeps both off the JSON
/// entirely, so non-Service consumers see a byte-identical wire.
#[tokio::test]
async fn alloc_status_for_job_kind_carries_no_probe_surface() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state(&tmp);
    let job = install_job(&state).await;
    write_running_alloc(&state, &alloc("alloc-payments-job-0"), &job.id).await;

    let body = read_snapshot(&state, JOB_ID).await;

    assert!(
        body.probes.is_empty(),
        "a Job cannot declare probes — the parser rejects `[[health_check.*]]` on this kind \
         (`JOB_PROBES_GUIDANCE`); got {:?}",
        body.probes,
    );
    assert!(
        body.probe_results.is_empty(),
        "a Job has no probe surface to observe; got {:?}",
        body.probe_results,
    );

    let wire = serde_json::to_string(&body).expect("serialise AllocStatusResponse");
    assert!(
        !wire.contains("\"probes\""),
        "both probe fields are skip-if-empty, so a Job's JSON must OMIT them entirely (not \
         emit `[]`) — that is what keeps the wire backward-compatible for non-Service \
         consumers; got: {wire}",
    );
    assert!(!wire.contains("\"probe_results\""), "got: {wire}");
}
