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
use std::os::unix::fs::FileExt as _;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use async_trait::async_trait;
use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::traits::vm_host_state::{
    KILL_SCOPE_SETTLE_BOUND, ScopeFacts, VmHostObservation, VmHostState, VmHostStateProbeError,
};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

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

        match write_cgroup_kill(&path) {
            Ok(()) => {}
            // Already gone -- idempotent.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        }

        // POPULATED FIRST (ADR-0349, ADR-0351). `populated 0` is the only
        // reading that says no task of the scope is left; an `rmdir` of a
        // scope whose last task is held in exit succeeds while the task is
        // alive (spike findings-cgroup-events.md), so the removal waits for
        // the reading and is never the thing that detects emptiness.
        wait_until_unpopulated(&path).await?;

        // The scope is empty: `rmdir` is expected to succeed at once. Any
        // failure is a real error returned at once, never retried.
        match remove_scope_dir(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
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

// The three cgroupfs calls below are plain synchronous syscalls, not
// `tokio::fs`, because `tokio::fs` dispatches each call onto the blocking pool
// and `spawn_blocking` PANICS when the OS refuses a thread (`EAGAIN`), which
// under `panic = "abort"` ends the node. None of them waits on a task: the
// `cgroup.kill` write only signals the scope's tasks
// (`cgroup_kill_write`, `kernel/cgroup/cgroup.c @ v7.0`), the `cgroup.events`
// open and the `rmdir` are kernfs operations that do not sleep on the scope's
// tasks, and the removal of an already-empty scope is immediate.

/// Writes `1` to the scope's `cgroup.kill`, signalling every task in it.
fn write_cgroup_kill(scope_dir: &Path) -> std::io::Result<()> {
    std::fs::write(scope_dir.join("cgroup.kill"), b"1")
}

/// Opens the scope's `cgroup.events`.
fn open_cgroup_events(scope_dir: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(scope_dir.join("cgroup.events"))
}

/// Removes the (empty) scope directory.
fn remove_scope_dir(scope_dir: &Path) -> std::io::Result<()> {
    std::fs::remove_dir(scope_dir)
}

/// Whether the scope directory no longer exists.
fn scope_is_gone(scope_dir: &Path) -> bool {
    matches!(std::fs::metadata(scope_dir), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// Whether `error` says the cgroup behind an already-open `cgroup.events` was
/// removed: kernfs fails a read of a removed node with `ENODEV`
/// (`kernfs_seq_start`, `fs/kernfs/file.c @ v7.0`).
fn is_scope_removed_error(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ENODEV)
}

/// Waits for the scope's `cgroup.events` to report `populated 0`, bounded by
/// [`KILL_SCOPE_SETTLE_BOUND`]; at the bound returns `ResourceBusy` unless the
/// scope directory is gone, in which case another remover took it and it is `Ok`.
///
/// Removing a cgroup wakes no `POLLPRI` waiter (`kernfs_drain_open_files`
/// notifies no poller, `fs/kernfs/file.c @ v7.0`), so a scope removed
/// mid-wait is noticed at the bound, or by the next read after any other wake,
/// which fails with `ENODEV`; both read as gone.
///
/// A scope with no `cgroup.events` (absent, or a directory that is not a
/// cgroup) has nothing to wait for and returns `Ok`. The wait is a readiness
/// wait on the file's `POLLPRI`, so no thread is blocked in the kernel and the
/// bound's timer cancels it: `populated` changes call `cgroup_file_notify`
/// (`kernel/cgroup/cgroup.c:864 @ v7.0`), whose `kernfs_notify` wakes pollers
/// and makes `kernfs_fop_poll` report `EPOLLPRI | EPOLLERR` while the reader's
/// event counter is stale (`fs/kernfs/file.c:855-865 @ v7.0`). The file is
/// read after arming, tested, then awaited, so a change between the read and
/// the wait is not lost.
async fn wait_until_unpopulated(scope_dir: &Path) -> std::io::Result<()> {
    let file = match open_cgroup_events(scope_dir) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let events = AsyncFd::with_interest(file, Interest::PRIORITY)?;
    let settled = async {
        loop {
            match is_populated(events.get_ref()) {
                Ok(false) => return Ok(()),
                Ok(true) => {}
                Err(e) if is_scope_removed_error(&e) => return Ok(()),
                Err(e) => return Err(e),
            }
            let mut ready = events.ready(Interest::PRIORITY).await?;
            ready.clear_ready();
        }
    };
    match tokio::time::timeout(KILL_SCOPE_SETTLE_BOUND, settled).await {
        Ok(outcome) => outcome,
        Err(_bound_elapsed) if scope_is_gone(scope_dir) => Ok(()),
        Err(_bound_elapsed) => Err(std::io::Error::from(std::io::ErrorKind::ResourceBusy)),
    }
}

/// Reads `cgroup.events` from its start and returns its `populated` flag.
/// A file with no `populated` line reads as not populated.
fn is_populated(events: &std::fs::File) -> std::io::Result<bool> {
    let mut buffer = [0_u8; 256];
    let read = events.read_at(&mut buffer, 0)?;
    let text = String::from_utf8_lossy(buffer.get(..read).unwrap_or_default());
    Ok(text.lines().any(|line| line.trim() == "populated 1"))
}
