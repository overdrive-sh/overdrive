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
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStderr, Command};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};

use overdrive_core::id::AllocationId;
use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, ExitEvent,
    ExitKind, Resources, STDERR_TAIL_LINES,
};

use crate::cgroup_manager::{CgroupManager, CgroupPath};

/// Default grace window between SIGTERM and SIGKILL during stop.
const DEFAULT_STOP_GRACE: Duration = Duration::from_secs(5);

/// Maximum number of cooperative yields the per-alloc watcher performs
/// after `child.wait()` resolves to let the stderr-tail reader catch
/// up on kernel-buffered bytes. NOT a wall-clock or logical-clock
/// timer — each yield gives the reader a runtime turn but completes
/// in zero (or near-zero) elapsed time when the reader has nothing
/// to do, so the bound is safe under both `SystemClock` and `SimClock`
/// wirings. The common case (child closes stderr cleanly on exit)
/// resolves well within the first few yields; the daemonised-
/// grandchildren case (FD held open by reparented descendants) hits
/// the budget and the watcher snapshots the partial tail.
/// Per step 02-05 / ADR-0033 Amendment 2026-05-10.
const STDERR_DRAIN_MAX_YIELDS: usize = 64;

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
fn classify_exit(status: std::process::ExitStatus, intentional_stop: bool) -> ExitKind {
    use std::os::unix::process::ExitStatusExt;

    if intentional_stop {
        // Operator-driven termination: any exit shape (clean code,
        // SIGTERM, SIGKILL) classifies as a clean Terminated upstream.
        return ExitKind::CleanExit;
    }

    status.code().map_or_else(
        || {
            status.signal().map_or(ExitKind::Crashed { exit_code: None, signal: None }, |sig| {
                ExitKind::Crashed { exit_code: None, signal: Some(sig) }
            })
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
    /// PID of the spawned child, copied from `Child::id()` at start time.
    /// Stored here so `Driver::stop` can deliver SIGTERM using the driver's
    /// own tracking state rather than relying on `AllocationHandle::pid`,
    /// which callers (e.g. the action shim) may construct as `pid: None`.
    pid: u32,
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
    /// "Running-confirmed" gate sender per
    /// `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
    /// (Solution 1'). The action shim consumes this via
    /// [`Driver::release_for_exit_emission`] after committing the
    /// `obs.write(AllocStatus::Running)` row (or after the May-2
    /// degraded-escalation `LifecycleEvent` path). The matching
    /// `oneshot::Receiver` was handed to the per-alloc watcher at
    /// spawn time and is awaited BEFORE the watcher emits its
    /// `ExitEvent` — that is the structural happens-before edge
    /// preventing the observer's `find_prior_row → NoPriorRow`
    /// silent-drop on sub-millisecond-lifetime workloads.
    ///
    /// `Some` from start until [`Driver::release_for_exit_emission`]
    /// `take()`s it; `None` thereafter (idempotent fire). On
    /// `Driver::stop` the `LiveAllocation` is dropped — if the
    /// sender was never taken (action shim crashed before
    /// release), the sender's `Drop` causes the watcher's
    /// `oneshot::Receiver::await` to resolve to `Err(RecvError)`,
    /// which the watcher treats as "proceed and emit". See the
    /// `Driver::start` rustdoc § "Sender drop (orphan path)".
    gate_sender: Option<oneshot::Sender<()>>,
}

/// Production `Driver` impl for native processes under cgroup v2
/// supervision. Linux-only; non-Linux builds compile but every
/// `Driver::start` returns `DriverError::StartRejected`.
#[derive(Clone)]
pub struct ExecDriver {
    /// Port-routed cgroupfs surface, constructed once at
    /// [`ExecDriver::new`] from the injected `fs: Arc<dyn CgroupFs>`
    /// and the shared `cgroup_root`. Every filesystem mutation in
    /// `Driver::{start,stop,resize}` flows through this manager;
    /// no direct `tokio::fs::*` calls from `driver.rs`. Production
    /// wires `overdrive_host::RealCgroupFs`; tests wire
    /// `overdrive_sim::SimCgroupFs`. Per ADR-0054 § D5 step 5.
    cgroup_manager: CgroupManager,
    stop_grace: Duration,
    /// Test-only injection: when `true`, force `write_resource_limits`
    /// to fail synthetically. Always `false` in production wiring.
    /// Validates ADR-0026 D9 warn-and-continue under controlled
    /// failure.
    force_limit_write_failure: bool,
    /// Opt-in target network namespace for every spawned child.
    /// `None` (the production default) yields bit-identical behaviour
    /// to the pre-2026-05-21 driver. `Some(path)` causes
    /// `Driver::start` to open `path` (typically
    /// `/var/run/netns/<name>`) as an `OwnedFd` and install a
    /// `pre_exec` hook that calls `setns(fd, CLONE_NEWNET)` in the
    /// forked child between fork and exec — the spawned binary is
    /// born already inside the target netns.
    ///
    /// Structurally aligned with the CNI spec model: the netns is
    /// created and managed by the caller (a test fixture today, a
    /// CNI-runtime-equivalent in future container/microvm driver
    /// types); the driver only ENTERS an already-existing namespace,
    /// never creates one. See
    /// `docs/research/testing/walking-skeleton-xdp-lb-topology.md`
    /// § Findings 2.4 + 2.5 for the Rust ecosystem precedent
    /// (`netns-rs`, `netns-exec`) and the CNI cross-reference.
    netns_path: Option<PathBuf>,
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
    /// Optional `ProbeRunner` reference. Production composition
    /// root threads this in via [`Self::with_probe_runner`] so the
    /// driver's [`Driver::on_alloc_running`] / [`Driver::on_alloc_terminal`]
    /// hooks can dispatch to `probe_runner.start_alloc` /
    /// `probe_runner.stop_alloc` per ADR-0054 § 2. `None` (the
    /// default) yields a no-op hook — used by acceptance tests that
    /// do not exercise the probe path.
    probe_runner: Option<Arc<crate::probe_runner::ProbeRunner>>,
}

impl std::fmt::Debug for ExecDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecDriver")
            .field("cgroup_root", &self.cgroup_manager.cgroup_root())
            .field("stop_grace", &self.stop_grace)
            .field("force_limit_write_failure", &self.force_limit_write_failure)
            .finish_non_exhaustive()
    }
}

impl ExecDriver {
    /// Construct a fresh `ExecDriver` rooted at `cgroup_root` with
    /// explicit `Clock` and `CgroupFs` dependencies. Production and
    /// integration tests alike wire `/sys/fs/cgroup` and either:
    ///   * production — `Arc::new(overdrive_host::SystemClock)` +
    ///     `Arc::new(overdrive_host::RealCgroupFs::new())`
    ///   * Tier 3 real-IO tests — same as production (the Tier 3 suite
    ///     exercises real cgroupfs; only the clock varies between
    ///     `TokioWallClock` and `SimClock` depending on whether
    ///     wall-clock progression is needed)
    ///   * DST / sim — `Arc::new(SimClock::new())` +
    ///     `Arc::new(overdrive_sim::SimCgroupFs::new())`
    ///
    /// Both `clock` and `fs` are REQUIRED, positional parameters per
    /// `.claude/rules/development.md` § "Port-trait dependencies":
    /// neither has a default, and there is no builder shape that makes
    /// them optional. A test that forgets to inject them fails to
    /// compile (see `tests/compile_fail/exec_driver_missing_fs.rs`
    /// for the structural defense). Internally the driver constructs
    /// a single [`CgroupManager`] from `cgroup_root.clone()` + `fs`
    /// and routes every filesystem mutation through it; the legacy
    /// transitional free-fn shims (`cgroup_manager_legacy_*`) from
    /// step 01-04 are gone as of step 01-05.
    #[must_use]
    pub fn new(cgroup_root: PathBuf, clock: Arc<dyn Clock>, fs: Arc<dyn CgroupFs>) -> Self {
        let (exit_tx, exit_rx) = mpsc::channel(EXIT_CHANNEL_CAPACITY);
        let cgroup_manager = CgroupManager::new(cgroup_root, fs);
        Self {
            cgroup_manager,
            stop_grace: DEFAULT_STOP_GRACE,
            force_limit_write_failure: false,
            netns_path: None,
            live: Arc::new(Mutex::new(BTreeMap::new())),
            exit_tx,
            exit_rx: Arc::new(Mutex::new(Some(exit_rx))),
            clock,
            probe_runner: None,
        }
    }

    /// Thread a `ProbeRunner` reference into the driver so the
    /// `Driver::on_alloc_running` / `Driver::on_alloc_terminal`
    /// lifecycle hooks dispatch to it. Production composition root
    /// wires this in via the builder; tests that don't exercise
    /// probes leave the field as `None` (default no-op hooks).
    ///
    /// Per ADR-0054 § 2 + § 3: the hooks are the structural seam
    /// between the action-shim's `obs.write(Running)` /
    /// `obs.write(Terminated)` writes and the `ProbeRunner` per-alloc
    /// supervisor lifecycle.
    #[must_use]
    pub fn with_probe_runner(
        mut self,
        probe_runner: Arc<crate::probe_runner::ProbeRunner>,
    ) -> Self {
        self.probe_runner = Some(probe_runner);
        self
    }

    /// Target every spawned child at `netns_path` (typically
    /// `/var/run/netns/<name>`). On `Driver::start`, the driver
    /// opens the path as an `OwnedFd` and installs a `pre_exec`
    /// hook calling `setns(fd, CLONE_NEWNET)` in the forked child
    /// before `execve` — the workload binary is born already inside
    /// the target netns.
    ///
    /// Default = `None` (production behaviour, bit-identical to the
    /// pre-2026-05-21 driver). The builder is opt-in for test
    /// fixtures that pre-create per-test netns (and, in future,
    /// for container / microvm driver types whose runtime owns the
    /// netns lifecycle — the same shape CNI plugins consume via
    /// `CNI_NETNS`).
    ///
    /// On netns-open / `setns` failure at start time, the start call
    /// returns `DriverError::NetnsEntry { netns_path, source }` so
    /// the caller can distinguish a netns-targeting setup error from
    /// a workload-spec rejection (`StartRejected`).
    #[must_use]
    pub fn with_netns_path(mut self, netns_path: PathBuf) -> Self {
        self.netns_path = Some(netns_path);
        self
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
    /// stop time. `cgroup.kill` is the primary mechanism on real
    /// cgroupfs (production and integration tests both run against
    /// `/sys/fs/cgroup` per `.claude/rules/testing.md` § "Running
    /// tests — Lima VM"); the process-group SIGKILL is a belt-and-
    /// braces backstop for grandchildren that have already escaped
    /// the cgroup at the moment of stop. Linux-only — `pre_exec` is
    /// `unsafe` because the closure runs between fork and exec where
    /// the contract is to call only async-signal-safe functions;
    /// `setsid(2)` is on the POSIX async-signal-safe list.
    ///
    /// When `netns_fd` is `Some`, a second `pre_exec` hook calls
    /// `setns(fd, CLONE_NEWNET)` so the spawned child enters the
    /// target network namespace before `execve`. `setns(2)` is on
    /// the kernel's async-signal-safe list for the namespace-FD case
    /// (it issues no allocation or signal-unsafe library call —
    /// just a single syscall against an already-open FD). The FD is
    /// owned by the parent and the closure receives a borrowed
    /// `RawFd` via move; on success the kernel duplicates the
    /// reference into the child's `nsproxy`, so closing the parent's
    /// FD after `spawn()` is safe.
    fn build_command(spec: &AllocationSpec, netns_fd: Option<std::os::fd::OwnedFd>) -> Command {
        let mut cmd = Command::new(&spec.command);
        cmd.args(&spec.args);
        cmd.kill_on_drop(false);
        // Pipe stderr per ADR-0033 Amendment 2026-05-10 / step 02-05:
        // the per-alloc watcher consumes lines into a bounded ring
        // buffer of capacity `STDERR_TAIL_LINES` and emits the tail
        // on the `ExitEvent`. stdout is left inherited — the workload's
        // human-readable output should still reach the operator's
        // terminal in development; a future operator-config knob can
        // pipe it too if needed.
        cmd.stderr(Stdio::piped());

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

        // Opt-in netns entry — see `with_netns_path()` rustdoc for
        // the rationale. The `setns(2)` syscall against a
        // namespace-FD is async-signal-safe; the closure runs in the
        // forked child between fork and exec and touches no
        // allocator / locked state. The FD is moved into the closure
        // (its `Drop` after `setns` returns drops the parent-side
        // reference; the kernel retains its own reference via the
        // child's `nsproxy`, so the netns stays live for the
        // workload's lifetime). Failure surfaces as `io::Error`
        // returned by the closure, which `Child::spawn()` converts
        // to a fatal start error.
        if let Some(fd) = netns_fd {
            unsafe {
                cmd.pre_exec(move || {
                    nix::sched::setns(&fd, nix::sched::CloneFlags::CLONE_NEWNET)
                        .map_err(|errno| std::io::Error::from_raw_os_error(errno as i32))?;
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
        if let Err(err) = self.cgroup_manager.create_workload_scope(&scope).await {
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
            self.cgroup_manager.write_resource_limits(&scope, &spec.resources).await
        };
        if let Err(err) = limit_result {
            warn!(
                alloc = %spec.alloc,
                scope = %scope,
                error = %err,
                "cgroup resource-limit write failed; continuing per ADR-0026 D9"
            );
        }

        // 3. If `netns_path` is set, open the netns FD up-front so
        //    failure surfaces as a structured `NetnsEntry` error
        //    (rather than as a generic `spawn()` failure buried in
        //    the `pre_exec` closure). Pre-open also fails fast on
        //    permission / missing-netns errors before the child fork
        //    happens, which is the cleaner failure mode. On success
        //    the FD is moved into the `pre_exec` closure for
        //    `setns()`.
        let netns_fd = match self.netns_path.as_ref() {
            None => None,
            Some(path) => match tokio::fs::File::open(path).await {
                Ok(f) => Some(std::os::fd::OwnedFd::from(f.into_std().await)),
                Err(source) => {
                    let _ = self.cgroup_manager.remove_workload_scope(&scope).await;
                    return Err(DriverError::NetnsEntry {
                        driver: DriverType::Exec,
                        netns_path: path.display().to_string(),
                        source,
                    });
                }
            },
        };

        // 4. Spawn the child. Failure here means the binary path is
        //    bogus or the kernel refused exec — clean up the scope dir
        //    so we don't orphan it (scenario 2.5).
        let mut cmd = Self::build_command(spec, netns_fd);
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                let _ = self.cgroup_manager.remove_workload_scope(&scope).await;
                // A spawn failure when `netns_path` is set most
                // likely came from the netns-entry `pre_exec` hook
                // (setns(2) returned EPERM / EINVAL — the open()
                // pre-flight above already ruled out a missing
                // path). Surface as `NetnsEntry` so the caller can
                // distinguish from a workload-spec rejection.
                if let Some(path) = self.netns_path.as_ref() {
                    return Err(DriverError::NetnsEntry {
                        driver: DriverType::Exec,
                        netns_path: path.display().to_string(),
                        source: err,
                    });
                }
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
            let _ = self.cgroup_manager.remove_workload_scope(&scope).await;
            return Err(start_rejected("tokio Child returned no pid (already reaped?)"));
        };
        if let Err(err) = self.cgroup_manager.place_pid_in_scope(&scope, pid).await {
            // Best-effort kill + cleanup. We don't await here —
            // the tokio Child's drop handler does not reap, but the
            // OS will reap orphans. For defence-in-depth we send
            // SIGKILL via libc.
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
            let _ = self.cgroup_manager.remove_workload_scope(&scope).await;
            return Err(start_rejected(format!("place pid in scope: {err}")));
        }

        // 5. Record the allocation as live and spawn the per-alloc
        //    exit watcher. The watcher takes ownership of the `Child`
        //    and emits an `ExitEvent` on the driver's mpsc channel
        //    when `child.wait()` resolves; the `exit_observer`
        //    subsystem (see `crates/overdrive-worker/src/
        //    exit_observer.rs`) consumes that event and writes the
        //    classified `AllocStatusRow` to the ObservationStore.
        //
        // Per step 02-05 / ADR-0033 Amendment 2026-05-10 the watcher
        // also drives a per-alloc stderr line-reader (over the piped
        // `child.stderr`) that fills a bounded `VecDeque` of capacity
        // `STDERR_TAIL_LINES`; on `child.wait()` resolution the tail
        // travels with the `ExitEvent` to the obs row.
        let intentional_stop = Arc::new(AtomicBool::new(false));
        let stderr_pipe = child.stderr.take();
        // Mint the Running-confirmed gate per
        // `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
        // (Solution 1'). Sender is stashed on `LiveAllocation`; the
        // action shim takes it via `Driver::release_for_exit_emission`
        // after `obs.write(Running)` resolves Ok (or after the May-2
        // degraded-escalation path). Receiver is handed to the
        // watcher and awaited BEFORE its first `ExitEvent` send —
        // this is the structural happens-before edge that prevents
        // `find_prior_row → NoPriorRow` silent-drops on
        // sub-millisecond-lifetime workloads.
        //
        // Per step 01-03 of `fix-exit-observer-running-gate`: the
        // action shim fires the gate via
        // `Driver::release_for_exit_emission` after committing
        // `obs.write(Running)` (or via the exit_observer's degraded
        // path on May-2 retry exhaustion). The 01-02 transitional
        // immediate-drop has been removed; the gate now provides the
        // production ordering edge. On `Driver::stop` the
        // `LiveAllocation` is dropped — if the sender was never taken
        // (action shim crashed before firing), the watcher's
        // `gate_receiver.await` resolves to `Err(RecvError)` (orphan
        // path) and emit proceeds.
        let (gate_sender, gate_receiver) = oneshot::channel::<()>();
        let watcher = spawn_exit_watcher(
            spec.alloc.clone(),
            child,
            stderr_pipe,
            intentional_stop.clone(),
            self.exit_tx.clone(),
            gate_receiver,
        );
        self.live.lock().insert(
            spec.alloc.clone(),
            LiveAllocation {
                scope,
                pid,
                intentional_stop,
                watcher,
                gate_sender: Some(gate_sender),
            },
        );

        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(pid) })
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        // Take ownership of the live state so we can await on the
        // watcher without holding the lock.
        let entry = {
            let mut live = self.live.lock();
            live.remove(&handle.alloc)
        };
        let Some(LiveAllocation { scope, pid, intentional_stop, watcher, gate_sender }) = entry
        else {
            return Err(DriverError::NotFound { alloc: handle.alloc.clone() });
        };
        // Drop the gate sender explicitly. If the action shim never
        // released the gate before stop (e.g. operator stop landed
        // mid-flight, or the watcher already emitted via the orphan
        // path), the receiver wakes with `Err(RecvError)` and the
        // watcher proceeds. Per the `Driver::start` rustdoc § "Sender
        // drop (orphan path)".
        drop(gate_sender);

        // 0. Set `intentional_stop = true` BEFORE delivering any
        //    signal. The watcher reads this flag at exit-classification
        //    time (the `ExitEvent::intentional_stop` field), so a
        //    SIGTERM/SIGKILL induced by this stop call must NOT race
        //    the flag-set to a `false` read. `SeqCst` is the strongest
        //    available ordering and pairs with the watcher's `SeqCst`
        //    load. Per RCA §Approved fix item 3.
        intentional_stop.store(true, Ordering::SeqCst);

        // The watcher owns the `Child`. We address the workload by
        // PID for the SIGTERM/SIGKILL signals; the PID comes from
        // `LiveAllocation::pid` (stored at start time) so that callers
        // who construct a `pid: None` handle — e.g. the action shim's
        // StopAllocation path — still receive a graceful SIGTERM.
        // `handle.pid` is intentionally ignored here.
        let pid_for_pgrp_kill = pid;

        // 1. Send SIGTERM via libc::kill.
        send_sigterm(pid_for_pgrp_kill);

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
        //       task in the workload's scope. Primary mechanism on
        //       both production and integration tests (both run
        //       against `/sys/fs/cgroup`).
        //    b) Process-group SIGKILL — belt-and-braces backstop for
        //       grandchildren that have already escaped the cgroup at
        //       the moment of stop. The child was `setsid`-ed at spawn
        //       so its PGID = its PID; `kill(-pid, SIGKILL)` reaches
        //       every member of that group regardless of cgroup
        //       residency.
        send_sigkill_pgrp(pid_for_pgrp_kill);
        let _ = self.cgroup_manager.cgroup_kill(&scope).await;
        // 5. Tear down the cgroup scope. NotFound is benign.
        let _ = self.cgroup_manager.remove_workload_scope(&scope).await;

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
        self.cgroup_manager.write_resource_limits_warn_on_error(&scope, &resources).await;
        Ok(())
    }

    fn take_exit_receiver(&self) -> Option<mpsc::Receiver<ExitEvent>> {
        self.exit_rx.lock().take()
    }

    /// Fire the Running-confirmed gate for `handle.alloc`. Idempotent:
    /// a call against an alloc whose gate has already fired (or whose
    /// alloc is unknown to the driver) is a no-op, NOT a panic. See
    /// `Driver::start` rustdoc on the `overdrive-core` trait for the
    /// full contract; the structural exactly-once guarantee comes
    /// from `Option::take` + `oneshot::Sender::send` consume-self.
    fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        // Hold the lock only long enough to take the sender; never
        // hold a parking_lot mutex across an `.await` (we don't
        // await here, but the discipline is uniform).
        let sender =
            self.live.lock().get_mut(&handle.alloc).and_then(|live| live.gate_sender.take());
        if let Some(sender) = sender {
            // `oneshot::Sender::send` consumes self — double-fire is
            // structurally impossible. `Err(())` from a closed
            // receiver (watcher already dropped, e.g. mid-flight
            // stop / shutdown) is benign; nothing to log on a
            // post-stop release race.
            let _ = sender.send(());
        }
        // Unknown alloc OR gate already fired: no-op per the
        // idempotent-fire contract.
    }

    /// Per ADR-0054 § 2 / § 3: when the action shim observes a
    /// successful `Driver::start` and writes the
    /// `AllocStatusRow { state: Running, .. }` row, it fires this
    /// hook with the same `AllocationSpec` it handed to `start`.
    /// The driver projects `spec.probe_descriptors` (validated
    /// upstream per ADR-0057) into a `start_alloc` call on the
    /// configured `ProbeRunner`. Drivers wired without a
    /// `ProbeRunner` (acceptance tests, sim wiring) inherit the
    /// trait's default no-op via the `None` arm below.
    fn on_alloc_running(&self, spec: &AllocationSpec) {
        if let Some(ref runner) = self.probe_runner {
            let _token = runner.start_alloc(&spec.alloc, spec.probe_descriptors.clone());
        }
    }

    /// Symmetric companion to [`Self::on_alloc_running`]. Fired by
    /// the action shim immediately after the terminal
    /// `AllocStatusRow` write commits. Dispatches to
    /// `probe_runner.stop_alloc(alloc_id)` so every per-probe task
    /// spawned under this allocation's supervisor is cooperatively
    /// shut down — no `JoinHandle::abort()` per
    /// `.claude/rules/testing.md` § cooperative-shutdown discipline.
    fn on_alloc_terminal(&self, alloc_id: &AllocationId) {
        if let Some(ref runner) = self.probe_runner {
            runner.stop_alloc(alloc_id);
        }
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
///
/// Per step 02-05 / ADR-0033 Amendment 2026-05-10 the watcher also
/// drives a stderr-tail capture: the piped `ChildStderr` (taken from
/// the spawned `Child` before this call) is consumed line-by-line
/// into a bounded `VecDeque` of capacity [`STDERR_TAIL_LINES`] held
/// behind a parking-lot `Mutex`. On `child.wait()` resolution the
/// watcher snapshots the ring's CURRENT contents non-blockingly —
/// the reader task is detached and may keep running until reparented
/// grandchildren close the stderr FD. This ordering matters: shells
/// spawning daemonised children (`while :; do :; done` busy loops,
/// `nohup` patterns) leave the FD open after the parent exits, and a
/// watcher that *awaited* the reader would block forever waiting for
/// EOF that only arrives after `cgroup.kill` reaps the workload tree.
fn spawn_exit_watcher(
    alloc: AllocationId,
    mut child: Child,
    stderr_pipe: Option<ChildStderr>,
    intentional_stop: Arc<AtomicBool>,
    exit_tx: mpsc::Sender<ExitEvent>,
    gate_receiver: oneshot::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Spawn the stderr-tail reader in parallel with `child.wait()`.
        // The reader task drains lines into a shared ring buffer; the
        // watcher snapshots the ring after `child.wait()` resolves
        // (NOT after the reader task finishes — see fn doc above).
        let ring: Arc<Mutex<VecDeque<String>>> =
            Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_LINES)));
        let reader_handle = stderr_pipe.map(|pipe| spawn_stderr_tail_reader(pipe, ring.clone()));

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
        // Snapshot the ring after letting the reader catch up on
        // any kernel-buffered stderr the child wrote immediately
        // before exit. Strategy: poll the reader's JoinHandle a
        // bounded number of times, yielding cooperatively between
        // polls so the reader can run on the same runtime. The
        // common case (the child closes stderr cleanly on exit) sees
        // the reader resolve within the first few yields; the
        // daemonised-grandchildren case (FD held open by reparented
        // descendants — `while :; do :; done` busy loops, `nohup`
        // patterns) snapshots the partial tail and detaches the
        // reader, which lives until `cgroup.kill` reaps the workload
        // tree.
        //
        // Cooperative yields (NOT `Clock::sleep`, NOT `tokio::time::
        // sleep`) are the load-bearing choice: the reader is a peer
        // task on the same runtime — yielding gives it scheduling
        // turns to drain bytes already in the kernel pipe buffer,
        // without depending on logical or wall-clock time. That
        // keeps the watcher correct under both real-time
        // (`SystemClock`) and DST (`SimClock`) wirings without
        // bending production code around test-double quirks.
        // Per `.claude/rules/development.md` § "Production code is
        // not shaped by simulation".
        let stderr_tail = if let Some(handle) = reader_handle {
            for _ in 0..STDERR_DRAIN_MAX_YIELDS {
                if handle.is_finished() {
                    break;
                }
                tokio::task::yield_now().await;
            }
            // Detach the reader if it's still running — see fn doc
            // for why awaiting it would deadlock the daemonised-
            // grandchildren case.
            drop(handle);
            let guard = ring.lock();
            if guard.is_empty() {
                None
            } else {
                Some(guard.iter().cloned().collect::<Vec<_>>().join("\n"))
            }
        } else {
            None
        };
        let event = ExitEvent { alloc, kind, intentional_stop: intentional, stderr_tail };
        // Running-confirmed gate: await the action-shim signal that
        // the corresponding `obs.write(Running)` row has committed
        // (or that the May-2 retry path has degraded to
        // `LifecycleEvent`-only). Without this happens-before edge,
        // a sub-millisecond-lifetime workload can have its
        // `ExitEvent` race the action shim's `Running` write and
        // be silently dropped by the observer's `find_prior_row →
        // NoPriorRow` arm. Per
        // `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
        // (Solution 1').
        //
        // ORDERING is load-bearing: gate await is AFTER `child.wait()`
        // resolves AND AFTER the stderr-tail drain budget completes,
        // BEFORE `exit_tx.send`. The stderr-tail drain is its own
        // pre-condition for emit (rendering correctness); the gate
        // is the additional pre-condition (ordering correctness).
        // Both must complete before the event is delivered to the
        // observer.
        //
        // `tokio::sync::oneshot` is NOT `Clock`-dependent — works
        // under `SimClock`, turmoil, real tokio identically. The
        // gate is a logical happens-before edge, not a wall-clock
        // budget. This is the structural production race fix; not a
        // sim concession (per
        // `.claude/rules/development.md` § "Production code is not
        // shaped by simulation").
        //
        // `Err(RecvError)` resolves when the sender is dropped
        // without sending — the action-shim-crashed orphan path. We
        // log at debug and proceed: the observer's own
        // `find_prior_row` handles present-or-absent prior rows the
        // same way it does today, and reconciler convergence cleans
        // up the orphan on the next tick. Per the `Driver::start`
        // rustdoc § "Sender drop (orphan path)".
        if let Err(_recv_err) = gate_receiver.await {
            debug!(
                alloc = %event.alloc,
                "exit_watcher: gate sender dropped before fire; \
                 proceeding with ExitEvent emission (orphan path)"
            );
        }
        // Send is best-effort: if the observer has shut down, the
        // event is dropped — the obs store already reflects a
        // shutdown-time terminal state, and there is no recovery
        // here.
        let _ = exit_tx.send(event).await;
    })
}

/// Spawn a task that reads `pipe` line-by-line and pushes each line
/// into a shared bounded ring (capacity [`STDERR_TAIL_LINES`]) held
/// behind a `parking_lot::Mutex`. The task runs until pipe EOF or
/// the first I/O error.
///
/// The function uses `tokio::io::BufReader::lines()` per
/// `.claude/rules/development.md` § "No blocking `std::fs::`* inside
/// async fn" — the reader is `async`-friendly and never blocks the
/// tokio worker.
///
/// Sharing the ring (rather than returning the joined tail at task
/// exit) lets the watcher snapshot the captured tail non-blockingly
/// at `child.wait()` resolution. See `spawn_exit_watcher` doc for
/// why awaiting the reader at watcher-exit time would deadlock when
/// reparented grandchildren keep the FD alive.
fn spawn_stderr_tail_reader(
    pipe: ChildStderr,
    ring: Arc<Mutex<VecDeque<String>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(pipe).lines();
        // `while let` exits on `Ok(None)` (pipe-EOF — workload's
        // stderr is fully drained) and on `Err(_)` (I/O error mid-
        // stream — keep what we have and stop). Either way, the
        // shared ring carries whatever lines were captured up to
        // exit.
        while let Ok(Some(line)) = reader.next_line().await {
            let mut guard = ring.lock();
            if guard.len() == STDERR_TAIL_LINES {
                guard.pop_front();
            }
            guard.push_back(line);
        }
    })
}

// ---------------------------------------------------------------------------
// SIGTERM / SIGKILL signalling
// ---------------------------------------------------------------------------

/// Send SIGTERM to a process via `libc::kill`.
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

/// Send SIGKILL to the entire process group led by `pid`. Used as a
/// fallback to reach reparented grandchildren whose lineage left the
/// driver's tokio `Child` handle. The child is placed in its own
/// session via `setsid` at spawn time (see [`ExecDriver::build_command`])
/// so its PGID equals its PID; passing `-pid` to `kill(2)` delivers
/// SIGKILL to every member of that process group.
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

// ---------------------------------------------------------------------------
// Exit-classification unit tests (mutation-gate target)
// ---------------------------------------------------------------------------
//
// `classify_exit` is the highest-mutation-density surface in the Step
// 01-02 diff per `.claude/rules/testing.md` §Mandatory targets. The
// table below pins the (ExitStatus, intentional_stop) → ExitKind
// mapping exhaustively. Linux-only — `ExitStatus` does not expose
// `from_raw` cross-platform, and signal handling is Linux-specific.
#[cfg(test)]
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

/// Service-health-check-probes step 01-03d — observable-outcome
/// unit tests for the new `on_alloc_running` / `on_alloc_terminal`
/// lifecycle hooks per ADR-0054 § 2 / § 3.
///
/// Asserts: when an [`ExecDriver`] is constructed with a
/// `ProbeRunner` via [`ExecDriver::with_probe_runner`], the
/// lifecycle hooks dispatch through to
/// `probe_runner.start_alloc` / `probe_runner.stop_alloc`. The
/// observable effect is `ProbeRunner::active_alloc_count` —
/// before the hooks fire it is 0; after `on_alloc_running` it is
/// 1; after `on_alloc_terminal` it is back to 0.
///
/// Mutation kill: these tests kill the per-method mutants
/// `<impl Driver for ExecDriver>::on_alloc_running → ()` and
/// `<impl Driver for ExecDriver>::on_alloc_terminal → ()` —
/// without the dispatch the supervisor-count delta would not
/// appear.
#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod lifecycle_hook_tests {
    use std::path::PathBuf;
    use std::str::FromStr as _;
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use overdrive_core::SpiffeId;
    use overdrive_core::aggregate::probe_descriptor::ProbeDescriptor;
    use overdrive_core::id::AllocationId;
    use overdrive_core::traits::clock::Clock;
    use overdrive_core::traits::driver::{AllocationSpec, Driver as _, Resources};
    use overdrive_core::traits::prober::{
        ExecProber, HttpProber, ProbeFailure, ProbeOutcome, TcpProber,
    };

    use super::ExecDriver;
    use crate::probe_runner::ProbeRunner;

    // Minimal sim TCP prober that returns Pass for everything.
    // (We don't use overdrive_sim here to avoid the dev-dep cycle.)
    struct AlwaysPassTcpProber;

    #[async_trait]
    impl TcpProber for AlwaysPassTcpProber {
        async fn probe(
            &self,
            _host: &str,
            _port: u16,
            _timeout: Duration,
        ) -> Result<ProbeOutcome, ProbeFailure> {
            Ok(ProbeOutcome::Pass)
        }
    }

    struct UnusedHttpProber;

    #[async_trait]
    impl HttpProber for UnusedHttpProber {
        async fn probe(
            &self,
            _url: &str,
            _timeout: Duration,
        ) -> Result<ProbeOutcome, ProbeFailure> {
            Ok(ProbeOutcome::Pass)
        }
    }

    struct UnusedExecProber;

    #[async_trait]
    impl ExecProber for UnusedExecProber {
        async fn probe(
            &self,
            _command: &[String],
            _cgroup_path: &str,
            _timeout: Duration,
        ) -> Result<ProbeOutcome, ProbeFailure> {
            Ok(ProbeOutcome::Pass)
        }
    }

    // Stub clock — never called for hook dispatch tests but
    // required by ExecDriver::new. Wall-clock methods return
    // synthetic zeroes; `sleep` is unreachable on this path.
    struct ZeroClock;

    #[async_trait]
    impl Clock for ZeroClock {
        fn now(&self) -> std::time::Instant {
            std::time::Instant::now()
        }
        fn unix_now(&self) -> Duration {
            Duration::ZERO
        }
        async fn sleep(&self, _: Duration) {
            // Not exercised by the lifecycle-hook dispatch path.
        }
    }

    fn build_driver_with_runner() -> (ExecDriver, Arc<ProbeRunner>) {
        // GAP-7 closure: ProbeRunner::new now takes `Arc<dyn Clock>` +
        // `Arc<dyn ObservationStore>` as mandatory constructor
        // parameters so `start_alloc` can spawn supervised
        // per-descriptor tick tasks. These tests exercise only the
        // lifecycle-hook dispatch path with an empty
        // `probe_descriptors` Vec — no tick task is actually spawned,
        // so the injected clock + obs are never observed.
        // `overdrive_sim` is a dev-dep (also used by `node_health`
        // tests in this crate); the AlwaysPassTcpProber comment
        // above predates the dev-dep addition.
        let obs: Arc<dyn overdrive_core::traits::observation_store::ObservationStore> =
            Arc::new(overdrive_sim::adapters::observation_store::SimObservationStore::single_peer(
                overdrive_core::id::NodeId::new("driver-test-node").expect("static NodeId parses"),
                0,
            ));
        let runner = Arc::new(ProbeRunner::new(
            Arc::new(AlwaysPassTcpProber),
            Arc::new(UnusedHttpProber),
            Arc::new(UnusedExecProber),
            Arc::new(ZeroClock),
            obs,
        ));
        let driver = ExecDriver::new(PathBuf::from("/tmp/overdrive-test"), Arc::new(ZeroClock))
            .with_probe_runner(Arc::clone(&runner));
        (driver, runner)
    }

    fn sample_spec(alloc_id: &AllocationId) -> AllocationSpec {
        AllocationSpec {
            alloc: alloc_id.clone(),
            identity: SpiffeId::from_str("spiffe://overdrive.local/test/wl")
                .expect("valid SpiffeId"),
            command: "/bin/true".to_owned(),
            args: vec![],
            resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
            probe_descriptors: Vec::<ProbeDescriptor>::new(),
        }
    }

    #[test]
    fn on_alloc_running_registers_supervisor_on_probe_runner() {
        let (driver, runner) = build_driver_with_runner();
        let alloc_id = AllocationId::new("alloc-run-1").expect("valid AllocationId");
        let spec = sample_spec(&alloc_id);

        assert_eq!(runner.active_alloc_count(), 0);
        driver.on_alloc_running(&spec);
        assert_eq!(
            runner.active_alloc_count(),
            1,
            "on_alloc_running must register an alloc supervisor on the wired ProbeRunner"
        );
    }

    #[test]
    fn on_alloc_terminal_cancels_supervisor_on_probe_runner() {
        let (driver, runner) = build_driver_with_runner();
        let alloc_id = AllocationId::new("alloc-term-1").expect("valid AllocationId");
        let spec = sample_spec(&alloc_id);

        driver.on_alloc_running(&spec);
        assert_eq!(runner.active_alloc_count(), 1);

        driver.on_alloc_terminal(&alloc_id);
        assert_eq!(
            runner.active_alloc_count(),
            0,
            "on_alloc_terminal must cancel the alloc's supervisor on the wired ProbeRunner"
        );
    }

    #[test]
    fn on_alloc_running_without_probe_runner_is_a_noop() {
        // Driver constructed WITHOUT `with_probe_runner` — the field
        // is `None`; the trait default no-op path fires. Observable
        // outcome: no panic, no state change, no side effect.
        let driver = ExecDriver::new(PathBuf::from("/tmp/overdrive-test"), Arc::new(ZeroClock));
        let alloc_id = AllocationId::new("alloc-noop-1").expect("valid AllocationId");
        let spec = sample_spec(&alloc_id);

        // No panic — the None branch returns immediately.
        driver.on_alloc_running(&spec);
        driver.on_alloc_terminal(&alloc_id);
    }
}
