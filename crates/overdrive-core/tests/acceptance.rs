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
}
