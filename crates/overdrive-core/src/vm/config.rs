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
//!   a cgroup OOM by construction). `reserve_bytes` now returns a real,
//!   measured bound — landed in step 01-05 via a real Cloud Hypervisor
//!   boot on the project's x86_64 KVM metal box (see its own docstring
//!   for the full memory.current / memory.stat measurement table).
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
//! # What Slice 03 (US-VM-7) lands (gap 5 CLOSED, ADR-0082 2026-08-17)
//!
//! - [`LandlockRule`] — one `--landlock-rules` grant. `access=rw` is
//!   rendered UNCONDITIONALLY (a read-only rule is insufficient — spike
//!   P5), the grant names a DIRECTORY (never a socket path CH would
//!   reject at parse time), and there is NO public constructor: the only
//!   producer is [`VmRunDir::landlock_grant`] (same-module private-field
//!   construction), which is what makes it the SOLE producer (brief.md
//!   § 113 dst-lint clause).
//! - [`VmRunDir::landlock_grant`] — the one explicit grant: `access=rw`
//!   on the run directory (CH auto-derives kernel/disk/serial/api grants
//!   but NOT the vsock socket it binds itself — P5 correction 2).
//! - [`VmConfig::landlock_rules`] — the aggregator, today exactly
//!   `vec![run_dir.landlock_grant()]`.
//! - [`VmConfinement::launch_wrapper`] — the `prlimit … -- setpriv … --`
//!   argv PREFIX that applies uid-drop + `setrlimit` with NO `unsafe`
//!   `pre_exec` (the resolution to `overdrive-host`'s
//!   `#![forbid(unsafe_code)]`; ADR-0082 §(c)).
//!
//! [`reserve_bytes`] (§D2.3) — measured GREEN in step 01-05 via a real
//! Cloud Hypervisor boot (see its own docstring for the full
//! memory.current / memory.stat measurement table).

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
    /// - `x86_64` accepts a `bzImage` (`HdrS` at `0x202`) or any
    ///   ELF-magic image (`\x7fELF` at `0x0`). PVH-ness is NOT
    ///   verifiable from this fixed [`KERNEL_MAGIC_WINDOW`] header
    ///   window — the PVH note lives in a `PT_NOTE` segment beyond the
    ///   window — so this validator accepts any ELF and leaves
    ///   non-PVH-vmlinux rejection to Cloud Hypervisor's own load-time
    ///   path (lie 3 / C-7 residual for the ELF class, ADR-0082 §D2.4;
    ///   review remediation F2). A distro `vmlinuz` loads directly.
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

    /// The validated image's path — the `--kernel` argument value. Step
    /// 01-06 (`CloudHypervisorVmm::create`) is the first consumer that
    /// needs the path back out; mirrors the accessor shape every sibling
    /// type in this file already exposes (`RootfsPlan::master`,
    /// `KernelCmdline::as_str`, `VmRunDir::*`).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
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
    pub const fn derive(declared: u64) -> Self {
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
/// **Measured, not guessed (step 01-05).** RSS structurally cannot supply
/// this figure — host page tables for the guest mapping are charged to
/// the allocation's cgroup scope via `memory.stat pagetables` and are
/// invisible to RSS (ADR-0082 §D2.3 Consequences, intake precedent
/// warning #7's "magic version floor" failure). The bound below is
/// derived from a REAL Cloud Hypervisor boot: `systemd-run --scope -p
/// Delegate=yes` wrapping `cloud-hypervisor v53.0` (`uname -r`
/// `7.0.0-15-generic`) on the project's real x86_64 KVM metal box
/// (`cargo xtask metal run --`), `--memory size=<N>M,prefault=on` (forces
/// full guest-RAM residency BEFORE the guest starts, so every reading
/// below is the worst-case/peak figure, never a transient one) against
/// the same bzImage-kernel + ext4-rootfs pairing step 01-04's `VmFixture`
/// stages, `--disk path=...,image_type=raw` (D2.1/C-2 — matches
/// [`DiskAttachment`]'s own rendering), `--vsock cid=3,socket=...`
/// (parity with the real launch shape). `memory.current` read at its
/// settled plateau (never a transient climbing/decaying sample) at seven
/// guest sizes:
///
/// | guest (MiB) | guest_bytes | `memory.current` (plateau) | reserve = current − guest_bytes |
/// |---:|---:|---:|---:|
/// | 128  | 134,217,728   | 137,732,096   | 3,514,368 (≈3.35 MiB)   |
/// | 256  | 268,435,456   | 272,420,864   | 3,985,408 (≈3.80 MiB)   |
/// | 512  | 536,870,912   | 540,844,032   | 3,973,120 (≈3.79 MiB)   |
/// | 1024 | 1,073,741,824 | 1,079,177,216 | 5,435,392 (≈5.18 MiB)   |
/// | 2048 | 2,147,483,648 | 2,154,864,640 | 7,380,992 (≈7.04 MiB)   |
/// | 4096 | 4,294,967,296 | 4,307,550,208 | 12,582,912 (≈12.00 MiB) |
/// | 8192 | 8,589,934,592 | 8,610,607,104 | 20,672,512 (≈19.72 MiB) |
///
/// Small guests are floor-dominated (Cloud Hypervisor's own userspace +
/// fixed emulation overhead — `kernel_stack`, `vmalloc`, `sock`,
/// `percpu`, process RSS — none of which scale with guest RAM); large
/// guests trend toward a roughly-linear per-byte cost consistent with the
/// host page-table overhead theory (an 8-byte PTE per 4096-byte guest
/// page is 1/512 ≈ 0.195%; `memory.stat pagetables` grew from 344,064 to
/// 16,883,712 bytes across the same 128 → 8192 MiB span). The
/// 4096→8192 MiB marginal rate is ≈0.19% (1/531); the noisier
/// 128→256 MiB marginal rate (small-sample jitter in slab/THP
/// allocation) is ≈0.35% (1/285).
///
/// `reserve_bytes` returns a DELIBERATELY CONSERVATIVE upper bound over
/// every measured point above, never the tightest fit: a fixed 8 MiB
/// floor (≈2× the largest floor-dominated reading, 3.80 MiB at 256 MiB)
/// plus `guest_bytes / 400` (≈0.25%, above the largest observed
/// large-guest marginal rate, ≈0.19%). The margin absorbs a different
/// kernel, a different Cloud Hypervisor version, or a host with
/// marginally higher per-page overhead than the one this was measured
/// on — a safety-margined ceiling, not "the exact measured number."
#[must_use]
pub const fn reserve_bytes(guest_bytes: u64) -> u64 {
    // 8 MiB -- comfortably above every floor-dominated small-guest
    // reading in the measured table above (largest observed: ~3.80 MiB
    // at 256 MiB guest RAM).
    const RESERVE_FLOOR_BYTES: u64 = 8 * 1024 * 1024;
    // guest_bytes / 400 (~0.25%) -- comfortably above the largest
    // observed large-guest marginal rate (~0.19%, the 4096->8192 MiB
    // step) and the ~1/512 host page-table theory.
    const RESERVE_FRACTION_DIVISOR: u64 = 400;

    RESERVE_FLOOR_BYTES.saturating_add(guest_bytes / RESERVE_FRACTION_DIVISOR)
}

// -----------------------------------------------------------------------
// [D8a]/[D8b] — VmVolume: the VmConfig volume payload, and the derived
// memory backing (`--memory shared=on` iff a volume is declared)
// -----------------------------------------------------------------------

/// One declared volume — a host↔guest share (feature-delta [D8a],
/// ADR-0083 §D3, GH #42, Slice 04). `source` is the host directory the
/// operator reads afterwards, `target` is the in-guest mount point the
/// operator's own command writes to, and `read_only` is enforced
/// HOST-side ([D8g]). The operator surface is exactly these three fields
/// — the virtiofsd mechanism (socket, tag, `--cache`, `--sandbox`,
/// `--memory shared=…`) is platform-derived and never appears here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmVolume {
    source: PathBuf,
    target: PathBuf,
    read_only: bool,
}

impl VmVolume {
    /// Build a declared volume from its three operator inputs.
    #[must_use]
    pub const fn new(source: PathBuf, target: PathBuf, read_only: bool) -> Self {
        Self { source, target, read_only }
    }

    /// The host directory the operator reads afterwards.
    #[must_use]
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// The in-guest mount point the operator's command writes to.
    #[must_use]
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// Whether the share is host-enforced read-only ([D8g]).
    #[must_use]
    pub const fn read_only(&self) -> bool {
        self.read_only
    }
}

/// How the guest's RAM is backed. `Shared` maps guest RAM as a shared
/// mapping (a memfd) — MANDATORY for any vhost-user backend, because the
/// backend (virtiofsd) must map the guest's memory; `Private` (anonymous
/// memory) is the volume-free default (feature-delta [D8b]).
///
/// A two-variant sum type so the `--memory` argument cannot carry an
/// invalid third state, and so `shared=on` is derived from ONE input —
/// the declared volumes — with no builder and no second config shape
/// (system constraint 4 / [D8b] reason 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryBacking {
    /// Private anonymous memory — the volume-free default. A VM with no
    /// volumes boots byte-identically to Slices 01–03 (S-VM-57).
    Private,
    /// Shared (memfd) backing — required by a vhost-user virtiofsd
    /// backend, so every volume-carrying VM uses it.
    Shared,
}

impl MemoryBacking {
    /// Derive the backing from the declared volumes: `Shared` iff at
    /// least one volume is declared, `Private` otherwise (feature-delta
    /// [D8b] — "`shared=on` iff `!volumes.is_empty()`"). This is THE ONE
    /// derivation `shared=on` comes from — no volume, no shared backing;
    /// one or more volumes, shared backing — which is what keeps a
    /// volume-free VM's boot byte-identical to Slices 01–03.
    #[must_use]
    pub const fn for_volumes(volumes: &[VmVolume]) -> Self {
        if volumes.is_empty() { Self::Private } else { Self::Shared }
    }

    /// `true` iff the guest RAM is shared-backed (renders `shared=on`).
    #[must_use]
    pub const fn is_shared(self) -> bool {
        matches!(self, Self::Shared)
    }

    /// The `--memory` argument suffix this backing appends after
    /// `size=<bytes>`: `,shared=on` for [`Shared`](Self::Shared), the
    /// empty string for [`Private`](Self::Private). This is the ONE
    /// branch [D8b] reason 2 names — rendered at the single `--memory`
    /// construction site, so a volume-free VM's `--memory` argument is
    /// unchanged from Slices 01–03.
    #[must_use]
    pub const fn memory_arg_suffix(self) -> &'static str {
        match self {
            Self::Private => "",
            Self::Shared => ",shared=on",
        }
    }
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

    /// The confined identity Cloud Hypervisor is spawned under — read by
    /// the create path so it can chown the run directory and the rootfs
    /// clone to the dropped `uid:gid` BEFORE spawn (ADR-0082 §(c)
    /// consequence 2). Without it, CH under the dropped uid could neither
    /// bind its sockets in the root-created run directory nor open the
    /// root-created rootfs clone `O_RDWR`. [`VmmIdentity`]'s fields are
    /// already public; this is the read accessor the wrapper application
    /// needs, not new confinement state.
    #[must_use]
    pub const fn identity(&self) -> &VmmIdentity {
        &self.identity
    }

    /// The `prlimit … -- setpriv … --` launch-wrapper PREFIX (argv tokens)
    /// that drops privilege and caps resource limits on the spawned
    /// hypervisor with NO `unsafe` `pre_exec` (ADR-0082 §(c)). The caller
    /// (`vmm.rs::create`) appends the hypervisor binary + its args. Per-child
    /// uid/gid drop and `setrlimit` are only expressible between `fork` and
    /// `exec`; the resolution honouring `overdrive-host`'s
    /// `#![forbid(unsafe_code)]` is to encapsulate the unsafe one layer out,
    /// in proven util-linux tools — exactly spike P5's 12/12 launch shape.
    ///
    /// `rlimit_fsize` is passed in — it is a [`VmConfig::rlimit_fsize`]
    /// derivation (`max(rootfs, guest RAM)`, C-6) this value does not hold.
    /// One pure rendering site and a mutation target (a mutant dropping
    /// `--no-new-privs`, rendering uid `0`, or omitting a wrapper token must
    /// be killed). Renders, in order:
    ///
    /// ```text
    /// prlimit --fsize=<rlimit_fsize> --nofile=<self.rlimit_nofile> --
    /// setpriv --reuid=<uid> --regid=<gid>
    ///         --groups=<supplementary,joined> --no-new-privs --
    /// ```
    ///
    /// `--groups=<numeric>` (from [`VmmIdentity::supplementary`]) — NOT
    /// `--init-groups` — because the confined uid has no passwd/group DB
    /// entry on the appliance (`setpriv` takes raw numerics); `--init-groups`
    /// would require one.
    #[must_use]
    pub fn launch_wrapper(&self, rlimit_fsize: u64) -> Vec<String> {
        let groups = self
            .identity
            .supplementary
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        vec![
            "prlimit".to_owned(),
            format!("--fsize={rlimit_fsize}"),
            format!("--nofile={}", self.rlimit_nofile),
            "--".to_owned(),
            "setpriv".to_owned(),
            format!("--reuid={}", self.identity.uid),
            format!("--regid={}", self.identity.gid),
            format!("--groups={groups}"),
            "--no-new-privs".to_owned(),
            "--".to_owned(),
        ]
    }
}

// -----------------------------------------------------------------------
// US-VM-7 (gap 5) — LandlockRule: one `--landlock-rules` directory grant
// -----------------------------------------------------------------------

/// One `--landlock-rules` grant (ADR-0082 §(a)). `access=rw` is rendered
/// UNCONDITIONALLY and is NOT a field: a read-only rule is insufficient
/// (spike P5 — a `vsock-only + dir-ro-rule` VM still `EACCES`es), so there
/// is no access parameter to get wrong — the same lever [`DiskAttachment`]
/// uses for `image_type=raw` (§D2.1). The grant names a DIRECTORY, never a
/// socket path: CH validates rule paths for existence at config-parse time
/// and the vsock UDS does not exist yet (P5 correction 2).
///
/// There is **no public constructor**: a `LandlockRule` is built only by
/// [`VmRunDir::landlock_grant`] (same-module private-field construction),
/// which is what makes that method the SOLE producer (brief.md § 113 —
/// "Landlock rules are never built outside `VmRunDir::landlock_grant`").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandlockRule {
    path: PathBuf,
}

impl LandlockRule {
    /// The complete `--landlock-rules` VALUE: `path=<dir>,access=rw`. One
    /// pure rendering site and a mutation target (a mutant flipping `rw` →
    /// `ro` or dropping `access=` must be killed). The FLAG literal
    /// `--landlock-rules` is rendered separately in `vmm.rs::create` — the
    /// sole site the 01-10 dst-lint clause sanctions.
    #[must_use]
    pub fn to_rule_arg(&self) -> String {
        format!("path={},access=rw", self.path.display())
    }

    /// The granted directory — accessor for the dst-lint / behavioural
    /// tests.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// -----------------------------------------------------------------------
// D2 gap 3 — RootfsPlan: the FICLONE source, its size, and the
// per-launch clone destination
// -----------------------------------------------------------------------

/// The rootfs staging plan: the operator's read-only master image, its
/// size (captured at construction so [`VmConfig::rlimit_fsize`] stays
/// pure), the per-launch clone destination, and the platform-owned
/// clone-index link that points at that clone (ADR-0082 §D2 gap 3;
/// ADR-0083 §§D3f-D3h).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootfsPlan {
    master: PathBuf,
    master_bytes: u64,
    clone_dest: PathBuf,
    index_link: PathBuf,
}

impl RootfsPlan {
    /// Build the plan for one allocation. `master` is the operator's
    /// BYO rootfs artifact; `master_bytes` is its size, captured HERE
    /// by the caller (the imperative shell does the `stat`) — the same
    /// Functional-Core / Imperative-Shell split [`KernelImage::validate`]
    /// applies.
    ///
    /// The clone destination sits in the **platform-owned staging root**
    /// `staging_dir` ([`clone_staging_dir`], derived over the node's durable
    /// `data_dir`), NOT beside the operator's master (ADR-0082 2026-08-18
    /// fourth amendment (c-fix.2), B1 fix). Reaching a clone under the
    /// operator's directory would require an `o+x` traverse grant on that
    /// operator dir — the DAC regression this fix removes. FICLONE is
    /// intra-filesystem, so `staging_dir` MUST share the master's filesystem
    /// (the appliance's one VM data partition — the operator-facing
    /// create-time precondition; a master on a foreign fs fails closed as
    /// `ConfinementUnavailable { UidDrop }` on the FICLONE `EXDEV`). The
    /// filename **carries `alloc`** so a reboot-orphaned clone is
    /// attributable (SD-1 / SD-2) and so ADR-0083's `index_link` parsing is
    /// unchanged wherever the clone lives.
    ///
    /// The **index link** sits in `index_dir` (the platform-owned
    /// [`clone_index_dir`], derived over the node's durable `data_dir`)
    /// and carries the SAME filename as the clone, so `VmHostState`'s
    /// `CLONE_PREFIX`/`CLONE_SUFFIX` parsing recovers the same allocation
    /// id from the link. The clone's location is **recorded** in this
    /// durable index at the moment the platform chooses it — never
    /// re-derived — because a boot that lost its in-memory [`RootfsPlan`]
    /// must still reclaim the clone by reading the link (ADR-0083 §§D3f-D3h,
    /// DWD-26).
    #[must_use]
    pub fn for_alloc(
        master: PathBuf,
        master_bytes: u64,
        alloc: &AllocationId,
        staging_dir: &Path,
        index_dir: &Path,
    ) -> Self {
        let file_name = format!(".overdrive-vm-rootfs-{alloc}.img");
        let clone_dest = staging_dir.join(&file_name);
        let index_link = index_dir.join(&file_name);
        Self { master, master_bytes, clone_dest, index_link }
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

    /// The platform-owned clone-index symlink for this launch, under
    /// [`clone_index_dir`]. `VmDriver` creates it BEFORE the FICLONE and
    /// removes it AFTER the clone, so `no link ⇒ no clone` and the
    /// reclamation sweep enumerates a superset of live clones by walking
    /// the index (ADR-0083 §§D3f-D3h).
    #[must_use]
    pub fn index_link(&self) -> &Path {
        &self.index_link
    }
}

/// The platform-owned clone-index directory under the node's durable
/// `data_dir`: `<data_dir>/vm/clone-index/` (ADR-0083 §D3g). ONE
/// derivation, called by BOTH composition sites (`compose_vm_driver`'s
/// `VmHostLayout.clone_index_dir` and `RealVmHostState::new`'s
/// `index_dir`), so neither derives it independently. Modelled on
/// `RedbViewStore::resolve_path` — a pure join over `data_dir`, and
/// deliberately **not under `/run`**, so the index survives a process
/// restart and a `VmDriver` that lost its in-memory [`RootfsPlan`] can
/// still have its clone reclaimed by reading the durable link (S-VM-84
/// ending 3).
#[must_use]
pub fn clone_index_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("vm").join("clone-index")
}

/// The platform-owned VM clone-staging root under the node's durable
/// `data_dir`: `<data_dir>/vm/clone-staging/` (ADR-0082 2026-08-18 fourth
/// amendment (c-fix.2)). Every per-launch rootfs clone is FICLONE'd HERE —
/// a platform-owned directory the confined identity reaches via a
/// set-once traverse grant (`0710 root:<confined-gid>`, applied at node
/// setup, never per-alloc) — instead of beside the operator's master,
/// whose directory the platform must never widen. Because FICLONE is
/// intra-filesystem, this root MUST live on the same filesystem as the
/// operator's rootfs master (the appliance's one VM data partition); a
/// master on a foreign filesystem is the create-time `EXDEV` that
/// [`crate::traits::vmm::Vmm::create`] maps to `ConfinementUnavailable`.
/// ONE derivation, sibling of [`clone_index_dir`] — a pure join over
/// `data_dir`, deliberately NOT under `/run` so it shares the durable
/// data partition where BYO artifacts land.
#[must_use]
pub fn clone_staging_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("vm").join("clone-staging")
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
/// convention (ADR-0082 §D2.2). Its [`landlock_grant`](Self::landlock_grant)
/// is the SOLE producer of a [`LandlockRule`] (gap 5, US-VM-7).
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

    /// `<dir>/kernel` — this allocation's OWN copy of the operator's kernel
    /// master (ADR-0082 2026-08-18 fourth amendment (c-fix.1), B1 fix). The
    /// platform copies the operator's kernel byte-for-byte here as root
    /// before spawn and `chown`s the copy to the confined identity;
    /// `Vmm::create` renders `--kernel` against THIS path, so Cloud
    /// Hypervisor loads the copy and auto-derives its read-only kernel
    /// Landlock grant against it. The operator's own kernel is only ever
    /// OPENED READ-ONLY to source the copy — never mode-widened. Sibling of
    /// [`console_log`](Self::console_log) / [`vsock_socket`](Self::vsock_socket)
    /// per SD-2's "`VmRunDir` owns every path inside it"; reclaimed with the
    /// run dir at teardown, and inside this allocation's OWN Landlock
    /// boundary.
    #[must_use]
    pub fn kernel_copy(&self) -> PathBuf {
        self.0.join("kernel")
    }

    /// The ONE explicit Landlock grant: `access=rw` on the run directory
    /// itself (ADR-0082 §(b), gap 5). CH auto-derives rules for
    /// `--kernel` / `--disk` / `--serial file=` / `--api-socket` but NOT
    /// the vsock UDS it binds (P5 correction 2); the rule must be the
    /// CONTAINING DIRECTORY (CH rejects a not-yet-existent socket path at
    /// parse time). SD-2 exclusivity — the run directory holds nothing but
    /// this VM's own sockets and logs — is what keeps this grant from
    /// widening. SOLE producer of a [`LandlockRule`] (same-module
    /// private-field construction).
    #[must_use]
    pub fn landlock_grant(&self) -> LandlockRule {
        LandlockRule { path: self.0.clone() }
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

    /// The explicit Landlock rules for this launch (ADR-0082 §(b), gap 5)
    /// — today exactly `vec![self.run_dir.landlock_grant()]`. CH
    /// auto-derives the kernel/disk/serial/api grants, so this list carries
    /// ONLY what CH omits: the run directory containing the vsock socket
    /// (C-4). `Vec` keeps the signature stable if a future explicit grant
    /// is ever added.
    #[must_use]
    pub fn landlock_rules(&self) -> Vec<LandlockRule> {
        vec![self.run_dir.landlock_grant()]
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
        /// `RootfsPlan::for_alloc` derives the clone destination in the
        /// PLATFORM-OWNED staging root (never beside the operator's master —
        /// ADR-0082 2026-08-18 fourth amendment (c-fix.2), B1 fix) with a
        /// filename carrying the allocation id (SD-1 / SD-2 reap attribution),
        /// while the index link keeps that SAME filename under the durable
        /// index dir.
        #[test]
        fn rootfs_plan_clone_dest_sits_in_staging_root_and_carries_alloc(
            dir_name in "[a-z0-9]{1,8}",
            file_name in "[a-z0-9]{1,8}",
            data_name in "[a-z0-9]{1,8}",
            alloc_suffix in ALLOC_SUFFIX_STRATEGY,
            master_bytes in any::<u64>(),
        ) {
            let alloc = AllocationId::new(&alloc_suffix).unwrap();
            let master = PathBuf::from(format!("/var/lib/overdrive/{dir_name}/{file_name}.img"));
            let data_dir = PathBuf::from(format!("/srv/{data_name}"));
            let staging_dir = clone_staging_dir(&data_dir);
            let index_dir = clone_index_dir(&data_dir);
            let plan =
                RootfsPlan::for_alloc(master.clone(), master_bytes, &alloc, &staging_dir, &index_dir);

            prop_assert_eq!(plan.master(), master.as_path());
            prop_assert_eq!(plan.master_bytes(), master_bytes);
            // The clone lives in the PLATFORM staging root, NOT beside the
            // operator's master — the whole point of the B1 fix.
            prop_assert_eq!(plan.clone_dest().parent(), Some(staging_dir.as_path()));
            prop_assert_ne!(plan.clone_dest().parent(), master.parent());

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

            // The index link sits in `index_dir` and shares the clone's
            // filename, so `VmHostState`'s prefix/suffix parsing recovers
            // the SAME alloc id from the link WHEREVER the clone lives
            // (ADR-0083 §§D3f-D3h — attribution is unchanged by the relocation).
            prop_assert_eq!(plan.index_link().parent(), Some(index_dir.as_path()));
            prop_assert_eq!(
                plan.index_link().file_name(),
                plan.clone_dest().file_name(),
                "index link and clone must share a filename so the sweep parses the alloc from the link",
            );
            // The index link is NOT the clone itself — the clone lives in the
            // staging root, the link under the durable index dir.
            prop_assert_ne!(plan.index_link(), plan.clone_dest());
        }
    }

    #[test]
    fn clone_staging_dir_is_data_dir_joined_vm_clone_staging() {
        let dir = clone_staging_dir(&PathBuf::from("/srv/overdrive/data"));
        assert_eq!(dir, PathBuf::from("/srv/overdrive/data/vm/clone-staging"));
    }

    #[test]
    fn clone_index_dir_is_data_dir_joined_vm_clone_index() {
        let dir = clone_index_dir(&PathBuf::from("/srv/overdrive/data"));
        assert_eq!(dir, PathBuf::from("/srv/overdrive/data/vm/clone-index"));
        // Deliberately NOT under /run — the index must survive a restart
        // (ADR-0083 §D3g).
        assert!(!dir.starts_with("/run"), "the clone index must live under the durable data_dir");
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
            // Exact derivation relation (review remediation F5): CH's wire
            // contract is `<vsock_socket>_<port>` (ADR-0082 §D2.2 / P2) --
            // pin it precisely, not just "some file under dir whose name
            // contains the port", which survives a `"vsock_{}"` ->
            // `"beacon_{}"` prefix mutation.
            prop_assert_eq!(
                &beacon,
                &PathBuf::from(format!("{}_{}", run_dir.vsock_socket().display(), port))
            );
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
    /// two inputs `rlimit_fsize` derives from. Uses the real
    /// `MemoryPlan::derive` constructor (switched in step 01-05, now that
    /// `reserve_bytes` has a real body — `derive` preserves `guest_bytes`
    /// exactly, so `rlimit_fsize`, which only reads `guest_bytes`, is
    /// unaffected by the ceiling `derive` computes alongside it).
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
                &clone_staging_dir(&PathBuf::from("/srv/overdrive/data")),
                &clone_index_dir(&PathBuf::from("/srv/overdrive/data")),
            ),
            cmdline: KernelCmdline::platform_default(HostArch::X86_64),
            memory: MemoryPlan::derive(guest_bytes),
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

    // -------------------------------------------------------------------
    // VmVolume + MemoryBacking (feature-delta [D8a]/[D8b], Slice 04)
    // -------------------------------------------------------------------

    fn sample_volume(i: usize) -> VmVolume {
        VmVolume::new(
            PathBuf::from(format!("/host/vol{i}")),
            PathBuf::from(format!("/guest/vol{i}")),
            i.is_multiple_of(2),
        )
    }

    proptest! {
        /// [D8b] — `shared=on` is derived from `!volumes.is_empty()` and
        /// nothing else: `MemoryBacking::for_volumes` is `Shared` iff at
        /// least one volume is declared, `Private` otherwise, for any
        /// number of declared volumes. The `--memory` suffix renders
        /// `,shared=on` ONLY for the shared backing, so a volume-free VM's
        /// `--memory` argument is byte-identical to Slices 01–03 (S-VM-57).
        #[test]
        fn memory_backing_shared_iff_volumes_present(volume_count in 0usize..8) {
            let volumes: Vec<VmVolume> = (0..volume_count).map(sample_volume).collect();
            let backing = MemoryBacking::for_volumes(&volumes);

            let expected_shared = volume_count > 0;
            prop_assert_eq!(backing.is_shared(), expected_shared);
            prop_assert_eq!(
                backing,
                if expected_shared { MemoryBacking::Shared } else { MemoryBacking::Private },
            );
            prop_assert_eq!(
                backing.memory_arg_suffix(),
                if expected_shared { ",shared=on" } else { "" },
            );
        }
    }

    proptest! {
        /// A `VmVolume` preserves its three operator inputs verbatim —
        /// `source`, `target` and `read_only` are exactly what was
        /// declared (the operator surface [D8a] pins).
        #[test]
        fn vm_volume_preserves_source_target_read_only(
            source in "[a-z/]{1,16}",
            target in "[a-z/]{1,16}",
            read_only in any::<bool>(),
        ) {
            let volume = VmVolume::new(
                PathBuf::from(&source),
                PathBuf::from(&target),
                read_only,
            );
            prop_assert_eq!(volume.source(), Path::new(&source));
            prop_assert_eq!(volume.target(), Path::new(&target));
            prop_assert_eq!(volume.read_only(), read_only);
        }
    }
}
