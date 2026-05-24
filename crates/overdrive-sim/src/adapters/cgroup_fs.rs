//! `SimCgroupFs` — test binding of the [`CgroupFs`] port trait.
//!
//! Per ADR-0054 § Sim adapter. In-memory `BTreeMap<PathBuf, (SimEntry,
//! Vec<u8>)>` byte store guarded by `parking_lot::Mutex`; per-(`SimOp`,
//! `PathBuf`) injectable error schedule keyed by
//! `BTreeMap<(u8, PathBuf), VecDeque<io::ErrorKind>>` for deterministic
//! iteration.
//!
//! # Cancellation semantics — method-entry deterministic
//!
//! Per ADR-0054 § D4. The mutation happens atomically inside the
//! method body BEFORE the first `.await`. Mid-syscall cancellation is a
//! kernel concept that does not apply in-process — either the
//! mutation has not happened yet (caller dropped the future before
//! the lock was acquired) or it is fully complete (lock acquired,
//! `BTreeMap` mutated, lock released). NEVER partial state. K3
//! reproducibility (seed → bit-identical trajectory) extends
//! naturally because the only nondeterminism source is the
//! `BTreeMap`-keyed injection schedule (`BTreeMap` iteration is
//! `Ord`-deterministic).
//!
//! # Concurrency
//!
//! Every method body acquires `parking_lot::Mutex`, mutates the
//! `BTreeMap`, and releases. NO `.await` while holding the guard
//! (per `.claude/rules/development.md` § "Concurrency & async" —
//! "Never hold a lock across `.await`"). The `async fn` surface
//! exists only to satisfy the trait signature.
//!
//! # Non-replacement contract
//!
//! `SimCgroupFs` is byte-honest about kernel side effects it does NOT
//! model: when test code writes a PID payload to `cgroup.procs`, the
//! snapshot contains those exact bytes — no process movement is
//! simulated. When test code writes `"1\n"` to `cgroup.kill`, the
//! snapshot contains `b"1\n"` — no mass-kill is simulated. Kernel-
//! side semantics are exclusively covered by Tier 3 (Lima sudo) per
//! ADR-0054 § D3.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use overdrive_core::traits::{CgroupFs, ProbeError};

/// Filesystem entry shape stored under each path in the in-memory
/// `BTreeMap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimEntry {
    /// A directory entry. The associated `Vec<u8>` byte payload is
    /// unused (kept as `Vec::new()` for storage parity with `File`).
    Dir,
    /// A regular file entry. The associated `Vec<u8>` carries the
    /// bytes most recently written via [`CgroupFs::write`].
    File,
}

/// Operation discriminator used to key the injectable error schedule.
///
/// Each variant maps to one [`CgroupFs`] method. The error schedule is
/// keyed by `(SimOp::to_byte(), PathBuf)`; calls pop pending errors
/// per-(op, path) and surface them in lieu of executing the underlying
/// mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimOp {
    /// Discriminator for [`CgroupFs::create_dir`].
    CreateDir,
    /// Discriminator for [`CgroupFs::write`].
    Write,
    /// Discriminator for [`CgroupFs::remove_dir`].
    RemoveDir,
    /// Discriminator for [`CgroupFs::probe`].
    Probe,
}

impl SimOp {
    /// Stable byte mapping for the `BTreeMap` schedule key. Stability
    /// matters because the schedule key participates in `BTreeMap`'s
    /// `Ord`-based iteration order — changing the mapping silently
    /// reorders pending-error processing across runs.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::CreateDir => 0,
            Self::Write => 1,
            Self::RemoveDir => 2,
            Self::Probe => 3,
        }
    }
}

/// Internal mutable state — both inner maps are `BTreeMap` (NOT
/// `HashMap`) per `.claude/rules/development.md` § "Ordered-collection
/// choice". The state map is observed by F1 K3 determinism via
/// [`SimCgroupFs::snapshot`]; iteration order must be deterministic
/// across runs.
type State = BTreeMap<PathBuf, (SimEntry, Vec<u8>)>;
type ErrorSchedule = BTreeMap<(u8, PathBuf), VecDeque<io::ErrorKind>>;

/// Sim binding of the [`CgroupFs`] port trait.
///
/// See module docstring for cancellation semantics, concurrency
/// discipline, and the non-replacement contract.
///
/// # Construction
///
/// ```
/// use overdrive_sim::SimCgroupFs;
/// let sim = SimCgroupFs::new();
/// ```
///
/// # Clone semantics
///
/// Cloning shares the underlying `Arc<Mutex<...>>` state and schedule.
/// Mirrors `SimClock` / `SimDataplane` so callers can hand one clone
/// to the harness and another to the system under test and have both
/// observe the same mutations.
#[derive(Clone, Debug, Default)]
pub struct SimCgroupFs {
    state: Arc<Mutex<State>>,
    errors: Arc<Mutex<ErrorSchedule>>,
    round_trip_mismatch: Arc<Mutex<bool>>,
}

impl SimCgroupFs {
    /// Construct an empty `SimCgroupFs` (empty state, empty schedule,
    /// no probe round-trip-mismatch injection set).
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(BTreeMap::new())),
            errors: Arc::new(Mutex::new(BTreeMap::new())),
            round_trip_mismatch: Arc::new(Mutex::new(false)),
        }
    }

    /// Inject a pending error for the next call of `op` against `path`.
    ///
    /// Multiple injections for the same `(op, path)` form a queue; each
    /// matching call pops one entry. An empty queue (no injections OR
    /// every prior injection consumed) means the call proceeds with
    /// the normal in-memory semantics.
    pub fn inject_error(&self, op: SimOp, path: PathBuf, kind: io::ErrorKind) {
        let key = (op.to_byte(), path);
        let mut errors = self.errors.lock();
        errors.entry(key).or_default().push_back(kind);
    }

    /// Inject a round-trip mismatch for the NEXT [`CgroupFs::probe`]
    /// invocation. Causes the probe to surface
    /// [`ProbeError::RoundTripMismatch`] even though the substrate's
    /// read returned bytes — the failure mode Earned Trust defends
    /// against (Docker overlayfs `fsync` no-op, WSL2 `DrvFs` caching,
    /// tmpfs eviction).
    ///
    /// **TEST-HOOK-ONLY**. Production composition root never calls
    /// this — the production binding (`RealCgroupFs`) detects the same
    /// failure mode via genuine substrate read-back assertion.
    pub fn inject_round_trip_mismatch(&self) {
        *self.round_trip_mismatch.lock() = true;
    }

    /// **TEST-HOOK-ONLY**. Clone the entire in-memory state for test
    /// inspection. The returned `BTreeMap` is a snapshot — subsequent
    /// mutations on `self` do not affect the returned value.
    ///
    /// F1 K3 determinism (per ADR-0054 § D4 + `.claude/rules/testing.md`
    /// § Tier 1 / DST) uses this method to assert two fresh
    /// `SimCgroupFs` instances given the same op sequence produce
    /// bit-identical final snapshots. `SimEntry::File` / `SimEntry::Dir`
    /// shape and byte payloads round-trip via `BTreeMap::clone`.
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<PathBuf, (SimEntry, Vec<u8>)> {
        self.state.lock().clone()
    }

    /// Walk strict ancestors of `path` and return `Err(NotADirectory)`
    /// if any ancestor exists as a [`SimEntry::File`]. Mirrors kernel
    /// POSIX semantics: a non-directory component used as a directory
    /// raises `ENOTDIR` before the target operation is attempted.
    fn check_no_file_ancestor(state: &State, path: &Path) -> io::Result<()> {
        let mut acc = PathBuf::new();
        for component in path.components() {
            acc.push(component);
            if acc == path {
                break;
            }
            if let Some((SimEntry::File, _)) = state.get(&acc) {
                return Err(io::Error::from(io::ErrorKind::NotADirectory));
            }
        }
        Ok(())
    }

    /// Take a pending error from the schedule for `(op, path)`, if any.
    /// Returns `Some(kind)` once per matching injection; `None`
    /// otherwise.
    fn take_pending_error(&self, op: SimOp, path: &Path) -> Option<io::ErrorKind> {
        let key = (op.to_byte(), path.to_path_buf());
        let mut errors = self.errors.lock();
        let kind = errors.get_mut(&key).and_then(VecDeque::pop_front);
        // Cleanup: when a queue empties, drop the entry so iteration
        // order over `errors` only reflects live injections. Stable
        // across runs because BTreeMap iteration is `Ord`-deterministic.
        if let Some(queue) = errors.get(&key) {
            if queue.is_empty() {
                errors.remove(&key);
            }
        }
        kind
    }
}

#[async_trait]
impl CgroupFs for SimCgroupFs {
    async fn create_dir(&self, path: &Path) -> io::Result<()> {
        if let Some(kind) = self.take_pending_error(SimOp::CreateDir, path) {
            return Err(io::Error::from(kind));
        }
        {
            let mut state = self.state.lock();
            // mkdir-p: walk each ancestor and insert `Dir` if absent.
            // Iterate over component-prefixes so we synthesise every
            // intermediate directory the way `tokio::fs::create_dir_all`
            // would on a real filesystem. Two failure modes mirror
            // kernel POSIX semantics:
            //   * strict-ancestor exists as File → `NotADirectory`
            //     (component used as dir is a file; ENOTDIR).
            //   * target itself exists as File → `AlreadyExists`
            //     (matches `tokio::fs::create_dir_all`, which retries
            //     after EEXIST, stats, finds non-dir, returns
            //     AlreadyExists).
            let mut acc = PathBuf::new();
            for component in path.components() {
                acc.push(component);
                if matches!(component, std::path::Component::RootDir) {
                    continue;
                }
                let is_target = acc == path;
                match state.get(&acc) {
                    Some((SimEntry::File, _)) => {
                        return Err(io::Error::from(if is_target {
                            io::ErrorKind::AlreadyExists
                        } else {
                            io::ErrorKind::NotADirectory
                        }));
                    }
                    Some((SimEntry::Dir, _)) => {}
                    None => {
                        state.insert(acc.clone(), (SimEntry::Dir, Vec::new()));
                    }
                }
            }
        }
        Ok(())
    }

    async fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(kind) = self.take_pending_error(SimOp::Write, path) {
            return Err(io::Error::from(kind));
        }
        {
            let mut state = self.state.lock();
            // Walk strict ancestors. If any exists as a File, the kernel
            // returns `ENOTDIR` (component used as dir is a file)
            // BEFORE attempting the open.
            Self::check_no_file_ancestor(&state, path)?;
            // Parent-existence check — matches Real adapter NotFound shape
            // per ADR-0054 § Trait contract (the trait docstring pins
            // `Err(NotFound)` when the parent is absent).
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() && !state.contains_key(parent) {
                    return Err(io::Error::from(io::ErrorKind::NotFound));
                }
            }
            // Refuse writing to a path that is currently a Dir. Matches
            // kernel POSIX semantics: `tokio::fs::write` (open + O_WRONLY)
            // on a directory returns `EISDIR` (`io::ErrorKind::IsADirectory`).
            if let Some((SimEntry::Dir, _)) = state.get(path) {
                return Err(io::Error::from(io::ErrorKind::IsADirectory));
            }
            state.insert(path.to_path_buf(), (SimEntry::File, bytes.to_vec()));
        }
        Ok(())
    }

    async fn remove_dir(&self, path: &Path) -> io::Result<()> {
        if let Some(kind) = self.take_pending_error(SimOp::RemoveDir, path) {
            return Err(io::Error::from(kind));
        }
        {
            let mut state = self.state.lock();
            // Walk ancestors. If any strict ancestor exists as a File,
            // the kernel returns `ENOTDIR` (component used as dir is a
            // file) BEFORE attempting the target removal.
            Self::check_no_file_ancestor(&state, path)?;
            let entry = match state.get(path) {
                Some(e) => e.clone(),
                None => return Err(io::Error::from(io::ErrorKind::NotFound)),
            };
            // `tokio::fs::remove_dir` against a regular file returns
            // `ENOTDIR` (`io::ErrorKind::NotADirectory`) per kernel
            // POSIX semantics. Mirror that here.
            if matches!(entry.0, SimEntry::File) {
                return Err(io::Error::from(io::ErrorKind::NotADirectory));
            }
            // DirectoryNotEmpty if any other path is a direct or transitive
            // child of `path`.
            let has_children = state.keys().any(|k| k != path && k.starts_with(path));
            if has_children {
                return Err(io::Error::from(io::ErrorKind::DirectoryNotEmpty));
            }
            state.remove(path);
        }
        Ok(())
    }

    async fn probe(&self) -> Result<(), ProbeError> {
        // Injection schedule check — keyed by (SimOp::Probe,
        // /sim-probe-root). Stable, hardcoded probe-root path keeps
        // the injection key reproducible across runs.
        let probe_root = PathBuf::from("/sim-probe-root");
        if let Some(kind) = self.take_pending_error(SimOp::Probe, &probe_root) {
            return Err(ProbeError::Substrate { source: io::Error::from(kind) });
        }
        // Round-trip mismatch injection — set by
        // [`SimCgroupFs::inject_round_trip_mismatch`]; consumed once.
        let mismatch_injected = {
            let mut flag = self.round_trip_mismatch.lock();
            let cur = *flag;
            *flag = false;
            cur
        };

        let probe_file = probe_root.join("probe-file");
        let payload = b"probe\n".to_vec();

        // (1) create_dir on the probe root.
        self.create_dir(&probe_root).await.map_err(|source| ProbeError::Substrate { source })?;

        // (2) write the canonical probe payload.
        if let Err(source) = self.write(&probe_file, &payload).await {
            // Best-effort teardown — mirrors RealCgroupFs::probe.
            let _ = self.remove_dir(&probe_root).await;
            return Err(ProbeError::Substrate { source });
        }

        // (3) read-back via direct BTreeMap lookup.
        let read_back = {
            let state = self.state.lock();
            state.get(&probe_file).map(|(_, bytes)| bytes.clone()).unwrap_or_default()
        };

        // (4) round-trip assertion. If injection flag set, corrupt the
        // observed bytes to simulate substrate dishonesty.
        let observed = if mismatch_injected { b"corrupted".to_vec() } else { read_back };
        if observed != payload {
            // Best-effort teardown — drop the File entry directly
            // since `remove_dir` on a File now returns NotADirectory
            // to match real-kernel POSIX semantics.
            {
                let mut state = self.state.lock();
                state.remove(&probe_file);
            }
            let _ = self.remove_dir(&probe_root).await;
            return Err(ProbeError::RoundTripMismatch { wrote: payload, read: observed });
        }

        // (5) teardown — remove the probe_file File entry directly
        // from state (the trait surface does NOT expose `remove_file`;
        // the kernel-side cgroupfs probe relies on the pseudo-file
        // being reaped automatically by `rmdir` on the parent). Then
        // `remove_dir` the probe_root, which now contains no children.
        {
            let mut state = self.state.lock();
            state.remove(&probe_file);
        }
        self.remove_dir(&probe_root).await.map_err(|source| ProbeError::Substrate { source })?;
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "overdrive_sim::SimCgroupFs"
    }
}
