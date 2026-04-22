//! Acceptance-test entrypoint for the `xtask` crate.
//!
//! Each scenario from
//! `docs/feature/phase-1-foundation/distill/test-scenarios.md` §6.1 and
//! §6.2 is translated to a Rust integration-test module under
//! `tests/acceptance/*.rs` per ADR-0005. This entrypoint wires those
//! modules into a single Cargo integration-test binary.

// `expect` is the standard idiom in test code — a panic with a message
// is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

mod acceptance {
    //! Phase-1-foundation acceptance scenarios for xtask.
    mod dst_lint_banned_apis;
}
