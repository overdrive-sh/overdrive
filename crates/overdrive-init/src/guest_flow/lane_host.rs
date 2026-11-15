//! The host-side scripts of the owner-lane cells in `real_kernel` (D-22).
//!
//! Each owner-lane cell boots a guest whose PID 1 is the scripted peer and runs
//! one `guest_flow::real_kernel` owner cell in it; the script here is the
//! cell's scripted host peer over the real vhost-vsock (V-24's host script,
//! plus the control session). A cell names its script on its own attribute
//! (`#[overdrive_test(lane_owner = "<fixture>", host = lane_host::<script>)]`),
//! and the `integration::in_guest_lane` fixture that drives the same cell from
//! inside a guest calls the same function, so each script exists once.
//!
//! **Oracle.** The record of the control session: the guest writes no message
//! for the aborted flow (a late `Paired` is allowed only where R5-28 names it,
//! and is answered with nothing), and the positive control flow's `Paired`
//! arrives. Both the cell's run and the host's record must pass.
//!
//! The file is self-contained (no `crate::` paths) because the integration
//! fixtures include it by `#[path]`.

use std::io::Write as _;
use std::time::Duration;

use overdrive_core::guest_transport::wire::{ControlMessage, FlowId, FlowOrigin};
use overdrive_testing::guest_lane::{
    OwnerLaneHost, control_messages_within, owner_lane, send_control,
};
use overdrive_testing::vsock::VsockStream;

/// Every host-side wait's bound.
const HOST_BOUND: Duration = Duration::from_secs(60);
/// The quiet window that ends a read of the guest's control messages.
const SESSION_QUIET: Duration = Duration::from_millis(500);

fn host_flow(sequence: u32) -> Result<FlowId, String> {
    FlowId::new(FlowOrigin::Host, sequence)
        .map_err(|error| format!("host-origin flow {sequence}: {error:?}"))
}

fn aborted_flow() -> Result<FlowId, String> {
    host_flow(owner_lane::ABORTED_FLOW)
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
    let heard = host
        .script
        .hear(line, HOST_BOUND)
        .map_err(|error| format!("waiting for {line}: {error}"))?;
    if heard == line { Ok(()) } else { Err(format!("expected {line}, the cell said {heard}")) }
}

fn say(host: &OwnerLaneHost, line: &str) -> Result<(), String> {
    host.script.say(line).map_err(|error| format!("saying {line}: {error}"))
}

/// Accepts the owner's control session and tells the cell.
fn open_session(host: &OwnerLaneHost) -> Result<VsockStream, String> {
    let session =
        host.accept_session(HOST_BOUND).map_err(|error| format!("control session: {error}"))?;
    say(host, owner_lane::SESSION_ACCEPTED)?;
    Ok(session)
}

/// The guest's control messages on `session` so far.
fn drain(session: &VsockStream) -> Result<Vec<ControlMessage>, String> {
    control_messages_within(session, SESSION_QUIET)
        .map_err(|error| format!("control session: {error}"))
}

/// D-22 (3): no message of the guest names `flow`.
fn none_for(messages: &[ControlMessage], flow: FlowId, when: &str) -> Result<(), String> {
    messages.iter().find(|message| flow_of(message) == flow).map_or(Ok(()), |message| {
        Err(format!("{when}: the guest wrote {message:?} for the aborted flow"))
    })
}

/// The positive control flow (the cell's `serve_control_flow`): `V_h` stays
/// open until the cell reports it paired; its `Paired` arrives on `session`
/// (the session record has teeth); then `V_h` closes with no write.
fn serve_control_flow(host: &mut OwnerLaneHost, session: &VsockStream) -> Result<(), String> {
    let v_h = host
        .open_tcp_accept(owner_lane::CONTROL_FLOW, owner_lane::APP_PORT)
        .map_err(|error| format!("control flow V_h: {error}"))?;
    say(host, owner_lane::REQUESTED)?;
    expect_line(host, owner_lane::PAIRED)?;
    let messages = drain(session)?;
    let control = host_flow(owner_lane::CONTROL_FLOW)?;
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
    none_for(&drain(session)?, aborted_flow()?, "after the cell's checks")
}

/// D-22 (4): V-24's kernel shape re-witnessed on the lane's kernel, in both
/// accept orders (V_g still queued; V_g accepted before the host writes and
/// closes).
///
/// # Errors
///
/// The step of the script that failed.
pub fn v24_kernel_shape(host: &mut OwnerLaneHost) -> Result<(), String> {
    expect_line(host, owner_lane::LISTENING)?;
    let v_h = host
        .open_tcp_accept_on(owner_lane::RAW_PORT, owner_lane::ABORTED_FLOW, owner_lane::APP_PORT)
        .map_err(|error| format!("queued V_h: {error}"))?;
    drop(v_h);
    say(host, owner_lane::RAW_QUEUED)?;
    let v_h =
        host.connect_raw(owner_lane::RAW_PORT).map_err(|error| format!("raw V_h: {error}"))?;
    say(host, owner_lane::RAW_CONNECTED)?;
    expect_line(host, owner_lane::RAW_ACCEPTED)?;
    let request = host
        .tcp_accept_request(owner_lane::ABORTED_FLOW, owner_lane::APP_PORT)
        .map_err(|error| format!("request: {error}"))?;
    (&v_h).write_all(&request).map_err(|error| format!("write: {error}"))?;
    drop(v_h);
    say(host, owner_lane::CLOSED)?;
    expect_line(host, owner_lane::DONE)
}

/// R5-31, quiescence shape: with the owner paused, open V_h, write the
/// request, close V_h (V_g still queued); then the positive control flow. The
/// guest writes no message for the aborted flow.
///
/// # Errors
///
/// The step of the script that failed.
pub fn abort_before_accept(host: &mut OwnerLaneHost) -> Result<(), String> {
    let session = open_session(host)?;
    expect_line(host, owner_lane::PAUSED)?;
    let v_h = host
        .open_tcp_accept(owner_lane::ABORTED_FLOW, owner_lane::APP_PORT)
        .map_err(|error| format!("V_h: {error}"))?;
    drop(v_h);
    say(host, owner_lane::QUEUED)?;
    serve_control_flow(host, &session)?;
    none_for(&drain(&session)?, aborted_flow()?, "before the cell's end")?;
    finish(host, &session)
}

/// R5-31, session-loss shape: with the owner paused, open, request and close
/// V_h, then close the control session; accept the owner's reconnect. Neither
/// session carries a message for the aborted flow; the positive control flow
/// pairs on the new session.
///
/// # Errors
///
/// The step of the script that failed.
pub fn session_loss(host: &mut OwnerLaneHost) -> Result<(), String> {
    let first = open_session(host)?;
    expect_line(host, owner_lane::PAUSED)?;
    let v_h = host
        .open_tcp_accept(owner_lane::ABORTED_FLOW, owner_lane::APP_PORT)
        .map_err(|error| format!("V_h: {error}"))?;
    drop(v_h);
    none_for(&drain(&first)?, aborted_flow()?, "on the first session")?;
    drop(first);
    say(host, owner_lane::QUEUED_THEN_SESSION_CLOSED)?;
    let second = open_session(host)?;
    serve_control_flow(host, &second)?;
    finish(host, &second)
}

/// R5-28, the guest-accepted order: open V_h and write the request, keep it
/// open until the cell reports the flow accepted, then close it with no write.
/// A late `Paired` for the flow is answered with nothing (the host writes no
/// control message); any other message for the flow fails. The host waits until
/// the cell has observed the aborted C before opening the positive control, so
/// the two application connections cannot be mistaken for one another.
///
/// # Errors
///
/// The step of the script that failed.
pub fn close_after_accept(host: &mut OwnerLaneHost) -> Result<(), String> {
    let session = open_session(host)?;
    let v_h = host
        .open_tcp_accept(owner_lane::ABORTED_FLOW, owner_lane::APP_PORT)
        .map_err(|error| format!("V_h: {error}"))?;
    say(host, owner_lane::REQUESTED)?;
    expect_line(host, owner_lane::ACCEPTED)?;
    drop(v_h);
    say(host, owner_lane::CLOSED)?;
    let aborted = aborted_flow()?;
    let before_control = drain(&session)?;
    let other: Vec<_> = before_control
        .iter()
        .filter(|message| {
            flow_of(message) == aborted && **message != ControlMessage::Paired { flow: aborted }
        })
        .collect();
    if !other.is_empty() {
        return Err(format!("the guest wrote more than a late Paired for the flow: {other:?}"));
    }
    expect_line(host, owner_lane::PAUSED)?;
    serve_control_flow(host, &session)?;
    expect_line(host, owner_lane::DONE)?;
    drain(&session)?.iter().find(|message| flow_of(message) == aborted).map_or(Ok(()), |message| {
        Err(format!("after the cell's checks the guest wrote {message:?} for the flow"))
    })
}

/// R5-28, the not-yet-held order: send `Abort` for a flow the guest does not
/// hold; then, with the owner paused, open, request and close that flow's V_h.
/// The guest replies nothing to the `Abort` and writes no message for the flow.
///
/// # Errors
///
/// The step of the script that failed.
pub fn unheld_abort(host: &mut OwnerLaneHost) -> Result<(), String> {
    let session = open_session(host)?;
    send_control(&session, &ControlMessage::Abort { flow: aborted_flow()? })
        .map_err(|error| format!("Abort: {error}"))?;
    say(host, owner_lane::ABORT_SENT)?;
    expect_line(host, owner_lane::PAUSED)?;
    none_for(&drain(&session)?, aborted_flow()?, "after the Abort")?;
    let v_h = host
        .open_tcp_accept(owner_lane::ABORTED_FLOW, owner_lane::APP_PORT)
        .map_err(|error| format!("V_h: {error}"))?;
    drop(v_h);
    say(host, owner_lane::QUEUED)?;
    serve_control_flow(host, &session)?;
    finish(host, &session)
}

/// R5-31's burst: open `BURST` flows and keep them open until the cell reports
/// every one paired, then close every V_h at once with no write. The cell
/// reports the abort-to-reset p50 / p99 (not asserted).
///
/// # Errors
///
/// The step of the script that failed.
pub fn burst_abort(host: &mut OwnerLaneHost) -> Result<(), String> {
    let session = open_session(host)?;
    let mut v_hs = Vec::with_capacity(owner_lane::BURST as usize);
    for seq in owner_lane::BURST_FIRST..owner_lane::BURST_FIRST + owner_lane::BURST {
        v_hs.push(
            host.open_tcp_accept(seq, owner_lane::APP_PORT)
                .map_err(|error| format!("burst V_h {seq}: {error}"))?,
        );
    }
    say(host, owner_lane::REQUESTED)?;
    expect_line(host, owner_lane::ALL_PAIRED)?;
    let paired = drain(&session)?;
    let mut missing = 0_usize;
    for seq in owner_lane::BURST_FIRST..owner_lane::BURST_FIRST + owner_lane::BURST {
        if !paired.contains(&ControlMessage::Paired { flow: host_flow(seq)? }) {
            missing += 1;
        }
    }
    if missing != 0 {
        return Err(format!("{missing} burst flows' Paired did not arrive"));
    }
    drop(v_hs);
    say(host, owner_lane::CLOSED_ALL)?;
    expect_line(host, owner_lane::DONE)
}

/// TCP-CORRELATION: the cell owns every identity, one-time-take,
/// direct-refusal and transient-cleanup oracle; the host only waits for the
/// cell to finish.
///
/// # Errors
///
/// The cell did not report `DONE`.
pub fn tcp_correlation(host: &mut OwnerLaneHost) -> Result<(), String> {
    expect_line(host, owner_lane::DONE)
}
