//! `ReconcilerRuntime` ↔ `ViewStore` wiring per ADR-0035 §5
//! (reconciler-memory-redb step 01-06).
//!
//! Three scenarios pin the contract:
//!
//! 1. `runtime_refuses_to_start_when_probe_fails` (AC#5, AC#8) — a
//!    probe failure surfaces as `ControlPlaneError::Internal` with a
//!    structured cause and prevents any reconciler from being
//!    registered.
//! 2. `runtime_bulk_loads_views_at_register` (AC#9) — pre-populated
//!    `(target, view)` rows are visible to the first tick's
//!    reconciler call. Verified by emitting an action that depends on
//!    the loaded view's contents.
//! 3. `runtime_writes_through_before_in_memory_update` (AC#4) —
//!    `write_through` failure leaves the runtime's in-memory map
//!    unchanged. Verified by injecting an fsync failure on tick N+1
//!    and asserting the loaded view at tick N+2 still reflects the
//!    original value.
//! 4. `runtime_skips_write_through_when_next_view_equals_in_memory`
//!    (Eq-diff additive extension per ADR-0035 §1, May 2026) — when
//!    a reconciler returns a `next_view` that is `Eq`-equal to the
//!    in-memory `view` it was given, the runtime MUST skip both
//!    `ViewStore::write_through` and the in-memory map insert. The
//!    fsync-then-memory ordering for the non-equal case is
//!    independently pinned by scenario 3 above.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::view_store::{ViewStore, ViewStoreExt};
use overdrive_core::id::AllocationId;
use overdrive_core::reconciler::{
    AnyReconciler, JobLifecycle, JobLifecycleView, NoopHeartbeat, Reconciler, ReconcilerName,
    TargetResource,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_sim::adapters::view_store::SimViewStore;

fn name(s: &str) -> ReconcilerName {
    ReconcilerName::new(s).expect("valid reconciler name")
}

fn target(s: &str) -> TargetResource {
    TargetResource::new(s).expect("valid target resource")
}

fn alloc(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid alloc id")
}

/// AC#5 + AC#8: a probe failure short-circuits the runtime's
/// `register` call with a structured cause. The composition root in
/// `overdrive-cli` translates this into `health.startup.refused` +
/// non-zero exit; this test pins the trait-level surface the
/// composition root depends on.
#[tokio::test]
async fn runtime_refuses_to_start_when_probe_fails() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sim = Arc::new(SimViewStore::new());
    sim.inject_fsync_failure();

    let mut runtime = ReconcilerRuntime::new(tmp.path(), sim.clone() as Arc<dyn ViewStore>)
        .expect("constructor must succeed even when probe will fail later");
    let result = runtime.register(AnyReconciler::NoopHeartbeat(NoopHeartbeat::canonical())).await;

    let err = result.expect_err("register must propagate probe failure");
    assert!(
        matches!(
            err,
            ControlPlaneError::ViewStoreBoot(
                overdrive_control_plane::error::ViewStoreBootError::Probe { .. }
            )
        ),
        "probe failure must map to ControlPlaneError::ViewStoreBoot(Probe), got {err:?}"
    );
    let rendered = err.to_string();
    assert!(
        rendered.contains("probe") || rendered.contains("fsync"),
        "rendered cause must name the probe failure, got: {rendered}"
    );
}

/// AC#9: pre-populated `ViewStore` rows are bulk-loaded at register
/// and become visible to subsequent reconcile calls. Verified by
/// invoking the runtime's view lookup helper after registration.
#[tokio::test]
async fn runtime_bulk_loads_views_at_register() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sim = Arc::new(SimViewStore::new());

    let n = name("job-lifecycle");
    let target_a = target("job/payments");
    let target_b = target("job/frontend");

    let mut view_a = JobLifecycleView::default();
    view_a.restart_counts.insert(alloc("alloc-payments-0"), 3);
    view_a.last_failure_seen_at.insert(
        alloc("alloc-payments-0"),
        UnixInstant::from_unix_duration(Duration::from_secs(1234)),
    );
    let mut view_b = JobLifecycleView::default();
    view_b.restart_counts.insert(alloc("alloc-frontend-0"), 1);

    sim.write_through(<JobLifecycle as Reconciler>::NAME, &target_a, &view_a)
        .await
        .expect("seed view_a");
    sim.write_through(<JobLifecycle as Reconciler>::NAME, &target_b, &view_b)
        .await
        .expect("seed view_b");

    let mut runtime = ReconcilerRuntime::new(tmp.path(), sim.clone() as Arc<dyn ViewStore>)
        .expect("runtime constructor");
    runtime
        .register(AnyReconciler::JobLifecycle(JobLifecycle::canonical()))
        .await
        .expect("register job-lifecycle");

    // Inspect the runtime's in-memory map via the test-only accessor.
    let loaded = runtime
        .loaded_job_lifecycle_views_for_test(&n)
        .expect("job-lifecycle map must exist after register");
    assert_eq!(loaded.get(&target_a), Some(&view_a), "view_a must be bulk-loaded");
    assert_eq!(loaded.get(&target_b), Some(&view_b), "view_b must be bulk-loaded");
}

/// AC#4 (`WriteThroughOrdering`): when `write_through` fails (fsync
/// injection fires), the runtime's in-memory map MUST NOT be updated
/// with the would-be `next_view`. Verified by dispatching a tick that
/// would have updated the view, then asserting the map still carries
/// the pre-tick value.
#[tokio::test]
async fn runtime_writes_through_before_in_memory_update() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sim = Arc::new(SimViewStore::new());
    let n = name("job-lifecycle");
    let t = target("job/payments");

    let mut original = JobLifecycleView::default();
    original.restart_counts.insert(alloc("alloc-payments-0"), 7);
    sim.write_through(<JobLifecycle as Reconciler>::NAME, &t, &original).await.expect("seed");

    let mut runtime =
        ReconcilerRuntime::new(tmp.path(), sim.clone() as Arc<dyn ViewStore>).expect("runtime");
    runtime
        .register(AnyReconciler::JobLifecycle(JobLifecycle::canonical()))
        .await
        .expect("register");

    // Construct a `next_view` that differs from the original and try
    // to persist it through the runtime's `apply_next_view_for_test`
    // helper. With the fsync injection set, the call must fail and
    // the in-memory map must remain unchanged.
    let mut next_view = original.clone();
    next_view.restart_counts.insert(alloc("alloc-payments-0"), 99);

    sim.inject_fsync_failure();
    let result = runtime.apply_next_view_for_test(&n, &t, next_view.clone()).await;
    assert!(result.is_err(), "fsync-injected write_through must error, got {result:?}");
    sim.clear_fsync_failure();

    let after = runtime.loaded_job_lifecycle_views_for_test(&n).expect("map present");
    assert_eq!(
        after.get(&t),
        Some(&original),
        "in-memory map must NOT be updated when write_through fails"
    );

    // Sanity check — the underlying SimViewStore also still has the
    // original view (the write was rolled back).
    let from_store: BTreeMap<TargetResource, JobLifecycleView> =
        sim.bulk_load(<JobLifecycle as Reconciler>::NAME).await.expect("bulk_load");
    assert_eq!(
        from_store.get(&t),
        Some(&original),
        "store must NOT have persisted the failed write"
    );
    let _ = (Instant::now(), Duration::from_millis(0));
}

/// Eq-diff additive extension per ADR-0035 §1: when a reconciler
/// returns a `next_view` byte-equal (`PartialEq`) to the in-memory
/// `view` it was given, the runtime MUST skip both `write_through`
/// and the in-memory map insert. The fsync is the expensive operation
/// — eliminating it on no-op ticks is the whole point of the
/// extension.
///
/// Test shape:
/// 1. Seed the runtime's in-memory map with a known view V (via the
///    test-only `seed_job_lifecycle_view_for_test` helper, bypassing
///    the store on purpose so the assertion below is unambiguous).
/// 2. Reset the [`SimViewStore`]'s `write_through_count` to zero
///    (clears the probe-internal write from `register`).
/// 3. Call `apply_next_view_for_test` with V (i.e. `next_view == view`).
/// 4. Assert: `write_through_count` is still zero (no fsync happened).
/// 5. Assert: the in-memory map still carries V (unchanged).
///
/// Mutation testing: a missed `==` swap or dropped match arm in the
/// runtime gate would let the fsync fire on equal views, which this
/// test catches. The `WriteThroughOrdering` invariant (test 3 above)
/// continues to assert the fsync-then-memory ordering for the
/// not-equal case.
#[tokio::test]
async fn runtime_skips_write_through_when_next_view_equals_in_memory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sim = Arc::new(SimViewStore::new());
    let n = name("job-lifecycle");
    let t = target("job/payments");

    let mut runtime =
        ReconcilerRuntime::new(tmp.path(), sim.clone() as Arc<dyn ViewStore>).expect("runtime");
    runtime
        .register(AnyReconciler::JobLifecycle(JobLifecycle::canonical()))
        .await
        .expect("register");

    // Seed an in-memory view directly. Using the test-only seeder
    // (rather than `apply_next_view_for_test`) keeps this test
    // independent of the very gate it's about to assert on — the
    // seed path bypasses `persist_view` entirely.
    let mut seeded = JobLifecycleView::default();
    seeded.restart_counts.insert(alloc("alloc-payments-0"), 2);
    seeded.last_failure_seen_at.insert(
        alloc("alloc-payments-0"),
        UnixInstant::from_unix_duration(Duration::from_secs(42)),
    );
    runtime.seed_job_lifecycle_view_for_test(&t, seeded.clone());

    // Reset the counter — `register` calls `probe()` which itself
    // performs a write_through against the probe sentinel name, and
    // we don't want that bleeding into the assertion.
    sim.reset_write_through_count();
    assert_eq!(
        sim.write_through_count(),
        0,
        "counter must be zero after explicit reset (test setup invariant)"
    );

    // Drive the runtime's persist path with a `next_view` byte-equal
    // to the seeded in-memory view. The Eq-diff gate MUST elide the
    // fsync; the call still returns Ok.
    let result = runtime.apply_next_view_for_test(&n, &t, seeded.clone()).await;
    assert!(result.is_ok(), "Eq-diff skip must return Ok without persisting, got {result:?}");

    assert_eq!(
        sim.write_through_count(),
        0,
        "runtime MUST skip write_through when next_view == in-memory view; \
         observed {} fsync(s)",
        sim.write_through_count(),
    );

    // The in-memory map must still carry the seeded view — the gate
    // skips the in-memory insert too (when next_view == view, the
    // insert is by definition a no-op, but the gate avoids even
    // taking the lock).
    let after = runtime
        .loaded_job_lifecycle_views_for_test(&n)
        .expect("job-lifecycle map must exist after register");
    assert_eq!(
        after.get(&t),
        Some(&seeded),
        "in-memory map must still carry the seeded view after the no-op tick"
    );

    // Belt-and-braces: a *different* next_view DOES write through.
    // Pinning this in the same test prevents a regression where the
    // gate accidentally short-circuits on every call (e.g. always
    // returning Ok before the comparison fires).
    let mut changed = seeded.clone();
    changed.restart_counts.insert(alloc("alloc-payments-0"), 3);
    runtime
        .apply_next_view_for_test(&n, &t, changed.clone())
        .await
        .expect("changed view must persist");
    assert_eq!(
        sim.write_through_count(),
        1,
        "a non-equal next_view MUST write through exactly once; \
         observed {} fsync(s)",
        sim.write_through_count(),
    );
    let after2 = runtime.loaded_job_lifecycle_views_for_test(&n).expect("map present");
    assert_eq!(after2.get(&t), Some(&changed), "in-memory map must reflect the changed view");
}
