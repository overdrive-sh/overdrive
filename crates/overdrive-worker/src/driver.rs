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
    self, CgroupPath, cgroup_kill, create_workload_scope, place_pid_in_scope,
    remove_workload_scope, write_resource_limits,
};

/// Default grace window between SIGTERM and SIGKILL during stop.
const DEFAULT_STOP_GRACE: Duration = Duration::from_secs(5);

/// Construct a `DriverError::StartRejected` for the process driver. The
/// `driver: DriverType::Process` discriminator is fixed by construction,
/// so the call sites only need to supply the human-readable reason. Used
/// by every fallible step in `Driver::start`.
fn start_rejected(reason: impl Into<String>) -> DriverError {
    DriverError::StartRejected { driver: DriverType::Process, reason: reason.into() }
}

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
    /// Per ADR-0028: when `true`, `Driver::start` SKIPS workload
    /// cgroup operations (scope creation, PID placement, limit
    /// writes, scope removal). Workloads run as ordinary child
    /// processes under the running UID with no cgroup isolation.
    /// Plumbed from `--allow-no-cgroups` at the CLI boundary.
    /// Production deployments leave this `false`.
    allow_no_cgroups: bool,
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
            .field("allow_no_cgroups", &self.allow_no_cgroups)
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
            allow_no_cgroups: false,
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

    /// Per ADR-0028: when `allow` is `true`, subsequent `Driver::start`
    /// calls SKIP workload-cgroup operations (scope creation, PID
    /// placement, limit writes, scope removal). The control-plane
    /// `--allow-no-cgroups` flag plumbs into this constructor knob.
    /// Production deployments leave this `false`.
    #[must_use]
    pub const fn with_allow_no_cgroups(mut self, allow: bool) -> Self {
        self.allow_no_cgroups = allow;
        self
    }

    /// Build the argv for the spec.
    ///
    /// Phase 1 hardcodes a small set of image-name conventions. This
    /// stands in for a real container/runtime resolver (Phase 2+):
    ///
    /// | Image | Behaviour |
    /// |---|---|
    /// | `/bin/sleep` | spawn `sleep 60` (canonical happy path) |
    /// | `/bin/sh` | SIGTERM-ignoring shell (`trap '' TERM; sleep 60`); used by stop-escalation tests |
    /// | `/bin/cpuburn` | spawn a CPU-busy `/bin/sh` loop that pegs every CPU until killed; used by the cgroup-isolation burst test (slice 4 scenario 4.2) |
    /// | other | passed as-is — spawn will fail with `NotFound` and the caller converts to `StartRejected` |
    ///
    /// The `cpuburn` shape is a deliberate Phase-1 affordance: the
    /// burst test needs a workload that consumes 100% CPU on every
    /// online core to disprove §4 paper-guarantees; baking the busy
    /// loop into the driver avoids depending on `stress(1)` (not
    /// installed in the Lima image by default).
    fn build_command(spec: &AllocationSpec) -> Command {
        let mut cmd = if spec.image == "/bin/cpuburn" {
            // Pin every online core. `nproc` returns the count; the
            // shell forks one busy loop per CPU and `wait`s. SIGKILL
            // via cgroup.kill or process-group reaches the entire
            // group at teardown.
            let mut c = Command::new("/bin/sh");
            c.arg("-c").arg("for i in $(seq 1 $(nproc)); do (while :; do :; done) & done; wait");
            c
        } else {
            Command::new(&spec.image)
        };
        if spec.image == "/bin/sleep" {
            cmd.arg("60");
        } else if spec.image == "/bin/sh" {
            // SIGTERM-ignoring shell — used by scenario 2.7.
            cmd.arg("-c").arg("trap '' TERM; sleep 60");
        }
        cmd.kill_on_drop(false);

        // Put the child in a fresh process group so the driver can
        // reach reparented grandchildren via `kill(-pgid, SIGKILL)`.
        // Without this, a `/bin/sh -c 'trap ""; sleep 60'` shell that
        // gets SIGKILL'd reparents its `sleep` child to init, and the
        // tokio `Child` handle has no way to find it. `cgroup.kill`
        // covers production (real cgroupfs); the process-group fallback
        // covers the integration tests, which mount a `tempfile::TempDir`
        // as a fake cgroupfs root where `cgroup.kill` is a no-op file
        // write. Linux-only — `pre_exec` is `unsafe` because the closure
        // runs between fork and exec, where the contract is to call
        // only async-signal-safe functions; `setsid(2)` is on the
        // POSIX async-signal-safe list.
        #[cfg(target_os = "linux")]
        {
            // SAFETY: `setsid` is async-signal-safe; the closure is
            // executed in the forked child between fork and exec, no
            // shared state is touched.
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }

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

        // Per ADR-0028 dev escape hatch: when `--allow-no-cgroups` is
        // set, skip every cgroup operation. The workload runs as an
        // ordinary child process with no cgroup scope of its own.
        // Lifecycle tracking still flows through `LiveAllocation` so
        // status/stop work correctly.
        if self.allow_no_cgroups {
            let mut cmd = Self::build_command(spec);
            let child = cmd
                .spawn()
                .map_err(|err| start_rejected(format!("spawn {}: {err}", spec.image)))?;
            let pid = child.id();
            self.live.lock().insert(spec.alloc.clone(), LiveAllocation::Running { child, scope });
            return Ok(AllocationHandle { alloc: spec.alloc.clone(), pid });
        }

        // 1. Create the scope directory. Failure here is fatal — we
        //    never have a PID to clean up.
        if let Err(err) = create_workload_scope(&self.cgroup_root, &scope).await {
            return Err(start_rejected(format!("create workload scope: {err}")));
        }

        // 2. Write limits BEFORE PID enrolment per ADR-0026 D9.
        //    Limit-write failure is warn-and-continue (NOT fatal).
        let limit_result = if self.force_limit_write_failure {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "force_limit_write_failure injected",
            ))
        } else {
            write_resource_limits(&self.cgroup_root, &scope, &spec.resources).await
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
                let _ = remove_workload_scope(&self.cgroup_root, &scope).await;
                return Err(start_rejected(format!("spawn {}: {err}", spec.image)));
            }
        };

        // 4. Place the PID into cgroup.procs. Failure here is fatal
        //    by design: the workload is running outside its scope.
        //    Kill it, remove the scope, return the error.
        let Some(pid) = child.id() else {
            // child.id() returns None only after wait() — should not
            // happen here since we just spawned. Treat as fatal start
            // failure for safety.
            let _ = remove_workload_scope(&self.cgroup_root, &scope).await;
            return Err(start_rejected("tokio Child returned no pid (already reaped?)"));
        };
        if let Err(err) = place_pid_in_scope(&self.cgroup_root, &scope, pid).await {
            // Best-effort kill + cleanup. We don't await here —
            // the tokio Child's drop handler does not reap, but the
            // OS will reap orphans. For defence-in-depth we send
            // SIGKILL via libc.
            #[cfg(target_os = "linux")]
            // SAFETY: `pid` came from `Child::id()` so it is a live
            // child PID owned by this process. `libc::kill` with a
            // valid pid + signal is sound; we ignore the return code
            // because cleanup is best-effort. PIDs fit in pid_t; if
            // conversion somehow fails (theoretical), skip the kill.
            unsafe {
                if let Ok(raw) = libc::pid_t::try_from(pid) {
                    libc::kill(raw, libc::SIGKILL);
                }
            }
            let _ = remove_workload_scope(&self.cgroup_root, &scope).await;
            return Err(start_rejected(format!("place pid in scope: {err}")));
        }

        // 5. Record the allocation as live.
        self.live.lock().insert(spec.alloc.clone(), LiveAllocation::Running { child, scope });

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
                self.live.lock().insert(handle.alloc.clone(), LiveAllocation::Terminated);
                return Ok(());
            }
            None => return Err(DriverError::NotFound { alloc: handle.alloc.clone() }),
        };

        // Capture the PID before any wait — `Child::id()` returns
        // `None` once the child is reaped, but we still need it to
        // address the process group at cleanup time.
        let pid_for_pgrp_kill = child.id();

        // 1. Send SIGTERM via libc::kill.
        if let Some(pid) = pid_for_pgrp_kill {
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

        // 4. Mass-kill any reparented grandchildren. /bin/sh-class
        //    workloads fork helpers (e.g. `/bin/sleep`) that reparent
        //    to init when the shell dies; the tokio `Child` only
        //    tracks the parent. Two complementary mechanisms:
        //
        //    a) `cgroup.kill` (real cgroupfs) — atomic SIGKILL of every
        //       task in the workload's scope.
        //    b) Process-group SIGKILL (TempDir test path, where
        //       `cgroup.kill` is a regular file write that doesn't
        //       reach the kernel). The child was `setsid`-ed at spawn
        //       so its PGID = its PID; `kill(-pid, SIGKILL)` reaches
        //       every member of that group regardless of what the
        //       fake-cgroupfs root happens to be.
        if let Some(pid) = pid_for_pgrp_kill {
            send_sigkill_pgrp(pid);
        }
        if !self.allow_no_cgroups {
            let _ = cgroup_kill(&self.cgroup_root, &scope).await;
            // 5. Tear down the cgroup scope. NotFound is benign.
            let _ = remove_workload_scope(&self.cgroup_root, &scope).await;
        }

        // 6. Record terminal state so subsequent status() calls
        //    return `Terminated` rather than `NotFound`.
        self.live.lock().insert(handle.alloc.clone(), LiveAllocation::Terminated);

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
        // Clone the scope out under the lock and drop the guard before
        // any `.await` — parking_lot mutexes must not be held across
        // suspension points (`.claude/rules/development.md`
        // § Concurrency & async).
        let scope = {
            let live = self.live.lock();
            match live.get(&handle.alloc) {
                Some(LiveAllocation::Running { scope, .. }) => scope.clone(),
                Some(LiveAllocation::Terminated) | None => {
                    return Err(DriverError::NotFound { alloc: handle.alloc.clone() });
                }
            }
        };
        cgroup_manager::write_resource_limits_warn_on_error(&self.cgroup_root, &scope, &resources)
            .await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SIGTERM / SIGKILL signalling
// ---------------------------------------------------------------------------

/// Send SIGTERM to a process. Linux uses `libc::kill`; non-Linux
/// builds are no-ops (the tokio API does not expose SIGTERM specifically).
#[cfg(target_os = "linux")]
fn send_sigterm(pid: u32) {
    // SAFETY: `libc::kill` is a thin syscall wrapper. Passing a pid
    // we obtained from `Child::id()` and a documented signal constant
    // is sound. We do not interpret the return — best-effort.
    // PIDs always fit in pid_t; the try_from handles the theoretical edge.
    unsafe {
        if let Ok(raw) = libc::pid_t::try_from(pid) {
            libc::kill(raw, libc::SIGTERM);
        }
    }
}

#[cfg(not(target_os = "linux"))]
const fn send_sigterm(_pid: u32) {
    // Non-Linux builds compile but do not run real-process tests.
}

/// Send SIGKILL to the entire process group led by `pid`. Used as a
/// fallback to reach reparented grandchildren whose lineage left the
/// driver's tokio `Child` handle. The child is placed in its own
/// session via `setsid` at spawn time (see [`ProcessDriver::build_command`])
/// so its PGID equals its PID; passing `-pid` to `kill(2)` delivers
/// SIGKILL to every member of that process group.
#[cfg(target_os = "linux")]
fn send_sigkill_pgrp(pid: u32) {
    // SAFETY: `libc::kill` with a negative pid targets a process group
    // and is sound for any signed pid_t. We ignore the return — best-effort.
    // PIDs always fit in pid_t; the try_from handles the theoretical edge.
    unsafe {
        if let Ok(raw) = libc::pid_t::try_from(pid) {
            libc::kill(-raw, libc::SIGKILL);
        }
    }
}

#[cfg(not(target_os = "linux"))]
const fn send_sigkill_pgrp(_pid: u32) {
    // Non-Linux builds compile but do not run real-process tests.
}
