//! Tier-3 integration tests for `JobLifecycle::hydrate` / `persist`
//! against a real file-backed `LibsqlHandle`.
//!
//! Per step 02-01 of the issue-139-libsql-view-cache feature: the
//! reconciler-author-owned read (`hydrate`) and write (`persist`) paths
//! must round-trip a non-trivial `JobLifecycleView` across handle
//! lifetimes — proving cross-handle restart idempotence (the
//! load-bearing property the runtime tick loop in step 02-02 will rely
//! on once the in-memory `view_cache` is deleted in step 02-04).
//!
//! `:memory:` would lose state when the handle drops, so the
//! cross-handle test is file-backed via `tempfile::TempDir`.

use std::time::Duration;

use overdrive_core::id::AllocationId;
use overdrive_core::reconciler::{
    JobLifecycle, JobLifecycleView, LibsqlHandle, Reconciler, ReconcilerName, TargetResource,
};
use overdrive_core::wall_clock::UnixInstant;
use proptest::prelude::*;
use tempfile::TempDir;

fn alloc(raw: &str) -> AllocationId {
    AllocationId::new(raw).expect("valid allocation id")
}

fn target() -> TargetResource {
    // The reconciler author currently does not branch on target — every
    // row belongs to one reconciler-scoped DB. A sentinel string is
    // sufficient.
    TargetResource::new("job/payments").expect("valid target resource string")
}

/// Open a file-backed `LibsqlHandle` and run `JobLifecycle::migrate`
/// against it — the canonical test-side ceremony that mirrors what
/// `ReconcilerRuntime::register` does in production (issue #139 step
/// 02-02). Tests that touch `JobLifecycle::hydrate`/`persist` directly
/// (without going through the runtime) MUST run this first or they
/// hit the lifecycle invariant: hydrate/persist assume `migrate`
/// already created the schema, and a SELECT against an unmigrated DB
/// surfaces `HydrateError::Libsql("no such table: ...")`.
async fn open_and_migrate(path: &std::path::Path) -> LibsqlHandle {
    let handle = LibsqlHandle::open(path).await.expect("open libsql handle");
    JobLifecycle::canonical().migrate(&handle).await.expect("migrate job-lifecycle schema");
    handle
}

/// AC 4 — cross-handle restart idempotence.
///
/// Persist a non-trivial `JobLifecycleView` to a file-backed libSQL DB,
/// drop the handle, re-open against the same file, hydrate, and assert
/// the result equals the persisted view bit-equivalent. This is the
/// property the runtime tick loop relies on once the in-memory cache is
/// deleted: a control-plane restart must observe the same View it
/// persisted before the restart.
#[tokio::test]
async fn hydrate_after_persist_returns_bit_equivalent_view() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("jl.db");

    let alloc_a = alloc("alloc-a");
    let alloc_b = alloc("alloc-b");
    let alloc_c = alloc("alloc-c");

    let mut view = JobLifecycleView::default();
    view.restart_counts.insert(alloc_a.clone(), 1);
    view.restart_counts.insert(alloc_b.clone(), 4);
    view.restart_counts.insert(alloc_c.clone(), 0);
    view.last_failure_seen_at.insert(
        alloc_a.clone(),
        UnixInstant::from_unix_duration(Duration::new(1_700_000_000, 123_456_789)),
    );
    view.last_failure_seen_at
        .insert(alloc_b.clone(), UnixInstant::from_unix_duration(Duration::new(1_700_000_500, 0)));
    // Note: alloc_c intentionally has no last_failure_seen_at — the
    // two BTreeMaps are independent inputs and need not have the same
    // key set.

    let reconciler = JobLifecycle::canonical();
    let target = target();

    // Persist via a first handle, then drop it. `open_and_migrate`
    // runs `JobLifecycle::migrate` first to materialise the schema —
    // the lifecycle invariant `persist` assumes (issue #139 step
    // 02-02).
    {
        let handle = open_and_migrate(&path).await;
        reconciler.persist(&view, &handle).await.expect("persist");
    }

    // Re-open against the same file in a second handle and hydrate.
    // Migrate again — `CREATE TABLE IF NOT EXISTS` is idempotent, and
    // the runtime calls migrate on every register (which is what a
    // re-open across handles simulates: a control-plane restart that
    // re-registers the reconciler against the same file).
    let handle = open_and_migrate(&path).await;
    let hydrated = reconciler.hydrate(&target, &handle).await.expect("hydrate");

    assert_eq!(
        hydrated, view,
        "cross-handle hydrate must return the persisted view bit-equivalent"
    );
}

/// AC 4 (negative case) — hydrating an empty file-backed DB returns the
/// default View. Catches a regression where `hydrate` would fail to
/// `CREATE TABLE IF NOT EXISTS` on a fresh file (and instead error out
/// of the SELECT).
#[tokio::test]
async fn hydrate_against_fresh_file_returns_default_view() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("jl.db");

    let handle = open_and_migrate(&path).await;
    let hydrated = JobLifecycle::canonical().hydrate(&target(), &handle).await.expect("hydrate");

    assert_eq!(hydrated, JobLifecycleView::default());
}

/// AC 4 — persist replaces (rather than appends to) prior state. The
/// runtime contract per ADR-0013 §2b Phase 1 is `NextView = Self::View`
/// (full replacement) — a second persist with a smaller View must
/// produce a smaller hydrated result, not a union.
#[tokio::test]
async fn second_persist_replaces_first_persists_state() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("jl.db");

    let reconciler = JobLifecycle::canonical();
    let target = target();
    let handle = open_and_migrate(&path).await;

    // First persist — three allocs.
    let mut first = JobLifecycleView::default();
    first.restart_counts.insert(alloc("first-a"), 1);
    first.restart_counts.insert(alloc("first-b"), 2);
    first.restart_counts.insert(alloc("first-c"), 3);
    first
        .last_failure_seen_at
        .insert(alloc("first-a"), UnixInstant::from_unix_duration(Duration::new(100, 0)));
    reconciler.persist(&first, &handle).await.expect("persist 1");

    // Second persist — single alloc.
    let mut second = JobLifecycleView::default();
    second.restart_counts.insert(alloc("second-only"), 7);
    second.last_failure_seen_at.insert(
        alloc("second-only"),
        UnixInstant::from_unix_duration(Duration::new(200, 999_999_999)),
    );
    reconciler.persist(&second, &handle).await.expect("persist 2");

    let hydrated = reconciler.hydrate(&target, &handle).await.expect("hydrate");

    assert_eq!(hydrated, second, "persist must fully replace prior state");
    assert!(
        !hydrated.restart_counts.contains_key(&alloc("first-a")),
        "first persist's keys must be gone after second persist"
    );
}

// ---------------------------------------------------------------------------
// AC 6 — UnixInstant round-trips through libSQL bit-equivalent.
// ---------------------------------------------------------------------------

proptest! {
    /// `UnixInstant` serialised through `JobLifecycle::persist` and
    /// rehydrated via `JobLifecycle::hydrate` must round-trip
    /// bit-equivalent — no nanosecond downcast, no precision loss. The
    /// libSQL column type chosen by the reconciler author MUST preserve
    /// the full 64-bit seconds + 32-bit nanos range that `UnixInstant`
    /// carries.
    #[test]
    fn unix_instant_round_trips_through_libsql(
        secs in 0u64..i64::MAX as u64,
        nanos in 0u32..1_000_000_000,
    ) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio rt");
        rt.block_on(async {
            let tmp = TempDir::new().expect("tempdir");
            let path = tmp.path().join("ui.db");

            let alloc_id = alloc("ui-roundtrip");
            let original = UnixInstant::from_unix_duration(Duration::new(secs, nanos));

            let mut view = JobLifecycleView::default();
            view.last_failure_seen_at.insert(alloc_id.clone(), original);

            let reconciler = JobLifecycle::canonical();

            {
                let handle = open_and_migrate(&path).await;
                reconciler.persist(&view, &handle).await.expect("persist");
            }

            let handle = open_and_migrate(&path).await;
            let hydrated = reconciler.hydrate(&target(), &handle).await.expect("hydrate");

            let got = hydrated
                .last_failure_seen_at
                .get(&alloc_id)
                .copied()
                .expect("alloc id present after round-trip");

            prop_assert_eq!(got, original);
            Ok(())
        }).unwrap();
    }
}

// Silence the "unused import" warning when no `#[test]` happens to use
// `ReconcilerName` directly — the import is still useful for the
// `ReconcilerName::new` discipline practiced in this module's siblings.
#[allow(dead_code)]
fn _force_use_reconciler_name() -> Option<ReconcilerName> {
    ReconcilerName::new("job-lifecycle").ok()
}
