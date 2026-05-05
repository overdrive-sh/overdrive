//! Integration-test entrypoint for `overdrive-sim`.
//!
//! Per `.claude/rules/testing.md` § "Integration vs unit gating":
//! integration tests live under `tests/integration/<scenario>.rs`
//! and are wired through this single entrypoint. Whole binary
//! gated behind `integration-tests` feature; per-scenario modules
//! inherit the gate without repeating the cfg attribute.
//!
//! Submodules MUST be declared inside an inline `mod integration { … }`
//! block — Cargo treats each `tests/*.rs` file as a crate root, so a
//! bare `mod foo;` resolves to `tests/foo.rs`, not
//! `tests/integration/foo.rs`. The inline wrapper shifts the lookup
//! base into the subdirectory.
//!
//! Phase 2.2 first integration scenario:
//! - `maglev_churn` — DST proptest of ASR-2.2-02 (≤ 1 % Maglev
//!   incidental disruption) and S-2.2-12 (Maglev determinism). RED
//!   scaffold; DELIVER fills the body per Slice 04.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

mod integration {
    /// phase-2-xdp-service-map Slice 04 (US-04) — Maglev determinism
    /// + ≤ 1 % incidental disruption proptests per
    /// `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
    /// S-2.2-12, S-2.2-13. RED scaffolds; DELIVER fills the bodies.
    mod maglev_churn;
}
