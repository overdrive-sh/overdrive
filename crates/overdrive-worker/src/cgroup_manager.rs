//! Workload-cgroup management for `ProcessDriver`.
//!
//! Creates and tears down
//! `overdrive.slice/workloads.slice/<alloc_id>.scope` directories,
//! writes `cpu.weight` / `memory.max` per ADR-0026, and removes the
//! scope after process reap. Five filesystem operations per workload
//! lifecycle, no `cgroups-rs` dep (ADR-0026 D6).

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use overdrive_core::id::AllocationId;
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

    /// Resolve under a cgroupfs root (`/sys/fs/cgroup`, or a tempdir
    /// for tests).
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

/// Create the workload scope directory under `root`.
/// `mkdir -p` semantics; idempotent on directory already existing.
///
/// # Errors
///
/// Returns an error if the cgroupfs is not mounted, the parent slice
/// does not exist (when not creating recursively), or the running UID
/// lacks delegation.
pub fn create_workload_scope(root: &Path, scope: &CgroupPath) -> Result<(), std::io::Error> {
    let dir = scope.resolve(root);
    std::fs::create_dir_all(&dir)
}

/// Place a process PID into the workload scope's `cgroup.procs` file.
///
/// # Errors
///
/// Returns an error if the scope's `cgroup.procs` cannot be written.
pub fn place_pid_in_scope(
    root: &Path,
    scope: &CgroupPath,
    pid: u32,
) -> Result<(), std::io::Error> {
    let path = scope.resolve(root).join("cgroup.procs");
    std::fs::write(&path, format!("{pid}\n"))
}

/// Compute `cpu.weight` from `cpu_milli` per ADR-0026 D9:
/// `clamp(cpu_milli / 10, 1, 10000)`.
#[must_use]
pub fn cpu_weight_for(cpu_milli: u32) -> u32 {
    (cpu_milli / 10).clamp(1, 10_000)
}

/// Write `cpu.weight` and `memory.max` for the given scope, derived
/// from `Resources` per ADR-0026 D9.
///
/// On failure, the caller `tracing::warn!`s and continues per ADR-0026
/// D9 warn-and-continue disposition. This helper itself surfaces the
/// io error to the caller so the caller can decide.
///
/// # Errors
///
/// Returns the underlying io error if either limit file cannot be
/// written.
pub fn write_resource_limits(
    root: &Path,
    scope: &CgroupPath,
    resources: &Resources,
) -> Result<(), std::io::Error> {
    let dir = scope.resolve(root);
    let weight = cpu_weight_for(resources.cpu_milli);
    std::fs::write(dir.join("cpu.weight"), format!("{weight}\n"))?;
    std::fs::write(dir.join("memory.max"), format!("{}\n", resources.memory_bytes))?;
    Ok(())
}

/// Wrapper for `write_resource_limits` that converts a write error
/// into a structured warning log AND returns `Ok(())` to the caller
/// per ADR-0026 D9 warn-and-continue disposition.
pub fn write_resource_limits_warn_on_error(
    root: &Path,
    scope: &CgroupPath,
    resources: &Resources,
) {
    if let Err(err) = write_resource_limits(root, scope, resources) {
        warn!(
            scope = %scope,
            error = %err,
            "cgroup resource-limit write failed; continuing per ADR-0026 D9"
        );
    }
}

/// Remove the workload scope directory after process reap.
/// Idempotent — succeeds when the directory is already gone.
///
/// # Errors
///
/// Returns the underlying io error if `rmdir` fails for a reason
/// other than `NotFound`.
pub fn remove_workload_scope(
    root: &Path,
    scope: &CgroupPath,
) -> Result<(), std::io::Error> {
    let dir = scope.resolve(root);
    match std::fs::remove_dir(&dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}
