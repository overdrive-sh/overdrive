//! backend-instance-replacement — read-side generation coupling.
//!
//! Pins that the reconciler's desired-generation READ path
//! (`generation_value` → `hydrate_workload_lifecycle_desired`) reads the
//! SAME `workloads/<id>/generation` key the writer bumps, decoded through
//! the SAME `IncrementU64` read contract. This closes the seam left
//! untested by the sibling coverage:
//!
//!   * the WRITE side is pinned by `restart_workload_intent_key` — but it
//!     asserts on the RAW store (`get(for_workload_generation)`), never
//!     through the reconciler's read.
//!   * the generation DECISION logic is pinned by
//!     `runtime_convergence_loop` — but with hand-built
//!     `WorkloadLifecycleState { generation, observed_generation }`
//!     literals, never a real store read.
//!
//! Neither drives the reconciler's real READ of the generation key.
//! Today the read side and the writer produce byte-identical keys and
//! decode, so nothing distinguishes them — this test locks the coupling
//! so a future divergence on EITHER the key (drop
//! `IntentKey::for_workload_generation`) or the decode (drop
//! `TxnOp::decode_counter`) trips.
//!
//! # The discriminator
//!
//! The fresh-placement tick stamps `observed_generation =
//! desired.generation` ONLY when `restart_pending`
//! (`observed(0) < desired.generation`). So `observed_generation == 1`
//! after a tick can hold only if `generation_value` read the value the
//! writer bumped: a read-side drift on the key or the decode makes
//! `desired.generation` read `0`, `restart_pending` false, no stamp — and
//! the persisted view is `default()` (elided), so the assertion fails.
//!
//! # Port-to-port
//!
//! Driving port: `run_convergence_tick` (the production convergence tick
//! — hydrate desired reads the store through `generation_value`).
//! Observable: the persisted `WorkloadLifecycleView.observed_generation`
//! via the runtime's `loaded_workload_lifecycle_views_for_test` accessor.
//!
//! In-process, real redb over `TempDir` (Strategy C). Gated behind
//! `integration-tests` for the same reason as `runtime_convergence_loop`
//! / `service_workload_emits_start_allocation`: `run_convergence_tick`
//! and the `*_for_test` view accessor are
//! `#[cfg(any(test, feature = "integration-tests"))]`, and `cfg(test)`
//! does not propagate to the integration-test binary's view of the lib.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, noop_heartbeat, workload_lifecycle};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput, WorkloadIntent,
    WorkloadKind,
};
use overdrive_core::id::{NodeId, WorkloadId};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::{IntentStore, TxnOp, TxnOutcome};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

/// Build an `AppState` whose runtime carries both production reconcilers
/// (`noop-heartbeat` + `job-lifecycle`) — the `run_server` boot shape.
/// The `SimClock` is held by the caller to source the tick's `now`.
async fn build_state(tmp: &TempDir, clock: Arc<SimClock>) -> AppState {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(workload_lifecycle()).await.expect("register job-lifecycle");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
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

/// GIVEN a declared `payments` Job whose desired-run generation was bumped
/// to 1 through the PRODUCTION write contract (`TxnOp::IncrementU64` at
/// `IntentKey::for_workload_generation` — the exact op `overdrive workload
/// restart` commits) —
/// WHEN one `run_convergence_tick` fires for `job-lifecycle` against it —
/// THEN the persisted `WorkloadLifecycleView.observed_generation` is `1`.
///
/// See the module docstring for why `observed_generation == 1` is a true
/// coupling discriminator for the read path (`generation_value`).
#[tokio::test]
async fn reconciler_observes_generation_written_at_for_workload_generation_key() {
    let tmp = TempDir::new().expect("tempdir");
    let clock = Arc::new(SimClock::new());
    let state = build_state(&tmp, clock.clone()).await;

    // GIVEN a declared Job workload `payments` (aggregate + kind).
    let workload_id = WorkloadId::new("payments").expect("valid id");
    let job = Job::from_submit(JobSpecInput {
        id: "payments".to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    })
    .expect("valid job spec");
    let archived = WorkloadIntent::Job(job).archive_for_store().expect("rkyv archive");
    state
        .store
        .put(IntentKey::for_workload(&workload_id).as_bytes(), archived.as_ref())
        .await
        .expect("seed job aggregate");
    state
        .store
        .put(
            IntentKey::for_workload_kind(&workload_id).as_bytes(),
            &[WorkloadKind::Job.discriminator_byte()],
        )
        .await
        .expect("seed workload kind");

    // AND the desired-run generation bumped to 1 through the production
    // write contract — one atomic `IncrementU64` at the constructor key,
    // exactly as `restart_workload` commits it (absent 0 → 1).
    let gen_key = IntentKey::for_workload_generation(&workload_id);
    let outcome = state
        .store
        .txn(vec![TxnOp::IncrementU64 { key: Bytes::copy_from_slice(gen_key.as_bytes()) }])
        .await
        .expect("generation bump txn");
    assert!(
        matches!(outcome, TxnOutcome::Committed),
        "the IncrementU64 generation bump must commit; got {outcome:?}"
    );

    // WHEN one convergence tick fires for job-lifecycle against the workload.
    let name = ReconcilerName::new("job-lifecycle").expect("valid reconciler name");
    let target = TargetResource::new("job/payments").expect("valid target");
    let now = clock.now();
    let deadline = now + Duration::from_millis(100);
    run_convergence_tick(&state, &name, &target, now, 0, deadline)
        .await
        .expect("convergence tick succeeds");

    // THEN the reconciler observed generation 1 — the restart-pending
    // fresh-placement tick stamps `observed_generation = desired.generation`,
    // which proves `generation_value` read `workloads/payments/generation`
    // (the writer's key) and decoded the bumped value through the shared
    // `IncrementU64` read contract.
    let views = state
        .runtime
        .loaded_workload_lifecycle_views_for_test(&name)
        .expect("job-lifecycle view map present after register");
    let view = views.get(&target).expect(
        "a WorkloadLifecycleView must be persisted for the restart-pending fresh placement — \
         its absence means desired.generation read 0 (read-side drift), so restart_pending was \
         false and next_view stayed default()",
    );
    assert_eq!(
        view.observed_generation, 1,
        "the reconciler must observe generation 1 read from \
         `workloads/payments/generation` and stamp `observed_generation = 1`; got {} — a non-1 \
         value means the read side (`generation_value`) drifted from the writer's key \
         (`IntentKey::for_workload_generation`) or the `IncrementU64` decode \
         (`TxnOp::decode_counter`)",
        view.observed_generation,
    );
}
