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
//! **The owner lane (D-22).** R5-28 / R5-31's Tier-3 half boots a guest whose
//! PID 1 is the scripted peer and runs one `guest_flow::real_kernel` owner
//! cell in it through `overdrive_testing::guest_lane::run_in_guest_owner_cell`:
//! the cell's production `GuestFlowOwner` is the guest's only owner, and the
//! case's closure is its scripted host peer over vhost-vsock (V-24's host
//! script, plus the control session). Both the cell and the host's record of
//! the control session must pass.
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

// ---------------------------------------------------------------------------
// D-22: R5-28 / R5-31's Tier-3 half — the owner lane
// ---------------------------------------------------------------------------
//
// Each case boots a guest whose PID 1 is the scripted peer and runs one
// `guest_flow::real_kernel` owner cell in it
// (`overdrive_testing::guest_lane::run_in_guest_owner_cell`): the cell's
// production `GuestFlowOwner` is the guest's only owner. The closure below is
// the scripted host peer over the real vhost-vsock — V-24's host script: it
// opens V_h to the guest's inbound port, writes the `TcpAccept` request and
// nothing else, and closes V_h in the order the cell names; it plays the
// control session. Its oracle (D-22 (3)) is the record of the control
// session: the guest writes no message for the aborted flow (a late `Paired`
// is allowed only where R5-28 names it, and is answered with nothing), and
// the positive control flow's `Paired` arrives. Both the cell's run and the
// host's record must pass.

use std::time::Duration;

use overdrive_core::guest_transport::wire::{ControlMessage, FlowId, FlowOrigin};
use overdrive_testing::guest_lane::{
    OwnerLaneHost, control_messages_within, owner_lane, run_in_guest_owner_cell, send_control,
};
use overdrive_testing::vsock::VsockStream;

/// Every host-side wait's bound.
const HOST_BOUND: Duration = Duration::from_secs(60);
/// The quiet window that ends a read of the guest's control messages.
const SESSION_QUIET: Duration = Duration::from_millis(500);
/// The owner lane guest's RAM (the burst holds 1,000 pairs).
const OWNER_LANE_MIB: u32 = 512;

fn host_flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Host, seq).expect("a small host-origin sequence")
}

const fn flow_of(message: &ControlMessage) -> FlowId {
    match *message {
        ControlMessage::Paired { flow }
        | ControlMessage::Refused { flow, .. }
        | ControlMessage::Abort { flow }
        | ControlMessage::TimedOut { flow, .. } => flow,
    }
}

/// Waits for the cell's `line` on the script channel.
fn expect_line(host: &mut OwnerLaneHost, line: &str) -> Result<(), String> {
    let heard =
        host.script.hear(line, HOST_BOUND).map_err(|e| format!("waiting for {line}: {e}"))?;
    if heard == line { Ok(()) } else { Err(format!("expected {line}, the cell said {heard}")) }
}

fn say(host: &OwnerLaneHost, line: &str) -> Result<(), String> {
    host.script.say(line).map_err(|e| format!("saying {line}: {e}"))
}

/// Accepts the owner's control session and tells the cell.
fn open_session(host: &OwnerLaneHost) -> Result<VsockStream, String> {
    let session = host.accept_session(HOST_BOUND).map_err(|e| format!("control session: {e}"))?;
    say(host, owner_lane::SESSION_ACCEPTED)?;
    Ok(session)
}

/// The guest's control messages on `session` so far.
fn drain(session: &VsockStream) -> Result<Vec<ControlMessage>, String> {
    control_messages_within(session, SESSION_QUIET).map_err(|e| format!("control session: {e}"))
}

/// D-22 (3): no message of the guest names `flow`.
fn none_for(messages: &[ControlMessage], flow: FlowId, when: &str) -> Result<(), String> {
    messages
        .iter()
        .find(|m| flow_of(m) == flow)
        .map_or(Ok(()), |m| Err(format!("{when}: the guest wrote {m:?} for the aborted flow")))
}

/// The positive control flow (the cell's `serve_control_flow`): `V_h` stays
/// open until the cell reports it paired; its `Paired` arrives on
/// `session` (the session record has teeth); then `V_h` closes with no write.
fn serve_control_flow(host: &mut OwnerLaneHost, session: &VsockStream) -> Result<(), String> {
    let v_h = host
        .open_tcp_accept(owner_lane::CONTROL_FLOW, owner_lane::APP_PORT)
        .map_err(|e| format!("control flow V_h: {e}"))?;
    say(host, owner_lane::REQUESTED)?;
    expect_line(host, owner_lane::PAIRED)?;
    let messages = drain(session)?;
    let control = host_flow(owner_lane::CONTROL_FLOW);
    if !messages.contains(&ControlMessage::Paired { flow: control }) {
        return Err(format!("the control flow's Paired did not arrive: {messages:?}"));
    }
    drop(v_h);
    say(host, owner_lane::CLOSED)
}

/// Waits for the cell's `DONE` and checks the session's remainder carries no
/// message for the aborted flow.
fn finish(host: &mut OwnerLaneHost, session: &VsockStream) -> Result<(), String> {
    expect_line(host, owner_lane::DONE)?;
    none_for(&drain(session)?, host_flow(owner_lane::ABORTED_FLOW), "after the cell's checks")
}

/// Boots the owner lane at the band's `slot`, runs `cell` with `host` as its
/// scripted host peer, and requires both to pass.
fn run_owner_cell(
    slot: u32,
    cell: &str,
    host: impl FnOnce(&mut OwnerLaneHost) -> Result<(), String>,
) {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("precondition: the guest kit provisions");
    let tests = InGuestTests::prepared("overdrive-init", "overdrive-init")
        .expect("precondition: the in-guest cell binary is staged (cargo xtask guest-kit-prepare)");
    let work = WorkDir::create(&staging, "init-owner-lane").expect("work dir");
    let (run, host_result) = run_in_guest_owner_cell(
        &kit,
        &tests,
        cell,
        INIT_IN_GUEST_LANE.raw(slot),
        work.path(),
        OWNER_LANE_MIB,
        host,
    )
    .expect("the owner lane boots the guest and runs the cell");
    assert!(
        run.passed(),
        "the in-guest owner cell {cell} must run and pass (exit {}); host: {host_result:?}; guest \
         console:\n{}",
        run.exit_status,
        run.console
    );
    if let Err(e) = host_result {
        panic!("the host peer's record of {cell} fails: {e}; guest console:\n{}", run.console);
    }
}

/// D-22 (4): V-24's kernel shape re-witnessed on the lane's kernel, in both
/// accept orders (V_g still queued; V_g accepted before the host writes and
/// closes).
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_v24_kernel_shape_holds_in_a_real_guest() {
    run_owner_cell(
        5,
        "guest_flow::real_kernel::the_v24_kernel_shape_holds_in_both_accept_orders",
        |host| {
            expect_line(host, owner_lane::LISTENING)?;
            let v_h = host
                .open_tcp_accept_on(
                    owner_lane::RAW_PORT,
                    owner_lane::ABORTED_FLOW,
                    owner_lane::APP_PORT,
                )
                .map_err(|e| format!("queued V_h: {e}"))?;
            drop(v_h);
            say(host, owner_lane::RAW_QUEUED)?;
            let v_h =
                host.connect_raw(owner_lane::RAW_PORT).map_err(|e| format!("raw V_h: {e}"))?;
            say(host, owner_lane::RAW_CONNECTED)?;
            expect_line(host, owner_lane::RAW_ACCEPTED)?;
            let request = host
                .tcp_accept_request(owner_lane::ABORTED_FLOW, owner_lane::APP_PORT)
                .map_err(|e| format!("request: {e}"))?;
            std::io::Write::write_all(&mut &v_h, &request).map_err(|e| format!("write: {e}"))?;
            drop(v_h);
            say(host, owner_lane::CLOSED)?;
            expect_line(host, owner_lane::DONE)
        },
    );
}

/// R5-31, quiescence shape (host script): with the owner paused, open V_h,
/// write the request, close V_h (V_g still queued); then the positive
/// control flow. The guest writes no message for the aborted flow.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_host_abort_before_the_guest_accepts_ends_the_flow_in_a_real_guest() {
    run_owner_cell(
        6,
        "guest_flow::real_kernel::a_flow_aborted_before_its_v_g_is_accepted_ends_with_no_application_connection",
        |host| {
            let session = open_session(host)?;
            expect_line(host, owner_lane::PAUSED)?;
            let v_h = host
                .open_tcp_accept(owner_lane::ABORTED_FLOW, owner_lane::APP_PORT)
                .map_err(|e| format!("V_h: {e}"))?;
            drop(v_h);
            say(host, owner_lane::QUEUED)?;
            serve_control_flow(host, &session)?;
            let control = drain(&session)?;
            none_for(&control, host_flow(owner_lane::ABORTED_FLOW), "before the cell's end")?;
            finish(host, &session)
        },
    );
}

/// R5-31, session-loss shape (host script): with the owner paused, open,
/// request and close V_h, then close the control session; accept the
/// owner's reconnect. Neither session carries a message for the aborted
/// flow; the positive control flow pairs on the new session.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_session_loss_abort_ends_the_flow_and_the_owner_reconnects_in_a_real_guest() {
    run_owner_cell(
        7,
        "guest_flow::real_kernel::a_flow_aborted_by_session_loss_ends_and_the_session_is_held_again",
        |host| {
            let first = open_session(host)?;
            expect_line(host, owner_lane::PAUSED)?;
            let v_h = host
                .open_tcp_accept(owner_lane::ABORTED_FLOW, owner_lane::APP_PORT)
                .map_err(|e| format!("V_h: {e}"))?;
            drop(v_h);
            none_for(&drain(&first)?, host_flow(owner_lane::ABORTED_FLOW), "on the first session")?;
            drop(first);
            say(host, owner_lane::QUEUED_THEN_SESSION_CLOSED)?;
            let second = open_session(host)?;
            serve_control_flow(host, &second)?;
            finish(host, &second)
        },
    );
}

/// R5-28, the guest-accepted order (host script): open V_h and write the
/// request, keep it open until the cell reports the flow accepted, then
/// close it with no write. A late `Paired` for the flow is answered with
/// nothing (the host writes no control message); any other message for the
/// flow fails.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_host_close_after_the_guest_accepted_ends_the_flow_in_a_real_guest() {
    run_owner_cell(
        8,
        "guest_flow::real_kernel::a_flow_the_host_closes_after_the_guest_accepted_it_ends_on_the_guest",
        |host| {
            let session = open_session(host)?;
            let v_h = host
                .open_tcp_accept(owner_lane::ABORTED_FLOW, owner_lane::APP_PORT)
                .map_err(|e| format!("V_h: {e}"))?;
            say(host, owner_lane::REQUESTED)?;
            expect_line(host, owner_lane::ACCEPTED)?;
            drop(v_h);
            say(host, owner_lane::CLOSED)?;
            let aborted = host_flow(owner_lane::ABORTED_FLOW);
            let before_control = drain(&session)?;
            let other: Vec<_> = before_control
                .iter()
                .filter(|m| {
                    flow_of(m) == aborted && **m != ControlMessage::Paired { flow: aborted }
                })
                .collect();
            if !other.is_empty() {
                return Err(format!(
                    "the guest wrote more than a late Paired for the flow: {other:?}"
                ));
            }
            serve_control_flow(host, &session)?;
            expect_line(host, owner_lane::DONE)?;
            let rest = drain(&session)?;
            rest.iter().find(|m| flow_of(m) == aborted).map_or(Ok(()), |m| {
                Err(format!("after the cell's checks the guest wrote {m:?} for the flow"))
            })
        },
    );
}

/// R5-28, the not-yet-held order (host script): send `Abort` for a flow the
/// guest does not hold; then, with the owner paused, open, request and close
/// that flow's V_h. The guest replies nothing to the `Abort` and writes no
/// message for the flow.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn an_abort_for_an_unheld_flow_is_discarded_in_a_real_guest() {
    run_owner_cell(
        9,
        "guest_flow::real_kernel::an_abort_for_a_flow_the_guest_does_not_hold_changes_nothing_and_the_flow_ends_on_the_close",
        |host| {
            let session = open_session(host)?;
            send_control(
                &session,
                &ControlMessage::Abort { flow: host_flow(owner_lane::ABORTED_FLOW) },
            )
            .map_err(|e| format!("Abort: {e}"))?;
            say(host, owner_lane::ABORT_SENT)?;
            expect_line(host, owner_lane::PAUSED)?;
            none_for(&drain(&session)?, host_flow(owner_lane::ABORTED_FLOW), "after the Abort")?;
            let v_h = host
                .open_tcp_accept(owner_lane::ABORTED_FLOW, owner_lane::APP_PORT)
                .map_err(|e| format!("V_h: {e}"))?;
            drop(v_h);
            say(host, owner_lane::QUEUED)?;
            serve_control_flow(host, &session)?;
            finish(host, &session)
        },
    );
}

/// R5-31's burst (host script): open `BURST` flows and keep them open until
/// the cell reports every one paired, then close every V_h at once with no
/// write. The cell reports the abort-to-reset p50 / p99 (not asserted).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_burst_of_host_aborts_resets_every_connection_in_a_real_guest() {
    run_owner_cell(
        10,
        "guest_flow::real_kernel::a_burst_of_host_aborts_resets_every_application_connection",
        |host| {
            let session = open_session(host)?;
            let mut v_hs = Vec::with_capacity(owner_lane::BURST as usize);
            for seq in owner_lane::BURST_FIRST..owner_lane::BURST_FIRST + owner_lane::BURST {
                v_hs.push(
                    host.open_tcp_accept(seq, owner_lane::APP_PORT)
                        .map_err(|e| format!("burst V_h {seq}: {e}"))?,
                );
            }
            say(host, owner_lane::REQUESTED)?;
            expect_line(host, owner_lane::ALL_PAIRED)?;
            let paired = drain(&session)?;
            let missing = (owner_lane::BURST_FIRST..owner_lane::BURST_FIRST + owner_lane::BURST)
                .filter(|seq| !paired.contains(&ControlMessage::Paired { flow: host_flow(*seq) }))
                .count();
            if missing != 0 {
                return Err(format!("{missing} burst flows' Paired did not arrive"));
            }
            drop(v_hs);
            say(host, owner_lane::CLOSED_ALL)?;
            expect_line(host, owner_lane::DONE)
        },
    );
}
