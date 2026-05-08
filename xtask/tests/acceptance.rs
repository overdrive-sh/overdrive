//! Acceptance-test entrypoint for the `xtask` crate.
//!
//! Each scenario from
//! `docs/feature/phase-1-foundation/distill/test-scenarios.md` §6.1 and
//! §6.2 is translated to a Rust integration-test module under
//! `tests/acceptance/*.rs` per ADR-0005. This entrypoint wires those
//! modules into a single Cargo integration-test binary.
//!
//! Gated behind the `integration-tests` feature — see the feature
//! comment in `xtask/Cargo.toml` for rationale.

#![cfg(feature = "integration-tests")]
// `expect` is the standard idiom in test code — a panic with a message
// is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

mod acceptance {
    //! Acceptance scenarios for xtask — phase-1-foundation `dst_lint_*`
    //! gates. The `dst_*` subprocess scenarios moved to
    //! `crates/overdrive-sim/tests/integration/` when the DST harness
    //! binary relocated; the openapi gate moved to
    //! `crates/overdrive-control-plane/tests/` alongside the openapi
    //! library module.
    mod dst_lint_banned_apis;
    mod dst_lint_catches_reconciler_violation;
}
