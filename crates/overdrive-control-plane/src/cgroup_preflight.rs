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
//!      `cgroup.subtree_control` of the *enclosing* slice. The
//!      enclosing slice is discovered by reading `/proc/self/cgroup`
//!      and parsing the cgroup-v2 (`0::/...`) line — see ADR-0028 §4
//!      step 4.
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
        /// Enclosing slice directory (per `/proc/self/cgroup`); the
        /// missing controllers are absent from
        /// `<this directory>/cgroup.subtree_control`.
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

    /// Step 4 — could not discover the enclosing cgroup. Either
    /// `/proc/self/cgroup` is unreadable, or it does not contain a
    /// cgroup v2 (`0::`) line (cgroup v1-only host shape that
    /// slipped past step 1 — extremely rare).
    #[error(
        "could not discover enclosing cgroup from /proc/self/cgroup: {source}.\n\
        \n\
        Detected: cgroup v2 IS available, BUT the running process's enclosing\n\
        cgroup could not be determined.\n\
        \n\
        Try one of:\n\
        \n\
          1. Verify /proc/self/cgroup is readable and contains a cgroup v2\n\
             (`0::`) line. On a healthy systemd-managed host this is\n\
             automatic.\n\
          2. Run without cgroup isolation (development only — workloads\n\
             are unbounded; control plane is not protected):\n\
               overdrive serve --allow-no-cgroups\n\
        \n\
        Documentation: https://docs.overdrive.sh/operations/cgroup-delegation"
    )]
    CgroupPathDiscoveryFailed {
        /// I/O error from reading `/proc/self/cgroup`, OR a synthetic
        /// `InvalidData` error when the file is readable but contains
        /// no cgroup-v2 (`0::`) line.
        #[source]
        source: std::io::Error,
    },

    /// Step 1 — `/proc/filesystems` is unreadable for a reason other
    /// than absence (`PermissionDenied`, `EIO`, `IsADirectory`, broken
    /// procfs, /proc not mounted in this container, etc.). `NotFound`
    /// does NOT trigger this variant — a missing `/proc/filesystems`
    /// is the v1-host signal and falls through to `NoCgroupV2`.
    ///
    /// The Display message names the failure cause and the
    /// `--allow-no-cgroups` dev escape hatch — and deliberately does
    /// NOT prescribe "boot a newer kernel", because that is the
    /// specific misdiagnosis this variant exists to correct.
    #[error(
        "could not read /proc/filesystems: {source}.\n\
        \n\
        Detected: /proc/filesystems is present but the running process\n\
        could not read it. This is distinct from cgroup v2 being unavailable\n\
        on the kernel — fix the procfs access issue and retry.\n\
        \n\
        Try one of:\n\
        \n\
          1. Verify /proc is mounted and /proc/filesystems is readable\n\
             by the running UID (a missing /proc mount in a container\n\
             sandbox is the most common cause).\n\
          2. Run without cgroup isolation (development only — workloads\n\
             are unbounded; control plane is not protected):\n\
               overdrive serve --allow-no-cgroups\n\
        \n\
        Documentation: https://docs.overdrive.sh/operations/cgroup-delegation"
    )]
    ProcFilesystemsUnreadable {
        /// I/O error from reading `/proc/filesystems` with a kind
        /// other than `NotFound`.
        #[source]
        source: std::io::Error,
    },

    /// Step 4 — the enclosing slice's `cgroup.subtree_control` is
    /// unreadable for any reason (`PermissionDenied`, `EIO`,
    /// `IsADirectory`, `NotFound`, `Other`, …). `NotFound` on this
    /// read indicates the enclosing-slice path is not a real cgroup-v2
    /// directory — the kernel guarantees `cgroup.subtree_control`
    /// exists under every cgroup directory per
    /// `Documentation/admin-guide/cgroup-v2.rst`, so its absence is
    /// structurally distinct from "no controllers delegated"
    /// (`DelegationMissing`). Every `io::Error` from the step-4 read
    /// surfaces via this variant — see Option B of the RCA at
    /// `docs/feature/fix-cgroup-preflight-subtree-unreadable/bugfix-rca.md`.
    ///
    /// The Display message names the failure cause and the
    /// `--allow-no-cgroups` dev escape hatch — and deliberately does
    /// NOT mention `Delegate=yes` or "delegation required", because
    /// those phrases are reserved for `DelegationMissing` and are
    /// exactly the misdiagnosis this variant exists to correct.
    #[error(
        "could not read cgroup.subtree_control of enclosing slice {slice}: {source}.\n\
        \n\
        Detected: cgroup v2 IS available and the enclosing slice path was\n\
        discovered, BUT the slice's cgroup.subtree_control could not be\n\
        read. This is distinct from delegation being absent — the kernel\n\
        creates cgroup.subtree_control under every cgroup directory, so an\n\
        I/O failure here points at a cgroupfs configuration issue or a\n\
        race against the slice being unmounted, not at missing controllers.\n\
        \n\
        Try one of:\n\
        \n\
          1. Verify the enclosing slice is a real cgroup-v2 directory:\n\
             confirm /sys/fs/cgroup is the cgroup-v2 unified hierarchy\n\
             mount and that /proc/self/cgroup matches your environment.\n\
          2. Verify the running UID can read files under the enclosing\n\
             slice (a NotFound here is unusual — cgroup.subtree_control\n\
             is kernel-created for every cgroup directory).\n\
          3. Run as root (development only — no isolation guarantees):\n\
               sudo overdrive serve\n\
          4. Run without cgroup isolation (development only — workloads\n\
             are unbounded; control plane is not protected):\n\
               overdrive serve --allow-no-cgroups\n\
        \n\
        Documentation: https://docs.overdrive.sh/operations/cgroup-delegation",
        slice = slice.display(),
    )]
    SubtreeControlUnreadable {
        /// Enclosing slice directory whose `cgroup.subtree_control`
        /// could not be read. Captured for operator triage —
        /// parallel to `DelegationMissing.slice`.
        slice: PathBuf,
        /// I/O error from reading `<slice>/cgroup.subtree_control`.
        /// Every `io::ErrorKind` flows through this variant per
        /// Option B of the RCA — including `NotFound`, which is
        /// structurally distinct from "no controllers delegated"
        /// because the kernel guarantees the file exists under every
        /// cgroup directory.
        #[source]
        source: std::io::Error,
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
/// `proc_self_cgroup` names the file from which the *enclosing* slice
/// path is discovered for step 4 per ADR-0028 §4 step 4 (production:
/// `/proc/self/cgroup`; tests: a tempdir-fabricated file). Step 4
/// reads this file, parses the cgroup-v2 (`0::/...`) line, joins the
/// parsed path to `cgroup_root`, and inspects the resulting
/// directory's `cgroup.subtree_control`.
///
/// # Errors
///
/// See [`CgroupPreflightError`] variants.
pub fn run_preflight_at(
    cgroup_root: &Path,
    uid: u32,
    proc_filesystems: &Path,
    proc_self_cgroup: &Path,
) -> Result<(), CgroupPreflightError> {
    // Step 1 — kernel exposes cgroup v2.
    let proc_fs = match std::fs::read_to_string(proc_filesystems) {
        Ok(s) => s,
        // NotFound IS the v1-host signal — fall through to the
        // cgroup_v2_available = false branch below, which returns
        // NoCgroupV2 with the kernel-upgrade remediation.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(CgroupPreflightError::ProcFilesystemsUnreadable { source: err }),
    };
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

    // Step 4 — required controllers in subtree_control of the
    // *enclosing* slice. Per ADR-0028 §4 step 4 the enclosing slice is
    // discovered by reading /proc/self/cgroup (a single line of the
    // form "0::/user.slice/user-1000.slice/session-3.scope" on cgroup
    // v2). The parsed path is relative to the cgroupfs mount root.
    let proc_self = std::fs::read_to_string(proc_self_cgroup)
        .map_err(|err| CgroupPreflightError::CgroupPathDiscoveryFailed { source: err })?;
    let enclosing_rel = parse_cgroup_v2_path(&proc_self).ok_or_else(|| {
        CgroupPreflightError::CgroupPathDiscoveryFailed {
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{} did not contain a cgroup v2 (`0::`) line; got {proc_self:?}",
                    proc_self_cgroup.display(),
                ),
            ),
        }
    })?;
    // Trim any trailing newline/whitespace; strip the leading `/` so
    // Path::join treats it as a relative path (Path::join on an
    // absolute argument discards the prefix).
    let enclosing_rel = enclosing_rel.trim();
    let enclosing_abs = cgroup_root.join(enclosing_rel.trim_start_matches('/'));
    let subtree_control = enclosing_abs.join("cgroup.subtree_control");

    // Every io::Error (NotFound included) surfaces as
    // SubtreeControlUnreadable per Option B of the RCA at
    // docs/feature/fix-cgroup-preflight-subtree-unreadable/bugfix-rca.md.
    // The kernel guarantees cgroup.subtree_control exists under every
    // cgroup-v2 directory, so its absence is structurally distinct
    // from "no controllers delegated" — it indicates the enclosing
    // slice path is not a cgroup directory at all. Absorbing any kind
    // here would misdiagnose the failure as DelegationMissing and
    // prescribe `Delegate=yes`, which doesn't fix any of the actual
    // causes (PermissionDenied, EIO, IsADirectory, NotFound, …).
    let contents = std::fs::read_to_string(&subtree_control).map_err(|err| {
        CgroupPreflightError::SubtreeControlUnreadable { slice: enclosing_abs.clone(), source: err }
    })?;
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
            slice: enclosing_abs,
            missing,
            missing_human,
            plural,
            is_or_are,
        });
    }

    Ok(())
}

/// Parse the cgroup v2 line (`"0::/path/to/slice"`) out of
/// `/proc/self/cgroup`. Returns the path tail (e.g.
/// `/user.slice/user-1000.slice/session-3.scope`) on success, or
/// `None` if the file lists only cgroup v1 hierarchy lines or is
/// empty. The returned slice borrows from `contents`.
fn parse_cgroup_v2_path(contents: &str) -> Option<&str> {
    contents.lines().find_map(|line| line.strip_prefix("0::"))
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
    run_preflight_at(
        Path::new(DEFAULT_CGROUP_ROOT),
        uid,
        Path::new("/proc/filesystems"),
        Path::new("/proc/self/cgroup"),
    )
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
