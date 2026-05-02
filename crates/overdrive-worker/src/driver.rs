//! `ExecDriver` — the Phase 1 production driver impl per ADR-0026
//! and ADR-0029.
//!
//! Linux-only by design. Spawns child processes via
//! `tokio::process::Command`, places them into a workload cgroup
//! scope, writes resource limits, and supervises lifecycle.
//!
//! The `exec` vocabulary aligns with Nomad's `exec` task driver and
//! Talos's terminology — see ADR-0029 amendment 2026-04-28.
//!
//! Per ADR-0026 D6: direct cgroupfs writes; no `cgroups-rs` dep.
//! Per ADR-0026 D9: `cpu.weight` + `memory.max` derived from
//! `AllocationSpec::resources` at start time.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::warn;

use overdrive_core::id::AllocationId;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, ExitEvent,
    ExitKind, Resources,
};

use crate::cgroup_manager::{
    self, CgroupPath, cgroup_kill, create_workload_scope, place_pid_in_scope,
    remove_workload_scope, write_resource_limits,
};

/// Default grace window between SIGTERM and SIGKILL during stop.
const DEFAULT_STOP_GRACE: Duration = Duration::from_secs(5);

/// Capacity of the per-driver `ExitEvent` channel. Sized for burst
/// load — every running alloc emits exactly one event in its
/// lifetime, so a small constant is plenty.
const EXIT_CHANNEL_CAPACITY: usize = 256;

/// Construct a `DriverError::StartRejected` for the exec driver. The
/// `driver: DriverType::Exec` discriminator is fixed by construction,
/// so the call sites only need to supply the human-readable reason. Used
/// by every fallible step in `Driver::start`.
fn start_rejected(reason: impl Into<String>) -> DriverError {
    DriverError::StartRejected { driver: DriverType::Exec, reason: reason.into() }
}

/// Classify a child's `wait()` resolution into the typed `ExitKind`
/// the worker subsystem consumes. The `intentional_stop` flag is the
/// load-bearing discriminator: when `true`, every exit shape collapses
/// to `CleanExit` so the worker subsystem writes `AllocState::Terminated`
/// regardless of the OS-level exit cause (per RCA §Approved fix item 4).
///
/// Mapping when `intentional_stop == false`:
/// - `ExitStatus::code() == Some(0)` → `CleanExit`
/// - `ExitStatus::code() == Some(c)` (c != 0) → `Crashed { exit_code: Some(c), signal: None }`
/// - `ExitStatus::signal() == Some(s)` (Linux) → `Crashed { exit_code: None, signal: Some(s) }`
/// - Otherwise (no code, no signal) → `Crashed { exit_code: None, signal: None }`
///
/// This is the highest-mutation-density surface in the diff per
/// `.claude/rules/testing.md` § "What it's NOT for" — keep it small
/// and exhaustively covered by the inline tests below.
#[cfg(target_os = "linux")]
fn classify_exit(status: std::process::ExitStatus, intentional_stop: bool) -> ExitKind {
    use std::os::unix::process::ExitStatusExt;

    if intentional_stop {
        // Operator-driven termination: any exit shape (clean code,
        // SIGTERM, SIGKILL) classifies as a clean Terminated upstream.
        return ExitKind::CleanExit;
    }

    status.code().map_or_else(
        || {
            if let Some(sig) = status.signal() {
                ExitKind::Crashed { exit_code: None, signal: Some(sig) }
            } else {
                ExitKind::Crashed { exit_code: None, signal: None }
            }
        },
        |code| {
            if code == 0 {
                ExitKind::CleanExit
            } else {
                ExitKind::Crashed { exit_code: Some(code), signal: None }
            }
        },
    )
}

#[cfg(not(target_os = "linux"))]
fn classify_exit(status: std::process::ExitStatus, intentional_stop: bool) -> ExitKind {
    if intentional_stop {
        return ExitKind::CleanExit;
    }
    match status.code() {
        Some(0) => ExitKind::CleanExit,
        Some(code) => ExitKind::Crashed { exit_code: Some(code), signal: None },
        None => ExitKind::Crashed { exit_code: None, signal: None },
    }
}

/// Tracking state for a running allocation owned by the driver. The
/// watcher task — spawned by `Driver::start` — owns the `Child`;
/// this struct records only the side-channel state the driver itself
/// needs to inspect (the `intentional_stop` flag, the cgroup scope,
/// and the watcher's `JoinHandle` for cleanup).
///
/// Slot lifecycle: inserted by `Driver::start`, removed by
/// `Driver::stop`. The driver does NOT retain a terminal-state slot
/// after stop — durable terminal-state truth lives in the
/// `ObservationStore` (`AllocStatusRow`) per the §18 three-layer
/// state taxonomy. See `Driver::status` rustdoc in `overdrive-core`
/// for the post-stop contract.
struct LiveAllocation {
    scope: CgroupPath,
    /// Set to `true` by `Driver::stop` BEFORE delivering SIGTERM.
    /// The watcher reads this when classifying the exit so a
    /// SIGTERM/SIGKILL induced by operator stop is `Terminated`,
    /// not `Failed`. Per RCA §Approved fix item 3.
    intentional_stop: Arc<AtomicBool>,
    /// Handle to the per-alloc watcher task that calls
    /// `child.wait().await`. Awaited in `Driver::stop` after the
    /// signal is delivered so the driver does not leak a zombie
    /// task, but the path is best-effort — the `JoinHandle` may
    /// already have completed naturally before stop runs.
    watcher: tokio::task::JoinHandle<()>,
}

/// Production `Driver` impl for native processes under cgroup v2
/// supervision. Linux-only; non-Linux builds compile but every
/// `Driver::start` returns `DriverError::StartRejected`.
#[derive(Clone)]
pub struct ExecDriver {
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
    /// Sender half of the `ExitEvent` channel. The per-alloc watcher
    /// tasks spawned by `Driver::start` clone this sender and emit
    /// one event when `child.wait()` resolves. The matching receiver
    /// is handed out exactly once via the `ExitWatcher` trait.
    exit_tx: mpsc::Sender<ExitEvent>,
    /// Receiver half of the `ExitEvent` channel. Stored in a `Mutex`
    /// so `take_receiver()` can move it out behind a shared reference.
    /// `None` once consumed.
    exit_rx: Arc<Mutex<Option<mpsc::Receiver<ExitEvent>>>>,
    /// Injected clock — production wires `SystemClock` (host),
    /// simulation wires `SimClock`. The driver's grace window in
    /// `Driver::stop` goes through `Clock::sleep` so the timeout is
    /// DST-controllable; bare `tokio::time::*` is banned in
    /// production code per `.claude/rules/testing.md` § Sources of
    /// Nondeterminism.
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for ExecDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecDriver")
            .field("cgroup_root", &self.cgroup_root)
            .field("stop_grace", &self.stop_grace)
            .field("force_limit_write_failure", &self.force_limit_write_failure)
            .finish_non_exhaustive()
    }
}

impl ExecDriver {
    /// Construct a fresh `ExecDriver` rooted at `cgroup_root` with an
    /// explicit `Clock` dependency. Production wires `/sys/fs/cgroup`
    /// and `Arc::new(overdrive_host::SystemClock)`; simulation / DST
    /// tests wire a tempdir and `Arc::new(SimClock::new())` so the
    /// SIGTERM/SIGKILL grace window in `Driver::stop` advances on
    /// logical time. The clock is a required parameter, never
    /// defaulted, so tests cannot accidentally inherit wall-clock
    /// behaviour by omission.
    #[must_use]
    pub fn new(cgroup_root: PathBuf, clock: Arc<dyn Clock>) -> Self {
        let (exit_tx, exit_rx) = mpsc::channel(EXIT_CHANNEL_CAPACITY);
        Self {
            cgroup_root,
            stop_grace: DEFAULT_STOP_GRACE,
            force_limit_write_failure: false,
            live: Arc::new(Mutex::new(BTreeMap::new())),
            exit_tx,
            exit_rx: Arc::new(Mutex::new(Some(exit_rx))),
            clock,
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

    /// Test-only inspection hook — number of entries currently in the
    /// internal `live` map.
    ///
    /// The `Driver` trait does not (and should not) expose live-map
    /// cardinality. This accessor is the regression hook for
    /// `fix-terminated-slot-accumulation` Step 01-01: a long-running
    /// node session must not accumulate one `BTreeMap` entry per
    /// finally-terminated allocation. The GREEN fix (Step 01-02)
    /// drops `LiveAllocation::Terminated` and evicts the slot in
    /// `stop()`; this accessor lets the regression test assert the
    /// post-stop cardinality is zero.
    ///
    /// Gated behind the `integration-tests` feature so production
    /// callers (and the public Driver trait surface) cannot reach
    /// it. The slow-lane `tests/integration/exec_driver/
    /// live_map_bounded.rs` regression test is the sole consumer.
    #[cfg(feature = "integration-tests")]
    pub fn live_count(&self) -> usize {
        self.live.lock().len()
    }

    /// Build the [`Command`] that this driver will exec for `spec`.
    ///
    /// `ExecDriver` invokes the binary at `spec.command` verbatim against
    /// `spec.args`; magic image-name dispatch (the pre-2026-04-28
    /// hardcoded `/bin/sleep` / `/bin/sh` / CPU-burner arg-injection
    /// tree that previously read test-fixture intent from production
    /// code) was removed per ADR-0029 amendment 2026-04-28 + ADR-0026
    /// amendment 2026-04-28 + ADR-0030. Test fixtures construct argv
    /// inline.
    ///
    /// The `setsid(2)` pre-exec hook is unconditional: every spawned
    /// child becomes its own process group leader so the driver can
    /// reach reparented grandchildren via `kill(-pgid, SIGKILL)` at
    /// stop time. `cgroup.kill` covers the production path (real
    /// cgroupfs); the process-group fallback covers the integration
    /// tests, which mount a `tempfile::TempDir` as a fake cgroupfs root
    /// where `cgroup.kill` is a no-op file write. Linux-only —
    /// `pre_exec` is `unsafe` because the closure runs between fork
    /// and exec where the contract is to call only async-signal-safe
    /// functions; `setsid(2)` is on the POSIX async-signal-safe list.
    fn build_command(spec: &AllocationSpec) -> Command {
        let mut cmd = Command::new(&spec.command);
        cmd.args(&spec.args);
        cmd.kill_on_drop(false);

        #[cfg(target_os = "linux")]
        {
            // SAFETY: `setsid` is async-signal-safe; the closure is
            // executed in the forked child between fork and exec, no
            // shared state is touched. `setsid()` places the spawned
            // child in its own process group so SIGKILL at stop time
            // reaches the entire workload tree (matches the pre-rename
            // behaviour for `/bin/sh`-class workloads, made unconditional
            // because every exec workload deserves the same guarantee).
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
impl Driver for ExecDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        let scope = CgroupPath::for_alloc(&spec.alloc);

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
                return Err(start_rejected(format!("spawn {}: {err}", spec.command)));
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

        // 5. Record the allocation as live and spawn the per-alloc
        //    exit watcher. The watcher takes ownership of the `Child`
        //    and emits an `ExitEvent` on the driver's mpsc channel
        //    when `child.wait()` resolves; the `exit_observer`
        //    subsystem (see `crates/overdrive-worker/src/
        //    exit_observer.rs`) consumes that event and writes the
        //    classified `AllocStatusRow` to the ObservationStore.
        let intentional_stop = Arc::new(AtomicBool::new(false));
        let watcher = spawn_exit_watcher(
            spec.alloc.clone(),
            child,
            intentional_stop.clone(),
            self.exit_tx.clone(),
        );
        self.live
            .lock()
            .insert(spec.alloc.clone(), LiveAllocation { scope, intentional_stop, watcher });

        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(pid) })
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        // Take ownership of the live state so we can await on the
        // watcher without holding the lock.
        let entry = {
            let mut live = self.live.lock();
            live.remove(&handle.alloc)
        };
        let Some(LiveAllocation { scope, intentional_stop, watcher }) = entry else {
            return Err(DriverError::NotFound { alloc: handle.alloc.clone() });
        };

        // 0. Set `intentional_stop = true` BEFORE delivering any
        //    signal. The watcher reads this flag at exit-classification
        //    time (the `ExitEvent::intentional_stop` field), so a
        //    SIGTERM/SIGKILL induced by this stop call must NOT race
        //    the flag-set to a `false` read. `SeqCst` is the strongest
        //    available ordering and pairs with the watcher's `SeqCst`
        //    load. Per RCA §Approved fix item 3.
        intentional_stop.store(true, Ordering::SeqCst);

        // The watcher owns the `Child`. We address the workload by
        // PID for the SIGTERM/SIGKILL signals; the PID lives in the
        // `AllocationHandle` (populated at start time). Tests and
        // action-shim call sites that build the handle by hand carry
        // `pid: None`; those code paths cannot deliver a process-
        // targeted signal but still tear down the workload via the
        // unconditional `cgroup.kill` write below.
        let pid_for_pgrp_kill = handle.pid;

        // 1. Send SIGTERM via libc::kill.
        if let Some(pid) = pid_for_pgrp_kill {
            send_sigterm(pid);
        }

        // 2. Wait up to the grace window for the watcher task (which
        //    owns the `Child`) to complete naturally — the watcher's
        //    `child.wait()` resolves once the SIGTERM-driven exit
        //    happens. The grace future goes through `Clock::sleep` so
        //    simulation advances logical time deterministically;
        //    production wires `SystemClock` whose `sleep` resolves on
        //    the tokio timer. Joining the watcher is best-effort: a
        //    panicked watcher surfaces as a `JoinError`, and we treat
        //    it the same as a clean completion (no SIGKILL escalation
        //    is required because there is no live task to escalate
        //    against — the `Child` is dropped with the watcher's
        //    frame).
        let watcher = tokio::select! {
            _join_result = watcher => {
                // Watcher resolved within grace — child exited or
                // task panicked. Either way, no escalation path.
                None
            }
            () = self.clock.sleep(self.stop_grace) => {
                // 3. Grace window elapsed — escalate via process-group
                //    SIGKILL below; the watcher is still running and
                //    will resolve once the child is reaped by the
                //    kernel. Dropping the `JoinHandle` here only
                //    detaches it; the task continues to run.
                Some(())
            }
        };

        // 4. Mass-kill any reparented grandchildren. /bin/sh-class
        //    workloads fork helpers (e.g. `/bin/sleep`) that reparent
        //    to init when the shell dies; the watcher's `Child` only
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
        let _ = cgroup_kill(&self.cgroup_root, &scope).await;
        // 5. Tear down the cgroup scope. NotFound is benign.
        let _ = remove_workload_scope(&self.cgroup_root, &scope).await;

        // If the grace window elapsed, the watcher is still running;
        // it will resolve once SIGKILL finishes reaping the child.
        // We do not block waiting for it — the obs row gets written
        // when the watcher's emitted ExitEvent reaches the observer.
        // The detached watcher cleans up its own `Child` on drop.
        let _ = watcher;

        // The slot was removed at the top of stop(); we deliberately
        // do NOT re-insert a terminal marker. Subsequent status()
        // calls return `Err(NotFound)`; durable terminal-state truth
        // lives in the `ObservationStore` (`AllocStatusRow`). See the
        // `Driver::status` rustdoc in `overdrive-core` for the
        // post-stop contract.
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        let live = self.live.lock();
        match live.get(&handle.alloc) {
            Some(_) => Ok(AllocationState::Running),
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
                Some(running) => running.scope.clone(),
                None => return Err(DriverError::NotFound { alloc: handle.alloc.clone() }),
            }
        };
        cgroup_manager::write_resource_limits_warn_on_error(&self.cgroup_root, &scope, &resources)
            .await;
        Ok(())
    }

    fn take_exit_receiver(&self) -> Option<mpsc::Receiver<ExitEvent>> {
        self.exit_rx.lock().take()
    }
}

/// Spawn a per-allocation watcher task that owns the `Child`, awaits
/// `child.wait()`, classifies the exit, and emits an `ExitEvent` to
/// the driver's mpsc channel.
///
/// Returns the `JoinHandle` so `Driver::stop` can opt to await it
/// during the grace window. The task is `'static` over its captured
/// state — the `Child`, the `intentional_stop` flag, the `Sender`,
/// and the `AllocationId`.
fn spawn_exit_watcher(
    alloc: AllocationId,
    mut child: Child,
    intentional_stop: Arc<AtomicBool>,
    exit_tx: mpsc::Sender<ExitEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let status_result = child.wait().await;
        // `Ordering::SeqCst` pairs with the `store` in `Driver::stop`.
        let intentional = intentional_stop.load(Ordering::SeqCst);
        let kind = match status_result {
            Ok(status) => classify_exit(status, intentional),
            Err(_io_err) => {
                // `Child::wait` failure is exotic — the kernel did not
                // give us a status. Treat as a crash with no payload
                // unless `intentional_stop` was set. The watcher is
                // best-effort: even if the channel send fails (the
                // observer subsystem already shut down), there is
                // nowhere else to record the event.
                if intentional {
                    ExitKind::CleanExit
                } else {
                    ExitKind::Crashed { exit_code: None, signal: None }
                }
            }
        };
        let event = ExitEvent { alloc, kind, intentional_stop: intentional };
        // Send is best-effort: if the observer has shut down, the
        // event is dropped — the obs store already reflects a
        // shutdown-time terminal state, and there is no recovery
        // here.
        let _ = exit_tx.send(event).await;
    })
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
/// session via `setsid` at spawn time (see [`ExecDriver::build_command`])
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

// ---------------------------------------------------------------------------
// Exit-classification unit tests (mutation-gate target)
// ---------------------------------------------------------------------------
//
// `classify_exit` is the highest-mutation-density surface in the Step
// 01-02 diff per `.claude/rules/testing.md` §Mandatory targets. The
// table below pins the (ExitStatus, intentional_stop) → ExitKind
// mapping exhaustively. Linux-only — `ExitStatus` does not expose
// `from_raw` cross-platform, and signal handling is Linux-specific.
#[cfg(all(test, target_os = "linux"))]
mod classify_exit_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    /// Construct an `ExitStatus` from a normal exit code.
    fn from_code(code: i32) -> std::process::ExitStatus {
        // `from_raw(code << 8)` mimics how the kernel encodes a
        // normal exit: low 8 bits 0, next 8 bits = exit code.
        std::process::ExitStatus::from_raw(code << 8)
    }

    /// Construct an `ExitStatus` from a terminating signal.
    fn from_signal(signal: i32) -> std::process::ExitStatus {
        // Low 7 bits encode the signal; bit 7 (0x80) signals coredump,
        // which we omit. `from_raw(signal)` produces an `ExitStatus`
        // whose `signal()` returns `Some(signal)`.
        std::process::ExitStatus::from_raw(signal)
    }

    #[test]
    fn clean_exit_zero_intentional_false_classifies_as_clean_exit() {
        let kind = classify_exit(from_code(0), false);
        assert_eq!(kind, ExitKind::CleanExit);
    }

    #[test]
    fn clean_exit_zero_intentional_true_classifies_as_clean_exit() {
        // intentional_stop wins — operator stop with code-0 exit.
        let kind = classify_exit(from_code(0), true);
        assert_eq!(kind, ExitKind::CleanExit);
    }

    #[test]
    fn nonzero_exit_intentional_false_classifies_as_crashed_with_code() {
        let kind = classify_exit(from_code(1), false);
        assert_eq!(kind, ExitKind::Crashed { exit_code: Some(1), signal: None });
    }

    #[test]
    fn nonzero_exit_intentional_true_classifies_as_clean_exit() {
        // operator stop wins — even if the workload exited non-zero,
        // the intentional flag declares it terminated.
        let kind = classify_exit(from_code(137), true);
        assert_eq!(kind, ExitKind::CleanExit);
    }

    #[test]
    fn signal_killed_intentional_false_classifies_as_crashed_with_signal() {
        // SIGKILL = 9 — external kill on a running workload, no
        // operator stop in flight. Crashed.
        let kind = classify_exit(from_signal(9), false);
        assert_eq!(kind, ExitKind::Crashed { exit_code: None, signal: Some(9) });
    }

    #[test]
    fn signal_killed_intentional_true_classifies_as_clean_exit() {
        // SIGTERM = 15 — operator stop delivered SIGTERM before
        // setting intentional_stop=true and waiting for the watcher;
        // the watcher reads intentional_stop=true and classifies as
        // Terminated upstream.
        let kind = classify_exit(from_signal(15), true);
        assert_eq!(kind, ExitKind::CleanExit);
    }
}
