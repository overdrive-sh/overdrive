//! Acceptance tests for the `FrontendAddrAllocator` WRITER seam —
//! deploy-time `assign(<job>)` at Service declaration + the empty-on-boot
//! converge-on-boot rebuild from declared-Service intent
//! (dial-by-name-responder step 01-05; ADR-0072 REV-3, GH #243).
//!
//! 01-04 built the `FrontendAddrAllocator` (`assign`/`release`/`snapshot`).
//! 01-03 built the `name_index` that READS the `<job> → F` binding. This step
//! adds the two production CALL SITES that WRITE the binding by invoking the
//! EXISTING `FrontendAddrAllocator::assign`:
//!
//!   * S-DBN-ASSIGN-01 — assign-on-declare: a Service submit through the
//!     `submit_workload` driving port binds `<job> → F` in the shared
//!     allocator; a Job-kind submit assigns NO frontend addr (Service-only
//!     guard, mirroring the VIP allocate).
//!   * S-DBN-ASSIGN-02 — idempotent across resubmit: a byte-identical
//!     resubmit does not consume a second addr nor change the binding; a
//!     CONFLICTING resubmit (different spec at the same key, 409) does NOT
//!     evict the existing `<job> → F` binding (release-on-conflict-ONLY
//!     discipline — and the frontend allocator is `<job>`-keyed + idempotent,
//!     so there is nothing to release).
//!   * S-DBN-ASSIGN-03 — converge-on-boot rebuild: the boot pass re-populates
//!     an EMPTY allocator from the currently-declared Service set (the
//!     declared-Service intent is the SSOT), and re-running it is idempotent
//!     (same F, no churn).
//!   * S-DBN-ASSIGN-04 — single-owner: the WRITER feeds the SAME instance the
//!     `name_index` reader reads (a clone shares the held map), so an answered
//!     F is byte-identical to the assigned F.
//!
//! All four drive through the production CALL SITES as their driving ports
//! (the `submit_workload` HTTP handler and the `rebuild_frontend_addrs_from_intent`
//! boot pass) — port-to-port per `.claude/rules/testing.md`. PORT-TO-PORT
//! litmus: deleting the Service-arm `assign` call flips ASSIGN-01 RED; deleting
//! the rebuild enumeration flips ASSIGN-03 RED. Default unit lane (in-process:
//! `LocalIntentStore` + `Sim*` adapters + the pure allocator — no
//! kernel/netns/socket).

use std::str::FromStr;
use std::sync::Arc;

use axum::Json;
use axum::body::to_bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;

use overdrive_control_plane::AppState;
use overdrive_control_plane::api::{SubmitWorkloadRequest, SubmitWorkloadResponse};
use overdrive_control_plane::dns_responder::boot_rebuild::rebuild_frontend_addrs_from_intent;
use overdrive_control_plane::dns_responder::frontend_addr_allocator::WORKLOAD_FRONTEND_BASE;
use overdrive_control_plane::dns_responder::name_index::NameIndex;
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::handlers::submit_workload;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;

use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, JobSpecInput, ResourcesInput, ServiceV1, WorkloadIntent,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::id::{MeshServiceName, NodeId};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;

use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixtures (mirror service_vip_submit_acceptance.rs)
// ---------------------------------------------------------------------------

/// Build an `AppState` against a fresh `LocalIntentStore` — the
/// `frontend_addr_allocator` field is default-constructed (empty) inside the
/// `AppState` constructor. Returns `(state, store, redb_path)` so the boot
/// rebuild tests can drive the same store the state carries.
fn build_app_state(tmp: &TempDir) -> (AppState, Arc<LocalIntentStore>, std::path::PathBuf) {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::from_str("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    let state = AppState::new(
        Arc::clone(&store),
        store_path.clone(),
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
    );
    (state, store, store_path)
}

fn service_spec(id: &str, listeners: Vec<(u16, &str)>) -> ServiceSpecInput {
    ServiceSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
        listeners: listeners
            .into_iter()
            .map(|(port, protocol)| ListenerInput { port, protocol: protocol.to_owned() })
            .collect(),
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

fn job_spec(id: &str) -> JobSpecInput {
    JobSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    }
}

/// Drive the content-negotiated `submit_workload` handler (no `Accept` header)
/// and decode the JSON response.
async fn submit_json(
    state: AppState,
    request: SubmitWorkloadRequest,
) -> Result<SubmitWorkloadResponse, ControlPlaneError> {
    let response: Response = submit_workload(State(state), HeaderMap::new(), Json(request)).await?;
    let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("body to bytes");
    Ok(serde_json::from_slice(&bytes).expect("JSON SubmitWorkloadResponse"))
}

/// The canonical `<job>` key the WRITER and READER share (OQ-1).
fn job_name(id: &str) -> MeshServiceName {
    MeshServiceName::new(&format!("{id}.{}", MeshServiceName::SUFFIX))
        .expect("test ids are valid single-label mesh names")
}

/// Seed the intent store with a declared Service (as a prior boot's
/// `submit_workload` left it): write the `WorkloadIntent::Service` envelope at
/// `IntentKey::for_workload(<id>)`. Does NOT touch the allocator — that is the
/// rebuild's job.
async fn seed_declared_service(store: &Arc<LocalIntentStore>, id: &str) {
    let service =
        ServiceV1::from_submit(service_spec(id, vec![(8080, "tcp")])).expect("valid service spec");
    let intent = WorkloadIntent::Service(service);
    let archived = intent.archive_for_store().expect("archive Service intent");
    let key = IntentKey::for_workload(&workload_id(id));
    store.put(key.as_bytes(), archived.as_ref()).await.expect("seed declared Service intent");
}

fn workload_id(id: &str) -> overdrive_core::id::WorkloadId {
    overdrive_core::id::WorkloadId::new(id).expect("valid workload id")
}

// ===========================================================================
// S-DBN-ASSIGN-01 — assign-on-declare (the WRITER seam).
// ===========================================================================

#[tokio::test]
async fn service_submit_assigns_frontend_addr_in_shared_allocator() {
    let tmp = TempDir::new().expect("tmpdir");
    let (state, _store, _path) = build_app_state(&tmp);
    let allocator = state.frontend_addr_allocator.clone();

    // No binding before the submit.
    assert!(
        allocator.snapshot().is_empty(),
        "the frontend allocator starts empty before any Service declaration",
    );

    let response = submit_json(
        state,
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("server", vec![(8080, "tcp")])),
        },
    )
    .await
    .expect("Service submit must succeed");
    assert_eq!(response.workload_id, "server");

    // After the submit returns Ok, the SHARED allocator binds `server -> F`,
    // where F ∈ 10.98.0.0/16. This is what flips RED if the Service-arm
    // `assign` call site is deleted.
    let binding = allocator.snapshot();
    let f = *binding
        .get(&job_name("server"))
        .expect("assign-on-declare must bind <job>=server -> F in the shared allocator");
    assert!(
        WORKLOAD_FRONTEND_BASE.contains(&f),
        "assigned frontend addr {f} must be within {WORKLOAD_FRONTEND_BASE}",
    );
    assert_eq!(
        f,
        std::net::Ipv4Addr::new(10, 98, 0, 1),
        "the first Service declaration binds the first usable frontend addr (network()+1)",
    );
}

#[tokio::test]
async fn job_submit_assigns_no_frontend_addr() {
    let tmp = TempDir::new().expect("tmpdir");
    let (state, _store, _path) = build_app_state(&tmp);
    let allocator = state.frontend_addr_allocator.clone();

    let response =
        submit_json(state, SubmitWorkloadRequest { spec: SubmitSpecInput::Job(job_spec("batch")) })
            .await
            .expect("Job submit must succeed");
    assert_eq!(response.workload_id, "batch");

    // A Job-kind submit assigns NO frontend addr — frontends are a
    // Service-name concern (mirrors the VIP allocate's Service-only guard).
    // This flips RED if the assign is moved outside the Service-only guard.
    assert!(
        allocator.snapshot().is_empty(),
        "a Job-kind submit must NOT consume a frontend address; the allocator stays empty",
    );
}

// ===========================================================================
// S-DBN-ASSIGN-02 — idempotent across resubmit + no eviction on conflict.
// ===========================================================================

#[tokio::test]
async fn byte_identical_resubmit_does_not_change_binding() {
    let tmp = TempDir::new().expect("tmpdir");
    let (state, _store, _path) = build_app_state(&tmp);
    let allocator = state.frontend_addr_allocator.clone();

    let spec = service_spec("idem", vec![(9090, "tcp")]);

    let first = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec.clone()) },
    )
    .await
    .expect("first submit");
    assert_eq!(first.outcome, overdrive_control_plane::api::IdempotencyOutcome::Inserted);
    let f_first = *allocator.snapshot().get(&job_name("idem")).expect("first submit binds F");

    // A byte-identical resubmit (the KeyExists idempotency path) must NOT
    // consume a second frontend addr and must NOT change the binding.
    let second = submit_json(state, SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec) })
        .await
        .expect("resubmit");
    assert_eq!(
        second.outcome,
        overdrive_control_plane::api::IdempotencyOutcome::Unchanged,
        "byte-identical resubmit reports Unchanged",
    );

    let after = allocator.snapshot();
    assert_eq!(after.len(), 1, "resubmit must NOT consume a second frontend address");
    assert_eq!(
        *after.get(&job_name("idem")).expect("binding survives resubmit"),
        f_first,
        "byte-identical resubmit leaves <job> -> F unchanged (idempotent per <job>)",
    );
}

#[tokio::test]
async fn conflicting_resubmit_does_not_evict_existing_binding() {
    let tmp = TempDir::new().expect("tmpdir");
    let (state, _store, _path) = build_app_state(&tmp);
    let allocator = state.frontend_addr_allocator.clone();

    // First submit binds clash -> F.
    let first = submit_json(
        state.clone(),
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("clash", vec![(8080, "tcp")])),
        },
    )
    .await
    .expect("first submit");
    assert_eq!(first.outcome, overdrive_control_plane::api::IdempotencyOutcome::Inserted);
    let f_original = *allocator.snapshot().get(&job_name("clash")).expect("first binds F");

    // A CONFLICTING resubmit — a different spec at the same workload_id —
    // returns 409 Conflict and MUST NOT evict the live workload's frontend F
    // (no eviction of the existing binding on a rejected resubmit; U6 / the
    // release-on-conflict-ONLY discipline).
    let conflict = submit_json(
        state,
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("clash", vec![(9090, "tcp")])),
        },
    )
    .await;
    assert!(
        matches!(&conflict, Err(ControlPlaneError::Conflict { .. })),
        "different spec at occupied key must return Conflict; got {conflict:?}",
    );

    let after = allocator.snapshot();
    assert_eq!(
        after.len(),
        1,
        "a rejected conflicting resubmit must NOT consume a second frontend addr",
    );
    assert_eq!(
        *after.get(&job_name("clash")).expect("existing binding survives the conflict"),
        f_original,
        "a rejected conflicting resubmit must NOT evict the existing <job> -> F binding",
    );
}

// ===========================================================================
// S-DBN-ASSIGN-03 — converge-on-boot rebuild from declared-Service intent.
// ===========================================================================

#[tokio::test]
async fn boot_rebuild_repopulates_empty_allocator_from_declared_services() {
    let tmp = TempDir::new().expect("tmpdir");
    let (state, store, path) = build_app_state(&tmp);
    let allocator = state.frontend_addr_allocator.clone();

    // GIVEN N declared Service intents already in the store (as a prior boot
    // left them) AND a FRESH (empty) allocator.
    seed_declared_service(&store, "alpha").await;
    seed_declared_service(&store, "bravo").await;
    seed_declared_service(&store, "charlie").await;
    assert!(allocator.snapshot().is_empty(), "the allocator is empty before the rebuild");

    // WHEN the boot rebuild pass runs (the driving port).
    rebuild_frontend_addrs_from_intent(&store, &path, &allocator)
        .await
        .expect("boot rebuild must succeed");

    // THEN the allocator binds a stable F for every declared <job>. This flips
    // RED if the rebuild skips a declared <job> (or never enumerates them).
    let after = allocator.snapshot();
    assert_eq!(after.len(), 3, "rebuild must bind every declared Service");
    for id in ["alpha", "bravo", "charlie"] {
        let f = *after.get(&job_name(id)).unwrap_or_else(|| {
            panic!("declared Service {id} must have a frontend addr after rebuild")
        });
        assert!(
            WORKLOAD_FRONTEND_BASE.contains(&f),
            "rebuilt frontend addr {f} for {id} must be within {WORKLOAD_FRONTEND_BASE}",
        );
    }

    // AND re-running the rebuild is idempotent — same F per <job>, no churn.
    rebuild_frontend_addrs_from_intent(&store, &path, &allocator)
        .await
        .expect("second rebuild must succeed");
    assert_eq!(
        allocator.snapshot(),
        after,
        "re-running the rebuild re-assigns each <job> to the SAME F (no churn)",
    );
}

// ===========================================================================
// S-DBN-ASSIGN-04 — single-owner: WRITER feeds the SAME instance the readers
// read (DDN-2).
// ===========================================================================

#[tokio::test]
async fn writer_feeds_the_same_allocator_instance_the_name_index_reads() {
    let tmp = TempDir::new().expect("tmpdir");
    let (state, _store, _path) = build_app_state(&tmp);

    // A single Arc-shared FrontendAddrAllocator (a clone shares the held map).
    let writer_allocator = state.frontend_addr_allocator.clone();
    let reader_allocator = state.frontend_addr_allocator.clone();

    // Build a name_index over the SAME instance (a different ObservationStore
    // is irrelevant — ASSIGN-04 asserts on the allocator binding the index
    // exposes, the single source of frontend truth).
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::from_str("reader").expect("NodeId"), 0));
    let _name_index = NameIndex::new(obs, reader_allocator.clone());

    // WHEN the writer (the Service submit) binds api -> F.
    let response = submit_json(
        state,
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("api", vec![(8080, "tcp")])),
        },
    )
    .await
    .expect("Service submit must succeed");
    assert_eq!(response.workload_id, "api");

    // THEN the reader_allocator (the SAME instance the name_index holds)
    // observes the SAME binding, byte-identical — no second <job> -> F source.
    let writer_f =
        *writer_allocator.snapshot().get(&job_name("api")).expect("writer binds api -> F");
    let reader_f = *reader_allocator
        .snapshot()
        .get(&job_name("api"))
        .expect("the reader's clone observes the writer's binding (single shared instance)");
    assert_eq!(
        writer_f, reader_f,
        "the answered F == the assigned F (single source of frontend truth, DDN-2)",
    );
}
