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
//! # What is deferred, and why
//!
//! `VmRunDir` (§D2.2), the outer `VmConfig` aggregate (§D2), and the
//! `Vmm` port trait (§D1 — `create`'s signature takes `&VmConfig`, so the
//! trait cannot compile without it) are **not** landed this step. Three
//! independent, confirmed blockers on `VmConfig`'s remaining field types
//! prevent building them without inventing API surface ADR-0082 does not
//! pin (CLAUDE.md § "Implement to the design — never invent API
//! surface"):
//!
//! 1. **`cgroup_scope: CgroupPath`** — a fully-built newtype already
//!    lives at `overdrive_worker::cgroup_manager::CgroupPath`
//!    (`adapter-host` class). `overdrive-core` (`core` class) cannot
//!    depend on `overdrive-worker` — the dependency runs the other way
//!    (`overdrive-worker` depends on `overdrive-core`) — so reusing it is
//!    impossible, and minting a second, differently-shaped `CgroupPath`
//!    in `overdrive-core` would create two divergent types under one
//!    name.
//! 2. **`netns: Option<NetnsName>`** — ADR-0082 §D2 names this newtype,
//!    but `crates/overdrive-core/src/traits/driver.rs:168-178` records a
//!    **deliberate, cited** prior decision (JOIN-1 /
//!    `docs/feature/transparent-mtls-enrollment/design/wave-decisions.md`
//!    D-TME-12) NOT to wrap the equivalent `AllocationSpec.netns` field
//!    in a newtype: it is a slot-derived name with "no parse surface, no
//!    operator-typed entry point, no `FromStr` round-trip to defend."
//!    `VmConfig.netns` is sourced from that same field.
//! 3. **`rootfs: RootfsPlan`** and **`cmdline: KernelCmdline`** — ADR-0082
//!    gives `RootfsPlan` a one-line shape hint ("master + master_bytes +
//!    clone destination") but `KernelCmdline` none at all beyond
//!    "platform-derived; NOT operator surface." Guessing field names and
//!    method surface for either risks a contract a later step then has to
//!    rework.
//!
//! `VmRunDir::landlock_grant() -> LandlockRule` (closing the vsock
//! Landlock gap, §D2.2) has the same shape gap: ADR-0082 states
//! `landlock_grant()` takes no parameter and always grants `rw` ("no
//! parameter to get wrong") but never shows `LandlockRule`'s field/method
//! surface, and the slice-01 doc's own Dependencies section assigns
//! "additive confinement items (Landlock, uid/gid drop, rlimits)" to
//! Slice 03 (US-VM-7), not Slice 01.
//!
//! These are flagged as a blocker in the step 01-01 handoff rather than
//! guessed. See the module doc in `crate::vm` for the summary.

use std::fmt;
use std::path::PathBuf;

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
