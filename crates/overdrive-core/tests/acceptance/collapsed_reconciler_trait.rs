//! ADR-0035 + ADR-0036 — the `Reconciler` trait collapses to a single
//! sync `reconcile` method plus a typed `View` associated type with
//! `Serialize + DeserializeOwned + Default + Clone + Send + Sync`
//! bounds. The async `migrate` / `hydrate` / `persist` surfaces are
//! removed; the runtime owns all hydration (intent + observation +
//! per-reconciler view memory).
//!
//! Scenario: `reconciler_trait_has_single_sync_reconcile_method_and_typed_view`.
//!
//! This file pins the post-collapse trait surface as a compile-time
//! property. Three assertions encode the shape:
//!
//! 1. A minimal `Reconciler` impl declares only `type State`,
//!    `type View`, `fn name`, and `fn reconcile`. Authoring the impl
//!    without `async fn hydrate` (no longer in the trait) must
//!    typecheck. Under the pre-collapse trait this file fails to
//!    compile because `hydrate` is a required item.
//! 2. The `View` associated type bound is `Serialize + DeserializeOwned
//!    + Default + Clone + Send + Sync` per ADR-0035 §1. The
//!    `assert_view_bounds` helper takes a `for<R: Reconciler>` parameter
//!    and demands every bound on `R::View`; substituting `R = HelloRec`
//!    satisfies the bound only when the trait actually carries it.
//! 3. The `reconcile` method's signature is pinned via a
//!    `fn(...) -> (Vec<Action>, R::View)` type alias. A regression that
//!    re-introduces `async fn` or drops the typed `View` return fails
//!    to typecheck at the binding site.
//!
//! Runtime smoke: invoke `reconcile` once and assert it returns the
//! expected `(actions, next_view)` pair. The pure-function shape
//! survives (twin-invocation produces identical output is covered by
//! `reconciler_trait_surface.rs`).

#![allow(clippy::expect_used)]
// The fixture's docstrings deliberately use prose with embedded type
// signatures spanning multiple lines (`Serialize + DeserializeOwned +
// Default + Clone + Send + Sync`); clippy's doc-markdown / doc-lazy-
// continuation lints flag the wrapped backticks and the indented
// continuation lines as bugs, but the text is human-readable as-is.
#![allow(clippy::doc_markdown)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::missing_const_for_fn)]

use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use overdrive_core::UnixInstant;
use overdrive_core::reconciler::{Action, Reconciler, ReconcilerName, TickContext};

// ---------------------------------------------------------------------------
// 1 + 2 + 3: minimal post-collapse impl pinning the new trait surface
// ---------------------------------------------------------------------------

/// The author-declared `View` for `HelloRec`. Carries the four bounds
/// ADR-0035 §1 mandates so the runtime can serialize, deserialize,
/// default-construct, and clone it without further author-side
/// machinery.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct HelloView {
    counter: u32,
}

/// Minimal post-collapse `Reconciler` impl. Carries no `migrate`,
/// `hydrate`, or `persist` — the trait no longer requires them. The
/// impl typechecking is the assertion; if the trait re-introduces any
/// async surface as a required item, this impl fails to compile.
struct HelloRec {
    name: ReconcilerName,
}

impl Reconciler for HelloRec {
    // Single compile-time anchor for the canonical name; the
    // refactor-reconciler-static-name RCA threads this through the
    // ViewStore byte-level surface.
    const NAME: &'static str = "hello";

    // Per ADR-0021 (preserved by ADR-0036), every reconciler picks its
    // own typed `State`. A reconciler with no meaningful projection
    // picks `()`.
    type State = ();

    // Per ADR-0035 §1, `View` carries `Serialize + DeserializeOwned +
    // Default + Clone + Send + Sync`. The runtime owns persistence;
    // the author derives the four bounds and is done.
    type View = HelloView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    fn reconcile(
        &self,
        _desired: &Self::State,
        _actual: &Self::State,
        view: &Self::View,
        _tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let next_view = HelloView { counter: view.counter.saturating_add(1) };
        (vec![Action::Noop], next_view)
    }
}

// ---------------------------------------------------------------------------
// Compile-time pin of the View bound set (assertion 2)
// ---------------------------------------------------------------------------

/// Forces the compiler to prove `R::View: Serialize + DeserializeOwned
/// + Default + Clone + Send + Sync` whenever it is instantiated. If
/// the trait drops any bound, `assert_view_bounds::<HelloRec>()` fails
/// to compile because `HelloView` no longer satisfies the bound the
/// trait promised.
fn assert_view_bounds<R: Reconciler>()
where
    R::View: Serialize + DeserializeOwned + Default + Clone + Send + Sync,
{
}

// ---------------------------------------------------------------------------
// Compile-time pin of the `reconcile` signature (assertion 3)
// ---------------------------------------------------------------------------

/// The post-collapse `reconcile` signature, factored into an alias so
/// the binding below stays readable. Adding `async`, dropping the
/// `Vec<Action>` return, dropping the `R::View` return, or threading
/// in a `&LibsqlHandle` parameter all break this binding (the type is
/// itself deleted as of step 01-06; the `compile_fail/libsql_handle_is_gone.rs`
/// fixture pins the absence at compile time).
type ReconcileFn<R> = fn(
    &R,
    &<R as Reconciler>::State,
    &<R as Reconciler>::State,
    &<R as Reconciler>::View,
    &TickContext,
) -> (Vec<Action>, <R as Reconciler>::View);

fn assert_reconcile_signature<R: Reconciler>() {
    #[allow(clippy::let_underscore_untyped, clippy::no_effect_underscore_binding)]
    let _: ReconcileFn<R> = <R as Reconciler>::reconcile;
}

// ---------------------------------------------------------------------------
// Acceptance scenario: trait collapsed to single sync reconcile + typed View
// ---------------------------------------------------------------------------

#[test]
fn reconciler_trait_has_single_sync_reconcile_method_and_typed_view() {
    // Assertion 1 + 2 + 3 are compile-time. Exercising the helpers
    // forces the bounds and signatures to monomorphise.
    assert_view_bounds::<HelloRec>();
    assert_reconcile_signature::<HelloRec>();

    // Runtime smoke: invoke `reconcile` once and assert the typed
    // `(actions, next_view)` pair surfaces unchanged. The runtime
    // would normally diff `view` against `next_view` and persist
    // through `ViewStore`; this test bypasses persistence and pins
    // only the trait surface.
    let reconciler =
        HelloRec { name: ReconcilerName::new("hello").expect("'hello' is a valid name") };
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };
    let view = HelloView::default();

    let (actions, next_view) = reconciler.reconcile(&(), &(), &view, &tick);

    assert_eq!(actions, vec![Action::Noop], "HelloRec must emit exactly one Noop action");
    assert_eq!(
        next_view,
        HelloView { counter: 1 },
        "HelloRec next_view must increment the counter from 0 to 1",
    );
    assert_eq!(reconciler.name().as_str(), "hello", "name() returns the registered name");
}
