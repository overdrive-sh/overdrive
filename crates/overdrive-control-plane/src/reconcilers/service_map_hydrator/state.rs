//! `ServiceMapHydratorState` — typed projection of intent +
//! observation for the hydrator reconciler per ADR-0021/0036 +
//! architecture.md § 8.
//!
//! Two parts merged by the runtime before `reconcile` is called:
//!
//! - `desired` — keyed by `ServiceId`, sourced from
//!   `service_backends` ObservationStore rows. Carries
//!   `(vip, backends, fingerprint)`.
//! - `actual` — keyed by `ServiceId`, sourced from the
//!   `service_hydration_results` ObservationStore table.
//!   Carries `ServiceHydrationStatus` (Pending / Completed /
//!   Failed) so the reconciler observes the dataplane's
//!   confirmed state, not a next-action prediction (Drift 2).
//!
//! `BTreeMap` per `.claude/rules/development.md` § Ordered-
//! collection choice — deterministic iteration order is
//! load-bearing for the Maglev permutation generator.
//!
//! Re-exports the canonical types from `overdrive-core::reconciler`
//! (the `ServiceHydrationStatus` enum lives in
//! `overdrive_core::traits::observation_store` because it is the
//! row payload, not reconciler-private state). The `overdrive-core`
//! placement is load-bearing — see the corresponding `view.rs`
//! docstring.

pub use overdrive_core::reconciler::{ServiceDesired, ServiceMapHydratorState};
pub use overdrive_core::traits::observation_store::ServiceHydrationStatus;
