//! Integration-test entrypoint for `overdrive-store-local`.
//!
//! Per `.claude/rules/testing.md` §"Integration vs unit gating":
//! integration tests — those that touch real infrastructure
//! (filesystem, network, subprocesses, real consensus / gossip) or
//! whose wall-clock exceeds the default unit-test budget — live under
//! `crates/{crate}/tests/integration/*.rs` and are wired into a single
//! Cargo integration-test binary by this entrypoint.
//!
//! Gated behind the `integration-tests` feature — see the feature
//! comment in `overdrive-store-local/Cargo.toml`.

#![cfg(feature = "integration-tests")]
// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

// The inline `mod integration { ... }` mirrors the `tests/acceptance.rs`
// pattern: an integration-test crate root resolves `mod foo;` against
// `tests/foo.rs`, not `tests/integration/foo.rs`. Wrapping the
// declarations in an inline module of the matching name shifts the
// lookup base so the per-scenario files under `tests/integration/`
// resolve naturally.
mod integration {
    //! Per ADR-0020 (drop `commit_index` from Phase 1) the
    //! `commit_counter_invariant` module was deleted in step 01-04 of
    //! `redesign-drop-commit-index` — the counter it asserted no
    //! longer exists.
    mod envelope_helpers;
    mod envelope_intent_refuse;
    mod envelope_observation_skip;
    mod envelope_walking_skeleton;
    mod lww_conformance;
    // fix-issuance-ordinal-toctou Step 01-01 — issuance-ordinal
    // allocation conformance (shared trait harness) + host-only
    // durable-across-reopen sub-case (ADR-0063 D6 rev 8).
    mod issuance_ordinal_conformance;
    mod issuance_ordinal_durable_reopen;
    mod observation_backend_proptest;
    mod probe_result_roundtrip;
    mod snapshot_proptest;
}
