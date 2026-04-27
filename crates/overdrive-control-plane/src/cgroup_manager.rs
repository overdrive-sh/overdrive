//! Control-plane cgroup management — pre-flight check + slice-creation
//! at server boot. Per ADR-0028 (hard refusal on missing delegation,
//! `--allow-no-cgroups` dev escape hatch) and ADR-0026 (cgroup v2
//! ONLY, direct cgroupfs writes).
//!
//! The workload half of the cgroup hierarchy
//! (`overdrive.slice/workloads.slice/`) is owned by `overdrive-worker`;
//! this module owns ONLY the control-plane half
//! (`overdrive.slice/control-plane.slice/`).
//!
//! # Status — RED scaffold
//!
//! Phase: phase-1-first-workload, slice 4 (US-04).
//! Wave: DISTILL. SCAFFOLD: true — every entrypoint panics.

use thiserror::Error;

/// SCAFFOLD marker.
pub const SCAFFOLD: bool = true;

// ---------------------------------------------------------------------------
// Pre-flight check
// ---------------------------------------------------------------------------

/// Run the cgroup v2 delegation pre-flight check at server boot per
/// ADR-0028.
///
/// Performs four checks in order:
/// 1. Kernel exposes cgroup v2 (`cgroup2` in `/proc/filesystems`).
/// 2. cgroup v2 is mounted (`/sys/fs/cgroup/cgroup.controllers`
///    exists).
/// 3. Running as root, OR delegation is granted to the running UID.
/// 4. Required controllers (`cpu` + `memory`) are in
///    `cgroup.subtree_control`.
///
/// Returns `Ok(())` on full success. Any failure produces a typed
/// `CgroupPreflightError` whose Display string answers
/// "what / why / how to fix" per the `nw-ux-tui-patterns` shape.
///
/// # Errors
///
/// See [`CgroupPreflightError`] variants.
///
/// # Panics
///
/// RED scaffold.
pub fn run_preflight() -> Result<(), CgroupPreflightError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// Pre-flight failure modes per ADR-0028 §4.
#[derive(Debug, Error)]
pub enum CgroupPreflightError {
    /// Kernel does not expose cgroup v2 (no `cgroup2` in
    /// `/proc/filesystems`). Names the kernel version observed and
    /// the minimum-supported kernel doc.
    #[error(
        "cgroup v2 not available on this kernel (uname: {kernel}); see \
         https://docs.overdrive.sh/operations/cgroup-delegation"
    )]
    NoCgroupV2 {
        /// Kernel version (output of `uname -r`).
        kernel: String,
    },
    /// cgroup v2 exists but is not mounted at the expected path.
    #[error(
        "cgroup v2 not mounted; expected /sys/fs/cgroup/cgroup.controllers; \
         try mounting via systemd-cgroup-mount"
    )]
    NotMounted,
    /// Running as a non-root user, and `subtree_control` lacks one or
    /// both of `cpu` / `memory`. Names the missing controller(s) and
    /// the systemd `Delegate=yes` fix.
    #[error(
        "controllers {missing:?} not delegated to UID {uid}; \
         try: sudo systemctl set-property user-{uid}.slice Delegate=yes; \
         or run with --allow-no-cgroups (dev only)"
    )]
    DelegationMissing {
        /// UID of the running process (`geteuid()`).
        uid: u32,
        /// Missing controllers (`cpu` and/or `memory`).
        missing: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// Control-plane slice creation + enrolment
// ---------------------------------------------------------------------------

/// Create `overdrive.slice/control-plane.slice/` (idempotent on
/// directory already existing) and enrol the running process into it
/// by writing its PID to `cgroup.procs`.
///
/// Per ADR-0028 §2, this runs after pre-flight but before listener
/// bind. A failure here aborts boot (no on-disk side effects survive).
///
/// # Errors
///
/// Returns an error if the slice cannot be created or the PID write
/// fails.
///
/// # Panics
///
/// RED scaffold.
pub fn create_and_enrol_control_plane_slice() -> Result<(), std::io::Error> {
    panic!("Not yet implemented -- RED scaffold")
}
