// P4 — `prefix_boot` Quint conformance (invariant 6; boot convergence).
//
// netns-density-295. Replays `specs/quint/guest-flow-owner/prefix_boot.qnt`
// (referenced IN PLACE, never copied — `.claude/rules/testing.md`
// § "Quint conformance") against the **production boot owner** through its
// driving ports, with `SimGuestPrefixSteering` scripting load/swap/pin
// failures and a scripted `serve` crash/restart. The driver maps each Quint
// action to a production call and projects `State` from the owner's observable
// surface (the pinned steering link + its program, the tagged `local` route,
// `serve` up/down) — never a private field.
//
// Named checks replayed as `#[quint_test]` (from `checks.toml`), plus
// `#[quint_run]` randomized traces with the seed printed on failure:
//   safety:   prefix-boot-safety, prefix-boot-safety-apalache,
//             prefix-boot-faulty-safety
//   inv 6:    prefix-boot-local-only-over-link, -local-only-over-link-while-down,
//             prefix-boot-no-wildcard, prefix-boot-nothing-while-down,
//             prefix-boot-open-converged
//   liveness: prefix-boot-opens (BootOpens)
//   witnesses (reachability, expect violation): -witness-refusal, -witness-load-refused,
//             -witness-update, -witness-crash-with-local, -witness-crash-after-swap,
//             -witness-crash-mid-attach, -witness-swap-fail-kept
//   hazards (teeth, expect violation): -hazard-route-first, -hazard-ignore-failure,
//             -hazard-detach, -hazard-unpinned, -hazard-unpinned-open-converged
//
// Spawns the `quint` CLI → gated behind `integration-tests`, run via
// `cargo xtask lima run --` (the Lima template provisions the pinned Quint
// toolchain).

// GATED — compiles to an EMPTY crate until DELIVER lands the production owner
// and the shared quint-connect base. The body below references
// `overdrive_system_conformance::{vsock_harness::boot_vsock_node,
// quint_driver::*}` (shared layer, NOT created by this DISTILL piece —
// shared-test-infrastructure.md § 2 / § 4) and the production
// `SharedGuestNetworkOwner` steering seam, none of which exists yet. Per
// shared-test-infrastructure.md § 0 / § 6 and CLAUDE.md § "Implement to the
// design — never invent API surface", this driver is authored now (the real
// DISTILL product) but the compiling binary is co-sequenced with DELIVER:
// DELIVER creates `tests/conformance/src/quint_driver.rs` + `vsock_harness.rs`
// and the production owner, then REMOVES the `#![cfg(any())]` line below.
// `cfg(any())` is the build-safe "disable this test binary" gate — always
// false, references no feature, emits no `unexpected_cfgs` warning.
#![cfg(any())]

use std::net::Ipv4Addr;

use overdrive_system_conformance::quint_driver::{QuintStep, StepDispatch, quint_run, quint_test};
use overdrive_system_conformance::vsock_harness::{
    ConformanceNode, VsockNodeConfig, boot_vsock_node,
};

const SPEC: &str = "specs/quint/guest-flow-owner/prefix_boot.qnt";

/// The observable projection of `prefix_boot`'s state (feature-delta /
/// prefix_boot.qnt vars: up, phase, link, prog, route). Read from the node's
/// kernel-query surface, never the owner's private fields.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BootState {
    up: bool,
    phase: &'static str, // start | load | swap | pin | route | open
    link: &'static str,  // none | attached | pinned
    prog: String,        // the program version the link runs ("none" with no link)
    route: &'static str, // absent | local
}

impl BootState {
    fn project(node: &ConformanceNode) -> Self {
        Self {
            up: node.serve_up(),
            phase: node.boot_phase(),
            link: node.steering_link_kind(),
            prog: node.steering_program_version(),
            route: node.prefix_route_kind(),
        }
    }
}

/// Maps each Quint action (Restart / BootStep / Fail / Crash) to a production
/// boot effect against the owner + the scripted sim steering, then re-projects.
struct PrefixBootDriver {
    node: ConformanceNode,
}

impl StepDispatch for PrefixBootDriver {
    type State = BootState;

    fn initial() -> (Self, Self::State) {
        let node = boot_vsock_node(VsockNodeConfig::steering_only(
            "100.95.0.0/16".parse().unwrap(),
            Ipv4Addr::new(100, 95, 0, 1),
        ));
        let st = BootState::project(&node);
        (Self { node }, st)
    }

    fn dispatch(&mut self, step: &QuintStep) -> Self::State {
        match step.action() {
            "Restart" => self.node.restart_serve(step.field_str("bin")),
            "BootStep" => self.node.advance_boot_step(),
            "Fail" => self.node.script_boot_step_failure(step.field_str("phase")),
            "Crash" => self.node.crash_serve(),
            other => panic!("unmodelled prefix_boot action: {other}"),
        }
        BootState::project(&self.node)
    }
}

// ---- named checks.toml runs (#[quint_test]) ----------------------------------

#[quint_test(spec = SPEC, run = "prefix-boot-safety")]
fn conforms_to_prefix_boot_safety_tlc() {}

#[quint_test(spec = SPEC, run = "prefix-boot-safety-apalache")]
fn conforms_to_prefix_boot_safety_apalache() {}

#[quint_test(spec = SPEC, run = "prefix-boot-faulty-safety")]
fn conforms_to_prefix_boot_faulty_safety() {}

#[quint_test(spec = SPEC, run = "prefix-boot-local-only-over-link")]
fn conforms_to_local_only_over_link() {}

#[quint_test(spec = SPEC, run = "prefix-boot-local-only-over-link-while-down")]
fn conforms_to_local_only_over_link_while_down() {}

#[quint_test(spec = SPEC, run = "prefix-boot-no-wildcard")]
fn conforms_to_no_wildcard_reached() {}

#[quint_test(spec = SPEC, run = "prefix-boot-nothing-while-down")]
fn conforms_to_nothing_delivered_while_down() {}

#[quint_test(spec = SPEC, run = "prefix-boot-open-converged")]
fn conforms_to_open_converged() {}

#[quint_test(spec = SPEC, run = "prefix-boot-opens")]
fn conforms_to_boot_opens_liveness() {}

// witnesses + hazards are non-vacuity / teeth traces: the production path must
// reproduce the modelled behaviour the counterexample describes (the trace is
// the reachability witness; the driver replays its steps).
#[quint_test(spec = SPEC, run = "prefix-boot-hazard-route-first")]
fn reproduces_hazard_route_first_counterexample() {}

#[quint_test(spec = SPEC, run = "prefix-boot-hazard-ignore-failure")]
fn reproduces_hazard_ignore_failure_counterexample() {}

#[quint_test(spec = SPEC, run = "prefix-boot-hazard-detach")]
fn reproduces_hazard_detach_counterexample() {}

#[quint_test(spec = SPEC, run = "prefix-boot-hazard-unpinned")]
fn reproduces_hazard_unpinned_counterexample() {}

// ---- randomized simulation traces (#[quint_run]) -----------------------------

#[quint_run(spec = SPEC, instance = "boot_faulty", steps = 40, samples = 200)]
fn random_faulty_boot_traces_conform() {
    // quint-connect generates traces from `boot_faulty`; `PrefixBootDriver`
    // replays each step against the production boot path and compares
    // `BootState` after every step. The seed is printed on failure
    // (`.claude/rules/testing.md` § "Quint conformance" / DST reproducibility).
    quint_run::<PrefixBootDriver>();
}
