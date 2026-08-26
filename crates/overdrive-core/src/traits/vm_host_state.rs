//! [`VmHostState`] — the host-observation-driven port `VmReclamation`
//! hydrates its `actual` half from (SD-1's Bar-2 reconciler,
//! `.claude/rules/reconcilers.md`; ADR-0083 §D7; `brief.md` §105a.2).
//!
//! Production wires [`overdrive_host::RealVmHostState`](../../overdrive_host/struct.RealVmHostState.html)
//! (crate `overdrive-host`); simulation wires
//! `overdrive_sim::adapters::vm_host_state::SimVmHostState`. The
//! `vm_host_state_equivalence` structural guard (`overdrive-host` tests,
//! S-VM-91) drives both through the same call sequence, including the
//! [`VmHostState::kill_scope`] settle postcondition.
//!
//! A NEW port, not a widened [`crate::traits::cgroup_fs::CgroupFs`]
//! (ADR-0083 §A8): `CgroupFs` is deliberately write-only, and two of this
//! port's three observation surfaces (the VM run root, the staging
//! directory) are not cgroupfs at all. `VmHostState` is composed
//! **unconditionally** — a node that uninstalled `cloud-hypervisor` still
//! observes and still reclaims (`brief.md` §105a.2).
//!
//! Every method's preconditions / postconditions / edge cases / observable
//! invariants are pinned here per `.claude/rules/development.md`
//! § "Trait definitions specify behavior, not just signature" — the
//! equivalence test is this contract's enforcement.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use async_trait::async_trait;
use thiserror::Error;

use crate::AllocationId;
use crate::cgroup::CgroupPath;

/// Per-scope facts [`VmHostState::observe`] reports for one cgroup scope
/// under `overdrive.slice/workloads.slice/`. **NOT VM-exclusive** — exec
/// allocations live on this same surface (`brief.md` §105a.2); the
/// two-surface join that narrows to VM allocations happens in
/// `VmReclamationState::allocations` (the desired half), not here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScopeFacts {
    /// PIDs currently enrolled in this scope's `cgroup.procs`. Empty
    /// means the scope directory exists but currently holds no live
    /// process (a benign, observable state — not an error).
    pub pids: BTreeSet<u32>,
}

/// A PLAIN VALUE — three observation surfaces, no verdicts, no
/// derivation (`brief.md` §105a.2). Returned by [`VmHostState::observe`],
/// the named hydration seam.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VmHostObservation {
    /// `overdrive.slice/workloads.slice/<alloc>.scope` → its
    /// `cgroup.procs`, keyed by the allocation id parsed out of the
    /// scope's directory name. NOT VM-exclusive — exec allocations live
    /// here too.
    pub scopes: BTreeMap<AllocationId, ScopeFacts>,
    /// Allocation ids whose per-allocation directory exists under the VM
    /// run root. VM-exclusive by construction (SD-2 — the run root holds
    /// nothing but VM per-allocation directories).
    pub run_dirs: BTreeSet<AllocationId>,
    /// Per-launch rootfs clones in the staging directory, attributed by
    /// filename (`.overdrive-vm-rootfs-<alloc>.img`, ADR-0082 §D2 gap 3).
    /// VM-exclusive by construction — only [`crate::vm::config::RootfsPlan`]
    /// mints this filename shape.
    pub clones: BTreeMap<AllocationId, PathBuf>,
}

/// The host-observation-driven port `VmReclamation`
/// (`overdrive-reconcilers`) hydrates its `actual` half from.
///
/// # Earned Trust
///
/// Every adapter MUST implement [`probe`](Self::probe). Composed
/// **unconditionally** at the composition root — never gated on
/// [`crate::traits::vmm::Vmm`] composition — so a node with no
/// `cloud-hypervisor` installed still probes and still reclaims.
#[async_trait]
pub trait VmHostState: Send + Sync + 'static {
    /// Adapter discriminator for diagnostic logging. Mirrors
    /// [`crate::traits::cgroup_fs::CgroupFs::kind`]'s contract: a
    /// `&'static str` compile-time constant, stable across versions,
    /// crate-qualified for real adapters (e.g.
    /// `"overdrive_host::RealVmHostState"`).
    fn kind(&self) -> &'static str;

    /// Empirically demonstrate the three observation roots are
    /// enumerable (Earned Trust, CLAUDE.md principle 13).
    ///
    /// Asks a DIFFERENT question from [`crate::traits::vmm::Vmm::probe`]'s
    /// scenario 5 — that asks "is the run root creatable and bindable",
    /// the question a **launch** depends on, and is composition-gated.
    /// This asks "are the three roots enumerable", the question a
    /// **reclamation** depends on, and is unconditional.
    ///
    /// # Preconditions
    /// - The adapter is constructed; no other operation has been issued.
    ///
    /// # Postconditions on Ok
    /// - Every one of the three observation roots (cgroup scope tree, VM
    ///   run root, staging directory) either exists and is listable, OR
    ///   is genuinely **absent**.
    ///
    /// # Edge cases
    /// - **An absent root is `Ok`.** A node that has never run a VM has
    ///   no run root; refusing its boot would be absurd. Absence is a
    ///   known fact about the world, not a missing observation.
    /// - Any OTHER [`std::io::ErrorKind`] (`PermissionDenied`, `EIO`, an
    ///   unreadable cgroup tree) is a hard probe failure — absorbing it
    ///   into "absent" is exactly the `unwrap_or_default` failure
    ///   `.claude/rules/development.md` § "Distinct failure modes get
    ///   distinct error variants" forbids.
    ///
    /// # Errors
    /// [`VmHostStateProbeError::Substrate`] naming which root failed and
    /// the originating [`std::io::Error`], for every non-benign-absence
    /// failure.
    async fn probe(&self) -> Result<(), VmHostStateProbeError>;

    /// THE HYDRATION SEAM. One call, one plain observed-state value, no
    /// interpretation (`brief.md` §105a.2 — the method SD-1's pin 1 and
    /// GH #197's future generalisation both lift verbatim).
    ///
    /// # Postconditions on Ok
    /// The returned [`VmHostObservation`] reflects every allocation id
    /// currently attributable to each of the three surfaces, read at one
    /// snapshot instant. Pure-function IN EFFECT (read-only) — no write,
    /// no derivation, no verdict.
    ///
    /// # Edge cases
    /// - No allocations on the host: every field is empty, not an error.
    /// - A scope directory with an empty (or unreadable-as-benign)
    ///   `cgroup.procs`: reported with `pids: BTreeSet::new()`.
    ///
    /// # Errors
    /// The underlying [`std::io::Error`] from a non-benign-absence
    /// substrate failure while enumerating any of the three roots.
    async fn observe(&self) -> std::io::Result<VmHostObservation>;

    /// Write `cgroup.kill`, then remove the scope directory.
    ///
    /// # Preconditions
    /// - `scope` names a cgroup scope this adapter's cgroup root is
    ///   expected to resolve.
    ///
    /// # Postconditions on Ok
    /// - **Does NOT return until the scope directory's removal has
    ///   succeeded or returned `NotFound`.** This settle contract is a
    ///   postcondition of the port, not a rule the caller must remember
    ///   — `adopt_on_restart_recovery`'s boot-time read of the same tree
    ///   treats a mid-deletion scope as a hard `ObserveRead` error that
    ///   refuses the boot (`brief.md` §105a.5 / ADR-0083 §D7).
    ///
    /// # Edge cases
    /// - **Idempotent.** An already-absent scope: `cgroup.kill` write and
    ///   removal both resolve as success (`NotFound` is not an error
    ///   here).
    /// - A scope whose `cgroup.procs` still drains asynchronously after
    ///   `cgroup.kill` (kernel-side SIGKILL delivery + reap is NOT
    ///   synchronous with the write): the adapter retries removal until
    ///   it settles — this is what makes the postcondition true.
    ///
    /// # Errors
    /// The underlying [`std::io::Error`] if the substrate genuinely
    /// refuses (e.g. `PermissionDenied`) rather than merely not-yet-
    /// settled.
    async fn kill_scope(&self, scope: &CgroupPath) -> std::io::Result<()>;

    /// Remove this allocation's run directory and per-launch rootfs
    /// clone.
    ///
    /// # Postconditions on Ok
    /// - Neither the run directory nor the clone is reachable afterward.
    ///
    /// # Edge cases
    /// - **Absence of EITHER is success.** Total over every partial
    ///   state a crash between teardown steps can leave — a prior
    ///   partial disposal (run dir gone, clone still present, or vice
    ///   versa) converges to fully-gone on retry.
    ///
    /// # Errors
    /// The underlying [`std::io::Error`] from a non-benign-absence
    /// substrate failure.
    async fn discard_artifacts(&self, alloc: &AllocationId) -> std::io::Result<()>;
}

/// Failure surface for [`VmHostState::probe`].
#[derive(Debug, Error)]
pub enum VmHostStateProbeError {
    /// The probe's enumeration of `root` failed at the substrate level
    /// for a reason other than benign absence. Wraps the originating
    /// [`std::io::Error`] without reinterpretation.
    #[error("VmHostState probe failed enumerating {root}: {source}")]
    Substrate {
        /// Which of the three observation roots the failing enumeration
        /// targeted, for diagnosis.
        root: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl VmHostStateProbeError {
    #[must_use]
    pub const fn substrate(root: PathBuf, source: std::io::Error) -> Self {
        Self::Substrate { root, source }
    }
}
