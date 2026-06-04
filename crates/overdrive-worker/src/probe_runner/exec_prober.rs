//! `CgroupExecProber` — production binding of `ExecProber` that spawns
//! a probe subprocess and places the spawned PID inside the workload's
//! cgroup scope via `place_pid_in_scope` (reuse from `cgroup_manager`
//! per ADR-0059 §2 DDD-17 / ADR-0026).
//!
//! Per ADR-0059 + DDD-17: `cgroup.procs` write (NOT `clone3 +
//! CLONE_INTO_CGROUP`; deferred to Phase 2+ pending
//! `nix-rust/nix#2120` / P3-Q12). Per DDD-18: timeout cleanup uses
//! `cgroup.kill` (Linux 5.14+) which mass-SIGKILLs every task in the
//! scope — including any fork descendants the probe spawned.
//!
//! Classification table (ExecProber trait postcondition):
//!
//! | Outcome | `ProbeOutcome` |
//! |---|---|
//! | exit 0 within timeout | `Pass` |
//! | exit N≠0 within timeout | `Fail { reason: "exit N" }` |
//! | binary not found / not executable | `Fail { reason: "exec: command not found" }` |
//! | timeout (SIGKILL via cgroup.kill) | `Fail { reason: "timeout after <N>s" }` |
//! | cgroup-placement error (ENOSPC/EACCES/ENOENT/EBUSY) | `Err(ProbeFailure::ExecSpawnFailed)` |
//!
//! The `reason` strings are the operator-facing contract per
//! [`ProbeOutcome::Fail`]'s docstring — renaming them is a wire-shape
//! change. Cgroup-placement errors surface as
//! [`ProbeFailure::ExecSpawnFailed`] per ADR-0054 §3 QR2 and are NOT
//! auto-retried by the runner.
//!
//! Per ADR-0059 §2 the Sim adapter
//! (`crates/overdrive-sim/src/adapters/probers.rs::SimExecProber`)
//! does NOT assert cgroup membership — that is the production-adapter
//! contract, pinned by the Tier-3 integration test
//! `crates/overdrive-worker/tests/integration/probe_runner/
//! real_exec_probe_cgroup.rs`.

#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::module_name_repetitions,
    clippy::missing_errors_doc,
    reason = "shared docstring style for the ProbeRunner subsystem"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::traits::cgroup_fs::CgroupFs;
use overdrive_core::traits::prober::{ExecProber, ProbeFailure, ProbeOutcome};

use crate::cgroup_manager::{CgroupManager, CgroupPath};

/// Production `ExecProber` over `tokio::process::Command` +
/// [`CgroupManager::place_pid_in_scope`] (per ADR-0026 / ADR-0059 §2).
///
/// Holds a [`CgroupManager`] rooted at `/`: [`split_scope`] strips the
/// leading `/` from the absolute scope path the prober receives, and
/// [`CgroupPath::resolve`] re-adds it against this root, so the
/// reconstructed path is the exact absolute scope. The manager carries
/// the injected `Arc<dyn CgroupFs>` so the prober writes through the
/// SAME substrate the composition root probed (Earned Trust invariant
/// per ADR-0054 § Composition root wiring) — production passes the real
/// cgroupfs adapter, tests pass a real or Sim adapter.
pub struct CgroupExecProber {
    cgroup: CgroupManager,
}

impl CgroupExecProber {
    #[must_use]
    pub fn new(fs: Arc<dyn CgroupFs>) -> Self {
        Self { cgroup: CgroupManager::new(PathBuf::from("/"), fs) }
    }
}

/// Classify a process exit code into a [`ProbeOutcome`] per the US-03
/// AC table. Pure function — the SUT for the
/// `ExecExitCodeClassification` proptest (universe `0..=255`).
///
/// - exit `0` → [`ProbeOutcome::Pass`].
/// - exit `N != 0` → [`ProbeOutcome::Fail`] with reason `"exit N"`.
///
/// The reason string `"exit <code>"` is the operator-facing contract;
/// renaming it is a wire-shape change.
#[must_use]
pub fn classify_exit_status(code: i32) -> ProbeOutcome {
    if code == 0 {
        ProbeOutcome::Pass
    } else {
        ProbeOutcome::Fail { reason: format!("exit {code}") }
    }
}

/// Classify a process termination given its optional exit code. A
/// process that exited normally carries `Some(code)`; a process killed
/// by a signal (without our timeout firing — e.g. an external `kill`,
/// or the kernel OOM-killer) carries `None`. The signal-death case is
/// a Fail naming the absence of a clean exit, NOT a forged exit code —
/// per `.claude/rules/development.md` § "Distinct failure modes get
/// distinct error variants" a missing code must not masquerade as
/// `exit 1` / `exit -1`.
#[must_use]
pub fn classify_termination(code: Option<i32>) -> ProbeOutcome {
    code.map_or_else(
        || ProbeOutcome::Fail { reason: "killed by signal".to_owned() },
        classify_exit_status,
    )
}

/// The operator-facing `reason` string for a binary that could not be
/// `execve`'d (not on PATH inside the cgroup namespace, or no execute
/// permission). Mapped from the spawn `io::Error` kind. Fixed
/// wire-shape string per the `ExecProber` trait postcondition.
#[must_use]
pub const fn not_found_reason() -> &'static str {
    "exec: command not found"
}

/// The operator-facing `reason` string for a probe that exceeded its
/// timeout and was SIGKILLed via `cgroup.kill`. Mirrors the
/// TCP / HTTP prober shape (`"timeout after 5s"`); whole-seconds form
/// because exec-probe timeouts are declared in `timeout_seconds`.
#[must_use]
pub fn timeout_reason(timeout: Duration) -> String {
    format!("timeout after {}s", timeout.as_secs())
}

/// Split an absolute cgroup scope path string (e.g.
/// `/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-x.scope`) into
/// a `("/", CgroupPath)` pair so the existing relative-path cgroup
/// primitives (`place_pid_in_scope` / `cgroup_kill`) can be REUSED
/// verbatim per ADR-0059 §2 DDD-17. The leading `/` becomes the root;
/// the remainder is a `CgroupPath` (which rejects `..`, double slashes,
/// and re-canonicalises to the same absolute path on `resolve`).
fn split_scope(cgroup_scope_path: &str) -> Result<CgroupPath, ProbeFailure> {
    let relative = cgroup_scope_path.trim_start_matches('/');
    relative.parse::<CgroupPath>().map_err(|err| ProbeFailure::ExecSpawnFailed {
        reason: format!("invalid cgroup scope path {cgroup_scope_path:?}: {err}"),
    })
}

/// Map a spawn-time `io::Error` into either a not-found
/// [`ProbeOutcome::Fail`] (the binary could not be executed) or a
/// [`ProbeFailure::ExecSpawnFailed`] (an infrastructure error that is
/// NOT an operator-visible probe outcome). Per
/// `.claude/rules/development.md` § "Distinct failure modes get
/// distinct error variants" the kinds are NOT collapsed into one
/// neutral value.
fn classify_spawn_error(err: &std::io::Error) -> Result<ProbeOutcome, ProbeFailure> {
    use std::io::ErrorKind;
    match err.kind() {
        // The binary is not on PATH / does not exist, or is present but
        // lacks execute permission — both are "the operator's probe
        // command cannot run", an observable Fail outcome.
        ErrorKind::NotFound | ErrorKind::PermissionDenied => {
            Ok(ProbeOutcome::Fail { reason: not_found_reason().to_owned() })
        }
        // Anything else (resource exhaustion, etc.) is an
        // infrastructure failure that prevented the probe from
        // executing at all — surfaced as ExecSpawnFailed, not a probe
        // outcome.
        _ => Err(ProbeFailure::ExecSpawnFailed { reason: format!("spawn failed: {err}") }),
    }
}

#[async_trait]
impl ExecProber for CgroupExecProber {
    async fn probe(
        &self,
        command: &[String],
        cgroup_scope_path: &str,
        timeout: Duration,
    ) -> Result<ProbeOutcome, ProbeFailure> {
        // Precondition validation mirrors the sim adapter — per
        // `nw-tdd-methodology` § "Test Doubles Must Validate Inputs".
        if command.is_empty() {
            return Err(ProbeFailure::ExecSpawnFailed {
                reason: "exec probe command must be non-empty".to_owned(),
            });
        }
        if cgroup_scope_path.is_empty() {
            return Err(ProbeFailure::ExecSpawnFailed {
                reason: "exec probe cgroup_scope_path must be non-empty".to_owned(),
            });
        }

        let scope = split_scope(cgroup_scope_path)?;

        // Spawn the child. The binary is command[0]; command[1..] are
        // argv. stdout/stderr are discarded (Phase 1 — capture deferred
        // per ADR-0059). `kill_on_drop` is a belt-and-braces backstop
        // for the panic path; the timeout branch below mass-kills via
        // cgroup.kill which also reaps fork descendants.
        let mut cmd = tokio::process::Command::new(&command[0]);
        cmd.args(&command[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            // execve failure (binary not found / not executable) is an
            // observable Fail; other io errors are ExecSpawnFailed.
            Err(err) => return classify_spawn_error(&err),
        };

        // Place the PID into the workload's cgroup scope. There is a
        // narrow race window where the child runs in the parent's
        // cgroup before placement (DDD-17 trade-off: clone3 +
        // CLONE_INTO_CGROUP deferred per nix#2120 / P3-Q12); a
        // long-lived probe child is observed in the alloc scope by the
        // Tier-3 membership test. A placement error (ENOSPC / EACCES /
        // ENOENT / EBUSY) is an infrastructure failure per ADR-0054
        // §3 QR2 — kill the child and surface ExecSpawnFailed (NOT
        // auto-retried by the runner).
        if let Some(pid) = child.id()
            && let Err(err) = self.cgroup.place_pid_in_scope(&scope, pid).await
        {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(ProbeFailure::ExecSpawnFailed {
                reason: format!("cgroup placement failed: {err}"),
            });
        }

        // Wait for exit within the timeout. On timeout, mass-SIGKILL
        // the cgroup (reaps the child AND any fork descendants) and
        // reap the child handle so no zombie survives.
        match tokio::time::timeout(timeout, child.wait()).await {
            // Process terminated within timeout — classify on exit
            // code, or name signal-death when there is no clean code.
            Ok(Ok(status)) => Ok(classify_termination(status.code())),
            // `child.wait()` itself errored — infrastructure failure.
            Ok(Err(err)) => {
                Err(ProbeFailure::ExecSpawnFailed { reason: format!("wait failed: {err}") })
            }
            // Timeout elapsed. SIGKILL the whole cgroup via cgroup.kill
            // (DDD-18); fall back to the tokio Child kill so the handle
            // is reaped even if the cgroup write is a no-op on the
            // happy path.
            Err(_elapsed) => {
                let _ = self.cgroup.cgroup_kill(&scope).await;
                let _ = child.kill().await;
                let _ = child.wait().await;
                Ok(ProbeOutcome::Fail { reason: timeout_reason(timeout) })
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code per workspace convention")]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn classify_exit_zero_is_pass_others_named() {
        // Pins the zero boundary against sign-flip / off-by-one mutants
        // on the exit-status guard.
        assert_eq!(classify_exit_status(0), ProbeOutcome::Pass);
        assert_eq!(classify_exit_status(1), ProbeOutcome::Fail { reason: "exit 1".to_owned() });
        assert_eq!(classify_exit_status(255), ProbeOutcome::Fail { reason: "exit 255".to_owned() });
        assert_eq!(classify_exit_status(-1), ProbeOutcome::Fail { reason: "exit -1".to_owned() });
    }

    #[test]
    fn classify_termination_distinguishes_clean_exit_from_signal_death() {
        // Clean exit delegates to classify_exit_status.
        assert_eq!(classify_termination(Some(0)), ProbeOutcome::Pass);
        assert_eq!(
            classify_termination(Some(3)),
            ProbeOutcome::Fail { reason: "exit 3".to_owned() }
        );
        // No exit code (signal death) is a distinct named Fail — NOT a
        // forged `exit -1` / `exit 1`. Pins the None branch against the
        // fallback-value mutant.
        assert_eq!(
            classify_termination(None),
            ProbeOutcome::Fail { reason: "killed by signal".to_owned() }
        );
    }

    #[test]
    fn timeout_reason_renders_whole_seconds() {
        assert_eq!(timeout_reason(Duration::from_secs(2)), "timeout after 2s");
        assert_eq!(timeout_reason(Duration::from_secs(0)), "timeout after 0s");
    }

    #[test]
    fn not_found_reason_is_fixed_wire_string() {
        assert_eq!(not_found_reason(), "exec: command not found");
    }

    #[test]
    fn spawn_not_found_and_permission_map_to_fail_others_to_spawn_failed() {
        let not_found = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(
            classify_spawn_error(&not_found).unwrap(),
            ProbeOutcome::Fail { reason: "exec: command not found".to_owned() }
        );
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            classify_spawn_error(&denied).unwrap(),
            ProbeOutcome::Fail { reason: "exec: command not found".to_owned() }
        );
        let other = std::io::Error::other("resource exhausted");
        assert!(
            matches!(classify_spawn_error(&other), Err(ProbeFailure::ExecSpawnFailed { .. })),
            "non-exec io errors surface as ExecSpawnFailed, not a probe outcome"
        );
    }

    #[test]
    fn split_scope_round_trips_absolute_path() {
        let scope =
            split_scope("/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-x.scope").unwrap();
        assert_eq!(
            scope.resolve(Path::new("/")),
            Path::new("/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-x.scope")
        );
    }

    #[test]
    fn split_scope_rejects_traversal() {
        assert!(split_scope("/sys/../etc/passwd").is_err());
    }
}
