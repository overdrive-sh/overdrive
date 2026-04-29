//! US-02 Scenario 2.7 — `Driver::stop` escalates to SIGKILL when the
//! workload ignores SIGTERM beyond the grace window.
//!
//! @real-io — Linux. The workload is a `/bin/sh -c 'trap "" TERM; ...'`
//! that ignores SIGTERM. After the grace window elapses, the driver
//! sends SIGKILL; the test asserts the process is reaped, the state
//! advances to `Terminated`, AND the reparented `sleep` grandchild
//! is also reaped — the latter is what pins the `kill(-pid, SIGKILL)`
//! process-group escalation in `send_sigkill_pgrp` (the only
//! mechanism that reaches the grandchild on the TempDir-as-cgroupfs
//! path, since `cgroup.kill` is a regular file write that never
//! reaches the kernel here).

use std::sync::Arc;
use std::time::Duration;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::driver::{AllocationSpec, AllocationState, Driver, Resources};
use overdrive_worker::ExecDriver;
use tempfile::TempDir;
use tokio::time::Instant;

/// Bit mask for SIGTERM (signal 15) in the `SigIgn` mask reported by
/// `/proc/<pid>/status`. Bit `n-1` corresponds to signal `n`.
const SIGTERM_BIT: u64 = 1u64 << (15 - 1);

/// Poll `/proc/<pid>/status` until the workload has set up its
/// SIGTERM ignore-trap, OR a deadline elapses. Eliminates the
/// race where SIGTERM is delivered to the freshly-spawned shell
/// before it has executed `trap '' TERM`.
///
/// Returns `Ok(())` once the bit is observed; `Err(...)` on timeout
/// or when `/proc/<pid>/status` cannot be read at all.
async fn await_sigterm_trap_installed(pid: u32, deadline: Duration) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        let path = format!("/proc/{pid}/status");
        match std::fs::read_to_string(&path) {
            Ok(status) => {
                if let Some(line) = status.lines().find(|l| l.starts_with("SigIgn:")) {
                    // Format: `SigIgn:\t0000000000004000`
                    let hex = line.trim_start_matches("SigIgn:").trim();
                    if let Ok(mask) = u64::from_str_radix(hex, 16) {
                        if mask & SIGTERM_BIT != 0 {
                            return Ok(());
                        }
                    }
                }
            }
            Err(_) => {
                // Process may have already exited — bail.
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
/// `/proc/<pid>/task/<pid>/children`. The kernel exposes children
/// PIDs as a single space-separated line. Empty file or missing pid
/// yields an empty vec.
fn read_direct_children(pid: u32) -> Vec<u32> {
    let path = format!("/proc/{pid}/task/{pid}/children");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    raw.split_whitespace().filter_map(|s| s.parse::<u32>().ok()).collect()
}

/// Determine whether the process at `pid` is a "live sleep". A pid
/// that maps to a `/proc/<pid>/comm` of `sleep` AND whose state line
/// is NOT `Z` (zombie) AND NOT `X` (dead) is treated as live.
///
/// Returns `false` when:
///   * `/proc/<pid>/comm` no longer exists (kernel reaped),
///   * the comm has changed (pid recycled to a different program),
///   * the process is in zombie / dead state.
fn sleep_grandchild_is_live(pid: u32) -> bool {
    let comm_path = format!("/proc/{pid}/comm");
    let comm = match std::fs::read_to_string(&comm_path) {
        Ok(s) => s,
        Err(_) => return false, // process gone
    };
    if comm.trim() != "sleep" {
        return false; // pid recycled to a different program
    }
    let status_path = format!("/proc/{pid}/status");
    let status = match std::fs::read_to_string(&status_path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    // Find the `State:` line; format `State:\tR (running)` etc.
    let state_char = status
        .lines()
        .find(|l| l.starts_with("State:"))
        .and_then(|l| l.trim_start_matches("State:").trim().chars().next());
    !matches!(state_char, Some('Z' | 'X'))
}

/// Wait up to `deadline` for the sleep grandchild at `pid` to no
/// longer be live (per `sleep_grandchild_is_live`). Returns `true`
/// if the grandchild was reaped within the deadline, `false` if it
/// was still live when the deadline expired.
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
async fn stop_escalates_to_sigkill_when_sigterm_ignored() {
    let cgroup_root = TempDir::new().expect("tempdir created");
    std::fs::create_dir_all(cgroup_root.path().join("overdrive.slice/workloads.slice"))
        .expect("workloads.slice created");

    // Custom stop-grace duration to keep the test fast — 250ms.
    let driver: Arc<dyn Driver> = Arc::new(
        ExecDriver::new(cgroup_root.path().to_path_buf())
            .with_stop_grace(Duration::from_millis(250)),
    );

    let alloc = AllocationId::new("alloc-stop-sigkill").expect("valid alloc id");
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
        // Filter to live `sleep` processes only — the shell may have
        // transient ancillary children depending on the libc.
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

    // The stop must have waited at least the grace window before
    // escalating, but not by orders of magnitude.
    assert!(
        elapsed >= Duration::from_millis(250),
        "stop returned faster than the configured grace ({elapsed:?})"
    );
    assert!(elapsed < Duration::from_secs(10), "stop did not escalate within budget ({elapsed:?})");

    let state = driver.status(&handle).await.expect("status succeeds");
    assert_eq!(state, AllocationState::Terminated);

    // Crucially: the reparented `sleep` grandchild MUST also be reaped.
    //
    // The shell's tokio `Child` handle only addresses the shell PID;
    // when `child.start_kill()` SIGKILL-s the shell, the `sleep`
    // grandchild reparents to init and survives. The driver follows
    // up with `send_sigkill_pgrp(pid)` — `kill(-pid, SIGKILL)` —
    // which addresses the entire process group (the child was
    // `setsid`-ed at spawn so PGID == shell PID). On real cgroupfs
    // (Lima/LVH/production) the parallel `cgroup.kill` write reaches
    // the same grandchildren via the kernel; on this test's TempDir
    // root, `cgroup.kill` is a regular file and the write never
    // reaches the kernel — so `send_sigkill_pgrp` is the ONLY
    // mechanism that reaps the grandchild here.
    //
    // This assertion pins both:
    //   * `send_sigkill_pgrp -> ()` (no-op body) — grandchild survives.
    //   * `kill(-raw, SIGKILL)` → `kill(raw, SIGKILL)` (drop the
    //     negation) — only the leader is signalled, grandchild
    //     survives. The leader is already dead via `start_kill` so
    //     the positive-PID kill is a no-op against a corpse.
    for sleep_pid in &sleep_pids {
        let reaped = await_sleep_grandchild_reaped(*sleep_pid, Duration::from_secs(5)).await;
        assert!(
            reaped,
            "sleep grandchild at pid={sleep_pid} must be reaped after stop(); \
             still live in /proc — process-group SIGKILL escalation did not reach it",
        );
    }
}
