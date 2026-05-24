//! `ServiceMapHydratorView` — typed reconciler memory persisted by
//! the runtime via `RedbViewStore` per ADR-0035 + architecture.md
//! § 8.
//!
//! Persists *inputs* per `.claude/rules/development.md` § Persist
//! inputs, not derived state — `attempts` + `last_failure_seen_at`
//! + `last_attempted_fingerprint`. The next-attempt deadline is
//! recomputed every tick from these inputs + the live backoff
//! policy, never persisted.
//!
//! `BTreeMap` per § Ordered-collection choice.
//!
//! Re-exports the canonical types from `overdrive-core::reconciler`,
//! where they live alongside the `Reconciler` trait impl. The
//! `overdrive-core` placement is load-bearing — `AnyReconciler`
//! holds the concrete type in its `ServiceMapHydrator` variant, and
//! `overdrive-core` cannot depend on `overdrive-control-plane`.

pub use overdrive_core::reconcilers::{RetryMemory, ServiceMapHydratorView};
