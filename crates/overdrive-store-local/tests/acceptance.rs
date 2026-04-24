//! Acceptance test entrypoint for `overdrive-store-local`.
//!
//! Each scenario from `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! is translated to a Rust integration-test module under
//! `tests/acceptance/*.rs` per ADR-0005. This entrypoint wires those
//! modules into Cargo's single integration-test binary.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

mod acceptance {
    //! Phase-1-foundation — US-03 `LocalIntentStore` acceptance scenarios.
    mod commit_index_monotonic;
    mod local_store_basic_ops;
    mod local_store_error_paths;
    mod put_if_absent;
    mod snapshot_roundtrip;

    // Phase-1-control-plane-core — step 03-06 `LocalObservationStore`.
    mod local_observation_store;
}
