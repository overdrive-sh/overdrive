//! Workload-cgroup management — creates and tears down
//! `overdrive.slice/workloads.slice/<alloc_id>.scope` directories,
//! writes `cpu.weight` / `memory.max` per ADR-0026, and removes the
//! scope after process reap.
//!
//! Five filesystem operations per workload lifecycle, no `cgroups-rs`
//! dep (ADR-0026 D6).
//!
//! # Status — RED scaffold
//!
//! Phase: phase-1-first-workload, slice 2 (US-02). Wave: DISTILL.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// SCAFFOLD marker.
pub const SCAFFOLD: bool = true;

// ---------------------------------------------------------------------------
// CgroupPath newtype
// ---------------------------------------------------------------------------

/// Canonical cgroup-path newtype for workload scopes. STRICT-newtype
/// per `.claude/rules/development.md` § Newtype completeness:
/// validating `FromStr`, `Display` matching the canonical string form,
/// `Serialize` / `Deserialize` round-tripping through the same
/// canonical form, and rkyv-archive support (added by DELIVER once
/// the on-disk shape lands; deferred for the RED scaffold).
///
/// Canonical form: `overdrive.slice/workloads.slice/<alloc_id>.scope`
/// for workload scopes. The newtype rejects path-traversal characters
/// (`..`, `/.../`, leading `/`) at construction time.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CgroupPath(String);

impl CgroupPath {
    /// Canonical string form.
    ///
    /// # Panics
    ///
    /// Phase 1 first-workload DISTILL — RED scaffold.
    #[must_use]
    pub fn as_str(&self) -> &str {
        panic!("Not yet implemented -- RED scaffold")
    }
}

impl fmt::Display for CgroupPath {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("Not yet implemented -- RED scaffold")
    }
}

impl FromStr for CgroupPath {
    type Err = CgroupPathError;

    fn from_str(_raw: &str) -> Result<Self, Self::Err> {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// Errors from [`CgroupPath::from_str`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CgroupPathError {
    /// Empty input.
    #[error("empty cgroup path")]
    Empty,
    /// Input contains a path-traversal sequence (`..`, leading `/`,
    /// double slashes).
    #[error("invalid cgroup path: {raw}")]
    InvalidPath {
        /// Echo of the rejected input for diagnostics.
        raw: String,
    },
}

// ---------------------------------------------------------------------------
// Workload-cgroup management entrypoints
// ---------------------------------------------------------------------------

/// Create the workload scope directory at the given path.
/// `mkdir -p` semantics; idempotent on directory already existing.
///
/// Per ADR-0026 D6: direct cgroupfs writes via `std::fs`.
///
/// # Errors
///
/// Returns an error if the cgroupfs is not mounted, the parent slice
/// does not exist, or the running UID lacks delegation.
///
/// # Panics
///
/// RED scaffold.
pub fn create_workload_scope(_scope: &CgroupPath) -> Result<(), std::io::Error> {
    panic!("Not yet implemented -- RED scaffold")
}

/// Place a process PID into the workload scope's `cgroup.procs` file.
///
/// # Panics
///
/// RED scaffold.
pub fn place_pid_in_scope(_scope: &CgroupPath, _pid: u32) -> Result<(), std::io::Error> {
    panic!("Not yet implemented -- RED scaffold")
}

/// Write `cpu.weight` and `memory.max` for the given scope, derived
/// from `AllocationSpec::resources` per ADR-0026 D9.
///
/// `cpu.weight = clamp(cpu_milli / 10, 1, 10000)` (proportional share).
/// `memory.max = memory_bytes` (hard cap).
///
/// On failure, the caller `tracing::warn!`s and continues per ADR-0026
/// warn-and-continue disposition.
///
/// # Panics
///
/// RED scaffold.
pub fn write_resource_limits(
    _scope: &CgroupPath,
    _resources: &overdrive_core::traits::driver::Resources,
) -> Result<(), std::io::Error> {
    panic!("Not yet implemented -- RED scaffold")
}

/// Remove the workload scope directory after process reap.
/// Idempotent — succeeds when the directory is already gone.
///
/// # Panics
///
/// RED scaffold.
pub fn remove_workload_scope(_scope: &CgroupPath) -> Result<(), std::io::Error> {
    panic!("Not yet implemented -- RED scaffold")
}
