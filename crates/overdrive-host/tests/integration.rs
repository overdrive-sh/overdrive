//! Integration-test entrypoint for `overdrive-host`.
//!
//! Per `.claude/rules/testing.md` §"Integration vs unit gating":
//! integration tests — those that touch real infrastructure (real crypto
//! key generation, real `openssl verify` subprocess, real kernel keyring)
//! or whose wall-clock exceeds the default unit-test budget — live under
//! `crates/overdrive-host/tests/integration/*.rs` and are wired into a
//! single Cargo integration-test binary by this entrypoint.
//!
//! Gated behind the `integration-tests` feature — see the feature comment
//! in `overdrive-host/Cargo.toml`. On macOS these run via Lima
//! (`cargo xtask lima run -- cargo nextest run -p overdrive-host
//! --features integration-tests`).
//!
//! built-in-ca (GH #28): the `RcgenCa` host-adapter acceptance suite —
//! real X.509 chain verification (`openssl verify`) and the root-key AEAD
//! envelope (HKDF->AES-256-GCM). These are DISTILL RED scaffolds
//! (`#[should_panic(expected = "RED scaffold")]`); DELIVER replaces the
//! `panic!` bodies with real `RcgenCa` assertions.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

// The inline `mod integration { ... }` mirrors the `tests/integration.rs`
// pattern used across the workspace (overdrive-control-plane, overdrive-cli):
// an integration-test crate root resolves `mod foo;` against `tests/foo.rs`,
// not `tests/integration/foo.rs`. Wrapping the declarations in an inline
// module of the matching name shifts the lookup base so the per-scenario
// files under `tests/integration/` resolve naturally.
mod integration {
    // built-in-ca (GH #28) — RcgenCa real-crypto acceptance.
    mod rcgen_ca_chain_verify;
    mod rcgen_ca_root_key_envelope;
}
