//! Integration-test entrypoint per `.claude/rules/testing.md` § Layout.
//!
//! Phase 2.1 step 01-02 ships this entrypoint with no scenarios. Tier
//! 3 smoke for `EbpfDataplane` (load → attach to `lo` → counter > 0
//! → clean detach) lives in `cargo xtask integration-test vm latest`
//! (step 03-02), not in this crate's `tests/` directory.
//!
//! When future scenarios land they go under
//! `tests/integration/<scenario>.rs` and are wired through the inline
//! `mod integration { … }` block below — see the layout rule in
//! `testing.md`.

#![cfg(feature = "integration-tests")]

mod integration {
    // No scenarios in 01-02. Tier 3 smoke for EbpfDataplane lives in
    // `cargo xtask integration-test vm latest`.
}
