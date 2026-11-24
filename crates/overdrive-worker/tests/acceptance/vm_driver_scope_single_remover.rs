//! Node-local persistent volume (GH #348, ADR-0351, ADR-0349, ADR-0352): the VM driver removes
//! a VM scope through `VmHostState::kill_scope` and through nothing else, a busy scope changes
//! neither the stop outcome nor the reported start failure nor the other cleanup steps, and the
//! driver no longer places the VMM pid in the scope.
//!
//! Owner path: the production `VmDriver` over the fixtures of `vm_driver_stop_unreaped`.
//! A `kill_scope` call is observed through the scripted-failure queue of `SimVmHostState`: a
//! queued `ResourceBusy` is consumed by the first call, so a direct probe call afterwards sees
//! `Ok` exactly when the driver called first. A driver removal of the scope directory through the
//! cgroup port is observed in `SimCgroupFs::snapshot` (the two sims are separate, so only a
//! driver removal through the cgroup port empties that entry).
//!
//! Evidence tier: in-memory acceptance (layer 2).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "test fixtures assert their preconditions"
)]

use std::time::Duration;

use overdrive_core::traits::driver::{Driver as _, DriverError, DriverStartClass, VmStartFailure};
use overdrive_core::traits::vm_host_state::VmHostState as _;

use super::vm_driver_stop_unreaped::{Node, join};

/// Drives a start whose guest never beacons to its boot-deadline failure. `scope_setup` runs once
/// the VMM exists (with its pid and the allocation). Returns the pid and the start result.
async fn fail_start_at_boot_deadline(
    node: &Node,
    name: &str,
    scope_setup: impl FnOnce(&Node, &overdrive_core::id::AllocationId, u32),
) -> (u32, Result<(), DriverError>) {
    let spec = node.spec(name, 2, false);
    let driver = node.driver.clone();
    let spec_owned = spec.clone();
    let start = tokio::spawn(async move {
        driver
            .start(&spec_owned, &overdrive_sim::adapters::SimLaunchSupersession::new())
            .await
            .map(|_| ())
    });
    let pid = node.beacon.next_created_pid().await;
    scope_setup(node, &spec.alloc, pid);
    for _ in 0..120 {
        if start.is_finished() {
            break;
        }
        node.advance(Duration::from_secs(1)).await;
    }
    (pid, join(start, "the start past its boot deadline").await)
}

fn assert_boot_deadline(result: &Result<(), DriverError>) {
    assert!(
        matches!(
            result,
            Err(DriverError::StartRejected { failure })
                if matches!(
                    failure.class,
                    DriverStartClass::Vm(VmStartFailure::BootDeadlineExceeded { .. })
                )
        ),
        "the reported start failure must stay the boot deadline: {result:?}"
    );
}

/// ADR-0351. A reaped stop removes the VM scope through `VmHostState::kill_scope`.
/// Oracle: the seeded scope is gone from the host-state port after the stop.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[should_panic(expected = "a reaped stop must remove the VM scope through VmHostState::kill_scope")]
async fn a_reaped_stop_removes_the_scope_through_the_host_state_port() {
    let node = Node::new().await;
    let spec = node.spec("alloc-remover-stop", 2, false);
    let (handle, _guest) = node.start_running(&spec).await;
    node.seed_scope(&spec.alloc, handle.pid.expect("pid"));

    let stop = node.begin_stop(&handle).await;
    node.advance(Duration::from_secs(2)).await;
    join(stop, "the stop").await.expect("the stop completes");

    assert!(!node.run_dir_of(&spec.alloc).path().exists(), "the run directory is removed");
    assert!(
        !node.host.has_scope(&spec.alloc),
        "a reaped stop must remove the VM scope through VmHostState::kill_scope"
    );
}

/// ADR-0351. A start-failure cleanup removes the VM scope through `VmHostState::kill_scope`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[should_panic(
    expected = "a start-failure cleanup must remove the VM scope through VmHostState::kill_scope"
)]
async fn a_start_failure_cleanup_removes_the_scope_through_the_host_state_port() {
    let node = Node::new().await;
    let (_pid, result) =
        fail_start_at_boot_deadline(&node, "alloc-remover-start", |node, alloc, pid| {
            node.seed_scope(alloc, pid);
        })
        .await;
    assert_boot_deadline(&result);
    let alloc = overdrive_core::id::AllocationId::new("alloc-remover-start").expect("alloc");
    assert!(!node.run_dir_of(&alloc).path().exists(), "the run directory is removed");
    assert!(
        !node.host.has_scope(&alloc),
        "a start-failure cleanup must remove the VM scope through VmHostState::kill_scope"
    );
}

/// ADR-0351. No other path removes the scope: after a reaped stop the scope directory the driver
/// created through the cgroup port is not removed by the driver (the host-state port is the only
/// remover). Oracle: the entry stays in `SimCgroupFs`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[should_panic(expected = "the VM driver removed the scope directory itself")]
async fn the_driver_never_removes_a_scope_directory_through_the_cgroup_port() {
    let node = Node::new().await;
    let spec = node.spec("alloc-remover-only", 2, false);
    let (handle, _guest) = node.start_running(&spec).await;
    let scope_dir =
        overdrive_core::cgroup::CgroupPath::for_alloc(&spec.alloc).resolve(&node.cgroup_root);
    assert!(
        node.cgroup_fs.snapshot().contains_key(&scope_dir),
        "precondition: the launch created the scope directory"
    );
    let stop = node.begin_stop(&handle).await;
    node.advance(Duration::from_secs(2)).await;
    join(stop, "the stop").await.expect("the stop completes");
    assert!(
        node.cgroup_fs.snapshot().contains_key(&scope_dir),
        "the VM driver removed the scope directory itself instead of leaving removal to \
         VmHostState::kill_scope"
    );
}

/// ADR-0349 and ADR-0351. A busy scope (`ResourceBusy` from `kill_scope`) leaves the stop outcome
/// and the other cleanup steps unchanged: the stop is Ok, the run directory and clone are
/// removed, and the scope is left for reclamation.
/// Oracle: the busy failure was consumed by a driver call, with the stop still completed.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[should_panic(expected = "the stop must call VmHostState::kill_scope for the scope")]
async fn a_busy_scope_leaves_the_stop_outcome_and_the_other_cleanup_steps_unchanged() {
    let node = Node::new().await;
    let spec = node.spec("alloc-remover-busy-stop", 2, false);
    let (handle, _guest) = node.start_running(&spec).await;
    let scope = node.seed_scope(&spec.alloc, handle.pid.expect("pid"));
    node.queue_busy_probe();

    let stop = node.begin_stop(&handle).await;
    node.advance(Duration::from_secs(2)).await;
    join(stop, "the stop").await.expect("a busy scope must not fail the stop");

    assert!(!node.run_dir_of(&spec.alloc).path().exists(), "the run directory is removed");
    assert!(!node.rootfs_plan_of(&spec.alloc).clone_dest().exists(), "the clone is removed");
    assert!(node.host.has_scope(&spec.alloc), "the busy scope is left to reclamation");
    assert!(
        node.kill_scope_was_called(&scope).await,
        "the stop must call VmHostState::kill_scope for the scope"
    );
}

/// ADR-0349 and ADR-0351. A busy scope leaves the reported start failure unchanged: the failure
/// stays the boot deadline (no cleanup-failure composite), the run directory and clone are removed.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[should_panic(expected = "the start-failure cleanup must call VmHostState::kill_scope")]
async fn a_busy_scope_leaves_the_reported_start_failure_unchanged() {
    let node = Node::new().await;
    let mut scope = None;
    let (_pid, result) =
        fail_start_at_boot_deadline(&node, "alloc-remover-busy-start", |n, a, p| {
            scope = Some(n.seed_scope(a, p));
            n.queue_busy_probe();
        })
        .await;
    assert_boot_deadline(&result);
    let alloc = overdrive_core::id::AllocationId::new("alloc-remover-busy-start").expect("alloc");
    assert!(!node.run_dir_of(&alloc).path().exists(), "the run directory is removed");
    assert!(!node.rootfs_plan_of(&alloc).clone_dest().exists(), "the clone is removed");
    assert!(
        node.kill_scope_was_called(&scope.expect("scope seeded")).await,
        "the start-failure cleanup must call VmHostState::kill_scope"
    );
}

/// ADR-0349. A held scope (`hold_scope_while`) refuses the kill with `ResourceBusy`; the stop is
/// still Ok, the run directory and clone are removed, and the host-state port still reports the
/// scope. A GUARD: it holds before the driver calls the port and must keep holding.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_held_scope_leaves_the_stop_outcome_and_artifacts_unchanged() {
    let node = Node::new().await;
    let spec = node.spec("alloc-remover-held", 2, false);
    let (handle, _guest) = node.start_running(&spec).await;
    node.seed_scope(&spec.alloc, handle.pid.expect("pid"));
    node.host.hold_scope_while(&spec.alloc, || true);

    let stop = node.begin_stop(&handle).await;
    node.advance(Duration::from_secs(2)).await;
    join(stop, "the stop").await.expect("a held scope must not fail the stop");

    assert!(!node.run_dir_of(&spec.alloc).path().exists(), "the run directory is removed");
    assert!(!node.rootfs_plan_of(&spec.alloc).clone_dest().exists(), "the clone is removed");
    let observed = node.host.observe().await.expect("observe");
    assert!(observed.scopes.contains_key(&spec.alloc), "the held scope is still reported");
}

/// ADR-0352. The driver makes no `place_pid` call: after a launch the scope holds no
/// `cgroup.procs` write by the driver (membership comes from the spawn). Oracle: no
/// `cgroup.procs` entry under the scope in `SimCgroupFs`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[should_panic(expected = "the driver must not write the VMM pid to the scope's cgroup.procs")]
async fn the_driver_does_not_place_the_vmm_pid_in_its_scope() {
    let node = Node::new().await;
    let spec = node.spec("alloc-no-place-pid", 2, false);
    let (_handle, _guest) = node.start_running(&spec).await;
    let procs = overdrive_core::cgroup::CgroupPath::for_alloc(&spec.alloc)
        .resolve(&node.cgroup_root)
        .join("cgroup.procs");
    assert!(
        !node.cgroup_fs.snapshot().contains_key(&procs),
        "the driver must not write the VMM pid to the scope's cgroup.procs"
    );
}

/// ADR-0352. A failed `Vmm::create` leaves no process (so no scope member) and no held vsock
/// device. A GUARD over the existing sim hooks; the scope-membership oracle is not available
/// (SimVmm records no membership).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn guard_a_failed_create_leaves_no_vmm_process() {
    let node = Node::new().await;
    node.sim.inject_create_failure();
    let spec = node.spec("alloc-create-fails", 2, false);
    let result = node
        .driver
        .start(&spec, &overdrive_sim::adapters::SimLaunchSupersession::new())
        .await
        .map(|_| ());
    assert!(result.is_err(), "a failed create fails the start: {result:?}");
    assert!(node.sim.live_pids().is_empty(), "no VMM process remains");
    assert!(node.sim.vsock_device_holders().is_empty(), "no vsock device is held");
}
