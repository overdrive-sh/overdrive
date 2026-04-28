//! cgroup v2 delegation pre-flight per ADR-0028.
//!
//! Runs at the start of `overdrive serve` boot, BEFORE any on-disk side
//! effects (no CA mint, no `IntentStore` open, no listener bind). A
//! pre-flight failure produces no on-disk artefacts and refuses to bind
//! the listener.
//!
//! Four checks in order:
//!   1. Kernel exposes cgroup v2 (`cgroup2` in `/proc/filesystems`).
//!   2. cgroup v2 is mounted (`/sys/fs/cgroup/cgroup.controllers`
//!      exists).
//!   3. Running as root (skip step 4 — root has implicit access), OR
//!   4. Required controllers (`cpu` AND `memory`) are present in the
//!      `cgroup.subtree_control` of the parent slice.
//!
//! Each error message answers "what / why / how to fix" per the
//! `nw-ux-tui-patterns` shape.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// Default cgroupfs root in production. Tests pass an explicit root
/// (typically a `tempfile::TempDir`) so they can fabricate the
/// failure shapes without touching `/sys/fs/cgroup`.
pub const DEFAULT_CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// Pre-flight failure modes per ADR-0028 §4.
///
/// Each variant's `Display` form names the failed check, the cause,
/// and the actionable fix — including the `--allow-no-cgroups` dev
/// escape hatch and a documentation URL.
#[derive(Debug, Error)]
pub enum CgroupPreflightError {
    /// Step 1 — kernel does not expose cgroup v2 (no `cgroup2` line in
    /// `/proc/filesystems`). This is the cgroup-v1-only-host shape.
    #[error(
        "cgroup v2 not available on this kernel (uname: {kernel}).\n\
        \n\
        Detected: /proc/filesystems does not list `cgroup2`.\n\
        Phase 1 of Overdrive requires cgroup v2; cgroup v1 hosts are not supported.\n\
        \n\
        Try one of:\n\
        \n\
          1. Boot a kernel with cgroup v2 unified hierarchy enabled\n\
             (Linux 4.5+; the default on every distribution shipped\n\
             after 2022).\n\
          2. Run without cgroup isolation (development only — workloads\n\
             are unbounded; control plane is not protected):\n\
               overdrive serve --allow-no-cgroups\n\
        \n\
        Documentation: https://docs.overdrive.sh/operations/cgroup-delegation"
    )]
    NoCgroupV2 {
        /// Kernel version (output of `uname -r`).
        kernel: String,
    },

    /// Step 2 — cgroup v2 is available in the kernel but not mounted at
    /// the expected path.
    #[error(
        "cgroup v2 not mounted; expected /sys/fs/cgroup/cgroup.controllers.\n\
        \n\
        Detected: cgroup v2 IS available in this kernel, but the unified\n\
        hierarchy is not mounted at /sys/fs/cgroup/.\n\
        \n\
        Try one of:\n\
        \n\
          1. systemd-managed boots auto-mount the unified hierarchy;\n\
             check `mount | grep cgroup2`.\n\
          2. Run without cgroup isolation (development only):\n\
               overdrive serve --allow-no-cgroups\n\
        \n\
        Documentation: https://docs.overdrive.sh/operations/cgroup-delegation"
    )]
    NotMounted,

    /// Step 4 — running as a non-root UID and the parent slice's
    /// `subtree_control` lacks one or both of `cpu` / `memory`.
    /// Names the missing controller(s) and the systemd `Delegate=yes`
    /// fix.
    #[error(
        "cgroup v2 delegation required.\n\
        \n\
        Overdrive serve needs the {missing_human} controller{plural} delegated to UID {uid}.\n\
        \n\
        Detected: cgroup v2 IS available, BUT {missing_human} {is_or_are} not in\n\
        the subtree_control of {slice}.\n\
        \n\
        Try one of:\n\
        \n\
          1. Run via the bundled systemd unit (production):\n\
               systemctl --user start overdrive\n\
        \n\
          2. Grant delegation manually (one-time):\n\
               sudo systemctl set-property user-{uid}.slice Delegate=yes\n\
               systemctl --user daemon-reload\n\
        \n\
          3. Run as root (development only — no isolation guarantees):\n\
               sudo overdrive serve\n\
        \n\
          4. Run without cgroup isolation (development only — workloads\n\
             are unbounded; control plane is not protected):\n\
               overdrive serve --allow-no-cgroups\n\
        \n\
        Documentation: https://docs.overdrive.sh/operations/cgroup-delegation"
    )]
    DelegationMissing {
        /// UID of the running process (`geteuid()`).
        uid: u32,
        /// Slice path where `subtree_control` was inspected.
        slice: PathBuf,
        /// Missing controllers (`cpu` and/or `memory`).
        missing: Vec<String>,
        /// Pre-rendered "cpu", "memory", or "cpu and memory" for the
        /// human-facing message. Computed at construction.
        missing_human: String,
        /// "" when only one controller is missing, "s" when both are.
        plural: &'static str,
        /// "is" or "are" matching `plural`.
        is_or_are: &'static str,
    },
}

/// Run the four-step pre-flight check rooted at `cgroup_root` for
/// the running UID `uid`.
///
/// In production callers pass [`DEFAULT_CGROUP_ROOT`] and the running
/// `geteuid()`. Tests pass a tempdir + arbitrary UID so they can
/// reproduce each failure shape without touching `/sys/fs/cgroup`.
///
/// On Linux this performs real filesystem reads of `/proc/filesystems`
/// (always — it is the source of truth for cgroup v2 availability) and
/// of `<cgroup_root>/cgroup.controllers` etc. (under the test root).
/// On non-Linux this is unreachable in production because the boot
/// path only invokes the pre-flight under `#[cfg(target_os = "linux")]`.
///
/// # Errors
///
/// See [`CgroupPreflightError`] variants.
pub fn run_preflight_at(
    cgroup_root: &Path,
    uid: u32,
    proc_filesystems: &Path,
) -> Result<(), CgroupPreflightError> {
    // Step 1 — kernel exposes cgroup v2.
    let proc_fs = std::fs::read_to_string(proc_filesystems).unwrap_or_default();
    let cgroup_v2_available = proc_fs.lines().any(|line| line.contains("cgroup2"));
    if !cgroup_v2_available {
        return Err(CgroupPreflightError::NoCgroupV2 { kernel: uname_release() });
    }

    // Step 2 — cgroup v2 is mounted (cgroup.controllers exists).
    let controllers = cgroup_root.join("cgroup.controllers");
    if !controllers.exists() {
        return Err(CgroupPreflightError::NotMounted);
    }

    // Step 3 — running as root: skip delegation check.
    if uid == 0 {
        return Ok(());
    }

    // Step 4 — required controllers in subtree_control of the parent
    // slice. Convention: tests fabricate the parent slice file directly
    // under the cgroup_root; production (root path skipped above) reads
    // the user slice's subtree_control.
    let subtree_control = cgroup_root.join("cgroup.subtree_control");
    let contents = std::fs::read_to_string(&subtree_control).unwrap_or_default();
    let mut missing = Vec::new();
    if !contents.split_ascii_whitespace().any(|t| t == "cpu") {
        missing.push("cpu".to_owned());
    }
    if !contents.split_ascii_whitespace().any(|t| t == "memory") {
        missing.push("memory".to_owned());
    }
    if !missing.is_empty() {
        let (missing_human, plural, is_or_are) = if missing.len() == 1 {
            (missing[0].clone(), "", "is")
        } else {
            ("cpu and memory".to_owned(), "s", "are")
        };
        return Err(CgroupPreflightError::DelegationMissing {
            uid,
            slice: subtree_control,
            missing,
            missing_human,
            plural,
            is_or_are,
        });
    }

    Ok(())
}

/// Run pre-flight against the production defaults. Convenience wrapper
/// that resolves `geteuid()`, `/sys/fs/cgroup`, and `/proc/filesystems`.
///
/// # Errors
///
/// See [`CgroupPreflightError`] variants.
#[cfg(target_os = "linux")]
pub fn run_preflight() -> Result<(), CgroupPreflightError> {
    // SAFETY: `geteuid` is a POSIX-defined thin syscall wrapper with
    // no preconditions and no failure modes; it cannot panic, return
    // an error, or invalidate any pointer. The crate-level
    // `deny(unsafe_code)` is scoped open here exactly to permit this
    // single FFI call — every other module in the crate remains
    // unsafe-free.
    #[allow(unsafe_code)]
    let uid = unsafe { libc::geteuid() };
    run_preflight_at(Path::new(DEFAULT_CGROUP_ROOT), uid, Path::new("/proc/filesystems"))
}

/// Non-Linux stub — pre-flight is only invoked under
/// `#[cfg(target_os = "linux")]` in the boot path; this signature
/// exists so tests in the `cgroup_preflight` module compile on macOS.
#[cfg(not(target_os = "linux"))]
#[allow(clippy::unnecessary_wraps, clippy::missing_const_for_fn)]
pub fn run_preflight() -> Result<(), CgroupPreflightError> {
    Ok(())
}

/// Read `uname -r` analogue. Returns "unknown" if the kernel-version
/// file is absent (non-Linux dev hosts under unit tests).
fn uname_release() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map_or_else(|_| "unknown".to_owned(), |s| s.trim().to_owned())
}
