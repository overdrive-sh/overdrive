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

use overdrive_core::eval_broker::{Evaluation, EvaluationEligibility};
use overdrive_core::reconcilers::Action;
use std::time::Instant;

use super::EvaluationSubmission;
use crate::reconciler_runtime::ReconcilerRuntime;

/// Dispatch one `Action::EnqueueEvaluation`. Submits an
/// [`Evaluation { reconciler, target }`] to the per-runtime
/// [`EvaluationBroker`] so the downstream reconciler ticks against
/// `target` on the next convergence cycle.
///
/// Production submissions go through the runtime so the convergence loop is
/// woken after the broker lock is released.
///
/// # Panics
///
/// Panics if `action` is not [`Action::EnqueueEvaluation`]. The
/// action shim's exhaustive match arm is the sole expected caller;
/// passing the wrong variant is a programmer error and follows the
/// established precedent across action-shim dispatch wrappers (see
/// [`super::write_service_backend_row::dispatch`] and
/// [`super::dataplane_update_service::dispatch`]).
pub fn dispatch(action: &Action, runtime: &ReconcilerRuntime, now: Instant) {
    let submission = EvaluationSubmission::Runtime(runtime);
    dispatch_for_submission(action, &submission, now);
}

pub(super) fn dispatch_for_submission(
    action: &Action,
    submission: &EvaluationSubmission<'_>,
    now: Instant,
) {
    let Action::EnqueueEvaluation { reconciler, target } = action else {
        panic!(
            "action_shim::enqueue_evaluation::dispatch invoked with \
             wrong Action variant — caller is the action shim's \
             match arm and is the sole expected caller"
        );
    };
    submission.submit(
        Evaluation { reconciler: reconciler.clone(), target: target.clone() },
        now,
        EvaluationEligibility::Immediate,
    );
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures may panic on programmer error per project precedent in tests/"
)]
mod tests {
    use std::time::Instant;

    use overdrive_core::reconcilers::{Action, ReconcilerName, TargetResource};

    use crate::reconciler_runtime::ReconcilerRuntime;

    use super::dispatch;

    fn runtime() -> (tempfile::TempDir, ReconcilerRuntime) {
        let tmp = tempfile::TempDir::new().expect("temporary runtime directory");
        let runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
        (tmp, runtime)
    }

    #[test]
    fn dispatch_submits_evaluation_to_broker() {
        let (_tmp, runtime) = runtime();
        let expected_reconciler = ReconcilerName::new("service-map-hydrator").expect("valid name");
        let expected_target = TargetResource::new("service/42").expect("valid target");
        let action = Action::EnqueueEvaluation {
            reconciler: expected_reconciler.clone(),
            target: expected_target.clone(),
        };

        dispatch(&action, &runtime, Instant::now());

        // Drain — the submitted evaluation must appear exactly once.
        let drained = {
            let mut broker = runtime.broker();
            broker.drain_pending(
                8,
                &std::collections::BTreeSet::new(),
                Instant::now(),
                overdrive_core::UnixInstant::from_unix_duration(std::time::Duration::ZERO),
            )
        };
        assert_eq!(drained.len(), 1, "exactly one evaluation must be pending");
        assert_eq!(drained[0].0.reconciler, expected_reconciler);
        assert_eq!(drained[0].0.target, expected_target);
    }

    #[test]
    fn dispatch_collapses_duplicate_submits_at_same_key() {
        let (_tmp, runtime) = runtime();
        let reconciler = ReconcilerName::new("service-map-hydrator").expect("valid name");
        let target = TargetResource::new("service/7").expect("valid target");
        let action = Action::EnqueueEvaluation { reconciler, target };

        dispatch(&action, &runtime, Instant::now());
        dispatch(&action, &runtime, Instant::now());

        let drained = {
            let mut broker = runtime.broker();
            broker.drain_pending(
                8,
                &std::collections::BTreeSet::new(),
                Instant::now(),
                overdrive_core::UnixInstant::from_unix_duration(std::time::Duration::ZERO),
            )
        };
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
        let (_tmp, runtime) = runtime();
        dispatch(&Action::Noop, &runtime, Instant::now());
    }
}
