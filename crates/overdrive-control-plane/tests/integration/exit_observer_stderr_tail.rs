//! Slice 02c (step 02-05) — `ExitObserver` stderr-tail capture per
//! ADR-0033 Amendment 2026-05-10.
//!
//! Spawns a real `/bin/sh -c '...'` workload via `ExecDriver` whose
//! stderr writes 7 lines and exits non-zero, observes the resulting
//! `AllocStatusRow` written by `exit_observer::spawn`, and asserts
//! the row's `stderr_tail` field carries exactly the LAST 5 lines
//! in order (per `STDERR_TAIL_LINES = 5` SSOT in `exit_observer.rs`).
//!
//! Linux-only — `ExecDriver` requires a real cgroup v2 root. Routed
//! through Lima per `.claude/rules/testing.md` § "Running tests —
//! Lima VM"; gated behind `integration-tests` feature per
//! § "Integration vs unit gating".

#![cfg(target_os = "linux")]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::id::{AllocationId, JobId, NodeId, SpiffeId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{AllocationSpec, Driver, Resources};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_worker::ExecDriver;
use overdrive_worker::cgroup_manager::create_workloads_slice_with_controllers;
use serial_test::serial;
use tokio::sync::broadcast;

use overdrive_control_plane::action_shim::LifecycleEvent;
use overdrive_control_plane::worker::exit_observer;

// ---------------------------------------------------------------------
// Cleanup guard — duplicated from
// `crates/overdrive-worker/tests/integration/exec_driver/cleanup.rs`
// because cross-crate test-helper sharing is not wired in this repo
// (each crate owns its own integration helpers per testing.md).
// ---------------------------------------------------------------------

struct AllocCleanup {
    cgroup_root: std::path::PathBuf,
    alloc: AllocationId,
}

impl Drop for AllocCleanup {
    fn drop(&mut self) {
        let scope = self
            .cgroup_root
            .join("overdrive.slice/workloads.slice")
            .join(format!("{}.scope", self.alloc));
        let pids: Vec<libc::pid_t> = std::fs::read_to_string(scope.join("cgroup.procs"))
            .ok()
            .map(|s| s.lines().filter_map(|l| l.trim().parse::<i32>().ok()).collect())
            .unwrap_or_default();
        let _ = std::fs::write(scope.join("cgroup.kill"), "1\n");
        for pid in pids {
            for _ in 0..20 {
                let mut status: libc::c_int = 0;
                // SAFETY: thin libc wrapper; pid is a value we read
                // out of cgroup.procs above and the status pointer
                // is valid for the duration of the call.
                let r = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
                if r == pid || r == -1 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let _ = std::fs::remove_dir(&scope);
    }
}

/// Seed a Running prior row for the alloc so `find_prior_row` in the
/// observer succeeds when the exit event arrives. Production wires
/// the action shim to write Running BEFORE `Driver::start` completes;
/// this test bypasses the action shim, so we do that step inline.
async fn seed_running_row(
    obs: &dyn ObservationStore,
    alloc: &AllocationId,
    job_id: &JobId,
    node_id: &NodeId,
) {
    let row = AllocStatusRow {
        alloc_id: alloc.clone(),
        job_id: job_id.clone(),
        node_id: node_id.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: node_id.clone() },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
        listeners: Vec::new(),
    };
    obs.write(ObservationRow::AllocStatus(row)).await.expect("seed Running row");
}

#[tokio::test]
#[serial(cgroup)]
async fn exit_observer_captures_last_n_stderr_lines_on_terminal() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    create_workloads_slice_with_controllers(cgroup_root)
        .expect("workloads.slice bootstrap succeeds");

    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let driver = Arc::new(ExecDriver::new(cgroup_root.to_path_buf(), clock.clone()));
    let driver_dyn: Arc<dyn Driver> = driver.clone();

    let node_id = NodeId::new("test-node").expect("valid node id");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(node_id.clone(), 0));
    let (events_tx, mut events_rx) = broadcast::channel::<LifecycleEvent>(64);
    let events = Arc::new(events_tx);

    let alloc = AllocationId::new("alloc-stderr-tail-test-0").expect("valid alloc id");
    let job_id = JobId::new("stderr-tail-test").expect("valid job id");
    let _cleanup = AllocCleanup { cgroup_root: cgroup_root.to_path_buf(), alloc: alloc.clone() };

    seed_running_row(obs.as_ref(), &alloc, &job_id, &node_id).await;

    let observer_handle =
        exit_observer::spawn(obs.clone(), driver_dyn.clone(), events.clone(), clock.clone());

    // Workload: writes 7 lines to stderr and exits 1. The last 5
    // lines (ERR 3..ERR 7) MUST appear in `row.stderr_tail`.
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/stderr-tail-test/alloc/0")
            .expect("valid spiffe id"),
        command: "/bin/sh".to_owned(),
        args: vec![
            "-c".to_owned(),
            "for i in 1 2 3 4 5 6 7; do echo \"ERR $i\" >&2; done; exit 1".to_owned(),
        ],
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
    };

    let handle = driver_dyn.start(&spec).await.expect("ExecDriver::start succeeds");

    // Per step 01-03 of `fix-exit-observer-running-gate`: the watcher
    // parks on a Running-confirmed gate before its first `ExitEvent`
    // send. This test does NOT go through the action shim's
    // StartAllocation arm (the seeded Running row was written
    // directly above), so we fire the gate manually here to mirror
    // what the action shim would do post-`obs.write(Running)` Ok.
    // Without this fire, the watcher would park indefinitely on the
    // gate and the test would time out at the 5s `tokio::time::
    // timeout` budget below.
    driver_dyn.release_for_exit_emission(&handle);

    // Wait for the observer's lifecycle event for the terminal row.
    // 5s is generous — the workload exits in milliseconds and the
    // observer writes the row inline.
    let terminal_event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events_rx.recv().await {
                Ok(ev) if ev.alloc_id == alloc => return ev,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => panic!("event channel closed"),
            }
        }
    })
    .await
    .expect("terminal lifecycle event arrives within 5s");

    assert_eq!(terminal_event.alloc_id, alloc, "lifecycle event must reference the test alloc");

    // Read the AllocStatusRow the observer wrote and verify
    // stderr_tail carries the last 5 lines.
    let row = obs
        .alloc_status_row(&alloc)
        .await
        .expect("alloc_status_row read")
        .expect("terminal row exists after exit event");

    assert_eq!(row.state, AllocState::Failed, "non-zero exit must produce Failed");

    let tail = row.stderr_tail.as_deref().expect("stderr_tail populated on Job-kind terminal");

    // Newline-terminated; collect non-empty lines.
    let lines: Vec<&str> = tail.lines().collect();
    assert_eq!(
        lines,
        vec!["ERR 3", "ERR 4", "ERR 5", "ERR 6", "ERR 7"],
        "stderr_tail must contain the last 5 stderr lines in order, got: {tail:?}"
    );

    // Drop driver Arc so observer's rx returns None and task exits.
    drop(driver);
    drop(driver_dyn);
    let _ = tokio::time::timeout(Duration::from_secs(2), observer_handle).await;
}
