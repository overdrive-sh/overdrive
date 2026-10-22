//! Integration-test entrypoint for `overdrive-host`.
//!
//! Per `.claude/rules/testing.md` §"Integration vs unit gating":
//! integration tests — those that touch real infrastructure (real crypto
//! key generation, real `openssl verify` subprocess, real kernel keyring)
//! or whose wall-clock exceeds the default unit-test budget — live under
//! `crates/overdrive-host/tests/integration/*.rs` and are wired into a
//! single Cargo integration-test binary by this entrypoint.
//!
//! Gated behind the `integration-tests` feature — see the feature comment
//! in `overdrive-host/Cargo.toml`. On macOS these run via Lima
//! (`cargo xtask lima run -- cargo nextest run -p overdrive-host
//! --features integration-tests`).
//!
//! built-in-ca (GH #28): the `RcgenCa` host-adapter acceptance suite —
//! real X.509 chain verification (`openssl verify`) and the root-key AEAD
//! envelope (HKDF->AES-256-GCM). These are DISTILL RED scaffolds
//! (`#[should_panic(expected = "RED scaffold")]`); DELIVER replaces the
//! `panic!` bodies with real `RcgenCa` assertions.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

// The inline `mod integration { ... }` mirrors the `tests/integration.rs`
// pattern used across the workspace (overdrive-control-plane, overdrive-cli):
// an integration-test crate root resolves `mod foo;` against `tests/foo.rs`,
// not `tests/integration/foo.rs`. Wrapping the declarations in an inline
// module of the matching name shifts the lookup base so the per-scenario
// files under `tests/integration/` resolve naturally.
mod integration {
    /// The production numeric VMM uid/gid pair with the groups that own the two
    /// devices the confined fork opens. Native KVM tests share this one
    /// test-support derivation so their launch identity cannot drift by
    /// hard-coded host device gids.
    #[cfg(feature = "kvm-tests")]
    fn production_vmm_identity() -> overdrive_core::vm::config::VmmIdentity {
        use std::os::unix::fs::MetadataExt as _;

        use overdrive_core::vm::config::{Gid, OVERDRIVE_VMM_UID, VmmIdentity};

        let group_of = |device: &str| {
            Gid::new(
                std::fs::metadata(device)
                    .unwrap_or_else(|error| panic!("stat {device}: {error}"))
                    .gid(),
            )
        };
        let kvm = group_of("/dev/kvm");
        let vhost = group_of("/dev/vhost-vsock");
        // The production composition uses the reserved numeric pair 4200:4200
        // and discovers the device groups into a non-empty supplementary set.
        // Keep KVM first as the existing owner path does; add vhost-vsock only
        // when it is a distinct group. `--groups=` must never render empty.
        let mut supplementary = vec![kvm];
        if vhost != kvm {
            supplementary.push(vhost);
        }
        VmmIdentity { uid: OVERDRIVE_VMM_UID, gid: Gid::new(OVERDRIVE_VMM_UID), supplementary }
    }

    // built-in-ca (GH #28) — RcgenCa real-crypto acceptance.
    mod rcgen_ca_chain_verify;
    mod rcgen_ca_root_key_envelope;
    // built-in-ca (GH #28, ADR-0063 D3) — SystemdCredsKeyring holds the KEK
    // in the real Linux kernel keyring (add_key/keyctl).
    mod systemd_creds_keyring;
    // microvm-driver-cloud-hypervisor step 01-09 (ADR-0082 §D8, GH #42) —
    // `CgroupAccounting` adapter equivalence (S-VM-93). Drives both
    // `RealCgroupAccounting` and `SimCgroupAccounting` against real
    // tempfile fixtures / injection hooks. Never goes through the `Vmm`
    // port and never spawns cloud-hypervisor, so this stays gated by
    // `integration-tests` alone — deliberately NOT `kvm-tests`.
    mod cgroup_accounting_equivalence;
    // microvm-driver-cloud-hypervisor step 01-06 (GH #42) — real Cloud
    // Hypervisor boot surface. Additionally gated `kvm-tests`: the x86_64
    // + nested-KVM floor these need cannot be honoured on arm64 Lima, so
    // a host that never enables the second feature never compiles them.
    // See `.claude/rules/testing.md` § "Running tests — bare-metal KVM
    // box (kvm-tests)".
    #[cfg(feature = "kvm-tests")]
    mod vmm_equivalence;
    #[cfg(feature = "kvm-tests")]
    mod vmm_ficlone_per_launch;
    // microvm-driver-cloud-hypervisor step 02-01 (ADR-0083 §D7, GH #42) —
    // `VmHostState` adapter equivalence (S-VM-91). Additionally gated
    // `kvm-tests`: the real-adapter half boots a real Cloud Hypervisor
    // VMM (via `Vmm` directly) so `kill_scope`'s settle postcondition is
    // exercised against a real cgroup v2 scope holding a real live
    // process, same x86_64 + nested-KVM floor as `vmm_equivalence`.
    #[cfg(feature = "kvm-tests")]
    mod vm_host_state_equivalence;
    // netns-density-295 (CID-NOCLAIM, RESET-EXIT; DG-2) — the fork launch with
    // the VMM-opened `/dev/vhost-vsock` device: the VMM is the only holder,
    // the lease outlives the VMM, and a guest reset ends the VMM. `kvm-tests`:
    // native x86_64 metal is the only real-guest lane.
    #[cfg(feature = "kvm-tests")]
    mod vsock_vmm_device;
    // netns-density-295 R5-59 (a), beacon seam: `VsockBeaconIo` against
    // `SimBeaconIo` with a guest-kit microVM as the peer (native metal).
    #[cfg(feature = "kvm-tests")]
    mod vsock_beacon_accept_path;
}
