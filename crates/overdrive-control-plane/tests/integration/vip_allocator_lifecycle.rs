//! Integration — service-vip-allocator step 03-03 — end-to-end VIP
//! lifecycle through the production driving ports.
//!
//! Owns the S-VIP-06 (end-to-end) and S-VIP-07 (released-VIP reuse)
//! scenarios from `docs/feature/service-vip-allocator/distill/test-
//! scenarios.md`. Both chain three production seams in a single test
//! run:
//!
//! 1. `submit_workload` HTTP handler (driving port) — admits a Service
//!    spec, drives the `PersistentServiceVipAllocator::allocate` path,
//!    and echoes the issued VIP in the response.
//! 2. `action_shim::dispatch` (driving port of the reconciler's emission
//!    contract) — consumes a hand-constructed
//!    `Action::ReleaseServiceVip { spec_digest, correlation }` carrying
//!    the same content-addressed digest the submit handler would derive,
//!    and invokes `PersistentServiceVipAllocator::release` under the
//!    shared `Arc<Mutex<_>>`.
//! 3. `submit_workload` re-entry — a second submit with a byte-different
//!    Service spec must observe the released VIP as available for
//!    reallocation. Under a single-address pool (`10.96.0.1/32`) this
//!    pins the reuse property mechanically: the second submit MUST
//!    return the same VIP the first one received.
//!
//! Per `.claude/rules/testing.md` § "Layout — integration tests live
//! under `tests/integration/`": this file lives under
//! `tests/integration/vip_allocator_lifecycle.rs` and is wired through
//! the `tests/integration.rs` entrypoint inside the inline `mod
//! integration { … }` block. The whole binary is gated behind the
//! `integration-tests` feature on the crate (see `Cargo.toml`).
//!
//! PORT-TO-PORT discipline (`.claude/rules/testing.md` § "Port-to-port
//! at all levels"): the only direct calls into the allocator are the
//! post-condition `get(...)` queries. Mutating calls
//! (`allocate`/`release`) flow through `submit_workload` and
//! `action_shim::dispatch` respectively — the same seams production
//! reaches them through. Deleting the release-arm wiring in either the
//! handler or the action shim must turn at least one assertion in this
//! file RED.
//!
//! Reconciler-tick coverage: the reconciler's emission contract for
//! `Action::ReleaseServiceVip` on a terminal-state observation has its
//! own acceptance test
//! (`crates/overdrive-core/tests/acceptance/workload_lifecycle_release_
//! service_vip.rs`, step 03-01). The action-shim dispatch contract has
//! its own acceptance test
//! (`crates/overdrive-control-plane/tests/acceptance/release_service_
//! vip_dispatch.rs`, step 03-02). This integration test owns the
//! *chained* assertion: submit → derived digest → hand-built action →
//! dispatch → release → reuse. The runtime tick that would join (1)
//! and (2) for a real Service-arm convergence loop is the
//! Service-arm hydration story tracked separately; the chained
//! property here is what S-VIP-06 / S-VIP-07 pin.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::to_bytes;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use ipnet::Ipv4Net;
use tempfile::TempDir;

use overdrive_control_plane::AppState;
use overdrive_control_plane::action_shim::dispatch;
use overdrive_control_plane::api::{
    AllocStatusResponse, IdempotencyOutcome, SubmitWorkloadRequest, SubmitWorkloadResponse,
};
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::handlers::{AllocStatusQuery, alloc_status, submit_workload};
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, ResourcesInput, ServiceV1, WorkloadIntent,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::id::{CorrelationKey, NodeId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;

use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;

// ---------------------------------------------------------------------------
// Fixture builders
// ---------------------------------------------------------------------------

/// Build an `AppState` whose `allocator` carries the supplied
/// `VipRange`. The allocator + state share a single tempdir-backed
/// `LocalIntentStore` so the durable allocator entries written under
/// `allocate(...)` survive into the `release(...)` round-trip.
///
/// Returns the `AppState` and a clone of the `Arc<Mutex<_>>` allocator
/// handle so the test can inject `Action::ReleaseServiceVip` through
/// `action_shim::dispatch` against the SAME allocator state the
/// `submit_workload` handler mutated. Sharing the handle is the
/// load-bearing detail — passing a freshly-constructed second allocator
/// here would silently uncouple the seams and the test would no longer
/// verify the lifecycle property.
fn build_state_with_range(
    tmp: &TempDir,
    vip_range: VipRange,
) -> (AppState, Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>) {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::from_str("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        vip_range,
        Arc::clone(&store) as Arc<dyn IntentStore>,
    )));
    let state = AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        NodeId::new("writer-1").expect("NodeId"),
        Arc::clone(&allocator),
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );
    (state, allocator)
}

/// Build a one-address `VipRange` — `10.96.0.1/32` with no reserved
/// addresses, capacity exactly 1. Used by S-VIP-07 and the
/// pool-exhaustion-and-recovery scenario to make the reuse / recovery
/// property mechanical: there is exactly one VIP, so any post-release
/// allocation MUST issue it.
fn one_address_vip_range() -> VipRange {
    let cidr = Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 1), 32).expect("/32 prefix is valid");
    VipRange::new(vec![cidr], BTreeSet::new()).expect("single-address pool satisfies invariants")
}

/// Compose a `ServiceSpecInput` with a single `(port, "tcp")` listener
/// and an `exec` driver. The `id` field is the operator-visible
/// workload identity; pick distinct ids per submit to keep the
/// `IntentStore` admission paths unambiguous.
fn service_spec(id: &str, port: u16) -> ServiceSpecInput {
    ServiceSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
        listeners: vec![ListenerInput { port, protocol: "tcp".to_owned() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

/// Compose a `ServiceSpecInput` with a single `(port, protocol)`
/// listener — used by the listener-projection test to drive a UDP
/// listener end-to-end through the production submit handler.
fn service_spec_proto(id: &str, port: u16, protocol: &str) -> ServiceSpecInput {
    ServiceSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 134_217_728 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
        listeners: vec![ListenerInput { port, protocol: protocol.to_owned() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

/// Drive the content-negotiated `submit_workload` handler with no
/// `Accept` header and decode the JSON response. Mirrors the shape used
/// by `tests/acceptance/service_vip_submit_acceptance.rs`.
async fn submit_json(
    state: AppState,
    request: SubmitWorkloadRequest,
) -> Result<SubmitWorkloadResponse, ControlPlaneError> {
    let response: Response = submit_workload(State(state), HeaderMap::new(), Json(request)).await?;
    let bytes = to_bytes(response.into_body(), usize::MAX).await.expect("body to bytes");
    Ok(serde_json::from_slice(&bytes).expect("JSON SubmitWorkloadResponse"))
}

async fn fetch_alloc_status(state: AppState, workload_id: &str) -> AllocStatusResponse {
    let query = AllocStatusQuery { job: Some(workload_id.to_owned()) };
    let response = alloc_status(State(state), Query(query)).await.expect("alloc_status ok");
    response.0
}

/// Derive the `spec_digest` of the Service spec the way the production
/// `submit_workload` handler does — wrap into `WorkloadIntent::Service`
/// via `ServiceV1::from_submit` and call `spec_digest()`. This is the
/// SAME digest the handler hands to `allocator.allocate(...)`, so the
/// hand-constructed `Action::ReleaseServiceVip` carries the digest the
/// allocator's memo is actually keyed by — the integration would pass
/// trivially against a bogus digest if this projection drifted, hence
/// the careful mirror of the handler's path.
fn digest_for_spec(spec: ServiceSpecInput) -> [u8; 32] {
    let service = ServiceV1::from_submit(spec).expect("Service spec must validate");
    let intent = WorkloadIntent::Service(service);
    let hash = intent.spec_digest().expect("spec_digest of WorkloadIntent");
    *hash.as_bytes()
}

/// Drive a `Action::ReleaseServiceVip` through the production
/// `action_shim::dispatch` against the supplied shared allocator. The
/// non-allocator ports (`driver`, `obs`, `dataplane`, broadcast bus) are
/// untouched by the `ReleaseServiceVip` arm — the same shape the focused
/// acceptance test in `release_service_vip_dispatch.rs` uses, so the
/// end-to-end integration here exercises EXACTLY the production
/// dispatch arm.
async fn dispatch_release(
    allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
    workload_id: &str,
    digest_bytes: [u8; 32],
) {
    let digest = overdrive_core::id::ContentHash::from_bytes(digest_bytes);
    let target = format!("job-lifecycle/{workload_id}");
    let correlation = CorrelationKey::derive(&target, &digest, "release-service-vip");
    let action = Action::ReleaseServiceVip { spec_digest: digest, correlation };

    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
    // The dispatch path's Driver port is not touched by the
    // ReleaseServiceVip arm — a SimDriver is sufficient.
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let writer_node = NodeId::new("writer-1").expect("NodeId");

    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    let test_broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    dispatch(
        vec![action],
        driver.as_ref(),
        obs.as_ref(),
        dataplane.as_ref(),
        &lifecycle_tx,
        &tick,
        &writer_node,
        Arc::clone(&allocator),
        &test_broker,
    )
    .await
    .expect("dispatch must succeed");
}

// ---------------------------------------------------------------------------
// S-VIP-06 — terminal-state release chains through submit + dispatch
// ---------------------------------------------------------------------------

/// S-VIP-06 (end-to-end). Submits a Service spec through the production
/// `submit_workload` handler, captures the issued VIP, dispatches a
/// hand-constructed `Action::ReleaseServiceVip` (the same action the
/// runtime tick would produce for a terminal observation, per step
/// 03-01 + 03-02), and asserts the allocator no longer carries the
/// digest.
///
/// The load-bearing post-condition is `allocator.get(&digest) == None`
/// AFTER the dispatch — if the action shim's release arm regressed (e.g.
/// the `allocator.release(...)` call was dropped), the memo would still
/// carry the entry and this assertion would fail. The seam under audit
/// is the same one `release_service_vip_dispatch.rs` exercises in
/// isolation; this test adds the upstream `submit_workload` seam so the
/// digest comes from the handler's content-addressed derivation rather
/// than a hand-rolled fixture.
#[tokio::test]
async fn terminal_state_releases_vip_end_to_end() {
    let tmp = TempDir::new().expect("tempdir");
    let (state, allocator) = build_state_with_range(&tmp, VipRange::default());

    // ---- (1) Submit Service A through the production driving port.
    let spec = service_spec("payments", 8080);
    let submit_response = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec.clone()) },
    )
    .await
    .expect("Service submit must succeed against the default pool");
    assert_eq!(submit_response.outcome, IdempotencyOutcome::Inserted);
    let issued_vip =
        submit_response.vip.as_ref().expect("Service submit echoes the allocated VIP").clone();

    // ---- (2) Confirm the allocator carries the digest BEFORE release.
    //         This is the pre-condition — failure here would mean the
    //         submit-handler path itself regressed and the test would be
    //         exercising the wrong invariant downstream.
    let digest_bytes = digest_for_spec(spec);
    {
        let guard = allocator.lock().await;
        assert!(
            guard.get(&digest_bytes).is_some(),
            "pre-condition: allocator carries the digest issued by submit_workload"
        );
        drop(guard);
    }

    // ---- (3) Dispatch the Action::ReleaseServiceVip — the same action
    //         the reconciler tick would emit on a terminal-state
    //         observation (per step 03-01). The dispatch arm under audit
    //         is the production seam from step 03-02.
    dispatch_release(Arc::clone(&allocator), "payments", digest_bytes).await;

    // ---- (4) Post-condition: the allocator has released the digest.
    {
        let guard = allocator.lock().await;
        assert!(
            guard.get(&digest_bytes).is_none(),
            "S-VIP-06: allocator.get(&digest) MUST return None after \
             Action::ReleaseServiceVip dispatch (end-to-end chained from \
             submit_workload's content-addressed digest derivation). \
             issued VIP was {issued_vip}",
        );
        drop(guard);
    }
}

// ---------------------------------------------------------------------------
// O03 sub-claim 3 — the alloc-status handler projects the persisted
// Service intent's listeners (port + Proto) into `AllocStatusResponse.
// listeners`, so a UDP listener's `Proto::Udp` is observable on the read
// surface.
// ---------------------------------------------------------------------------

/// Submit a Service with a single UDP listener on port 5353 through the
/// production `submit_workload` handler, then read it back via the
/// `alloc_status` handler. The response's `listeners` field MUST carry
/// the `(5353, Proto::Udp)` listener projected from the persisted
/// `WorkloadIntent::Service` aggregate — NOT a synthesised value.
///
/// This is the handler-side half of O03 sub-claim 3: the CLI render
/// test (`overdrive-cli` `alloc_status.rs`) proves the section renders
/// `5353/udp`; this test proves the handler actually threads the
/// listener protocol from the intent onto the wire response.
#[tokio::test]
async fn alloc_status_projects_service_udp_listener_protocol() {
    use overdrive_core::dataplane::Proto;

    let tmp = TempDir::new().expect("tempdir");
    let (state, _allocator) = build_state_with_range(&tmp, VipRange::default());

    let spec = service_spec_proto("dns-resolver", 5353, "udp");
    let resp =
        submit_json(state.clone(), SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec) })
            .await
            .expect("Service submit must succeed");
    assert_eq!(resp.outcome, IdempotencyOutcome::Inserted);

    let status = fetch_alloc_status(state, "dns-resolver").await;

    assert_eq!(
        status.listeners.len(),
        1,
        "alloc_status must project exactly one listener for the single-listener Service; \
         got {:?}",
        status.listeners,
    );
    let listener = &status.listeners[0];
    assert_eq!(listener.port.get(), 5353, "projected listener port must be 5353");
    assert_eq!(
        listener.protocol,
        Proto::Udp,
        "projected listener protocol must be Proto::Udp (UDP service observable black-box); \
         got {:?}",
        listener.protocol,
    );
}

// ---------------------------------------------------------------------------
// S-VIP-07 — released VIP is reusable on next allocation
// ---------------------------------------------------------------------------

/// S-VIP-07. Pool of exactly 1 address (`10.96.0.1/32`, zero reserved).
/// Submit Service A → assert VIP `10.96.0.1`. Release through the
/// action shim. Submit a byte-different Service B → assert VIP
/// `10.96.0.1` again. The single-address pool makes the reuse property
/// mechanical: there is no other address the second submit could
/// possibly receive, so the assertion would fail iff the release path
/// did not actually return the VIP to the pool.
///
/// This is the integration counterpart to the focused
/// `service_vip_properties.rs::release_then_reallocate_reuses_address`
/// property test — that one verifies the allocator's `release` /
/// `allocate` contract in isolation; this one verifies the same
/// invariant END-TO-END through the production driving ports.
#[tokio::test]
async fn released_vip_reusable_on_next_allocation() {
    let tmp = TempDir::new().expect("tempdir");
    let (state, allocator) = build_state_with_range(&tmp, one_address_vip_range());

    // ---- Submit A — consume the sole VIP.
    let spec_a = service_spec("svc-a", 8001);
    let resp_a = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec_a.clone()) },
    )
    .await
    .expect("Service A submit must succeed against the 1-address pool");
    let vip_a = resp_a.vip.as_ref().expect("Service A response carries vip").clone();
    assert_eq!(
        vip_a, "10.96.0.1",
        "single-address pool 10.96.0.1/32 must issue 10.96.0.1; got {vip_a}",
    );

    // ---- Pre-release: alloc_status round-trips the same VIP for A.
    let status_a = fetch_alloc_status(state.clone(), "svc-a").await;
    let status_vip_a = status_a.vip.as_ref().expect("alloc_status VIP").clone();
    assert_eq!(status_vip_a, vip_a, "alloc_status VIP must match submit-echoed VIP pre-release");

    // ---- Release A via action-shim dispatch.
    let digest_a = digest_for_spec(spec_a);
    dispatch_release(Arc::clone(&allocator), "svc-a", digest_a).await;

    // ---- Submit B — byte-different spec (distinct id + distinct port).
    //      The digest derivation is content-addressed, so a different id
    //      OR a different port produces a different hash; either way
    //      the allocator's memo cannot short-circuit.
    let spec_b = service_spec("svc-b", 8002);
    let digest_b = digest_for_spec(spec_b.clone());
    assert_ne!(
        digest_b, digest_a,
        "S-VIP-07 fixture invariant: byte-different specs must produce \
         distinct content-addressed digests, else the allocator would \
         memo-hit and the reuse property would be untested",
    );

    let resp_b =
        submit_json(state, SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec_b) })
            .await
            .expect("Service B submit must succeed against the now-empty pool");
    let vip_b = resp_b.vip.as_ref().expect("Service B response carries vip").clone();
    assert_eq!(
        vip_b, "10.96.0.1",
        "S-VIP-07: byte-different Service B MUST receive the previously \
         released VIP 10.96.0.1 (single-address pool — there is no other \
         address available). vip_a was {vip_a}, vip_b is {vip_b}",
    );
}

// ---------------------------------------------------------------------------
// vip_allocator_pool_exhaustion_and_recovery — recovery property
// ---------------------------------------------------------------------------

/// Pool of exactly 1 address. Submit A → succeeds. Submit B → fails
/// with pool exhaustion (HTTP 503 surface verified by
/// `service_vip_submit_acceptance::pool_exhaustion_typed_rejection` in
/// isolation; this test asserts on the typed error directly). Release
/// A via action-shim dispatch. Re-submit B → succeeds with the
/// previously-allocated VIP.
///
/// The recovery property — "the pool genuinely returns the address to
/// availability, NOT just clears the memo for the released key" — is
/// what this test pins. A regression where `release` cleared the memo
/// but failed to return the address to the pool would let the first
/// re-submit of B still fail with exhaustion; the post-recovery submit
/// here would fire that regression.
#[tokio::test]
async fn vip_allocator_pool_exhaustion_and_recovery() {
    let tmp = TempDir::new().expect("tempdir");
    let (state, allocator) = build_state_with_range(&tmp, one_address_vip_range());

    let spec_a = service_spec("svc-keep", 9000);
    let resp_a = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec_a.clone()) },
    )
    .await
    .expect("first submit consumes the sole address");
    let vip_a = resp_a.vip.as_ref().expect("first response carries vip").clone();
    assert_eq!(vip_a, "10.96.0.1");

    let spec_b = service_spec("svc-second", 9001);
    let err = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec_b.clone()) },
    )
    .await
    .expect_err("second distinct submit must reject on pool exhaustion");
    // The typed-error shape detail (503, error body) is pinned by the
    // existing acceptance test; here we only require the error
    // discriminator matches the exhaustion class. The shape is
    // ControlPlaneError::VipAllocator(PersistentAllocatorError::Allocator(
    //   ServiceVipAllocatorError::Exhausted { .. })) per the
    // pass-through embedding chain defined in
    // crates/overdrive-control-plane/src/error.rs.
    match &err {
        ControlPlaneError::VipAllocator(
            overdrive_dataplane::allocators::PersistentAllocatorError::Allocator(
                overdrive_dataplane::allocators::ServiceVipAllocatorError::Exhausted { .. },
            ),
        ) => {}
        other => panic!(
            "expected ControlPlaneError::VipAllocator(.. Exhausted ..) on second \
             submit; got {other:?}"
        ),
    }

    // Release A through the production action-shim dispatch arm.
    let digest_a = digest_for_spec(spec_a);
    dispatch_release(Arc::clone(&allocator), "svc-keep", digest_a).await;

    // Re-submit B — must succeed and receive the recovered VIP.
    let resp_b =
        submit_json(state, SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec_b) })
            .await
            .expect("post-recovery submit must succeed against the now-empty pool");
    let vip_b = resp_b.vip.as_ref().expect("recovered submit echoes vip").clone();
    assert_eq!(
        vip_b, "10.96.0.1",
        "recovery: after action-shim release, the next submit must \
         receive the previously-allocated VIP (the pool is genuinely \
         recovered, not just memo-cleared)",
    );
}

/// Like [`build_state_with_range`] but also registers the
/// `workload-lifecycle` reconciler so `run_convergence_tick` can
/// dispatch against it.
async fn build_state_with_range_and_reconciler(
    tmp: &TempDir,
    vip_range: VipRange,
) -> (AppState, Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>) {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    runtime
        .register(overdrive_control_plane::noop_heartbeat())
        .await
        .expect("register noop-heartbeat");
    runtime
        .register(overdrive_control_plane::workload_lifecycle())
        .await
        .expect("register job-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::from_str("local").expect("NodeId"), 0));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        vip_range,
        Arc::clone(&store) as Arc<dyn IntentStore>,
    )));
    let state = AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        NodeId::new("writer-1").expect("NodeId"),
        Arc::clone(&allocator),
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );
    (state, allocator)
}

/// Regression: `hydrate_desired` must populate `service_spec_digest`
/// from the persisted `WorkloadIntent` so the reconciler's
/// `service_vip_release_emission` gate fires on the production
/// `run_convergence_tick` path — not only in unit tests that construct
/// `WorkloadLifecycleState` directly.
///
/// Before the fix, `hydrate_desired` hardcoded `service_spec_digest =
/// None`, which short-circuited the release gate (condition 2 in
/// `service_vip_release_emission`: `desired.service_spec_digest?`).
/// Every VIP allocated at submit time was permanently held in the
/// allocator memo regardless of terminal-state observations.
#[tokio::test]
async fn convergence_tick_releases_vip_on_terminal_service() {
    use overdrive_control_plane::reconciler_runtime::run_convergence_tick;

    let tmp = TempDir::new().expect("tempdir");
    let (state, allocator) = build_state_with_range_and_reconciler(&tmp, VipRange::default()).await;

    // ---- (1) Submit a Service workload through the production handler.
    let spec = service_spec("svc-release", 8080);
    let resp = submit_json(
        state.clone(),
        SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec.clone()) },
    )
    .await
    .expect("Service submit must succeed");
    assert_eq!(resp.outcome, IdempotencyOutcome::Inserted);

    // ---- (2) Pre-condition: the allocator carries the digest.
    let digest_bytes = digest_for_spec(spec);
    {
        let guard = allocator.lock().await;
        assert!(
            guard.get(&digest_bytes).is_some(),
            "pre-condition: allocator must carry the digest after submit"
        );
        drop(guard);
    }

    // ---- (3) Write a terminal alloc-status observation row.
    let workload_id = overdrive_core::id::WorkloadId::new("svc-release").expect("WorkloadId");
    let writer = NodeId::new("local").expect("NodeId");
    let terminal_row = overdrive_core::traits::observation_store::AllocStatusRow {
        alloc_id: overdrive_core::id::AllocationId::new("alloc-svc-release-0")
            .expect("AllocationId"),
        workload_id: workload_id.clone(),
        node_id: writer.clone(),
        state: overdrive_core::traits::observation_store::AllocState::Terminated,
        updated_at: overdrive_core::traits::observation_store::LogicalTimestamp {
            counter: 1,
            writer,
        },
        reason: Some(overdrive_core::transition_reason::TransitionReason::Stopped {
            by: overdrive_core::transition_reason::StoppedBy::Operator,
        }),
        detail: None,
        terminal: Some(overdrive_core::transition_reason::TerminalCondition::Stopped {
            by: overdrive_core::transition_reason::StoppedBy::Operator,
        }),
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
        listeners: vec![],
        // GAP-1 subsidiary: Terminated was Running first.
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
    };
    state
        .obs
        .write(overdrive_core::traits::observation_store::ObservationRow::AllocStatus(Box::new(
            terminal_row,
        )))
        .await
        .expect("write terminal observation row");

    // ---- (4) Write a stop intent so desired_to_stop fires.
    let stop_key = overdrive_core::aggregate::IntentKey::for_workload_stop(&workload_id);
    state.store.put(stop_key.as_bytes(), &[1u8]).await.expect("put stop intent");

    // ---- (5) Drive convergence ticks — should emit ReleaseServiceVip.
    let reconciler_name =
        overdrive_core::reconcilers::ReconcilerName::new("job-lifecycle").expect("reconciler name");
    let target_resource =
        overdrive_core::reconcilers::TargetResource::new(&format!("job/{workload_id}"))
            .expect("valid target");
    let now = Instant::now();
    let deadline = now + Duration::from_secs(60);
    for tick_n in 0..10_u64 {
        run_convergence_tick(
            &state,
            &reconciler_name,
            &target_resource,
            now + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("convergence tick");
    }

    // ---- (6) Post-condition: the allocator MUST have released the digest.
    {
        let guard = allocator.lock().await;
        assert!(
            guard.get(&digest_bytes).is_none(),
            "regression: convergence tick must release the VIP on a terminal Service workload"
        );
        drop(guard);
    }
}
