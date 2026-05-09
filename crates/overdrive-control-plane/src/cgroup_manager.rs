//! Control-plane cgroup management.
//!
//! Per ADR-0028 the server creates `overdrive.slice/control-plane.slice/`
//! at boot and enrols its own PID into it via `cgroup.procs`.
//! Idempotent: a second boot reuses the existing slice.
//!
//! Per `docs/feature/fix-cgroup-subtree-control-delegation/bugfix-rca.md`
//! § "Production fix #1" the control-plane init also delegates
//! `+cpu +memory +io +pids` to `overdrive.slice/cgroup.subtree_control`
//! BEFORE creating any child cgroup that would enrol a process. The
//! cgroup v2 contract forbids modifying a parent's `subtree_control`
//! while any child cgroup contains a live process; without the early
//! delegation the workload-bearing `workloads.slice` child has no
//! `cpu.*` / `memory.*` interface files and the per-allocation
//! resource-limit writes return EACCES — silently absorbed by the
//! ADR-0026 D9 warn-and-continue disposition.
//!
//! The workload half of the cgroup hierarchy
//! (`overdrive.slice/workloads.slice/`) is owned by `overdrive-worker`;
//! this module owns ONLY the control-plane half
//! (`overdrive.slice/control-plane.slice/`) plus the parent
//! `overdrive.slice/cgroup.subtree_control` delegation that both
//! children depend on.

use std::path::Path;

use crate::error::CgroupBootstrapError;

/// Relative path of the control-plane slice under the cgroupfs root.
pub const CONTROL_PLANE_SLICE: &str = "overdrive.slice/control-plane.slice";

/// Controllers delegated to `overdrive.slice` children. Per AC2 of
/// step 01-02: enable all four (`+cpu +memory +io +pids`) for symmetry
/// with the workloads-side init rather than the bare-minimum
/// `+cpu +memory` the resource-limit surface technically requires —
/// costs nothing on the kernel side and avoids "why is the parent
/// narrower than the child" reviewer comments later. Trailing newline
/// matches the cgroup-v2 admin-guide write shape.
const SUBTREE_CONTROL_CONTROLLERS: &str = "+cpu +memory +io +pids\n";

/// Create `overdrive.slice/control-plane.slice/` under `cgroup_root`,
/// delegate the standard set of controllers (`+cpu +memory +io +pids`)
/// to `overdrive.slice/cgroup.subtree_control`, and enrol the running
/// process into the control-plane slice via `cgroup.procs`.
///
/// # Order is load-bearing
///
/// The four steps run in this order, no exceptions:
///
/// 1. `mkdir -p overdrive.slice` (idempotent)
/// 2. write `+cpu +memory +io +pids\n` to
///    `overdrive.slice/cgroup.subtree_control` (idempotent on already-
///    enabled controllers — kernel accepts the re-enable as a no-op)
/// 3. `mkdir -p overdrive.slice/control-plane.slice` (idempotent)
/// 4. write `pid` to `overdrive.slice/control-plane.slice/cgroup.procs`
///
/// Step 2 MUST complete before step 4 enrols a process anywhere under
/// `overdrive.slice`. The cgroup v2 kernel contract forbids modifying
/// a parent's `subtree_control` while any child cgroup contains a live
/// process — the kernel returns `EBUSY`. If any future call sites add
/// peer slices under `overdrive.slice`, they MUST follow the same
/// rule: delegate first via this function, then enrol.
///
/// # Idempotency
///
/// Every step uses `mkdir -p` (`std::fs::create_dir_all`) or a
/// re-enable write the kernel treats as a no-op, so calling this
/// function twice in a row on the same `cgroup_root` is safe and
/// observable as Ok both times. This is what lets a process supervisor
/// (systemd, k8s, docker restart) call the init on every restart
/// without special "first-boot" branching.
///
/// # Errors
///
/// * [`CgroupBootstrapError::SubtreeControlBusy`] — the kernel
///   returned EBUSY on the `subtree_control` write because a process
///   was already enrolled in `overdrive.slice` (a previous boot
///   initialised the slice in the wrong order). Operator hint in the
///   `Display` impl: restart cleanly so no stale process remains.
/// * [`CgroupBootstrapError::SubtreeControlWriteFailed`] — any other
///   I/O failure on the `subtree_control` write (typically
///   `PermissionDenied` from cgroupfs delegation refusal, or
///   `NotFound` if the enclosing slice does not exist).
/// * [`CgroupBootstrapError::BootstrapIoFailed`] — any other
///   non-`subtree_control` failure (`mkdir`, `cgroup.procs` write).
pub fn create_and_enrol_control_plane_slice_at(
    cgroup_root: &Path,
    pid: u32,
) -> Result<(), CgroupBootstrapError> {
    // Step 1 — mkdir overdrive.slice (idempotent).
    let parent_slice = cgroup_root.join("overdrive.slice");
    std::fs::create_dir_all(&parent_slice)
        .map_err(|source| CgroupBootstrapError::BootstrapIoFailed { source })?;

    // Step 2 — delegate controllers to overdrive.slice's children.
    // MUST happen BEFORE step 4 (PID enrolment) per the cgroup v2
    // contract documented in this fn's "Order is load-bearing"
    // section.
    let subtree_control = parent_slice.join("cgroup.subtree_control");
    if let Err(err) = std::fs::write(&subtree_control, SUBTREE_CONTROL_CONTROLLERS) {
        return Err(CgroupBootstrapError::from_subtree_control_io(err));
    }

    // Step 3 — mkdir control-plane.slice underneath (idempotent).
    let dir = cgroup_root.join(CONTROL_PLANE_SLICE);
    std::fs::create_dir_all(&dir)
        .map_err(|source| CgroupBootstrapError::BootstrapIoFailed { source })?;

    // Step 4 — enrol the server PID into the control-plane slice.
    let procs = dir.join("cgroup.procs");
    std::fs::write(&procs, format!("{pid}\n"))
        .map_err(|source| CgroupBootstrapError::BootstrapIoFailed { source })?;

    Ok(())
}

/// Production wrapper — resolves the running PID from `getpid()` and
/// targets `/sys/fs/cgroup`.
///
/// # Errors
///
/// See [`create_and_enrol_control_plane_slice_at`].
pub fn create_and_enrol_control_plane_slice() -> Result<(), CgroupBootstrapError> {
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
