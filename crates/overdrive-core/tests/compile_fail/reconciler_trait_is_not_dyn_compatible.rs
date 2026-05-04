//! Compile-fail fixture asserting that the `Reconciler` trait is
//! deliberately NOT dyn-compatible after the
//! `refactor-reconciler-static-name` change.
//!
//! Per ADR-0035 §1 the runtime uses `AnyReconciler` (an enum-dispatch
//! wrapper) for heterogeneous reconciler dispatch, NOT
//! `Box<dyn Reconciler>`. The `const NAME: &'static str` associated
//! item added by the `refactor-reconciler-static-name` RCA is the
//! load-bearing source of that incompatibility — it lets every
//! `ViewStore::{bulk_load_bytes, write_through_bytes, delete}` call site
//! receive `&'static str` directly from `Reconciler::NAME` without an
//! interner or `Box::leak`.
//!
//! If a future refactor either (a) removes the `const NAME` to "make
//! the trait dyn-compatible again" or (b) reintroduces a runtime-owned
//! `&str` shape on the ViewStore surface, this fixture starts compiling
//! and the trybuild harness in `tests/compile_fail.rs` flags the
//! regression. The diagnostic — "the trait `Reconciler` is not dyn
//! compatible because it contains associated const `NAME`" — IS the
//! load-bearing assertion. The sibling `.stderr` fixture pins the
//! exact compiler diagnostic.
//!
//! The property the deleted `compile_pass/reconciler_trait_is_dyn_compatible.rs`
//! fixture was guarding (catching an `async fn` regression on the
//! trait) is now covered by the typed `ReconcileFn<R>` alias in
//! `tests/acceptance/reconciler_trait_surface.rs::enforce_pure_sync_signature`.

use overdrive_core::reconciler::Reconciler;

fn main() {
    // Coercing a concrete reconciler into `Box<dyn Reconciler<...>>`
    // MUST fail to compile because of the `const NAME: &'static str`
    // associated item. If this line ever compiles, the `&'static str`
    // pipeline through the ViewStore surface has been compromised.
    let _: Option<Box<dyn Reconciler<State = (), View = ()>>> = None;
}
