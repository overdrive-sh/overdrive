//! [`Vmm`] — the hypervisor-process port, and the values [`Vmm::create`]
//! races and returns (ADR-0082 §§D1, D3).
//!
//! Four methods; Cloud Hypervisor is the only implementor in scope.
//! Production wires `overdrive_host::CloudHypervisorVmm` (a real process
//! spawn); simulation wires `overdrive_sim::SimVmm` (an
//! injectable-outcome double). `overdrive_worker::VmDriver`
//! (`crate::traits::driver::Driver` over `Arc<dyn Vmm>`) is the one
//! production consumer — its `start`'s three-way race (guest beacon vs.
//! VMM exit vs. boot deadline) and its `stop`'s
//! guest-shutdown-then-terminate split are `VmDriver`'s, not this
//! port's (ADR-0082 §§D3-D4).
//!
//! **No implementor lands in this step** (`CloudHypervisorVmm` /
//! `SimVmm` are step 01-06); the trait compiles with zero implementors,
//! which is expected.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::oneshot;

use crate::vm::config::VmConfig;

/// Result alias for [`Vmm`]'s fallible methods.
pub type Result<T, E = VmmError> = std::result::Result<T, E>;

/// The hypervisor-process port (ADR-0082 §D1). Every method's contract
/// below is pinned by ADR-0082 §D6 ("the trait's contract is
/// behaviour") and enforced by a two-adapter equivalence test
/// (`crates/overdrive-host/tests/integration/vmm_equivalence.rs`,
/// gated behind `integration-tests`) once both adapters land.
///
/// # Scope
///
/// This port owns the hypervisor **process** half only: spawning it,
/// awaiting its own exit, and killing it. It does NOT own the guest's
/// graceful-shutdown request — that rides the vsock beacon connection
/// `VmDriver` holds directly (ADR-0082 §D4), because [`VmControl`]
/// carries no handle to that connection and no implementor of this port
/// could honestly answer a "ask the guest to power down" method.
#[async_trait]
pub trait Vmm: Send + Sync + 'static {
    /// Stable adapter discriminator for structured logs and
    /// probe-refusal events.
    ///
    /// # Contract
    /// - Returns a `&'static str` compile-time constant; no runtime
    ///   formatting.
    /// - Stable across versions — operators grep on this string in
    ///   startup logs and structured events.
    /// - `"cloud-hypervisor"` for the production adapter, `"sim"` for
    ///   the DST binding.
    fn kind(&self) -> &'static str;

    /// Earned-Trust startup probe (ADR-0082 §D5 — the sixth trait
    /// instance of the pattern; see
    /// [`crate::traits::cgroup_fs::CgroupFs::probe`] for the canonical
    /// shape this copies). Composition-root invariant: **wire → probe
    /// → use**.
    ///
    /// # Preconditions
    /// - The adapter is constructed; no other method has been called
    ///   yet.
    ///
    /// # Postconditions on Ok
    /// - The adapter has empirically demonstrated it can honor this
    ///   trait's contract against the real substrate — reflink support
    ///   on the image directory, a `--landlock`-capable
    ///   `cloud-hypervisor` binary, an active host Landlock LSM,
    ///   `/dev/kvm` reachable under the confined identity, and a
    ///   creatable/bindable run-directory root (ADR-0082 §D5's five
    ///   fault-injection scenarios).
    /// - Any probe-scoped scratch artifacts have been removed.
    ///
    /// # Edge cases
    /// - Called twice: idempotent, leaves no probe-scoped residue
    ///   (mirrors `CgroupFs::probe`'s stated postcondition).
    ///
    /// # Errors
    /// Returns [`VmmProbeError`] naming the specific substrate lie the
    /// probe caught. The composition root emits `health.startup.refused`
    /// with the structured cause and the process refuses to start.
    async fn probe(&self) -> std::result::Result<(), VmmProbeError>;

    /// Stage this VM's per-launch rootfs clone and spawn ONE confined
    /// hypervisor process for it.
    ///
    /// # Preconditions
    /// - `config` is a fully-constructed [`VmConfig`] — every substrate
    ///   lie ADR-0082 §D2 discourages (image type, memory ceiling,
    ///   kernel format, seccomp mode) is already resolved by `config`'s
    ///   own construction; `create` does not re-validate them.
    ///
    /// # Postconditions on Ok
    /// - Exactly one hypervisor process is running, confined per
    ///   `config.confinement`, with its rootfs clone staged at
    ///   `config.rootfs.clone_dest()`.
    /// - The returned [`VmProcess`] carries a live [`VmControl`] (the
    ///   process pid and its `--api-socket` path) and a
    ///   [`VmExitWatch`] that resolves exactly once, when the
    ///   hypervisor process itself exits.
    ///
    /// # Edge cases
    /// - `config.rootfs.clone_dest()` already exists (a crashed prior
    ///   launch left it behind): the adapter **replaces it**. The clone
    ///   is per-launch and carries no state a restart may inherit.
    /// - The process spawn fails **after** the rootfs clone succeeded:
    ///   the adapter removes the clone before returning `Err` — no
    ///   partial artifact escapes a failed `create` (the boot-time reap
    ///   sweeps only clones an adapter never got the chance to remove —
    ///   ADR-0082 §A6).
    /// - `config.netns` is `None`: the hypervisor process runs in the
    ///   host network namespace. Not an error — Job-kind VMs need no
    ///   tap device, and an mTLS-uncomposed boot never supplies a
    ///   netns.
    ///
    /// # Errors
    /// Returns [`VmmError`] when the rootfs clone cannot be staged or
    /// the hypervisor process cannot be spawned. On any `Err`, no
    /// process is left running and no clone is left on disk (see the
    /// spawn-after-clone edge case above).
    async fn create(&self, config: &VmConfig) -> Result<VmProcess>;

    /// Terminate the hypervisor **process**: await its exit for
    /// `grace`, then kill it unconditionally.
    ///
    /// **This method does NOT ask the guest to power down.** The guest
    /// shutdown request travels on the beacon connection, which
    /// `VmDriver` owns and writes to directly, before calling this
    /// method (ADR-0082 §D4). By the time `terminate` is called, the
    /// guest has already been asked to shut down (or there was no
    /// connection yet to ask on) — this method's job is bounding how
    /// long the process is given to comply.
    ///
    /// # Preconditions
    /// - `control` was returned by a prior [`Vmm::create`] call on the
    ///   same adapter instance.
    ///
    /// # Postconditions on Ok
    /// - The hypervisor process named by `control.pid` is no longer
    ///   running.
    ///
    /// # Edge cases
    /// - The VMM is already gone (process exited, or was killed by
    ///   something else, before this call): `Ok(VmTermination::Killed)`.
    ///   Idempotent — a second `terminate` against the same `control`
    ///   is not an error.
    /// - `grace == Duration::ZERO`: kill immediately, with no await.
    ///
    /// # Errors
    /// Returns [`VmmError`] only when the substrate refuses the
    /// terminate/kill operation itself (e.g. a permission failure
    /// signalling the process) — never for "the process was already
    /// dead," which is the `Ok(VmTermination::Killed)` case above.
    async fn terminate(&self, control: &VmControl, grace: Duration) -> Result<VmTermination>;
}

// -----------------------------------------------------------------------
// D3 — create returns a process, an exit watch, and a control handle
// -----------------------------------------------------------------------

/// What [`Vmm::create`] hands back on success: the live process handle
/// plus the adapter-agnostic watch on its own ending.
#[derive(Debug)]
pub struct VmProcess {
    pub control: VmControl,
    pub exit: VmExitWatch,
}

/// The hypervisor process's identity and its control-plane socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmControl {
    pub pid: u32,
    pub api_socket: PathBuf,
}

/// Adapter-agnostic await on the hypervisor process's own ending. Wraps
/// a [`tokio::sync::oneshot::Receiver`]; the adapter's watcher task
/// fills it (`SimVmm` fills it from an injected script).
///
/// **`recv` takes `&mut self`, NOT `self`.** The receiver must SURVIVE
/// `VmDriver::start`'s three-way race (ADR-0082 §D3): a by-value `recv`
/// would move the whole watch into the `select!` arm's future, so when
/// a different arm won, that future — and the receiver inside it —
/// would be dropped, the adapter's `send` would fail, and the VMM's
/// exit would never be observed; the allocation would never leave
/// `Running`. Borrowing lets the SAME `VmExitWatch` be raced in
/// `select!` and then, on the winning `Ok` path, moved intact into the
/// long-lived per-allocation exit watcher.
#[derive(Debug)]
pub struct VmExitWatch(oneshot::Receiver<VmmExit>);

impl VmExitWatch {
    /// Wrap the receiving half of the channel an adapter's watcher task
    /// sends the process's [`VmmExit`] over. The constructor every
    /// implementor of [`Vmm::create`] needs — this trait module lands
    /// (step 01-01) with zero implementors, so no constructor was needed
    /// until the first one (`CloudHypervisorVmm` / `SimVmm`, step 01-06).
    #[must_use]
    pub const fn new(receiver: oneshot::Receiver<VmmExit>) -> Self {
        Self(receiver)
    }

    /// Await the hypervisor process's exit. The adapter's watcher task
    /// sends exactly one [`VmmExit`] over the wrapped channel when the
    /// process exits.
    ///
    /// # Returns
    /// - `Some(exit)` — the process exited; `exit` carries its code,
    ///   signal, and stderr tail.
    /// - `None` — the sending half was dropped without sending (the
    ///   adapter, or its watcher task, was torn down before observing
    ///   an exit).
    pub async fn recv(&mut self) -> Option<VmmExit> {
        (&mut self.0).await.ok()
    }
}

/// The HYPERVISOR's ending — never the workload's (ADR-0082 §D3). The
/// guest's own exit status travels on the beacon (`EXIT <status>`,
/// ADR-0082 §D7) and is `VmDriver`'s concern, not this type's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmmExit {
    pub exit_code: Option<i32>,
    pub signal: Option<u8>,
    /// Reuses [`crate::traits::driver::STDERR_TAIL_LINES`]'s shape —
    /// the last N lines of the hypervisor process's own stderr, joined
    /// by `\n`.
    pub stderr_tail: Option<String>,
}

/// The outcome of terminating the hypervisor PROCESS (ADR-0082 §D3).
/// Deliberately carries no guest-shaped variant: classifying the
/// *workload's* ending is `VmDriver`'s job, off the beacon, not this
/// port's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VmTermination {
    /// The process exited on its own within `grace`.
    ExitedWithinGrace(VmmExit),
    /// The process did not exit within `grace` and was killed.
    Killed,
}

// -----------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------

/// Failure surface for [`Vmm::create`] / [`Vmm::terminate`]. No
/// implementor lands in this step; the variant set below is the
/// minimal shape the trait signature needs and is expected to grow
/// when `CloudHypervisorVmm` (step 01-06) discovers concrete substrate
/// failure modes.
#[derive(Debug, thiserror::Error)]
pub enum VmmError {
    /// The hypervisor process could not be spawned, or `create`'s
    /// staging steps (rootfs clone, socket bind, run-directory setup)
    /// failed before the process existed. `detail` carries the
    /// substrate's own diagnostic text — never reinterpreted (the same
    /// discipline ADR-0082 §D2.4 states for `KernelFormatError`).
    #[error("VMM create failed: {detail}")]
    Create { detail: String },

    /// A filesystem or process-control operation failed at a point
    /// this trait's methods do not further distinguish. Wraps the
    /// originating I/O error without reinterpretation.
    #[error("VMM I/O: {0}")]
    Io(#[from] std::io::Error),
}

impl VmmError {
    #[must_use]
    pub fn create(detail: impl Into<String>) -> Self {
        Self::Create { detail: detail.into() }
    }
}

/// Failure surface for [`Vmm::probe`] — ADR-0082 §D5's five
/// fault-injection scenarios, one variant each.
#[derive(Debug, thiserror::Error)]
pub enum VmmProbeError {
    /// The VM image directory cannot `FICLONE` (`EOPNOTSUPP` on ext4,
    /// `EXDEV` across filesystems) — lie 1: `cp --reflink=auto`
    /// degrades to a full copy with no error.
    #[error("reflink unsupported on {dir} ({fstype}): {source}")]
    ReflinkUnsupported { dir: PathBuf, fstype: String, source: std::io::Error },

    /// The installed `cloud-hypervisor` has no `--landlock` flag — lie
    /// 4: a CH built without it silently runs unconfined.
    #[error("cloud-hypervisor {binary} ({version}) has no --landlock flag")]
    LandlockFlagAbsent { binary: PathBuf, version: String },

    /// The host kernel does not expose the Landlock LSM
    /// (`/sys/kernel/security/lsm`) — lie 4, host half.
    #[error("host kernel exposes no Landlock LSM (active LSMs: {lsms})")]
    LandlockLsmAbsent { lsms: String },

    /// `/dev/kvm` is not openable under the target identity — lie 7:
    /// `0660 root:kvm`; a uid-dropped VMM reaches it only via group
    /// membership.
    #[error("/dev/kvm unreachable for uid={uid} gid={gid} mode={mode:o}: {source}")]
    KvmUnreachable { uid: u32, gid: u32, mode: u32, source: std::io::Error },

    /// The run-directory root is absent or unwritable — SD-2: the run
    /// directory must be creatable and bindable, since the vsock and
    /// beacon sockets both land in it.
    #[error("VM run-directory root {root} is unusable: {source}")]
    RunDirUnusable { root: PathBuf, source: std::io::Error },
}

impl VmmProbeError {
    #[must_use]
    pub fn reflink_unsupported(
        dir: PathBuf,
        fstype: impl Into<String>,
        source: std::io::Error,
    ) -> Self {
        Self::ReflinkUnsupported { dir, fstype: fstype.into(), source }
    }

    #[must_use]
    pub fn landlock_flag_absent(binary: PathBuf, version: impl Into<String>) -> Self {
        Self::LandlockFlagAbsent { binary, version: version.into() }
    }

    #[must_use]
    pub fn landlock_lsm_absent(lsms: impl Into<String>) -> Self {
        Self::LandlockLsmAbsent { lsms: lsms.into() }
    }

    #[must_use]
    pub const fn kvm_unreachable(uid: u32, gid: u32, mode: u32, source: std::io::Error) -> Self {
        Self::KvmUnreachable { uid, gid, mode, source }
    }

    #[must_use]
    pub const fn run_dir_unusable(root: PathBuf, source: std::io::Error) -> Self {
        Self::RunDirUnusable { root, source }
    }
}
