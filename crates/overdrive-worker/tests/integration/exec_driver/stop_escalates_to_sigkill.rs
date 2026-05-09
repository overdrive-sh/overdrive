//! US-02 Scenario 2.7 — `Driver::stop` escalates to SIGKILL when the
//! workload ignores SIGTERM beyond the grace window.
//!
//! @real-io — Linux. The workload is a `/bin/sh -c 'trap "" TERM; ...'`
//! that ignores SIGTERM. After the grace window elapses, the driver
//! sends SIGKILL; the test asserts the process is reaped, the state
//! advances to `NotFound`, AND the reparented `sleep` grandchild is
//! also reaped — the latter is what pins the post-grace SIGKILL path
//! (process-group `kill(-pid, SIGKILL)` in `send_sigkill_pgrp` AND
//! the parallel `cgroup.kill` write in production cgroupfs).
//!
//! Phase 02 migration: real `/sys/fs/cgroup` per the bugfix RCA § D.
//! On real cgroupfs both mechanisms (`cgroup.kill` AND
//! `send_sigkill_pgrp`) reach the grandchild; the test asserts the
//! observable outcome (grandchild reaped) without distinguishing
//! which mechanism delivered the kill.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant as StdInstant};

use async_trait::async_trait;
use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverError, Resources};
use overdrive_worker::ExecDriver;
use overdrive_worker::cgroup_manager::create_workloads_slice_with_controllers;
use serial_test::serial;
use tokio::time::Instant;

use super::cleanup::AllocCleanup;

/// Test-local [`Clock`] impl that delegates `sleep` to the tokio
/// timer and reads `now` / `unix_now` from real wall-clock. Used in
/// place of `SimClock` for real-IO tests where the SUT runs real
/// processes — `SimClock::sleep` parks until an external `tick()`,
/// which has no caller in real-IO scenarios. This intentionally lives
/// in the test crate rather than `overdrive-sim` (DST-only) or
/// `overdrive-host` (forbidden as a dev-dep per
/// `.claude/rules/development.md`).
struct TokioWallClock;

#[async_trait]
impl Clock for TokioWallClock {
    fn now(&self) -> StdInstant {
        StdInstant::now()
    }

    fn unix_now(&self) -> Duration {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// Bit mask for SIGTERM (signal 15) in the `SigIgn` mask reported by
/// `/proc/<pid>/status`. Bit `n-1` corresponds to signal `n`.
const SIGTERM_BIT: u64 = 1u64 << (15 - 1);

/// Poll `/proc/<pid>/status` until the workload has set up its
/// SIGTERM ignore-trap, OR a deadline elapses. Eliminates the
/// race where SIGTERM is delivered to the freshly-spawned shell
/// before it has executed `trap '' TERM`.
async fn await_sigterm_trap_installed(pid: u32, deadline: Duration) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        let path = format!("/proc/{pid}/status");
        match std::fs::read_to_string(&path) {
            Ok(status) => {
                if let Some(line) = status.lines().find(|l| l.starts_with("SigIgn:")) {
                    let hex = line.trim_start_matches("SigIgn:").trim();
                    if let Ok(mask) = u64::from_str_radix(hex, 16) {
                        if mask & SIGTERM_BIT != 0 {
                            return Ok(());
                        }
                    }
                }
            }
            Err(_) => {
                return Err(format!("could not read {path}"));
            }
        }
        if started.elapsed() >= deadline {
            return Err(format!("SIGTERM trap not installed within {deadline:?}"));
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Read direct children of `pid` from
/// `/proc/<pid>/task/<pid>/children`.
fn read_direct_children(pid: u32) -> Vec<u32> {
    let path = format!("/proc/{pid}/task/{pid}/children");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    raw.split_whitespace().filter_map(|s| s.parse::<u32>().ok()).collect()
}

/// Determine whether the process at `pid` is a "live sleep".
fn sleep_grandchild_is_live(pid: u32) -> bool {
    let comm_path = format!("/proc/{pid}/comm");
    let Ok(comm) = std::fs::read_to_string(&comm_path) else {
        return false;
    };
    if comm.trim() != "sleep" {
        return false;
    }
    let status_path = format!("/proc/{pid}/status");
    let Ok(status) = std::fs::read_to_string(&status_path) else {
        return false;
    };
    let state_char = status
        .lines()
        .find(|l| l.starts_with("State:"))
        .and_then(|l| l.trim_start_matches("State:").trim().chars().next());
    !matches!(state_char, Some('Z' | 'X'))
}

/// Wait up to `deadline` for the sleep grandchild at `pid` to no
/// longer be live (per `sleep_grandchild_is_live`).
async fn await_sleep_grandchild_reaped(pid: u32, deadline: Duration) -> bool {
    let started = std::time::Instant::now();
    loop {
        if !sleep_grandchild_is_live(pid) {
            return true;
        }
        if started.elapsed() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
#[serial(cgroup)]
async fn stop_escalates_to_sigkill_when_sigterm_ignored() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    create_workloads_slice_with_controllers(cgroup_root)
        .expect("workloads.slice bootstrap succeeds");

    // Real-IO test: the SUT runs a real `/bin/sh` and the grace window
    // in `Driver::stop` must elapse against actual wall-clock for the
    // SIGKILL escalation path to fire. `SimClock` would park
    // indefinitely waiting for `tick()`. See `TokioWallClock` above.
    let driver: Arc<dyn Driver> = Arc::new(
        ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(TokioWallClock))
            .with_stop_grace(Duration::from_millis(250)),
    );

    let alloc = AllocationId::new("alloc-stop-sigkill").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    // /bin/sh that traps and ignores SIGTERM; sleeps 60s.
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/x/alloc/sk")
            .expect("valid spiffe id"),
        command: "/bin/sh".to_owned(),
        args: vec!["-c".to_owned(), "trap '' TERM; sleep 60".to_owned()],
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
    };

    let handle = driver.start(&spec).await.expect("start succeeds");

    // Wait for the shell to actually install `trap '' TERM` before we
    // probe stop()'s escalation behaviour. Without this, SIGTERM races
    // the shell's startup and kills it with the default action — the
    // grace window then never applies and the test sees a sub-grace
    // elapsed (~100µs).
    let pid = handle.pid.expect("ExecDriver always populates pid on Linux");
    await_sigterm_trap_installed(pid, Duration::from_secs(2))
        .await
        .expect("workload installed SIGTERM trap before stop()");

    // Capture the `sleep` grandchild PID BEFORE stop. The shell forks
    // `sleep` as its direct child via `exec` after the trap is set;
    // the `trap '' TERM` line installs the trap, then `sleep 60`
    // forks. Poll briefly for the child to appear (the test races
    // sub-shell exec, so the grandchild may take a few ms to fork).
    let mut sleep_pids: Vec<u32> = Vec::new();
    let children_deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < children_deadline {
        let children = read_direct_children(pid);
        sleep_pids = children.into_iter().filter(|p| sleep_grandchild_is_live(*p)).collect();
        if !sleep_pids.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !sleep_pids.is_empty(),
        "test setup invariant: a `sleep` grandchild must be running under the shell at pid={pid} \
         before stop() is invoked; saw none after 2s of polling /proc/{pid}/task/{pid}/children",
    );

    let started = Instant::now();
    driver.stop(&handle).await.expect("stop eventually succeeds via SIGKILL");
    let elapsed = started.elapsed();

    // Wall-clock upper bound — escalation must complete within budget.
    assert!(elapsed < Duration::from_secs(10), "stop did not escalate within budget ({elapsed:?})");

    // Per `fix-terminated-slot-accumulation` Step 01-02: the driver
    // does not retain a terminal-state slot after stop. Durable
    // terminal-state truth lives in `ObservationStore::AllocStatusRow`;
    // `Driver::status` returns `Err(NotFound)` post-stop.
    let err = driver.status(&handle).await.expect_err("status returns NotFound after stop");
    assert!(
        matches!(err, DriverError::NotFound { ref alloc } if *alloc == handle.alloc),
        "status after stop must be Err(NotFound {{ alloc }}); got {err:?}",
    );

    // Crucially: the reparented `sleep` grandchild MUST also be
    // reaped. On real cgroupfs (this test) BOTH mechanisms fire:
    //   * `cgroup.kill` write — atomic SIGKILL of every task in the
    //     scope, reaches the grandchild via the kernel.
    //   * `send_sigkill_pgrp(pid)` — `kill(-pid, SIGKILL)` reaches
    //     every member of the process group (the child was setsid-ed
    //     at spawn so PGID == shell PID).
    // Either is sufficient on real cgroupfs; the assertion pins the
    // observable outcome regardless of which mechanism delivered the
    // signal.
    for sleep_pid in &sleep_pids {
        let reaped = await_sleep_grandchild_reaped(*sleep_pid, Duration::from_secs(5)).await;
        assert!(
            reaped,
            "sleep grandchild at pid={sleep_pid} must be reaped after stop(); \
             still live in /proc — neither cgroup.kill nor process-group SIGKILL reached it",
        );
    }
}
