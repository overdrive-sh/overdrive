//! Native fault matrix of the vsock attachment (GH #295, P6-S14; Roadmap
//! impact 10-02, V-8; R5-10, R5-55 (d), R5-56, R5-62, D25 slot release):
//! faults injected into a real `overdrive serve` + `overdrive deploy`
//! composition on native metal, each against a healthy baseline.
//!
//! Every case first establishes the healthy baseline — a VM Job whose guest
//! application moves bytes both ways with a remote peer (`hold`), or writes
//! as fast as the flow accepts (`flood`), or churns UDP sockets — then
//! injects exactly one fault and observes the contract during and after it:
//!
//! | Fault (stimulus) | Contract (oracle) |
//! |---|---|
//! | `serve` dies (the killed mode of the serve lifetime port; every descriptor of the owner released, as at a `SIGKILL`) | the context's release aborts every host pair: the remote peer's first terminal event is a reset, never EOF; the guest application's is a reset too (0x3 with a direction open, R5-10); the VMM survives the death; the next boot ends it before its first placement and places the Job again on the reclaimed CID (NO-SURVIVE, R5-55 (d)) |
//! | the guest kernel crashes (`/proc/sysrq-trigger`, `panic=1`) | the guest reset ends the VMM (RESET-EXIT); the device release resets the remote peer; the allocation fails `WorkloadCrashedImmediately` and the restart is charged (R5-56) |
//! | the VMM is killed by the host (`SIGKILL`) | the device release resets the remote peer, never EOF; the allocation fails as a crash |
//! | the far reader stops for 10 s | the sender blocks (its written count is observed to plateau within a bound, then stays flat for the 10 s stall), nothing is lost or reordered, and the flow resumes in order when the reader reads again (R5-62) |
//! | 1,100 UDP sockets each send one datagram and close at once — before or after their association pairs | every send succeeds (a slot is released with its socket; leaked slots would refuse the 1,025th with `EPERM`), and a later datagram round trip succeeds (D25, EVMOD-DGRAM) |
//!
//! Not in this matrix (D-9, architect 2026-10-19; § *Roadmap impact* 10-02,
//! V-8):
//!
//! - **Leg-F loss and the `Forwarder` component recovery.** No
//!   production-reachable stimulus exists on the appliance and no test hook
//!   is added to `run_server`. Their evidence is the seeded-sim composed rig
//!   (the owner and supervisor suites, R5-61) plus the Tier-3 kernel effects:
//!   R5-61 through `KernelFlowForwarder`, the `serve`-death cell below
//!   (process exit releases the context), and the worker-level cell
//!   `leg_f_repair_rebinds_transparent_listener` (`overdrive-worker`
//!   integration, real `HostMtlsIntercept`).
//! - **Control-session loss without a VM fault (D-16; the guest-driven 10-02
//!   cell is withdrawn).** Through `serve` + `deploy` the guest's PID 1 is the
//!   production `overdrive-init`, which owns the VM's one control session and
//!   closes it only by exiting — which ends the VM (RESET-EXIT) — so no
//!   production stimulus ends the session without a VM fault, and no entry
//!   point or test-only guest is added to the production VM driver. Its
//!   evidence is the seeded-sim composed rig (the session's close and a
//!   malformed message scripted at the owner's seam, § *Control session*,
//!   "Loss") plus the in-guest session cells on the guest-kit lane.
//!
//! Bounds: one VM per case (two across a restart), each fault observed within
//! its named bound; every case stops its Job and shuts `serve` down; the
//! `TeardownBound` kills what a failed case left. Lane:
//! `integration-tests,kvm-tests`, `cargo xtask metal run --`; module in
//! `host-kernel-shared`. Pending DELIVER step 14-02.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::doc_markdown, reason = "kernel and protocol names in prose")]

use std::time::Duration;

use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::TransitionReason;
use overdrive_core::id::AllocationId;
use overdrive_testing::guest_app::{
    FLOOD_REPORT_PERIOD_MS, FLOWING_LINE, HOLD_FLOWING_BYTES, RemotePeer, TerminalEvent,
    flood_written_in_console, subcommand, terminal_event_in_console,
};
use serial_test::serial;

use super::vm_walking_skeleton::TeardownBound;
use super::vsock_support::{
    Lane, alloc_of, arg, console_of, hypervisor_of, process_alive, vsock_cid, wait_console,
    wait_process_gone,
};

/// How long a fault's effect may take to reach every far end.
const FAR_END_BOUND: Duration = Duration::from_secs(15);
/// Boot to Running.
const RUNNING_BOUND: Duration = Duration::from_secs(120);
/// R5-62's stall: how long the blocked sender must stay blocked.
const STALL_WINDOW: Duration = Duration::from_secs(10);
/// How far apart two equal written-count samples must be to count as a
/// plateau: four of the guest application's report periods
/// (`FLOOD_REPORT_PERIOD_MS`), so a sender still filling the flow shows at
/// least one larger report in between.
const PLATEAU_SPACING: Duration = Duration::from_millis(4 * FLOOD_REPORT_PERIOD_MS);
/// How long the flow may take to fill to its per-flow bound and block.
const PLATEAU_BOUND: Duration = Duration::from_secs(60);

/// Waits until the guest application's written count is non-zero and two
/// samples [`PLATEAU_SPACING`] apart are equal (the sender blocked at the
/// per-flow bound); returns that count. Fails, with every report seen, if no
/// plateau is observed within [`PLATEAU_BOUND`].
async fn flood_plateau(alloc: &AllocationId) -> u64 {
    let deadline = tokio::time::Instant::now() + PLATEAU_BOUND;
    let mut previous: Option<u64> = None;
    loop {
        let reports = flood_written_in_console(&console_of(alloc));
        let latest = reports.last().copied();
        if let (Some(prior), Some(now)) = (previous, latest)
            && now > 0
            && now == prior
        {
            return now;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the sender never blocked within {PLATEAU_BOUND:?}: the written count kept growing \
             (or nothing was written); reports {reports:?}; console:\n{}",
            console_of(alloc)
        );
        previous = latest;
        tokio::time::sleep(PLATEAU_SPACING).await;
    }
}

/// Deploys a `hold` Job tagged `tag` (with `extra` arguments) and waits until
/// its flow moves bytes both ways; returns its id and allocation.
async fn hold_flowing(
    lane: &Lane,
    peer: &RemotePeer,
    tag: &str,
    extra: &[&str],
) -> (String, AllocationId) {
    let tcp = arg(peer.tcp_addr());
    let mut args = vec![subcommand::HOLD, tcp.as_str(), tag];
    args.extend_from_slice(extra);
    let id = lane.deploy_job(tag, &args).await;
    let alloc = lane.running_alloc(&id, RUNNING_BOUND).await;
    let key = tag.to_owned();
    peer.wait_for(Duration::from_secs(30), |log| {
        log.holds
            .get(&key)
            .is_some_and(|h| h.received >= HOLD_FLOWING_BYTES && h.sent >= HOLD_FLOWING_BYTES)
    })
    .unwrap_or_else(|log| panic!("the healthy baseline: bytes flow both ways; peer log {log:#?}"));
    wait_console(&alloc, Duration::from_secs(30), |c| c.contains(FLOWING_LINE))
        .await
        .unwrap_or_else(|c| panic!("the guest application sees bytes both ways; console:\n{c}"));
    (id, alloc)
}

/// The remote peer's first terminal event on the flow keyed `key`.
fn peer_terminal(peer: &RemotePeer, key: &str) -> TerminalEvent {
    let key = key.to_owned();
    let log = peer
        .wait_for(FAR_END_BOUND, |log| log.holds.get(&key).is_some_and(|h| h.terminal.is_some()))
        .unwrap_or_else(|log| panic!("the remote peer observes the flow's end; peer log {log:#?}"));
    let hold = &log.holds[&key];
    assert_eq!(hold.corrupted_at, None, "no corrupted byte before the end");
    hold.terminal.expect("a terminal event")
}

const fn crashed(reason: Option<&TransitionReason>) -> bool {
    matches!(reason, Some(TransitionReason::WorkloadCrashedImmediately { .. }))
}

/// R5-10 (host owner) and R5-55 (d): `serve` dies with a VM's flow moving
/// bytes both ways. The remote peer and the guest application each observe
/// a reset first, never EOF; the VMM outlives `serve`; the next boot ends
/// it before its first placement, and the persisted Job is placed again on
/// the reclaimed CID with a fresh flow.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 14-02"]
async fn the_serve_owners_death_resets_every_far_end_and_the_next_boot_reclaims_the_vm() {
    let _bound = TeardownBound::arm();
    let peer =
        RemotePeer::start("vsock-fm-owner", 1).expect("precondition: the remote peer starts");
    let mut lane = Lane::boot("vsock-fm-owner").await;
    let tag = "fm-owner";
    let (id, alloc) = hold_flowing(&lane, &peer, tag, &[]).await;
    let (vmm_pid, argv) = hypervisor_of(&alloc).expect("the VM's VMM runs");
    let cid = vsock_cid(&argv);

    lane.kill_serve().await;

    assert_eq!(peer_terminal(&peer, tag), TerminalEvent::Reset, "the remote peer reads a reset");
    let console = wait_console(&alloc, FAR_END_BOUND, |c| terminal_event_in_console(c).is_some())
        .await
        .unwrap_or_else(|c| panic!("the guest application observes the end; console:\n{c}"));
    assert_eq!(
        terminal_event_in_console(&console),
        Some(TerminalEvent::Reset),
        "the guest application reads a reset (0x3 with a direction open)"
    );
    assert!(process_alive(vmm_pid), "the VM outlives the owner's death until the next boot");

    lane.reboot_serve().await;

    assert!(
        wait_process_gone(vmm_pid, Duration::from_secs(5)).await,
        "the boot ended the VMM it does not supervise before serving (NO-SURVIVE)"
    );
    let rows = lane
        .wait_rows(&id, RUNNING_BOUND, "a new Running allocation", |rows| {
            rows.iter().any(|r| r.state == AllocStateWire::Running && alloc_of(r) != alloc)
        })
        .await;
    let next = rows
        .iter()
        .find(|r| r.state == AllocStateWire::Running && alloc_of(r) != alloc)
        .map(alloc_of)
        .expect("the re-placed allocation");
    let (_, next_argv) = hypervisor_of(&next).expect("the re-placed VM's VMM runs");
    assert_eq!(vsock_cid(&next_argv), cid, "the next boot places the Job on the reclaimed CID");
    let second = format!("{tag}#2");
    peer.wait_for(Duration::from_secs(30), |log| {
        log.holds.get(&second).is_some_and(|h| h.received >= HOLD_FLOWING_BYTES)
    })
    .unwrap_or_else(|log| panic!("the re-placed VM's flow moves bytes; peer log {log:#?}"));

    lane.stop(&id).await;
    lane.shutdown().await;
}

/// R5-10 (guest) and R5-56: the guest kernel crashes with the flow moving
/// bytes. The guest reset ends the VMM; the device release resets the
/// remote peer, never EOF; the allocation fails `WorkloadCrashedImmediately`
/// and a restart follows.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 14-02"]
async fn a_guest_reset_ends_the_vmm_and_resets_every_far_end() {
    let _bound = TeardownBound::arm();
    let peer =
        RemotePeer::start("vsock-fm-reset", 2).expect("precondition: the remote peer starts");
    let lane = Lane::boot("vsock-fm-reset").await;
    let tag = "fm-reset";
    let (id, alloc) = hold_flowing(&lane, &peer, tag, &["crash-after=1048576"]).await;
    let (vmm_pid, _) = hypervisor_of(&alloc).expect("the VM's VMM runs");

    assert!(
        wait_process_gone(vmm_pid, Duration::from_secs(60)).await,
        "the guest reset ends the VMM (RESET-EXIT); console:\n{}",
        console_of(&alloc)
    );
    assert_eq!(
        peer_terminal(&peer, tag),
        TerminalEvent::Reset,
        "the device release resets the peer"
    );
    let rows = lane
        .wait_rows(&id, Duration::from_secs(60), "the crashed allocation and its restart", |rows| {
            rows.iter().any(|r| alloc_of(r) == alloc && crashed(r.reason.as_ref()))
                && rows.iter().any(|r| alloc_of(r) != alloc)
        })
        .await;
    let first = rows.iter().find(|r| alloc_of(r) == alloc).expect("the first allocation");
    assert_eq!(first.state, AllocStateWire::Failed, "a reset guest's allocation fails: {first:?}");

    lane.stop(&id).await;
    lane.shutdown().await;
}

/// The VMM is killed by the host with the flow moving bytes: the device
/// release resets the remote peer, never EOF; the allocation fails as a
/// crash.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 14-02"]
async fn a_killed_vmm_resets_the_far_end_of_its_flow() {
    let _bound = TeardownBound::arm();
    let peer = RemotePeer::start("vsock-fm-kill", 3).expect("precondition: the remote peer starts");
    let lane = Lane::boot("vsock-fm-kill").await;
    let tag = "fm-kill";
    let (id, alloc) = hold_flowing(&lane, &peer, tag, &[]).await;
    let (vmm_pid, _) = hypervisor_of(&alloc).expect("the VM's VMM runs");

    let pid = i32::try_from(vmm_pid).expect("a pid");
    // SAFETY: a signal to the VMM process this test located.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0, "kill the VMM");

    assert_eq!(
        peer_terminal(&peer, tag),
        TerminalEvent::Reset,
        "the device release resets the peer"
    );
    lane.wait_rows(
        &id,
        Duration::from_secs(60),
        "the killed VM's allocation fails as a crash",
        |rows| {
            rows.iter().any(|r| {
                alloc_of(r) == alloc
                    && r.state == AllocStateWire::Failed
                    && crashed(r.reason.as_ref())
            })
        },
    )
    .await;

    lane.stop(&id).await;
    lane.shutdown().await;
}

/// R5-62: the remote reader stops reading for 10 s. The guest application's
/// writes block (its written count stops growing), nothing is lost or
/// reordered, and the flow resumes in order once the reader reads again.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 14-02"]
async fn a_far_reader_that_stops_blocks_the_sender_and_the_flow_resumes_in_order() {
    let _bound = TeardownBound::arm();
    let peer =
        RemotePeer::start("vsock-fm-stall", 4).expect("precondition: the remote peer starts");
    peer.stall_floods(true);
    let lane = Lane::boot("vsock-fm-stall").await;
    let tag = "fm-stall";
    let id = lane.deploy_job(tag, &[subcommand::FLOOD, &arg(peer.tcp_addr()), tag]).await;
    let alloc = lane.running_alloc(&id, RUNNING_BOUND).await;

    // The flow fills to its bound, then the sender blocks: observed, not
    // assumed — the written count must stop growing within the bound.
    let before = flood_plateau(&alloc).await;
    // R5-62's 10 s stall: the count stays flat for the whole window.
    tokio::time::sleep(STALL_WINDOW).await;
    let reports = flood_written_in_console(&console_of(&alloc));
    let after = reports.last().copied().unwrap_or(0);
    assert_eq!(
        after, before,
        "the sender stays blocked for the {STALL_WINDOW:?} stall: {reports:?}"
    );
    assert!(
        peer.snapshot().floods.get(tag).is_some_and(|f| f.verified == 0),
        "the stalled reader read nothing"
    );

    peer.stall_floods(false);
    let key = tag.to_owned();
    let log = peer
        .wait_for(Duration::from_secs(30), |log| {
            log.floods.get(&key).is_some_and(|f| f.verified > after + (1 << 20))
        })
        .unwrap_or_else(|log| panic!("the flow resumes once the reader reads; peer log {log:#?}"));
    let flood = &log.floods[tag];
    assert_eq!(flood.corrupted_at, None, "every byte in order, none lost");
    assert_eq!(flood.terminal, None, "the stall ended nothing");

    lane.stop(&id).await;
    lane.shutdown().await;
}

/// D25 / EVMOD-DGRAM slot release: 1,100 UDP sockets each send one datagram
/// and close at once, most before their association pairs. Every send
/// succeeds — a slot is released with its socket, so the guest's 1,024 slots
/// are never exhausted — and a later datagram round trip succeeds.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 14-02"]
async fn closed_udp_sockets_release_their_slots_before_and_after_pairing() {
    let _bound = TeardownBound::arm();
    let peer =
        RemotePeer::start("vsock-fm-churn", 5).expect("precondition: the remote peer starts");
    let lane = Lane::boot("vsock-fm-churn").await;
    let tag = "fm-churn";
    let id =
        lane.deploy_job(tag, &[subcommand::UDP_CHURN, &arg(peer.udp_addr()), "1100", tag]).await;
    let rows = lane
        .wait_rows(&id, Duration::from_secs(180), "a terminal allocation", |rows| {
            rows.first().is_some_and(|r| {
                matches!(r.state, AllocStateWire::Terminated | AllocStateWire::Failed)
            })
        })
        .await;
    let row = rows.first().expect("one allocation");
    assert_eq!(
        row.state,
        AllocStateWire::Terminated,
        "every churn send succeeded and the round trip came back; console:\n{}",
        console_of(&alloc_of(row))
    );
    let log = peer.snapshot();
    assert_eq!(log.datagrams.iter().filter(|d| d.tag == tag).count(), 1, "the round trip, once");
    assert!(log.churn.get(tag).copied().unwrap_or(0) <= 1100, "no datagram duplicated");

    lane.shutdown().await;
}
