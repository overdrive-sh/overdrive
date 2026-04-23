//! §5.3 — `ObservationStore::write` rejects intent-class payloads.
//!
//! The write surface must be parametric on observation-row types, not
//! raw bytes. Passing an intent-class type (here, [`JobSpec`]) to
//! `ObservationStore::write` is a compile error; the diagnostic names
//! both `ObservationRow` (the expected type) and `JobSpec` (the supplied
//! type) so the operator sees which boundary was crossed.
//!
//! Counterpart to `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! §5.3 "Attempting to persist a job spec into the observation store
//! fails at compile time".

use overdrive_core::traits::observation_store::{JobSpec, ObservationStore};

/// Exercises the type boundary: any `ObservationStore` must reject a
/// `JobSpec` payload at compile time because `write` is parametric on
/// [`ObservationRow`](overdrive_core::traits::observation_store::ObservationRow),
/// not `&[u8]` or a generic `T`.
async fn persist_job_spec_into_observation<S: ObservationStore + ?Sized>(
    store: &S,
    spec: JobSpec,
) {
    // This line must fail to compile: `spec` is a `JobSpec` (intent class),
    // `write` expects `ObservationRow` (observation class).
    let _ = store.write(spec).await;
}

fn main() {}
