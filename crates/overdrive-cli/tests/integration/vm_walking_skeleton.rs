//! Walking-skeleton gate for `microvm-driver-cloud-hypervisor` (GH #42).
//!
//! RED scaffolds for US-VM-1's five UAT scenarios
//! (`docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
//! § Slice 01, S-VM-01 through S-VM-05). Per
//! `.claude/rules/testing.md` § "RED scaffolds", every function here is a
//! placeholder — `#[should_panic(expected = "RED scaffold")]` plus a
//! `panic!` naming the scenario. DELIVER's RED phase replaces each body
//! with a real assertion, one scenario at a time, driven through
//! `overdrive_cli::commands::deploy::deploy(DeployArgs { .. })` (direct
//! handler call, real in-process `overdrive serve`, per
//! `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess")
//! against a REAL Cloud Hypervisor VMM, run under
//! `cargo xtask lima run --` as root.
//!
//! **This is the ONLY scenario group in the feature driven with a real
//! guest kernel booting under a real hypervisor.** Every other Tier-3
//! scenario in this feature (see `vm_boot_race_and_composition.rs` and
//! siblings, scaffolded by DELIVER per-slice) reuses this same fixture
//! shape.
//!
//! Fixture prerequisites this test group depends on (Slice 00 PROMOTE,
//! per `spike/wave-decisions.md`): a pinned kernel + ext4 rootfs staged on
//! a reflink-capable filesystem, `cloud-hypervisor` installed, and the
//! `overdrive-init` guest agent baked into the rootfs at its well-known
//! path (`[D4]`). None of that exists in the tree yet — DELIVER's Slice 00
//! artifact-provisioning step lands the fixture helper this file's real
//! bodies will call.
//!
//! System constraint 1 (vertical-slice bar): no test here may install,
//! bind, program, or supply anything `run_server` does not supply itself.
//! `DriverRegistry` composition (discover cloud-hypervisor → probe →
//! insert) happens inside `overdrive serve`'s own boot sequence — these
//! tests never hand-construct a `VmDriver`.

#![allow(clippy::missing_panics_doc)]

/// S-VM-01 — A VM workload runs to completion and its exit code reaches
/// the operator. The walking skeleton itself.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn vm_workload_runs_to_completion_and_exit_code_reaches_operator() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-01 / walking skeleton -- \
         [vm]+[job] deploy boots a real Cloud Hypervisor VM, the guest \
         exits 0, workload describe shows Terminated/Completed{{exit_code: 0}})"
    );
}

/// S-VM-02 — A non-zero guest exit code is reported, never the
/// hypervisor's own exit code.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn vm_non_zero_guest_exit_code_is_reported_not_the_hypervisors() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-02 / guest exits 7, \
         cloud-hypervisor process exits 0 -- workload describe must show \
         Failed{{exit_code: 7}}, never a trace of the VMM's own 0)"
    );
}

/// S-VM-03 — A guest that never starts is never reported Running.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn vm_guest_that_never_starts_is_never_reported_running() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-03 / rootfs with no \
         working init -- boot deadline elapses, allocation goes \
         Pending to Failed, transition history never contains Running -- K2 guardrail)"
    );
}

/// S-VM-04 — A VM workload deploys through the same verb as a process
/// workload; no new verb, no new flag.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn vm_workload_deploys_through_the_same_verb_as_a_process_workload() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-04 / [vm] driver table \
         accepted by the same `overdrive deploy <spec>` verb as [exec], no \
         new CLI surface)"
    );
}

/// S-VM-05 — The platform contains the hypervisor it started: cgroup
/// scope and network-namespace placement, verified on the mTLS-composed
/// production boot (the GH #248 / ADR-0074 trap this feature deliberately
/// re-proves closed).
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn vm_platform_contains_the_hypervisor_it_started() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-05 / on an mTLS-composed \
         serve boot, /proc/<vmm-pid>/cgroup resolves to the allocation's \
         workload scope and /proc/<vmm-pid>/ns/net is spec.netns's inode, \
         not the host netns -- [D7] item 5, US-VM-1 AC 5)"
    );
}
