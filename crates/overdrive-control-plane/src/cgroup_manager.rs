//! Control-plane cgroup management.
//!
//! Per ADR-0028 the server creates `overdrive.slice/control-plane.slice/`
//! at boot and enrols its own PID into it via `cgroup.procs`.
//! Idempotent: a second boot reuses the existing slice.
//!
//! The workload half of the cgroup hierarchy
//! (`overdrive.slice/workloads.slice/`) is owned by `overdrive-worker`;
//! this module owns ONLY the control-plane half
//! (`overdrive.slice/control-plane.slice/`).

use std::path::Path;

/// Relative path of the control-plane slice under the cgroupfs root.
pub const CONTROL_PLANE_SLICE: &str = "overdrive.slice/control-plane.slice";

/// Create `overdrive.slice/control-plane.slice/` under `cgroup_root`
/// and enrol the running process into it via `cgroup.procs`.
///
/// `mkdir -p` semantics — idempotent on the directory already
/// existing. Production callers pass `/sys/fs/cgroup`; tests pass a
/// `tempfile::TempDir` so the assertion can read the resulting tree
/// without touching the real cgroupfs.
///
/// `pid` is the PID to enrol — production passes `getpid()`; the
/// post-boot smoke test passes a known value.
///
/// # Errors
///
/// Returns the underlying io error if either the `mkdir_p` or the
/// `cgroup.procs` write fails.
pub fn create_and_enrol_control_plane_slice_at(
    cgroup_root: &Path,
    pid: u32,
) -> Result<(), std::io::Error> {
    let dir = cgroup_root.join(CONTROL_PLANE_SLICE);
    // `mkdir -p` — idempotent on already-exists per std::fs docs.
    std::fs::create_dir_all(&dir)?;
    let procs = dir.join("cgroup.procs");
    std::fs::write(&procs, format!("{pid}\n"))?;
    Ok(())
}

/// Production wrapper — resolves the running PID from `getpid()` and
/// targets `/sys/fs/cgroup`. Linux-only; non-Linux is unreachable in
/// the production boot path.
///
/// # Errors
///
/// See [`create_and_enrol_control_plane_slice_at`].
#[cfg(target_os = "linux")]
pub fn create_and_enrol_control_plane_slice() -> Result<(), std::io::Error> {
    // `std::process::id()` returns the OS-assigned process id as a
    // safe u32 — no FFI, no unsafe block required. (Older drafts of
    // this code reached for `libc::getpid()` directly; the std API
    // is the right choice once `forbid(unsafe_code)` is on.)
    let pid = std::process::id();
    create_and_enrol_control_plane_slice_at(
        Path::new(crate::cgroup_preflight::DEFAULT_CGROUP_ROOT),
        pid,
    )
}

/// Non-Linux stub — control-plane slice creation is only invoked
/// under `#[cfg(target_os = "linux")]` in the boot path; this
/// signature exists so the call site in `lib.rs` compiles uniformly.
#[cfg(not(target_os = "linux"))]
#[allow(clippy::unnecessary_wraps, clippy::missing_const_for_fn)]
pub fn create_and_enrol_control_plane_slice() -> Result<(), std::io::Error> {
    Ok(())
}
