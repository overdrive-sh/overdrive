//! Workload-cgroup management for `ExecDriver`.
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
/// Uses `tokio::fs::create_dir_all` per `.claude/rules/development.md`
/// § Concurrency & async — sync `std::fs::*` is forbidden inside
/// `async fn` in adapter-host crates and the dst-lint gate enforces
/// it at PR time.
///
/// # Errors
///
/// Returns an error if the cgroupfs is not mounted, the parent slice
/// does not exist (when not creating recursively), or the running UID
/// lacks delegation.
pub async fn create_workload_scope(root: &Path, scope: &CgroupPath) -> Result<(), std::io::Error> {
    let dir = scope.resolve(root);
    tokio::fs::create_dir_all(&dir).await
}

/// Place a process PID into the workload scope's `cgroup.procs` file.
///
/// # Errors
///
/// Returns an error if the scope's `cgroup.procs` cannot be written.
pub async fn place_pid_in_scope(
    root: &Path,
    scope: &CgroupPath,
    pid: u32,
) -> Result<(), std::io::Error> {
    let path = scope.resolve(root).join("cgroup.procs");
    tokio::fs::write(&path, format!("{pid}\n")).await
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
pub async fn write_resource_limits(
    root: &Path,
    scope: &CgroupPath,
    resources: &Resources,
) -> Result<(), std::io::Error> {
    let dir = scope.resolve(root);
    let weight = cpu_weight_for(resources.cpu_milli);
    tokio::fs::write(dir.join("cpu.weight"), format!("{weight}\n")).await?;
    tokio::fs::write(dir.join("memory.max"), format!("{}\n", resources.memory_bytes)).await?;
    Ok(())
}

/// Wrapper for `write_resource_limits` that converts a write error
/// into a structured warning log AND returns `Ok(())` to the caller
/// per ADR-0026 D9 warn-and-continue disposition.
pub async fn write_resource_limits_warn_on_error(
    root: &Path,
    scope: &CgroupPath,
    resources: &Resources,
) {
    if let Err(err) = write_resource_limits(root, scope, resources).await {
        warn!(
            scope = %scope,
            error = %err,
            "cgroup resource-limit write failed; continuing per ADR-0026 D9"
        );
    }
}

/// Mass-kill every process in the workload cgroup.
///
/// Uses the kernel's `cgroup.kill` interface (cgroup v2, kernel 5.14+)
/// — writes `1\n` to `<scope>/cgroup.kill`, which atomically delivers
/// SIGKILL to every task in the cgroup including grandchildren that
/// escaped the driver's tokio `Child` handle (e.g. `/bin/sh -c '...'`
/// shells whose `sleep` child reparents to init when the shell dies).
///
/// Idempotent — `NotFound` (scope already gone) is reported as `Ok`.
/// Invalid-argument writes (a path that exists but is not a v2 cgroup)
/// surface to the caller; production wires `/sys/fs/cgroup` so this
/// code path is the happy path on Lima / LVH / production hosts alike.
///
/// # Errors
///
/// Returns the underlying io error if the write fails for a reason
/// other than the scope being absent. The caller is expected to
/// `tracing::warn!` and continue — terminal cleanup is best-effort by
/// design.
pub async fn cgroup_kill(root: &Path, scope: &CgroupPath) -> Result<(), std::io::Error> {
    let path = scope.resolve(root).join("cgroup.kill");
    match tokio::fs::write(&path, "1\n").await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Remove the workload scope directory after process reap.
/// Idempotent — succeeds when the directory is already gone.
///
/// On real cgroupfs the directory looks empty to userspace because
/// the kernel-managed virtual files (`cgroup.procs`, `cpu.weight`,
/// `memory.max`, ...) cannot be `unlink`ed individually — they are
/// reaped automatically by `rmdir`. On a non-cgroupfs (the integration
/// tests use a `tempfile::TempDir`) those files are real on-disk
/// entries and `rmdir` returns `ENOTEMPTY`. To make production code
/// portable across both, fall back to `remove_dir_all` on
/// `ENOTEMPTY` so the test-fixture path also succeeds.
///
/// # Errors
///
/// Returns the underlying io error if neither `rmdir` nor the
/// `remove_dir_all` fallback succeeds.
pub async fn remove_workload_scope(root: &Path, scope: &CgroupPath) -> Result<(), std::io::Error> {
    let dir = scope.resolve(root);
    match tokio::fs::remove_dir(&dir).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) if is_dir_not_empty(&err) => {
            // Tempdir-as-cgroupfs path (tests). Real cgroupfs never
            // returns ENOTEMPTY for a workload scope because the
            // virtual files are reaped on `rmdir`. Single-writer
            // ownership through `ExecDriver`'s `live` mutex serialises
            // start/stop for a given allocation, so the inner
            // `remove_dir_all` cannot race with another caller of
            // `remove_workload_scope` for the same scope; a NotFound
            // here would imply an out-of-band remover that the
            // architecture forbids. Surface the error rather than
            // swallow it.
            tokio::fs::remove_dir_all(&dir).await
        }
        Err(err) => Err(err),
    }
}

/// Detect the `ENOTEMPTY` `io::Error` from `remove_dir`. Rust's
/// `io::ErrorKind::DirectoryNotEmpty` is stable in 1.83+; fall back
/// to the raw OS error code for portability against older toolchains.
fn is_dir_not_empty(err: &std::io::Error) -> bool {
    // The stable kind name (Rust 1.83+).
    if format!("{:?}", err.kind()) == "DirectoryNotEmpty" {
        return true;
    }
    // Linux: ENOTEMPTY = 39. The OS error survives any libc surface.
    err.raw_os_error() == Some(39)
}

// ---------------------------------------------------------------------------
// Unit tests — pure-logic helpers (no real cgroupfs)
// ---------------------------------------------------------------------------

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

    /// `is_dir_not_empty` returns true for `ENOTEMPTY` (raw os
    /// error 39 on Linux, or the named `DirectoryNotEmpty` kind in
    /// Rust 1.83+) and false for other error kinds. Pin both
    /// branches and the negative case — kills the four mutations
    /// on this function (body→true / body→false / two `==` flips).
    #[test]
    fn is_dir_not_empty_recognises_directory_not_empty_kind() {
        let err = std::io::Error::from(std::io::ErrorKind::DirectoryNotEmpty);
        assert!(is_dir_not_empty(&err), "DirectoryNotEmpty kind must be recognised");
    }

    #[test]
    fn is_dir_not_empty_recognises_raw_enotempty_os_error() {
        let err = std::io::Error::from_raw_os_error(39);
        assert!(is_dir_not_empty(&err), "raw ENOTEMPTY (39) must be recognised");
    }

    #[test]
    fn is_dir_not_empty_rejects_unrelated_errors() {
        let err = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(!is_dir_not_empty(&err), "NotFound must not be recognised as ENOTEMPTY");
        let err = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(!is_dir_not_empty(&err), "PermissionDenied must not be recognised as ENOTEMPTY");
    }

    /// `cgroup_kill` is idempotent on `NotFound` (the scope is gone
    /// or never existed). Pins the match-guard mutations on the
    /// `err.kind() == NotFound` branch.
    #[tokio::test]
    async fn cgroup_kill_is_idempotent_on_missing_scope() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let scope_path = "overdrive.slice/workloads.slice/alloc-missing-0.scope";
        let scope = CgroupPath::from_str(scope_path).expect("valid CgroupPath");

        let result = cgroup_kill(tmp.path(), &scope).await;
        assert!(
            result.is_ok(),
            "cgroup_kill on a missing scope must be idempotent (Ok); got {result:?}",
        );
    }

    /// `cgroup_kill` writes `1\n` to `<scope>/cgroup.kill` on the
    /// happy path. Pins the body-replace mutation (`-> Ok(())` skips
    /// the write entirely; `cgroup.kill` would not appear on disk).
    #[tokio::test]
    async fn cgroup_kill_writes_one_to_cgroup_kill_file() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let scope_path = "overdrive.slice/workloads.slice/alloc-kill-write-0.scope";
        let scope = CgroupPath::from_str(scope_path).expect("valid CgroupPath");
        let scope_dir = scope.resolve(tmp.path());
        std::fs::create_dir_all(&scope_dir).expect("create scope dir");

        cgroup_kill(tmp.path(), &scope).await.expect("cgroup_kill on real dir succeeds");

        let written = std::fs::read_to_string(scope_dir.join("cgroup.kill"))
            .expect("cgroup.kill must be written");
        assert_eq!(
            written, "1\n",
            "cgroup_kill must write '1\\n' to cgroup.kill (kernel cgroup.kill protocol)",
        );
    }

    /// `cgroup_kill` propagates non-`NotFound` errors rather than
    /// swallowing them. Pins the match-guard mutation that flips
    /// `err.kind() == NotFound` to `true` (which would route every
    /// error through the idempotent arm).
    ///
    /// Setup creates a regular file at the *scope* path; writing
    /// `<file>/cgroup.kill` then fails with a non-`NotFound` error
    /// (typically `NotADirectory` / `ENOTDIR`). The unmutated guard
    /// propagates; the `-> true` mutant turns it into `Ok(())`.
    #[tokio::test]
    async fn cgroup_kill_propagates_non_notfound_errors() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let scope_path = "overdrive.slice/workloads.slice/alloc-blocker-0.scope";
        let scope = CgroupPath::from_str(scope_path).expect("valid CgroupPath");
        let scope_as_path = scope.resolve(tmp.path());
        if let Some(parent) = scope_as_path.parent() {
            std::fs::create_dir_all(parent).expect("create parent slice dirs");
        }
        // Place a regular file where the scope DIR would be — writing
        // `<file>/cgroup.kill` produces a non-NotFound error.
        std::fs::write(&scope_as_path, b"blocker").expect("write blocker file");

        let result = cgroup_kill(tmp.path(), &scope).await;
        let err = result.expect_err("non-NotFound errors must propagate");
        assert_ne!(
            err.kind(),
            std::io::ErrorKind::NotFound,
            "the test setup must NOT produce NotFound (would render the test vacuous)",
        );
    }

    /// `remove_workload_scope` is idempotent on `NotFound`. Pins
    /// the outer match-guard mutations.
    #[tokio::test]
    async fn remove_workload_scope_is_idempotent_on_missing_scope() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let scope_path = "overdrive.slice/workloads.slice/alloc-missing-1.scope";
        let scope = CgroupPath::from_str(scope_path).expect("valid CgroupPath");

        let result = remove_workload_scope(tmp.path(), &scope).await;
        assert!(
            result.is_ok(),
            "remove_workload_scope on missing scope must be idempotent; got {result:?}",
        );
    }

    /// `remove_workload_scope` falls back to `remove_dir_all` on
    /// `ENOTEMPTY` (the tempdir-as-cgroupfs path). Pins the
    /// `is_dir_not_empty(&err)` match-guard mutation.
    #[tokio::test]
    async fn remove_workload_scope_falls_back_to_remove_dir_all_on_enotempty() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let scope_path = "overdrive.slice/workloads.slice/alloc-non-empty-0.scope";
        let scope = CgroupPath::from_str(scope_path).expect("valid CgroupPath");
        let scope_dir = scope.resolve(tmp.path());
        std::fs::create_dir_all(&scope_dir).expect("create scope dir");
        std::fs::write(scope_dir.join("cgroup.procs"), "").expect("write fake cgroup.procs");
        assert!(scope_dir.exists(), "scope dir must exist before removal");

        let result = remove_workload_scope(tmp.path(), &scope).await;
        assert!(
            result.is_ok(),
            "remove_workload_scope must succeed via the ENOTEMPTY fallback; got {result:?}",
        );
        assert!(
            !scope_dir.exists(),
            "scope dir must be gone after remove_workload_scope's fallback path",
        );
    }

    /// `remove_workload_scope` propagates non-`ENOTEMPTY`
    /// non-`NotFound` errors from the outer `remove_dir` rather than
    /// routing them through the `remove_dir_all` fallback. Pins the
    /// `is_dir_not_empty(&err)` match-guard mutation that would flip
    /// to `true` and incorrectly mask a hard error as a transient
    /// ENOTEMPTY.
    ///
    /// Setup creates a SYMLINK at the scope path pointing to a real
    /// directory elsewhere in the tempdir. On Linux:
    ///
    ///   * `remove_dir(symlink_to_dir)` returns `NotADirectory` —
    ///     `lstat(2)` resolves the symlink and `rmdir(2)` rejects
    ///     a non-directory inode.
    ///   * `remove_dir_all(symlink_to_dir)` returns `Ok(())` —
    ///     standard library follows the symlink, finds an empty
    ///     directory, and removes the link itself.
    ///
    /// The two functions producing different observable outcomes on
    /// the same path is exactly what makes mutant 3 killable: the
    /// unmutated guard returns `Err(NotADirectory)`; the `-> true`
    /// mutant routes through `remove_dir_all` and returns `Ok(())`.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn remove_workload_scope_propagates_non_enotempty_non_notfound_errors() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let scope_path = "overdrive.slice/workloads.slice/alloc-symlink-1.scope";
        let scope = CgroupPath::from_str(scope_path).expect("valid CgroupPath");
        let scope_link = scope.resolve(tmp.path());
        if let Some(parent) = scope_link.parent() {
            std::fs::create_dir_all(parent).expect("create parent slice dirs");
        }
        // The symlink target — a real, empty directory elsewhere in
        // the tempdir.
        let target_dir = tmp.path().join("symlink-target");
        std::fs::create_dir_all(&target_dir).expect("create symlink target dir");
        symlink(&target_dir, &scope_link).expect("create symlink at scope path");

        let result = remove_workload_scope(tmp.path(), &scope).await;
        let err = result.expect_err(
            "remove_workload_scope on a symlink-to-dir must propagate the \
             outer remove_dir error (NotADirectory) without falling back to \
             remove_dir_all; the `is_dir_not_empty -> true` mutation diverges \
             by calling remove_dir_all on the symlink, which succeeds",
        );
        assert_ne!(
            err.kind(),
            std::io::ErrorKind::NotFound,
            "the test setup must NOT produce NotFound (would render the test vacuous)",
        );
    }

    /// `create_workload_scope` writes a directory. Kills body→Ok(())
    /// — the mutant skips `create_dir_all`, so the directory does
    /// NOT appear on disk; the assertion catches the missing dir.
    #[tokio::test]
    async fn create_workload_scope_writes_a_real_directory() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let scope_path = "overdrive.slice/workloads.slice/alloc-create-0.scope";
        let scope = CgroupPath::from_str(scope_path).expect("valid CgroupPath");

        let result = create_workload_scope(tmp.path(), &scope).await;
        assert!(result.is_ok(), "create_workload_scope must succeed; got {result:?}");
        let scope_dir = scope.resolve(tmp.path());
        assert!(scope_dir.exists(), "scope dir must exist on disk after create");
        assert!(scope_dir.is_dir(), "scope path must be a directory");
    }

    /// `place_pid_in_scope` writes the pid to `cgroup.procs`. Kills
    /// body→Ok(()).
    #[tokio::test]
    async fn place_pid_in_scope_writes_pid_to_cgroup_procs() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let scope_path = "overdrive.slice/workloads.slice/alloc-place-0.scope";
        let scope = CgroupPath::from_str(scope_path).expect("valid CgroupPath");
        std::fs::create_dir_all(scope.resolve(tmp.path())).expect("create scope dir");

        let result = place_pid_in_scope(tmp.path(), &scope, 1234).await;
        assert!(result.is_ok(), "place_pid_in_scope must succeed; got {result:?}");

        let procs = std::fs::read_to_string(scope.resolve(tmp.path()).join("cgroup.procs"))
            .expect("read cgroup.procs");
        assert_eq!(procs, "1234\n", "cgroup.procs must contain the pid + newline");
    }

    /// `write_resource_limits` writes cpu.weight and memory.max.
    /// Kills body→Ok(()) and pins the cpu_weight_for delegation.
    #[tokio::test]
    async fn write_resource_limits_writes_cpu_weight_and_memory_max() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let scope_path = "overdrive.slice/workloads.slice/alloc-limits-0.scope";
        let scope = CgroupPath::from_str(scope_path).expect("valid CgroupPath");
        std::fs::create_dir_all(scope.resolve(tmp.path())).expect("create scope dir");

        let resources = Resources { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 };
        let result = write_resource_limits(tmp.path(), &scope, &resources).await;
        assert!(result.is_ok(), "write_resource_limits must succeed; got {result:?}");

        let weight = std::fs::read_to_string(scope.resolve(tmp.path()).join("cpu.weight"))
            .expect("read cpu.weight");
        assert_eq!(weight, "10\n", "cpu.weight must be cpu_milli/10 = 10");

        let memmax = std::fs::read_to_string(scope.resolve(tmp.path()).join("memory.max"))
            .expect("read memory.max");
        assert_eq!(
            memmax,
            format!("{}\n", 256 * 1024 * 1024),
            "memory.max must equal memory_bytes",
        );
    }

    /// `write_resource_limits_warn_on_error` returns `()` and only
    /// warns on failure. Pins body→() (the mutant returns nothing
    /// either, but production also writes side effects on success;
    /// the mutant skips the call entirely → cpu.weight does NOT
    /// appear on disk). The assertion catches the missing file.
    #[tokio::test]
    async fn write_resource_limits_warn_on_error_writes_files_on_success() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let scope_path = "overdrive.slice/workloads.slice/alloc-warn-ok-0.scope";
        let scope = CgroupPath::from_str(scope_path).expect("valid CgroupPath");
        std::fs::create_dir_all(scope.resolve(tmp.path())).expect("create scope dir");

        let resources = Resources { cpu_milli: 200, memory_bytes: 1024 * 1024 };
        write_resource_limits_warn_on_error(tmp.path(), &scope, &resources).await;

        assert!(
            scope.resolve(tmp.path()).join("cpu.weight").exists(),
            "cpu.weight must be written on the happy path",
        );
        assert!(
            scope.resolve(tmp.path()).join("memory.max").exists(),
            "memory.max must be written on the happy path",
        );
    }
}
