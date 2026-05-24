//! Workload-cgroup management for `ExecDriver`.
//!
//! Creates and tears down
//! `overdrive.slice/workloads.slice/<alloc_id>.scope` directories,
//! writes `cpu.weight` / `memory.max` per ADR-0026, and removes the
//! scope after process reap. Five filesystem operations per workload
//! lifecycle.
//!
//! # `CgroupManager` — port-routed surface (ADR-0054)
//!
//! Per ADR-0054 § D5, the cgroup-manager surface is a struct that
//! holds an `Arc<dyn CgroupFs>` and a `cgroup_root: PathBuf`. Every
//! mutation goes through the port (`self.fs.{create_dir,write,
//! remove_dir}`); no direct `tokio::fs::*` references in the methods.
//! Production wires `RealCgroupFs`; tests wire `SimCgroupFs`.
//!
//! Mandatory-not-defaulted constructor per
//! `.claude/rules/development.md` § "Port-trait dependencies": both
//! `cgroup_root` and `fs` must be supplied at construction — no
//! `Default`, no `with_fs` / `with_cgroup_root` builder.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use overdrive_core::id::AllocationId;
use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::driver::Resources;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

/// Concrete relative path of a workload cgroup, validated at
/// construction. STRICT-newtype per
/// `.claude/rules/development.md` § Newtype completeness:
///   * `FromStr` — validating, rejects path-traversal characters
///     (leading `/`, `..`, `//`, NUL).
///   * `Display` — canonical relative form.
///   * `Serialize`/`Deserialize` — round-trip via `Display`/`FromStr`.
///   * `rkyv::Archive` — deferred to durable boundary (Phase 1 transient).
///
/// Canonical form for workload scopes:
///   `overdrive.slice/workloads.slice/<alloc_id>.scope`
///
/// Stored relative; the cgroupfs root is supplied by the driver.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct CgroupPath(String);

impl CgroupPath {
    /// Construct the canonical workload scope path for a given
    /// allocation: `overdrive.slice/workloads.slice/<alloc>.scope`.
    #[must_use]
    pub fn for_alloc(alloc: &AllocationId) -> Self {
        // The constructed shape is canonical-by-construction: the
        // alloc id is already validated, the slice prefix is fixed,
        // so `from_str` would also accept it.
        Self(format!("overdrive.slice/workloads.slice/{alloc}.scope"))
    }

    /// Borrow the canonical relative-path string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Resolve under a cgroupfs root (`/sys/fs/cgroup` in production
    /// and integration tests).
    #[must_use]
    pub fn resolve(&self, root: &Path) -> PathBuf {
        root.join(&self.0)
    }
}

impl fmt::Display for CgroupPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for CgroupPath {
    type Err = CgroupPathError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        if raw.is_empty() {
            return Err(CgroupPathError::Empty);
        }
        if raw.contains('\0') {
            return Err(CgroupPathError::InvalidPath { raw: raw.to_owned() });
        }
        if raw.starts_with('/') {
            return Err(CgroupPathError::InvalidPath { raw: raw.to_owned() });
        }
        if raw.contains("//") {
            return Err(CgroupPathError::InvalidPath { raw: raw.to_owned() });
        }
        // Reject any `..` segment.
        for segment in raw.split('/') {
            if segment.is_empty() || segment == ".." {
                return Err(CgroupPathError::InvalidPath { raw: raw.to_owned() });
            }
        }
        Ok(Self(raw.to_owned()))
    }
}

impl TryFrom<String> for CgroupPath {
    type Error = CgroupPathError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::from_str(&raw)
    }
}

impl TryFrom<&str> for CgroupPath {
    type Error = CgroupPathError;
    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        Self::from_str(raw)
    }
}

impl From<CgroupPath> for String {
    fn from(v: CgroupPath) -> Self {
        v.0
    }
}

/// Errors from [`CgroupPath::from_str`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CgroupPathError {
    /// Empty input.
    #[error("empty cgroup path")]
    Empty,
    /// Input contains a path-traversal sequence (`..`, leading `/`,
    /// double slashes, NUL, etc.).
    #[error("invalid cgroup path: {raw}")]
    InvalidPath {
        /// Echo of the rejected input for diagnostics.
        raw: String,
    },
}

// ---------------------------------------------------------------------------
// Workload-cgroup management entrypoints
// ---------------------------------------------------------------------------

/// Controllers delegated to `overdrive.slice/workloads.slice` children.
/// Per AC2 of step 01-02: enable all four (`+cpu +memory +io +pids`)
/// for symmetry with the control-plane half rather than the
/// bare-minimum `+cpu +memory` the resource-limit surface technically
/// requires — costs nothing on the kernel side and avoids "why is
/// workloads.slice narrower than overdrive.slice" reviewer comments
/// later. Trailing newline matches the cgroup-v2 admin-guide write
/// shape.
const WORKLOADS_SUBTREE_CONTROL_CONTROLLERS: &str = "+cpu +memory +io +pids\n";

/// Errors from the workload-side cgroup-bootstrap surface
/// ([`create_workloads_slice_with_controllers`]).
///
/// Per `docs/feature/fix-cgroup-subtree-control-delegation/bugfix-rca.md`
/// § "Production fix #2 — Typed errors", the `subtree_control` write
/// surfaces as a discrete error variant per `.claude/rules/development.md`
/// § "Distinct failure modes get distinct error variants" — never
/// absorbed into a generic `io::Error`. EBUSY (a process is already
/// in the cgroup; the slice was previously initialised in the wrong
/// order) and "any other I/O error" carry distinct operator
/// remediation hints.
///
/// Mirrors `overdrive_control_plane::error::CgroupBootstrapError` —
/// the worker crate cannot depend on the control-plane crate
/// (architecture is the other way around per ADR-0029), so the
/// taxonomy is duplicated here. The two enums are kept in lockstep by
/// review.
#[derive(Debug, Error)]
pub enum WorkloadsBootstrapError {
    /// The kernel returned EBUSY on the
    /// `overdrive.slice/workloads.slice/cgroup.subtree_control` write —
    /// a process is already enrolled somewhere under
    /// `workloads.slice` (an alloc scope was created BEFORE the
    /// controllers were enabled). The cgroup v2 contract forbids
    /// modifying `subtree_control` on a cgroup whose descendants
    /// contain a live process.
    ///
    /// Operator hint: the worker must call this init BEFORE accepting
    /// any allocations. If the leak persists across restarts, run the
    /// leftover-cgroup cleanup discipline from
    /// `.claude/rules/testing.md` § "Leaked workload cgroups across
    /// runs".
    #[error(
        "workloads.slice `subtree_control` write rejected with EBUSY — a \
         process is already enrolled under workloads.slice (controllers \
         must be enabled BEFORE any alloc scope is created).\n\
         \n\
         Try: ensure create_workloads_slice_with_controllers is called \
         before the convergence loop accepts any allocations. If a \
         stale alloc scope persists across restarts, sweep leftover \
         cgroups per the leftover-cgroup cleanup discipline.\n\
         \n\
         Underlying: {source}"
    )]
    SubtreeControlBusy {
        /// Underlying `io::Error` carrying `ErrorKind::ResourceBusy`
        /// (or the equivalent `raw_os_error() == EBUSY`).
        #[source]
        source: std::io::Error,
    },

    /// Catch-all I/O failure on the workloads-slice setup that is NOT
    /// EBUSY. Any other `ErrorKind` — typically `PermissionDenied`
    /// (`EACCES` from cgroupfs delegation refusal), `NotFound` (the
    /// enclosing slice does not exist), or a `mkdir` failure — flows
    /// here.
    ///
    /// Operator hint: inspect cgroupfs delegation for `overdrive.slice`.
    /// The control-plane pre-flight must have passed for the worker to
    /// reach this code path; a failure here typically means the
    /// runtime delegation surface differs (e.g. a systemd unit
    /// replaced the slice between control-plane bootstrap and worker
    /// bootstrap).
    #[error(
        "workloads.slice bootstrap failed: {source}\n\
         \n\
         Try: inspect cgroupfs delegation for overdrive.slice — the \
         pre-flight passed, so this is typically a runtime divergence \
         (a systemd unit replaced the slice between control-plane and \
         worker bootstrap, or a concurrent operator action removed \
         delegation)."
    )]
    WriteFailed {
        /// Underlying `io::Error` for any non-EBUSY I/O failure.
        #[source]
        source: std::io::Error,
    },
}

impl WorkloadsBootstrapError {
    /// Construct from an `io::Error` returned by a `subtree_control`
    /// write, dispatching on `ErrorKind` so EBUSY surfaces as the
    /// discrete [`WorkloadsBootstrapError::SubtreeControlBusy`]
    /// variant and everything else collapses into
    /// [`WorkloadsBootstrapError::WriteFailed`].
    ///
    /// Mirrors `CgroupBootstrapError::from_subtree_control_io` — the
    /// EBUSY-vs-other discrimination logic is identical because the
    /// kernel-level contract is identical. See
    /// `crates/overdrive-control-plane/src/error.rs`'s constructor for
    /// the rationale on `ErrorKind::ResourceBusy` vs `raw_os_error`.
    #[must_use]
    pub fn from_subtree_control_io(source: std::io::Error) -> Self {
        let is_ebusy = matches!(source.kind(), std::io::ErrorKind::ResourceBusy)
            || source.raw_os_error() == Some(libc::EBUSY);
        if is_ebusy { Self::SubtreeControlBusy { source } } else { Self::WriteFailed { source } }
    }
}

// Bootstrap surface removed as a free fn at step 01-07: per ADR-0054
// § D5 the surface is an async method on `CgroupManager` that routes
// every filesystem mutation through `self.fs.{create_dir,write}`. See
// `CgroupManager::create_workloads_slice_with_controllers` below.

/// Compute `cpu.weight` from `cpu_milli` per ADR-0026 D9:
/// `clamp(cpu_milli / 10, 1, 10000)`.
#[must_use]
pub fn cpu_weight_for(cpu_milli: u32) -> u32 {
    (cpu_milli / 10).clamp(1, 10_000)
}

// ---------------------------------------------------------------------------
// CgroupManager — port-routed surface (ADR-0054 § D5)
// ---------------------------------------------------------------------------

/// Workload-cgroup management routed through the [`CgroupFs`] port.
///
/// Per ADR-0054 § D5. Every filesystem mutation flows through
/// `self.fs.{create_dir,write,remove_dir}`; no direct `tokio::fs::*`
/// references in method bodies. Production wires
/// `overdrive_host::RealCgroupFs`; tests wire
/// `overdrive_sim::SimCgroupFs`.
///
/// # Construction
///
/// Mandatory-not-defaulted per `.claude/rules/development.md`
/// § "Port-trait dependencies": both `cgroup_root` and `fs` are
/// required at [`CgroupManager::new`]; there is no `Default` impl
/// and no builder. Tests that forget to inject the port fail to
/// compile rather than silently inheriting wall-clock / OS-cgroupfs
/// behaviour.
#[derive(Clone)]
pub struct CgroupManager {
    fs: Arc<dyn CgroupFs>,
    cgroup_root: PathBuf,
}

impl CgroupManager {
    /// Construct a `CgroupManager`. Both parameters are mandatory.
    #[must_use]
    pub fn new(cgroup_root: PathBuf, fs: Arc<dyn CgroupFs>) -> Self {
        Self { fs, cgroup_root }
    }

    /// Borrow the configured cgroup root.
    #[must_use]
    pub fn cgroup_root(&self) -> &Path {
        &self.cgroup_root
    }

    /// Create the workload scope directory under the configured
    /// cgroup root.
    ///
    /// # Errors
    /// Returns the underlying `std::io::Error` from
    /// [`CgroupFs::create_dir`].
    pub async fn create_workload_scope(&self, scope: &CgroupPath) -> Result<(), std::io::Error> {
        let dir = scope.resolve(&self.cgroup_root);
        self.fs.create_dir(&dir).await
    }

    /// Place a process PID into the workload scope's `cgroup.procs`
    /// file. Writes `"{pid}\n"`.
    ///
    /// # Errors
    /// Returns the underlying `std::io::Error` from
    /// [`CgroupFs::write`].
    pub async fn place_pid_in_scope(
        &self,
        scope: &CgroupPath,
        pid: u32,
    ) -> Result<(), std::io::Error> {
        let path = scope.resolve(&self.cgroup_root).join("cgroup.procs");
        let payload = format!("{pid}\n");
        self.fs.write(&path, payload.as_bytes()).await
    }

    /// Write `cpu.weight` and `memory.max` for the given scope,
    /// derived from `Resources` per ADR-0026 D9.
    ///
    /// # Errors
    /// Returns the underlying `std::io::Error` if either limit file
    /// cannot be written.
    pub async fn write_resource_limits(
        &self,
        scope: &CgroupPath,
        resources: &Resources,
    ) -> Result<(), std::io::Error> {
        let dir = scope.resolve(&self.cgroup_root);
        let weight = cpu_weight_for(resources.cpu_milli);
        let weight_payload = format!("{weight}\n");
        let memory_payload = format!("{}\n", resources.memory_bytes);
        self.fs.write(&dir.join("cpu.weight"), weight_payload.as_bytes()).await?;
        self.fs.write(&dir.join("memory.max"), memory_payload.as_bytes()).await?;
        Ok(())
    }

    /// Wrapper for [`Self::write_resource_limits`] that converts a
    /// write error into a structured warning log AND returns to the
    /// caller per ADR-0026 D9 warn-and-continue disposition.
    pub async fn write_resource_limits_warn_on_error(
        &self,
        scope: &CgroupPath,
        resources: &Resources,
    ) {
        if let Err(err) = self.write_resource_limits(scope, resources).await {
            warn!(
                scope = %scope,
                error = %err,
                "cgroup resource-limit write failed; continuing per ADR-0026 D9"
            );
        }
    }

    /// Mass-kill every process in the workload cgroup via the kernel's
    /// `cgroup.kill` interface (cgroup v2, kernel 5.14+) — writes
    /// `1\n` to `<scope>/cgroup.kill`.
    ///
    /// Idempotent — `NotFound` (scope already gone) is reported as
    /// `Ok`. Invalid-argument writes (a path that exists but is not a
    /// v2 cgroup) surface to the caller.
    ///
    /// # Errors
    /// Returns the underlying `std::io::Error` if the write fails for
    /// a reason other than the scope being absent.
    pub async fn cgroup_kill(&self, scope: &CgroupPath) -> Result<(), std::io::Error> {
        let path = scope.resolve(&self.cgroup_root).join("cgroup.kill");
        match self.fs.write(&path, b"1\n").await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Remove the workload scope directory after process reap.
    /// Idempotent — succeeds when the directory is already gone.
    ///
    /// # Errors
    /// Returns the underlying `std::io::Error` if `rmdir` fails for a
    /// reason other than the directory being absent.
    pub async fn remove_workload_scope(&self, scope: &CgroupPath) -> Result<(), std::io::Error> {
        let dir = scope.resolve(&self.cgroup_root);
        match self.fs.remove_dir(&dir).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Bootstrap the workload-bearing slice.
    ///
    /// Creates `overdrive.slice/workloads.slice` under the configured
    /// `cgroup_root` and delegates the standard set of controllers
    /// (`+cpu +memory +io +pids`) to its `cgroup.subtree_control`.
    /// Called once at worker startup BEFORE the convergence loop
    /// accepts any allocations.
    ///
    /// # Order is load-bearing
    ///
    /// The two steps run in this order, no exceptions:
    ///
    /// 1. `mkdir -p overdrive.slice/workloads.slice` (idempotent)
    /// 2. write `+cpu +memory +io +pids\n` to
    ///    `overdrive.slice/workloads.slice/cgroup.subtree_control`
    ///    (idempotent on already-enabled controllers — kernel accepts
    ///    the re-enable as a no-op)
    ///
    /// Step 2 MUST complete before any `alloc-*.scope` directory is
    /// created underneath `workloads.slice`. The cgroup v2 kernel
    /// contract forbids modifying a parent's `subtree_control` while
    /// any child cgroup contains a live process — the kernel returns
    /// `EBUSY`. The production wiring at
    /// `overdrive-control-plane::run_server_with_*` calls this init
    /// before spawning the convergence loop.
    ///
    /// # Idempotency
    ///
    /// Both steps survive repeated boots: the port's
    /// [`CgroupFs::create_dir`](overdrive_core::traits::CgroupFs::create_dir)
    /// is `mkdir -p` semantics (idempotent on existing directories),
    /// and the kernel treats a re-write of `+cpu +memory +io +pids` to
    /// an already-enabled `cgroup.subtree_control` as a no-op. A
    /// process supervisor calling the init on every restart sees Ok
    /// every time.
    ///
    /// # Errors
    ///
    /// * [`WorkloadsBootstrapError::SubtreeControlBusy`] — the kernel
    ///   returned EBUSY on the `subtree_control` write because a
    ///   process was already enrolled under `workloads.slice`.
    /// * [`WorkloadsBootstrapError::WriteFailed`] — any other I/O
    ///   failure from either the `mkdir` or the `subtree_control`
    ///   write (`PermissionDenied` from cgroupfs delegation refusal,
    ///   `NotFound` if the enclosing `overdrive.slice` does not exist,
    ///   etc.).
    pub async fn create_workloads_slice_with_controllers(
        &self,
    ) -> Result<(), WorkloadsBootstrapError> {
        // Step 1 — mkdir -p overdrive.slice/workloads.slice. The
        // `CgroupFs::create_dir` contract is `mkdir -p`; both ancestor
        // and leaf land in one call.
        let workloads_slice = self.cgroup_root.join("overdrive.slice/workloads.slice");
        self.fs
            .create_dir(&workloads_slice)
            .await
            .map_err(|source| WorkloadsBootstrapError::WriteFailed { source })?;

        // Step 2 — delegate controllers. MUST happen BEFORE any alloc
        // scope is created under workloads.slice per the cgroup v2
        // contract documented in this method's "Order is load-bearing"
        // section.
        let subtree_control = workloads_slice.join("cgroup.subtree_control");
        if let Err(err) =
            self.fs.write(&subtree_control, WORKLOADS_SUBTREE_CONTROL_CONTROLLERS.as_bytes()).await
        {
            return Err(WorkloadsBootstrapError::from_subtree_control_io(err));
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Unit tests — pure-logic helpers retained in #[cfg(test)] mod tests
// ---------------------------------------------------------------------------
//
// Per ADR-0054 § E1 KEEP-INLINE rows 1-5: the following pure-logic
// tests stay inline (no FS, no port involvement). The 7 CONVERT
// rows (cgroup_kill, remove_workload_scope, create_workload_scope,
// place_pid_in_scope, write_resource_limits,
// write_resource_limits_warn_on_error, plus cgroup_kill_writes_one_newline)
// have moved to SimCgroupFs-backed acceptance tests under
// `tests/acceptance/cgroup_manager/`.

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]
mod tests {
    use super::*;

    /// `CgroupPath::as_str` returns the canonical relative form. Pin
    /// the exact string for a representative `for_alloc` construction
    /// — kills the two body-replacement mutations
    /// (`as_str -> &str with ""` and `with "xyzzy"`).
    #[test]
    fn cgroup_path_as_str_returns_canonical_string() {
        let alloc = AllocationId::new("alloc-as-str-0").expect("valid AllocationId");
        let scope = CgroupPath::for_alloc(&alloc);
        assert_eq!(
            scope.as_str(),
            "overdrive.slice/workloads.slice/alloc-as-str-0.scope",
            "as_str must return the canonical form",
        );
        // Belt-and-braces: explicitly reject the mutant marker and
        // empty string.
        assert_ne!(scope.as_str(), "", "as_str must not be empty");
        assert_ne!(scope.as_str(), "xyzzy", "as_str must not be the mutant marker `xyzzy`");
    }

    /// `cpu_weight_for` is `cpu_milli / 10` clamped to `[1, 10000]`.
    /// Pin samples at the divider, lower clamp, and upper clamp —
    /// together they kill the four mutations on this function:
    ///
    ///   - body → 0  — fails the 1 mCPU lower-clamp test (expects 1)
    ///   - body → 1  — fails the 100_000 mCPU upper-clamp test
    ///   - `/` → `*` — 100 mCPU becomes 1000, not 10
    ///   - `/` → `%` — 1000 mCPU becomes 0, then clamps to 1, not 100
    #[test]
    fn cpu_weight_for_pins_division_and_clamp() {
        assert_eq!(cpu_weight_for(100), 10, "100 mCPU → weight 10");
        assert_eq!(cpu_weight_for(1), 1, "1 mCPU clamps up to 1 (lower bound)");
        assert_eq!(cpu_weight_for(100_000), 10_000, "100k mCPU at upper clamp");
        assert_eq!(cpu_weight_for(200_000), 10_000, "200k mCPU clamps down to 10_000");
        assert_eq!(cpu_weight_for(1000), 100, "1000 mCPU → weight 100");
    }

    /// `WorkloadsBootstrapError::from_subtree_control_io` discriminates
    /// EBUSY (typed `SubtreeControlBusy`) from any other I/O error
    /// (typed `WriteFailed`). Pins the `||` and `==` mutations on
    /// the EBUSY check — `||` → `&&` would force both
    /// `ResourceBusy` AND `raw_os_error == EBUSY` to be true; `==`
    /// → `!=` would invert the raw-os-error match. Either mutation
    /// reroutes EBUSY into the catch-all `WriteFailed` arm.
    #[test]
    fn from_subtree_control_io_discriminates_ebusy_via_resource_busy_kind() {
        let err = std::io::Error::from(std::io::ErrorKind::ResourceBusy);
        let mapped = WorkloadsBootstrapError::from_subtree_control_io(err);
        assert!(
            matches!(mapped, WorkloadsBootstrapError::SubtreeControlBusy { .. }),
            "ResourceBusy ErrorKind must map to SubtreeControlBusy; got {mapped:?}",
        );
    }

    #[test]
    fn from_subtree_control_io_discriminates_ebusy_via_raw_os_error() {
        let err = std::io::Error::from_raw_os_error(libc::EBUSY);
        let mapped = WorkloadsBootstrapError::from_subtree_control_io(err);
        assert!(
            matches!(mapped, WorkloadsBootstrapError::SubtreeControlBusy { .. }),
            "raw_os_error == EBUSY must map to SubtreeControlBusy; got {mapped:?}",
        );
    }

    #[test]
    fn from_subtree_control_io_routes_non_ebusy_to_write_failed() {
        let err = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let mapped = WorkloadsBootstrapError::from_subtree_control_io(err);
        assert!(
            matches!(mapped, WorkloadsBootstrapError::WriteFailed { .. }),
            "PermissionDenied must map to WriteFailed (NOT EBUSY); got {mapped:?}",
        );

        let err = std::io::Error::from(std::io::ErrorKind::NotFound);
        let mapped = WorkloadsBootstrapError::from_subtree_control_io(err);
        assert!(
            matches!(mapped, WorkloadsBootstrapError::WriteFailed { .. }),
            "NotFound must map to WriteFailed (NOT EBUSY); got {mapped:?}",
        );
    }

    // E1 KEEP-AND-MOVE rows 15 + 16 (per DISTILL § E1) — bootstrap
    // pair tests moved at step 01-07 alongside the async conversion
    // of the bootstrap surface from a sync free fn to an async method
    // on `CgroupManager`. See
    // `tests/acceptance/cgroup_manager/workloads_slice_bootstrap.rs`.
}
