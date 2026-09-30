//! netns-density-295 test support: a guest image whose `READY` is
//! deterministically late (E12 (f), "a guest image that delays READY"), and
//! the same image in a variant that powers the guest off before `READY` (the
//! E5 native launch failure after provision and queue attach).
//!
//! # How the delay is made
//!
//! The fixture kernel boots the ext4 root with no `init=` parameter
//! (`KernelCmdline::platform_default`), so the program the kernel runs as
//! PID 1 is `/sbin/init` (`overdrive_testing::vm_fixture` module doc,
//! "`GUEST_INIT_PATH` and `GUEST_INIT_SBIN_PATH`"). [`stage_holding_rootfs`]
//! takes a per-test copy of the shared fixture image, moves the fixture's real
//! `overdrive-init` from `/sbin/init` to [`REAL_INIT_GUEST_PATH`], and installs
//! a tiny static holding init at `/sbin/init`. The shared fixture image is
//! never mutated.
//!
//! The holding init writes [`HOLD_MARKER`] to its console, waits the fixed
//! interval [`HoldingInit::hold`], writes [`RELEASE_MARKER`], and then:
//!
//! * [`HoldingInit::DelayedReady`] `exec`s the real `overdrive-init`, which
//!   remains PID 1 and sends `READY` only after the hold. Until then the
//!   allocation's start is waiting in the boot race, so its TAP stays
//!   provisioned administratively down and no Running row is written.
//! * [`HoldingInit::PowerOffBeforeReady`] powers the guest off without ever
//!   running the real init. Cloud Hypervisor exits before any `READY`, and
//!   the start is rejected through the existing VMM-exit start path, after
//!   the TAP was provisioned and its queue handed to the hypervisor.
//!
//! Both holds are shorter than the driver's pinned boot budget
//! (ADR-0082 §D3, 30 s), so neither variant can end through the boot
//! deadline instead of the path the scenario names.
//!
//! The console is Cloud Hypervisor's serial file
//! (`VmRunDir::console_log`), so a body can read the two markers to prove
//! the guest is still holding when it acts.
//!
//! Test support only: nothing here is production surface, and nothing is
//! installed on the host. `losetup`/`mount` run as root under
//! `cargo xtask metal run --`.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    reason = "Tier-3 guest-image staging fails fast with the precondition it could not meet"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use overdrive_testing::vm_fixture::VmFixture;

use super::guest_stack_mtls_egress::build_static_binary;

/// The driver's boot budget as ADR-0082 §D3 pins it (`vm_driver.rs`
/// `VM_BOOT_DEADLINE`). Each hold below stays under it, so a held guest ends
/// through the path its scenario names, never through the boot deadline.
const PINNED_VM_BOOT_BUDGET: Duration = Duration::from_secs(30);

/// How long [`HoldingInit::DelayedReady`] holds before `exec`ing the real
/// init. It covers the fault a body applies inside the window (the TAP
/// deletion after its witness, at most a few seconds after the hypervisor
/// starts) plus the audit's detection and kill, with margin.
pub(super) const READY_HOLD: Duration = Duration::from_secs(20);

/// How long [`HoldingInit::PowerOffBeforeReady`] holds before powering the
/// guest off: long enough for a body to observe the hypervisor holding the
/// TAP's queue, short enough to keep the launch failure quick.
pub(super) const POWER_OFF_HOLD: Duration = Duration::from_secs(8);

const _: () = assert!(READY_HOLD.as_secs() < PINNED_VM_BOOT_BUDGET.as_secs());
const _: () = assert!(POWER_OFF_HOLD.as_secs() < PINNED_VM_BOOT_BUDGET.as_secs());

/// Console marker the holding init writes before it starts holding.
pub(super) const HOLD_MARKER: &str = "ND295-HOLDING-INIT-HOLD";

/// Console marker the holding init writes when the hold ends, before it
/// `exec`s the real init or powers the guest off.
pub(super) const RELEASE_MARKER: &str = "ND295-HOLDING-INIT-RELEASE";

/// Where the fixture's real `overdrive-init` is moved in the per-test image.
pub(super) const REAL_INIT_GUEST_PATH: &str = "/sbin/overdrive-init.real";

/// `LINUX_REBOOT_CMD_POWER_OFF` (`include/uapi/linux/reboot.h`).
const LINUX_REBOOT_CMD_POWER_OFF: u32 = 0x4321_fedc;

/// What the holding init does when its hold ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HoldingInit {
    /// Hold [`READY_HOLD`], then `exec` the real init: `READY` follows.
    DelayedReady,
    /// Hold [`POWER_OFF_HOLD`], then power off: the real init never runs
    /// and no `READY` is ever sent.
    PowerOffBeforeReady,
}

impl HoldingInit {
    /// The fixed interval this variant holds before acting.
    pub(super) const fn hold(self) -> Duration {
        match self {
            Self::DelayedReady => READY_HOLD,
            Self::PowerOffBeforeReady => POWER_OFF_HOLD,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::DelayedReady => "nd295-delayed-ready-init",
            Self::PowerOffBeforeReady => "nd295-power-off-init",
        }
    }
}

/// The holding init's source: a std-only static program. It never exits,
/// because PID 1 exiting panics the guest kernel (`panic=1` then reboots
/// it), which would turn the fixture into a different fault.
fn holding_init_source(init: HoldingInit) -> String {
    let hold_ms = init.hold().as_millis();
    let label = init.label();
    let release = match init {
        HoldingInit::DelayedReady => format!(
            r#"    console("{RELEASE_MARKER} {label} exec {REAL_INIT_GUEST_PATH}");
    let error = std::process::Command::new("{REAL_INIT_GUEST_PATH}")
        .args(std::env::args_os().skip(1))
        .exec();
    console(&format!("ND295-HOLDING-INIT-EXEC-FAILED {{error}}"));
    park_forever();"#
        ),
        HoldingInit::PowerOffBeforeReady => format!(
            r#"    console("{RELEASE_MARKER} {label} power-off before READY");
    // SAFETY: plain libc calls with no pointers; musl provides both.
    let rc = unsafe {{
        sync();
        reboot({LINUX_REBOOT_CMD_POWER_OFF} as i32)
    }};
    console(&format!("ND295-HOLDING-INIT-POWER-OFF-FAILED rc={{rc}}"));
    park_forever();"#
        ),
    };
    format!(
        r#"use std::io::Write as _;
use std::os::unix::process::CommandExt as _;
use std::time::Duration;

extern "C" {{
    fn sync();
    fn reboot(cmd: i32) -> i32;
}}

fn console(line: &str) {{
    let mut stderr = std::io::stderr();
    if stderr.write_all(line.as_bytes()).is_err() || stderr.write_all(b"\n").is_err() {{
        // PID 1 without a console has nobody to report to; the hold goes on.
    }}
}}

fn park_forever() -> ! {{
    loop {{
        std::thread::sleep(Duration::from_secs(3600));
    }}
}}

fn main() {{
    console("{HOLD_MARKER} {label} hold_ms={hold_ms}");
    std::thread::sleep(Duration::from_millis({hold_ms}));
{release}
}}
"#
    )
}

/// Stage a per-test copy of the shared fixture image whose `/sbin/init` is
/// the `init` holding init, with the fixture's real init moved to
/// [`REAL_INIT_GUEST_PATH`] and each `(host binary, guest name)` in `extra`
/// installed at `/sbin/<guest name>`.
///
/// The fixture bakes the same `overdrive-init` at `/sbin/init` and `/init`;
/// staging refuses an image where the two differ, because then the file it
/// moves aside is not the fixture's real init.
pub(super) fn stage_holding_rootfs(
    tmp: &Path,
    fixture: &VmFixture,
    init: HoldingInit,
    extra: &[(&Path, &str)],
) -> PathBuf {
    let holding_init = build_static_binary(tmp, init.label(), &holding_init_source(init));
    let image = tmp.join(format!("{}-rootfs.ext4", init.label()));
    std::fs::copy(&fixture.rootfs_path, &image)
        .expect("copy the shared fixture rootfs into a per-test holding-init image");

    let mount = LoopMount::attach(&image, &tmp.join(format!("{}-rootfs-mnt", init.label())));
    let root = mount.root().to_path_buf();
    let sbin_init = root.join("sbin").join("init");
    let init_alias = root.join("init");
    let real_init = root.join(REAL_INIT_GUEST_PATH.trim_start_matches('/'));
    let sbin_bytes = std::fs::read(&sbin_init)
        .unwrap_or_else(|error| panic!("read the fixture's {}: {error}", sbin_init.display()));
    let alias_bytes = std::fs::read(&init_alias)
        .unwrap_or_else(|error| panic!("read the fixture's {}: {error}", init_alias.display()));
    assert!(
        !sbin_bytes.is_empty() && sbin_bytes == alias_bytes,
        "the fixture image carries the same overdrive-init at /sbin/init and /init \
         ({} and {} bytes), so /sbin/init is the real init this staging moves aside",
        sbin_bytes.len(),
        alias_bytes.len(),
    );
    std::fs::rename(&sbin_init, &real_init).unwrap_or_else(|error| {
        panic!("move the fixture's real init aside to {REAL_INIT_GUEST_PATH}: {error}")
    });
    install_executable(&holding_init, &sbin_init);
    for (host_binary, guest_name) in extra {
        install_executable(host_binary, &root.join("sbin").join(guest_name));
    }
    mount.release();
    image
}

fn install_executable(source: &Path, destination: &Path) {
    std::fs::copy(source, destination).unwrap_or_else(|error| {
        panic!("install {} at {}: {error}", source.display(), destination.display())
    });
    let mut permissions = std::fs::metadata(destination)
        .unwrap_or_else(|error| panic!("stat {}: {error}", destination.display()))
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(destination, permissions)
        .unwrap_or_else(|error| panic!("chmod 0755 {}: {error}", destination.display()));
}

/// A loop-mounted image. [`LoopMount::release`] unmounts and detaches it and
/// fails the body if either step fails; `Drop` does the same best-effort on a
/// panic path and reports any failure on stderr.
struct LoopMount {
    mountpoint: PathBuf,
    loop_device: String,
    mounted: bool,
    attached: bool,
}

impl LoopMount {
    fn attach(image: &Path, mountpoint: &Path) -> Self {
        std::fs::create_dir_all(mountpoint)
            .unwrap_or_else(|error| panic!("create {}: {error}", mountpoint.display()));
        let output = Command::new("losetup")
            .arg("--find")
            .arg("--show")
            .arg(image)
            .output()
            .expect("spawn losetup --find --show");
        assert!(
            output.status.success(),
            "losetup --find --show {} failed: {}",
            image.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let loop_device = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let mut mount = Self {
            mountpoint: mountpoint.to_path_buf(),
            loop_device,
            mounted: false,
            attached: true,
        };
        let status = Command::new("mount")
            .arg(&mount.loop_device)
            .arg(mountpoint)
            .status()
            .expect("spawn mount");
        assert!(status.success(), "mount {} {} failed", mount.loop_device, mountpoint.display());
        mount.mounted = true;
        mount
    }

    fn root(&self) -> &Path {
        &self.mountpoint
    }

    fn release(mut self) {
        let status = Command::new("umount").arg(&self.mountpoint).status().expect("spawn umount");
        assert!(status.success(), "umount {} failed", self.mountpoint.display());
        self.mounted = false;
        let status = Command::new("losetup")
            .arg("-d")
            .arg(&self.loop_device)
            .status()
            .expect("spawn losetup -d");
        assert!(status.success(), "losetup -d {} failed", self.loop_device);
        self.attached = false;
    }
}

impl Drop for LoopMount {
    fn drop(&mut self) {
        if self.mounted {
            match Command::new("umount").arg(&self.mountpoint).status() {
                Ok(status) if status.success() => self.mounted = false,
                outcome => eprintln!(
                    "[holding-init staging] cleanup umount {} failed: {outcome:?}",
                    self.mountpoint.display()
                ),
            }
        }
        if self.attached && !self.mounted {
            match Command::new("losetup").arg("-d").arg(&self.loop_device).status() {
                Ok(status) if status.success() => self.attached = false,
                outcome => eprintln!(
                    "[holding-init staging] cleanup losetup -d {} failed: {outcome:?}",
                    self.loop_device
                ),
            }
        }
    }
}
