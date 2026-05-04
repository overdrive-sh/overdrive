//! Integration-test entrypoint for the `xtask` crate.
//!
//! Per `.claude/rules/testing.md` § "Integration vs unit gating" —
//! integration tests live under `tests/integration/<scenario>.rs` and
//! are wired through this single entrypoint. The whole binary is gated
//! behind the `integration-tests` feature; per-scenario modules inherit
//! the gate without repeating the cfg attribute.
//!
//! Submodules MUST be declared inside an inline `mod integration { … }`
//! block — Cargo treats each `tests/*.rs` file as a crate root, so a
//! bare `mod foo;` resolves to `tests/foo.rs`, not
//! `tests/integration/foo.rs`. The inline wrapper shifts the lookup
//! base into the subdirectory.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]

mod integration {
    mod crate_class_metadata;
}
