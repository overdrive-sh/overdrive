//! Action shim for `Action::EnqueueEvaluation` per UI-05 (the
//! cross-reconciler handoff RCA surfaced during
//! `backend-discovery-bridge-service-reachability` step 02-04
//! walking-skeleton investigation).
//!
//! Dispatch submits an [`Evaluation { reconciler, target }`] to the
//! per-runtime [`EvaluationBroker`] so the named downstream
//! reconciler ticks against `target` on the next convergence cycle.
//! The variant is emitted by a reconciler to make a cross-reconciler
//! handoff explicit at the action boundary — the alternative
//! (implicit re-enqueue inside the action-shim dispatch surface
//! based on emitting-action shape) would couple the shim to
//! reconciler-pair-specific knowledge.
//!
//! No correlation-driven follow-up is needed at the shim level — the
//! downstream reconciler's hydrated `actual` / `desired` carries the
//! state that motivated the enqueue, and the enqueue itself is
//! idempotent at the broker key `(ReconcilerName, TargetResource)`
//! per ADR-0013 §8 / whitepaper §18 (a second submit at the same
//! key collapses to one dispatch).
//!
//! [`Evaluation`]: overdrive_core::eval_broker::Evaluation
//! [`EvaluationBroker`]: overdrive_core::eval_broker::EvaluationBroker

use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};
use overdrive_core::reconciler::Action;

/// Dispatch one `Action::EnqueueEvaluation`. Submits an
/// [`Evaluation { reconciler, target }`] to the per-runtime
/// [`EvaluationBroker`] so the downstream reconciler ticks against
/// `target` on the next convergence cycle.
///
/// The broker access is supplied as a `&mut EvaluationBroker`
/// rather than the runtime's `parking_lot::MutexGuard` so the
/// caller is responsible for the lock-discipline contract (drop
/// the guard before `.await` per
/// `.claude/rules/development.md` § Concurrency & async). The
/// caller in [`super::dispatch_single`] holds the guard for the
/// duration of one synchronous submit and drops it before the next
/// `.await`.
///
/// # Panics
///
/// Panics if `action` is not [`Action::EnqueueEvaluation`]. The
/// action shim's exhaustive match arm is the sole expected caller;
/// passing the wrong variant is a programmer error and follows the
/// established precedent across action-shim dispatch wrappers (see
/// [`super::write_service_backend_row::dispatch`] and
/// [`super::dataplane_update_service::dispatch`]).
pub fn dispatch(action: &Action, broker: &mut EvaluationBroker) {
    let Action::EnqueueEvaluation { reconciler, target } = action else {
        panic!(
            "action_shim::enqueue_evaluation::dispatch invoked with \
             wrong Action variant — caller is the action shim's \
             match arm and is the sole expected caller"
        );
    };
    broker.submit(Evaluation { reconciler: reconciler.clone(), target: target.clone() });
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures may panic on programmer error per project precedent in tests/"
)]
mod tests {
    use overdrive_core::eval_broker::EvaluationBroker;
    use overdrive_core::reconciler::{Action, ReconcilerName, TargetResource};

    use super::dispatch;

    #[test]
    fn dispatch_submits_evaluation_to_broker() {
        let mut broker = EvaluationBroker::new();
        let expected_reconciler = ReconcilerName::new("service-map-hydrator").expect("valid name");
        let expected_target = TargetResource::new("service/42").expect("valid target");
        let action = Action::EnqueueEvaluation {
            reconciler: expected_reconciler.clone(),
            target: expected_target.clone(),
        };

        dispatch(&action, &mut broker);

        // Drain — the submitted evaluation must appear exactly once.
        let drained = broker.drain_pending();
        assert_eq!(drained.len(), 1, "exactly one evaluation must be pending");
        assert_eq!(drained[0].reconciler, expected_reconciler);
        assert_eq!(drained[0].target, expected_target);
    }

    #[test]
    fn dispatch_collapses_duplicate_submits_at_same_key() {
        let mut broker = EvaluationBroker::new();
        let reconciler = ReconcilerName::new("service-map-hydrator").expect("valid name");
        let target = TargetResource::new("service/7").expect("valid target");
        let action = Action::EnqueueEvaluation { reconciler, target };

        dispatch(&action, &mut broker);
        dispatch(&action, &mut broker);

        let drained = broker.drain_pending();
        assert_eq!(
            drained.len(),
            1,
            "second submit at same key must collapse — broker is LWW per ADR-0013 §8"
        );
    }

    #[test]
    #[should_panic(
        expected = "action_shim::enqueue_evaluation::dispatch invoked with wrong Action variant"
    )]
    fn dispatch_panics_on_wrong_variant() {
        let mut broker = EvaluationBroker::new();
        dispatch(&Action::Noop, &mut broker);
    }
}
