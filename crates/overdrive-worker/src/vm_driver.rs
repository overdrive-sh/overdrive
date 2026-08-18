//! `VmDriver` — `Driver` over `Arc<dyn Vmm>` (ADR-0082 §§D3-D4, GH #42).
//!
//! Owns everything `Vmm` deliberately does not: cgroup/netns placement,
//! the per-VM run directory, the host-side beacon `UnixListener` +
//! accepted session, the three-way boot race (guest beacon vs. VMM exit
//! vs. boot deadline), the guest-shutdown-then-terminate split in
//! `stop`, and the authorship-claim lifecycle (brief §105a.3) that a
//! future reclamation-shaped consumer reads through the two defaulted
//! `Driver` methods (`live_allocations`, `release_supervision`).
//!
//! Production wiring (a real `overdrive serve` composing
//! `CloudHypervisorVmm` + a real `VmHostLayout`) lands at the
//! `DriverRegistry` step; this step's own evidence is a
//! `VmDriver`-level acceptance suite against `SimVmm`
//! (`tests/acceptance/vm_driver_stop_totality.rs`) — the enforcement
//! vehicle ADR-0082 §D4 names by name, because `vmm_equivalence.rs`
//! drives the `Vmm` port only and structurally cannot reach the
//! relocated guest half of `stop`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::cgroup_accounting::CgroupAccounting;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverPayload,
    DriverStartClass, DriverStartFailure, DriverType, ExitEvent, ExitKind, OomFacts, Resources,
    VmStartFailure,
};
use overdrive_core::traits::vmm::{VmControl, VmExitWatch, Vmm, VmmDiagnostics, VmmError, VmmExit};
use overdrive_core::vm::beacon::{BEACON_VSOCK_PORT, BeaconMessage};
use overdrive_core::vm::config::{
    HostArch, KERNEL_MAGIC_WINDOW, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan, VmConfig,
    VmConfinement, VmRunDir, vcpus_for,
};
use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use crate::cgroup_manager::{CgroupManager, CgroupPath};

/// ADR-0082 §D3 — the boot race's give-up bound. A policy constant in
/// the driver, not persisted and not test-overridable: the slowest
/// measured substrate is 8.7 s (nested aarch64), plus guest fsck and
/// the three `CONFIG_VSOCKETS=m` module loads; 30 s is ~3.4x the worst
/// observation.
const VM_BOOT_DEADLINE: Duration = Duration::from_secs(30);

/// ADR-0082 §D4 — bounds `stop`'s step 1: the `SHUTDOWN` write's chance
/// to take effect before escalating to `Vmm::terminate`. A guest not
/// reading its socket must not block `stop`.
const VM_SHUTDOWN_REQUEST_DEADLINE: Duration = Duration::from_secs(2);

/// ADR-0082 §D4 — bounds `stop`'s step 2 (`Vmm::terminate`'s grace
/// window). Mirrors `ExecDriver`'s `DEFAULT_STOP_GRACE` role rather
/// than inventing a new policy shape.
const VM_STOP_GRACE: Duration = Duration::from_secs(10);

/// Bounded cooperative-yield budget for draining an already-buffered
/// guest `EXIT` report after the VMM's own exit resolves first at a
/// `select!` poll. Mirrors `overdrive_worker::driver::
/// STDERR_DRAIN_MAX_YIELDS` — cooperative yields, never a
/// `Clock::sleep`, per `.claude/rules/development.md` § "Production
/// code is not shaped by simulation".
const GUEST_REPORT_DRAIN_MAX_YIELDS: u32 = 64;

/// Capacity of the per-driver `ExitEvent` channel. Mirrors
/// `overdrive_worker::driver::EXIT_CHANNEL_CAPACITY`.
const EXIT_CHANNEL_CAPACITY: usize = 256;

/// Construct a `DriverError::StartRejected` carrying a typed VM cause plus
/// the verbatim low-level diagnostic (ADR-0083 §D5, DWD-24). Mirrors
/// `overdrive_worker::driver::start_rejected`.
fn start_rejected(class: VmStartFailure, detail: impl Into<String>) -> DriverError {
    DriverError::StartRejected {
        failure: DriverStartFailure { class: DriverStartClass::Vm(class), detail: detail.into() },
    }
}

/// Construct a `DriverError::StartRejected` for a VM failure with no named
/// class. Converts to the pre-existing `DriverInternalError` — the only
/// unknown fallback, never a guessed named cause.
fn start_rejected_unclassified(detail: impl Into<String>) -> DriverError {
    DriverError::StartRejected {
        failure: DriverStartFailure {
            class: DriverStartClass::Unclassified { driver: DriverType::Vm },
            detail: detail.into(),
        },
    }
}

/// The STRUCTURAL `VmmError -> DriverStartClass` join (ADR-0082 §D1.1).
/// Selection is by VARIANT only — no `VmmError::Display` string selects a
/// class, so changing an adapter's prose cannot change the operator's
/// diagnosis. `rootfs` is consumed ONLY to enrich the unclassified `Io`
/// arm's free-form detail with the operator's rootfs directory (ADR-0083
/// §D3b); it never participates in class selection. `@mandatory:mutation_target`.
fn classify_vmm_error(err: &VmmError, rootfs: &RootfsPlan) -> DriverError {
    match err {
        // The detail is the WHOLE `VmmError`, not its inner `source`: the
        // adapter's own `Display` is the low-level diagnostic that names
        // every path searched, and dropping to the bare `io::Error`
        // ("No such file or directory") discards exactly the evidence an
        // operator acts on. Still verbatim adapter text, never a
        // classification input.
        VmmError::HypervisorAbsent { searched, .. } => start_rejected(
            VmStartFailure::HypervisorAbsent { searched: searched.clone() },
            err.to_string(),
        ),
        VmmError::RootfsNotFound { path, source } => start_rejected(
            VmStartFailure::RootfsNotFound { path: path.display().to_string() },
            source.to_string(),
        ),
        VmmError::ConfinementUnavailable { control, detail } => start_rejected(
            VmStartFailure::ConfinementUnavailable { control: *control, detail: detail.clone() },
            detail.clone(),
        ),
        // A staging/spawn failure the adapter does not further distinguish
        // reaches the explicit unknown fallback rather than being guessed
        // into an absence class.
        VmmError::Create { detail } => start_rejected_unclassified(detail.clone()),
        // `create`'s ONLY `Io` source is `ficlone_rootfs` — the per-launch
        // rootfs clone (host adapter: "Every other staging failure stays
        // Io / Create", whose sole `Io` producer there is the clone). After
        // ADR-0083 §D3a the clone targets the operator's OWN rootfs directory
        // (the parent of `[vm] rootfs`), which `Vmm::probe`'s boot-time
        // reflink self-test no longer speaks for (§D3b: `FICLONE` is
        // intra-filesystem). Name that directory and the filesystem capability
        // the clone requires, rather than leaving the operator a bare
        // internal-shaped I/O error. Class selection stays by variant
        // (`Io` -> unclassified) — `rootfs` enriches only the detail — and no
        // `VmStartFailure` variant is minted here: S-VM-94 owns that typing.
        VmmError::Io { .. } => {
            let clone_dir = rootfs.master().parent().unwrap_or_else(|| std::path::Path::new(""));
            start_rejected_unclassified(format!(
                "per-launch rootfs clone into {} failed: {err}; this directory's filesystem \
                 must support reflink (FICLONE) and be writable",
                clone_dir.display(),
            ))
        }
    }
}

/// Per-allocation kernel preflight (ADR-0082 §D2.4, ADR-0083 §D3b): open
/// the path THIS allocation's `[vm]` spec names, read a bounded magic
/// window, and run the pure validator immediately before `Vmm::create`.
///
/// This is the ONLY site where a guest kernel is validated. Artifacts are
/// per-allocation (§D3a), so there is no node-wide kernel to have proven
/// once at boot — the proof is scoped to the allocation that names the
/// path, which is the only honest scope for a per-workload input.
/// `KernelImage::validate` stays the sole constructor of [`KernelImage`];
/// this returns the value it built rather than discarding it, so
/// [`VmConfig`] can consume it. Reads only; it never writes the path or
/// its directory.
async fn preflight_kernel(
    path: &std::path::Path,
    arch: HostArch,
) -> Result<KernelImage, DriverError> {
    let path = path.to_path_buf();
    let header = match read_kernel_magic_window(&path).await {
        Ok(header) => header,
        // The detail NAMES the configured path, mirroring the rootfs
        // preflight's `stat rootfs master {configured}: {err}` shape. A
        // bare `io::Error` ("No such file or directory") tells an operator
        // nothing about WHICH artifact went missing.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(start_rejected(
                VmStartFailure::KernelNotFound { path: path.display().to_string() },
                format!("open kernel image {}: {err}", path.display()),
            ));
        }
        Err(err) => {
            return Err(start_rejected_unclassified(format!(
                "read kernel header {}: {err}",
                path.display()
            )));
        }
    };

    // The validator's own diagnosis is the payload — never the
    // hypervisor's misleading firmware-size-cap wording (C-7).
    KernelImage::validate(path.clone(), arch, &header).map_err(|format_err| {
        start_rejected(
            VmStartFailure::KernelFormatUnsupported {
                path: path.display().to_string(),
                arch: arch.to_string(),
                detail: format_err.to_string(),
            },
            format_err.to_string(),
        )
    })
}

/// Read at most [`KERNEL_MAGIC_WINDOW`] bytes off the named kernel.
/// A short file is not an error here — the pure validator decides whether
/// the bytes it got constitute a loadable image.
async fn read_kernel_magic_window(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;

    let file = tokio::fs::File::open(path).await?;
    let mut header = Vec::with_capacity(KERNEL_MAGIC_WINDOW);
    file.take(KERNEL_MAGIC_WINDOW as u64).read_to_end(&mut header).await?;
    Ok(header)
}

/// Record this launch's rootfs clone location in the platform-owned
/// durable index by creating a symlink `index_link -> clone_dest`
/// (ADR-0083 §§D3f-D3h). Called BEFORE the FICLONE so the link's lifetime
/// contains the clone's. Idempotent against a stale link left by a prior
/// crashed launch of the SAME allocation id: the pre-existing link is
/// removed (NotFound-tolerant) before the fresh one is created, since
/// `symlink(2)` fails `EEXIST` otherwise.
async fn create_index_link(rootfs: &RootfsPlan) -> std::io::Result<()> {
    // Ensure the platform-owned clone-staging root exists before the FICLONE
    // writes the clone into it (ADR-0082 2026-08-18 fourth amendment). In
    // production `compose_vm_driver` created it once at node setup with the
    // confined-identity traverse posture (`0710 root:<gid>`); `create_dir_all`
    // is idempotent and NEVER alters an existing directory's mode, so this
    // never disturbs that posture — it is the driver's belt-and-braces (and
    // lets a harness that bypasses composition stage a clone).
    if let Some(staging) = rootfs.clone_dest().parent() {
        tokio::fs::create_dir_all(staging).await?;
    }
    let link = rootfs.index_link();
    if let Some(parent) = link.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    match tokio::fs::remove_file(link).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    tokio::fs::symlink(rootfs.clone_dest(), link).await
}

/// Remove the rootfs clone FIRST, then its index link (ADR-0083 §D3f) —
/// the ordering that makes `no link ⇒ no clone` true across a crash
/// between the two steps. Best-effort and NotFound-tolerant, but the link
/// is dropped ONLY once the clone is gone (removed now, or already
/// absent). If the clone removal fails for any reason OTHER than
/// `NotFound` — the operator's rootfs directory went read-only / `EROFS` /
/// `EACCES` between the FICLONE and stop — the surviving clone MUST keep
/// its index entry, or it becomes an orphan no reclamation sweep can ever
/// enumerate (§D3h `no link ⇒ no clone`; the "clone exists, link absent"
/// state §D3f's crash table marks *unreachable by construction*). The
/// non-`NotFound` error is surfaced, not swallowed
/// (`.claude/rules/development.md` § "Errors"), and the link is left for
/// the sweep to retry — mirroring the sweep-side twin
/// `RealVmHostState::discard_artifacts`, which likewise returns before
/// touching the link on a non-`NotFound` clone-removal failure. Stop stays
/// best-effort: a failed clone removal must not crash stop, but it must NOT
/// actively create the invisible orphan by removing the link.
/// `@mandatory:mutation_target` — swapping the two removals removes the
/// link first and reopens the "clone without a link" window S-VM-85 exists
/// to close (the `stop_keeps_the_index_link_when_the_clone_removal_fails`
/// test drives exactly this).
async fn remove_clone_then_index_link(rootfs: &RootfsPlan) {
    match tokio::fs::remove_file(rootfs.clone_dest()).await {
        // Clone gone (removed now, or already absent) — safe to drop the link.
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        // Clone still on disk for a reason other than absence — removing the
        // link now would strand it invisibly. Keep the link so the sweep
        // retries; surface the cause rather than absorbing it into silence.
        Err(err) => {
            warn!(
                clone = %rootfs.clone_dest().display(),
                error = %err,
                "per-launch rootfs clone removal failed (non-NotFound); keeping the clone-index \
                 link so the reclamation sweep retries — removing it would orphan the clone \
                 (ADR-0083 §§D3f/D3h)"
            );
            return;
        }
    }
    // The clone is gone — now drop its index link. Best-effort and
    // NotFound-tolerant (a concurrent double-stop may have removed it).
    let _ = tokio::fs::remove_file(rootfs.index_link()).await;
}

// ---------------------------------------------------------------------
// VmHostLayout — the per-node, fixed VM-boot template this slice needs
// ---------------------------------------------------------------------

/// The genuinely node-invariant inputs `VmDriver::start` needs to build a
/// [`VmConfig`] that `AllocationSpec` (generic across every driver type)
/// does not carry.
///
/// **Artifacts are NOT here** (ADR-0083 §D3a): the guest kernel and rootfs
/// are per-allocation, read from that allocation's own `VmPayload`
/// (`[vm] kernel` / `[vm] rootfs`), never from node state. There is no
/// node-level artifact configuration anywhere in the process — that is
/// what makes two workloads on one node able to boot two different
/// images, and what lets GH #259's image factory fill the same
/// `VmPayload` fields without `VmDriver` changing at all. Every field
/// that remains is a property of the HOST, not of a workload.
///
/// Every field is `pub` and there is no validating constructor beyond
/// what the field types themselves already enforce — this is
/// `VmDriver`-internal plumbing, not a port or a cross-crate contract.
#[derive(Debug, Clone)]
pub struct VmHostLayout {
    /// Cgroupfs root (`/sys/fs/cgroup` in production). `VmDriver::new`
    /// carries no separate `cgroup_root` parameter (ADR-0082 §D1 pins
    /// its arity at exactly four), so this is where it lives.
    pub cgroup_root: PathBuf,
    /// Root directory under which per-allocation [`VmRunDir`]s are
    /// created (SD-2 — tmpfs in production).
    pub run_dir_root: PathBuf,
    /// The platform-owned clone-index directory
    /// ([`overdrive_core::vm::config::clone_index_dir`] over the node's
    /// durable `data_dir`) where `start` records each launch's rootfs
    /// clone location as a symlink BEFORE the FICLONE (ADR-0083 §§D3f-D3h).
    /// The SAME expression `RealVmHostState`'s `index_dir` is fed, so the
    /// sweep enumerates exactly the links `start` writes.
    pub clone_index_dir: PathBuf,
    /// The platform-owned VM clone-staging root
    /// ([`overdrive_core::vm::config::clone_staging_dir`] over the node's
    /// durable `data_dir`) where each per-launch rootfs clone is FICLONE'd —
    /// a platform-owned directory, never beside the operator's master, so the
    /// confined identity never needs an operator-dir traverse grant (ADR-0082
    /// 2026-08-18 fourth amendment, B1 fix). MUST share the rootfs master's
    /// filesystem (FICLONE is intra-filesystem); `compose_vm_driver` creates
    /// it once with the confined-identity traverse posture at node setup.
    pub clone_staging_dir: PathBuf,
    /// Host CPU architecture — selects the guest cmdline's console
    /// token via [`KernelCmdline::platform_default`], and the
    /// architecture each allocation's kernel is validated against.
    pub arch: HostArch,
    /// The confined identity + rlimits Cloud Hypervisor is spawned
    /// under.
    pub confinement: VmConfinement,
}

// ---------------------------------------------------------------------
// The authorship claim (brief §105a.3) — VmSupervision, LiveVm
// ---------------------------------------------------------------------

/// A host-side beacon session: the write half of the accepted
/// connection the guest opened. [`VmDriver::stop`] writes `SHUTDOWN`
/// on it (ADR-0082 §D4); the read half is moved into the per-alloc
/// exit watcher at accept time and never touches `VmDriver` state
/// again.
struct BeaconSession {
    write_half: OwnedWriteHalf,
}

impl BeaconSession {
    /// Best-effort graceful-shutdown request. Callers ignore the
    /// `Result` — an unresponsive guest, or a connection the peer has
    /// already closed, must not surface as an error; both fall through
    /// to `Vmm::terminate` (ADR-0082 §D4).
    async fn write_shutdown(&mut self) -> std::io::Result<()> {
        self.write_half.write_all(b"SHUTDOWN\n").await?;
        self.write_half.flush().await
    }
}

/// The claim's `Held` payload once `Vmm::create` has returned
/// (ADR-0082 §D4). `beacon` is `None` until the guest dials — the
/// type-level statement of the pre-beacon window
/// (`.claude/rules/development.md` § "Sum types over sentinels").
struct LiveVm {
    control: VmControl,
    beacon: Option<BeaconSession>,
    scope: CgroupPath,
    run_dir: VmRunDir,
    rootfs: RootfsPlan,
    /// "Running-confirmed" gate sender — the [`Driver::start`]
    /// post-condition every `ExitEvent`-emitting driver must honour
    /// (`overdrive_core::traits::driver`), mirroring `ExecDriver`'s
    /// `LiveAllocation::gate_sender`. The action shim takes it via
    /// [`Driver::release_for_exit_emission`] after
    /// `obs.write(AllocStatus::Running)` commits Ok (or after the May-2
    /// degraded-escalation `LifecycleEvent` path); the matching
    /// `oneshot::Receiver` is handed to [`run_exit_watcher`] and awaited
    /// BEFORE its first `ExitEvent` send. That is the happens-before
    /// edge preventing the exit observer's `find_prior_row → NoPriorRow`
    /// silent-drop when a guest exits sub-millisecond after receiving
    /// its command, before the Running row commits.
    ///
    /// `Some` from the beacon-win arm of `start` until
    /// [`Driver::release_for_exit_emission`] `take()`s it; `None`
    /// thereafter (idempotent fire). Dropped when `stop` /
    /// `release_supervision` replaces or removes this entry — the
    /// watcher's `gate_receiver.await` then resolves `Err(RecvError)`
    /// and emit proceeds (orphan path), per the `Driver::start` rustdoc
    /// § "Sender drop (orphan path)".
    gate_sender: Option<oneshot::Sender<()>>,
}

/// The authorship claim on one allocation's ending, in one of three
/// phases (brief §105a.3). EVERY variant is supervised —
/// [`VmDriver::live_allocations`] reports all three; reporting only
/// `Live` is exactly the defect DD-1(b.i) refuses.
///
/// `Held` (the transition table's term) means `Starting | Live` — the
/// two variants `VmDriver` itself holds; `EndingInFlight` is the
/// hand-off to the ending-authoring path (out of this step's scope —
/// no caller drives that transition yet).
enum VmSupervision {
    /// Claimed; the boot race is in progress (step 0, before the run
    /// directory exists).
    Starting,
    /// Running: the claim plus the per-allocation live state.
    Live(LiveVm),
    /// The ending is being authored; the live state has been released.
    /// Reachable via [`ClaimGuard::try_begin_ending`] (transition 3 —
    /// the exit watcher's natural-exit path) OR synchronously from
    /// [`VmDriver::stop`] on the operator-initiated stop path (brief
    /// §105a.3 transition 3b) — two independent producers of this SAME
    /// terminal-pending state, both satisfying the `Driver::stop`
    /// post-condition (driver.rs) that a subsequent `status()` reports
    /// `Err(NotFound)`.
    EndingInFlight,
}

type LiveMap = Mutex<BTreeMap<AllocationId, VmSupervision>>;

/// RAII guard implementing claim transitions 3 and 4 (brief §105a.3).
/// Constructed once per exit-watcher invocation. On successful hand-off
/// ([`Self::try_begin_ending`] returning `true`), the guard's `Drop` is
/// a no-op — transition 3 already moved the entry to `EndingInFlight`.
/// If the watcher task ends WITHOUT a successful hand-off (the entry
/// was no longer `Held` when checked), `Drop` removes the entry — but
/// ONLY if it is STILL `Held` at drop time (transition 4: "only from
/// Held"), which also makes an unwind or an abort safe: the guard's
/// `Drop` still runs and still obeys the same guard.
struct ClaimGuard {
    alloc: AllocationId,
    live: Arc<LiveMap>,
    emitted: bool,
}

impl ClaimGuard {
    const fn new(alloc: AllocationId, live: Arc<LiveMap>) -> Self {
        Self { alloc, live, emitted: false }
    }

    /// Transition 3: an atomic `Held -> EndingInFlight` check-and-act
    /// (`.claude/rules/development.md` § "Check-and-act must be
    /// atomic") whose return value IS the verdict gating `ExitEvent`
    /// emission. `@mandatory:mutation_target` — a mutation that ignores
    /// this verdict re-opens the "ending authored twice" hazard the
    /// atomicity exists to close.
    fn try_begin_ending(&mut self) -> bool {
        let mut live = self.live.lock();
        let held =
            matches!(live.get(&self.alloc), Some(VmSupervision::Starting | VmSupervision::Live(_)));
        if held {
            live.insert(self.alloc.clone(), VmSupervision::EndingInFlight);
            self.emitted = true;
        }
        drop(live);
        held
    }
}

impl Drop for ClaimGuard {
    fn drop(&mut self) {
        if self.emitted {
            return;
        }
        // Transition 4: Held -> ∅, ONLY from Held. If some other path
        // already moved the entry away from Held (or it is already
        // absent), this is correctly a no-op.
        let mut live = self.live.lock();
        if matches!(live.get(&self.alloc), Some(VmSupervision::Starting | VmSupervision::Live(_))) {
            live.remove(&self.alloc);
        }
    }
}

// ---------------------------------------------------------------------
// VmDriver
// ---------------------------------------------------------------------

/// `Driver` implementation over `Arc<dyn Vmm>` (ADR-0082 §§D1, D3-D4).
#[derive(Clone)]
pub struct VmDriver {
    vmm: Arc<dyn Vmm>,
    clock: Arc<dyn Clock>,
    cgroup_manager: CgroupManager,
    cgroup_accounting: Arc<dyn CgroupAccounting>,
    layout: VmHostLayout,
    live: Arc<LiveMap>,
    exit_tx: mpsc::Sender<ExitEvent>,
    exit_rx: Arc<Mutex<Option<mpsc::Receiver<ExitEvent>>>>,
}

impl VmDriver {
    /// Every port is a mandatory constructor parameter — no
    /// `with_vmm`-style builder override
    /// (`.claude/rules/development.md` § "Port-trait dependencies").
    /// Arity and parameter order are pinned by ADR-0082 §§D1, D8 —
    /// `cgroup_accounting` was added ahead of `layout` by the D-3 fold-in
    /// (`VmDriver::new(Arc::new(vmm), clock, fs, cgroup_accounting,
    /// vm_layout)`).
    #[must_use]
    pub fn new(
        vmm: Arc<dyn Vmm>,
        clock: Arc<dyn Clock>,
        fs: Arc<dyn CgroupFs>,
        cgroup_accounting: Arc<dyn CgroupAccounting>,
        layout: VmHostLayout,
    ) -> Self {
        let (exit_tx, exit_rx) = mpsc::channel(EXIT_CHANNEL_CAPACITY);
        let cgroup_manager = CgroupManager::new(layout.cgroup_root.clone(), fs);
        Self {
            vmm,
            clock,
            cgroup_manager,
            cgroup_accounting,
            layout,
            live: Arc::new(Mutex::new(BTreeMap::new())),
            exit_tx,
            exit_rx: Arc::new(Mutex::new(Some(exit_rx))),
        }
    }

    /// Transition 2: `Held -> ∅` on any non-`Ok` return of `start` — but
    /// ONLY when the entry is STILL `Held` (`Starting` or `Live`) at the
    /// moment `start`'s failure-cleanup runs. A CONDITIONAL removal
    /// (`.claude/rules/development.md` § "Check-and-act must be
    /// atomic"), mirroring [`ClaimGuard::drop`]'s same guard.
    ///
    /// 01-07 RE-REVIEW remediation (HIGH): this corrects the PRIOR
    /// premise that nothing else could touch this entry while `start`
    /// unwinds. It is false — an operator `stop()` call (brief §105a.3
    /// transition 3b) can race `start`'s own boot-failure cleanup: a
    /// PRE-BEACON `stop()` moves this SAME entry `Live -> EndingInFlight`
    /// synchronously under the lock, then calls `Vmm::terminate`, which
    /// is exactly what resolves the in-flight `start`'s `exit.recv()`
    /// race arm and drives it into THIS cleanup path. When that
    /// interleaving occurs, `stop` — not `start`'s unwind — owns the
    /// ending, and this method must NOT clobber it: an unconditional
    /// remove would strip the allocation out of `live_allocations()`
    /// entirely, reopening the second-authorship hazard
    /// `EndingInFlightIsNeverReclaimed` (brief §105a.11) forbids.
    /// `@mandatory:mutation_target` — a mutant that widens this back to
    /// an unconditional remove must be caught by
    /// `stop_sequence_a_pre_beacon_stop_skips_write_and_terminates`'s
    /// post-interleaving `live_allocations()` retention assertion.
    fn release_claim(&self, alloc: &AllocationId) {
        let mut live = self.live.lock();
        if matches!(live.get(alloc), Some(VmSupervision::Starting | VmSupervision::Live(_))) {
            live.remove(alloc);
        }
    }

    /// Cleanup shared by every non-`Ok` arm of `start`'s boot sequence:
    /// SIGKILL the VMM (if one was ever spawned), remove the rootfs clone
    /// then its index link, `cgroup.kill` + remove the workload scope,
    /// remove the run directory (which also removes the beacon socket
    /// file), and release the claim taken at step 0. Every step is
    /// best-effort — this function is ALREADY the failure path; a
    /// secondary failure here must not mask the original error nor panic.
    ///
    /// `control` and `rootfs` are separate `Option`s, not one bundled
    /// tuple: the index link is created BEFORE `Vmm::create` (ADR-0083
    /// §D3f), so the `Vmm::create`-failure arm has a `RootfsPlan` (whose
    /// link must be removed) but no `VmControl`.
    async fn cleanup_after_start_failure(
        &self,
        alloc: &AllocationId,
        run_dir: &VmRunDir,
        scope: Option<&CgroupPath>,
        control: Option<&VmControl>,
        rootfs: Option<&RootfsPlan>,
    ) {
        if let Some(control) = control {
            let _ = self.vmm.terminate(control, Duration::ZERO).await;
        }
        if let Some(rootfs) = rootfs {
            remove_clone_then_index_link(rootfs).await;
        }
        if let Some(scope) = scope {
            let _ = self.cgroup_manager.cgroup_kill(scope).await;
            let _ = self.cgroup_manager.remove_workload_scope(scope).await;
        }
        let _ = tokio::fs::remove_dir_all(run_dir.path()).await;
        self.release_claim(alloc);
    }

    /// Steps 0 through `Vmm::create` of `start`'s boot sequence
    /// (ADR-0082 §D3) — claim the supervision slot, create the run
    /// directory and beacon listener, provision the cgroup scope and
    /// resource limits, stage the rootfs plan, build the [`VmConfig`],
    /// and spawn the confined hypervisor process. Split out of `start`
    /// itself purely to stay under the file's line-count budget; every
    /// cleanup call and its trigger point are unchanged from the
    /// pre-split body.
    #[expect(
        clippy::too_many_lines,
        reason = "linear VM boot sequence (steps 0..Vmm::create, ADR-0082 §D3); rustfmt 1.95.0 wrapping the cleanup_after_start_failure calls tips it past 100"
    )]
    async fn provision_vmm(&self, spec: &AllocationSpec) -> Result<ProvisionedVmm, DriverError> {
        // ADR-0083 §D3a: the kernel and rootfs are THIS allocation's own,
        // read from the `[vm]` block the operator wrote. There is no
        // node-level artifact anywhere to fall back to, which is exactly
        // why the platform can no longer silently ignore what the spec
        // named. `VmPayload`'s fields are already `pub`, so the refutable
        // binding reaches them and states the routing precondition in the
        // same breath — no accessor is added to `DriverPayload`.
        //
        // A non-`Vm` payload reaching `VmDriver` is a registry-ROUTING
        // defect, not a VM-start failure, so it takes the existing
        // `DriverStartClass::Unclassified` fallback rather than minting a
        // class of its own. It runs before step 0 below: nothing has been
        // claimed or provisioned yet, so there is nothing to release.
        let DriverPayload::Vm(payload) = &spec.driver else {
            return Err(start_rejected_unclassified(format!(
                "VmDriver received a {} payload",
                spec.driver.driver_type()
            )));
        };

        // Step 0 (brief §105a.3, transition 1): take the supervision
        // claim BEFORE the run directory exists. The ordinal is
        // load-bearing — see ADR-0082 §D4 / brief §103's "the claim is
        // step 0" — not tidy sequencing.
        self.live.lock().insert(spec.alloc.clone(), VmSupervision::Starting);

        let run_dir = VmRunDir::for_alloc(&self.layout.run_dir_root, &spec.alloc);
        if let Err(err) = tokio::fs::create_dir_all(run_dir.path()).await {
            self.release_claim(&spec.alloc);
            return Err(start_rejected_unclassified(format!("create VM run directory: {err}")));
        }

        // Per-allocation artifact preflight (ADR-0082 §D2.4). Runs before
        // anything else is provisioned so a deleted/replaced kernel costs
        // no scope, no clone, and no hypervisor spawn.
        let kernel = match preflight_kernel(&payload.kernel, self.layout.arch).await {
            Ok(kernel) => kernel,
            Err(rejection) => {
                self.cleanup_after_start_failure(&spec.alloc, &run_dir, None, None, None).await;
                return Err(rejection);
            }
        };

        // The listener must exist before the guest can dial (SD-1
        // handoff item 6's pinned start-path ordering).
        let listener = match UnixListener::bind(run_dir.beacon_socket(BEACON_VSOCK_PORT)) {
            Ok(listener) => listener,
            Err(err) => {
                self.cleanup_after_start_failure(&spec.alloc, &run_dir, None, None, None).await;
                return Err(start_rejected_unclassified(format!("bind beacon listener: {err}")));
            }
        };

        let scope = CgroupPath::for_alloc(&spec.alloc);
        if let Err(err) = self.cgroup_manager.create_workload_scope(&scope).await {
            drop(listener);
            self.cleanup_after_start_failure(&spec.alloc, &run_dir, None, None, None).await;
            return Err(start_rejected_unclassified(format!("create workload scope: {err}")));
        }

        // Resource limits: memory.max MUST be the reserve-padded
        // ceiling, never the guest's declared RAM directly (ADR-0082
        // §D2.3 lie 3 — the whole reason `MemoryPlan` exists). CPU
        // share passes through from the spec unchanged. Warn-and-continue
        // on write failure, mirroring ExecDriver's ADR-0026 D9 discipline
        // — a limit-write failure must not abort a boot that is
        // otherwise healthy.
        let memory = MemoryPlan::derive(spec.resources.memory_bytes);
        let cgroup_resources = Resources {
            cpu_milli: spec.resources.cpu_milli,
            memory_bytes: memory.cgroup_max_bytes(),
        };
        if let Err(err) = self.cgroup_manager.write_resource_limits(&scope, &cgroup_resources).await
        {
            warn!(
                alloc = %spec.alloc,
                scope = %scope,
                error = %err,
                "cgroup resource-limit write failed; continuing per ADR-0026 D9"
            );
        }

        // Per-allocation rootfs preflight. `NotFound` is the ONE arm that
        // names the absence class; a permission error / EIO / broken mount
        // is NOT relabelled as absence — it reaches the unknown fallback,
        // so the operator never gets "file missing" remediation for a
        // permissions problem (`.claude/rules/development.md` § Errors).
        let master_bytes = match tokio::fs::metadata(&payload.rootfs).await {
            Ok(meta) => meta.len(),
            Err(err) => {
                self.cleanup_after_start_failure(&spec.alloc, &run_dir, Some(&scope), None, None)
                    .await;
                let configured = payload.rootfs.display().to_string();
                let detail = format!("stat rootfs master {configured}: {err}");
                return Err(if err.kind() == std::io::ErrorKind::NotFound {
                    start_rejected(VmStartFailure::RootfsNotFound { path: configured }, detail)
                } else {
                    start_rejected_unclassified(detail)
                });
            }
        };
        let rootfs = RootfsPlan::for_alloc(
            payload.rootfs.clone(),
            master_bytes,
            &spec.alloc,
            &self.layout.clone_staging_dir,
            &self.layout.clone_index_dir,
        );
        let cmdline = KernelCmdline::platform_default(self.layout.arch);
        let config = VmConfig {
            alloc: spec.alloc.clone(),
            kernel,
            rootfs: rootfs.clone(),
            cmdline,
            memory,
            // vCPUs are DERIVED per-allocation from this allocation's own
            // `[resources] cpu_milli` — `max(1, round_up(cpu_milli/1000))`,
            // floor 1 (US-VM-5, ADR-0082 §D2). `[resources]` is the single
            // source of truth for VM size — there is no per-node vCPU
            // template (step 06-01).
            vcpus: vcpus_for(spec.resources.cpu_milli),
            run_dir: run_dir.clone(),
            confinement: self.layout.confinement.clone(),
            netns: spec.netns.clone(),
            cgroup_scope: scope.clone(),
        };

        // ADR-0083 §D3f: record the clone's location in the platform-owned
        // durable index BEFORE the FICLONE runs (inside `Vmm::create`). The
        // link's lifetime CONTAINS the clone's, so at no instant does a
        // clone exist without a link — enumerating the index enumerates a
        // superset of live clones and the reclamation sweep cannot miss
        // one. The ORDERING is the correctness contract, not sequencing;
        // swapping it reopens the invisible-orphan leak S-VM-85
        // mutation-tests. `@mandatory:mutation_target`.
        if let Err(err) = create_index_link(&rootfs).await {
            self.cleanup_after_start_failure(
                &spec.alloc,
                &run_dir,
                Some(&scope),
                None,
                Some(&rootfs),
            )
            .await;
            return Err(start_rejected_unclassified(format!("create clone-index link: {err}")));
        }

        let (control, exit, diagnostics) = match self.vmm.create(&config).await {
            Ok(process) => (process.control, process.exit, process.diagnostics),
            Err(err) => {
                // §D6: `Vmm::create`'s own Err contract guarantees no
                // process is left running and no clone is left on disk —
                // but the index link created just above IS ours to remove
                // (the §D3f "after link, before clone" residue), so pass
                // the `RootfsPlan` through to strip the dangling link.
                self.cleanup_after_start_failure(
                    &spec.alloc,
                    &run_dir,
                    Some(&scope),
                    None,
                    Some(&rootfs),
                )
                .await;
                return Err(classify_vmm_error(&err, &rootfs));
            }
        };

        Ok(ProvisionedVmm { run_dir, listener, scope, control, exit, diagnostics, rootfs, memory })
    }

    /// Clone every handle [`run_exit_watcher`] needs off `self` and spawn
    /// it. Split out of `start`'s beacon-win arm purely to stay under the
    /// file's line-count budget — every parameter and the spawned body
    /// are otherwise unchanged from the pre-split call.
    fn spawn_exit_watcher_task(
        &self,
        alloc: AllocationId,
        exit: VmExitWatch,
        reader: BufReader<OwnedReadHalf>,
        scope: CgroupPath,
        limit_bytes: u64,
        gate_receiver: oneshot::Receiver<()>,
    ) {
        let watcher_live = Arc::clone(&self.live);
        let watcher_tx = self.exit_tx.clone();
        let watcher_cgroup_accounting = Arc::clone(&self.cgroup_accounting);
        let watcher_cgroup_root = self.layout.cgroup_root.clone();
        tokio::spawn(async move {
            run_exit_watcher(
                alloc,
                exit,
                reader,
                watcher_live,
                watcher_tx,
                watcher_cgroup_accounting,
                watcher_cgroup_root,
                scope,
                limit_bytes,
                gate_receiver,
            )
            .await;
        });
    }
}

/// The three-way race's outcome (ADR-0082 §D3), named so `start`'s
/// `match` reads as the design's own three arms rather than an
/// anonymous tuple.
enum BootRaceOutcome {
    Beacon(std::io::Result<(BufReader<OwnedReadHalf>, OwnedWriteHalf)>),
    /// Carries the RESOLVED [`VmmExit`] rather than discarding it: the
    /// hypervisor's exit code, terminating signal, and final stderr tail
    /// are the operator-visible facts this arm exists to preserve
    /// (ADR-0082 §D3). `None` means the watch closed without reporting.
    VmmExited(Option<VmmExit>),
    Deadline,
}

/// What [`VmDriver::provision_vmm`] hands to `start`'s three-way race:
/// the spawned, confined hypervisor process plus every host-footprint
/// handle `start`'s non-`Ok` race arms clean up (ADR-0082 §D3).
struct ProvisionedVmm {
    run_dir: VmRunDir,
    listener: UnixListener,
    scope: CgroupPath,
    control: VmControl,
    exit: VmExitWatch,
    /// READ handle on this process's bounded capture. The deadline arm
    /// snapshots it live — the only way to report what an unresponsive
    /// guest printed, since `VmmExit` exists only after termination.
    diagnostics: VmmDiagnostics,
    rootfs: RootfsPlan,
    /// Carried through to the exit watcher so its post-mortem
    /// `CgroupAccounting::oom_kill_count` read (ADR-0082 §D8) can report
    /// `OomFacts::limit_bytes` at zero extra I/O —
    /// `MemoryPlan::cgroup_max_bytes()` is already known from `start`.
    memory: MemoryPlan,
}

// `too_many_lines` allow: the lint measures the whole `#[async_trait]`-
// expanded impl, not one method, and `start`'s body IS the ADR-0082 §D3
// boot sequence — provision, the three-way race, and one cleanup-and-
// reject arm per outcome. Splitting the arms apart to satisfy a line
// count would scatter the cleanup contract (`@mandatory:mutation_target`
// on every non-Ok arm) across call sites, which is the property this
// file is most at risk of leaking. `allow`, not `expect`: the flagged
// length depends on `#[cfg(target_os = "linux")]` code that is absent on
// a macOS host check, so an `expect` would itself go unfulfilled there.
#[allow(clippy::too_many_lines)]
#[async_trait]
impl Driver for VmDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        let ProvisionedVmm {
            run_dir,
            listener,
            scope,
            control,
            mut exit,
            diagnostics,
            rootfs,
            memory,
        } = self.provision_vmm(spec).await?;

        // Transition Starting -> Live (still §103 step 0's Held phase;
        // brief §105a.3's transition table treats Starting|Live as one
        // "Held" state — this is the internal refinement, not a new
        // transition).
        self.live.lock().insert(
            spec.alloc.clone(),
            VmSupervision::Live(LiveVm {
                control: control.clone(),
                beacon: None,
                scope: scope.clone(),
                run_dir: run_dir.clone(),
                rootfs: rootfs.clone(),
                // Minted in the beacon-win arm below, once the guest has
                // dialled and the exit watcher is about to spawn — there
                // is no watcher to gate until then.
                gate_sender: None,
            }),
        );

        if let Err(err) = self.cgroup_manager.place_pid_in_scope(&scope, control.pid).await {
            self.cleanup_after_start_failure(
                &spec.alloc,
                &run_dir,
                Some(&scope),
                Some(&control),
                Some(&rootfs),
            )
            .await;
            return Err(start_rejected_unclassified(format!("place VMM pid in scope: {err}")));
        }

        // The three-way race (ADR-0082 §D3). `biased;` is load-bearing:
        // if the beacon and the VMM exit are both ready, the beacon
        // wins — a guest that beaconed and then died is a STARTED VM
        // whose ending belongs to the exit watcher, not to `start`.
        let outcome = tokio::select! {
            biased;
            accepted = accept_ready(&listener) => BootRaceOutcome::Beacon(accepted),
            ended = exit.recv() => BootRaceOutcome::VmmExited(ended),
            () = self.clock.sleep(VM_BOOT_DEADLINE) => BootRaceOutcome::Deadline,
        };

        match outcome {
            BootRaceOutcome::Beacon(Ok((reader, mut write_half))) => {
                // ADR-0082 §D7 amendment (GH #42, item 2): the
                // operator's command travels host -> guest as EXEC,
                // immediately after READY is accepted and before
                // anything else touches this connection — the kernel
                // cmdline never carries it. Written BEFORE the beacon
                // session is stored / start returns Ok, so a write
                // failure still takes the ordinary cleanup-and-reject
                // path below rather than leaving a Live entry with no
                // EXEC ever sent.
                let argv: Vec<String> = std::iter::once(spec.driver.command().to_owned())
                    .chain(spec.driver.args().iter().cloned())
                    .collect();
                let exec_message = BeaconMessage::Exec { argv };
                let exec_write = async {
                    write_half.write_all(format!("{exec_message}\n").as_bytes()).await?;
                    write_half.flush().await
                }
                .await;

                if let Err(err) = exec_write {
                    self.cleanup_after_start_failure(
                        &spec.alloc,
                        &run_dir,
                        Some(&scope),
                        Some(&control),
                        Some(&rootfs),
                    )
                    .await;
                    let detail = format!("EXEC write failed: {err}");
                    return Err(start_rejected(
                        VmStartFailure::GuestCommandDispatchFailed { detail: detail.clone() },
                        detail,
                    ));
                }

                // Mint the Running-confirmed gate (the `Driver::start`
                // post-condition, mirroring `ExecDriver`). The sender is
                // stashed on the `LiveVm` entry; the action shim takes it
                // via `Driver::release_for_exit_emission` after
                // `obs.write(Running)` commits Ok (or via the exit
                // observer's May-2 degraded path). The receiver is handed
                // to the watcher and awaited BEFORE its first `ExitEvent`
                // send — the happens-before edge that stops a
                // sub-millisecond-lifetime guest's exit racing the Running
                // write into the observer's `find_prior_row → NoPriorRow`
                // silent-drop.
                let (gate_sender, gate_receiver) = oneshot::channel::<()>();
                {
                    let mut live = self.live.lock();
                    if let Some(VmSupervision::Live(live_vm)) = live.get_mut(&spec.alloc) {
                        live_vm.beacon = Some(BeaconSession { write_half });
                        live_vm.gate_sender = Some(gate_sender);
                    }
                    // If the entry is no longer `Live` (a concurrent stop
                    // raced in), `gate_sender` drops here — the watcher's
                    // `gate_receiver.await` then resolves `Err(RecvError)`
                    // and emit proceeds via the orphan path, which is
                    // correct: there is no Running row to gate against.
                }
                self.spawn_exit_watcher_task(
                    spec.alloc.clone(),
                    exit,
                    reader,
                    scope,
                    memory.cgroup_max_bytes(),
                    gate_receiver,
                );
                Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(control.pid) })
            }
            // @mandatory:mutation_target — every non-Ok race arm below
            // MUST clean up the scope/dir/clone/VMM and release the
            // claim (brief §113 / ADR-0082 §D3's "the arm an
            // implementation is most likely to leak on").
            BootRaceOutcome::Beacon(Err(err)) => {
                self.cleanup_after_start_failure(
                    &spec.alloc,
                    &run_dir,
                    Some(&scope),
                    Some(&control),
                    Some(&rootfs),
                )
                .await;
                Err(start_rejected_unclassified(format!("beacon accept failed: {err}")))
            }
            // The VMM's own ending is CONSUMED, never discarded: the exit
            // code and terminating signal become the typed cause, and the
            // hypervisor's captured stderr becomes the verbatim detail.
            BootRaceOutcome::VmmExited(ended) => {
                self.cleanup_after_start_failure(
                    &spec.alloc,
                    &run_dir,
                    Some(&scope),
                    Some(&control),
                    Some(&rootfs),
                )
                .await;
                let (vmm_exit_code, vmm_signal, detail) = match ended {
                    Some(VmmExit { exit_code, signal, stderr_tail }) => (
                        exit_code,
                        signal,
                        stderr_tail.unwrap_or_else(|| {
                            "VMM exited before the guest signalled ready; no stderr captured"
                                .to_owned()
                        }),
                    ),
                    // A stable channel-closed diagnostic — the watch was
                    // torn down before it observed an exit.
                    None => {
                        (None, None, "VMM exit watch closed before reporting an exit".to_owned())
                    }
                };
                Err(start_rejected(
                    VmStartFailure::GuestExitUnreported { vmm_exit_code, vmm_signal },
                    detail,
                ))
            }
            // The live capture is snapshotted HERE, while the process is
            // still up — `VmmExit` does not exist yet on this arm, so the
            // tail can come from nowhere else.
            BootRaceOutcome::Deadline => {
                let console_tail = diagnostics.console_tail();
                self.cleanup_after_start_failure(
                    &spec.alloc,
                    &run_dir,
                    Some(&scope),
                    Some(&control),
                    Some(&rootfs),
                )
                .await;
                let deadline_ms = u64::try_from(VM_BOOT_DEADLINE.as_millis()).unwrap_or(u64::MAX);
                Err(start_rejected(
                    VmStartFailure::BootDeadlineExceeded {
                        deadline_ms,
                        console_tail: console_tail.clone(),
                    },
                    console_tail.unwrap_or_else(|| {
                        format!(
                            "boot deadline ({deadline_ms}ms) elapsed with no beacon; no console output captured"
                        )
                    }),
                ))
            }
        }
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        let extracted = {
            let mut live = self.live.lock();
            let fields = match live.get_mut(&handle.alloc) {
                Some(VmSupervision::Live(live_vm)) => Some((
                    live_vm.control.clone(),
                    live_vm.beacon.take(),
                    live_vm.scope.clone(),
                    live_vm.run_dir.clone(),
                    live_vm.rootfs.clone(),
                )),
                _ => None,
            };
            // Transition 3b (brief §105a.3): the operator-stop path's
            // OWN synchronous Live -> EndingInFlight move, under the
            // SAME lock as the extraction above. Required by the
            // `Driver::stop` post-condition (driver.rs) — a subsequent
            // `status()` must report `Err(NotFound)` immediately, not
            // only once the exit watcher's independent transition 3
            // eventually runs. Never a full remove: `status` and
            // `live_allocations` both already treat `EndingInFlight`
            // as "not Held", and a full remove would reopen a second
            // authorship path onto the SAME allocation's ending
            // (`VmReclamation`'s future `PlatformReclaimed`, ADR-0082
            // §D4) — exactly the hazard `EndingInFlightIsNeverReclaimed`
            // exists to forbid.
            //
            // This replace also RELEASES THE RUNNING-GATE. The prior
            // `Live(LiveVm)` value is dropped here, and with it the
            // stashed `LiveVm.gate_sender` (see its field docs). So even
            // when the action shim never fired the gate —
            // `obs.write(Running)` failed, so `release_for_exit_emission`
            // was skipped — the watcher's `gate_receiver.await` resolves
            // `Err(RecvError)` (the `Driver::start` § "Sender drop"
            // orphan path) and the watcher proceeds instead of stranding
            // on the gate. This is the implicit-drop analogue of
            // `ExecDriver::stop`'s explicit `drop(gate_sender)`;
            // `release_supervision` releases the gate the same way by
            // removing the entry.
            // `@mandatory:mutation_target` — a mutant that drops or
            // no-ops this insert leaves the entry `Live` after `stop`
            // returns `Ok`, so `status` keeps reporting `Running`
            // instead of the `Driver::stop` post-condition's
            // `Err(NotFound)`; `stop_ok_then_status_reports_not_found`
            // exists to catch exactly that.
            if fields.is_some() {
                live.insert(handle.alloc.clone(), VmSupervision::EndingInFlight);
            }
            fields
        };
        let Some((control, beacon, scope, run_dir, rootfs)) = extracted else {
            return Err(DriverError::NotFound { alloc: handle.alloc.clone() });
        };

        // Step 1 (ADR-0082 §D4): if a beacon session exists, write
        // SHUTDOWN best-effort, then bound the guest's chance to react.
        // Pre-beacon stop (S-VM-76 sequence (a)) skips this entirely —
        // there is no connection to write to, and `beacon.take()` above
        // already makes a SECOND `stop` call (sequence (d)) take this
        // same skip path too.
        if let Some(mut session) = beacon {
            let _ = session.write_shutdown().await;
            self.clock.sleep(VM_SHUTDOWN_REQUEST_DEADLINE).await;
        }

        // Step 2: bound how long the process is given to comply.
        // `Vmm::terminate` is idempotent on an already-dead process
        // (S-VM-76 sequence (c)) — `Ok(VmTermination::Killed)`, never an
        // error.
        let _ = self.vmm.terminate(&control, VM_STOP_GRACE).await;

        // Step 3: tear down the host footprint. Best-effort — benign if
        // already gone (a concurrent double-stop, S-VM-76 sequence (d)).
        // The clone is removed FIRST and its index link SECOND (ADR-0083
        // §D3f), so `no link ⇒ no clone` holds at every interruption point
        // of the stop sequence — a crash after the clone's removal leaves
        // at most a dangling link the next sweep disposes idempotently.
        // `VmDriver::stop` still removes the clone DIRECTLY (it holds this
        // allocation's own `RootfsPlan`); the sweep is the backstop for the
        // without-stop endings, not a replacement for this.
        let _ = self.cgroup_manager.cgroup_kill(&scope).await;
        let _ = self.cgroup_manager.remove_workload_scope(&scope).await;
        let _ = tokio::fs::remove_dir_all(run_dir.path()).await;
        remove_clone_then_index_link(&rootfs).await;

        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        let live = self.live.lock();
        match live.get(&handle.alloc) {
            Some(VmSupervision::Starting | VmSupervision::Live(_)) => Ok(AllocationState::Running),
            Some(VmSupervision::EndingInFlight) | None => {
                Err(DriverError::NotFound { alloc: handle.alloc.clone() })
            }
        }
    }

    async fn resize(
        &self,
        handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        // Resize is NOT implemented by this driver in this feature. The
        // hotplug substrate — Cloud Hypervisor's `--api-socket` — is kept
        // in `VmConfig` for GH #92 (right-sizing / CPU hotplug) but is
        // exercised by no path here (ADR-0082 §D4 Amendment 2026-08-18),
        // so resize REJECTS HONESTLY rather than silently no-oping — a
        // silent `Ok(())` would falsely tell the operator a resource
        // change landed. UNCONDITIONAL: the driver has no resize mechanism
        // at all, so a live-or-not allocation is refused identically;
        // `NotFound` would wrongly assert the allocation is absent when
        // resize is called on a running VM, and `Ok(())` would fabricate a
        // change no path implements. A resize-refusal is its own failure
        // mode, so it gets its own typed variant callers can `matches!` on
        // (`.claude/rules/development.md` § "Errors").
        Err(DriverError::ResizeUnsupported {
            driver: DriverType::Vm,
            alloc: handle.alloc.clone(),
            detail: "resize (right-sizing / CPU hotplug) is deferred to GH #92; the \
                     --api-socket hotplug substrate is kept in VmConfig for that work but is \
                     not exercised by any path in this feature"
                .to_owned(),
        })
    }

    fn take_exit_receiver(&self) -> Option<mpsc::Receiver<ExitEvent>> {
        self.exit_rx.lock().take()
    }

    /// Fire the Running-confirmed gate for `handle.alloc`, releasing the
    /// exit watcher's pre-emit await. Idempotent: a call against an
    /// alloc whose gate has already fired, whose entry is no longer
    /// `Live` (a stop / `release_supervision` already dropped the
    /// sender), or which is unknown to the driver, is a no-op. The
    /// structural exactly-once guarantee is `Option::take` +
    /// `oneshot::Sender::send` consume-self. See the `Driver::start`
    /// rustdoc post-condition (`overdrive_core::traits::driver`).
    fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        // Hold the lock only long enough to take the sender — never
        // across an `.await` (we do not await here; the discipline is
        // uniform, `.claude/rules/development.md` § Concurrency & async).
        let sender = self.live.lock().get_mut(&handle.alloc).and_then(|sup| match sup {
            VmSupervision::Live(live_vm) => live_vm.gate_sender.take(),
            VmSupervision::Starting | VmSupervision::EndingInFlight => None,
        });
        if let Some(sender) = sender {
            // `send` consumes self — double-fire is structurally
            // impossible. `Err(())` from a closed receiver (watcher
            // already dropped, e.g. mid-flight stop) is benign.
            let _ = sender.send(());
        }
    }

    /// EVERY variant of [`VmSupervision`] is supervised — reporting
    /// only `Live` is exactly the defect this method's trait-level
    /// contract refuses.
    fn live_allocations(&self) -> Option<Vec<AllocationId>> {
        Some(self.live.lock().keys().cloned().collect())
    }

    /// Idempotent by construction: removing an absent key from a
    /// `BTreeMap` is already a safe no-op.
    fn release_supervision(&self, alloc: &AllocationId) {
        self.live.lock().remove(alloc);
    }
}

/// The beacon arm of the three-way race: accept one connection and
/// confirm its first line is `READY ...`. A cancelled call (the exit or
/// deadline arm wins instead) abandons the connection entirely — the
/// caller tears down the whole run directory in that case, so losing
/// any bytes buffered mid-read is inconsequential (nothing reads this
/// SAME connection again).
async fn accept_ready(
    listener: &UnixListener,
) -> std::io::Result<(BufReader<OwnedReadHalf>, OwnedWriteHalf)> {
    let (stream, _addr) = listener.accept().await?;
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    match line.parse::<BeaconMessage>() {
        Ok(BeaconMessage::Ready { .. }) => Ok((reader, write_half)),
        Ok(other) => {
            Err(std::io::Error::other(format!("beacon accept: expected READY, got {other:?}")))
        }
        Err(parse_err) => Err(std::io::Error::other(format!(
            "beacon accept: unparseable first line: {parse_err}"
        ))),
    }
}

/// ADR-0082 §D3 / brief §103's exit-classification join. The guest's
/// OWN report is authoritative; the hypervisor PROCESS's own exit
/// status (`VmmExit::exit_code`) is NEVER consulted — intake precedent
/// warning #3's bug (a guest that boots, panics, and powers off cleanly
/// still exits the VMM `0`). `vmm_signal` (never `exit_code`) is
/// consulted ONLY in the no-report fallback row.
/// `@mandatory:mutation_target`.
fn classify_vm_exit(guest_report: Option<i32>, vmm_signal: Option<u8>) -> ExitKind {
    match guest_report {
        Some(0) => ExitKind::CleanExit,
        Some(status) => ExitKind::Crashed { exit_code: Some(status), signal: None },
        None => ExitKind::Crashed { exit_code: None, signal: vmm_signal.map(i32::from) },
    }
}

/// Cancel-safe single-line read over `reader`, accumulating into
/// `accumulated` across repeated calls. Uses `fill_buf`/`consume`
/// (both cancel-safe per tokio's `AsyncBufReadExt` contract) rather
/// than `read_line`, whose partial-read data loss on cancellation
/// (documented on `AsyncBufReadExt::read_line`: "if the method is used
/// as the event in a `tokio::select!` ... and some other branch
/// completes first, then it is possible that a line was partially
/// read") is exactly the hazard `drain_guest_report` below would hit —
/// this future IS raced, and re-created, inside a `select!` loop.
async fn read_one_line(
    reader: &mut BufReader<OwnedReadHalf>,
    accumulated: &mut Vec<u8>,
) -> std::io::Result<Option<String>> {
    loop {
        if let Some(pos) = accumulated.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = accumulated.drain(..=pos).collect();
            return Ok(Some(String::from_utf8_lossy(&line_bytes).into_owned()));
        }
        let n = {
            let buf = reader.fill_buf().await?;
            if buf.is_empty() {
                return Ok(None); // EOF -- peer closed the connection.
            }
            accumulated.extend_from_slice(buf);
            buf.len()
        };
        reader.consume(n);
    }
}

/// Race the guest's `EXIT <status>` beacon line against the VMM's own
/// process exit, with the read side preferred (`biased;`) and a bounded
/// cooperative-yield drain if the VMM's exit resolves first at a poll —
/// mirrors `ExecDriver`'s stderr-drain-before-emit shape
/// (`driver.rs::spawn_exit_watcher`), never a `Clock::sleep`. Returns
/// `(guest_reported_status, vmm_own_signal)`; the caller,
/// `classify_vm_exit`, never reads the VMM's own `exit_code`.
async fn drain_guest_report(
    exit: &mut VmExitWatch,
    mut reader: BufReader<OwnedReadHalf>,
) -> (Option<i32>, Option<u8>) {
    let mut accumulated = Vec::new();

    let vmm_signal = tokio::select! {
        biased;
        line = read_one_line(&mut reader, &mut accumulated) => {
            return finish_guest_report_line(line, None);
        }
        vmm_exit = exit.recv() => vmm_exit.and_then(|e| e.signal),
    };

    for _ in 0..GUEST_REPORT_DRAIN_MAX_YIELDS {
        tokio::select! {
            biased;
            line = read_one_line(&mut reader, &mut accumulated) => {
                return finish_guest_report_line(line, vmm_signal);
            }
            () = tokio::task::yield_now() => {}
        }
    }
    (None, vmm_signal)
}

/// Parses one drained line into the guest-reported exit status, if
/// any. `Ok(None)` (EOF) or an I/O error both mean "no report" — the
/// no-report fallback row.
fn finish_guest_report_line(
    line: std::io::Result<Option<String>>,
    vmm_signal: Option<u8>,
) -> (Option<i32>, Option<u8>) {
    match line {
        Ok(Some(line)) => match line.parse::<BeaconMessage>() {
            Ok(BeaconMessage::Exit { status }) => (Some(status), None),
            _ => (None, vmm_signal),
        },
        Ok(None) | Err(_) => (None, vmm_signal),
    }
}

/// The per-allocation exit watcher spawned by `start`'s beacon-win arm.
/// Owns `exit` (the `VmExitWatch` moved intact out of the boot race —
/// ADR-0082 §D3's correction: `VmExitWatch::recv` borrows so it can be
/// raced and then handed on) and the accepted connection's read half.
///
/// # OOM diagnosis (ADR-0082 §D8, the D-3 fold-in)
///
/// Immediately after `drain_guest_report` resolves and BEFORE any
/// teardown — this watcher performs none of its own; teardown happens
/// later, in `stop` or `cleanup_after_start_failure`, both driven by a
/// SEPARATE caller — reads `cgroup_accounting.oom_kill_count` against
/// this allocation's own `memory.events`, but ONLY on the "no agent EXIT
/// report, VMM died" branch (`guest_report.is_none()`). A guest that
/// self-reported an exit status is never second-guessed by a cgroup
/// read: the guest's own report is authoritative (the same precedence
/// `classify_vm_exit` already gives it over the VMM's own signal).
#[allow(clippy::too_many_arguments)]
async fn run_exit_watcher(
    alloc: AllocationId,
    mut exit: VmExitWatch,
    reader: BufReader<OwnedReadHalf>,
    live: Arc<LiveMap>,
    exit_tx: mpsc::Sender<ExitEvent>,
    cgroup_accounting: Arc<dyn CgroupAccounting>,
    cgroup_root: PathBuf,
    scope: CgroupPath,
    limit_bytes: u64,
    gate_receiver: oneshot::Receiver<()>,
) {
    let (guest_report, vmm_signal) = drain_guest_report(&mut exit, reader).await;
    let kind = classify_vm_exit(guest_report, vmm_signal);
    let oom = if guest_report.is_none() {
        let memory_events_path = scope.resolve(&cgroup_root).join("memory.events");
        match cgroup_accounting.oom_kill_count(&memory_events_path).await {
            Ok(count) if count > 0 => Some(OomFacts { limit_bytes, oom_kill_count: count }),
            // A read error, or a genuinely-zero counter, both collapse to
            // "not observed to be OOM" per `ExitEvent::oom`'s own
            // documented contract -- `None` does NOT mean "confirmed not
            // OOM."
            _ => None,
        }
    } else {
        None
    };

    // Running-confirmed gate: block until the action shim signals that
    // `obs.write(Running)` has committed (`release_for_exit_emission`),
    // or until the gate sender is dropped (orphan path: `RecvError`).
    // This is the structural happens-before edge the `Driver::start`
    // post-condition mandates for every `ExitEvent`-emitting driver;
    // without it a guest that exits before the Running row commits has
    // its only exit event dropped by the observer's `find_prior_row →
    // NoPriorRow` arm, stranding the allocation as Running forever.
    //
    // ORDERING is load-bearing: the await MUST precede
    // `try_begin_ending` below. That transition replaces the `Live`
    // entry with `EndingInFlight`, dropping the stashed `gate_sender`;
    // awaiting after it would always resolve `RecvError` and the gate
    // would never actually block. `tokio::sync::oneshot` is not
    // `Clock`-dependent — this is a logical edge, identical under
    // `SimClock`, turmoil, and real tokio (`.claude/rules/development.md`
    // § "Production code is not shaped by simulation").
    if gate_receiver.await.is_err() {
        tracing::debug!(
            alloc = %alloc,
            "vm exit_watcher: gate sender dropped before fire; \
             proceeding with ExitEvent emission (orphan path)",
        );
    }

    let mut guard = ClaimGuard::new(alloc.clone(), live);
    if guard.try_begin_ending() {
        let event = ExitEvent {
            alloc,
            kind,
            // Distinguishing an operator-initiated stop from a natural
            // guest exit needs a signal `VmDriver::stop` does not yet
            // thread to this watcher, and no consumer reads this field
            // until the exit-observer wiring lands (a later,
            // DriverRegistry-dependent step) — `false` is the honest
            // value for the surface this step ships, not a
            // misclassification of anything this step's ACs assert on.
            intentional_stop: false,
            stderr_tail: None,
            oom,
        };
        let _ = exit_tx.send(event).await;
    }
    // Else: the entry was no longer Held (nothing in this step's scope
    // makes that reachable in practice — no caller yet drives
    // transitions 5/6 — but the guard's Drop still covers it correctly
    // per transition 4 if it ever is).
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use overdrive_sim::adapters::cgroup_accounting::SimCgroupAccounting;
    use tokio::net::UnixStream;

    use super::*;

    /// The Running-gate ORPHAN path (greptile PR #268 P1). The happy
    /// path — the action shim firing `release_for_exit_emission` after
    /// `obs.write(Running)` commits — is proven by
    /// `tests/acceptance/vm_driver_stop_totality.rs::
    /// exit_event_is_gated_until_running_confirmed_release`. This pins its
    /// COMPLEMENT: the branch that STRANDS a watcher if it is wrong.
    ///
    /// When `obs.write(Running)` FAILS, `release_for_exit_emission` is
    /// skipped and the gate is never fired. The watcher must not block on
    /// `gate_receiver.await` forever — `VmDriver::stop` (its
    /// `Live -> EndingInFlight` replace) and `release_supervision` (its
    /// remove) both DROP the stashed `LiveVm.gate_sender`, and the drop
    /// MUST resolve the await to `Err(RecvError)` so the watcher proceeds
    /// (the `Driver::start` § "Sender drop (orphan path)" contract).
    ///
    /// This test drops the gate sender WITHOUT firing it and asserts the
    /// watcher completes AND emits — with a `Held` (`Starting`) entry left
    /// in the map so `try_begin_ending` fires the post-release emit path.
    /// A watcher stranded on the gate would never reach the send, so the
    /// `timeout` elapsing IS the strand detector. Black-box coverage is
    /// impossible here: after `stop`/`release_supervision` the entry is no
    /// longer `Held`, so the released watcher emits nothing and "released"
    /// is indistinguishable from "stranded" through the public surface.
    #[tokio::test]
    async fn dropped_gate_sender_releases_watcher_without_stranding() {
        let alloc = AllocationId::new("orphan-gate").expect("valid alloc id");

        // `Starting` is the trivial `Held` variant — the released
        // watcher's `try_begin_ending` sees it and emits (transition 3),
        // giving a positive observable without any `LiveVm` scaffolding.
        let live: Arc<LiveMap> =
            Arc::new(Mutex::new(BTreeMap::from([(alloc.clone(), VmSupervision::Starting)])));

        // Closed beacon connection -> immediate EOF, so `drain_guest_report`
        // resolves on its biased read arm with no guest report (`None`),
        // which then routes through the cgroup OOM read.
        let (near, far) = UnixStream::pair().expect("unix socketpair");
        drop(far);
        let (read_half, _write_half) = near.into_split();
        let reader = BufReader::new(read_half);

        // A `VmExitWatch` the biased read arm never consults (EOF wins
        // first), but the watcher requires one; the unused sender half
        // drops at the end of this statement.
        let exit = VmExitWatch::new(oneshot::channel::<VmmExit>().1);

        let (exit_tx, mut exit_rx) = mpsc::channel::<ExitEvent>(1);
        let cgroup_accounting: Arc<dyn CgroupAccounting> = Arc::new(SimCgroupAccounting::new());

        // Orphan: mint the gate, then DROP the sender WITHOUT firing it —
        // exactly what `stop` (replace) and `release_supervision` (remove)
        // do to the stashed `LiveVm.gate_sender`.
        let (gate_sender, gate_receiver) = oneshot::channel::<()>();
        drop(gate_sender);

        let event = tokio::time::timeout(Duration::from_secs(5), async {
            run_exit_watcher(
                alloc.clone(),
                exit,
                reader,
                Arc::clone(&live),
                exit_tx,
                cgroup_accounting,
                PathBuf::from("/sys/fs/cgroup"),
                CgroupPath::for_alloc(&alloc),
                0,
                gate_receiver,
            )
            .await;
            exit_rx.recv().await
        })
        .await
        .expect("dropped gate sender must release the watcher; it stranded on the gate")
        .expect("the released watcher emits its ExitEvent (entry still Held)");

        assert_eq!(event.alloc, alloc, "the emitted event is this allocation's");
        // EOF beacon (no guest EXIT report) + no VMM signal -> an
        // unreported crash with neither an exit code nor a signal.
        assert!(
            matches!(event.kind, ExitKind::Crashed { exit_code: None, signal: None }),
            "EOF beacon + no VMM signal classifies as an unreported crash; got {:?}",
            event.kind
        );
    }
}
