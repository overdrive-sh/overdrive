//! ADR-0035 §1 + ADR-0036 — `&LibsqlHandle` cannot leak into
//! `reconcile`'s parameter list. The trait's `reconcile` signature is
//! pinned at `fn(&Self, &Self::State, &Self::State, &Self::View,
//! &TickContext) -> (Vec<Action>, Self::View)`. A synthetic impl that
//! replaces the fifth-parameter `&TickContext` with `&LibsqlHandle`
//! triggers E0053 ("method has an incompatible type for trait"), naming
//! the expected `&TickContext` and the supplied `&LibsqlHandle`.
//!
//! Per the ADR-0035 collapse, the trait carries NO async surface at
//! all — `migrate`, `hydrate`, and `persist` are removed. The runtime
//! owns all hydration (intent + observation + view memory). This
//! fixture defends against a future refactor that re-opens the
//! reconciler-author async surface by attempting to plumb a libSQL
//! handle through `reconcile`.

use overdrive_core::reconciler::{Action, LibsqlHandle, Reconciler, ReconcilerName, TickContext};

struct BadReconciler {
    name: ReconcilerName,
}

impl Reconciler for BadReconciler {
    type State = ();
    type View = ();

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    // The trait requires `&TickContext` in the fifth parameter slot;
    // substituting `&LibsqlHandle` is a type mismatch the compiler
    // catches via E0053. The trait NO LONGER has any async hydrate /
    // migrate / persist surface — those are removed by ADR-0035 §1
    // and the runtime owns hydration via `ViewStore`.
    fn reconcile(
        &self,
        _desired: &Self::State,
        _actual: &Self::State,
        _view: &Self::View,
        _db: &LibsqlHandle,
    ) -> (Vec<Action>, Self::View) {
        (vec![], ())
    }
}

fn main() {}
