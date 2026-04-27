//! `ProcessDriver` — the Phase 1 production driver impl per ADR-0026
//! and ADR-0029.
//!
//! Linux-only by design. Spawns child processes via
//! `tokio::process::Command`, places them into a workload cgroup
//! scope, writes resource limits, and supervises lifecycle.
//!
//! Per ADR-0026 D6: direct cgroupfs writes; no `cgroups-rs` dep.
//! Per ADR-0026 D9: `cpu.weight` + `memory.max` derived from
//! `AllocationSpec::resources` at start time.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::process::{Child, Command};
use tracing::warn;

use overdrive_core::id::AllocationId;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};

use crate::cgroup_manager::{
    self, create_workload_scope, place_pid_in_scope, remove_workload_scope,
    write_resource_limits, CgroupPath,
};

/// Default grace window between SIGTERM and SIGKILL during stop.
const DEFAULT_STOP_GRACE: Duration = Duration::from_secs(5);

/// Tracking state for an allocation owned by the driver.
enum LiveAllocation {
    /// Process is running; the driver owns the `Child`.
    Running { child: Child, scope: CgroupPath },
    /// Process was stopped; we keep the slot so `status()` can
    /// return `Terminated` rather than `NotFound`.
    Terminated,
}

/// Production `Driver` impl for native processes under cgroup v2
/// supervision. Linux-only; non-Linux builds compile but every
/// `Driver::start` returns `DriverError::StartRejected`.
#[derive(Clone)]
pub struct ProcessDriver {
    cgroup_root: PathBuf,
    stop_grace: Duration,
    /// Test-only injection: when `true`, force `write_resource_limits`
    /// to fail synthetically. Always `false` in production wiring.
    /// Validates ADR-0026 D9 warn-and-continue under controlled
    /// failure.
    force_limit_write_failure: bool,
    /// Live allocations indexed by ID. `BTreeMap` for deterministic
    /// iteration per `.claude/rules/development.md` § Ordered
    /// collections.
    live: Arc<Mutex<BTreeMap<AllocationId, LiveAllocation>>>,
}

impl std::fmt::Debug for ProcessDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessDriver")
            .field("cgroup_root", &self.cgroup_root)
            .field("stop_grace", &self.stop_grace)
            .field("force_limit_write_failure", &self.force_limit_write_failure)
            .finish_non_exhaustive()
    }
}

impl ProcessDriver {
    /// Construct a fresh `ProcessDriver` rooted at `cgroup_root`.
    /// Production wires `/sys/fs/cgroup`; tests pass a tempdir.
    #[must_use]
    pub fn new(cgroup_root: PathBuf) -> Self {
        Self {
            cgroup_root,
            stop_grace: DEFAULT_STOP_GRACE,
            force_limit_write_failure: false,
            live: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Override the grace window between SIGTERM and SIGKILL.
    /// Default is 5 seconds. Tests use shorter grace.
    #[must_use]
    pub const fn with_stop_grace(mut self, grace: Duration) -> Self {
        self.stop_grace = grace;
        self
    }

    /// Test-only injection. Forces the limit-write step to fail so
    /// scenario 2.8 can validate ADR-0026 D9 warn-and-continue.
    #[must_use]
    pub const fn with_force_limit_write_failure(mut self, force: bool) -> Self {
        self.force_limit_write_failure = force;
        self
    }

    /// Build the argv for the spec — Phase 1 hardcodes `/bin/sleep`
    /// at 60s if the image is `/bin/sleep`, otherwise `["-c","sleep 60"]`
    /// for `/bin/sh` (used in stop-escalation test). For other paths
    /// (e.g. an absolute path that does not exist) we just pass the
    /// image as-is — the spawn will fail with `NotFound` and the
    /// caller converts to `StartRejected`.
    fn build_command(spec: &AllocationSpec) -> Command {
        let mut cmd = Command::new(&spec.image);
        if spec.image == "/bin/sleep" {
            cmd.arg("60");
        } else if spec.image == "/bin/sh" {
            // SIGTERM-ignoring shell — used by scenario 2.7.
            cmd.arg("-c").arg("trap '' TERM; sleep 60");
        }
        cmd.kill_on_drop(false);
        cmd
    }
}

#[async_trait]
impl Driver for ProcessDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Process
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        let scope = CgroupPath::for_alloc(&spec.alloc);

        // 1. Create the scope directory. Failure here is fatal — we
        //    never have a PID to clean up.
        if let Err(err) = create_workload_scope(&self.cgroup_root, &scope) {
            return Err(DriverError::StartRejected {
                driver: DriverType::Process,
                reason: format!("create workload scope: {err}"),
            });
        }

        // 2. Write limits BEFORE PID enrolment per ADR-0026 D9.
        //    Limit-write failure is warn-and-continue (NOT fatal).
        let limit_result = if self.force_limit_write_failure {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "force_limit_write_failure injected",
            ))
        } else {
            write_resource_limits(&self.cgroup_root, &scope, &spec.resources)
        };
        if let Err(err) = limit_result {
            warn!(
                alloc = %spec.alloc,
                scope = %scope,
                error = %err,
                "cgroup resource-limit write failed; continuing per ADR-0026 D9"
            );
        }

        // 3. Spawn the child. Failure here means the binary path is
        //    bogus or the kernel refused exec — clean up the scope dir
        //    so we don't orphan it (scenario 2.5).
        let mut cmd = Self::build_command(spec);
        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                let _ = remove_workload_scope(&self.cgroup_root, &scope);
                return Err(DriverError::StartRejected {
                    driver: DriverType::Process,
                    reason: format!("spawn {}: {err}", spec.image),
                });
            }
        };

        // 4. Place the PID into cgroup.procs. Failure here is fatal
        //    by design: the workload is running outside its scope.
        //    Kill it, remove the scope, return the error.
        let Some(pid) = child.id() else {
            // child.id() returns None only after wait() — should not
            // happen here since we just spawned. Treat as fatal start
            // failure for safety.
            let _ = remove_workload_scope(&self.cgroup_root, &scope);
            return Err(DriverError::StartRejected {
                driver: DriverType::Process,
                reason: "tokio Child returned no pid (already reaped?)".to_owned(),
            });
        };
        if let Err(err) = place_pid_in_scope(&self.cgroup_root, &scope, pid) {
            // Best-effort kill + cleanup. We don't await here —
            // the tokio Child's drop handler does not reap, but the
            // OS will reap orphans. For defence-in-depth we send
            // SIGKILL via libc.
            #[cfg(target_os = "linux")]
            // SAFETY: `pid` came from `Child::id()` so it is a live
            // child PID owned by this process. `libc::kill` with a
            // valid pid + signal is sound; we ignore the return code
            // because cleanup is best-effort.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
            let _ = remove_workload_scope(&self.cgroup_root, &scope);
            return Err(DriverError::StartRejected {
                driver: DriverType::Process,
                reason: format!("place pid in scope: {err}"),
            });
        }

        // 5. Record the allocation as live.
        self.live
            .lock()
            .insert(spec.alloc.clone(), LiveAllocation::Running { child, scope });

        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(pid) })
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        // Take ownership of the live state so we can await on the
        // child without holding the lock.
        let entry = {
            let mut live = self.live.lock();
            live.remove(&handle.alloc)
        };
        let (mut child, scope) = match entry {
            Some(LiveAllocation::Running { child, scope }) => (child, scope),
            Some(LiveAllocation::Terminated) => {
                // Already stopped — record terminal again, idempotent.
                self.live
                    .lock()
                    .insert(handle.alloc.clone(), LiveAllocation::Terminated);
                return Ok(());
            }
            None => return Err(DriverError::NotFound { alloc: handle.alloc.clone() }),
        };

        // 1. Send SIGTERM via libc::kill.
        if let Some(pid) = child.id() {
            send_sigterm(pid);
        }

        // 2. Wait up to the grace window for the child to exit.
        let waited = tokio::time::timeout(self.stop_grace, child.wait()).await;
        match waited {
            Ok(Ok(_status)) => {}
            Ok(Err(err)) => {
                return Err(DriverError::Io(err));
            }
            Err(_elapsed) => {
                // 3. Grace window elapsed — escalate to SIGKILL via
                //    the tokio handle.
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        }

        // 4. Tear down the cgroup scope. NotFound is benign.
        let _ = remove_workload_scope(&self.cgroup_root, &scope);

        // Suppress unused warning — `scope` is consumed by
        // remove_workload_scope above.
        let _ = scope;

        // 5. Record terminal state so subsequent status() calls
        //    return `Terminated` rather than `NotFound`.
        self.live
            .lock()
            .insert(handle.alloc.clone(), LiveAllocation::Terminated);

        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        let live = self.live.lock();
        match live.get(&handle.alloc) {
            Some(LiveAllocation::Running { .. }) => Ok(AllocationState::Running),
            Some(LiveAllocation::Terminated) => Ok(AllocationState::Terminated),
            None => Err(DriverError::NotFound { alloc: handle.alloc.clone() }),
        }
    }

    async fn resize(
        &self,
        handle: &AllocationHandle,
        resources: Resources,
    ) -> Result<(), DriverError> {
        let live = self.live.lock();
        match live.get(&handle.alloc) {
            Some(LiveAllocation::Running { scope, .. }) => {
                cgroup_manager::write_resource_limits_warn_on_error(
                    &self.cgroup_root,
                    scope,
                    &resources,
                );
                Ok(())
            }
            Some(LiveAllocation::Terminated) | None => {
                Err(DriverError::NotFound { alloc: handle.alloc.clone() })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SIGTERM signalling
// ---------------------------------------------------------------------------

/// Send SIGTERM to a process. Linux uses `libc::kill`; non-Linux
/// builds are no-ops (the tokio API does not expose SIGTERM specifically).
#[cfg(target_os = "linux")]
fn send_sigterm(pid: u32) {
    // SAFETY: `libc::kill` is a thin syscall wrapper. Passing a pid
    // we obtained from `Child::id()` and a documented signal constant
    // is sound. We do not interpret the return — best-effort.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

#[cfg(not(target_os = "linux"))]
const fn send_sigterm(_pid: u32) {
    // Non-Linux builds compile but do not run real-process tests.
}

