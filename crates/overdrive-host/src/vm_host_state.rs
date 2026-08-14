//! Host [`VmHostState`] binding — real cgroupfs + real filesystem
//! directory walks over `overdrive.slice/workloads.slice/`, the VM run
//! root, and the rootfs-clone staging directory.
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
/// rootfs clone (`overdrive_core::vm::config::RootfsPlan`, ADR-0082 §D2
/// gap 3): `.overdrive-vm-rootfs-<alloc>.img`.
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
    /// Rootfs-clone staging directory — where per-launch clones are
    /// enumerated. Production composition (a later step) decides which
    /// concrete path this is; `RootfsPlan::for_alloc` derives the clone
    /// destination from the operator's own rootfs artifact's parent
    /// directory, so a non-default artifact location's clone is outside
    /// this adapter's enumeration by construction — a stated limitation,
    /// not a bug this port closes.
    staging_dir: PathBuf,
}

impl RealVmHostState {
    /// Construct a `RealVmHostState` against the three observation
    /// roots. Per `.claude/rules/development.md` § "Port-trait
    /// dependencies", all three are mandatory constructor parameters —
    /// no builder override, no in-constructor default.
    #[must_use]
    pub const fn new(cgroup_root: PathBuf, run_root: PathBuf, staging_dir: PathBuf) -> Self {
        Self { cgroup_root, run_root, staging_dir }
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
                |content| content.lines().filter_map(|line| line.trim().parse::<u32>().ok()).collect(),
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
        for entry in Self::list_dir_tolerant(&self.staging_dir).await? {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(middle) = name.strip_prefix(CLONE_PREFIX).and_then(|s| s.strip_suffix(CLONE_SUFFIX))
            else {
                continue;
            };
            if let Ok(alloc_id) = AllocationId::from_str(middle) {
                clones.insert(alloc_id, entry.path());
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
        for root in [self.workloads_slice_root(), self.run_root.clone(), self.staging_dir.clone()]
        {
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

        let clone_path = self.staging_dir.join(format!("{CLONE_PREFIX}{alloc}{CLONE_SUFFIX}"));
        match tokio::fs::remove_file(&clone_path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }

        Ok(())
    }
}
