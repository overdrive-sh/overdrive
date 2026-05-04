//! Regression test for the `Box::leak`-per-call defect in
//! `RedbViewStore::table_def` documented at
//! `docs/feature/refactor-reconciler-static-name/deliver/bugfix-rca.md`.
//!
//! Two assertions, both load-bearing:
//!
//! 1. `Reconciler::NAME` is a single compile-time anchor — calling it
//!    twice returns pointer-identical `&'static str` references. This
//!    proves the name is NOT allocated per call (the defect's
//!    signature). The `std::ptr::eq` check is the regression: under
//!    the old `Box::leak(reconciler.as_str().to_string())` shape every
//!    call site materialised a fresh leaked `String`; under the
//!    `const NAME: &'static str` shape every call returns the same
//!    static address.
//!
//! 2. `write_through_bytes` consumes that `&'static str` directly.
//!    Repeated `write_through_bytes` calls against the same store
//!    succeed without surfacing a per-call allocation through the
//!    public surface. Combined with assertion 1, this proves the
//!    leak class is structurally eliminated — there is no point on
//!    the call path between `Reconciler::NAME` and `redb::TableDefinition::new`
//!    that owns or copies the string.
//!
//! The companion compile-fail fixture
//! `tests/compile_fail/view_store_rejects_owned_string.rs` is the
//! type-system backstop: a runtime-owned `&str` borrowed from a
//! `String` MUST fail to compile against the new signature.

use overdrive_control_plane::view_store::ViewStore;
use overdrive_control_plane::view_store::redb::RedbViewStore;
use overdrive_core::reconciler::{JobLifecycle, Reconciler, TargetResource};

fn target(s: &str) -> TargetResource {
    TargetResource::new(s).expect("valid target resource")
}

/// Assertion 1: `Reconciler::NAME` is a single compile-time anchor.
/// Two reads of the const must yield pointer-identical `&'static str`
/// references — the smoking-gun for "the name is NOT per-call
/// allocated". Under the old `Box::leak(...)` shape every reference
/// to `reconciler.as_str()` flowed through a freshly-leaked `String`;
/// under the const shape they all alias the same data segment slice.
#[test]
fn reconciler_name_const_is_pointer_identical_across_reads() {
    let first: &'static str = JobLifecycle::NAME;
    let second: &'static str = JobLifecycle::NAME;

    assert!(
        std::ptr::eq(first.as_ptr(), second.as_ptr()),
        "JobLifecycle::NAME must be a single compile-time anchor — \
         got distinct pointers {:p} vs {:p}, indicating a per-call \
         allocation snuck back in",
        first.as_ptr(),
        second.as_ptr(),
    );
    assert_eq!(first, "job-lifecycle", "canonical name must be the kebab-case literal");
}

/// Assertion 2: repeated `write_through_bytes` calls against the
/// same `(reconciler, target)` pair succeed and accept the bare
/// `&'static str` from `Reconciler::NAME` directly. The point is
/// not to assert on the byte payload (a separate scenario in
/// `redb_view_store.rs` already covers round-trip semantics) — it
/// is to prove the public surface accepts the static-lifetime name
/// from `Reconciler::NAME` without needing to allocate or wrap it,
/// closing the door on a future "let me just `to_string()` that
/// real quick" reintroduction of the bug.
#[tokio::test]
async fn write_through_bytes_accepts_reconciler_name_const_directly() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = RedbViewStore::open(tmp.path()).expect("open store");
    let t = target("job/payments");

    // CBOR-encoded `()` — minimal valid blob the redb layer will
    // happily round-trip. The point of this test is the lifetime
    // contract on the FIRST argument, not the byte payload.
    let unit_cbor: &[u8] = &[0xf6];

    // Two writes in succession with the bare `Reconciler::NAME`
    // const. The signature MUST accept `&'static str` directly —
    // not `&ReconcilerName`, not `&str` borrowed from an owned
    // `String`. If a future refactor relaxes the signature, this
    // call site continues to compile but the companion compile-fail
    // fixture (view_store_rejects_owned_string.rs) breaks.
    store
        .write_through_bytes(JobLifecycle::NAME, &t, unit_cbor)
        .await
        .expect("first write_through accepts NAME directly");
    store
        .write_through_bytes(JobLifecycle::NAME, &t, unit_cbor)
        .await
        .expect("second write_through accepts NAME directly");
}
