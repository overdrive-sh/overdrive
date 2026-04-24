//! ADR-0013 §2c — `&LibsqlHandle` cannot leak into `reconcile`'s
//! parameter list. The only visibility path for `LibsqlHandle` is
//! `hydrate`; substituting it for the trait's `&TickContext`
//! parameter in a `reconcile` impl must fail to compile.
//!
//! The `Reconciler` trait fixes the `reconcile` signature as
//! `fn(&Self, &State, &State, &Self::View, &TickContext) ->
//! (Vec<Action>, Self::View)`. A synthetic impl that replaces
//! `&TickContext` with `&LibsqlHandle` triggers E0053 ("method has
//! an incompatible type for trait"), naming the expected
//! `&TickContext` and the supplied `&LibsqlHandle`.
//!
//! This defends against a future refactor that accidentally relaxes
//! the trait method signature to accept `&LibsqlHandle` in
//! `reconcile`, which would re-open the async I/O surface the
//! pre-hydration pattern was designed to close.

use overdrive_core::reconciler::{
    Action, HydrateError, LibsqlHandle, Reconciler, ReconcilerName, State, TargetResource,
};

struct BadReconciler {
    name: ReconcilerName,
}

impl Reconciler for BadReconciler {
    type View = ();

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    async fn hydrate(
        &self,
        _target: &TargetResource,
        _db: &LibsqlHandle,
    ) -> Result<Self::View, HydrateError> {
        Ok(())
    }

    // The trait requires `&TickContext` in the fifth parameter slot;
    // substituting `&LibsqlHandle` is a type mismatch the compiler
    // catches via E0053.
    fn reconcile(
        &self,
        _desired: &State,
        _actual: &State,
        _view: &Self::View,
        _db: &LibsqlHandle,
    ) -> (Vec<Action>, Self::View) {
        (vec![], ())
    }
}

fn main() {}
