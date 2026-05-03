//! ADR-0035 §1 — after the collapse, `Reconciler` carries no `async
//! fn` and only synchronous methods. The trait is therefore
//! dyn-compatible *for any concrete `(State, View)` pair*: a caller
//! holding a `Box<dyn Reconciler<State = (), View = ()>>` can invoke
//! `reconcile` and `name` through the dyn dispatch.
//!
//! Note: bare `dyn Reconciler` is NOT a valid type — the trait carries
//! associated types (`State`, `View`) and Rust requires every
//! associated type to be specified at the dyn-trait reference. Erased
//! dispatch *across heterogeneous reconciler kinds* is provided by
//! `AnyReconciler` (an enum), not by raw dyn-dispatch.
//!
//! This fixture proves the compile-time property that the
//! associated-type-pair-anchored dyn form compiles. A regression that
//! re-introduces `async fn` on the trait would break this fixture
//! (async-fn-in-trait drives dyn-incompatibility unless the future is
//! erased; the post-collapse trait has no async fn at all).

use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::reconciler::{Action, Reconciler, ReconcilerName, TickContext};

struct UnitRec {
    name: ReconcilerName,
}

impl Reconciler for UnitRec {
    type State = ();
    type View = ();

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    fn reconcile(
        &self,
        _desired: &Self::State,
        _actual: &Self::State,
        _view: &Self::View,
        _tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        (vec![Action::Noop], ())
    }
}

fn main() {
    // Concrete-pair dyn-compatibility: the trait MUST be dyn-safe
    // for a given `(State, View)` pair. Coercing `UnitRec` into the
    // boxed dyn is the assertion — if the trait re-acquired an
    // `async fn` (or any other dyn-incompatible item), this line
    // would fail to compile with "the trait `Reconciler` is not
    // dyn compatible".
    let boxed: Box<dyn Reconciler<State = (), View = ()>> =
        Box::new(UnitRec { name: ReconcilerName::new("unit").expect("'unit' is valid") });

    // Invoke through the dyn dispatch — proves the vtable is
    // populated for every method.
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };
    let _ = boxed.reconcile(&(), &(), &(), &tick);
    let _ = boxed.name();
}
