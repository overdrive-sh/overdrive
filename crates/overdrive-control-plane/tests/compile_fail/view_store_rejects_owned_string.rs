//! `ViewStore::write_through_bytes` (and its peers) take `&'static str`
//! for the `reconciler` parameter — the type-system backstop for the
//! per-call `Box::leak` defect documented in
//! `docs/feature/refactor-reconciler-static-name/deliver/bugfix-rca.md`.
//!
//! Passing a `&str` borrowed from a runtime-owned `String` MUST NOT
//! compile. If a future refactor relaxes the signature back to `&str`,
//! this fixture stops failing and the trybuild harness in
//! `tests/compile_fail.rs` flags the regression — the door to
//! reintroducing the leak class through a "harmless `to_string()`" is
//! welded shut at the type level.
//!
//! The diagnostic the compiler produces — "borrowed value does not
//! live long enough" or "argument requires that `runtime` is
//! borrowed for `'static`" — IS the load-bearing assertion. A
//! reviewer who wants to relax the signature would have to update the
//! sibling `.stderr` fixture explicitly, which is the gate.

use overdrive_control_plane::view_store::ViewStore;
use overdrive_control_plane::view_store::redb::RedbViewStore;
use overdrive_core::reconciler::TargetResource;

#[tokio::main]
async fn main() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = RedbViewStore::open(tmp.path()).expect("open");
    let target = TargetResource::new("job/payments").expect("valid");

    // Construct an owned `String` at runtime and try to borrow `&str`
    // from it. The borrow's lifetime is bounded by `runtime`'s scope —
    // shorter than `'static`. The `write_through_bytes` signature
    // requires `&'static str`, so this MUST fail to compile.
    let runtime: String = "job-lifecycle".to_string();
    let runtime_ref: &str = runtime.as_str();

    // The line below is the assertion. The borrow is non-`'static`
    // by construction; the call must be rejected with a lifetime
    // mismatch.
    let _ = store.write_through_bytes(runtime_ref, &target, &[]).await;
}
