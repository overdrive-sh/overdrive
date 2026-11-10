//! Shared helpers for `overdrive-dataplane` integration tests.
//!
//! Lives under `tests/integration/helpers/` per `.claude/rules/testing.md`
//! § Layout — every `tests/<scenario>.rs` file is a Cargo crate root, so
//! shared helpers must hang off the inline `mod integration { … }`
//! block in `tests/integration.rs`.

// public-ingress-gateway (S-PIG-41, 56, 57, 63, 67, 71, 72) — the real
// `EbpfDataplane` rig, model frontends/backends and bounded dial for the gateway
// connect-dataplane Tier-3 tests.
pub mod gateway_connect_rig;
// A `GuestStreamDial` that serves no VM, for `HostMtlsEnforcement::new`'s
// required `guest_dial` (LEGS-DIAL) in fixtures with no guest-prefix target.
pub mod guest_dial;
// Shared transparent-mTLS test fixture — the PKI (root→intermediate→leaf chain
// minting) used by the guest-relay and guardrails tests.
pub mod mtls_pki;
pub mod packets;
// public-ingress-gateway (G1; S-PIG-104..118, 123) — real rustls/kTLS peers, kernel
// oracles (`/proc/net/tls_stat`, `/proc/self/fd`) and relay-accepted legs for the
// host kernel-relay Tier-3 suite.
pub mod relay_peers;
pub mod traffic;
pub mod veth;
