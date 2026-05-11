//! `ServiceMapHydrator` reconciler — Slice 08 (US-08; ASR-2.2-04).
//!
//! The §18-reference reconciler J-PLAT-004 activates on. Watches
//! `service_backends` ObservationStore rows, emits one
//! `Action::DataplaneUpdateService` per service whose backend set
//! has drifted, reads `service_hydration_results` rows on the
//! next tick to advance the state machine.
//!
//! Contract per architecture.md § 8 + ADR-0042:
//!
//! - `type State = ServiceMapHydratorState` — typed projection
//!   of (`desired` from `service_backends`, `actual` from
//!   `service_hydration_results`).
//! - `type View = ServiceMapHydratorView` — runtime-persisted
//!   `RetryMemory` keyed on `ServiceId`. Persists *inputs*
//!   (`attempts`, `last_failure_seen_at`,
//!   `last_attempted_fingerprint`) — never derived deadlines per
//!   `.claude/rules/development.md` § Persist inputs, not derived
//!   state.
//! - `fn reconcile` — sync, no `.await`, no wall-clock reads
//!   (`tick.now` is the runtime's snapshot), no DB handle held
//!   by the reconciler. Pure function over `(desired, actual,
//!   view, tick) → (actions, next_view)` per ADR-0035.
//!
//! ESR pair (DST invariants in `crates/overdrive-sim/src/
//! invariants/service_map_hydrator.rs`):
//!
//! - `HydratorEventuallyConverges` (eventual: from any combination
//!   of `service_backends` rows + starting BPF map state, repeated
//!   ticks drive `actual.fingerprint == desired.fingerprint`).
//! - `HydratorIdempotentSteadyState` (always: once converged,
//!   no further `Action` is emitted on subsequent ticks given
//!   unchanged inputs).
//!
//! See test-scenarios.md S-2.2-26..30.
//!
//! ## Module layout
//!
//! The canonical reconciler types and trait impl live in
//! `overdrive-core::reconciler` alongside the `Reconciler` trait
//! and its `WorkloadLifecycle` peer — [`AnyReconciler`] holds the
//! concrete type in its `ServiceMapHydrator` variant, and
//! `overdrive-core` cannot depend on `overdrive-control-plane`.
//!
//! This module exists as the architecture-mandated entry point
//! (`crates/overdrive-control-plane/src/reconcilers/
//! service_map_hydrator/`) per architecture.md § 9. It re-exports
//! the public surface for callers that previously imported from
//! this path; the reconciler's actual implementation is in
//! `overdrive_core::reconciler`.

pub mod state;
pub mod view;

pub use overdrive_core::reconciler::ServiceMapHydrator;
pub use state::{ServiceDesired, ServiceHydrationStatus, ServiceMapHydratorState};
pub use view::{RetryMemory, ServiceMapHydratorView};

/// `ReconcilerName` constant for this reconciler. Wired into the
/// runtime registry per ADR-0035 / ADR-0036 by DELIVER.
pub const NAME: &str = "service-map-hydrator";
