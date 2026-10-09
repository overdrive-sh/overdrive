// P4 — `intake` Quint conformance (invariants 7, 8, 12; intake listeners).
//
// netns-density-295. Replays `specs/quint/guest-flow-owner/intake.qnt`
// (referenced IN PLACE) against the **production guest-flow owner** through
// its driving ports (`provision` / `activate` / `teardown` /
// `quiesce_forwarding` / `restore_forwarding`) with `SimGuestPrefixSteering`
// at the steering seam and a scripted `ListenState` / session feed. The driver
// projects `State` from the owner's observable surface — open intake listeners,
// steering entries, per-CID alloc state, declared-port reservations — never a
// private field.
//
// Invariants covered (feature-delta § "Formal protocol model"):
//   7  (D8a-LOOKUP, U-3): traffic to a workload address reaches only a steered
//      intake, leg-C or a refusal; every entry names an open listening listener
//      of a serving port; no lease released while a listener of it is open
//   8  (U-6): activation ends all-or-nothing
//   12 (D8a-CAP): reservations ≤ capacity ≥ open listeners ≥ entries; no steer
//      fails for capacity
//
// Named checks replayed as `#[quint_test]` (`checks.toml`):
//   intake-safety-tlc, intake-two-safety-tlc, intake-safety-apalache,
//   intake-serving-reachable, intake-serving-reachable-with-defects,
//   intake-activation-completes, intake-reserved-within-capacity,
//   intake-listeners-within-reserved, intake-entries-within-listeners,
//   intake-no-capacity-steer-failure, intake-steer-forever-unreachable
//   (A-MEM off, expect violation), witnesses, and the hazards
//   (entry-outlives-close / takedown-before-close / rollback-partial /
//    act-ok-partial / teardown-keeps-listeners / quiesce-keeps-listeners /
//    no-admission-over-capacity / no-admission-capacity-steer-failure /
//    cap-retry-unreachable).
//
// Spawns the `quint` CLI → gated behind `integration-tests`, run via
// `cargo xtask lima run --`.

// GATED — compiles to an EMPTY crate until DELIVER lands the production owner +
// `SimGuestPrefixSteering` + the shared quint-connect base
// (`tests/conformance/src/quint_driver.rs`, `vsock_harness.rs`). None exists
// yet; none invented here (CLAUDE.md § "Implement to the design"). DELIVER
// removes the `#![cfg(any())]` gate when the surface lands (co-sequenced per
// shared-test-infrastructure.md § 0 / § 6). `cfg(any())` = always-false,
// no feature name, no `unexpected_cfgs` warning.
#![cfg(any())]

use std::collections::BTreeSet;

use overdrive_system_conformance::quint_driver::{QuintStep, StepDispatch, quint_run, quint_test};
use overdrive_system_conformance::vsock_harness::{
    IntakeConformanceNode, VsockNodeConfig, boot_intake_node,
};

const SPEC: &str = "specs/quint/guest-flow-owner/intake.qnt";

/// Observable projection of `intake.qnt` state (vars: alloc, sess, lastLS,
/// quiesced, lis, entries, reservations). One workload address (the abstraction).
#[derive(Clone, Debug, PartialEq, Eq)]
struct IntakeState {
    up: bool,
    alloc: &'static str, // Absent | Provisioned | Activating | Active | Retiring
    sess: bool,
    last_ls: BTreeSet<u16>,
    quiesced: bool,
    lis: BTreeSet<u16>,     // ports with an open intake listener
    entries: BTreeSet<u16>, // ports with a steering entry
    reserved: u32,          // declared ports reserved across held leases
}

impl IntakeState {
    fn project(node: &IntakeConformanceNode) -> Self {
        Self {
            up: node.serve_up(),
            alloc: node.cid_state(),
            sess: node.session_live(),
            last_ls: node.last_listen_state(),
            quiesced: node.quiesced(),
            lis: node.open_intake_ports(),
            entries: node.steering_entry_ports(),
            reserved: node.reserved_ports(),
        }
    }
}

struct IntakeDriver {
    node: IntakeConformanceNode,
}

impl StepDispatch for IntakeDriver {
    type State = IntakeState;

    fn initial() -> (Self, Self::State) {
        let node = boot_intake_node(VsockNodeConfig::steering_only(
            "100.95.0.0/16".parse().unwrap(),
            std::net::Ipv4Addr::new(100, 95, 0, 1),
        ));
        let st = IntakeState::project(&node);
        (Self { node }, st)
    }

    fn dispatch(&mut self, step: &QuintStep) -> Self::State {
        match step.action() {
            "Provision" => self.node.provision(),
            "ProvisionRefused" => self.node.provision_expecting_capacity_refusal(),
            "SessOpen" => self.node.open_session(),
            "SessLoss" => self.node.lose_session(),
            "ListenState" => self.node.feed_listen_state(step.field_ports("ls")),
            "Activate" => self.node.activate(),
            "Teardown" => self.node.teardown(),
            "Release" => self.node.release_lease(),
            "Quiesce" => self.node.quiesce(),
            "Restore" => self.node.restore(),
            "Steer" => self.node.drive_steer(step.field_port("port")),
            "SteerFailMem" => self.node.script_steer_kernel_memory(1),
            "SteerFailDefect" => self.node.script_steer_defect(),
            "OtherAdmit" => self.node.admit_other_allocation(step.field_u32("ports")),
            "OtherRelease" => self.node.release_other_allocation(),
            other => panic!("unmodelled intake action: {other}"),
        }
        IntakeState::project(&self.node)
    }
}

// ---- named checks.toml runs --------------------------------------------------

#[quint_test(spec = SPEC, run = "intake-safety-tlc")]
fn conforms_to_intake_safety_one_declared() {}

#[quint_test(spec = SPEC, run = "intake-two-safety-tlc")]
fn conforms_to_intake_safety_two_declared() {}

#[quint_test(spec = SPEC, run = "intake-safety-apalache")]
fn conforms_to_intake_safety_apalache() {}

#[quint_test(spec = SPEC, run = "intake-serving-reachable")]
fn conforms_to_serving_reachable_inv7() {}

#[quint_test(spec = SPEC, run = "intake-serving-reachable-with-defects")]
fn conforms_to_serving_reachable_with_defects() {}

#[quint_test(spec = SPEC, run = "intake-activation-completes")]
fn conforms_to_activation_completes_inv8() {}

#[quint_test(spec = SPEC, run = "intake-reserved-within-capacity")]
fn conforms_to_reserved_within_capacity_inv12() {}

#[quint_test(spec = SPEC, run = "intake-listeners-within-reserved")]
fn conforms_to_listeners_within_reserved_inv12() {}

#[quint_test(spec = SPEC, run = "intake-entries-within-listeners")]
fn conforms_to_entries_within_listeners_inv12() {}

#[quint_test(spec = SPEC, run = "intake-no-capacity-steer-failure")]
fn conforms_to_no_capacity_steer_failure_inv12() {}

// A-MEM off: `ServingReachableNoMemRecovery` is reachable-violation — the
// production path reproduces the trace where an un-recovered KernelMemory steer
// leaves a wanted port unreachable (teeth for D8a-CAP-RETRY).
#[quint_test(spec = SPEC, run = "intake-steer-forever-unreachable")]
fn reproduces_steer_forever_unreachable_without_mem_recovery() {}

// hazards (teeth): each reproduces the counterexample the switched-off rule
// admits, proving the production owner implements the rule.
#[quint_test(spec = SPEC, run = "intake-hazard-entry-outlives-close")]
fn reproduces_hazard_entry_outlives_close() {}

#[quint_test(spec = SPEC, run = "intake-hazard-takedown-before-close")]
fn reproduces_hazard_takedown_before_close() {}

#[quint_test(spec = SPEC, run = "intake-hazard-rollback-partial")]
fn reproduces_hazard_rollback_partial() {}

#[quint_test(spec = SPEC, run = "intake-hazard-act-ok-partial")]
fn reproduces_hazard_act_ok_partial() {}

#[quint_test(spec = SPEC, run = "intake-hazard-teardown-keeps-listeners")]
fn reproduces_hazard_teardown_keeps_listeners() {}

#[quint_test(spec = SPEC, run = "intake-hazard-quiesce-keeps-listeners")]
fn reproduces_hazard_quiesce_keeps_listeners() {}

#[quint_test(spec = SPEC, run = "intake-hazard-no-admission-over-capacity")]
fn reproduces_hazard_no_admission_over_capacity() {}

#[quint_test(spec = SPEC, run = "intake-hazard-cap-retry-unreachable")]
fn reproduces_hazard_cap_retry_unreachable() {}

// ---- randomized simulation traces -------------------------------------------

#[quint_run(spec = SPEC, instance = "intake", steps = 50, samples = 300)]
fn random_intake_traces_conform() {
    quint_run::<IntakeDriver>();
}
