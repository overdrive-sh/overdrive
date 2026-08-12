//! `VmConfig`'s pure value family — the anti-corruption layer against
//! Cloud Hypervisor's silent substrate lies (ADR-0082 §§D2.1-D2.5).
//!
//! Each type below makes one lie structurally discouraged: the field a
//! crafter (or a future adapter) could get wrong does not exist, and the
//! correct value is computed from an input that cannot be omitted. Per
//! ADR-0082 §D2's own honesty correction, the enforcement here is
//! **private fields + one rendering site + a `dst-lint` clause** (landing
//! alongside this code per the ADR), not a type-level impossibility.
//!
//! # What step 01-01 lands
//!
//! - [`DiskAttachment`] (§D2.1) — `image_type=raw` unconditional (lie:
//!   CH's auto-detect disables sector-0 writes on bare-filesystem images).
//! - [`MemoryPlan`] + [`reserve_bytes`] (§D2.3) — `guest_bytes !=
//!   cgroup_max_bytes` by construction (lie: `memory.max == guest RAM` is
//!   a cgroup OOM by construction). `reserve_bytes` ships as a `todo!()`
//!   until step 01-05 measures the real bound via a real boot.
//! - [`KernelImage`] (§D2.4) — validates the boot-image magic for the
//!   host architecture before Cloud Hypervisor ever sees the file (lie:
//!   an unloadable `--kernel` is silently reinterpreted as UEFI firmware
//!   and reported as a misleading 3 MiB size cap).
//! - [`VmConfinement`] (§D2.5) — `seccomp_arg()` always renders the
//!   literal `"true"`; CH's `log` and `false` modes have no
//!   representation anywhere in this workspace.
//!
//! # What step 01-01 PART 2 lands
//!
//! PART 1 (commit `2636ba1c`) resolved the first two of three blockers
//! on `VmConfig`'s remaining field types (`cgroup_scope: CgroupPath`
//! relocated into [`crate::cgroup`]; `netns: Option<NetnsName>` via the
//! new [`crate::id::NetnsName`] newtype). PART 2 closes the third and
//! lands everything it unblocks:
//!
//! - [`RootfsPlan`] (§D2, gap 3) — the rootfs staging plan: the
//!   operator's read-only master image, its size (captured at
//!   construction so [`VmConfig::rlimit_fsize`] stays pure), and the
//!   per-launch clone destination (derived on the master's own
//!   filesystem — FICLONE is intra-filesystem — with a filename
//!   carrying the allocation id for reap attribution).
//! - [`KernelCmdline`] (§D2, gap 4) — the platform-owned guest kernel
//!   command line. NOT operator surface: `[vm]` carries
//!   kernel/rootfs/command/args, never a cmdline.
//! - [`VsockPort`] — the beacon socket's vsock port number, an internal
//!   newtype (GH #42).
//! - [`VmRunDir`] (§D2.2) — the per-allocation run directory; owns
//!   every path inside it (SD-2 exclusivity).
//! - [`VmConfig`] (§D2) — the outer aggregate. Every field now names a
//!   concrete `overdrive-core`-resident type, so this struct — and
//!   therefore `Vmm::create(&VmConfig)` (§D1,
//!   `crate::traits::vmm::Vmm`) — compiles.
//!
//! # What remains deferred, and why
//!
//! `VmRunDir::landlock_grant() -> LandlockRule` and
//! `VmConfig::landlock_rules()` are deferred to Slice 03 (US-VM-7) per
//! ADR-0082's 2026-08-12 amendment (gap 5) — `LandlockRule` has no shape
//! anywhere, and the slice-01 doc already assigns the additive
//! confinement items (Landlock, uid/gid drop, rlimits) to US-VM-7. This
//! is a scoping decision, not a blocker: Slice 01 runs Cloud Hypervisor
//! without `--landlock`/`--landlock-rules`, so no run-directory grant is
//! needed until Landlock confinement is opted into in Slice 03.
//!
//! [`reserve_bytes`] (§D2.3) still ships as a RED scaffold until step
//! 01-05 measures the real bound via a real boot — unchanged by this
//! step.

use std::fmt;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};

use crate::AllocationId;
use crate::cgroup::CgroupPath;
use crate::id::NetnsName;

// -----------------------------------------------------------------------
// D2.4 — KernelImage: validates before Cloud Hypervisor ever sees the file
// -----------------------------------------------------------------------

/// The host CPU architecture Cloud Hypervisor is running on. Determines
/// which kernel boot-image format [`KernelImage::validate`] accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostArch {
    X86_64,
    Aarch64,
}

impl fmt::Display for HostArch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        })
    }
}

/// The number of leading bytes a caller must read from a candidate kernel
/// image before calling [`KernelImage::validate`]. Sized to the deepest
/// offset any recognised format checks — the x86_64 `bzImage` `HdrS`
/// signature at `0x202` (4 bytes, ending at `0x206`).
pub const KERNEL_MAGIC_WINDOW: usize = 0x206;

const BZIMAGE_HDRS_OFFSET: usize = 0x202;
const BZIMAGE_HDRS_MAGIC: [u8; 4] = *b"HdrS";
const ELF_MAGIC_OFFSET: usize = 0;
const ELF_MAGIC: [u8; 4] = *b"\x7fELF";
const AARCH64_IMAGE_MAGIC_OFFSET: usize = 0x38;
const AARCH64_IMAGE_MAGIC: [u8; 4] = *b"ARM\x64";

/// Bounds-checked magic-byte comparison. Never panics on a short buffer —
/// `header` ultimately originates from an external file read (Functional
/// Core / Imperative Shell: the caller does the `read`), so a truncated
/// or garbage header is an expected input, not an invariant violation.
/// Per `.claude/rules/development.md` § "Safe byte-slice access".
fn header_matches(header: &[u8], offset: usize, magic: [u8; 4]) -> bool {
    header.get(offset..offset + magic.len()) == Some(magic.as_slice())
}

/// The format problem [`KernelImage::validate`] rejected, named before
/// Cloud Hypervisor ever sees the file — closing the lie where an
/// unloadable `--kernel` is silently reinterpreted as UEFI firmware and
/// reported as a misleading size cap (ADR-0082 §D2.4, correction C-7).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("kernel image at {path:?} is not a valid {arch} boot image: {detail}")]
pub struct KernelFormatError {
    pub path: PathBuf,
    pub arch: HostArch,
    pub detail: String,
}

/// A kernel image this hypervisor can actually load. Constructed only by
/// [`validate`](Self::validate)ing the image magic for the host
/// architecture — never handed to Cloud Hypervisor unvalidated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelImage {
    path: PathBuf,
}

impl KernelImage {
    /// Validate a candidate kernel image's boot-format magic for `arch`.
    ///
    /// **PURE** (Functional Core / Imperative Shell): the caller reads
    /// the first [`KERNEL_MAGIC_WINDOW`] bytes of the candidate file;
    /// this function performs no I/O of its own.
    ///
    /// - `x86_64` accepts a `bzImage` (`HdrS` at `0x202`) or a
    ///   PVH-enabled `vmlinux` ELF (`\x7fELF` at `0x0`). A distro
    ///   `vmlinuz` loads directly.
    /// - `aarch64` accepts the raw PE `Image` (`ARM\x64` at `0x38`). A
    ///   distro `vmlinuz` is a UKI wrapper and does **not** match —
    ///   unwrapping it is BYO-artifact's job, not the platform's.
    ///
    /// # Errors
    ///
    /// Returns [`KernelFormatError`] when `header` does not carry a
    /// recognised magic for `arch` at the expected offset — including
    /// when `header` is shorter than the offset requires (no panic on a
    /// truncated buffer; a short header simply fails to match).
    pub fn validate(
        path: PathBuf,
        arch: HostArch,
        header: &[u8],
    ) -> Result<Self, KernelFormatError> {
        let recognised = match arch {
            HostArch::X86_64 => {
                header_matches(header, BZIMAGE_HDRS_OFFSET, BZIMAGE_HDRS_MAGIC)
                    || header_matches(header, ELF_MAGIC_OFFSET, ELF_MAGIC)
            }
            HostArch::Aarch64 => {
                header_matches(header, AARCH64_IMAGE_MAGIC_OFFSET, AARCH64_IMAGE_MAGIC)
            }
        };

        if recognised {
            return Ok(Self { path });
        }

        Err(KernelFormatError {
            path,
            arch,
            detail: format!(
                "no recognised {arch} boot-image magic found in the first {} bytes",
                header.len().min(KERNEL_MAGIC_WINDOW)
            ),
        })
    }
}

// -----------------------------------------------------------------------
// D2.1 — DiskAttachment: the value renders its own `--disk` argument
// -----------------------------------------------------------------------

/// One `--disk` attachment. `image_type=raw` is emitted unconditionally
/// and is NOT a field: Cloud Hypervisor's auto-detect path — which
/// disables sector-0 writes and faults our bare-filesystem images — has
/// no representation anywhere in this workspace (ADR-0082 §D2.1,
/// correction C-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskAttachment {
    path: PathBuf,
    readonly: bool,
}

impl DiskAttachment {
    #[must_use]
    pub const fn new(path: PathBuf, readonly: bool) -> Self {
        Self { path, readonly }
    }

    /// The complete `--disk` argument value. This is the anti-corruption
    /// translation itself — exactly one site in the workspace can ever
    /// emit a `--disk` argument, it is pure, and it is a mutation target.
    #[must_use]
    pub fn to_disk_arg(&self) -> String {
        let mut arg = format!("path={},image_type=raw", self.path.display());
        if self.readonly {
            arg.push_str(",readonly=on");
        }
        arg
    }
}

// -----------------------------------------------------------------------
// D2.3 — MemoryPlan: `memory.max` cannot equal guest RAM
// -----------------------------------------------------------------------

/// Guest RAM and the allocation's cgroup ceiling, together, derived from
/// ONE operator figure. `guest_bytes == cgroup_max_bytes` is not
/// representable: there is no constructor that takes two numbers
/// (ADR-0082 §D2.3, correction C-3/SD-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryPlan {
    guest_bytes: u64,
    cgroup_max_bytes: u64,
}

impl MemoryPlan {
    /// The ONLY constructor. `declared` is `resources.memory_bytes` — the
    /// figure the operator wrote, and the RAM the guest observes. The
    /// ceiling adds the result of calling [`reserve_bytes`] on `declared`.
    #[must_use]
    pub fn derive(declared: u64) -> Self {
        let cgroup_max_bytes = declared.saturating_add(reserve_bytes(declared));
        Self { guest_bytes: declared, cgroup_max_bytes }
    }

    #[must_use]
    pub const fn guest_bytes(&self) -> u64 {
        self.guest_bytes
    }

    #[must_use]
    pub const fn cgroup_max_bytes(&self) -> u64 {
        self.cgroup_max_bytes
    }
}

/// The reserve policy. A function evaluated fresh at every
/// [`MemoryPlan::derive`] call, never a persisted field — per
/// `.claude/rules/development.md` § "Persist inputs, not derived state",
/// a persisted reserve would be a stale cache of this policy.
///
/// **Ships as a RED scaffold.** The reserve is a hard DELIVER
/// dependency, not a guess (ADR-0082 §D2.3 Consequences, intake
/// precedent warning #7's "magic version floor" failure with different
/// units): RSS structurally cannot supply it (host page tables for the
/// guest mapping are charged to the allocation's cgroup scope and are
/// invisible to RSS). Lands GREEN in step 01-05, measured via
/// `memory.current` / `memory.stat` against a real boot.
///
/// # Panics
///
/// Always, until step 01-05 gives this function a real body.
#[expect(
    clippy::todo,
    reason = "RED scaffold; lands GREEN in step 01-05 -- measured via \
              memory.current/memory.stat against a real boot, ADR-0082 §D2.3"
)]
#[must_use]
pub fn reserve_bytes(guest_bytes: u64) -> u64 {
    todo!(
        "RED scaffold: reserve_bytes({guest_bytes}) measured in DELIVER via \
         memory.current / memory.stat against a real boot, per SD-4 -- \
         ADR-0082 §D2.3. NEVER RSS (host page tables for the guest mapping \
         are charged to the cgroup scope and invisible to RSS)."
    );
}

// -----------------------------------------------------------------------
// D2.5 — VmConfinement: three variants, one reachable constructor
// -----------------------------------------------------------------------

/// A POSIX numeric group id. Plain value type — never a human-typed or
/// wire-parsed identifier in this feature, so it carries only the
/// construction/read surface actually used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gid(u32);

impl Gid {
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Gid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The confined process identity Cloud Hypervisor is spawned under.
/// Settled by spike probe P5 and not re-opened by this step: an
/// unprivileged uid in the `kvm` group against `0660 root:kvm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmmIdentity {
    pub uid: u32,
    pub gid: Gid,
    pub supplementary: Vec<Gid>,
}

/// `identity` + `rlimit_nofile`, plus the seccomp mode Cloud Hypervisor
/// is launched under. `seccomp_arg()` is the complete `--seccomp`
/// argument value — one pure rendering site, and that site is the
/// mutation target: CH's `log` and `false` modes have no representation
/// anywhere in this workspace (ADR-0082 §D2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmConfinement {
    identity: VmmIdentity,
    rlimit_nofile: u64,
}

impl VmConfinement {
    #[must_use]
    pub const fn confined(identity: VmmIdentity, rlimit_nofile: u64) -> Self {
        Self { identity, rlimit_nofile }
    }

    /// The complete `--seccomp` argument value. Same lever as
    /// [`DiskAttachment::to_disk_arg`] — one pure rendering site, and
    /// that site is the mutation target a mutation flipping the rendered
    /// literal to `"false"` or `"log"` must be killed by.
    ///
    /// `&self` is deliberately unused: ADR-0082 §D2.5 pins this exact
    /// instance-method signature (corrected at review iteration 1, after
    /// the first draft kept a `SeccompMode` enum specifically to avoid
    /// this shape) so that CH's `log` and `false` modes have no
    /// representation anywhere in this workspace, on this type or any
    /// other. Do not "fix" this into an associated function.
    #[expect(
        clippy::unused_self,
        reason = "ADR-0082 §D2.5 pins &self on this method precisely so no \
                  seccomp-mode field can ever exist to read from"
    )]
    #[must_use]
    pub const fn seccomp_arg(&self) -> &'static str {
        "true"
    }
}

// -----------------------------------------------------------------------
// D2 gap 3 — RootfsPlan: the FICLONE source, its size, and the
// per-launch clone destination
// -----------------------------------------------------------------------

/// The rootfs staging plan: the operator's read-only master image, its
/// size (captured at construction so [`VmConfig::rlimit_fsize`] stays
/// pure), and the per-launch clone destination (ADR-0082 §D2, gap 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootfsPlan {
    master: PathBuf,
    master_bytes: u64,
    clone_dest: PathBuf,
}

impl RootfsPlan {
    /// Build the plan for one allocation. `master` is the operator's
    /// BYO rootfs artifact; `master_bytes` is its size, captured HERE
    /// by the caller (the imperative shell does the `stat`) — the same
    /// Functional-Core / Imperative-Shell split [`KernelImage::validate`]
    /// applies.
    ///
    /// The clone destination is derived on the **master's own
    /// filesystem** (FICLONE is intra-filesystem; staging into `/run`
    /// fails `EXDEV`), with a filename **carrying `alloc`** so a
    /// reboot-orphaned clone is attributable (SD-1 / SD-2; the reap
    /// keys off it — ADR-0083 §D7).
    #[must_use]
    pub fn for_alloc(master: PathBuf, master_bytes: u64, alloc: &AllocationId) -> Self {
        let dir = master.parent().unwrap_or_else(|| Path::new(""));
        let clone_dest = dir.join(format!(".overdrive-vm-rootfs-{alloc}.img"));
        Self { master, master_bytes, clone_dest }
    }

    /// The FICLONE source — the operator's read-only master image.
    #[must_use]
    pub fn master(&self) -> &Path {
        &self.master
    }

    /// The master image's size, captured at construction. Feeds
    /// [`VmConfig::rlimit_fsize`].
    #[must_use]
    pub const fn master_bytes(&self) -> u64 {
        self.master_bytes
    }

    /// The FICLONE target — becomes the virtio-blk disk source.
    #[must_use]
    pub fn clone_dest(&self) -> &Path {
        &self.clone_dest
    }
}

// -----------------------------------------------------------------------
// D2 gap 4 — KernelCmdline: platform-derived, never operator surface
// -----------------------------------------------------------------------

/// The guest kernel command line. PLATFORM-DERIVED — there is NO
/// operator surface for it (`[vm]` carries kernel/rootfs/command/args,
/// never a cmdline). Constructed by the platform (the VM driver, the
/// imperative shell) from fixed boot parameters — the operator cannot
/// inject kernel parameters (ADR-0082 §D2, gap 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelCmdline(String);

impl KernelCmdline {
    /// The platform's kernel command line for `arch`. Called by the VM
    /// driver, NEVER from operator input. Renders the fixed platform
    /// boot line — the arch-appropriate `console=`, `panic=1`, and the
    /// virtio-blk `root=` device for the ext4 rootfs — so the guest
    /// kernel boots, mounts the rootfs, and reaches `overdrive-init`.
    ///
    /// The exact token set is platform boot policy, provisional until
    /// validated against a real boot (step 01-08's walking skeleton) —
    /// the same provisional-constant discipline as [`reserve_bytes`].
    /// The *shape* ADR-0082 §D2 pins — a single platform-owned
    /// constructor with no operator input — is fixed now.
    #[must_use]
    pub fn platform_default(arch: HostArch) -> Self {
        let console = match arch {
            HostArch::X86_64 => "ttyS0",
            HostArch::Aarch64 => "ttyAMA0",
        };
        Self(format!("console={console} panic=1 root=/dev/vda rw"))
    }

    /// The complete `--cmdline` argument value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// -----------------------------------------------------------------------
// VsockPort — the beacon socket's vsock port number (GH #42)
// -----------------------------------------------------------------------

/// The vsock port the guest agent dials for the beacon connection
/// (ADR-0082 §D2.2). INTERNAL newtype: a platform-internal beacon port,
/// never operator-typed, never persisted — same completeness class as
/// [`crate::id::NetnsName`], so no `serde`, `rkyv`, or `FromStr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VsockPort(u32);

impl VsockPort {
    #[must_use]
    pub const fn new(port: u32) -> Self {
        Self(port)
    }

    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Display for VsockPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// -----------------------------------------------------------------------
// D2.2 — VmRunDir: owns every path inside the per-allocation run
// directory (SD-2 exclusivity)
// -----------------------------------------------------------------------

/// The per-allocation run directory (SD-2 — tmpfs, one per allocation,
/// holding NOTHING else). This type owns every path inside it, which is
/// why SD-2's exclusivity is a structural property rather than a
/// convention (ADR-0082 §D2.2).
///
/// `landlock_grant() -> LandlockRule` is deferred to Slice 03 (US-VM-7,
/// gap 5) — Slice 01 launches Cloud Hypervisor without
/// `--landlock`/`--landlock-rules`, so no directory grant is minted this
/// slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmRunDir(PathBuf);

impl VmRunDir {
    /// The per-allocation run directory under `root` (the tmpfs mount
    /// point).
    #[must_use]
    pub fn for_alloc(root: &Path, alloc: &AllocationId) -> Self {
        Self(root.join(alloc.as_str()))
    }

    /// The run directory itself.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// `<dir>/vsock` — Cloud Hypervisor binds this (the guest connects
    /// out to it).
    #[must_use]
    pub fn vsock_socket(&self) -> PathBuf {
        self.0.join("vsock")
    }

    /// `<dir>/vsock_<port>` — the DRIVER binds this; Cloud Hypervisor's
    /// guest→host vsock path connects out to `<vsock_socket>_<port>`
    /// (ADR-0082 §D2.2).
    #[must_use]
    pub fn beacon_socket(&self, port: VsockPort) -> PathBuf {
        self.0.join(format!("vsock_{}", port.as_u32()))
    }

    /// `<dir>/api` — Cloud Hypervisor's `--api-socket`.
    #[must_use]
    pub fn api_socket(&self) -> PathBuf {
        self.0.join("api")
    }

    /// `<dir>/console.log` — the guest's serial console capture.
    #[must_use]
    pub fn console_log(&self) -> PathBuf {
        self.0.join("console.log")
    }
}

// -----------------------------------------------------------------------
// D2 — VmConfig: the outer aggregate. Every field now names a concrete
// overdrive-core-resident type (ADR-0082 §D2, 2026-08-12 amendment).
// -----------------------------------------------------------------------

/// One VM launch's complete, validated configuration — the value
/// [`crate::traits::vmm::Vmm::create`] takes. Every field is either
/// already-validated (`kernel`, `memory`, `confinement`) or a pure
/// derivation from a mandatory input (`rootfs`, `cmdline`, `run_dir`,
/// `cgroup_scope`) — "one value, one call" (ADR-0082 §D2, rejecting the
/// reference implementation's stateful config-accumulating builder,
/// §A1).
///
/// Transient — passed by reference to [`crate::traits::vmm::Vmm::create`]
/// and not persisted; per `.claude/rules/development.md` § "Persist
/// inputs, not derived state" it carries no `serde`/`rkyv` derives (the
/// same discipline `AllocationSpec` follows for the same reason).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmConfig {
    pub alloc: AllocationId,
    pub kernel: KernelImage,
    pub rootfs: RootfsPlan,
    pub cmdline: KernelCmdline,
    pub memory: MemoryPlan,
    pub vcpus: NonZeroU8,
    pub run_dir: VmRunDir,
    pub confinement: VmConfinement,
    /// `Some` only when the action-shim provisioned a per-workload
    /// netns for this allocation; `None` runs the hypervisor process in
    /// the host netns (not an error — Job-kind VMs need no tap device).
    /// Sourced from the same [`NetnsName`] as
    /// [`crate::traits::driver::AllocationSpec::netns`].
    pub netns: Option<NetnsName>,
    pub cgroup_scope: CgroupPath,
}

impl VmConfig {
    /// `max(rootfs image size, guest RAM)` (lie 6 / C-6, ADR-0082 §D2).
    /// Encoded from Slice 01, BEFORE Slice 04 turns `--memory
    /// shared=on` on, because `shared=on` backs guest RAM with a memfd
    /// and a memfd is a *file* for `RLIMIT_FSIZE`.
    ///
    /// **PURE.** The rootfs size is `self.rootfs.master_bytes()`,
    /// captured at [`RootfsPlan`] construction by the caller — the same
    /// Functional-Core / Imperative-Shell split [`KernelImage::validate`]
    /// uses, where the caller does the `read` and the validator does
    /// not.
    #[must_use]
    pub const fn rlimit_fsize(&self) -> u64 {
        let rootfs_bytes = self.rootfs.master_bytes();
        let guest_bytes = self.memory.guest_bytes();
        if rootfs_bytes > guest_bytes { rootfs_bytes } else { guest_bytes }
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// Valid-label regex for `AllocationId` — lowercase alnum segments
    /// joined by hyphens, always starting/ending alnum (matches
    /// `id::validate_label`'s grammar).
    const ALLOC_SUFFIX_STRATEGY: &str = "[a-z0-9]{1,4}(-[a-z0-9]{1,4}){0,3}";

    // -------------------------------------------------------------------
    // RootfsPlan (ADR-0082 §D2, gap 3)
    // -------------------------------------------------------------------

    proptest! {
        /// `RootfsPlan::for_alloc` derives the clone destination on the
        /// master's OWN parent directory (FICLONE is intra-filesystem)
        /// with a filename carrying the allocation id (SD-1 / SD-2 reap
        /// attribution).
        #[test]
        fn rootfs_plan_clone_dest_sits_beside_master_and_carries_alloc(
            dir_name in "[a-z0-9]{1,8}",
            file_name in "[a-z0-9]{1,8}",
            alloc_suffix in ALLOC_SUFFIX_STRATEGY,
            master_bytes in any::<u64>(),
        ) {
            let alloc = AllocationId::new(&alloc_suffix).unwrap();
            let master = PathBuf::from(format!("/var/lib/overdrive/{dir_name}/{file_name}.img"));
            let plan = RootfsPlan::for_alloc(master.clone(), master_bytes, &alloc);

            prop_assert_eq!(plan.master(), master.as_path());
            prop_assert_eq!(plan.master_bytes(), master_bytes);
            prop_assert_eq!(plan.clone_dest().parent(), master.parent());

            let clone_name = plan
                .clone_dest()
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            prop_assert!(
                clone_name.contains(alloc.as_str()),
                "clone dest filename {clone_name:?} must carry the alloc id {alloc}",
            );
            prop_assert_ne!(plan.clone_dest(), master.as_path());
        }
    }

    // -------------------------------------------------------------------
    // KernelCmdline (ADR-0082 §D2, gap 4)
    // -------------------------------------------------------------------

    #[test]
    fn kernel_cmdline_platform_default_is_arch_correct_and_carries_panic_1() {
        let x86 = KernelCmdline::platform_default(HostArch::X86_64);
        assert!(x86.as_str().contains("panic=1"), "must set panic=1: {}", x86.as_str());
        assert!(
            x86.as_str().contains("console=ttyS0"),
            "x86_64 must set the serial console: {}",
            x86.as_str(),
        );
        assert!(
            x86.as_str().contains("root=/dev/vda"),
            "must set the virtio-blk root device: {}",
            x86.as_str(),
        );

        let aarch64 = KernelCmdline::platform_default(HostArch::Aarch64);
        assert!(aarch64.as_str().contains("panic=1"), "must set panic=1: {}", aarch64.as_str());
        assert!(
            aarch64.as_str().contains("console=ttyAMA0"),
            "aarch64 must set the PL011 console: {}",
            aarch64.as_str(),
        );
        assert!(
            aarch64.as_str().contains("root=/dev/vda"),
            "must set the virtio-blk root device: {}",
            aarch64.as_str(),
        );

        assert_ne!(
            x86.as_str(),
            aarch64.as_str(),
            "the two arches must render distinct console tokens",
        );
    }

    // -------------------------------------------------------------------
    // VsockPort (GH #42)
    // -------------------------------------------------------------------

    proptest! {
        /// `VsockPort::new` / `as_u32` round-trip for every `u32`, and
        /// `Display` renders the decimal form.
        #[test]
        fn vsock_port_roundtrips_and_displays_decimal(raw in any::<u32>()) {
            let port = VsockPort::new(raw);
            prop_assert_eq!(port.as_u32(), raw);
            prop_assert_eq!(port.to_string(), raw.to_string());
        }
    }

    // -------------------------------------------------------------------
    // VmRunDir (ADR-0082 §D2.2)
    // -------------------------------------------------------------------

    proptest! {
        /// Every socket/log path `VmRunDir` exposes is `<dir>/<fixed
        /// name>`, and `beacon_socket` embeds the port number.
        #[test]
        fn vm_run_dir_paths_are_dir_joined_names(
            root_name in "[a-z0-9]{1,8}",
            alloc_suffix in ALLOC_SUFFIX_STRATEGY,
            port in any::<u32>(),
        ) {
            let alloc = AllocationId::new(&alloc_suffix).unwrap();
            let root = PathBuf::from(format!("/run/overdrive/{root_name}"));
            let run_dir = VmRunDir::for_alloc(&root, &alloc);
            let dir = run_dir.path();

            prop_assert_eq!(run_dir.vsock_socket(), dir.join("vsock"));
            prop_assert_eq!(run_dir.api_socket(), dir.join("api"));
            prop_assert_eq!(run_dir.console_log(), dir.join("console.log"));

            let beacon = run_dir.beacon_socket(VsockPort::new(port));
            prop_assert_eq!(beacon.parent(), Some(dir));

            let beacon_name = beacon.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            let port_str = port.to_string();
            prop_assert!(
                beacon_name.contains(port_str.as_str()),
                "beacon socket name {beacon_name:?} must embed the port {port}",
            );
            prop_assert_ne!(run_dir.vsock_socket(), beacon);
        }
    }

    // -------------------------------------------------------------------
    // VmConfig::rlimit_fsize (ADR-0082 §D2, lie 6 / C-6)
    // -------------------------------------------------------------------

    /// Builds a structurally-valid `VmConfig` fixture, varying only the
    /// two inputs `rlimit_fsize` derives from. Constructs `MemoryPlan`
    /// via its private fields (visible to this nested test module)
    /// rather than `MemoryPlan::derive`, because `derive` calls
    /// `reserve_bytes`, which is a RED scaffold that panics until step
    /// 01-05 (§D2.3) — this fixture exercises `VmConfig::rlimit_fsize`,
    /// not `MemoryPlan::derive`.
    fn sample_vm_config(master_bytes: u64, guest_bytes: u64) -> VmConfig {
        let alloc = AllocationId::new("alloc-rlimit-fixture").unwrap();
        let mut header = vec![0u8; KERNEL_MAGIC_WINDOW];
        header[..4].copy_from_slice(b"\x7fELF");
        let kernel =
            KernelImage::validate(PathBuf::from("/boot/vmlinuz"), HostArch::X86_64, &header)
                .unwrap();
        VmConfig {
            alloc: alloc.clone(),
            kernel,
            rootfs: RootfsPlan::for_alloc(
                PathBuf::from("/var/lib/overdrive/images/base.img"),
                master_bytes,
                &alloc,
            ),
            cmdline: KernelCmdline::platform_default(HostArch::X86_64),
            memory: MemoryPlan { guest_bytes, cgroup_max_bytes: guest_bytes },
            vcpus: NonZeroU8::new(1).unwrap(),
            run_dir: VmRunDir::for_alloc(&PathBuf::from("/run/overdrive"), &alloc),
            confinement: VmConfinement::confined(
                VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
                1024,
            ),
            netns: None,
            cgroup_scope: CgroupPath::for_alloc(&alloc),
        }
    }

    proptest! {
        /// `VmConfig::rlimit_fsize` is `max(rootfs master bytes, guest
        /// memory bytes)` (lie 6 / C-6) for every combination of the two
        /// inputs it derives from.
        #[test]
        fn vm_config_rlimit_fsize_is_max_of_rootfs_and_guest_bytes(
            master_bytes in any::<u64>(),
            guest_bytes in any::<u64>(),
        ) {
            let config = sample_vm_config(master_bytes, guest_bytes);
            prop_assert_eq!(config.rlimit_fsize(), master_bytes.max(guest_bytes));
        }
    }
}
