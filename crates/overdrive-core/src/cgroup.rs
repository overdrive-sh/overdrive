//! [`CgroupPath`] — the validated relative path of a workload cgroup.
//!
//! Relocated VERBATIM from `overdrive_worker::cgroup_manager` into
//! `overdrive-core` per ADR-0082 §D2 (Amendment 2026-08-12, gap 1,
//! GH #42): `VmConfig` (`crate::vm::config`) needs this type as a field,
//! and `overdrive-core` (`core` class) cannot depend on
//! `overdrive-worker` (`adapter-host` class) — the dependency graph runs
//! the other way. The type, its derives, its error type, and its
//! `for_alloc` / `as_str` / `resolve` / `Display` / `FromStr` / `TryFrom`
//! surface are UNCHANGED by the move. `overdrive_worker::cgroup_manager`
//! re-exports [`CgroupPath`] / [`CgroupPathError`] so its existing call
//! sites keep resolving with no changes.
//!
//! rkyv layout is structural (`struct CgroupPath(String)` — one field),
//! so the relocation is byte-compatible for any already-persisted value.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::AllocationId;

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

// Relocated VERBATIM from `overdrive_worker::cgroup_manager` alongside
// the type itself — the only `CgroupPath`-specific test in the prior
// module; `CgroupManager` / `WorkloadsBootstrapError` tests stay behind
// in `overdrive-worker` (they test the manager, not this value type).
#[cfg(test)]
#[allow(clippy::expect_used, clippy::doc_markdown)]
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
}
