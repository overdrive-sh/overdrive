//! Real-boot provisioning fixture for the Cloud-Hypervisor microVM driver's
//! Tier-3 tests (`microvm-driver-cloud-hypervisor`, GH #42, roadmap step
//! 01-04).
//!
//! [`VmFixture::provision`] is the SINGLE surface every Tier-3 VM test in
//! this feature calls before attempting a real boot. The whole sequence
//! below runs under one exclusive advisory lock (D2 review remediation —
//! see [`acquire_provision_lock`]), so nextest's default parallel-process
//! execution can call `provision()` concurrently against the same shared
//! `staging_root` without racing. It:
//!
//! 1. Runs the [`preflight_kvm_capability`] check (`/dev/kvm` reachable,
//!    host is not nested-Apple-virtualized) — ADR-0082 §D5 / roadmap AC3.
//! 2. Confirms `cloud-hypervisor` is present and has the `--landlock`
//!    capability the driver depends on (brief.md §109's "name the floor
//!    against a capability, not a number") — AC2.
//! 3. Confirms the staging root is genuinely reflink-capable via an
//!    EXECUTED `FICLONE` (`cp --reflink=always` against a real written
//!    file) — never an fstype string comparison (brief.md §102/§107,
//!    correction C-1) — AC1.
//! 4. Stages a kernel image and builds an ext4 rootfs with a static-musl
//!    `overdrive-init` baked in at BOTH [`GUEST_INIT_SBIN_PATH`] and
//!    [`GUEST_INIT_PATH`] — AC1.
//!
//! Every step surfaces a distinct, named [`VmFixtureError`] variant on
//! failure — never a downstream test timeout, never a silent skip.
//!
//! # What this fixture is NOT
//!
//! - **Not the production appliance image factory.** `infra/metal/
//!   provision.sh` stages the operator-facing `/srv/vm` data directory;
//!   this fixture is TEST-envelope provisioning only and never touches
//!   `infra/`. The two independently probe `FICLONE` because they answer
//!   different questions on different filesystems.
//! - **Not gated by a Cargo feature itself.** `kvm-tests` (landing step
//!   01-06) is the PRIMARY gate on whether a consumer compiles at all;
//!   [`preflight_kvm_capability`] is the runtime DEFENSE for the case
//!   `kvm-tests` is enabled on a host that cannot actually honor it
//!   (roadmap AC4). This module carries no `#[cfg(feature = "kvm-tests")]`
//!   of its own.
//!
//! # Design decisions this step made (no ADR pins the exact shape)
//!
//! **Pinned kernel source.** "Pinned" here means *the same staged file is
//! reused across a fixture's lifetime* (AC6), not "downloaded from a
//! project-wide fixed artifact". The source is the box's OWN running
//! kernel, `/boot/vmlinuz-$(uname -r)`, copied verbatim — **x86_64 only**
//! (D8 review remediation). This is not a guess: spike P1 measured
//! exactly this shape as **WORKS** — "the kernel is the distro `vmlinuz`
//! copied verbatim... `file` identifies it as a bzImage and CH loads it
//! directly — no UKI, no EFI-zboot, no zstd"
//! (`docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md`,
//! § P1). The ADR-0068 pinned-appliance-kernel (6.18 LTS) is a distinct,
//! out-of-scope concern — spike P3, which would have exercised THAT
//! kernel, was never run, and building/fetching it is DEVOPS's
//! `infra/metal/provision.sh` territory, not this fixture's.
//!
//! **Verbatim copy does NOT extend to aarch64.** The Ubuntu aarch64
//! `vmlinuz` is a UKI wrapping EFI-zboot wrapping a zstd-compressed raw
//! `Image` (`spike-scratch/increment-a/build.sh`'s two-layer unwrap);
//! staged verbatim it always fails [`KernelImage::validate`]. This
//! fixture does not implement the unwrap — [`stage_kernel`] reports
//! [`VmFixtureError::KernelImageRequiresUkiUnwrap`] on aarch64 rather
//! than the generic "should not happen" [`VmFixtureError::KernelImageInvalid`]
//! it reports for a genuinely-broken x86_64 host.
//!
//! **`GUEST_INIT_PATH` and `GUEST_INIT_SBIN_PATH` (D1 review
//! remediation — was a BLOCKER: the original note here was FALSE and
//! would have broken the very first real boot).** The claim used to be
//! that absent an `init=` boot parameter the kernel tries `/init` first
//! "regardless of whether root is an initramfs or a real block device."
//! That is wrong. `/init` is the INITRAMFS convention only — it is
//! `ramdisk_execute_command`'s default, consulted only while unpacking an
//! initramfs (`rdinit=`'s default), resolved *inside the cpio* before any
//! `switch_root`. For a **block-device root** (this fixture's ext4 image)
//! with **no `init=`** on the cmdline, `kernel_init_freeable()` never
//! looks at `/init` at all: it falls through the (unset) `init=`-supplied
//! command, then tries, in order, `/sbin/init`, `/etc/init`, `/bin/init`,
//! `/bin/sh`, and panics with "No working init found" only if every one
//! of those four is absent (`init/main.c`). `/sbin/init` is therefore the
//! path a no-`init=` boot ACTUALLY reaches first.
//!
//! [`overdrive_core::vm::config::KernelCmdline::platform_default`] sets
//! NO `init=` today — confirmed by reading its body, not assumed — so
//! this fixture bakes `overdrive-init` at BOTH [`GUEST_INIT_SBIN_PATH`]
//! (`/sbin/init`, the load-bearing fix: the path a no-`init=` boot
//! reaches) and [`GUEST_INIT_PATH`] (`/init`, reached only when the
//! caller explicitly passes `init=/init` — the shape EVERY spike boot
//! used, `spike-scratch/increment-a/run.sh:32`, and the exact path
//! `spike-scratch/increment-a/build.sh` staged its guest binary at).
//! Landing `init=/init` on `platform_default` itself would be
//! belt-and-suspenders and is outside this fixture's file boundary —
//! flagged here, at the SAME prominence as the vsock gap immediately
//! below, for whichever step finalizes the cmdline against a real boot
//! (01-08).
//!
//! **Guest vsock modules — closed at step 01-08 (DISTILL DWD-21).**
//! Ubuntu kernels build `CONFIG_VSOCKETS`/`CONFIG_VIRTIO_VSOCKETS` as
//! *modules* (spike finding `[D2]`), so a guest booted from a
//! verbatim-copied stock kernel needs them `finit_module`-loaded before
//! it can open the beacon socket, or it fails `EAFNOSUPPORT`. This
//! fixture's [`build_staging_tree`] now stages the three vsock `.ko`
//! (zstd-decompressed, from the SAME `uname -r` [`stage_kernel`] copied
//! the kernel from — no rootfs↔kernel skew) into the shared in-guest
//! directory [`overdrive_core::vm::beacon::GUEST_VSOCK_MODULE_DIR`]
//! pins; `overdrive-init` (step 01-03's crate) is the loader that reads
//! them, tolerating "already loaded" and "absent" (the vsock=y
//! appliance-kernel case, ADR-0068 §4) as success. See ADR-0082 §D4's
//! 2026-08-14 amendment for the full contract both sides honor.

#![allow(clippy::doc_markdown)]
#![cfg(target_os = "linux")]

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use overdrive_core::vm::beacon::{GUEST_VSOCK_MODULE_DIR, GUEST_VSOCK_MODULE_FILES};
use overdrive_core::vm::config::{HostArch, KERNEL_MAGIC_WINDOW, KernelFormatError, KernelImage};

/// The in-guest absolute path the kernel's own no-`init=` fallback
/// search finds FIRST.
///
/// `try_to_run_init_process("/sbin/init")` — see the module doc's
/// "Design decisions" section. This is the LOAD-BEARING path (D1): the
/// one a real boot with
/// [`overdrive_core::vm::config::KernelCmdline::platform_default`]'s
/// cmdline (no `init=`) actually reaches.
pub const GUEST_INIT_SBIN_PATH: &str = "/sbin/init";

/// The in-guest absolute path this fixture ALSO bakes `overdrive-init`
/// into.
///
/// Reached only when the boot cmdline carries `init=/init` (every spike
/// boot did this explicitly — `spike-scratch/increment-a/run.sh:32`).
/// `/init` is the INITRAMFS convention and has NO special standing for a
/// real block-device root absent an explicit `init=` — see
/// [`GUEST_INIT_SBIN_PATH`] for the path a no-`init=` boot actually
/// reaches, and the module doc's "Design decisions" section for the
/// corrected kernel search order.
pub const GUEST_INIT_PATH: &str = "/init";

/// `/dev/kvm`'s well-known path — named once so the preflight and its
/// diagnostics never drift against each other.
const KVM_DEVICE_PATH: &str = "/dev/kvm";

/// The exclusive advisory-lock file name under `staging_root`.
///
/// Serializes the whole [`VmFixture::provision`] body across concurrent
/// callers (D2). Named once so [`acquire_provision_lock`] and its
/// diagnostics never drift against each other.
const PROVISION_LOCK_FILENAME: &str = ".provision-lock";

/// 8 MiB of real, written (non-sparse) bytes — matches `infra/metal/
/// provision.sh`'s own probe size. A sparse file would let `FICLONE`
/// trivially "succeed" with nothing to actually clone, which is exactly
/// the dishonest signal correction C-1 exists to refuse.
const REFLINK_PROBE_BYTES: usize = 8 * 1024 * 1024;

/// The ext4 (and ext2/ext3) superblock's byte offset within the image —
/// the superblock starts at byte 1024 and `s_magic` is 56 bytes into it.
const EXT4_SUPERBLOCK_MAGIC_OFFSET: u64 = 1080;

/// `0xEF53` as the two little-endian bytes actually stored on disk at
/// [`EXT4_SUPERBLOCK_MAGIC_OFFSET`].
const EXT4_SUPERBLOCK_MAGIC: [u8; 2] = [0x53, 0xEF];

/// Everything a Tier-3 VM test needs to attempt a real Cloud Hypervisor
/// boot.
///
/// A validated, staged kernel image; an ext4 rootfs with
/// `overdrive-init` baked in at [`GUEST_INIT_SBIN_PATH`] and
/// [`GUEST_INIT_PATH`]; and the confirmed location/version of a capable
/// `cloud-hypervisor` binary.
///
/// Returned only by [`VmFixture::provision`], which has already run every
/// precondition check this struct's fields imply — a caller never
/// constructs one directly.
#[derive(Debug, Clone)]
pub struct VmFixture {
    /// The staged, format-validated kernel image
    /// (`<staging_root>/kernel`).
    pub kernel_path: PathBuf,
    /// The staged ext4 rootfs image, `overdrive-init` baked in at
    /// [`GUEST_INIT_SBIN_PATH`] and [`GUEST_INIT_PATH`]
    /// (`<staging_root>/rootfs.ext4`).
    pub rootfs_path: PathBuf,
    /// Absolute path to the confirmed-capable `cloud-hypervisor` binary.
    pub cloud_hypervisor_bin: PathBuf,
    /// `cloud-hypervisor --version`'s raw output, captured for
    /// diagnostics (never parsed as a floor gate — see
    /// [`VmFixtureError::CloudHypervisorTooOld`]'s docs).
    pub cloud_hypervisor_version: String,
}

impl VmFixture {
    /// Provision (or re-verify) the shared VM-boot fixture rooted at
    /// `staging_root`, creating it if absent.
    ///
    /// Acquires the exclusive provisioning lock FIRST (D2 — see
    /// [`acquire_provision_lock`]), then runs, in order: the
    /// KVM/nested-virt preflight, the `cloud-hypervisor` presence +
    /// capability check, the executed `FICLONE` reflink probe on
    /// `staging_root`, kernel staging, and rootfs staging (which itself
    /// builds `overdrive-init` for the host's static-musl target). Cheap
    /// checks run first so an incapable host fails before any expensive
    /// staging work begins.
    ///
    /// # Concurrency
    ///
    /// AC5 makes concurrent invocation the DEFAULT, not an edge case:
    /// nextest runs every Tier-3 test as its own parallel process, and
    /// every one of them calls `provision()` against the SAME shared
    /// `staging_root`. The whole body runs under one held-for-the-call
    /// `flock(2)`-style lock, so interleaved `remove_dir_all`/`create`/
    /// `mkfs.ext4` calls against the same image paths — and the FICLONE
    /// probe's fixed filenames — can never race across processes.
    ///
    /// # Idempotency
    ///
    /// A second call against the same `staging_root` re-verifies rather
    /// than re-downloads/re-bakes: an already-staged, still-valid kernel
    /// is reused; the rootfs image is rebuilt only when the freshly-built
    /// `overdrive-init` binary differs (by length + mtime, down to
    /// nanosecond precision — D11) from the one the last-built image was
    /// staged from, OR when the staged image itself no longer validates
    /// as a real ext4 filesystem (D10 — re-verified on every reuse,
    /// never trusted on the marker file's word alone). The KVM/virt
    /// preflight, the `cloud-hypervisor` check, and the `FICLONE` probe
    /// always re-run in full — they are the trust boundary and
    /// re-verifying them is cheap.
    ///
    /// # Errors
    ///
    /// Returns the first [`VmFixtureError`] any step surfaces; no
    /// partially-staged artifact is left masquerading as complete (a
    /// failed rootfs build invalidates its idempotency marker before the
    /// rebuild starts and removes its own stale image on an
    /// `mkfs.ext4` failure).
    pub fn provision(staging_root: &Path) -> Result<Self, VmFixtureError> {
        let _lock = acquire_provision_lock(staging_root)?;
        preflight_kvm_capability()?;
        let (cloud_hypervisor_bin, cloud_hypervisor_version) = check_cloud_hypervisor()?;
        ensure_reflink_capable(staging_root)?;
        let kernel_path = stage_kernel(staging_root)?;
        let rootfs_path = stage_rootfs(staging_root)?;
        Ok(Self { kernel_path, rootfs_path, cloud_hypervisor_bin, cloud_hypervisor_version })
    }
}

/// A reasonable default staging root for local/manual use.
///
/// `/srv/vm`'s `overdrive-testing` subdirectory when `/srv/vm` exists (the
/// metal box's own convention — `infra/metal/provision.sh` mounts it
/// XFS-reflink-formatted when `--data-disk` was given), else a path under
/// `std::env::temp_dir()`.
///
/// This is a convenience a caller opts into by calling it explicitly and
/// passing the result to [`VmFixture::provision`] — `provision` itself
/// never picks a path on the caller's behalf (`.claude/rules/
/// development.md` § "Port-trait dependencies" — required, not defaulted,
/// at the call site).
#[must_use]
pub fn default_staging_root() -> PathBuf {
    let srv_vm = Path::new("/srv/vm");
    if srv_vm.is_dir() {
        srv_vm.join("overdrive-testing")
    } else {
        std::env::temp_dir().join("overdrive-vm-fixture")
    }
}

/// Acquires an exclusive, held-for-the-whole-body advisory lock on
/// `<staging_root>/.provision-lock` (D2 — see [`VmFixture::provision`]'s
/// "Concurrency" doc section for why this is needed by default, not just
/// under contention). `ensure_reflink_capable` and `stage_rootfs` are
/// both PRIVATE to this module and reachable only through `provision`,
/// so holding the lock across the whole call protects both — including
/// the FICLONE probe's fixed filenames — without a per-caller filename
/// scheme.
///
/// `std::fs::File::lock()` (stable since Rust 1.89; this workspace's
/// PINNED TOOLCHAIN is 1.95 per `rust-toolchain.toml` — no new
/// dependency) blocks until the lock is acquired, so a second process
/// simply waits its turn rather than racing. The lock is released when
/// the returned `File` (and its underlying fd) is dropped at the end of
/// the caller's scope.
///
/// The `#[clippy::msrv]` override below is a per-item MSRV bump, not a
/// Cargo.toml edit: the workspace's DECLARED `rust-version` (1.88, one
/// point below `File::lock`'s 1.89 stabilization) is a metadata floor
/// with no external consumer to honor (every crate here is `publish =
/// false`) — the toolchain that actually compiles this code is always
/// 1.95. Scoping the override to this one function keeps the crate-wide
/// declared floor untouched while unblocking the exact primitive D2
/// calls for, entirely within this file.
///
/// # Errors
///
/// [`VmFixtureError::StagingDirUnusable`] if `staging_root` itself
/// cannot be created, or [`VmFixtureError::ProvisionLockFailed`] if the
/// lock file could not be opened/created or the lock itself could not be
/// acquired (never for lock *contention*, which blocks rather than
/// erroring).
#[clippy::msrv = "1.89"]
fn acquire_provision_lock(staging_root: &Path) -> Result<fs::File, VmFixtureError> {
    fs::create_dir_all(staging_root).map_err(|source| {
        VmFixtureError::staging_dir_unusable(staging_root.to_path_buf(), source)
    })?;
    let lock_path = staging_root.join(PROVISION_LOCK_FILENAME);
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|source| VmFixtureError::provision_lock_failed(lock_path.clone(), source))?;
    lock_file
        .lock()
        .map_err(|source| VmFixtureError::provision_lock_failed(lock_path.clone(), source))?;
    Ok(lock_file)
}

/// Capability preflight (roadmap AC3/AC4).
///
/// Confirms `/dev/kvm` is reachable `O_RDWR` under the current identity
/// and the host is not nested-Apple-virtualized. This is the runtime
/// DEFENSE for the case the `kvm-tests` Cargo feature is enabled on an
/// incapable host — the feature gate is the primary control; this
/// function is what fires when that gate was crossed anyway. It never
/// silently skips: every call performs both checks and returns a real
/// verdict.
///
/// # Errors
///
/// - [`VmFixtureError::NoKvmDevice`] — no `/dev/kvm` node at all.
/// - [`VmFixtureError::KvmPermissionDenied`] — the node exists but the
///   current identity cannot open it `O_RDWR`.
/// - [`VmFixtureError::KvmDeviceIo`] — any other failure opening the
///   device.
/// - [`VmFixtureError::SystemdDetectVirtUnavailable`] — `systemd-detect-
///   virt` itself could not be run.
/// - [`VmFixtureError::SystemdDetectVirtEmptyOutput`] — it ran but
///   produced no usable output (D12).
/// - [`VmFixtureError::NestedAppleHost`] — the host reports
///   `systemd-detect-virt=apple`.
pub fn preflight_kvm_capability() -> Result<(), VmFixtureError> {
    open_kvm_device()?;
    let virt = detect_virt()?;
    if virt == "apple" {
        return Err(VmFixtureError::NestedAppleHost);
    }
    Ok(())
}

/// Opens [`KVM_DEVICE_PATH`] `O_RDWR` and classifies the failure, if any,
/// into the three distinct modes `preflight_kvm_capability` can report.
fn open_kvm_device() -> Result<(), VmFixtureError> {
    match fs::OpenOptions::new().read(true).write(true).open(KVM_DEVICE_PATH) {
        Ok(_handle) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            Err(VmFixtureError::no_kvm_device(source))
        }
        Err(source) if source.kind() == io::ErrorKind::PermissionDenied => {
            Err(VmFixtureError::kvm_permission_denied(describe_kvm_device_mode(), source))
        }
        Err(source) => Err(VmFixtureError::kvm_device_io(source)),
    }
}

/// `uid=… gid=… mode=…` as observed via `stat(2)` on [`KVM_DEVICE_PATH`],
/// for [`VmFixtureError::KvmPermissionDenied`]'s diagnostic message.
fn describe_kvm_device_mode() -> String {
    match fs::metadata(KVM_DEVICE_PATH) {
        Ok(meta) => format!("uid={} gid={} mode={:o}", meta.uid(), meta.gid(), meta.mode() & 0o777),
        Err(source) => format!("(could not stat {KVM_DEVICE_PATH}: {source})"),
    }
}

/// Runs `systemd-detect-virt` and returns its trimmed stdout verbatim
/// (`"apple"`, `"kvm"`, `"none"`, …). A non-zero exit is NOT a spawn
/// failure — `systemd-detect-virt` exits non-zero precisely when it
/// detects no virtualization at all, while still printing `"none"`.
///
/// A well-behaved `systemd-detect-virt` ALWAYS prints something (a virt
/// type name or `"none"`); blank stdout (garbage output, or an internal
/// tool failure that still exits 0) is therefore its OWN distinct
/// failure mode (D12) and is never silently read as "not apple" — the
/// caller's `virt == "apple"` check would otherwise treat "no usable
/// signal at all" as a clean pass.
fn detect_virt() -> Result<String, VmFixtureError> {
    let output = Command::new("systemd-detect-virt")
        .output()
        .map_err(VmFixtureError::systemd_detect_virt_unavailable)?;
    let virt = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if virt.is_empty() {
        return Err(VmFixtureError::systemd_detect_virt_empty_output(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(virt)
}

/// Confirms `cloud-hypervisor` is on `PATH`, runnable, and carries the
/// `--landlock` capability the driver depends on (the floor is named
/// against a capability, not a version number — brief.md §109). Returns
/// the resolved absolute binary path and the raw `--version` output.
///
/// # Errors
///
/// - [`VmFixtureError::CloudHypervisorMissing`] — the binary could not be
///   spawned at all (not on `PATH`, or not executable).
/// - [`VmFixtureError::CloudHypervisorBroken`] — the binary WAS spawned
///   but exited non-zero on `--version` or `--help` (D9: present but
///   broken — corrupted binary, missing shared library, … — never
///   conflated with "too old", which requires the binary to have
///   actually run and reported a clean exit).
/// - [`VmFixtureError::CloudHypervisorTooOld`] — the binary ran cleanly,
///   but its `--help` output has no `--landlock` flag.
fn check_cloud_hypervisor() -> Result<(PathBuf, String), VmFixtureError> {
    let version_output = Command::new("cloud-hypervisor")
        .arg("--version")
        .output()
        .map_err(VmFixtureError::cloud_hypervisor_missing)?;
    if !version_output.status.success() {
        return Err(VmFixtureError::cloud_hypervisor_broken(
            "--version",
            version_output.status.code(),
            String::from_utf8_lossy(&version_output.stderr).into_owned(),
        ));
    }
    let version = String::from_utf8_lossy(&version_output.stdout).trim().to_owned();

    let help_output = Command::new("cloud-hypervisor")
        .arg("--help")
        .output()
        .map_err(|source| VmFixtureError::spawn("cloud-hypervisor --help", source))?;
    if !help_output.status.success() {
        return Err(VmFixtureError::cloud_hypervisor_broken(
            "--help",
            help_output.status.code(),
            String::from_utf8_lossy(&help_output.stderr).into_owned(),
        ));
    }
    let help_text = format!(
        "{}{}",
        String::from_utf8_lossy(&help_output.stdout),
        String::from_utf8_lossy(&help_output.stderr),
    );
    if !help_text.contains("--landlock") {
        return Err(VmFixtureError::cloud_hypervisor_too_old(version));
    }

    Ok((resolve_on_path("cloud-hypervisor"), version))
}

/// Resolves `name` to an absolute path via `which`(1) so the returned
/// [`VmFixture`] carries an inspectable location rather than a bare
/// `PATH`-dependent command name. Best-effort and INFALLIBLE by design
/// (D6): the earlier spawn already proved the binary IS runnable, so a
/// `which` that cannot even be spawned is diagnostic enrichment falling
/// short — never a second gate on `provision`. Falls back to the bare
/// `name` whenever `which` cannot be run, exits non-zero, or prints
/// nothing.
fn resolve_on_path(name: &str) -> PathBuf {
    let Ok(output) = Command::new("which").arg(name).output() else {
        return PathBuf::from(name);
    };
    let resolved = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if output.status.success() && !resolved.is_empty() {
        PathBuf::from(resolved)
    } else {
        PathBuf::from(name)
    }
}

/// Confirms `dir` (creating it if absent) genuinely supports `FICLONE` via
/// an EXECUTED clone of a real written file — never an `fstype` string
/// comparison (correction C-1; the exact pattern `infra/metal/
/// provision.sh:419-430` already proved out, reused here).
///
/// # Errors
///
/// - [`VmFixtureError::StagingDirUnusable`] — `dir` could not be created
///   or written to at all.
/// - [`VmFixtureError::NotReflinkCapable`] — `dir` is writable but
///   `cp --reflink=always` against a real file in it failed.
fn ensure_reflink_capable(dir: &Path) -> Result<(), VmFixtureError> {
    fs::create_dir_all(dir)
        .map_err(|source| VmFixtureError::staging_dir_unusable(dir.to_path_buf(), source))?;

    let probe = dir.join(".vm-fixture-reflink-probe");
    let clone = dir.join(".vm-fixture-reflink-probe.clone");
    // Best-effort cleanup of leftovers from a prior aborted run.
    let _ = fs::remove_file(&probe);
    let _ = fs::remove_file(&clone);

    fs::write(&probe, vec![0xAB_u8; REFLINK_PROBE_BYTES])
        .map_err(|source| VmFixtureError::staging_dir_unusable(dir.to_path_buf(), source))?;

    let result = Command::new("cp").arg("--reflink=always").arg(&probe).arg(&clone).output();

    let _ = fs::remove_file(&probe);
    let _ = fs::remove_file(&clone);

    match result {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let detail = String::from_utf8_lossy(&out.stderr).into_owned();
            Err(VmFixtureError::not_reflink_capable(dir.to_path_buf(), detail))
        }
        Err(source) => Err(VmFixtureError::spawn("cp --reflink=always", source)),
    }
}

/// Stages (or re-verifies) the host's own running kernel at
/// `<staging_root>/kernel`, validated via [`KernelImage::validate`] before
/// it is trusted. See the module doc's "Pinned kernel source" design
/// note.
///
/// Re-verification is release-aware, not just format-aware (review
/// finding, 01-08 review remediation): [`kernel_is_valid`] alone only
/// confirms the STAGED file still parses as a valid bzImage for this
/// arch — it says nothing about whether that bzImage still matches the
/// CURRENTLY RUNNING kernel. A host kernel package upgrade (`apt
/// upgrade`, unattended-upgrades) changes `uname -r` and the module set
/// under `/lib/modules/<new-release>/` WITHOUT touching a
/// previously-staged `<staging_root>/kernel` copy, which stays
/// syntactically valid forever. [`build_staging_tree`]'s vsock modules
/// (ADR-0082 §D4 amendment 2026-08-14) are staged from the CURRENT
/// `uname -r`, so a stale kernel copy silently paired with fresh
/// modules is a real kernel/module ABI mismatch — confirmed on the real
/// metal box: a guest booting the stale `7.0.0-15-generic` copy against
/// modules built for the box's current `7.0.0-29-generic` failed
/// `finit_module` with `ENOEXEC` ("vsock: disagrees about version of
/// symbol module_layout"). The `.kernel-staged-from` marker (mirroring
/// [`stage_rootfs`]'s own `.rootfs-built-from` marker shape) pins the
/// release the staged copy came from, so a release change invalidates
/// it the same way a rebuilt `overdrive-init` invalidates the rootfs.
///
/// # Errors
///
/// - [`VmFixtureError::UnsupportedHostArch`] — compiled for neither
///   `x86_64` nor `aarch64`.
/// - [`VmFixtureError::KernelImageMissing`] — no
///   `/boot/vmlinuz-$(uname -r)` at all (D5: a genuinely ABSENT file —
///   `io::ErrorKind::NotFound` only).
/// - [`VmFixtureError::KernelImageUnreadable`] — the file exists but
///   could not be READ (permission denied, EIO, …) (D5: a `0600
///   root:root` image is unreadable, not missing — never conflated).
/// - [`VmFixtureError::KernelStagingWriteFailed`] — the validated image
///   could not be copied into `staging_root`, or the release marker
///   could not be written (a write-side failure — disk full, unwritable
///   staging dir — never conflated with a read-side/source failure,
///   D5).
/// - [`VmFixtureError::KernelImageInvalid`] — the host's own kernel
///   image failed [`KernelImage::validate`] on `x86_64` (should not
///   happen on a supported distro kernel; named rather than assumed).
/// - [`VmFixtureError::KernelImageRequiresUkiUnwrap`] — the SAME
///   validation failure on `aarch64` (D8: this fixture stages `vmlinuz`
///   VERBATIM, which is x86_64-only per spike P1's arch split — on
///   aarch64 this is the EXPECTED failure, not a should-never-happen
///   case; see the module doc's "Pinned kernel source" note).
fn stage_kernel(staging_root: &Path) -> Result<PathBuf, VmFixtureError> {
    fs::create_dir_all(staging_root).map_err(|source| {
        VmFixtureError::staging_dir_unusable(staging_root.to_path_buf(), source)
    })?;
    let dest = staging_root.join("kernel");
    let marker_path = staging_root.join(".kernel-staged-from");

    let release = host_kernel_release()?;

    let already_current = kernel_is_valid(&dest)
        && fs::read_to_string(&marker_path).is_ok_and(|marker| marker == release);
    if already_current {
        return Ok(dest);
    }

    let source = PathBuf::from(format!("/boot/vmlinuz-{release}"));

    let header = read_header(&source).map_err(|io_source| {
        classify_kernel_read_error(source.clone(), release.clone(), io_source)
    })?;
    let arch = host_arch()?;
    KernelImage::validate(source.clone(), arch, &header).map_err(|validate_source| {
        if arch == HostArch::Aarch64 {
            VmFixtureError::kernel_image_requires_uki_unwrap(source.clone(), validate_source)
        } else {
            VmFixtureError::kernel_image_invalid(source.clone(), validate_source)
        }
    })?;

    fs::copy(&source, &dest).map_err(|io_source| {
        VmFixtureError::kernel_staging_write_failed(dest.clone(), io_source)
    })?;

    fs::write(&marker_path, &release).map_err(|io_source| {
        VmFixtureError::kernel_staging_write_failed(marker_path.clone(), io_source)
    })?;

    Ok(dest)
}

/// Classifies a failure READING the candidate kernel image at `path`
/// into the two distinct modes `stage_kernel` can report (D5): a
/// genuinely absent file names [`VmFixtureError::KernelImageMissing`];
/// any other read failure (permission denied, EIO, …) names
/// [`VmFixtureError::KernelImageUnreadable`] instead. A `0600 root:root`
/// `/boot/vmlinuz-*` read as an unprivileged user must never be reported
/// as "missing" when it genuinely exists.
fn classify_kernel_read_error(
    path: PathBuf,
    kernel_release: String,
    source: io::Error,
) -> VmFixtureError {
    match source.kind() {
        io::ErrorKind::NotFound => {
            VmFixtureError::kernel_image_missing(path, kernel_release, source)
        }
        _ => VmFixtureError::kernel_image_unreadable(path, kernel_release, source),
    }
}

/// `true` iff `path` already holds a kernel image that validates for this
/// host's architecture — the idempotency check `stage_kernel` uses to
/// skip re-copying.
fn kernel_is_valid(path: &Path) -> bool {
    let Ok(header) = read_header(path) else { return false };
    let Ok(arch) = host_arch() else { return false };
    KernelImage::validate(path.to_path_buf(), arch, &header).is_ok()
}

/// Reads up to [`KERNEL_MAGIC_WINDOW`] bytes — exactly what
/// [`KernelImage::validate`] needs and no more.
fn read_header(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let file = fs::File::open(path)?;
    let mut buf = Vec::new();
    file.take(KERNEL_MAGIC_WINDOW as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

/// `uname -r`'s trimmed output — the running kernel release, used to
/// locate `/boot/vmlinuz-<release>`.
fn host_kernel_release() -> Result<String, VmFixtureError> {
    let output = Command::new("uname")
        .arg("-r")
        .output()
        .map_err(|source| VmFixtureError::spawn("uname -r", source))?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Stages (or re-verifies) the ext4 rootfs at
/// `<staging_root>/rootfs.ext4`, with a freshly-built static-musl
/// `overdrive-init` baked in at BOTH [`GUEST_INIT_SBIN_PATH`] (`/sbin/
/// init` — the path a no-`init=` boot actually reaches, D1) and
/// [`GUEST_INIT_PATH`] (`/init` — reached only under an explicit
/// `init=/init` cmdline).
///
/// # Errors
///
/// Any of [`VmFixtureError::OverdriveInitBuildFailed`],
/// [`VmFixtureError::RootfsStagingFailed`],
/// [`VmFixtureError::RootfsBuildFailed`], or [`VmFixtureError::Spawn`].
fn stage_rootfs(staging_root: &Path) -> Result<PathBuf, VmFixtureError> {
    let init_bin = build_overdrive_init_static()?;
    // The SAME `uname -r` used both to derive the invalidation
    // signature below AND to locate the `.ko.zst` sources
    // `build_staging_tree` stages (ADR-0082 §D4 amendment 2026-08-14) —
    // computed once here so the two can never skew against each other.
    let kernel_release = host_kernel_release()?;
    let signature = format!("{}:{kernel_release}", compute_init_signature(&init_bin)?);

    let rootfs_path = staging_root.join("rootfs.ext4");
    let marker_path = staging_root.join(".rootfs-built-from");

    let already_current = rootfs_image_is_valid(&rootfs_path)
        && fs::read_to_string(&marker_path).is_ok_and(|marker| marker == signature);
    if already_current {
        return Ok(rootfs_path);
    }

    // D4: invalidate the marker BEFORE starting a rebuild -- no
    // partially-staged artifact is ever left claiming validity, even if
    // this attempt itself fails partway through.
    if marker_path.exists() {
        fs::remove_file(&marker_path).map_err(|source| {
            VmFixtureError::rootfs_staging_failed(
                format!("invalidating stale idempotency marker at {}", marker_path.display()),
                source,
            )
        })?;
    }

    let stage_dir = staging_root.join("rootfs-stage");
    build_staging_tree(&stage_dir, &init_bin, &kernel_release)?;
    mkfs_rootfs_image(&stage_dir, &rootfs_path)?;

    fs::write(&marker_path, &signature).map_err(|source| {
        VmFixtureError::rootfs_staging_failed(
            format!("writing idempotency marker at {}", marker_path.display()),
            source,
        )
    })?;

    Ok(rootfs_path)
}

/// `<len>:<mtime-secs>:<mtime-nanos>` — the `overdrive-init` binary's
/// own contribution to the idempotency signature [`stage_rootfs`]
/// compares against the persisted marker to decide whether a rebuild is
/// needed. Carries nanosecond resolution (D11): whole-second
/// [`MetadataExt::mtime`] alone could falsely reuse a stale image when
/// `overdrive-init` is rebuilt more than once within the same
/// wall-clock second.
///
/// [`stage_rootfs`] appends the host's `uname -r` release to THIS
/// function's return value before comparing/persisting (ADR-0082 §D4
/// amendment 2026-08-14) — a kernel change invalidates the staged
/// `.ko` set [`build_staging_tree`] stages, so it must invalidate the
/// rootfs too. This function's own contract (the binary-artifact
/// signature) is unchanged; the composition happens at the call site.
fn compute_init_signature(init_bin: &Path) -> Result<String, VmFixtureError> {
    let meta = fs::metadata(init_bin).map_err(|source| {
        VmFixtureError::rootfs_staging_failed(
            format!("reading metadata for built overdrive-init at {}", init_bin.display()),
            source,
        )
    })?;
    Ok(format!("{}:{}:{}", meta.len(), meta.mtime(), meta.mtime_nsec()))
}

/// `true` iff `path` is a plausible ext4 image: present, long enough to
/// carry a superblock, and holding the ext4 superblock magic (`0xEF53`
/// at byte offset 1080). Mirrors [`kernel_is_valid`]'s
/// re-validate-on-reuse discipline (D10) rather than trusting the
/// idempotency marker file alone — a marker can outlive the image it
/// describes (truncation, an out-of-band `rm`, disk corruption), so
/// `stage_rootfs` reads the actual bytes before skipping a rebuild.
fn rootfs_image_is_valid(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(meta) = fs::metadata(path) else { return false };
    if meta.len() < EXT4_SUPERBLOCK_MAGIC_OFFSET + EXT4_SUPERBLOCK_MAGIC.len() as u64 {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else { return false };
    if file.seek(SeekFrom::Start(EXT4_SUPERBLOCK_MAGIC_OFFSET)).is_err() {
        return false;
    }
    let mut magic = [0_u8; 2];
    file.read_exact(&mut magic).is_ok() && magic == EXT4_SUPERBLOCK_MAGIC
}

/// Clears and rebuilds the staging tree `mkfs.ext4 -d` will populate the
/// image from: the kernel-required empty mountpoints, `overdrive-init`
/// baked in at both in-guest init paths (D1), the guest vsock transport
/// modules `overdrive-init` `finit_module`-loads before dialing the
/// beacon (ADR-0082 §D4 amendment 2026-08-14, DISTILL DWD-21), and the
/// two static device nodes PID 1 needs before devtmpfs is up.
///
/// `kernel_release` is the SAME `uname -r` [`stage_kernel`] copied
/// `/boot/vmlinuz-<release>` from (computed once, by [`stage_rootfs`],
/// and threaded through here) — the `.ko.zst` sources
/// [`stage_vsock_modules`] reads live at
/// `/lib/modules/<release>/kernel/net/vmw_vsock/`, so staging against a
/// DIFFERENT release than the copied kernel would be exactly the
/// rootfs↔kernel skew this fixture's own "Pinned kernel source" design
/// note warns against.
fn build_staging_tree(
    stage_dir: &Path,
    init_bin: &Path,
    kernel_release: &str,
) -> Result<(), VmFixtureError> {
    if stage_dir.exists() {
        fs::remove_dir_all(stage_dir).map_err(|source| {
            VmFixtureError::rootfs_staging_failed(
                format!("clearing prior staging tree at {}", stage_dir.display()),
                source,
            )
        })?;
    }
    for sub in ["proc", "sys", "dev", "tmp", "sbin"] {
        fs::create_dir_all(stage_dir.join(sub)).map_err(|source| {
            VmFixtureError::rootfs_staging_failed(
                format!("creating {sub} under {}", stage_dir.display()),
                source,
            )
        })?;
    }

    // D1: bake overdrive-init in at BOTH reachable paths -- /sbin/init is
    // the load-bearing one (what a no-`init=` boot's kernel fallback
    // search finds first); /init is kept for an explicit `init=/init`
    // cmdline (every spike boot's shape). See the module doc's "Design
    // decisions" section.
    install_guest_init(stage_dir, init_bin, GUEST_INIT_SBIN_PATH)?;
    install_guest_init(stage_dir, init_bin, GUEST_INIT_PATH)?;

    stage_vsock_modules(stage_dir, kernel_release)?;

    mknod_char_device(&stage_dir.join("dev/console"), 5, 1, 0o600)?;
    mknod_char_device(&stage_dir.join("dev/null"), 1, 3, 0o666)?;
    Ok(())
}

/// Stages the three vsock transport `.ko` (ADR-0082 §D4 amendment
/// 2026-08-14, DISTILL DWD-21) into the shared in-guest directory
/// [`overdrive_core::vm::beacon::GUEST_VSOCK_MODULE_DIR`] pins,
/// zstd-decompressed (`finit_module` takes uncompressed ELF, per
/// `overdrive-init`'s loader) from
/// `/lib/modules/<kernel_release>/kernel/net/vmw_vsock/<name>.ko.zst`
/// — the SAME `kernel_release` [`stage_kernel`] copied
/// `/boot/vmlinuz-<release>` from, so the staged modules and the
/// staged kernel can never version-skew. Matches the spike's
/// proven-12/12 mechanism
/// (`spike-scratch/increment-a/build.sh:76-84`).
///
/// A source `.ko.zst` that does not exist for this `kernel_release` is
/// the vsock=y appliance-kernel case (ADR-0068 §4: vsock built in, no
/// modules to stage at all) — SKIPPED, never an error; `overdrive-init`
/// mirrors this exact tolerance on the read side.
///
/// # Errors
///
/// [`VmFixtureError::RootfsStagingFailed`] if the module directory
/// cannot be created, or [`VmFixtureError::Spawn`] /
/// [`VmFixtureError::RootfsStagingFailed`] if `zstd -d` cannot be run
/// or exits non-zero on a source that DOES exist.
fn stage_vsock_modules(stage_dir: &Path, kernel_release: &str) -> Result<(), VmFixtureError> {
    let module_dir = stage_dir.join(GUEST_VSOCK_MODULE_DIR.trim_start_matches('/'));
    fs::create_dir_all(&module_dir).map_err(|source| {
        VmFixtureError::rootfs_staging_failed(
            format!("creating vsock module directory at {}", module_dir.display()),
            source,
        )
    })?;

    for filename in GUEST_VSOCK_MODULE_FILES {
        let src = PathBuf::from(format!(
            "/lib/modules/{kernel_release}/kernel/net/vmw_vsock/{filename}.zst"
        ));
        if !src.exists() {
            continue;
        }
        let dest = module_dir.join(filename);
        let status = Command::new("zstd")
            .arg("-d")
            .arg("-f")
            .arg("-q")
            .arg(&src)
            .arg("-o")
            .arg(&dest)
            .status()
            .map_err(|source| {
                VmFixtureError::spawn(format!("zstd -d {}", src.display()), source)
            })?;
        if !status.success() {
            return Err(VmFixtureError::rootfs_staging_failed(
                format!(
                    "zstd -d {} -o {} exited {:?}",
                    src.display(),
                    dest.display(),
                    status.code()
                ),
                io::Error::other(format!("zstd -d {} failed", src.display())),
            ));
        }
    }
    Ok(())
}

/// Copies `init_bin` into `stage_dir` at the in-guest absolute path
/// `guest_path` (one of [`GUEST_INIT_SBIN_PATH`] / [`GUEST_INIT_PATH`])
/// and marks the copy executable.
fn install_guest_init(
    stage_dir: &Path,
    init_bin: &Path,
    guest_path: &str,
) -> Result<(), VmFixtureError> {
    let dest = stage_dir.join(guest_path.trim_start_matches('/'));
    fs::copy(init_bin, &dest).map_err(|source| {
        VmFixtureError::rootfs_staging_failed(
            format!("installing overdrive-init at {}", dest.display()),
            source,
        )
    })?;
    set_executable(&dest)
}

/// Sizes a fresh 64 MiB sparse image at `rootfs_path` (removing any stale
/// one first) and runs `mkfs.ext4 -d stage_dir` to populate it — 64 MiB
/// matches the spike's proven-sufficient size for a single static binary
/// plus device nodes (`spike-scratch/increment-a/build.sh`).
///
/// # Errors
///
/// [`VmFixtureError::RootfsStagingFailed`], [`VmFixtureError::Spawn`], or
/// [`VmFixtureError::RootfsBuildFailed`].
fn mkfs_rootfs_image(stage_dir: &Path, rootfs_path: &Path) -> Result<(), VmFixtureError> {
    if rootfs_path.exists() {
        fs::remove_file(rootfs_path).map_err(|source| {
            VmFixtureError::rootfs_staging_failed(
                format!("removing stale rootfs image at {}", rootfs_path.display()),
                source,
            )
        })?;
    }
    {
        let image_file = fs::File::create(rootfs_path).map_err(|source| {
            VmFixtureError::rootfs_staging_failed(
                format!("creating rootfs image at {}", rootfs_path.display()),
                source,
            )
        })?;
        image_file.set_len(64 * 1024 * 1024).map_err(|source| {
            VmFixtureError::rootfs_staging_failed(
                format!("sizing rootfs image at {}", rootfs_path.display()),
                source,
            )
        })?;
    }

    let mkfs = Command::new("mkfs.ext4")
        .arg("-F")
        .args(["-L", "overdrive-vm-fixture"])
        .arg("-d")
        .arg(stage_dir)
        .arg(rootfs_path)
        .output()
        .map_err(|source| VmFixtureError::spawn("mkfs.ext4", source))?;
    if !mkfs.status.success() {
        // D4: never leave a partially-populated image masquerading as
        // staged -- best-effort; the caller reports the real failure
        // below regardless of whether this cleanup itself succeeds.
        let _ = fs::remove_file(rootfs_path);
        return Err(VmFixtureError::rootfs_build_failed(
            rootfs_path.to_path_buf(),
            mkfs.status.code(),
            String::from_utf8_lossy(&mkfs.stderr).into_owned(),
        ));
    }
    Ok(())
}

/// `chmod 0755` — `fs::copy` preserves the source's permission bits, but
/// the built `overdrive-init` release binary is not guaranteed executable
/// by every umask, so this is asserted explicitly rather than assumed.
fn set_executable(path: &Path) -> Result<(), VmFixtureError> {
    let mut perms = fs::metadata(path)
        .map_err(|source| {
            VmFixtureError::rootfs_staging_failed(
                format!("reading permissions for {}", path.display()),
                source,
            )
        })?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).map_err(|source| {
        VmFixtureError::rootfs_staging_failed(
            format!("setting permissions on {}", path.display()),
            source,
        )
    })
}

/// `mknod <path> c <major> <minor>` then `chmod <mode> <path>` — the
/// static device nodes a minimal rootfs needs before devtmpfs is up
/// (`/dev/console` as PID 1's fd 0/1/2; `/dev/null` as a general sink).
/// Matches `spike-scratch/increment-a/build.sh`'s proven-working shape
/// exactly.
fn mknod_char_device(path: &Path, major: u32, minor: u32, mode: u32) -> Result<(), VmFixtureError> {
    let status = Command::new("mknod")
        .arg(path)
        .arg("c")
        .arg(major.to_string())
        .arg(minor.to_string())
        .status()
        .map_err(|source| VmFixtureError::spawn("mknod", source))?;
    if !status.success() {
        return Err(VmFixtureError::rootfs_staging_failed(
            format!("mknod {} c {major} {minor} exited {:?}", path.display(), status.code()),
            io::Error::other(format!("mknod {} failed", path.display())),
        ));
    }

    let chmod_status = Command::new("chmod")
        .arg(format!("{mode:o}"))
        .arg(path)
        .status()
        .map_err(|source| VmFixtureError::spawn("chmod", source))?;
    if !chmod_status.success() {
        return Err(VmFixtureError::rootfs_staging_failed(
            format!("chmod {mode:o} {} exited {:?}", path.display(), chmod_status.code()),
            io::Error::other(format!("chmod {} failed", path.display())),
        ));
    }

    Ok(())
}

/// Cross-builds `overdrive-init` for [`musl_target_triple`] in `--release`
/// mode, returning the resulting binary's path. `cargo build`'s own
/// incremental caching makes repeat calls cheap when nothing changed —
/// the idempotency logic in [`stage_rootfs`] is about skipping the
/// (comparatively expensive) `mkfs.ext4` rebuild, not this step.
///
/// **musl cross-compile validated (D3, 2026-08-13).** `cargo xtask metal
/// run -- cargo check -p overdrive-init --target
/// x86_64-unknown-linux-musl --release` completed clean in 8s on the
/// real x86_64 metal box, including every `build.rs` step — the earlier
/// concern that a C-compiled dependency might not cross-compile cleanly
/// for musl is refuted; no C toolchain invocation appeared in the trace.
/// `cargo check` proves the compile graph resolves; the actual LINK this
/// function runs is exercised for the first time when this fixture
/// itself runs, at step 01-06.
///
/// # Errors
///
/// [`VmFixtureError::UnsupportedHostArch`], [`VmFixtureError::Spawn`], or
/// [`VmFixtureError::OverdriveInitBuildFailed`].
fn build_overdrive_init_static() -> Result<PathBuf, VmFixtureError> {
    let target = musl_target_triple()?;
    let root = workspace_root();
    let output = Command::new("cargo")
        .args(["build", "--package", "overdrive-init", "--target", target, "--release"])
        .current_dir(&root)
        .output()
        .map_err(|source| {
            VmFixtureError::spawn(
                format!("cargo build --package overdrive-init --target {target} --release"),
                source,
            )
        })?;
    if !output.status.success() {
        return Err(VmFixtureError::overdrive_init_build_failed(
            target,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(cargo_target_dir(&root).join(target).join("release").join("overdrive-init"))
}

/// The workspace root, resolved at RUNTIME via `cargo locate-project
/// --workspace` (D13) — never `env!("CARGO_MANIFEST_DIR")` alone, which
/// bakes in the COMPILE-TIME path of whichever workspace last compiled
/// this test binary. Conductor runs many workspaces against the SAME
/// Lima VM's cached `target/`; a stale compiled artifact reused from a
/// different workspace would then stage/build `overdrive-init` against
/// the WRONG tree entirely — the exact class documented in
/// `.claude/rules/development.md` § "Stale Lima xtask test artifacts
/// across workspaces".
fn workspace_root() -> PathBuf {
    locate_workspace_root_via_cargo().unwrap_or_else(compile_time_workspace_root_fallback)
}

/// Runs `cargo locate-project --workspace --message-format plain` and
/// returns the workspace root (the located `Cargo.toml`'s parent
/// directory) — `None` if `cargo` could not be run, exited non-zero, or
/// produced no usable path.
fn locate_workspace_root_via_cargo() -> Option<PathBuf> {
    let output = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let manifest_path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if manifest_path.is_empty() {
        return None;
    }
    Path::new(&manifest_path).parent().map(Path::to_path_buf)
}

/// Last-resort fallback when `cargo locate-project` itself could not be
/// run — the pre-D13 compile-time derivation, kept only as a backstop so
/// [`workspace_root`] stays infallible.
fn compile_time_workspace_root_fallback() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Honors an already-set `CARGO_TARGET_DIR` (the Lima/metal wrappers
/// re-inject it — `.claude/rules/testing.md` § "Running tests — Lima
/// VM"), falling back to `<workspace_root>/target`.
fn cargo_target_dir(workspace_root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace_root.join("target"), PathBuf::from)
}

/// Resolves the compiled-for host architecture to the [`HostArch`] this
/// feature supports.
fn host_arch() -> Result<HostArch, VmFixtureError> {
    match std::env::consts::ARCH {
        "x86_64" => Ok(HostArch::X86_64),
        "aarch64" => Ok(HostArch::Aarch64),
        other => Err(VmFixtureError::unsupported_host_arch(other)),
    }
}

/// The static-musl target triple `overdrive-init` is cross-built for on
/// this host (ADR-0082 §D7 / §D1's `{x86_64,aarch64}-unknown-linux-musl`).
fn musl_target_triple() -> Result<&'static str, VmFixtureError> {
    match host_arch()? {
        HostArch::X86_64 => Ok("x86_64-unknown-linux-musl"),
        HostArch::Aarch64 => Ok("aarch64-unknown-linux-musl"),
    }
}

/// Distinct, named failure modes for every step [`VmFixture::provision`]
/// and [`preflight_kvm_capability`] can take.
///
/// Per `.claude/rules/development.md` § "Errors": never collapsed into a
/// shared catch-all — each variant names the exact substrate lie or tool
/// gap an operator (or the next crafter) needs to act on.
#[derive(Debug, thiserror::Error)]
pub enum VmFixtureError {
    /// The exclusive provisioning lock on `<staging_root>/.provision-lock`
    /// could not be acquired (D2) — either the lock FILE itself could not
    /// be opened/created, or `flock(2)` itself failed. Never raised for
    /// lock *contention*, which blocks the caller rather than erroring.
    #[error("could not acquire the provisioning lock at {path}: {source}")]
    ProvisionLockFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// `/dev/kvm` does not exist — no KVM support in this kernel, or the
    /// `kvm`/`kvm_intel`/`kvm_amd` modules are not loaded.
    #[error(
        "/dev/kvm does not exist (no KVM support in this kernel, or kvm/kvm_intel/kvm_amd not loaded): {source}"
    )]
    NoKvmDevice {
        #[source]
        source: io::Error,
    },

    /// `/dev/kvm` exists but the current identity cannot open it
    /// `O_RDWR`. Names the production-realistic shape (`crw-rw----
    /// root:kvm`, mode `0660`) and the observed owner/group/mode so the
    /// remediation (group membership) is actionable without a second
    /// investigation step.
    #[error(
        "/dev/kvm exists but is not opened O_RDWR by this process (expected crw-rw---- \
         root:kvm, mode 0660 -- observed: {observed}; add the running user to the `kvm` \
         group): {source}"
    )]
    KvmPermissionDenied {
        /// `uid=… gid=… mode=…` as observed via `stat(2)` on `/dev/kvm`,
        /// or a note that metadata itself was unreadable.
        observed: String,
        #[source]
        source: io::Error,
    },

    /// Opening `/dev/kvm` failed for a reason other than absence or
    /// permission (e.g. a genuine I/O error) — never silently folded into
    /// one of the two named cases above.
    #[error("opening /dev/kvm failed for a reason other than absence or permission: {source}")]
    KvmDeviceIo {
        #[source]
        source: io::Error,
    },

    /// `systemd-detect-virt` itself could not be spawned — distinct from
    /// "ran and reported a virtualization type" so a missing tool is never
    /// silently read as "not nested".
    #[error("systemd-detect-virt could not be run to check for nested virtualization: {source}")]
    SystemdDetectVirtUnavailable {
        #[source]
        source: io::Error,
    },

    /// `systemd-detect-virt` was spawned successfully but produced no
    /// usable output (blank stdout) — distinct from
    /// [`Self::SystemdDetectVirtUnavailable`] (the tool itself could not
    /// be run) and never silently folded into "not apple" (D12): a
    /// well-behaved `systemd-detect-virt` always prints a virt type or
    /// `"none"`.
    #[error(
        "systemd-detect-virt ran but produced no output (expected a virtualization type or \
         \"none\"); stderr: {stderr}"
    )]
    SystemdDetectVirtEmptyOutput { stderr: String },

    /// The host reports `systemd-detect-virt=apple` — nested virtualization
    /// on Apple Silicon. Cites the spike's own settled finding by name so
    /// this is never mistaken for a driver or Cloud Hypervisor bug.
    #[error(
        "host reports systemd-detect-virt=apple (nested virtualization on Apple Silicon); \
         microVM boots stall roughly 1-in-3 on this substrate and never produce a WRONG \
         answer, only a MISSING one -- see docs/feature/microvm-driver-cloud-hypervisor/\
         spike/findings.md § \"The nested-virt stall -- SETTLED 2026-08-10 by removing the \
         nesting\". kvm-tests must run on real x86_64 hardware via `cargo xtask metal run --`, \
         never a nested Lima VM"
    )]
    NestedAppleHost,

    /// `cloud-hypervisor` could not be spawned at all — not on `PATH`, or
    /// not executable.
    #[error("cloud-hypervisor binary not found or not executable on PATH: {source}")]
    CloudHypervisorMissing {
        #[source]
        source: io::Error,
    },

    /// `cloud-hypervisor` WAS spawned but exited non-zero on `--version`
    /// or `--help` — present but broken (corrupted binary, missing shared
    /// library, …) (D9). Never conflated with [`Self::CloudHypervisorTooOld`],
    /// which requires the binary to have actually run and reported a
    /// clean exit.
    #[error(
        "cloud-hypervisor is present but exited non-zero on `{subcommand}` (exit={status:?}): {stderr}"
    )]
    CloudHypervisorBroken { subcommand: &'static str, status: Option<i32>, stderr: String },

    /// `cloud-hypervisor` ran, but its `--help` output has no `--landlock`
    /// flag. The floor is named against this capability, not a version
    /// number (brief.md §109 / `infra/provision/versions.env`'s own
    /// corrective history) — a build that lacks it cannot honor
    /// ADR-0082 §D5's Landlock probe scenarios regardless of its reported
    /// version string.
    #[error(
        "cloud-hypervisor is present ({version}) but its --help output has no --landlock \
         flag -- the floor is the --landlock/--landlock-rules capability (pinned v53.0 in \
         infra/provision/versions.env), not a version number"
    )]
    CloudHypervisorTooOld {
        /// The raw `--version` output captured before the capability
        /// check failed.
        version: String,
    },

    /// `dir` is writable but does not genuinely support `FICLONE` — an
    /// EXECUTED `cp --reflink=always` against a real file failed. Never
    /// raised from an `fstype` string comparison (correction C-1).
    #[error(
        "staging directory {dir} is not reflink-capable (`cp --reflink=always` failed): {detail}"
    )]
    NotReflinkCapable {
        dir: PathBuf,
        /// The failed clone's captured stderr.
        detail: String,
    },

    /// `dir` could not be created or is not writable at all — distinct
    /// from [`Self::NotReflinkCapable`], which requires the directory to
    /// already be usable.
    #[error("staging directory {dir} could not be created or is unwritable: {source}")]
    StagingDirUnusable {
        dir: PathBuf,
        #[source]
        source: io::Error,
    },

    /// Compiled for neither `x86_64` nor `aarch64` — the only two
    /// [`HostArch`] variants this feature supports.
    #[error(
        "host architecture {arch} is not supported by the Cloud Hypervisor VM driver (only x86_64 and aarch64)"
    )]
    UnsupportedHostArch { arch: String },

    /// No `/boot/vmlinuz-<release>` on this host for the running kernel
    /// release — a genuinely ABSENT file (D5: `io::ErrorKind::NotFound`
    /// only; any other read failure is [`Self::KernelImageUnreadable`]).
    #[error(
        "no kernel image at {path} for the running kernel (uname -r = {kernel_release}): {source}"
    )]
    KernelImageMissing {
        path: PathBuf,
        kernel_release: String,
        #[source]
        source: io::Error,
    },

    /// The candidate kernel image EXISTS but could not be read
    /// (permission denied, EIO, …) — distinct from
    /// [`Self::KernelImageMissing`], which is reserved for a genuinely
    /// absent file (D5).
    #[error(
        "kernel image at {path} exists but could not be read (uname -r = {kernel_release}): {source}"
    )]
    KernelImageUnreadable {
        path: PathBuf,
        kernel_release: String,
        #[source]
        source: io::Error,
    },

    /// The validated kernel image could not be copied into the staging
    /// root — a write-side failure (staging dir unwritable, disk full,
    /// …), never conflated with [`Self::KernelImageMissing`] /
    /// [`Self::KernelImageUnreadable`], which are read-side/source
    /// failures (D5).
    #[error("staging the validated kernel image to {dest} failed: {source}")]
    KernelStagingWriteFailed {
        dest: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The host's own kernel image failed format validation on `x86_64`.
    #[error("kernel image at {path} failed format validation: {source}")]
    KernelImageInvalid {
        path: PathBuf,
        #[source]
        source: KernelFormatError,
    },

    /// The SAME format-validation failure, but on `aarch64` — EXPECTED,
    /// not a surprise (D8). This fixture stages `/boot/vmlinuz-*`
    /// VERBATIM on every arch, but the Ubuntu aarch64 `vmlinuz` is a UKI
    /// wrapping a nested EFI-zboot PE wrapping a zstd-compressed raw
    /// `Image` — CH needs the raw `Image`, and unwrapping both layers is
    /// BYO-artifact tooling this fixture does not implement (see
    /// `spike-scratch/increment-a/build.sh`'s `inspect_kernel.py` +
    /// `zstd -d` dance for the unwrap this host would need).
    #[error(
        "kernel image at {path} failed format validation on aarch64: the Ubuntu aarch64 \
         vmlinuz is a UKI wrapping EFI-zboot wrapping a zstd-compressed raw Image; this \
         fixture only stages the kernel VERBATIM (x86_64-only) and does not unwrap a UKI -- \
         see spike-scratch/increment-a/build.sh's inspect_kernel.py + zstd -d dance for the \
         unwrap this host would need: {source}"
    )]
    KernelImageRequiresUkiUnwrap {
        path: PathBuf,
        #[source]
        source: KernelFormatError,
    },

    /// `cargo build --package overdrive-init --target <musl> --release`
    /// exited non-zero.
    #[error("building overdrive-init for target {target} failed (exit={status:?}): {stderr}")]
    OverdriveInitBuildFailed { target: &'static str, status: Option<i32>, stderr: String },

    /// A filesystem operation while staging the rootfs tree failed (tree
    /// setup, copying `overdrive-init` in, device-node creation, writing
    /// the idempotency marker).
    #[error("rootfs staging failed at {detail}: {source}")]
    RootfsStagingFailed {
        detail: String,
        #[source]
        source: io::Error,
    },

    /// `mkfs.ext4` itself exited non-zero.
    #[error("mkfs.ext4 failed building {path} (exit={status:?}): {stderr}")]
    RootfsBuildFailed { path: PathBuf, status: Option<i32>, stderr: String },

    /// A subprocess this fixture depends on (`uname`, `which`, `mknod`,
    /// `chmod`, `cargo`, …) could not be spawned at all. The command name
    /// is carried in `command` so this generic variant stays diagnosable
    /// without a dedicated variant per tool.
    #[error("spawning `{command}` failed: {source}")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },
}

impl VmFixtureError {
    #[must_use]
    pub const fn provision_lock_failed(path: PathBuf, source: io::Error) -> Self {
        Self::ProvisionLockFailed { path, source }
    }

    #[must_use]
    pub const fn no_kvm_device(source: io::Error) -> Self {
        Self::NoKvmDevice { source }
    }

    #[must_use]
    pub const fn kvm_permission_denied(observed: String, source: io::Error) -> Self {
        Self::KvmPermissionDenied { observed, source }
    }

    #[must_use]
    pub const fn kvm_device_io(source: io::Error) -> Self {
        Self::KvmDeviceIo { source }
    }

    #[must_use]
    pub const fn systemd_detect_virt_unavailable(source: io::Error) -> Self {
        Self::SystemdDetectVirtUnavailable { source }
    }

    #[must_use]
    pub fn systemd_detect_virt_empty_output(stderr: impl Into<String>) -> Self {
        Self::SystemdDetectVirtEmptyOutput { stderr: stderr.into() }
    }

    #[must_use]
    pub const fn cloud_hypervisor_missing(source: io::Error) -> Self {
        Self::CloudHypervisorMissing { source }
    }

    #[must_use]
    pub fn cloud_hypervisor_broken(
        subcommand: &'static str,
        status: Option<i32>,
        stderr: impl Into<String>,
    ) -> Self {
        Self::CloudHypervisorBroken { subcommand, status, stderr: stderr.into() }
    }

    #[must_use]
    pub fn cloud_hypervisor_too_old(version: impl Into<String>) -> Self {
        Self::CloudHypervisorTooOld { version: version.into() }
    }

    #[must_use]
    pub fn not_reflink_capable(dir: PathBuf, detail: impl Into<String>) -> Self {
        Self::NotReflinkCapable { dir, detail: detail.into() }
    }

    #[must_use]
    pub const fn staging_dir_unusable(dir: PathBuf, source: io::Error) -> Self {
        Self::StagingDirUnusable { dir, source }
    }

    #[must_use]
    pub fn unsupported_host_arch(arch: impl Into<String>) -> Self {
        Self::UnsupportedHostArch { arch: arch.into() }
    }

    #[must_use]
    pub const fn kernel_image_missing(
        path: PathBuf,
        kernel_release: String,
        source: io::Error,
    ) -> Self {
        Self::KernelImageMissing { path, kernel_release, source }
    }

    #[must_use]
    pub const fn kernel_image_unreadable(
        path: PathBuf,
        kernel_release: String,
        source: io::Error,
    ) -> Self {
        Self::KernelImageUnreadable { path, kernel_release, source }
    }

    #[must_use]
    pub const fn kernel_staging_write_failed(dest: PathBuf, source: io::Error) -> Self {
        Self::KernelStagingWriteFailed { dest, source }
    }

    #[must_use]
    pub const fn kernel_image_invalid(path: PathBuf, source: KernelFormatError) -> Self {
        Self::KernelImageInvalid { path, source }
    }

    #[must_use]
    pub const fn kernel_image_requires_uki_unwrap(
        path: PathBuf,
        source: KernelFormatError,
    ) -> Self {
        Self::KernelImageRequiresUkiUnwrap { path, source }
    }

    #[must_use]
    pub fn overdrive_init_build_failed(
        target: &'static str,
        status: Option<i32>,
        stderr: impl Into<String>,
    ) -> Self {
        Self::OverdriveInitBuildFailed { target, status, stderr: stderr.into() }
    }

    #[must_use]
    pub fn rootfs_staging_failed(detail: impl Into<String>, source: io::Error) -> Self {
        Self::RootfsStagingFailed { detail: detail.into(), source }
    }

    #[must_use]
    pub fn rootfs_build_failed(
        path: PathBuf,
        status: Option<i32>,
        stderr: impl Into<String>,
    ) -> Self {
        Self::RootfsBuildFailed { path, status, stderr: stderr.into() }
    }

    #[must_use]
    pub fn spawn(command: impl Into<String>, source: io::Error) -> Self {
        Self::Spawn { command: command.into(), source }
    }
}

// ---------------------------------------------------------------------
// Pure-helper unit tests (review remediation 01-04). Real (trivial)
// filesystem I/O against unique per-test scratch paths -- gated behind
// `integration-tests` per `.claude/rules/testing.md` § "Integration vs
// unit gating" ("Real filesystem I/O ... MUST be gated"), even though
// each write is a handful of bytes under `/tmp`. No `tempfile` dev-dep:
// out of this step's file boundary (only `vm_fixture.rs` +
// `overdrive-init/Cargo.toml`), so uniqueness comes from a per-process,
// per-call atomic counter instead, mirroring what `tempfile::TempDir`
// would give per `.claude/rules/testing.md` § "Shared filesystem paths".
//
// There is no runtime acceptance test at this boundary -- the roadmap's
// first real exercise of this fixture is step 01-06 -- so these cover
// only the two genuinely pure-logic helpers the review flagged.
// ---------------------------------------------------------------------
#[cfg(all(test, feature = "integration-tests"))]
mod tests {
    // Mirrors `overdrive-core`'s crate-level `#![cfg_attr(test,
    // allow(clippy::unwrap_used, clippy::expect_used))]`
    // (`crates/overdrive-core/src/lib.rs`) scoped to this module instead
    // -- `overdrive-testing/src/lib.rs` is outside this step's file
    // boundary, so the allow lives here rather than crate-wide.
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn scratch_path(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!("overdrive-vm-fixture-test-{label}-{}-{n}", std::process::id()))
    }

    // -------------------------------------------------------------
    // compute_init_signature (D11 -- nanosecond-resolution signature)
    // -------------------------------------------------------------

    #[test]
    fn compute_init_signature_encodes_len_mtime_and_mtime_nsec() {
        let path = scratch_path("signature");
        fs::write(&path, b"static-musl-binary-bytes").unwrap();
        let meta = fs::metadata(&path).unwrap();

        let signature = compute_init_signature(&path).unwrap();

        let expected = format!("{}:{}:{}", meta.len(), meta.mtime(), meta.mtime_nsec());
        assert_eq!(signature, expected);

        let parts: Vec<&str> = signature.split(':').collect();
        assert_eq!(parts.len(), 3, "signature must carry exactly len:mtime:mtime_nsec");
        assert_eq!(parts[0], meta.len().to_string());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn compute_init_signature_reports_rootfs_staging_failed_when_file_absent() {
        let path = scratch_path("missing");
        let err = compute_init_signature(&path).unwrap_err();
        assert!(
            matches!(err, VmFixtureError::RootfsStagingFailed { .. }),
            "unexpected error variant: {err:?}"
        );
    }

    // -------------------------------------------------------------
    // rootfs_image_is_valid (D10 -- re-verify on reuse, not just the
    // marker)
    // -------------------------------------------------------------

    #[test]
    fn rootfs_image_is_valid_true_only_when_ext4_magic_present_at_offset() {
        let path = scratch_path("valid-ext4");
        let mut bytes = vec![0_u8; 2048];
        let offset = usize::try_from(EXT4_SUPERBLOCK_MAGIC_OFFSET).expect("offset fits usize");
        bytes[offset] = EXT4_SUPERBLOCK_MAGIC[0];
        bytes[offset + 1] = EXT4_SUPERBLOCK_MAGIC[1];
        fs::write(&path, &bytes).unwrap();

        assert!(rootfs_image_is_valid(&path));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rootfs_image_is_valid_false_when_magic_bytes_wrong() {
        let path = scratch_path("bad-magic");
        let bytes = vec![0_u8; 2048]; // zeroed -- no ext4 magic anywhere
        fs::write(&path, &bytes).unwrap();

        assert!(!rootfs_image_is_valid(&path));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rootfs_image_is_valid_false_when_file_too_short_for_the_magic_offset() {
        let path = scratch_path("truncated");
        fs::write(&path, b"too short").unwrap();

        assert!(!rootfs_image_is_valid(&path));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rootfs_image_is_valid_false_when_file_absent() {
        let path = scratch_path("absent");
        assert!(!rootfs_image_is_valid(&path));
    }
}
