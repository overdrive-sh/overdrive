//! Host [`VmHostState`] binding — real cgroupfs + real filesystem
//! directory walks over `overdrive.slice/workloads.slice/`, the VM run
//! root, and the platform-owned clone-index directory.
//!
//! The clone-index symlinks resolve to clones sitting beside the
//! operator's own rootfs master (ADR-0083 §§D3f-D3h, DWD-26).
//!
//! Production binding of the [`VmHostState`] port trait (ADR-0083 §D7,
//! `brief.md` §105a.2). See
//! `overdrive_core::traits::vm_host_state::VmHostState` for the full
//! port-trait contract (preconditions, postconditions, edge cases) — this
//! adapter implements that contract; it does not restate it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::traits::vm_host_state::{
    ScopeFacts, VmHostObservation, VmHostState, VmHostStateProbeError,
};

/// The fixed relative path segment every workload cgroup scope lives
/// under, resolved against this adapter's `cgroup_root`. Mirrors the
/// literal `CgroupPath::for_alloc` hard-codes
/// (`overdrive_core::cgroup::CgroupPath`) — kept as a literal here too
/// rather than introduced as a shared constant, since `CgroupPath` does
/// not expose one either.
const WORKLOADS_SLICE: &str = "overdrive.slice/workloads.slice";

/// Filename prefix/suffix `RootfsPlan::for_alloc` mints for a per-launch
/// rootfs clone AND for its clone-index symlink (both carry the SAME
/// name so this parsing recovers the alloc id from the link —
/// `overdrive_core::vm::config::RootfsPlan`, ADR-0082 §D2 gap 3 /
/// ADR-0083 §§D3f-D3h): `.overdrive-vm-rootfs-<alloc>.img`.
const CLONE_PREFIX: &str = ".overdrive-vm-rootfs-";
const CLONE_SUFFIX: &str = ".img";

/// Bound on `kill_scope`'s settle retry loop — the kernel's SIGKILL
/// delivery + reap after `cgroup.kill` is asynchronous, so a `rmdir`
/// immediately after the write can race a still-draining
/// `cgroup.procs`. 150 attempts * 20ms = 3s ceiling, comfortably above
/// an ordinary process's reap latency.
const KILL_SCOPE_SETTLE_MAX_ATTEMPTS: u32 = 150;
const KILL_SCOPE_SETTLE_RETRY_INTERVAL: Duration = Duration::from_millis(20);

/// Production [`VmHostState`] binding.
///
/// The sim counterpart is
/// `overdrive_sim::adapters::vm_host_state::SimVmHostState`. Swap at the
/// wiring boundary; no call site should need both.
///
/// # Concurrency
///
/// Carries no mutable state beyond the three configured roots. Cloning is
/// cheap (three `PathBuf`s); `Send + Sync + 'static` per the
/// [`VmHostState`] supertrait requirement.
#[derive(Debug, Clone)]
pub struct RealVmHostState {
    /// cgroupfs root (`/sys/fs/cgroup` in production and integration
    /// tests) — `WORKLOADS_SLICE` is resolved under this.
    cgroup_root: PathBuf,
    /// VM run root (`VmRunDir`'s root — `overdrive_core::vm::config::VmRunDir`)
    /// — each child directory is one allocation's run directory, named
    /// verbatim as the allocation id.
    run_root: PathBuf,
    /// The platform-owned clone-index directory
    /// ([`overdrive_core::vm::config::clone_index_dir`] over the node's
    /// durable `data_dir`) — one symlink per live per-launch clone,
    /// `.overdrive-vm-rootfs-<alloc>.img -> <clone beside the operator
    /// master>`. `observe_clones` enumerates the LINKS and resolves each
    /// via `read_link`, so a clone written beside an operator-chosen
    /// rootfs (ADR-0083 §D3a/§D3b — FICLONE is intra-filesystem) is
    /// reclaimable even though it lives outside any platform directory.
    /// The clone's location is RECORDED here at launch, never re-derived
    /// (§§D3f-D3h, DWD-26).
    index_dir: PathBuf,
}

impl RealVmHostState {
    /// Construct a `RealVmHostState` against the three observation
    /// roots. Per `.claude/rules/development.md` § "Port-trait
    /// dependencies", all three are mandatory constructor parameters —
    /// no builder override, no in-constructor default. `index_dir` is the
    /// platform-owned clone-index directory
    /// ([`overdrive_core::vm::config::clone_index_dir`]) — the SAME
    /// expression `compose_vm_driver` feeds `VmHostLayout.clone_index_dir`.
    #[must_use]
    pub const fn new(cgroup_root: PathBuf, run_root: PathBuf, index_dir: PathBuf) -> Self {
        Self { cgroup_root, run_root, index_dir }
    }

    fn workloads_slice_root(&self) -> PathBuf {
        self.cgroup_root.join(WORKLOADS_SLICE)
    }

    /// List `dir`'s entries. A genuinely absent `dir` yields an empty
    /// iterator rather than an error — mirrors [`VmHostState::probe`]'s
    /// "an absent root is Ok" tolerance, applied to `observe` too: a
    /// node that has never run a VM has no run root / no staging
    /// directory yet, and that must not fail an ordinary reclamation
    /// tick.
    async fn list_dir_tolerant(dir: &Path) -> std::io::Result<Vec<tokio::fs::DirEntry>> {
        let mut read_dir = match tokio::fs::read_dir(dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut entries = Vec::new();
        loop {
            match read_dir.next_entry().await {
                Ok(Some(entry)) => entries.push(entry),
                Ok(None) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(entries)
    }

    async fn observe_scopes(&self) -> std::io::Result<BTreeMap<AllocationId, ScopeFacts>> {
        let root = self.workloads_slice_root();
        let mut scopes = BTreeMap::new();
        for entry in Self::list_dir_tolerant(&root).await? {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(alloc_str) = name.strip_suffix(".scope") else { continue };
            let Ok(alloc_id) = AllocationId::from_str(alloc_str) else { continue };
            let procs_path = entry.path().join("cgroup.procs");
            // A scope with an unreadable/absent cgroup.procs (mid
            // teardown, or a benign race) reports empty rather than
            // failing the whole observation.
            let pids = tokio::fs::read_to_string(&procs_path).await.map_or_else(
                |_| BTreeSet::new(),
                |content| {
                    content.lines().filter_map(|line| line.trim().parse::<u32>().ok()).collect()
                },
            );
            scopes.insert(alloc_id, ScopeFacts { pids });
        }
        Ok(scopes)
    }

    async fn observe_run_dirs(&self) -> std::io::Result<BTreeSet<AllocationId>> {
        let mut run_dirs = BTreeSet::new();
        for entry in Self::list_dir_tolerant(&self.run_root).await? {
            let is_dir = entry.file_type().await.is_ok_and(|ft| ft.is_dir());
            if !is_dir {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if let Ok(alloc_id) = AllocationId::from_str(name) {
                run_dirs.insert(alloc_id);
            }
        }
        Ok(run_dirs)
    }

    async fn observe_clones(&self) -> std::io::Result<BTreeMap<AllocationId, PathBuf>> {
        let mut clones = BTreeMap::new();
        for entry in Self::list_dir_tolerant(&self.index_dir).await? {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(middle) =
                name.strip_prefix(CLONE_PREFIX).and_then(|s| s.strip_suffix(CLONE_SUFFIX))
            else {
                continue;
            };
            let Ok(alloc_id) = AllocationId::from_str(middle) else { continue };
            // The mapped value is the clone the index link RESOLVES to
            // (ADR-0083 §D3h) — `read_link`, never `entry.path()`. A
            // DANGLING link (its target already gone) still `read_link`s
            // successfully and MUST still yield an entry (§D3f's crash
            // table: `after clone removal, before link removal` leaves a
            // dangling link the sweep is required to report and dispose).
            match tokio::fs::read_link(entry.path()).await {
                Ok(target) => {
                    clones.insert(alloc_id, target);
                }
                // Vanished between listing and resolution — benign.
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                // A non-symlink entry (`readlink` → `EINVAL`) is skipped
                // and logged. This is a greenfield single cut: no migration
                // is written for pre-existing regular files.
                Err(err) if err.kind() == std::io::ErrorKind::InvalidInput => {
                    let entry_path = entry.path();
                    tracing::warn!(
                        name: "vm.clone_index.non_symlink_entry",
                        entry = %entry_path.display(),
                        "clone-index entry is not a symlink; skipping (no migration for pre-existing regular files)"
                    );
                }
                Err(err) => return Err(err),
            }
        }
        Ok(clones)
    }
}

#[async_trait]
impl VmHostState for RealVmHostState {
    fn kind(&self) -> &'static str {
        "overdrive_host::RealVmHostState"
    }

    async fn probe(&self) -> Result<(), VmHostStateProbeError> {
        for root in [self.workloads_slice_root(), self.run_root.clone(), self.index_dir.clone()] {
            match tokio::fs::read_dir(&root).await {
                Ok(_) => {}
                // An absent root is Ok -- a node that has never run a VM
                // has no run root; refusing its boot would be absurd.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(VmHostStateProbeError::substrate(root, e)),
            }
        }
        Ok(())
    }

    async fn observe(&self) -> std::io::Result<VmHostObservation> {
        let scopes = self.observe_scopes().await?;
        let run_dirs = self.observe_run_dirs().await?;
        let clones = self.observe_clones().await?;
        Ok(VmHostObservation { scopes, run_dirs, clones })
    }

    async fn kill_scope(&self, scope: &CgroupPath) -> std::io::Result<()> {
        let path = scope.resolve(&self.cgroup_root);

        match tokio::fs::write(path.join("cgroup.kill"), b"1").await {
            Ok(()) => {}
            // Already gone -- idempotent.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        }

        // SETTLE: does not return until the rmdir has succeeded or
        // returned NotFound. The kernel's SIGKILL delivery + reap after
        // `cgroup.kill` is asynchronous, so an immediate `rmdir` can
        // race a still-draining `cgroup.procs` -- retry until it
        // settles.
        for attempt in 0..KILL_SCOPE_SETTLE_MAX_ATTEMPTS {
            match tokio::fs::remove_dir(&path).await {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) if attempt + 1 == KILL_SCOPE_SETTLE_MAX_ATTEMPTS => return Err(e),
                Err(_not_yet_settled) => {
                    tokio::time::sleep(KILL_SCOPE_SETTLE_RETRY_INTERVAL).await;
                }
            }
        }
        // Unreachable: the loop above always returns on its last
        // iteration (Ok, NotFound-as-Ok, or the final Err).
        Ok(())
    }

    async fn discard_artifacts(&self, alloc: &AllocationId) -> std::io::Result<()> {
        let run_dir = self.run_root.join(alloc.as_str());
        match tokio::fs::remove_dir_all(&run_dir).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }

        // Resolve the clone by READING THE INDEX LINK (ADR-0083 §D3h) —
        // never by re-deriving the clone path from a directory join. That
        // re-derivation IS the defect this step fixes: an operator
        // spec-edit or workload deletion can destroy `parent([vm] rootfs)`
        // while the clone survives, so its location is RECORDED in the
        // durable link, not recomputed. The link path itself is the
        // platform-owned `<index_dir>/<clone-name>` — the index directory
        // is ours, so deriving the LINK's location is not the forbidden
        // re-derivation (the CLONE's location is what must come from
        // `read_link`). Remove the TARGET first, then the LINK — the same
        // clone-before-link ordering `VmDriver::stop` uses — both
        // NotFound-tolerant, so `no link ⇒ no clone` survives an
        // interruption here too.
        let link_path = self.index_dir.join(format!("{CLONE_PREFIX}{alloc}{CLONE_SUFFIX}"));
        let clone_target = match tokio::fs::read_link(&link_path).await {
            Ok(target) => target,
            // No link (already removed, or never existed) — nothing to do.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            // A non-symlink at the link path (`readlink` → `EINVAL`) —
            // skip; greenfield single cut, no pre-existing-file migration.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => return Ok(()),
            Err(e) => return Err(e),
        };
        match tokio::fs::remove_file(&clone_target).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        match tokio::fs::remove_file(&link_path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }

        Ok(())
    }
}
