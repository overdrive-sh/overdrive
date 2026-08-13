//! `SimVmm` — test binding of the [`Vmm`] port trait.
//!
//! Per ADR-0082 §D1/§D6 (microvm-driver-cloud-hypervisor, GH #42): the
//! in-memory double the `vmm_equivalence` structural guard
//! (`overdrive-host` tests) drives through the same call sequence as the
//! production `CloudHypervisorVmm`, asserting observable equivalence.
//!
//! # What this double does NOT model
//!
//! `SimVmm` has no real child process, so nothing here ever exits "on
//! its own" within a `terminate` grace window — there is no guest, no
//! beacon, nothing that could cause an early clean exit in this step's
//! scope (that composition is `VmDriver`'s, landing later). `terminate`
//! therefore always resolves as [`VmTermination::Killed`] once a process
//! is live, which is the SAME outcome a real `cloud-hypervisor` process
//! settles on in the equivalent scenario (nothing asked the guest to
//! shut down, so the process only ever leaves via the grace-timeout
//! kill). The `FICLONE` clone is genuinely real bytes on disk — a plain
//! `std::fs::copy`, not the ioctl (this double has nothing to prove
//! about reflink savings) — because the observable fact the equivalence
//! test checks (`clone_dest` exists / does not exist, replaced not
//! adopted, removed on failure) must be identical to the host adapter's.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::traits::vmm::{
    Result, Vmm, VmControl, VmExitWatch, VmProcess, VmTermination, VmmError, VmmExit,
    VmmProbeError,
};
use overdrive_core::vm::config::VmConfig;
use parking_lot::Mutex;
use tokio::sync::oneshot;

/// Per-pid bookkeeping — mirrors the host adapter's `VmProcessState`
/// shape closely enough to reason about equivalence, without needing a
/// real reaper task (there is nothing to reap).
struct SimProcessEntry {
    terminated: bool,
    /// Consumed exactly once, on the first `terminate` call (mirrors the
    /// host adapter's `VmExitWatch` fill-once contract).
    exit_tx: Option<oneshot::Sender<VmmExit>>,
}

#[derive(Default)]
struct State {
    processes: BTreeMap<u32, SimProcessEntry>,
}

/// Sim binding of the [`Vmm`] port trait.
///
/// # Construction
///
/// ```
/// use overdrive_sim::SimVmm;
/// let sim = SimVmm::new();
/// ```
///
/// # Clone semantics
///
/// Cloning shares the underlying `Arc<Mutex<State>>` — mirrors
/// `SimClock` / `SimCgroupFs` so callers can hand one clone to the
/// harness and another to the system under test.
#[derive(Clone)]
pub struct SimVmm {
    state: Arc<Mutex<State>>,
    next_pid: Arc<AtomicU32>,
    /// Consumed by the NEXT `create` call only (mirrors
    /// `SimCgroupFs::inject_error`'s one-shot-per-call injection style,
    /// simplified to a single flag since `Vmm` has exactly one
    /// spawn-shaped operation). §D6's "spawn fails after the clone
    /// succeeded" edge case needs this to be exercisable on `SimVmm`
    /// too, per S-VM-90's driving sequence.
    fail_next_create: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for SimVmm {
    fn default() -> Self {
        Self::new()
    }
}

impl SimVmm {
    /// Construct an empty `SimVmm` — no live processes, no pending fault
    /// injection. Pids are minted from a base far above real-world PIDs
    /// so a test can never confuse a sim pid for a real one.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            next_pid: Arc::new(AtomicU32::new(1_000_000)),
            fail_next_create: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Arm a one-shot failure for the NEXT [`Vmm::create`] call — fires
    /// AFTER the clone step (matching the real adapter's own failure
    /// point: the clone can succeed and the spawn can still fail),
    /// leaving no live process behind and removing the clone it just
    /// made. Exercises §D6's "create removes its clone if the spawn
    /// fails" edge case against this adapter.
    pub fn inject_create_failure(&self) {
        self.fail_next_create.store(true, Ordering::SeqCst);
    }

    /// **TEST-HOOK-ONLY.** `true` iff `pid` is currently tracked as a
    /// live (not yet terminated) process.
    #[must_use]
    pub fn is_live(&self, pid: u32) -> bool {
        self.state.lock().processes.get(&pid).is_some_and(|entry| !entry.terminated)
    }
}

#[async_trait]
impl Vmm for SimVmm {
    fn kind(&self) -> &'static str {
        "sim"
    }

    async fn probe(&self) -> std::result::Result<(), VmmProbeError> {
        // Pure and stateless: nothing to round-trip, nothing that can
        // leave residue. Trivially idempotent (§D6).
        Ok(())
    }

    async fn create(&self, config: &VmConfig) -> Result<VmProcess> {
        let master = config.rootfs.master().to_path_buf();
        let clone_dest = config.rootfs.clone_dest().to_path_buf();

        // §D6: replace, never adopt, a stale clone from a prior launch.
        if clone_dest.exists() {
            std::fs::remove_file(&clone_dest).map_err(VmmError::Io)?;
        }
        std::fs::copy(&master, &clone_dest).map_err(VmmError::Io)?;

        if self.fail_next_create.swap(false, Ordering::SeqCst) {
            // §D6: the spawn "fails" after the clone succeeded — remove
            // it. No partial artifact escapes a failed `create`.
            let _ = std::fs::remove_file(&clone_dest);
            return Err(VmmError::create("SimVmm: injected create failure".to_string()));
        }

        let pid = self.next_pid.fetch_add(1, Ordering::SeqCst);
        let (exit_tx, exit_rx) = oneshot::channel::<VmmExit>();
        self.state.lock().processes.insert(pid, SimProcessEntry { terminated: false, exit_tx: Some(exit_tx) });

        Ok(VmProcess {
            control: VmControl { pid, api_socket: config.run_dir.api_socket() },
            exit: VmExitWatch::new(exit_rx),
        })
    }

    async fn terminate(&self, control: &VmControl, _grace: Duration) -> Result<VmTermination> {
        // Scoped tightly: the lock is held only long enough to flip
        // `terminated` and take the (already-live) exit sender back out
        // -- the actual `send` happens after the guard drops.
        let exit_tx = {
            let mut state = self.state.lock();
            let Some(entry) = state.processes.get_mut(&control.pid) else {
                // §D6: no record at all -- already gone (never tracked, or
                // a second adapter instance). Idempotent Killed.
                return Ok(VmTermination::Killed);
            };
            if entry.terminated {
                // §D6: "already gone" -- ALWAYS Killed, idempotently.
                return Ok(VmTermination::Killed);
            }
            entry.terminated = true;
            let taken = entry.exit_tx.take();
            drop(state);
            taken
        };
        // Nothing in this double ever exits "on its own" within grace
        // (see module doc) -- every live process settles as Killed, the
        // same outcome a real un-beaconed cloud-hypervisor process
        // reaches once its grace window elapses.
        if let Some(tx) = exit_tx {
            let _ = tx.send(VmmExit { exit_code: None, signal: Some(9), stderr_tail: None });
        }
        Ok(VmTermination::Killed)
    }
}
