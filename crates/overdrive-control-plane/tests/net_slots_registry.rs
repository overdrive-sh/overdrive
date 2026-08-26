//! Default-lane entrypoint for the cross-file net-slot partitioning registry.
//!
//! The registry SSOT (`tests/common/net_slots.rs`) is `#[path]`-included by the
//! feature-gated `integration` binary so the real-netns integration tests draw
//! their slots from it. This top-level, NON-feature-gated binary includes the
//! SAME source so the registry's structural disjointness guard
//! (`registered_bands_are_pairwise_disjoint_and_within_domain`) runs in the
//! DEFAULT lane — a fast, pure, no-I/O check that fails the build if two file
//! bands ever overlap or leave the valid slot domain, independently of whether
//! the heavyweight integration suite is compiled.
//!
//! Run: `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane \
//!   -E 'test(registered_bands_are_pairwise_disjoint_and_within_domain)'`
//! (no `--features integration-tests`).

// `expect` / `unwrap` / `panic` are the standard idioms in test-support code —
// a panic with a message is exactly what you want when an invariant is violated.
// Mirror the allow-set the `integration` binary carries at its crate root, since
// the shared `net_slots.rs` source is compiled into BOTH binaries.
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

#[path = "common/net_slots.rs"]
mod net_slots;
