//! Integration-test entrypoint per `.claude/rules/testing.md` § Layout.
//!
//! Phase 2.1 step 01-03 wires the first scenario:
//! `build_rs_artifact_check` — asserts the `build.rs` artifact-check
//! diagnostic shape on Linux. Tier 3 smoke for the full
//! `EbpfDataplane` (load → attach → counter > 0 → detach) lives in
//! `cargo xtask integration-test vm latest` (step 03-02), not here.
//!
//! Submodules MUST be declared inside the inline `mod integration { … }`
//! block — Cargo treats each `tests/*.rs` file as a crate root, so a
//! bare `mod foo;` resolves to `tests/foo.rs`, not
//! `tests/integration/foo.rs`. The inline wrapper shifts the lookup
//! base into the subdirectory. See `testing.md` § Layout.

#![cfg(feature = "integration-tests")]

mod integration {
    mod build_rs_artifact_check;
}
