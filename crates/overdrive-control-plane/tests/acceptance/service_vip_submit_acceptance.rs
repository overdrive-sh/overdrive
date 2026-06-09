//! Acceptance tests for the Service-arm `submit_workload` /
//! `alloc_status` paths per service-vip-allocator step 02-03d.
//!
//! Six scenarios — S-VIP-01, S-VIP-02, S-VIP-03, S-VIP-04, S-VIP-08,
//! S-VIP-09 — pinning the Service-arm wire contract per ADR-0049
//! (amended 2026-05-15) + ADR-0050 + ADR-0051:
//!
//! * S-VIP-01 `submit_service_allocates_vip` — submit Service spec via
//!   the HTTP driving port; response body carries
//!   `vip = Some("10.96.x.y")` from the default `VipRange::default()`
//!   pool, not in the reserved set.
//! * S-VIP-02 `alloc_status_renders_same_vip` — after submit, the
//!   alloc-status read renders the SAME VIP echoed by submit; per-listener
//!   line rendering is `(port, protocol)` only — there is no per-listener
//!   VIP field.
//! * S-VIP-03 `multi_listener_one_vip` — two listeners on different
//!   `(port, protocol)` tuples share ONE Service-level VIP. Listener
//!   uniqueness is enforced by `ServiceV1::from_submit`; the allocator
//!   issues one VIP per `WorkloadIntent::Service(_).spec_digest()`.
//! * S-VIP-04 `idempotent_resubmit_same_vip` — resubmitting a byte-
//!   identical Service spec returns the SAME VIP. The allocator memo is
//!   keyed by content-addressed `spec_digest`; byte-identical inputs hash
//!   to the same digest by construction, and the persistent allocator
//!   short-circuits on memo hit.
//! * S-VIP-08 `pool_exhaustion_typed_rejection` — when the pool is
//!   exhausted, a fresh distinct Service submission returns HTTP 503 with
//!   a typed error naming the allocated / capacity counts.
//! * S-VIP-09 `pool_exhaustion_existing_unaffected` — after exhaustion
//!   rejection, prior allocations remain readable via the allocator's
//!   memo (round-trip through alloc-status).
//!
//! All six tests drive through the public `submit_workload` /
//! `alloc_status` HTTP handlers as their driving ports — port-to-port
//! per `.claude/rules/testing.md` § "Port-to-port at all levels". The
//! Service-arm code paths inside the handlers are the production
//! integration site; the tests would not flip from RED to GREEN if the
//! handler short-circuited around `allocator.allocate(...)`.

use std::str::FromStr;
use std::sync::Arc;

use axum::Json;
use axum::body::to_bytes;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;

use overdrive_control_plane::AppState;
use overdrive_control_plane::api::{
    AllocStatusResponse, IdempotencyOutcome, SubmitWorkloadRequest, SubmitWorkloadResponse,
};
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::handlers::{AllocStatusQuery, alloc_status, submit_workload};
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;

use overdrive_core::aggregate::{DriverInput, ExecInput, ResourcesInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;

use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

use ipnet::Ipv4Net;
use std::collections::BTreeSet;
use std::net::Ipv4Addr;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Build an `AppState` whose `allocator` carries the default
/// `VipRange::default()` pool (`10.96.0.0/16` reserved
/// `[.0, .1, .255.255]`).
fn build_app_state_default(tmp: &TempDir) -> AppState {
    build_app_state_with_allocator(tmp, |store| {
        Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
            VipRange::default(),
            store,
        )))
    })
}

/// Build an `AppState` whose `allocator` carries a 1-usable-address
/// `VipRange` — `10.97.0.0/30` reserved `[.0, .2, .3]`, leaving exactly
/// `10.97.0.1` allocatable. Used by S-VIP-08 / S-VIP-09 to trigger
/// pool exhaustion deterministically on the second allocation.
fn build_app_state_tiny(tmp: &TempDir) -> AppState {
    let range = Ipv4Net::new(Ipv4Addr::new(10, 97, 0, 0), 30).expect("/30 prefix is valid");
    let mut reserved = BTreeSet::new();
    reserved.insert(Ipv4Addr::new(10, 97, 0, 0));
    reserved.insert(Ipv4Addr::new(10, 97, 0, 2));
    reserved.insert(Ipv4Addr::new(10, 97, 0, 3));
    let vip_range = VipRange::new(vec![range], reserved)
        .expect("tiny test range satisfies VipRange invariants");
    build_app_state_with_allocator(tmp, move |store| {
        Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(vip_range, store)))
    })
}

fn build_app_state_with_allocator<F>(tmp: &TempDir, allocator_for: F) -> AppState
where
    F: FnOnce(Arc<dyn IntentStore>) -> Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
{
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::from_str("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator = allocator_for(Arc::clone(&store) as Arc<dyn IntentStore>);
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

/// Drive the content-negotiated `submit_workload` handler with no
/// `Accept` header and decode the JSON response. Same shape as the
/// existing Job-arm acceptance helpers.
async fn submit_json(
    state: AppState,
    request: SubmitWorkloadRequest,
) -> Result<SubmitWorkloadResponse, ControlPlaneError> {
    let response: Response = submit_workload(State(state), HeaderMap::new(), Json(request)).await?;
    let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("body to bytes");
    Ok(serde_json::from_slice(&bytes).expect("JSON SubmitWorkloadResponse"))
}

async fn fetch_alloc_status(state: AppState, job_id: &str) -> AllocStatusResponse {
    let query = AllocStatusQuery { job: Some(job_id.to_owned()) };
    let response = alloc_status(State(state), Query(query)).await.expect("alloc_status ok");
    response.0
}

fn parse_vip(s: &str) -> Ipv4Addr {
    s.parse::<Ipv4Addr>()
        .unwrap_or_else(|e| panic!("response.vip must be a valid IPv4 string; got {s:?}: {e}"))
}

// ---------------------------------------------------------------------------
// S-VIP-01 — Service submit allocates a VIP from the default pool.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn submit_service_allocates_vip() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state_default(&tmp);

    let response = submit_json(
        state,
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("payments", vec![(8080, "tcp")])),
        },
    )
    .await
    .expect("Service submit must succeed");

    assert_eq!(response.workload_id, "payments");
    assert_eq!(response.outcome, IdempotencyOutcome::Inserted);
    let vip_str = response.vip.as_ref().expect("Service submit response MUST carry vip = Some(_)");
    let vip = parse_vip(vip_str);

    // Within 10.96.0.0/16
    let default_net = Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 0), 16).unwrap();
    assert!(
        default_net.contains(&vip),
        "allocated VIP {vip} must be within the default range 10.96.0.0/16",
    );

    // Not in the reserved set.
    let reserved =
        [Ipv4Addr::new(10, 96, 0, 0), Ipv4Addr::new(10, 96, 0, 1), Ipv4Addr::new(10, 96, 255, 255)];
    assert!(
        !reserved.contains(&vip),
        "allocated VIP {vip} must not be in the reserved set {reserved:?}",
    );
}

// ---------------------------------------------------------------------------
// S-VIP-02 — alloc_status renders the SAME VIP echoed by submit.
//            Per-listener lines are (port, protocol) only — no per-listener VIP.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn alloc_status_renders_same_vip() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state_default(&tmp);

    let submit = submit_json(
        state.clone(),
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("paymentsvc", vec![(8080, "tcp")])),
        },
    )
    .await
    .expect("Service submit must succeed");
    let submit_vip = submit.vip.as_ref().expect("submit response carries vip").clone();

    let alloc = fetch_alloc_status(state, "paymentsvc").await;
    let alloc_vip = alloc
        .vip
        .as_ref()
        .expect("alloc_status response MUST carry vip for Service workload")
        .clone();
    assert_eq!(
        submit_vip, alloc_vip,
        "alloc_status VIP must equal submit-echoed VIP for the same workload",
    );
}

// ---------------------------------------------------------------------------
// S-VIP-03 — Multi-listener Service gets exactly ONE service-level VIP.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_listener_one_vip() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state_default(&tmp);

    let response = submit_json(
        state,
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec(
                "edgesvc",
                vec![(8080, "tcp"), (8443, "tcp")],
            )),
        },
    )
    .await
    .expect("multi-listener Service submit must succeed");

    // ONE Service-level VIP — present once in the top-level response field.
    let vip_str =
        response.vip.as_ref().expect("Service submit response carries vip for multi-listener spec");
    let _ = parse_vip(vip_str); // syntax check
}

// ---------------------------------------------------------------------------
// S-VIP-04 — Idempotent resubmit returns same VIP.
//            Content-addressed `spec_digest` is deterministic over canonical
//            bytes; the persistent allocator short-circuits on memo hit.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn idempotent_resubmit_same_vip() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state_default(&tmp);

    let spec = service_spec("idemp", vec![(9090, "tcp")]);

    let first = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec.clone()) },
    )
    .await
    .expect("first submit");
    assert_eq!(first.outcome, IdempotencyOutcome::Inserted);
    let vip_first = first.vip.as_ref().expect("first carries vip").clone();

    let second = submit_json(state, SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec) })
        .await
        .expect("resubmit");
    assert_eq!(
        second.outcome,
        IdempotencyOutcome::Unchanged,
        "byte-identical resubmit must report Unchanged",
    );
    let vip_second = second.vip.as_ref().expect("resubmit carries vip").clone();

    assert_eq!(
        vip_first, vip_second,
        "byte-identical resubmit must return the SAME VIP — content-addressed memo hit",
    );
}

// ---------------------------------------------------------------------------
// S-VIP-08 — Pool exhaustion returns HTTP 503 with typed error body.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_exhaustion_typed_rejection() {
    use axum::response::IntoResponse;
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state_tiny(&tmp);

    // First Service submit consumes the sole allocatable address.
    let first = submit_json(
        state.clone(),
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("svc-a", vec![(8000, "tcp")])),
        },
    )
    .await
    .expect("first submit must succeed against a single-address pool");
    assert!(first.vip.is_some(), "first submit allocated a VIP");

    // Second distinct submit must exhaust.
    let err = submit_json(
        state,
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("svc-b", vec![(8001, "tcp")])),
        },
    )
    .await
    .expect_err("second distinct Service submit must be rejected on pool exhaustion");

    // Render to HTTP response and assert on the status / typed body shape.
    let response = err.into_response();
    assert_eq!(
        response.status().as_u16(),
        503,
        "pool exhaustion must map to HTTP 503 Service Unavailable",
    );
    let body_bytes = to_bytes(response.into_body(), usize::MAX).await.expect("body");
    let body: serde_json::Value =
        serde_json::from_slice(&body_bytes).expect("error body must be valid JSON");
    let error_kind =
        body.get("error").and_then(|v| v.as_str()).expect("error body carries `error` field");
    assert_eq!(
        error_kind, "pool_exhausted",
        "503 body must carry error = \"pool_exhausted\" naming the failure class",
    );
    let message =
        body.get("message").and_then(|v| v.as_str()).expect("error body carries `message` field");
    // The body must surface the allocated-of-capacity ratio. The
    // production renderer uses the shape `allocated N of M`; the
    // assertion pins both the keyword `allocated` and the magnitude
    // `1` (the small-range fixture's capacity).
    assert!(
        message.contains("allocated"),
        "503 message must name the `allocated` count; got {message:?}",
    );
    assert!(
        message.contains('1'),
        "503 message must surface the capacity / allocated magnitudes; got {message:?}",
    );
}

// ---------------------------------------------------------------------------
// S-VIP-09 — After pool exhaustion, existing allocations remain readable.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_exhaustion_existing_unaffected() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state_tiny(&tmp);

    let first = submit_json(
        state.clone(),
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("svc-keep", vec![(8000, "tcp")])),
        },
    )
    .await
    .expect("first submit");
    let original_vip = first.vip.as_ref().expect("first vip").clone();

    // Distinct submit exhausts.
    let _ = submit_json(
        state.clone(),
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("svc-bounce", vec![(8001, "tcp")])),
        },
    )
    .await
    .expect_err("second submit must reject");

    // Existing allocation is unaffected — alloc_status renders the
    // same VIP as the original submit echoed.
    let alloc = fetch_alloc_status(state, "svc-keep").await;
    let alloc_vip =
        alloc.vip.as_ref().expect("alloc_status carries vip for surviving Service").clone();
    assert_eq!(
        original_vip, alloc_vip,
        "post-exhaustion read of an existing Service must still resolve to its original VIP",
    );
}

/// S-VIP-10 `conflict_releases_vip` — when a different Service spec is
/// submitted at an already-occupied `workload_id`, the handler returns
/// 409 Conflict. The VIP allocated for the rejected spec MUST be
/// released back to the pool — otherwise it leaks permanently (no
/// downstream `Action::ReleaseServiceVip` will ever fire because the
/// rejected spec never gets a persisted `WorkloadIntent`).
///
/// Regression test for: VIP allocation precedes idempotency check —
/// leak on 409 Conflict path.
#[tokio::test]
async fn conflict_releases_vip() {
    let tmp = TempDir::new().expect("tmpdir");
    let state = build_app_state_default(&tmp);

    // 1. First submit — allocates a VIP from the default pool.
    let first = submit_json(
        state.clone(),
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("clash", vec![(8080, "tcp")])),
        },
    )
    .await
    .expect("first submit");
    assert_eq!(first.outcome, IdempotencyOutcome::Inserted);
    assert!(first.vip.is_some(), "first submit must carry a VIP");

    // Confirm exactly one allocation in the memo.
    {
        let guard = state.allocator.lock().await;
        assert_eq!(guard.memo_len(), 1, "one allocation after first submit");
        drop(guard);
    }

    // 2. Submit a DIFFERENT spec at the same workload_id ("clash") —
    //    different listeners ⇒ different spec_digest ⇒ allocator
    //    issues a second VIP before put_if_absent detects the conflict.
    let conflict = submit_json(
        state.clone(),
        SubmitWorkloadRequest {
            spec: SubmitSpecInput::Service(service_spec("clash", vec![(9090, "tcp")])),
        },
    )
    .await;
    assert!(
        matches!(&conflict, Err(ControlPlaneError::Conflict { .. })),
        "different spec at occupied key must return Conflict; got {conflict:?}",
    );

    // 3. The allocator memo must hold exactly ONE entry — the
    //    original. If the rejected spec's VIP leaked, memo_len == 2.
    let guard = state.allocator.lock().await;
    assert_eq!(
        guard.memo_len(),
        1,
        "rejected 409 Conflict must NOT leak a VIP — allocator memo \
         must hold only the original allocation, not the rejected one",
    );
    drop(guard);
}
