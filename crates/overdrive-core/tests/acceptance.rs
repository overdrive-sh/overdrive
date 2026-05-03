//! Acceptance test entrypoint.
//!
//! Each scenario from `docs/feature/{feature-id}/distill/test-scenarios.md`
//! is translated to a Rust integration-test module under
//! `tests/acceptance/*.rs` per ADR-0005. This entrypoint wires those
//! modules into Cargo's single integration-test binary.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

mod acceptance {
    //! Phase-1-foundation + phase-1-control-plane-core acceptance
    //! scenarios.

    // Phase-1-foundation acceptance scenarios.
    mod content_hash_cert_serial;
    mod core_newtype_roundtrip;
    mod core_newtype_validation;
    mod extended_newtype_completeness;
    mod spiffe_region_validation;

    // Phase-1-control-plane-core acceptance scenarios.
    mod aggregate_constructors;
    mod aggregate_roundtrip;
    mod aggregate_validation;
    mod intent_key_canonical;
    mod observation_row_display;
    mod reconciler_trait_surface;

    // Bug-fix `fix-observation-lww-merge` — function-level mutation-killing
    // surface for `LogicalTimestamp::dominates`. Trait-level conformance
    // is exercised from each adapter's test suite via
    // `overdrive_core::testing::observation_store::run_lww_conformance`.
    mod logical_timestamp_dominates;

    // phase-1-first-workload — branch-coverage tests pinning the
    // `JobLifecycle::reconcile` decision points (Stop/Run/Restart).
    mod any_reconciler_dispatch;
    mod first_fit_place_branches;
    mod job_lifecycle_reconcile_branches;

    // wire-exec-spec-end-to-end — operator-facing job spec carries
    // explicit `[exec]` block (command + args) and the projection
    // flows end-to-end through Job::from_spec → Action::Start/Restart.
    // Per ADR-0031.
    mod exec_constructors;
    mod exec_reconciler_purity;
    mod exec_roundtrip;
    mod exec_validation;

    // issue-141-persist-backoff-inputs — `UnixInstant` newtype for
    // portable wall-clock deadlines. Step 01-01 covers arithmetic +
    // constructor surface; step 01-02 covers Display/FromStr/Serde
    // completeness + proptest roundtrips; step 02-01 wires it through
    // `TickContext.now_unix` + introduces the `backoff_for_attempt`
    // const fn; subsequent steps wire it through `JobLifecycleView`.
    mod unix_instant_arithmetic;
    mod unix_instant_completeness;

    // Step 02-01 — `TickContext.now_unix` field surface +
    // `backoff_for_attempt` const fn. The runtime construction-site
    // verification lives in the control-plane acceptance suite (the
    // core crate cannot build an `AppState` without circular deps).
    mod tick_context_now_unix;
}
