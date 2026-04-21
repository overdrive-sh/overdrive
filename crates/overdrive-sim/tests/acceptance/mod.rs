//! Acceptance harness root for `overdrive-sim`.
//!
//! SCAFFOLD: true — DISTILL placeholder per DWD-06. Each DISTILL
//! scenario in `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! is translated by the crafter into one `#[test]` / `#[tokio::test]`
//! function in this directory.
//!
//! Per ADR-0005, `tests/acceptance/` is RESERVED for DISTILL-translated
//! scenarios. Plumbing integration tests stay in `tests/*.rs` at the
//! crate root.

// DELIVER adds sub-modules such as:
//   mod walking_skeleton;
//   mod us_03_intent_store;
//   mod us_04_observation_store;
//   mod us_05_lint_gate;
//   mod us_06_dst_harness;
