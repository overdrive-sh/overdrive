//! Shared helpers for `overdrive-dataplane` integration tests.
//!
//! Lives under `tests/integration/helpers/` per `.claude/rules/testing.md`
//! § Layout — every `tests/<scenario>.rs` file is a Cargo crate root, so
//! shared helpers must hang off the inline `mod integration { … }`
//! block in `tests/integration.rs`.

pub mod netns;
pub mod packets;
pub mod traffic;
pub mod veth;
