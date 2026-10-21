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
//!
//! # The vsock device (CID-NOCLAIM, RESET-EXIT)
//!
//! `create(config, cid)` models the fork's `--vsock cid=<cid>,backend=vhost-kernel`
//! launch: the simulated VMM process opens its CID's `/dev/vhost-vsock`
//! instance itself and is its only holder until the process exits — by
//! `terminate`, by an injected exit of its own, by an external kill, or by a
//! guest reset, which ends the VMM (ADR-0182) rather than rebooting the guest
//! in place. A launch whose CID another live process holds cannot set up its
//! device: the process is created and exits at once, before any guest could
//! dial (the driver's existing pre-READY VMM-exit start failure); the holder
//! is unaffected (K-C1). Nothing ends a process when the composition that
//! started it is dropped, so a dropped `serve` composition leaves its VMMs
//! running as orphans holding their devices until something kills them
//! (NO-SURVIVE). [`SimVmm::vsock_device_holders`] is the pinned QPROJ-7
//! arbiter of these facts (Tier-3 pair: V-30 Q2a / Q2c / Q2d,
//! `overdrive-host` `vsock_vmm_device`; V-RECLAIM).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::guest_transport::GuestCid;
use overdrive_core::traits::driver::ConfinementControl;
use overdrive_core::traits::vmm::{
    Result, VmControl, VmExitWatch, VmProcess, VmTermination, Vmm, VmmDiagnostics, VmmError,
    VmmExit, VmmProbeError,
};
use overdrive_core::vm::config::VmConfig;
use parking_lot::Mutex;
use tokio::sync::oneshot;

/// Per-pid bookkeeping — mirrors the host adapter's `VmProcessState`
/// shape closely enough to reason about equivalence, without needing a
/// real reaper task (there is nothing to reap).
struct SimProcessEntry {
    terminated: bool,
    /// CID-NOCLAIM: the CID whose vhost-vsock device this process opened
    /// itself and holds until it exits. `None` for a bare process, and for a
    /// launch refused at device setup (its CID was held by another process).
    device: Option<GuestCid>,
    /// Consumed exactly once, on the first `terminate` call (mirrors the
    /// host adapter's `VmExitWatch` fill-once contract).
    exit_tx: Option<oneshot::Sender<VmmExit>>,
    /// Reader on this process's bounded capture. The final
    /// `VmmExit.stderr_tail` is read back off it at terminate time — the
    /// SAME ordering the host adapter's reaper uses, so live and final
    /// tails cannot disagree in either adapter.
    diagnostics: VmmDiagnostics,
    /// Scripted process ending, if one was injected. `None` keeps this
    /// double's established default (an un-beaconed process settles as
    /// `Killed`, signal 9).
    scripted_exit: Option<ScriptedEnding>,
}

/// One injected process ending: `(exit_code, signal)`, mirroring
/// [`VmmExit`]'s own two fields. Named so the injection slot below reads
/// as an ending rather than as an anonymous nest of options.
type ScriptedEnding = (Option<i32>, Option<u8>);

/// The armed [`Vmm::create`] fault: the verbatim diagnostic the refusal
/// carries, plus whether the arming survives the call that consumed it.
///
/// Both fields exist because the two consumers need opposite lifetimes.
/// §D6's "spawn fails after the clone succeeded" edge case wants a
/// one-shot arming, so the sequence under test observes exactly one
/// refusal. A scenario about an allocation that is *restarted* wants a
/// persistent one: an unmappable substrate failure does not heal between
/// attempts, and a one-shot arming would let attempt 2 succeed into a
/// different ending entirely — a race between the restart and the
/// observer, not a property of the failure being modelled.
struct CreateFault {
    detail: String,
    persistent: bool,
    /// When `Some`, the refusal is the TYPED
    /// [`VmmError::ConfinementUnavailable`] naming this control — the
    /// fail-closed producer S-VM-51 drives through the VM adapters the
    /// composition root passes in `NodeAdapters` (ADR-0083 §D8, ADR-0219): a host that cannot supply a required confinement
    /// control refuses the workload rather than starting it unconfined. When
    /// `None`, the refusal stays the existing unclassified [`VmmError::Create`]
    /// (S-VM-37's shape). No new injection mechanism — the SAME `create_fault`
    /// slot, carrying an already-existing typed error rather than a bare
    /// string.
    confinement: Option<ConfinementControl>,
}

/// The diagnostic [`SimVmm::inject_create_failure`] refuses with. Held as
/// a named constant because the equivalence and driver-contract suites
/// assert on this exact text.
const DEFAULT_CREATE_FAILURE_DETAIL: &str = "SimVmm: injected create failure";

#[derive(Default)]
struct State {
    processes: BTreeMap<u32, SimProcessEntry>,
}

/// Injectable probe-fault CLASS for [`SimVmm::probe`] (ADR-0083 §D8, GH #42).
///
/// A one-shot fault consumed by the NEXT `probe()` call, mirroring
/// [`SimVmm::inject_create_failure`]'s one-shot-per-call style.
///
/// Carries only the ADR-0082 §D5 **capability-flag** classes
/// (`LandlockFlagAbsent`, `LandlockLsmAbsent`, `KvmUnreachable`) plus
/// `RunDirUnusable` — per S-VM-13's crafter note, these are the classes for
/// which "no genuinely lying host exists in the Lima test envelope" is
/// true. The non-reflink class (§D5 scenario 1) is deliberately NOT
/// injectable here — S-VM-75 proves it against a REAL non-reflink
/// substrate instead (a real `CloudHypervisorVmm` constructed with
/// `.with_image_dir(...)` pointed at a genuinely non-reflink directory,
/// passed through the SAME `NodeAdapters` VM port this fault type serves —
/// the port is adapter-agnostic, `Arc<dyn Vmm>`, so either a faulty
/// `SimVmm` or a differently-configured real adapter satisfies it,
/// ADR-0219).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimVmmProbeFault {
    /// The installed `cloud-hypervisor` has no `--landlock` flag.
    LandlockFlagAbsent,
    /// The host kernel does not expose the Landlock LSM.
    LandlockLsmAbsent,
    /// `/dev/kvm` is not openable under the target identity.
    KvmUnreachable,
    /// The run-directory root is absent or unwritable.
    RunDirUnusable,
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
    /// The armed [`Vmm::create`] refusal, if any — see [`CreateFault`]
    /// for why the arming carries its own lifetime. Mirrors
    /// `SimCgroupFs::inject_error`'s injection style, collapsed to a
    /// single slot since `Vmm` has exactly one spawn-shaped operation.
    /// §D6's "spawn fails after the clone succeeded" edge case needs
    /// this to be exercisable on `SimVmm` too, per S-VM-90's driving
    /// sequence.
    create_fault: Arc<Mutex<Option<CreateFault>>>,
    /// Consumed by the NEXT `probe` call only — ADR-0083 §D8's probe fault,
    /// injected at the `Vmm` port the composition root passes in
    /// `NodeAdapters` (S-VM-13, ADR-0219).
    probe_fault: Arc<Mutex<Option<SimVmmProbeFault>>>,
    /// Console/stderr bytes the NEXT `create` call captures into its
    /// bounded diagnostics. The injected-script equivalent of a real
    /// hypervisor writing to its stderr pipe.
    scripted_console: Arc<Mutex<Option<Vec<u8>>>>,
    /// The ending the NEXT created process reports. Consumed by that one
    /// `create`, mirroring [`Self::inject_create_failure`]'s one-shot
    /// style.
    scripted_exit: Arc<Mutex<Option<ScriptedEnding>>>,
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
            create_fault: Arc::new(Mutex::new(None)),
            probe_fault: Arc::new(Mutex::new(None)),
            scripted_console: Arc::new(Mutex::new(None)),
            scripted_exit: Arc::new(Mutex::new(None)),
        }
    }

    /// Script the console/stderr bytes the NEXT [`Vmm::create`] captures
    /// into that process's bounded diagnostics — the injected-script
    /// stand-in for a real hypervisor writing to its stderr pipe.
    ///
    /// Byte-oriented and framing-agnostic, exactly like the real capture:
    /// callers may inject a partial line, many lines, or no newline at
    /// all, and the same bounds apply.
    pub fn inject_console_output(&self, bytes: impl Into<Vec<u8>>) {
        *self.scripted_console.lock() = Some(bytes.into());
    }

    /// Script the ending the NEXT created process reports, so a test can
    /// pin an exact exit code / terminating signal instead of this
    /// double's default `Killed`-with-signal-9 settle.
    pub fn inject_exit(&self, exit_code: Option<i32>, signal: Option<u8>) {
        *self.scripted_exit.lock() = Some((exit_code, signal));
    }

    /// Arm a one-shot failure for the NEXT [`Vmm::create`] call — fires
    /// AFTER the clone step (matching the real adapter's own failure
    /// point: the clone can succeed and the spawn can still fail),
    /// leaving no live process behind and removing the clone it just
    /// made. Exercises §D6's "create removes its clone if the spawn
    /// fails" edge case against this adapter.
    pub fn inject_create_failure(&self) {
        *self.create_fault.lock() = Some(CreateFault {
            detail: DEFAULT_CREATE_FAILURE_DETAIL.to_owned(),
            persistent: false,
            confinement: None,
        });
    }

    /// Arm a PERSISTENT [`Vmm::create`] refusal that fails CLOSED because the
    /// host cannot supply the required confinement `control` — the typed
    /// [`VmmError::ConfinementUnavailable`] carrying `detail` verbatim.
    ///
    /// This is the injection S-VM-51 drives through the VM adapters the
    /// composition root passes in `NodeAdapters` (ADR-0083 §D8, ADR-0219,
    /// US-VM-7 fail-closed):
    /// the Lima/metal test envelope runs one kernel, so no genuinely
    /// Landlock-less host exists in it (system constraint 1). Injecting the
    /// unavailable-control condition at the `Vmm` port boundary lets a REAL
    /// in-process `overdrive serve` drive the already-wired fail-closed path
    /// (`VmDriver::start` → `classify_vmm_error`'s `ConfinementUnavailable`
    /// arm → `TransitionReason::VmConfinementUnavailable` → allocation
    /// `Failed`) end to end, and NEVER starts the hypervisor unconfined.
    ///
    /// Persistent for the SAME reason [`Self::inject_persistent_create_failure`]
    /// is: an unavailable confinement control does not heal between attempts,
    /// so a one-shot arming would let a retry succeed into a different ending.
    ///
    /// No new mechanism and no new type: the existing `create_fault` slot
    /// carries the already-existing typed [`VmmError::ConfinementUnavailable`]
    /// (ADR-0082 confinement + [`ConfinementControl`]) rather than a bare
    /// string. `detail` is never a classification input downstream — class and
    /// diagnostic travel on independent channels, per the
    /// [`overdrive_core::traits::driver::DriverStartFailure`] contract.
    pub fn inject_persistent_confinement_unavailable(
        &self,
        control: ConfinementControl,
        detail: impl Into<String>,
    ) {
        *self.create_fault.lock() = Some(CreateFault {
            detail: detail.into(),
            persistent: true,
            confinement: Some(control),
        });
    }

    /// Arm a PERSISTENT [`Vmm::create`] refusal carrying `detail`
    /// verbatim — every `create` on this adapter fails identically until
    /// the fault is re-armed or replaced.
    ///
    /// Two properties distinguish this from
    /// [`Self::inject_create_failure`], and both are load-bearing for
    /// S-VM-37 (an unmapped VM start failure reaching the operator as the
    /// unclassified cause, GH #42 / ADR-0083 §D5):
    ///
    /// * **Caller-supplied diagnostic.** `detail` reaches
    ///   [`VmmError::Create`] byte-for-byte, so a scenario can pin a
    ///   sentinel that matches NO named `VmStartFailure` class and then
    ///   prove the platform preserved it rather than reworded it. A
    ///   fixed adapter-authored string cannot express that, and cannot
    ///   express the second half either — that varying ONLY the wording
    ///   leaves the selected cause class unchanged.
    /// * **Persistent arming.** The refusal must not heal between
    ///   restart attempts; see [`CreateFault`].
    ///
    /// `detail` is never interpreted here, and must never become a
    /// classification input downstream — the whole point of the typed
    /// [`overdrive_core::traits::driver::DriverStartFailure`] contract is
    /// that class and diagnostic travel on independent channels.
    pub fn inject_persistent_create_failure(&self, detail: impl Into<String>) {
        *self.create_fault.lock() =
            Some(CreateFault { detail: detail.into(), persistent: true, confinement: None });
    }

    /// Consume the armed `create` fault for ONE call, returning its verbatim
    /// `detail` and its optional confinement control (`Some` → fail closed with
    /// the typed [`VmmError::ConfinementUnavailable`]; `None` → the unclassified
    /// [`VmmError::Create`]). A one-shot arming disarms itself here; a
    /// persistent one stays armed. The lock is scoped to this function so no
    /// guard is ever held across the caller's `await` points.
    fn take_armed_create_fault(&self) -> Option<(String, Option<ConfinementControl>)> {
        let mut armed = self.create_fault.lock();
        let fault = armed.as_ref()?;
        let (detail, persistent, confinement) =
            (fault.detail.clone(), fault.persistent, fault.confinement);
        if !persistent {
            *armed = None;
        }
        drop(armed);
        Some((detail, confinement))
    }

    /// Arm a one-shot [`SimVmmProbeFault`] for the NEXT [`Vmm::probe`]
    /// call (ADR-0083 §D8, step 01-09) — the fault S-VM-13 drives through a
    /// real in-process `run_server` boot whose `NodeAdapters` carry this
    /// `SimVmm` (ADR-0219).
    pub fn inject_probe_failure(&self, fault: SimVmmProbeFault) {
        *self.probe_fault.lock() = Some(fault);
    }

    /// **TEST-HOOK-ONLY.** `true` iff `pid` is currently tracked as a
    /// live (not yet terminated) process.
    #[must_use]
    pub fn is_live(&self, pid: u32) -> bool {
        self.state.lock().processes.get(&pid).is_some_and(|entry| !entry.terminated)
    }

    /// **TEST-HOOK-ONLY.** Tracks a new live process without a rootfs clone
    /// and returns its control handle — for exercising a port double coupled
    /// to this VMM (`SimBeaconIo`, K-A2′) without staging an image on disk.
    /// [`Vmm::terminate`] ends it exactly as it ends a `create`d process.
    #[must_use]
    pub fn spawn_bare_process(&self) -> VmControl {
        let pid = self.next_pid.fetch_add(1, Ordering::SeqCst);
        let (diagnostics, _writer) = VmmDiagnostics::new();
        self.state.lock().processes.insert(
            pid,
            SimProcessEntry {
                terminated: false,
                device: None,
                exit_tx: None,
                diagnostics,
                scripted_exit: None,
            },
        );
        VmControl { pid, api_socket: std::path::PathBuf::from(format!("/sim/vm-{pid}/api.sock")) }
    }

    // -----------------------------------------------------------------------
    // The pinned arbiter (QPROJ-7)
    // -----------------------------------------------------------------------

    /// The CIDs whose `/dev/vhost-vsock` instance a live simulated VMM
    /// process holds, each with that process's pid. A VMM is the only holder
    /// of its device (CID-NOCLAIM); the entry is gone at the VMM's exit, a
    /// guest reset included (RESET-EXIT). Tier-3 pair: V-30 Q2a / Q2c / Q2d
    /// (`spike/findings-v30-guest-reset.md`) and V-RECLAIM.
    #[must_use]
    pub fn vsock_device_holders(&self) -> BTreeMap<GuestCid, u32> {
        self.state
            .lock()
            .processes
            .iter()
            .filter(|(_, entry)| !entry.terminated)
            .filter_map(|(pid, entry)| entry.device.map(|cid| (cid, *pid)))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Environment stepping (DISTILL test support)
    // -----------------------------------------------------------------------

    /// **TEST-HOOK-ONLY.** The guest of process `pid` resets — a PID-1 crash
    /// under `panic=1`, a triple fault, a reset register write. The pinned
    /// fork ends the VMM through its exit path (RESET-EXIT, ADR-0182): the
    /// process exits with no guest report and its device is released. `false`
    /// when `pid` is not a live process.
    pub fn guest_reset(&self, pid: u32) -> bool {
        self.end_process(pid, (Some(0), None))
    }

    /// **TEST-HOOK-ONLY.** Process `pid` exits by itself with `exit_code` /
    /// `signal` (a VMM crash). Its device is released. `false` when `pid` is
    /// not a live process.
    pub fn exit_process(&self, pid: u32, exit_code: Option<i32>, signal: Option<u8>) -> bool {
        self.end_process(pid, (exit_code, signal))
    }

    /// **TEST-HOOK-ONLY.** Process `pid` is killed from outside the adapter —
    /// a `cgroup.kill` of its scope, the SIGKILL boot reclamation sends an
    /// orphan of a previous `serve` process (NO-SURVIVE). It ends with signal
    /// 9 and its device is released. `false` when `pid` is not a live
    /// process.
    pub fn kill(&self, pid: u32) -> bool {
        self.end_process(pid, (None, Some(9)))
    }

    /// The pids of every live simulated process, in order.
    #[must_use]
    pub fn live_pids(&self) -> Vec<u32> {
        self.state
            .lock()
            .processes
            .iter()
            .filter(|(_, entry)| !entry.terminated)
            .map(|(pid, _)| *pid)
            .collect()
    }

    /// Ends live process `pid` with `ending`: the process is marked exited,
    /// its device is released, and its exit watch resolves once. The lock is
    /// released before the watch is filled. `false` when `pid` is not live.
    fn end_process(&self, pid: u32, ending: ScriptedEnding) -> bool {
        let taken = {
            let mut state = self.state.lock();
            let Some(entry) = state.processes.get_mut(&pid) else {
                return false;
            };
            if entry.terminated {
                return false;
            }
            entry.terminated = true;
            entry.device = None;
            let taken = (entry.exit_tx.take(), entry.diagnostics.clone());
            drop(state);
            taken
        };
        let (exit_tx, diagnostics) = taken;
        let (exit_code, signal) = ending;
        if let Some(tx) = exit_tx {
            let _ = tx.send(VmmExit { exit_code, signal, stderr_tail: diagnostics.console_tail() });
        }
        true
    }
}

/// The exit status a launch refused at vsock device setup ends with: the fork
/// cannot open the device or the kernel refuses the CID, and it exits before
/// READY (§ *VMM backend contract*). Diagnostic only; no owner branches on it.
const DEVICE_SETUP_REFUSED_EXIT: ScriptedEnding = (Some(1), None);

/// The console line a launch refused at vsock device setup leaves in its
/// bounded capture (the fork's own diagnostic, carried as context only).
const DEVICE_SETUP_REFUSED_DETAIL: &[u8] =
    b"cloud-hypervisor: error: vhost-vsock: setting guest CID failed: Address in use\n";

#[async_trait]
impl Vmm for SimVmm {
    fn kind(&self) -> &'static str {
        "sim"
    }

    async fn probe(&self) -> std::result::Result<(), VmmProbeError> {
        // One-shot fault consumption BEFORE the (otherwise pure,
        // stateless, trivially idempotent per §D6) healthy default —
        // mirrors `create`'s `fail_next_create.swap(...)` shape.
        let pending_fault = self.probe_fault.lock().take();
        if let Some(fault) = pending_fault {
            return Err(match fault {
                SimVmmProbeFault::LandlockFlagAbsent => VmmProbeError::landlock_flag_absent(
                    std::path::PathBuf::from("cloud-hypervisor"),
                    "sim-injected: no --landlock flag",
                ),
                SimVmmProbeFault::LandlockLsmAbsent => {
                    VmmProbeError::landlock_lsm_absent("sim-injected: landlock LSM absent")
                }
                SimVmmProbeFault::KvmUnreachable => VmmProbeError::kvm_unreachable(
                    0,
                    0,
                    0o660,
                    std::io::Error::from(std::io::ErrorKind::PermissionDenied),
                ),
                SimVmmProbeFault::RunDirUnusable => VmmProbeError::run_dir_unusable(
                    std::path::PathBuf::from("/sim/run-dir"),
                    std::io::Error::from(std::io::ErrorKind::PermissionDenied),
                ),
            });
        }
        Ok(())
    }

    async fn create(&self, config: &VmConfig, cid: GuestCid) -> Result<VmProcess> {
        let master = config.rootfs.master().to_path_buf();
        let clone_dest = config.rootfs.clone_dest().to_path_buf();

        // §D6: replace, never adopt, a stale clone from a prior launch.
        if clone_dest.exists() {
            std::fs::remove_file(&clone_dest).map_err(VmmError::Io)?;
        }
        std::fs::copy(&master, &clone_dest).map_err(VmmError::Io)?;

        if let Some((detail, confinement)) = self.take_armed_create_fault() {
            // §D6: the "spawn" fails after the clone succeeded — remove it. No
            // partial artifact escapes a failed `create`. Fail CLOSED with the
            // typed confinement refusal when a control was named (S-VM-51), else
            // the unclassified create refusal (S-VM-37) — the SAME slot, the
            // real typed error, never an unconfined `Ok(VmProcess)`.
            let _ = std::fs::remove_file(&clone_dest);
            return Err(match confinement {
                Some(control) => VmmError::ConfinementUnavailable { control, detail },
                None => VmmError::create(detail),
            });
        }

        let pid = self.next_pid.fetch_add(1, Ordering::SeqCst);
        let (exit_tx, exit_rx) = oneshot::channel::<VmmExit>();

        // ONE bounded capture per process, same as the host adapter. The
        // writer stays adapter-side and is dropped once the scripted
        // output is captured — this double has no long-lived pipe to read.
        let (diagnostics, writer) = VmmDiagnostics::new();
        // The lock is released BEFORE the branch body runs: a guard held
        // across the scrutinee would keep this double's console mutex
        // locked for the whole `if let`, which is the deadlock shape
        // `significant_drop_in_scrutinee` names.
        let scripted_console = self.scripted_console.lock().take();
        if let Some(scripted) = scripted_console {
            writer.append(&scripted);
        }
        let scripted_exit = self.scripted_exit.lock().take();

        // CID-NOCLAIM: the VMM opens its CID's device itself. A device another
        // live process holds refuses the open, and the VMM exits before any
        // guest could dial; the holder is unaffected (K-C1).
        let refused = {
            let mut state = self.state.lock();
            let held = state
                .processes
                .values()
                .any(|entry| !entry.terminated && entry.device == Some(cid));
            state.processes.insert(
                pid,
                SimProcessEntry {
                    terminated: false,
                    device: (!held).then_some(cid),
                    exit_tx: Some(exit_tx),
                    diagnostics: diagnostics.clone(),
                    scripted_exit,
                },
            );
            drop(state);
            held
        };
        if refused {
            writer.append(DEVICE_SETUP_REFUSED_DETAIL);
            self.end_process(pid, DEVICE_SETUP_REFUSED_EXIT);
        }

        Ok(VmProcess {
            control: VmControl { pid, api_socket: config.run_dir.api_socket() },
            exit: VmExitWatch::new(exit_rx),
            diagnostics,
        })
    }

    async fn terminate(&self, control: &VmControl, _grace: Duration) -> Result<VmTermination> {
        // Scoped tightly: the lock is held only long enough to flip
        // `terminated` and take the (already-live) exit sender back out
        // -- the actual `send` happens after the guard drops.
        let taken = {
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
            // The simulated VMM exits: its device is released with it.
            entry.device = None;
            let taken = (entry.exit_tx.take(), entry.diagnostics.clone(), entry.scripted_exit);
            drop(state);
            taken
        };
        let (exit_tx, diagnostics, scripted_exit) = taken;
        // Absent an injected script, nothing in this double ever exits "on
        // its own" within grace (see module doc) -- every live process
        // settles as Killed, the same outcome a real un-beaconed
        // cloud-hypervisor process reaches once its grace window elapses.
        let (exit_code, signal) = scripted_exit.unwrap_or((None, Some(9)));
        if let Some(tx) = exit_tx {
            // The final tail is READ BACK off the same bounded capture the
            // live reader observes -- never separately assembled.
            let _ = tx.send(VmmExit { exit_code, signal, stderr_tail: diagnostics.console_tail() });
        }
        Ok(VmTermination::Killed)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "test fixtures assert their preconditions")]
mod tests {
    //! The sim's own semantics of the vsock device (CID-NOCLAIM, RESET-EXIT,
    //! NO-SURVIVE): the facts [`SimVmm::vsock_device_holders`] reports as the
    //! QPROJ-7 arbiter, each paired with the real adapter's Tier-3 case in
    //! `overdrive-host/tests/integration/vsock_vmm_device.rs`.

    use std::num::NonZeroU8;

    use overdrive_core::AllocationId;
    use overdrive_core::cgroup::CgroupPath;
    use overdrive_core::guest_transport::GUEST_CID_BASE;
    use overdrive_core::vm::config::{
        Gid, HostArch, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan, VmConfinement, VmRunDir,
        VmmIdentity,
    };

    use super::*;

    fn cid(offset: u32) -> GuestCid {
        GuestCid::new(GUEST_CID_BASE + offset).expect("an in-range guest CID")
    }

    /// A launch config whose rootfs master is a real file under `root`.
    fn config(root: &std::path::Path, alloc: &str) -> VmConfig {
        let alloc = AllocationId::new(alloc).expect("valid allocation id");
        let master = root.join(format!("{alloc}-master.img"));
        std::fs::write(&master, [0_u8; 512]).expect("write rootfs master");
        let kernel = KernelImage::validate(root.join("vmlinux"), HostArch::X86_64, b"\x7fELF")
            .expect("an ELF header validates");
        VmConfig {
            alloc: alloc.clone(),
            kernel,
            rootfs: RootfsPlan::for_alloc(master, 512, &alloc, root, &root.join("clone-index")),
            cmdline: KernelCmdline::platform_default(HostArch::X86_64),
            memory: MemoryPlan::derive(128 * 1024 * 1024),
            vcpus: NonZeroU8::new(1).expect("nonzero"),
            run_dir: VmRunDir::for_alloc(root, &alloc),
            confinement: VmConfinement::confined(
                VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
                1024,
            ),
            cgroup_scope: CgroupPath::for_alloc(&alloc),
        }
    }

    /// CID-NOCLAIM: the launched process holds its CID's device until it
    /// exits; `terminate` releases it.
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER attachment-cut step (3)"]
    async fn a_launch_holds_its_cid_device_until_terminate() {
        let root = tempfile::tempdir().expect("tempdir");
        let sim = SimVmm::new();
        assert!(sim.vsock_device_holders().is_empty(), "precondition: no device held");

        let process = sim.create(&config(root.path(), "dev-a"), cid(2)).await.expect("create");
        assert_eq!(
            sim.vsock_device_holders(),
            BTreeMap::from([(cid(2), process.control.pid)]),
            "the VMM holds exactly its own CID's device"
        );

        sim.terminate(&process.control, Duration::ZERO).await.expect("terminate");
        assert!(sim.vsock_device_holders().is_empty(), "the exit released the device");
        assert!(!sim.is_live(process.control.pid));
    }

    /// K-C1: a launch on a held CID exits at once (before any guest could
    /// dial) with the device-setup diagnostic; the holder keeps its device.
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER attachment-cut step (3)"]
    async fn a_launch_on_a_held_cid_exits_before_ready_and_the_holder_is_unaffected() {
        let root = tempfile::tempdir().expect("tempdir");
        let sim = SimVmm::new();
        let holder = sim.create(&config(root.path(), "dev-holder"), cid(3)).await.expect("create");

        let mut clash = sim.create(&config(root.path(), "dev-clash"), cid(3)).await.expect("spawn");
        let exit = clash.exit.recv().await.expect("the refused VMM reports its ending");
        assert_eq!((exit.exit_code, exit.signal), DEVICE_SETUP_REFUSED_EXIT);
        assert!(
            exit.stderr_tail.as_deref().is_some_and(|tail| tail.contains("Address in use")),
            "the fork's diagnostic is carried as context: {exit:?}"
        );
        assert!(!sim.is_live(clash.control.pid), "the refused VMM is gone");
        assert_eq!(
            sim.vsock_device_holders(),
            BTreeMap::from([(cid(3), holder.control.pid)]),
            "the holder keeps its device"
        );
    }

    /// RESET-EXIT: a guest reset ends the VMM (no in-place reboot) and the
    /// device is released; the CID is free for the next launch at once.
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER attachment-cut step (3)"]
    async fn a_guest_reset_ends_the_vmm_and_frees_its_cid() {
        let root = tempfile::tempdir().expect("tempdir");
        let sim = SimVmm::new();
        let mut first =
            sim.create(&config(root.path(), "dev-reset"), cid(4)).await.expect("create");

        assert!(sim.guest_reset(first.control.pid), "the reset reaches a live VMM");
        assert!(first.exit.recv().await.is_some(), "the reset ended the VMM process");
        assert!(sim.vsock_device_holders().is_empty());
        assert!(!sim.guest_reset(first.control.pid), "a gone VMM cannot reset again");

        let next = sim.create(&config(root.path(), "dev-next"), cid(4)).await.expect("create");
        assert_eq!(sim.vsock_device_holders(), BTreeMap::from([(cid(4), next.control.pid)]));
    }

    /// NO-SURVIVE: nothing but an exit, a kill or a reset ends a process —
    /// dropping every handle of the launch (a crashed composition) leaves it
    /// running with its device until it is killed.
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    #[ignore = "pending DELIVER attachment-cut step (3)"]
    async fn an_orphan_keeps_its_device_until_killed() {
        let root = tempfile::tempdir().expect("tempdir");
        let sim = SimVmm::new();
        let pid = {
            let process =
                sim.create(&config(root.path(), "dev-orphan"), cid(5)).await.expect("create");
            process.control.pid
        };
        assert_eq!(sim.vsock_device_holders(), BTreeMap::from([(cid(5), pid)]), "still held");
        assert_eq!(sim.live_pids(), vec![pid]);

        assert!(sim.kill(pid), "the kill reaches the orphan");
        assert!(sim.vsock_device_holders().is_empty());
        assert!(sim.live_pids().is_empty());
        assert!(!sim.exit_process(pid, Some(0), None), "a killed process cannot exit again");
    }
}
