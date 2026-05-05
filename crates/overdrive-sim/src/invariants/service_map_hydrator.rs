//! `ServiceMapHydrator` ESR invariants — Slice 08 (US-08;
//! ASR-2.2-04).
//!
//! Two named DST invariants per
//! `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
//! S-2.2-26 / S-2.2-27:
//!
//! - [`assert_hydrator_eventually_converges`] — eventual: from
//!   any combination of `service_backends` rows + starting BPF
//!   map state, repeated reconcile ticks drive
//!   `actual.fingerprint == desired.fingerprint`.
//! - [`assert_hydrator_idempotent_steady_state`] — always: once
//!   converged, no further `Action::DataplaneUpdateService` is
//!   emitted on subsequent ticks given unchanged inputs.
//!
//! Wired into the existing `Invariant` enum's exhaustive match at
//! `crates/overdrive-sim/src/invariants/mod.rs` as additive
//! variants `HydratorEventuallyConverges` and
//! `HydratorIdempotentSteadyState`.
//!
//! **RED scaffold** — the evaluator bodies panic with named
//! `RED scaffold` messages until DELIVER fills them per Slice 08.

#![allow(dead_code)]

/// Eventual invariant: every service's `actual.fingerprint`
/// reaches its `desired.fingerprint` within a bounded number of
/// reconcile ticks.
///
/// **RED scaffold** — DELIVER fills the body per Slice 08 / S-2.2-26.
pub fn assert_hydrator_eventually_converges() {
    panic!("Not yet implemented -- RED scaffold: HydratorEventuallyConverges (S-2.2-26)")
}

/// Always invariant: once `actual.fingerprint == desired.fingerprint`
/// for every service, the hydrator emits zero
/// `Action::DataplaneUpdateService` actions per tick.
///
/// **RED scaffold** — DELIVER fills the body per Slice 08 / S-2.2-27.
pub fn assert_hydrator_idempotent_steady_state() {
    panic!("Not yet implemented -- RED scaffold: HydratorIdempotentSteadyState (S-2.2-27)")
}
