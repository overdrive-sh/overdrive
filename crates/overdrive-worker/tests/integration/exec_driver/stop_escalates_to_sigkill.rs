//! US-02 Scenario 2.7 — `Driver::stop` escalates to SIGKILL when the
//! workload ignores SIGTERM beyond the grace window.
//!
//! @real-io — Linux. The workload is a `/bin/sh -c 'trap "" TERM; ...'`
//! that ignores SIGTERM. After the grace window elapses, the driver
//! sends SIGKILL; the test asserts the process is reaped and the
//! state advances to `Terminated`.

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
}
