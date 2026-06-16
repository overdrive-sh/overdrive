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
// Shared transparent-mTLS test fixtures — the PKI (root→intermediate→leaf chain
// minting) and the WORKER/peer/workload/server role harness. Promoted out of the
// 01-01 walking-skeleton subdir so BOTH the composed walking skeleton AND the
// 02-02 agent-handshake acceptance test drive `HostMtlsEnforcement::enforce`
// through ONE shared role harness — no parallel implementation (§ "Extension
// Justification (Mandate against Parallel Implementations)").
pub mod mtls_pki;
pub mod mtls_roles;
pub mod packets;
pub mod traffic;
pub mod veth;
