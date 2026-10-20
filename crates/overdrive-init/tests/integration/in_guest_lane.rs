//! The in-guest lane of `overdrive-init`'s real-kernel cells (GH #295, T-2;
//! D-UP-18, EVMOD-GUEST-KIT, EVMOD-LOAD, D25, D25-BIND, R5-15, R5-59 (b)).
//!
//! `transport::real_kernel` and `guest_flow::real_kernel` observe a guest
//! that the production `overdrive-init` configured on the real guest kernel
//! before READY; they run only as that guest's operator command and fail
//! closed anywhere else. Each case here boots a fresh guest from the guest
//! kit (the metal host's 7.0 kernel, the `ovd_fwd.ko` built for it, the
//! production static `overdrive-init` as PID 1) and runs exactly one cell in
//! it through `overdrive_testing::guest_lane::run_in_guest_cell`: the lane
//! plays the node beacon (READY → `EXEC` → `EXIT`) and holds the guest
//! owner's control session; the cell binary is this crate's libtest binary
//! built with `integration-tests,kvm-tests`.
//!
//! **Oracle.** The cell reports `test result: ok. 1 passed` and the guest's
//! `EXIT` is 0. An in-guest cell that is still ignored reports `0 passed`,
//! so its pending marker keeps this case red as well: activating a cell
//! means removing both markers.
//!
//! Lane: `cargo xtask metal run -- cargo nextest run -p overdrive-init
//! --features integration-tests,kvm-tests`. The lane binds the node-global
//! beacon and control ports, so this binary is in `host-kernel-shared`. CIDs
//! from the [`INIT_IN_GUEST_LANE`] band, one per case.

#![allow(clippy::doc_markdown, reason = "kernel and module names in prose")]

use overdrive_testing::guest_kit::cid_bands::INIT_IN_GUEST_LANE;
use overdrive_testing::guest_kit::{GuestImageSpec, GuestKit, WorkDir};
use overdrive_testing::guest_lane::{BootOutcome, InGuestTests, boot_to_ready, run_in_guest_cell};
use overdrive_testing::vm_fixture::default_staging_root;

/// Boots one guest at the band's `slot` and runs `cell` in it.
fn run_cell(slot: u32, cell: &str) {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("precondition: the guest kit provisions");
    let tests = InGuestTests::prepared("overdrive-init", "overdrive-init")
        .expect("precondition: the in-guest cell binary is staged (cargo xtask guest-kit-prepare)");
    let work = WorkDir::create(&staging, "init-in-guest").expect("work dir");
    let run = run_in_guest_cell(&kit, &tests, cell, INIT_IN_GUEST_LANE.raw(slot), work.path())
        .expect("the guest boots, sends READY and reports EXIT");
    assert!(
        run.passed(),
        "the in-guest cell {cell} must run and pass (exit {}); guest console:\n{}",
        run.exit_status,
        run.console
    );
}

/// The guest transport is configured on the real guest kernel before READY
/// (G-V7): workload address local, `fwmark_reflect`, the reserved ports, every
/// intake bound, the module loaded and referenced, the inbound vsock
/// listener held.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_guest_transport_is_configured_before_ready_in_a_real_guest() {
    run_cell(
        0,
        "transport::real_kernel::the_guest_transport_is_configured_on_the_real_guest_kernel",
    );
}

/// R5-15 (D25-BIND): the intake ports refuse application binds of their own
/// protocol only, and no ephemeral port is reserved.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_reserved_intake_ports_refuse_application_binds_in_a_real_guest() {
    run_cell(
        1,
        "transport::real_kernel::the_reserved_intake_ports_refuse_application_binds_of_their_protocol_only",
    );
}

/// EVMOD-GUEST-KIT: the module the guest loaded was built for the running
/// guest kernel.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_guest_kits_module_matches_the_running_guest_kernel() {
    run_cell(2, "transport::real_kernel::the_guest_kits_module_release_matches_the_running_kernel");
}

/// R5-59 (b), real half: the production guest owner serves the accept-path
/// table with the sim half's per-connection outcome.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_guest_accept_path_matches_the_sim_in_a_real_guest() {
    run_cell(3, "guest_flow::real_kernel::the_accept_path_matches_the_sim_per_connection");
}

/// EVMOD-GUEST-KIT, the missing-module refusal: a root filesystem without the
/// kit's module at `GUEST_FORWARDER_MODULE_PATH` boots to no READY — the
/// production `overdrive-init` refuses before READY with
/// `ForwarderModuleMissing`, names the path on the console, and the guest
/// powers off (the VMM exits).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_guest_without_the_kits_module_refuses_before_ready() {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("precondition: the guest kit provisions");
    let work = WorkDir::create(&staging, "init-no-module").expect("work dir");
    let spec = GuestImageSpec { forwarder_module: false, ..GuestImageSpec::workload() };
    let outcome = boot_to_ready(&kit, &spec, INIT_IN_GUEST_LANE.raw(4), work.path())
        .expect("the guest boots and ends");
    let BootOutcome::ExitedBeforeReady { console } = outcome else {
        panic!("a guest without the kit's module must not send READY: {outcome:?}");
    };
    assert!(
        console.contains("overdrive-init: guest transport setup failed: the guest forwarder module /lib/overdrive/ovd_fwd.ko is missing"),
        "the refusal names the missing module; console:\n{console}"
    );
}
