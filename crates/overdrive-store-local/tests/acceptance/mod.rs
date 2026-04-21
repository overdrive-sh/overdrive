//! Acceptance harness root for `overdrive-store-local`.
//!
//! SCAFFOLD: true — DISTILL placeholder per DWD-06. Per ADR-0005,
//! `tests/acceptance/` is reserved for scenarios translated directly
//! from `docs/feature/phase-1-foundation/distill/test-scenarios.md`.
//! The US-03 scenarios (§4 in test-scenarios.md) land here during
//! DELIVER — real redb against `tempfile::TempDir`, per DWD-01
//! (Strategy C).

// DELIVER adds:
//   mod us_03_local_store;
//   mod us_03_snapshot_roundtrip;
