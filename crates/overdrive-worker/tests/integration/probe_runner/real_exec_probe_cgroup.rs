//! Tier 3 integration — `CgroupExecProber` against a real cgroup
//! scope inside Lima.
//!
//! Slice 02 (step 02-02 / US-03). @real-io — Linux + cgroup v2 +
//! root (via `cargo xtask lima run --`) required.
//!
//! Per ADR-0059 §1 + ADR-0054 §3 ExecProber postcondition: the
//! spawned probe process MUST be a member of the workload's cgroup
//! scope per its `/proc/<pid>/cgroup` readout — the load-bearing
//! prod/sim divergence invariant (S-SHCP-INT-03-03). The Sim adapter
//! does NOT assert membership (Tier 1 concern); only the production
//! `CgroupExecProber` places the PID via `place_pid_in_scope`.
//!
//! Per `.claude/rules/testing.md` § "Running tests — Lima VM" the
//! cgroup-write path needs root via `cargo xtask lima run --` (the
//! wrapper defaults to running as root). Lima sudo is the canonical
//! shape per ADR-0034.
//!
//! Cgroup-leak hygiene: every scope created here is guarded by an
//! `AllocCleanup`-shape RAII guard (see `super::super::exec_driver::
//! cleanup::AllocCleanup`) whose Drop fires `cgroup.kill` + reaps +
//! rmdirs the scope. After a crash / SIGKILL the suite re-runs
//! cleanly without manual cleanup.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    reason = "operator-readable test module docs naming ADR refs + cgroup paths"
)]

use std::path::Path;
use std::time::{Duration, Instant};

use overdrive_core::id::AllocationId;
use overdrive_core::traits::prober::{ExecProber, ProbeOutcome};
use overdrive_worker::cgroup_manager::{CgroupPath, create_workloads_slice_with_controllers};
use overdrive_worker::probe_runner::CgroupExecProber;
use serial_test::serial;

use crate::integration::exec_driver::cleanup::AllocCleanup;

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// Bootstrap the workloads.slice + create the alloc scope dir, returning
/// the absolute scope path string the `ExecProber` API consumes.
async fn make_scope(alloc: &AllocationId) -> String {
    let cgroup_root = Path::new(CGROUP_ROOT);
    create_workloads_slice_with_controllers(cgroup_root)
        .expect("workloads.slice bootstrap succeeds");
    let scope = CgroupPath::for_alloc(alloc);
    let dir = scope.resolve(cgroup_root);
    tokio::fs::create_dir_all(&dir).await.expect("create alloc scope dir");
    dir.to_string_lossy().into_owned()
}

/// S-SHCP-INT-03-01 (US-03 / K1) — exec probe spawns `/bin/true`
/// inside a real test cgroup scope; exit 0 → Pass.
#[tokio::test]
#[serial(cgroup)]
async fn given_real_cgroup_scope_when_cgroup_exec_prober_runs_bin_true_then_returns_pass() {
    let alloc = AllocationId::new("alloc-exec-probe-true").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(Path::new(CGROUP_ROOT).to_path_buf(), alloc.clone());
    let scope_path = make_scope(&alloc).await;

    let prober = CgroupExecProber::new();
    let outcome = prober
        .probe(&["/bin/true".to_owned()], &scope_path, Duration::from_secs(5))
        .await
        .expect("probe executes against real cgroup scope");

    assert_eq!(outcome, ProbeOutcome::Pass, "/bin/true exits 0 → Pass");
}

/// S-SHCP-INT-03-02 (US-03 / K1) — `/bin/sleep 3600` with
/// `timeout_seconds = 2` is SIGKILLed at the 2s boundary via
/// `cgroup.kill`; ProbeOutcome Fail with `timeout after 2s`, and the
/// child is reaped within bounded time (the scope's `cgroup.procs` is
/// empty after the probe returns).
#[tokio::test]
#[serial(cgroup)]
async fn given_real_cgroup_scope_when_cgroup_exec_prober_times_out_then_sigkill_and_fail_named() {
    let alloc = AllocationId::new("alloc-exec-probe-timeout").expect("valid alloc id");
    let cgroup_root = Path::new(CGROUP_ROOT);
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let scope_path = make_scope(&alloc).await;

    let prober = CgroupExecProber::new();
    let started = Instant::now();
    let outcome = prober
        .probe(&["/bin/sleep".to_owned(), "3600".to_owned()], &scope_path, Duration::from_secs(2))
        .await
        .expect("probe executes against real cgroup scope");
    let elapsed = started.elapsed();

    assert_eq!(
        outcome,
        ProbeOutcome::Fail { reason: "timeout after 2s".to_owned() },
        "sleep 3600 with 2s timeout → Fail \"timeout after 2s\""
    );
    // The probe returns shortly after the 2s boundary — it does not
    // block for the full 3600s sleep. Generous upper bound to absorb
    // scheduler jitter on the CI runner.
    assert!(
        elapsed < Duration::from_secs(30),
        "probe returned bounded after timeout (got {elapsed:?}), did not block on the sleep"
    );

    // `cgroup.kill` fired and reaped the child: the scope's
    // cgroup.procs is empty after the probe returns. This is the
    // observable kernel side effect (per Tier-3 assertion rules — we
    // assert on cgroup.procs, not on internal SIGKILL reachability).
    let scope_dir = Path::new(&scope_path);
    let procs = tokio::fs::read_to_string(scope_dir.join("cgroup.procs"))
        .await
        .expect("cgroup.procs readable");
    let live: Vec<&str> = procs.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(live.is_empty(), "cgroup.kill reaped the workload; cgroup.procs empty, got {live:?}");
}

/// S-SHCP-INT-03-03 (US-03 AC — cgroup membership) — the spawned probe
/// process is a member of `alloc-<id>.scope`, NOT the worker's scope.
/// The structural defense against ADR-0059 §2 sim/prod divergence: the
/// production adapter places the probe child in the WORKLOAD's cgroup.
///
/// Observed mid-flight: the probe runs `/bin/sleep 10` in a spawned
/// task; the test polls the scope's `cgroup.procs` for the live PID,
/// then reads `/proc/<pid>/cgroup` and asserts it names
/// `alloc-<id>.scope`. The probe is then cancelled by the timeout.
#[tokio::test]
#[serial(cgroup)]
async fn given_real_cgroup_scope_when_cgroup_exec_prober_runs_then_pid_membership_matches_scope() {
    let alloc = AllocationId::new("alloc-exec-probe-member").expect("valid alloc id");
    let cgroup_root = Path::new(CGROUP_ROOT);
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let scope_path = make_scope(&alloc).await;
    let scope_substring = format!("{alloc}.scope");

    // Run the probe against a long-ish sleep so the child is alive
    // while we observe membership. A 4s timeout caps the probe; we
    // observe within the first second.
    let prober = CgroupExecProber::new();
    let scope_for_probe = scope_path.clone();
    let probe_task = tokio::spawn(async move {
        prober
            .probe(
                &["/bin/sleep".to_owned(), "10".to_owned()],
                &scope_for_probe,
                Duration::from_secs(4),
            )
            .await
    });

    // Poll cgroup.procs for the placed PID. Production writes the PID
    // into <scope>/cgroup.procs via place_pid_in_scope shortly after
    // spawn; give it a bounded window.
    let scope_dir = Path::new(&scope_path);
    let procs_path = scope_dir.join("cgroup.procs");
    let mut pid: Option<u32> = None;
    for _ in 0..200 {
        if let Ok(contents) = tokio::fs::read_to_string(&procs_path).await {
            if let Some(first) = contents.lines().find_map(|l| l.trim().parse::<u32>().ok()) {
                pid = Some(first);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let pid = pid.expect("probe child PID landed in the alloc scope's cgroup.procs");

    // The load-bearing assertion: /proc/<pid>/cgroup names the alloc
    // scope, NOT the worker (test-process) scope. cgroup v2 emits a
    // single `0::<path>` line.
    let proc_cgroup = tokio::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .await
        .expect("/proc/<pid>/cgroup readable while child alive");
    assert!(
        proc_cgroup.contains(&scope_substring),
        "probe PID {pid} must be a member of {scope_substring}; \
         /proc/{pid}/cgroup was {proc_cgroup:?}"
    );

    // Let the probe complete (it will time out at 4s and SIGKILL the
    // sleep via cgroup.kill). Awaiting drains the task cleanly so the
    // AllocCleanup guard finds an empty scope on the happy path.
    let outcome = probe_task.await.expect("probe task joins").expect("probe executes");
    assert_eq!(
        outcome,
        ProbeOutcome::Fail { reason: "timeout after 4s".to_owned() },
        "sleep 10 with 4s timeout → Fail \"timeout after 4s\""
    );
}
