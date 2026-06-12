//! Shared helpers for `overdrive-dataplane` integration tests.
//!
//! Lives under `tests/integration/helpers/` per `.claude/rules/testing.md`
//! § Layout — every `tests/<scenario>.rs` file is a Cargo crate root, so
//! shared helpers must hang off the inline `mod integration { … }`
//! block in `tests/integration.rs`.

// transparent-mtls-host-socket (ADR-0069, GH #26) — the composed
// walking-skeleton netns/veth + cgroup-isolated-workload topology fixture
// (single-consumer; step 01-01). Promote to `overdrive-testing` only if a
// second consumer appears.
pub mod mtls_netns_topology;
pub mod packets;
pub mod traffic;
pub mod veth;
