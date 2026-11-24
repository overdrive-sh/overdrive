//! Node-local persistent volume (GH #348, ADR-0348, ADR-0349, ADR-0351): the contract of the
//! stuck-VM test-support hooks on the two sim adapters.
//!
//! `SimVmm` models a process the kernel cannot reap (`inject_unreapable_process`, `is_unreaped`,
//! the scripted reap `exit_process`). `SimVmHostState` models a scope the host cannot remove
//! (`hold_scope_while`, `release_scope_hold`) and a failing observation (`script_observe_failure`).
//! Every test here drives the hook through the port trait it stands in for and asserts on the
//! port's own observables, so the tests that use these hooks (driver, shim, reclamation, boot)
//! rest on a pinned double.
//!
//! The hooks exist, so every test is a guard and carries no pending marker. The unreaped
//! process's scope membership has no `SimVmm` accessor, so it is not asserted here.
//!
//! Evidence tier: in-memory acceptance (layer 2) on a logical clock. No real I/O beyond a tempdir.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::poll;
use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::guest_transport::{GUEST_CID_BASE, GuestCid};
use overdrive_core::traits::vm_host_state::{VmHostState, VmHostStateProbeError};
use overdrive_core::traits::vmm::{VMM_REAP_BOUND, VmProcess, VmTermination, Vmm};
use overdrive_core::vm::config::{
    Gid, HostArch, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan, VmConfig, VmConfinement,
    VmRunDir, VmmIdentity,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::{SimVmHostState, SimVmm};
use overdrive_testing::placement;

placement!(host_pure);

const NANOSECOND: Duration = Duration::from_nanos(1);

fn cid(offset: u32) -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + offset).expect("an in-range guest CID")
}

fn alloc(name: &str) -> AllocationId {
    AllocationId::new(name).expect("a valid allocation id")
}

/// A launch config whose rootfs master is a real file under `root` (the sim clones it).
fn config(root: &Path, name: &str) -> VmConfig {
    let alloc = alloc(name);
    let master = root.join(format!("{alloc}-master.img"));
    std::fs::write(&master, [0_u8; 512]).expect("write the rootfs master");
    let kernel = KernelImage::validate(root.join("vmlinux"), HostArch::X86_64, b"\x7fELF")
        .expect("an ELF header validates");
    VmConfig {
        volume: None,
        alloc: alloc.clone(),
        kernel,
        rootfs: RootfsPlan::for_alloc(master, 512, &alloc, root, &root.join("clone-index")),
        cmdline: KernelCmdline::platform_default(HostArch::X86_64),
        memory: MemoryPlan::derive(128 * 1024 * 1024),
        vcpus: NonZeroU8::new(1).expect("nonzero"),
        run_dir: VmRunDir::for_alloc(root, &alloc),
        confinement: VmConfinement::confined(
            VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
            1024,
        ),
        cgroup_scope: CgroupPath::for_alloc(&alloc),
    }
}

/// A `SimVmm` on a logical clock the test drives, and the clock.
fn sim_vmm() -> (SimVmm, SimClock) {
    let clock = SimClock::new();
    (SimVmm::new(Arc::new(clock.clone())), clock)
}

/// One unreapable launch: the process the driver cannot reap after its kill.
async fn launch_unreapable(vmm: &SimVmm, root: &Path, name: &str, offset: u32) -> VmProcess {
    vmm.inject_unreapable_process();
    vmm.create(&config(root, name), cid(offset)).await.expect("the sim launch succeeds")
}

/// Drives `terminate` on the logical clock: it is pending one nanosecond before the reap bound and
/// resolved exactly at it. Returns the outcome.
async fn terminate_across_exactly_the_reap_bound(
    vmm: &SimVmm,
    clock: &SimClock,
    process: &VmProcess,
) -> VmTermination {
    let mut terminate = Box::pin(vmm.terminate(&process.control, Duration::ZERO));
    assert!(
        poll!(&mut terminate).is_pending(),
        "terminate on an unreapable process must wait for the reap, not return at once"
    );
    clock.tick(VMM_REAP_BOUND.saturating_sub(NANOSECOND));
    assert!(
        poll!(&mut terminate).is_pending(),
        "terminate must still be waiting one nanosecond before the reap bound"
    );
    clock.tick(NANOSECOND);
    match poll!(&mut terminate) {
        std::task::Poll::Ready(outcome) => outcome.expect("terminate is Ok when the kill landed"),
        std::task::Poll::Pending => {
            panic!("terminate must resolve exactly at the reap bound on the injected clock")
        }
    }
}

// ---------------------------------------------------------------------
// SimVmm: the unreapable process and its scripted reap
// ---------------------------------------------------------------------

/// ADR-0348 item 5. Owner path: `Vmm::terminate`. Fault: the next launch is unreapable. Oracle: the
/// outcome, the clock, and the process's liveness observables before and after.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_kill_of_an_unreapable_process_reports_not_reaped_after_exactly_the_reap_bound() {
    let root = tempfile::tempdir().expect("tempdir");
    let (vmm, clock) = sim_vmm();
    let process = launch_unreapable(&vmm, root.path(), "stuck-bound", 1).await;
    let pid = process.control.pid;
    assert!(vmm.is_live(pid), "precondition: the process is live before the kill");
    assert!(!vmm.is_unreaped(pid), "precondition: a live process is not unreaped");

    let outcome = terminate_across_exactly_the_reap_bound(&vmm, &clock, &process).await;

    assert_eq!(outcome, VmTermination::KilledNotReaped);
    assert!(!vmm.is_live(pid), "a killed process is not live, reaped or not");
    assert!(!vmm.live_pids().contains(&pid), "a killed process is omitted from the live pids");
    assert!(vmm.is_unreaped(pid), "a killed process that was not reaped is unreaped");
}

/// ADR-0348 item 5, RB-1: the unreaped process keeps what it holds and its exit watch stays
/// silent. Oracle: `vsock_device_holders` and the exit watch.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn an_unreaped_process_keeps_its_vsock_device_and_its_exit_watch_stays_silent() {
    let root = tempfile::tempdir().expect("tempdir");
    let (vmm, clock) = sim_vmm();
    let mut process = launch_unreapable(&vmm, root.path(), "stuck-holds", 2).await;
    let pid = process.control.pid;

    let outcome = terminate_across_exactly_the_reap_bound(&vmm, &clock, &process).await;

    assert_eq!(outcome, VmTermination::KilledNotReaped);
    assert_eq!(
        vmm.vsock_device_holders(),
        BTreeMap::from([(cid(2), pid)]),
        "the unreaped process still holds its CID's device"
    );
    assert!(
        poll!(Box::pin(process.exit.recv())).is_pending(),
        "an unreaped process must not report an ending: its exit watch fires only at the reap"
    );
}

/// ADR-0348 item 5: `exit_process` on an unreaped process IS the reap. Oracle: the unreaped state,
/// the device holders, the exit watch (once), and the return value.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_scripted_reap_releases_the_device_and_fires_the_exit_watch_once() {
    let root = tempfile::tempdir().expect("tempdir");
    let (vmm, clock) = sim_vmm();
    let mut process = launch_unreapable(&vmm, root.path(), "stuck-reap", 3).await;
    let pid = process.control.pid;
    terminate_across_exactly_the_reap_bound(&vmm, &clock, &process).await;

    let reaped = vmm.exit_process(pid, None, Some(9));

    assert!(reaped, "exit_process on an unreaped process is the reap and reports it");
    assert!(!vmm.is_unreaped(pid), "the reap clears the unreaped state");
    assert!(!vmm.is_live(pid), "a reaped process is not live");
    assert!(vmm.vsock_device_holders().is_empty(), "the reap releases the device");
    let ending = process.exit.recv().await.expect("the reap fires the exit watch once");
    assert_eq!((ending.exit_code, ending.signal), (None, Some(9)), "the given ending is reported");
    assert!(
        !vmm.exit_process(pid, Some(0), None),
        "a reaped process cannot be reaped or exited again"
    );
}

/// ADR-0348 item 1: a second `terminate` on an unreaped process waits the bound again and reports
/// the same outcome; nothing is released by asking twice.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_second_terminate_on_an_unreaped_process_waits_the_bound_again() {
    let root = tempfile::tempdir().expect("tempdir");
    let (vmm, clock) = sim_vmm();
    let process = launch_unreapable(&vmm, root.path(), "stuck-twice", 4).await;
    let pid = process.control.pid;
    let first = terminate_across_exactly_the_reap_bound(&vmm, &clock, &process).await;
    assert_eq!(first, VmTermination::KilledNotReaped);

    let second = terminate_across_exactly_the_reap_bound(&vmm, &clock, &process).await;

    assert_eq!(second, VmTermination::KilledNotReaped, "the same outcome while still unreaped");
    assert!(vmm.is_unreaped(pid), "asking twice does not reap");
    assert_eq!(vmm.vsock_device_holders().len(), 1, "asking twice does not release the device");
}

/// The scripted reap accepts only a process that is live or unreaped. Oracle: the return value.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_scripted_reap_refuses_an_unknown_or_already_gone_process() {
    let root = tempfile::tempdir().expect("tempdir");
    let (vmm, _clock) = sim_vmm();
    assert!(!vmm.exit_process(42, Some(0), None), "an unknown pid is not reaped");

    let ordinary = vmm
        .create(&config(root.path(), "stuck-ordinary"), cid(5))
        .await
        .expect("the sim launch succeeds");
    vmm.terminate(&ordinary.control, Duration::ZERO).await.expect("an ordinary terminate");
    assert!(
        !vmm.exit_process(ordinary.control.pid, Some(0), None),
        "a process already reaped by an ordinary terminate is not reaped again"
    );
}

/// The unreapable arming is one-shot and the reapable path is unchanged. Oracle: the second
/// launch's `terminate` resolves at once with `Killed` and releases its device.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_unreapable_arming_is_consumed_by_one_launch_and_an_ordinary_launch_reaps_at_once() {
    let root = tempfile::tempdir().expect("tempdir");
    let (vmm, _clock) = sim_vmm();
    let stuck = launch_unreapable(&vmm, root.path(), "stuck-armed", 6).await;
    let ordinary = vmm
        .create(&config(root.path(), "stuck-unarmed"), cid(7))
        .await
        .expect("the sim launch succeeds");

    let outcome = {
        let mut terminate = Box::pin(vmm.terminate(&ordinary.control, Duration::ZERO));
        match poll!(&mut terminate) {
            std::task::Poll::Ready(outcome) => outcome.expect("terminate is Ok"),
            std::task::Poll::Pending => {
                panic!("an ordinary process is reaped at once and waits for no bound")
            }
        }
    };

    assert_eq!(outcome, VmTermination::Killed);
    assert!(!vmm.is_unreaped(ordinary.control.pid), "an ordinary kill is a reap");
    assert!(!vmm.is_unreaped(stuck.control.pid), "the stuck process is not yet killed");
    assert_eq!(
        vmm.vsock_device_holders(),
        BTreeMap::from([(cid(6), stuck.control.pid)]),
        "only the stuck process still holds a device"
    );
}

// ---------------------------------------------------------------------
// SimVmHostState: the held scope
// ---------------------------------------------------------------------

/// A host with one scope, run directory and clone for `name`, the scope seeded with `pids`.
fn host_with_scope(name: &str, pids: &[u32]) -> (SimVmHostState, AllocationId) {
    let host = SimVmHostState::new();
    let alloc = alloc(name);
    host.set_scope(alloc.clone(), pids.iter().copied().collect::<BTreeSet<u32>>());
    host.set_run_dir(alloc.clone());
    host.set_clone(alloc.clone(), PathBuf::from(format!("/sim/clone-{name}.img")));
    (host, alloc)
}

fn scope_of(alloc: &AllocationId) -> CgroupPath {
    CgroupPath::for_alloc(alloc)
}

/// ADR-0349 item 5. Owner path: `VmHostState::kill_scope` and `observe`. Fault: the scope is held.
/// Oracle: the error kind, and an unchanged whole observation before and after.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_held_scope_refuses_the_kill_with_resource_busy_and_the_observation_is_unchanged() {
    let (host, alloc) = host_with_scope("held-a", &[11, 12]);
    host.hold_scope_while(&alloc, || true);
    let before = host.observe().await.expect("observe");

    let refusal = host.kill_scope(&scope_of(&alloc)).await.expect_err("a held scope is refused");

    assert_eq!(refusal.kind(), ErrorKind::ResourceBusy);
    let after = host.observe().await.expect("observe");
    assert_eq!(after, before, "a refused kill changes nothing: scope, pids, run dir and clone");
    assert_eq!(
        after.scopes.get(&alloc).map(|facts| facts.pids.clone()),
        Some(BTreeSet::from([11, 12])),
        "the hold never edits the scope's pids"
    );
}

/// ADR-0349 item 5: the held refusal does not consume the scripted-failure queue, and the queue is
/// served first once the hold ends. Oracle: the sequence of kinds the calls return.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_held_refusal_leaves_the_scripted_failure_queue_for_the_call_after_the_hold() {
    let (host, alloc) = host_with_scope("held-queue", &[21]);
    host.script_kill_scope_failure(ErrorKind::PermissionDenied);
    host.hold_scope_while(&alloc, || true);
    let scope = scope_of(&alloc);

    let busy = host.kill_scope(&scope).await.expect_err("held");
    let busy_again = host.kill_scope(&scope).await.expect_err("still held");
    host.release_scope_hold(&alloc);
    let scripted = host.kill_scope(&scope).await.expect_err("the queued failure is served");
    host.kill_scope(&scope).await.expect("the queue is empty, the ordinary removal succeeds");

    assert_eq!(
        (busy.kind(), busy_again.kind(), scripted.kind()),
        (ErrorKind::ResourceBusy, ErrorKind::ResourceBusy, ErrorKind::PermissionDenied),
        "held calls do not pop the queue; the first call after the hold does"
    );
    assert!(
        !host.observe().await.expect("observe").scopes.contains_key(&alloc),
        "Ok from kill_scope means the scope is absent from observe"
    );
}

/// ADR-0349 item 5: `held()` is read at each `kill_scope` call and the hold never removes the scope
/// by itself; the scope leaves only through `kill_scope`. Oracle: the kinds, and `observe`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_hold_predicate_is_read_at_each_call_and_the_scope_leaves_only_through_the_kill() {
    let (host, alloc) = host_with_scope("held-read", &[31]);
    let held = Arc::new(AtomicBool::new(true));
    host.hold_scope_while(&alloc, {
        let held = Arc::clone(&held);
        move || held.load(Ordering::SeqCst)
    });
    let scope = scope_of(&alloc);

    host.kill_scope(&scope).await.expect_err("held while the predicate is true");
    held.store(false, Ordering::SeqCst);
    assert!(
        host.observe().await.expect("observe").scopes.contains_key(&alloc),
        "a predicate that turns false removes nothing by itself"
    );
    host.kill_scope(&scope).await.expect("the predicate is read again: false, so the kill is Ok");
    assert!(
        !host.observe().await.expect("observe").scopes.contains_key(&alloc),
        "the scope left through kill_scope"
    );

    // The hold is still registered: a scope seeded again under it is refused again.
    held.store(true, Ordering::SeqCst);
    host.set_scope(alloc.clone(), BTreeSet::from([32]));
    let refusal = host.kill_scope(&scope).await.expect_err("held again, read at the call");
    assert_eq!(refusal.kind(), ErrorKind::ResourceBusy);
}

/// ADR-0349 item 5: `release_scope_hold` ends the hold whatever the closure says, and is a no-op
/// when there is none. Oracle: the kill after the release, and no panic.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn releasing_a_hold_ends_it_whatever_the_predicate_says_and_releasing_none_is_a_no_op() {
    let (host, alloc) = host_with_scope("held-release", &[41]);
    host.release_scope_hold(&alloc);
    host.release_scope_hold(&AllocationId::new("never-held").expect("valid id"));
    host.hold_scope_while(&alloc, || true);
    host.kill_scope(&scope_of(&alloc)).await.expect_err("held");

    host.release_scope_hold(&alloc);

    host.kill_scope(&scope_of(&alloc))
        .await
        .expect("the hold is gone although the closure says true");
    assert!(!host.observe().await.expect("observe").scopes.contains_key(&alloc));
}

/// `hold_scope_while` on an allocation with no scope is a test-authoring error. Oracle: the panic.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "was not seeded with set_scope")]
fn holding_an_allocation_with_no_scope_is_refused() {
    let host = SimVmHostState::new();
    host.hold_scope_while(&alloc("no-scope"), || true);
}

/// The reap couples the unreaped process and the hold through the one shared fact
/// (`is_unreaped`): the scripted reap clears the device and ends the hold with no second call.
/// Oracle: `kill_scope` before and after `exit_process`, and the device holders.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_scripted_reap_ends_the_scope_hold_through_the_shared_unreaped_fact() {
    let root = tempfile::tempdir().expect("tempdir");
    let (vmm, clock) = sim_vmm();
    let process = launch_unreapable(&vmm, root.path(), "stuck-coupled", 8).await;
    let pid = process.control.pid;
    let (host, alloc) = host_with_scope("stuck-coupled", &[pid]);
    host.hold_scope_while(&alloc, {
        let vmm = vmm.clone();
        move || vmm.is_unreaped(pid)
    });
    let scope = scope_of(&alloc);
    host.kill_scope(&scope).await.expect("before the kill the process is live: the hold is open");
    host.set_scope(alloc.clone(), BTreeSet::from([pid]));

    terminate_across_exactly_the_reap_bound(&vmm, &clock, &process).await;
    let busy = host.kill_scope(&scope).await.expect_err("the scope is held while unreaped");
    assert_eq!(busy.kind(), ErrorKind::ResourceBusy);
    assert_eq!(vmm.vsock_device_holders().len(), 1, "the device is held while unreaped");

    assert!(vmm.exit_process(pid, None, Some(9)), "the scripted reap");

    assert!(vmm.vsock_device_holders().is_empty(), "the reap released the device");
    host.kill_scope(&scope).await.expect("the reap ended the hold; the scope is removable");
    assert!(!host.observe().await.expect("observe").scopes.contains_key(&alloc));
}

// ---------------------------------------------------------------------
// SimVmHostState: the scripted observe failure
// ---------------------------------------------------------------------

/// ADR-0348 RB-2 (c). Owner path: `VmHostState::observe`. Fault: queued failures. Oracle: the kinds
/// in FIFO order, an unchanged observation, and the ordinary path once the queue is empty.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn scripted_observe_failures_are_served_in_order_and_change_nothing() {
    let (host, alloc) = host_with_scope("observe-fifo", &[51]);
    let pristine = host.observe().await.expect("observe before any script");
    host.script_observe_failure(ErrorKind::PermissionDenied);
    host.script_observe_failure(ErrorKind::TimedOut);

    let first = host.observe().await.expect_err("first scripted failure");
    let second = host.observe().await.expect_err("second scripted failure");
    let third = host.observe().await.expect("the queue is empty: the ordinary path");

    assert_eq!((first.kind(), second.kind()), (ErrorKind::PermissionDenied, ErrorKind::TimedOut));
    assert_eq!(third, pristine, "a failing observe reads and changes nothing");
    assert!(third.scopes.contains_key(&alloc));
}

/// The scripted observe failure does not touch the probe-fault slot. Oracle: the probe fault is
/// still armed after the observe failure is served, and fires once on the probe.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_scripted_observe_failure_leaves_the_probe_fault_armed() {
    let host = SimVmHostState::new();
    host.inject_probe_fault(ErrorKind::PermissionDenied);
    host.script_observe_failure(ErrorKind::TimedOut);

    let observe = host.observe().await.expect_err("the scripted observe failure");
    let probe = host.probe().await.expect_err("the probe fault is still armed");

    assert_eq!(observe.kind(), ErrorKind::TimedOut);
    assert!(
        matches!(probe, VmHostStateProbeError::Substrate { .. }),
        "the probe fault surfaces as its own Substrate error: {probe:?}"
    );
    host.probe().await.expect("the probe fault was one-shot and is consumed by the probe");
}

/// An armed observe barrier is honoured before the scripted failure: the call signals it started,
/// waits for the release, then fails. Oracle: the call is unfinished until released, then Err.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn an_armed_observe_barrier_is_honoured_before_the_scripted_failure() {
    let host = SimVmHostState::new();
    let barrier = host.arm_observe_barrier();
    host.script_observe_failure(ErrorKind::TimedOut);
    let observer = {
        let host = host.clone();
        tokio::spawn(async move { host.observe().await })
    };

    barrier.wait_for_observe_started().await;
    tokio::task::yield_now().await;
    assert!(!observer.is_finished(), "the barrier holds the call before it can fail");
    barrier.release_observe();

    let failure =
        observer.await.expect("the observer task").expect_err("then the scripted failure");
    assert_eq!(failure.kind(), ErrorKind::TimedOut);
}
