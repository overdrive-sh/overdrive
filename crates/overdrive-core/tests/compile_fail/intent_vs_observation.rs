//! §4.4 — `IntentStore` and `ObservationStore` are NOT substitutable.
//!
//! Passing an `&dyn IntentStore` to a function parameter typed
//! `&dyn ObservationStore` must be a compile error, and the diagnostic
//! must name both trait paths so the operator can tell which side of
//! the state split they conflated.
//!
//! Counterpart to `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! §4.4 "A function taking an observation store rejects an intent store
//! at compile time".

use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;

fn expects_observation(_store: &dyn ObservationStore) {}

fn takes_intent(store: &dyn IntentStore) {
    expects_observation(store);
}

fn main() {}
