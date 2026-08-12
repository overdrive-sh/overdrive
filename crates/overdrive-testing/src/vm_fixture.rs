//! Real-boot provisioning fixture for the Cloud-Hypervisor microVM driver's
//! Tier-3 tests (`microvm-driver-cloud-hypervisor`, GH #42, roadmap step
//! 01-04).
//!
//! [`VmFixture::provision`] is the SINGLE surface every Tier-3 VM test in
//! this feature calls before attempting a real boot. It:
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
//!    `overdrive-init` baked in at [`GUEST_INIT_PATH`] — AC1.
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
//! kernel, `/boot/vmlinuz-$(uname -r)`, copied verbatim on x86_64. This is
//! not a guess: spike P1 measured exactly this shape as **WORKS** — "the
//! kernel is the distro `vmlinuz` copied verbatim... `file` identifies it
//! as a bzImage and CH loads it directly — no UKI, no EFI-zboot, no zstd"
//! (`docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md`,
//! § P1). The ADR-0068 pinned-appliance-kernel (6.18 LTS) is a distinct,
//! out-of-scope concern — spike P3, which would have exercised THAT
//! kernel, was never run, and building/fetching it is DEVOPS's
//! `infra/metal/provision.sh` territory, not this fixture's.
//!
//! **`GUEST_INIT_PATH`.** `/init`, matching the exact path
//! `spike-scratch/increment-a/build.sh` staged its guest binary at (the
//! same P1-proven recipe) and the Linux kernel's own init-search order:
//! absent an `init=` boot parameter, `kernel_init_freeable()` tries `/init`
//! first regardless of whether root is an initramfs or a real block
//! device. [`overdrive_core::vm::config::KernelCmdline::platform_default`]
//! never sets `init=`, so `/init` is exactly where the kernel looks. No
//! existing constant in the tree names this path; this module is its first
//! and, for now, only definition.
//!
//! **Known gap this step does NOT close.** Ubuntu kernels build
//! `CONFIG_VSOCKETS`/`CONFIG_VIRTIO_VSOCKETS` as *modules*
//! (spike finding `[D2]`), and `overdrive-init` (as landed in step 01-03)
//! has no `finit_module` logic — it goes straight to
//! `socket(AF_VSOCK, ...)`. A guest booted from a verbatim-copied stock
//! kernel may therefore fail to open the beacon socket with
//! `EAFNOSUPPORT` unless the host kernel happens to carry vsock built in.
//! Staging the three `.ko` modules without a loader to `finit_module` them
//! would be inert, so this step deliberately does not attempt it — flagged
//! here for whichever step first drives a REAL guest boot (01-06) to
//! discover and resolve, per this feature's design-context note that
//! "kernel/rootfs staging here is TEST-envelope provisioning" and no more.

#![allow(clippy::doc_markdown)]
#![cfg(target_os = "linux")]

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use overdrive_core::vm::config::{HostArch, KERNEL_MAGIC_WINDOW, KernelFormatError, KernelImage};

/// The in-guest absolute path every fixture rootfs bakes `overdrive-init`
/// into. See the module doc's "Design decisions" section for why `/init`
/// is correct and where the precedent comes from.
pub const GUEST_INIT_PATH: &str = "/init";

/// `/dev/kvm`'s well-known path — named once so the preflight and its
/// diagnostics never drift against each other.
const KVM_DEVICE_PATH: &str = "/dev/kvm";

/// 8 MiB of real, written (non-sparse) bytes — matches `infra/metal/
/// provision.sh`'s own probe size. A sparse file would let `FICLONE`
/// trivially "succeed" with nothing to actually clone, which is exactly
/// the dishonest signal correction C-1 exists to refuse.
const REFLINK_PROBE_BYTES: usize = 8 * 1024 * 1024;

/// Everything a Tier-3 VM test needs to attempt a real Cloud Hypervisor
/// boot.
///
/// A validated, staged kernel image; an ext4 rootfs with
/// `overdrive-init` baked in at [`GUEST_INIT_PATH`]; and the confirmed
/// location/version of a capable `cloud-hypervisor` binary.
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
    /// [`GUEST_INIT_PATH`] (`<staging_root>/rootfs.ext4`).
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
    /// Runs, in order: the KVM/nested-virt preflight, the
    /// `cloud-hypervisor` presence + capability check, the executed
    /// `FICLONE` reflink probe on `staging_root`, kernel staging, and
    /// rootfs staging (which itself builds `overdrive-init` for the
    /// host's static-musl target). Cheap checks run first so an incapable
    /// host fails before any expensive staging work begins.
    ///
    /// # Idempotency
    ///
    /// A second call against the same `staging_root` re-verifies rather
    /// than re-downloads/re-bakes: an already-staged, still-valid kernel
    /// is reused; the rootfs image is rebuilt only when the freshly-built
    /// `overdrive-init` binary differs (by size + mtime) from the one the
    /// last-built image was staged from. The KVM/virt preflight, the
    /// `cloud-hypervisor` check, and the `FICLONE` probe always re-run in
    /// full — they are the trust boundary and re-verifying them is cheap.
    ///
    /// # Errors
    ///
    /// Returns the first [`VmFixtureError`] any step surfaces; no
    /// partially-staged artifact is left masquerading as complete (a
    /// failed rootfs build removes its own stale image before returning).
    pub fn provision(staging_root: &Path) -> Result<Self, VmFixtureError> {
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
fn detect_virt() -> Result<String, VmFixtureError> {
    let output = Command::new("systemd-detect-virt")
        .output()
        .map_err(VmFixtureError::systemd_detect_virt_unavailable)?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
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
/// - [`VmFixtureError::CloudHypervisorTooOld`] — the binary ran, but its
///   `--help` output has no `--landlock` flag.
fn check_cloud_hypervisor() -> Result<(PathBuf, String), VmFixtureError> {
    let version_output = Command::new("cloud-hypervisor")
        .arg("--version")
        .output()
        .map_err(VmFixtureError::cloud_hypervisor_missing)?;
    let version = String::from_utf8_lossy(&version_output.stdout).trim().to_owned();

    let help_output = Command::new("cloud-hypervisor")
        .arg("--help")
        .output()
        .map_err(|source| VmFixtureError::spawn("cloud-hypervisor --help", source))?;
    let help_text = format!(
        "{}{}",
        String::from_utf8_lossy(&help_output.stdout),
        String::from_utf8_lossy(&help_output.stderr),
    );
    if !help_text.contains("--landlock") {
        return Err(VmFixtureError::cloud_hypervisor_too_old(version));
    }

    let bin = resolve_on_path("cloud-hypervisor")?;
    Ok((bin, version))
}

/// Resolves `name` to an absolute path via `which`(1) so the returned
/// [`VmFixture`] carries an inspectable location rather than a bare
/// `PATH`-dependent command name. Falls back to the bare name if `which`
/// itself reports nothing — the earlier spawn already proved the binary
/// IS runnable, so this is best-effort diagnostic enrichment, not a
/// second gate.
fn resolve_on_path(name: &str) -> Result<PathBuf, VmFixtureError> {
    let output = Command::new("which")
        .arg(name)
        .output()
        .map_err(|source| VmFixtureError::spawn(format!("which {name}"), source))?;
    let resolved = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if output.status.success() && !resolved.is_empty() {
        Ok(PathBuf::from(resolved))
    } else {
        Ok(PathBuf::from(name))
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
/// # Errors
///
/// - [`VmFixtureError::UnsupportedHostArch`] — compiled for neither
///   `x86_64` nor `aarch64`.
/// - [`VmFixtureError::KernelImageMissing`] — no
///   `/boot/vmlinuz-$(uname -r)` on this host.
/// - [`VmFixtureError::KernelImageInvalid`] — the host's own kernel image
///   failed [`KernelImage::validate`] (should not happen on a supported
///   distro kernel; named rather than assumed).
fn stage_kernel(staging_root: &Path) -> Result<PathBuf, VmFixtureError> {
    fs::create_dir_all(staging_root).map_err(|source| {
        VmFixtureError::staging_dir_unusable(staging_root.to_path_buf(), source)
    })?;
    let dest = staging_root.join("kernel");

    if kernel_is_valid(&dest) {
        return Ok(dest);
    }

    let release = host_kernel_release()?;
    let source = PathBuf::from(format!("/boot/vmlinuz-{release}"));

    let header = read_header(&source).map_err(|io_source| {
        VmFixtureError::kernel_image_missing(source.clone(), release.clone(), io_source)
    })?;
    let arch = host_arch()?;
    KernelImage::validate(source.clone(), arch, &header).map_err(|validate_source| {
        VmFixtureError::kernel_image_invalid(source.clone(), validate_source)
    })?;

    fs::copy(&source, &dest).map_err(|io_source| {
        VmFixtureError::kernel_image_missing(source.clone(), release.clone(), io_source)
    })?;

    Ok(dest)
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
/// `overdrive-init` baked in at [`GUEST_INIT_PATH`].
///
/// # Errors
///
/// Any of [`VmFixtureError::OverdriveInitBuildFailed`],
/// [`VmFixtureError::RootfsStagingFailed`],
/// [`VmFixtureError::RootfsBuildFailed`], or [`VmFixtureError::Spawn`].
fn stage_rootfs(staging_root: &Path) -> Result<PathBuf, VmFixtureError> {
    let init_bin = build_overdrive_init_static()?;
    let init_meta = fs::metadata(&init_bin).map_err(|source| {
        VmFixtureError::rootfs_staging_failed(
            format!("reading metadata for built overdrive-init at {}", init_bin.display()),
            source,
        )
    })?;
    let signature = format!("{}:{}", init_meta.len(), init_meta.mtime());

    let rootfs_path = staging_root.join("rootfs.ext4");
    let marker_path = staging_root.join(".rootfs-built-from");

    let already_current = rootfs_path.is_file()
        && fs::read_to_string(&marker_path).is_ok_and(|marker| marker == signature);
    if already_current {
        return Ok(rootfs_path);
    }

    let stage_dir = staging_root.join("rootfs-stage");
    if stage_dir.exists() {
        fs::remove_dir_all(&stage_dir).map_err(|source| {
            VmFixtureError::rootfs_staging_failed(
                format!("clearing prior staging tree at {}", stage_dir.display()),
                source,
            )
        })?;
    }
    for sub in ["proc", "sys", "dev", "tmp"] {
        fs::create_dir_all(stage_dir.join(sub)).map_err(|source| {
            VmFixtureError::rootfs_staging_failed(
                format!("creating {sub} under {}", stage_dir.display()),
                source,
            )
        })?;
    }

    let init_dest = stage_dir.join(GUEST_INIT_PATH.trim_start_matches('/'));
    fs::copy(&init_bin, &init_dest).map_err(|source| {
        VmFixtureError::rootfs_staging_failed(
            format!("installing overdrive-init at {}", init_dest.display()),
            source,
        )
    })?;
    set_executable(&init_dest)?;

    mknod_char_device(&stage_dir.join("dev/console"), 5, 1, 0o600)?;
    mknod_char_device(&stage_dir.join("dev/null"), 1, 3, 0o666)?;

    if rootfs_path.exists() {
        fs::remove_file(&rootfs_path).map_err(|source| {
            VmFixtureError::rootfs_staging_failed(
                format!("removing stale rootfs image at {}", rootfs_path.display()),
                source,
            )
        })?;
    }
    {
        let image_file = fs::File::create(&rootfs_path).map_err(|source| {
            VmFixtureError::rootfs_staging_failed(
                format!("creating rootfs image at {}", rootfs_path.display()),
                source,
            )
        })?;
        // 64 MiB — matches the spike's proven-sufficient size for a
        // single static binary plus device nodes
        // (`spike-scratch/increment-a/build.sh`).
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
        .arg(&stage_dir)
        .arg(&rootfs_path)
        .output()
        .map_err(|source| VmFixtureError::spawn("mkfs.ext4", source))?;
    if !mkfs.status.success() {
        return Err(VmFixtureError::rootfs_build_failed(
            rootfs_path,
            mkfs.status.code(),
            String::from_utf8_lossy(&mkfs.stderr).into_owned(),
        ));
    }

    fs::write(&marker_path, &signature).map_err(|source| {
        VmFixtureError::rootfs_staging_failed(
            format!("writing idempotency marker at {}", marker_path.display()),
            source,
        )
    })?;

    Ok(rootfs_path)
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

/// The workspace root, derived from this crate's own manifest directory
/// at compile time (`crates/overdrive-testing` → its grandparent).
fn workspace_root() -> PathBuf {
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
    /// release.
    #[error(
        "no kernel image at {path} for the running kernel (uname -r = {kernel_release}): {source}"
    )]
    KernelImageMissing {
        path: PathBuf,
        kernel_release: String,
        #[source]
        source: io::Error,
    },

    /// The host's own kernel image failed format validation.
    #[error("kernel image at {path} failed format validation: {source}")]
    KernelImageInvalid {
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
    pub const fn cloud_hypervisor_missing(source: io::Error) -> Self {
        Self::CloudHypervisorMissing { source }
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
    pub const fn kernel_image_invalid(path: PathBuf, source: KernelFormatError) -> Self {
        Self::KernelImageInvalid { path, source }
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
