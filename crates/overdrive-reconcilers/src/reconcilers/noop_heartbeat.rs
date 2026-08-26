//! Phase 1 proof-of-life reconciler.
//!
//! `NoopHeartbeat::reconcile` always emits `vec![Action::Noop]` and
//! an unchanged `()` next-view; `hydrate` is a trivial `Ok(())`. The
//! reconciler serves as the fixture against which the
//! `ReconcilerIsPure` invariant's twin-invocation check runs and as the
//! seed entry for the `AtLeastOneReconcilerRegistered` invariant.
//!
//! The struct lives in `overdrive-core::reconcilers` (rather than
//! in `overdrive-control-plane`) because `AnyReconciler` — the enum
//! that replaces `Box<dyn Reconciler>` — holds the concrete type in
//! its `NoopHeartbeat` variant.

use super::{Action, Reconciler, ReconcilerName, TickContext};

pub struct NoopHeartbeat {
    name: ReconcilerName,
}

impl NoopHeartbeat {
    /// Construct the canonical `noop-heartbeat` instance. Named
    /// constructor rather than `Default` because the name is not
    /// defaultable — it carries the canonical string literal.
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal
    /// satisfying every `ReconcilerName` validation rule. Failure
    /// would indicate a bug in the newtype constructor.
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .expect("'noop-heartbeat' is a valid ReconcilerName by construction");
        Self { name }
    }
}

impl Reconciler for NoopHeartbeat {
    /// Canonical kebab-case name; single compile-time anchor.
    const NAME: &'static str = "noop-heartbeat";

    // Per ADR-0021, reconcilers with no meaningful projection pick
    // `type State = ()`. `NoopHeartbeat` ignores `desired`/`actual`
    // entirely and always emits `Action::Noop`.
    type State = ();
    // Per ADR-0035 §1, `View` carries `Serialize + DeserializeOwned +
    // Default + Clone + Send + Sync`. `()` satisfies them trivially.
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
