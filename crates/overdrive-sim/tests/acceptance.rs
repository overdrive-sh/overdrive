//! Acceptance test entrypoint for `overdrive-sim`.
//!
//! Each scenario from `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! is translated to a Rust integration-test module under
//! `tests/acceptance/*.rs` per ADR-0005. This entrypoint wires those
//! modules into Cargo's single integration-test binary.

// `expect` / `expect_err` are the standard idiom in test code.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

mod acceptance {
    //! Phase-1-foundation acceptance scenarios.

    // US-04 — ObservationStore trait + SimObservationStore.
    mod sim_observation_gossip;
    mod sim_observation_gossip_mechanics;
    mod sim_observation_lww_converges;
    mod sim_observation_single_peer;

    // US-06 §6.1 — Sim adapters for every nondeterminism port.
    mod sim_adapters_deterministic;

    // Step 04-05 — Reconciler-primitive DST invariants.
    mod reconciler_invariants_pass;
}
