//! Tier-3 — the fork launch with the VMM-opened vhost-vsock device
//! (netns-density-295, CID-NOCLAIM, RESET-EXIT; GH #295).
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md`
//! § *VMM backend contract — `overdrive-host::CloudHypervisorVmm`* and
//! § *VMM fork provisioning contract*; obligations R5-55 (b), (c) and R5-56
//! at the VMM port:
//!
//! - a launch carries exactly one `--vsock cid=<cid>,backend=vhost-kernel`;
//!   the VMM opens `/dev/vhost-vsock` itself and is the only holder of that
//!   file — no Overdrive process holds a copy, before or after `create`
//!   returns (V-30 Q2c);
//! - the VMM's exit is the device's last close, so the CID is free at once
//!   (V-30 Q2a / Q2d): a later launch on the same CID opens the device;
//! - a launch whose CID another live VMM holds cannot set up its device and
//!   the VMM exits before READY; the first holder is unaffected (K-C1,
//!   `drivers/vhost/vsock.c:807-813 @ v7.0`);
//! - a guest reset — a PID-1 crash under `panic=1`, a triple fault
//!   (`reboot=t`), a forced reboot (`reboot -f`, the `reboot(2)` restart
//!   command) — ends the VMM instead of rebooting the guest in place
//!   (RESET-EXIT, ADR-0182): the guest kernel boots once per VMM and the
//!   device goes with the VMM.
//!
//! Driving port: `Vmm::create` / `Vmm::terminate` of the production
//! `CloudHypervisorVmm`. Observable layer: the kernel's process and
//! descriptor tables (`/proc/<pid>/fd`), the VMM's own ending
//! (`VmExitWatch`) and the guest's serial console (the kernel's boot banner,
//! one per guest boot). Example-based (Mandate 9, layer 3+); each case is a
//! named sad or boundary path. The driver's classification of these exits
//! (`VmGuestExitUnreported` before READY, `WorkloadCrashedImmediately`
//! after) and the restart budget are the composed cells of
//! `overdrive-cli`'s `vsock_fault_matrix`.
//!
//! Lane: `integration-tests,kvm-tests`, native x86_64 metal only
//! (`cargo xtask metal run -- cargo nextest run -p overdrive-host --features
//! integration-tests,kvm-tests`). The device cases need the pinned fork and
//! its launch argument (DELIVER step 3); the reset cases need the fork
//! commit that carries RESET-EXIT (DELIVER step 1).
//!
//! Fault bounds and cleanup: every case launches at most two VMMs and ends
//! each within its named bound; a [`KillOnDrop`] guard sends `SIGKILL` to a
//! VMM a failed assertion left running (the adapter spawns with
//! `kill_on_drop(false)`), so no VMM outlives the case.
//!
//! CID band: `GUEST_CID_BASE + 0x0400 ..= + 0x0408`, disjoint from the
//! other `kvm-tests` files of this binary (`0x0300`, `0x0310..=0x0317`,
//! `0x0320`), so no two cases ever launch on one CID.

#![allow(
    clippy::doc_markdown,
    reason = "per-test CONTRACT_SHAPE declarations and kernel symbol names in prose"
)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU8;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::guest_transport::{GUEST_CID_BASE, GuestCid};
use overdrive_core::traits::vmm::{VmProcess, Vmm};
use overdrive_core::vm::config::{
    HostArch, KERNEL_MAGIC_WINDOW, KernelCmdline, KernelImage, MemoryPlan, OVERDRIVE_VMM_UID,
    RootfsPlan, VmConfig, VmConfinement, VmRunDir,
};
use overdrive_host::CloudHypervisorVmm;
use overdrive_testing::guest_kit::{
    GuestFile, GuestImageSpec, GuestKit, SCRIPTED_BOOT_BOUND, WorkDir,
};
use overdrive_testing::guest_peer::{CONTROL_PORT_PARAM, GuestPeerControl};
use overdrive_testing::vm_fixture::{VmFixture, default_staging_root};
use overdrive_testing::vsock::{VMADDR_CID_HOST, VMADDR_PORT_ANY, VsockStream};
use serial_test::serial;

/// The kernel's vhost-vsock character device.
const VHOST_VSOCK: &str = "/dev/vhost-vsock";

/// Small guest RAM, as the sibling `kvm-tests` files use.
const GUEST_BYTES: u64 = 128 * 1024 * 1024;

/// How long the VMM may take to open its device after `create` returns.
const DEVICE_OPEN_BOUND: Duration = Duration::from_secs(5);

/// How long a launch refused at device setup may take to exit.
const REFUSED_LAUNCH_EXIT_BOUND: Duration = Duration::from_secs(10);

/// How long a guest reset may take to end the VMM: kernel boot to the panic,
/// `panic=1`'s one-second wait, then the reset.
const RESET_EXIT_BOUND: Duration = Duration::from_secs(30);

/// The guest kernel prints this banner once per boot (`init/main.c`,
/// `linux_banner`), on the serial console the launch captures.
const KERNEL_BOOT_BANNER: &str = "Linux version ";

fn cid(offset: u32) -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + offset).expect("an in-range guest CID")
}

fn read_kernel_header(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("open staged kernel for header read");
    let mut buf = Vec::new();
    file.take(KERNEL_MAGIC_WINDOW as u64).read_to_end(&mut buf).expect("read kernel header");
    buf
}

/// A launch config over the fixture's kernel and `rootfs`. `extra` are
/// platform kernel parameters appended to the platform default (the reset
/// cases make the guest's PID 1 fail and pick the reset method).
fn launch_config_on(
    fixture: &VmFixture,
    rootfs: &Path,
    run_root: &Path,
    alloc: &str,
    extra: &[&str],
) -> VmConfig {
    let alloc = AllocationId::new(alloc).expect("valid alloc id");
    let header = read_kernel_header(&fixture.kernel_path);
    let kernel = KernelImage::validate(fixture.kernel_path.clone(), HostArch::X86_64, &header)
        .expect("fixture-staged kernel validates for x86_64");
    let master_bytes = std::fs::metadata(rootfs).expect("stat staged rootfs").len();
    let mut cmdline = KernelCmdline::platform_default(HostArch::X86_64);
    for token in extra {
        assert!(cmdline.append_platform_token(token), "fixture kernel parameter {token} appends");
    }
    let config = VmConfig {
        alloc: alloc.clone(),
        kernel,
        rootfs: RootfsPlan::for_alloc(
            rootfs.to_path_buf(),
            master_bytes,
            &alloc,
            run_root,
            Path::new("/run/overdrive/vm/clone-index"),
        ),
        cmdline,
        memory: MemoryPlan::derive(GUEST_BYTES),
        vcpus: NonZeroU8::new(1).expect("1 is nonzero"),
        run_dir: VmRunDir::for_alloc(run_root, &alloc),
        confinement: VmConfinement::confined(super::production_vmm_identity(), 1024),
        cgroup_scope: CgroupPath::for_alloc(&alloc),
    };
    std::fs::create_dir_all(config.run_dir.path()).expect("create run dir");
    config
}

fn launch_config(fixture: &VmFixture, run_root: &Path, alloc: &str, extra: &[&str]) -> VmConfig {
    launch_config_on(fixture, &fixture.rootfs_path, run_root, alloc, extra)
}

fn fixture_and_run_root(case: &str) -> (VmFixture, PathBuf) {
    let staging_root = default_staging_root();
    let fixture = VmFixture::provision(&staging_root).expect("fixture provisions on this host");
    let run_root = staging_root.join(format!("run-vsock-device-{case}"));
    (fixture, run_root)
}

fn proc_status_numbers(path: &Path, field: &str) -> Vec<u32> {
    let status = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let values = status
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .unwrap_or_else(|| panic!("{field} is present in {}", path.display()));
    values
        .split_whitespace()
        .map(|value| {
            value.parse().unwrap_or_else(|error| panic!("parse {field} value {value:?}: {error}"))
        })
        .collect()
}

fn process_args(pid: u32) -> Vec<String> {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .unwrap_or_else(|error| panic!("read /proc/{pid}/cmdline: {error}"))
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8_lossy(arg).into_owned())
        .collect()
}

/// Sends `SIGKILL` to a VMM the case left running when it unwinds; a VMM the
/// case ended is gone and the signal is refused harmlessly.
struct KillOnDrop(u32);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Ok(pid) = libc::pid_t::try_from(self.0) {
            // SAFETY: `kill` takes plain integers and retains nothing.
            let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

/// The number of descriptors of process `pid` that are open on
/// `/dev/vhost-vsock`. `None` when the process no longer exists.
fn vhost_vsock_fds(pid: u32) -> Option<usize> {
    let entries = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    Some(
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
            .filter(|target| target == Path::new(VHOST_VSOCK))
            .count(),
    )
}

/// `/dev/vhost-vsock` descriptors held by this test process (the process
/// that called `Vmm::create`, standing for every Overdrive process).
fn own_vhost_vsock_fds() -> usize {
    vhost_vsock_fds(std::process::id()).expect("the test process exists")
}

/// Polls until the VMM `pid` holds a vhost-vsock descriptor, or the bound.
async fn wait_until_vmm_holds_its_device(pid: u32) -> usize {
    let deadline = Instant::now() + DEVICE_OPEN_BOUND;
    loop {
        match vhost_vsock_fds(pid) {
            Some(held) if held > 0 => return held,
            Some(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            other => panic!(
                "VMM {pid} did not open {VHOST_VSOCK} within {DEVICE_OPEN_BOUND:?}: {other:?}"
            ),
        }
    }
}

async fn ends_within(process: &mut VmProcess, bound: Duration) -> bool {
    tokio::time::timeout(bound, process.exit.recv()).await.is_ok()
}

/// How many times the guest kernel booted under the VMM of `config`: the
/// boot banners on its serial console.
fn kernel_boots(config: &VmConfig) -> usize {
    let console = std::fs::read_to_string(config.run_dir.console_log())
        .unwrap_or_else(|error| panic!("read {}: {error}", config.run_dir.console_log().display()));
    console.matches(KERNEL_BOOT_BANNER).count()
}

/// Cross-builds a static-musl guest init that calls `reboot(2)` with the
/// restart command (`RB_AUTOBOOT`, the `reboot -f` path), staged at
/// `/sbin/reboot-now` in a per-case copy of the fixture rootfs (the shared
/// image is never mutated). Runs as root, as the whole lane does.
fn rootfs_with_forced_reboot_init(fixture: &VmFixture, dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).expect("create the case's staging dir");
    let source = dir.join("reboot-now.rs");
    std::fs::write(
        &source,
        r#"extern "C" {
    fn reboot(cmd: i32) -> i32;
}
fn main() {
    const RB_AUTOBOOT: i32 = 0x0123_4567;
    // SAFETY: a plain syscall wrapper; on success it does not return.
    let _ = unsafe { reboot(RB_AUTOBOOT) };
    std::process::exit(99);
}
"#,
    )
    .expect("write the forced-reboot init source");
    let binary = dir.join("reboot-now");
    let built = Command::new("rustc")
        .args(["--edition", "2021", "-C", "opt-level=0", "-C", "target-feature=+crt-static"])
        .args(["--target", "x86_64-unknown-linux-musl", "-o"])
        .arg(&binary)
        .arg(&source)
        .status()
        .expect("spawn rustc for the forced-reboot init");
    assert!(built.success(), "rustc builds the static forced-reboot init");

    let rootfs = dir.join("rootfs.ext4");
    std::fs::copy(&fixture.rootfs_path, &rootfs).expect("copy the fixture rootfs");
    let mount = dir.join("rootfs-mnt");
    std::fs::create_dir_all(&mount).expect("create the loop mount point");
    let attached = Command::new("losetup")
        .args(["--find", "--show"])
        .arg(&rootfs)
        .output()
        .expect("spawn losetup");
    assert!(attached.status.success(), "losetup: {}", String::from_utf8_lossy(&attached.stderr));
    let device = String::from_utf8_lossy(&attached.stdout).trim().to_owned();
    assert!(
        Command::new("mount").arg(&device).arg(&mount).status().expect("spawn mount").success(),
        "mount {device}"
    );
    let staged = mount.join("sbin").join("reboot-now");
    std::fs::copy(&binary, &staged).expect("stage the forced-reboot init");
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
        .expect("make the forced-reboot init executable");
    assert!(
        Command::new("umount").arg(&mount).status().expect("spawn umount").success(),
        "umount {}",
        mount.display()
    );
    assert!(
        Command::new("losetup")
            .arg("-d")
            .arg(&device)
            .status()
            .expect("spawn losetup -d")
            .success(),
        "detach {device}"
    );
    rootfs
}

/// CID-NOCLAIM: the VMM opens its CID's device itself and is its only
/// holder. Before `create`, during the run and after `create` returned, this
/// process holds no vhost-vsock descriptor; the VMM holds exactly one.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[serial(vhost_vsock)]
async fn the_vmm_opens_its_own_vsock_device_and_overdrive_holds_no_copy() {
    let (fixture, run_root) = fixture_and_run_root("own-device");
    let config = launch_config(&fixture, &run_root, "vsock-dev-own", &[]);
    let vmm = CloudHypervisorVmm::new();
    assert_eq!(own_vhost_vsock_fds(), 0, "precondition: no Overdrive vhost-vsock descriptor");

    let process = vmm.create(&config, cid(0x0400)).await.expect("the fork launches");
    let pid = process.control.pid;
    let _kill = KillOnDrop(pid);

    let held = wait_until_vmm_holds_its_device(pid).await;
    assert_eq!(held, 1, "the VMM holds exactly one vhost-vsock device");
    assert_eq!(own_vhost_vsock_fds(), 0, "no Overdrive process holds a copy of the VMM's device");

    vmm.terminate(&process.control, Duration::ZERO).await.expect("terminate");
    assert_eq!(own_vhost_vsock_fds(), 0, "still none after the VMM exited");
}

/// CID-NOCLAIM: the VMM's exit is the device's last close, so its CID is
/// free at once — the next launch on the same CID opens the device and keeps
/// running.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(vhost_vsock)]
async fn the_cid_is_free_at_once_after_its_vmm_exits() {
    let (fixture, run_root) = fixture_and_run_root("cid-free");
    let vmm = CloudHypervisorVmm::new();

    let first_config = launch_config(&fixture, &run_root, "vsock-dev-first", &[]);
    let first = vmm.create(&first_config, cid(0x0401)).await.expect("the first launch");
    let _kill_first = KillOnDrop(first.control.pid);
    wait_until_vmm_holds_its_device(first.control.pid).await;
    vmm.terminate(&first.control, Duration::ZERO).await.expect("terminate the first VMM");
    assert_eq!(vhost_vsock_fds(first.control.pid), None, "the first VMM is gone");

    let second_config = launch_config(&fixture, &run_root, "vsock-dev-second", &[]);
    let mut second = vmm.create(&second_config, cid(0x0401)).await.expect("the second launch");
    let _kill_second = KillOnDrop(second.control.pid);
    let held = wait_until_vmm_holds_its_device(second.control.pid).await;
    assert_eq!(held, 1, "the second VMM opened the device of the freed CID");
    assert!(
        !ends_within(&mut second, Duration::from_millis(500)).await,
        "a launch on a freed CID does not exit at device setup"
    );
    vmm.terminate(&second.control, Duration::ZERO).await.expect("terminate the second VMM");
}

/// K-C1: a launch on a CID another live VMM holds cannot set up its device
/// and its VMM exits before READY (the driver's existing pre-READY VMM-exit
/// start failure); the first holder keeps running and keeps its device.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(vhost_vsock)]
async fn a_launch_on_a_held_cid_exits_before_ready_and_leaves_the_holder_running() {
    let (fixture, run_root) = fixture_and_run_root("held-cid");
    let vmm = CloudHypervisorVmm::new();

    let holder_config = launch_config(&fixture, &run_root, "vsock-dev-holder", &[]);
    let holder = vmm.create(&holder_config, cid(0x0402)).await.expect("the holder launches");
    let _kill_holder = KillOnDrop(holder.control.pid);
    wait_until_vmm_holds_its_device(holder.control.pid).await;

    let clash_config = launch_config(&fixture, &run_root, "vsock-dev-clash", &[]);
    let mut clash =
        vmm.create(&clash_config, cid(0x0402)).await.expect("the spawn itself succeeds");
    let _kill_clash = KillOnDrop(clash.control.pid);
    assert!(
        ends_within(&mut clash, REFUSED_LAUNCH_EXIT_BOUND).await,
        "a VMM refused its CID exits within {REFUSED_LAUNCH_EXIT_BOUND:?}"
    );
    assert_eq!(
        vhost_vsock_fds(holder.control.pid),
        Some(1),
        "the first holder is unaffected: alive and still holding its device"
    );
    vmm.terminate(&holder.control, Duration::ZERO).await.expect("terminate the holder");
}

/// V-1(c) / A-17: the fork opens its vhost-vsock device, boots and carries
/// bidirectional control data under the production numeric uid, the device
/// groups, the ADR-0129 child hook, the ADR-0143 launch filter and Cloud
/// Hypervisor Landlock. Successful data after the device open is the oracle
/// that neither Landlock nor the filter ended the VMM.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial(vhost_vsock)]
#[allow(
    clippy::print_stderr,
    reason = "V-1(c) requires an executed native receipt with identity and filter evidence"
)]
async fn the_fork_boots_and_passes_data_under_the_production_launch_identity() {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("the prepared guest kit provisions");
    let work = WorkDir::create(&staging, "v1c-production-identity").expect("work dir");
    let mut image_spec = GuestImageSpec::scripted();
    image_spec.forwarder_module = false;
    let image = kit
        .image(work.path(), "v1c-peer", &image_spec)
        .expect("the scripted image needs no future forwarder behavior");
    let guest_cid = cid(0x0406).get();
    let control = GuestPeerControl::bind(VMADDR_PORT_ANY).expect("bind a kernel-assigned port");
    let control_arg = format!("{CONTROL_PORT_PARAM}{}", control.port());
    let fixture = VmFixture::provision(&staging).expect("the VM fixture provisions");
    let config = launch_config_on(
        &fixture,
        &image.rootfs,
        &work.path().join("run"),
        "v1c-production-identity",
        &[&control_arg],
    );
    let vmm = CloudHypervisorVmm::new().with_binary(fixture.cloud_hypervisor_bin);
    let mut process = vmm
        .create(&config, cid(0x0406))
        .await
        .expect("the fork opens /dev/vhost-vsock and boots under production confinement");
    let pid = process.control.pid;
    let _kill = KillOnDrop(pid);
    let mut peer = control
        .accept_peer(guest_cid, SCRIPTED_BOOT_BOUND)
        .expect("the guest dials the host over the fork-opened device");

    let passwd = std::fs::read_to_string("/etc/passwd").expect("read the metal host passwd file");
    assert!(
        !passwd.lines().any(|line| {
            line.split(':').nth(2).and_then(|uid| uid.parse::<u32>().ok())
                == Some(OVERDRIVE_VMM_UID)
        }),
        "the production numeric uid has no passwd entry; setpriv must use raw ids"
    );

    let kvm_gid = std::fs::metadata("/dev/kvm").expect("stat /dev/kvm").gid();
    let vhost_gid = std::fs::metadata(VHOST_VSOCK).expect("stat /dev/vhost-vsock").gid();
    let status_path = PathBuf::from(format!("/proc/{pid}/status"));
    assert_eq!(proc_status_numbers(&status_path, "Uid:"), vec![OVERDRIVE_VMM_UID; 4]);
    assert_eq!(proc_status_numbers(&status_path, "Gid:"), vec![OVERDRIVE_VMM_UID; 4]);
    let mut expected_groups = vec![kvm_gid];
    if vhost_gid != kvm_gid {
        expected_groups.push(vhost_gid);
    }
    assert_eq!(proc_status_numbers(&status_path, "Groups:"), expected_groups);

    let args = process_args(pid);
    assert!(args.iter().any(|arg| arg == "--landlock"), "Landlock is active: {args:?}");
    assert!(
        args.windows(2).any(|pair| pair[0] == "--seccomp" && pair[1] == "true"),
        "Cloud Hypervisor's own seccomp is active: {args:?}"
    );
    let vsock_args: Vec<_> =
        args.windows(2).filter(|pair| pair[0] == "--vsock").map(|pair| pair[1].clone()).collect();
    assert_eq!(vsock_args, vec![format!("cid={guest_cid},backend=vhost-kernel")]);
    assert_eq!(wait_until_vmm_holds_its_device(pid).await, 1);

    let mut filter_receipt = Vec::new();
    for entry in std::fs::read_dir(format!("/proc/{pid}/task")).expect("read VMM tasks") {
        let entry = entry.expect("read VMM task entry");
        let task_status = entry.path().join("status");
        let no_new_privs = proc_status_numbers(&task_status, "NoNewPrivs:");
        let seccomp = proc_status_numbers(&task_status, "Seccomp:");
        assert_eq!(no_new_privs, vec![1], "{} inherits no-new-privs", task_status.display());
        assert_eq!(seccomp, vec![2], "{} inherits filter mode", task_status.display());
        filter_receipt.push((entry.file_name(), no_new_privs[0], seccomp[0]));
    }

    assert_eq!(peer.cid(), guest_cid, "the accepted control peer has the leased CID");
    let bound = peer
        .tcp_listen("v1c-data", SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("a host-to-guest command and guest-to-host reply cross the vhost-vsock device");
    assert_ne!(bound.port(), 0, "the round-trip carries the guest's reply value");
    peer.close("v1c-data").expect("close the guest-local witness listener");
    assert!(
        !ends_within(&mut process, Duration::from_millis(250)).await,
        "the filter did not end the VMM after data"
    );

    let uname = Command::new("uname").arg("-r").output().expect("uname -r");
    eprintln!(
        "V1C kernel={} pid={pid} uid={} gid={} groups={:?} kvm_gid={kvm_gid} \
         vhost_gid={vhost_gid} \
         vhost_fds=1 filter={filter_receipt:?} data_round_trip_port={} argv={args:?}",
        String::from_utf8_lossy(&uname.stdout).trim(),
        OVERDRIVE_VMM_UID,
        OVERDRIVE_VMM_UID,
        proc_status_numbers(&status_path, "Groups:"),
        bound.port(),
    );

    peer.power_off().expect("the guest acknowledges poweroff");
    assert!(
        ends_within(&mut process, Duration::from_secs(10)).await,
        "the VMM exits after the guest powers off"
    );
}

/// V-30-FORK: once the guest is reachable over its control connection, both a
/// kernel panic (`sysrq-c`) and an immediate reboot (`sysrq-b`) end the VMM on
/// their first reset. The VMM's last device close resets a live host flow, the
/// guest kernel booted once, and the CID is reusable at once.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial(vhost_vsock)]
async fn post_boot_resets_end_the_vmm_and_reset_live_host_flows() {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("the prepared guest kit provisions");
    let fixture = VmFixture::provision(&staging).expect("the VM fixture provisions");
    let busybox_output = Command::new("which").arg("busybox").output().expect("locate busybox");
    assert!(busybox_output.status.success(), "the metal fixture provides busybox");
    let busybox = PathBuf::from(String::from_utf8_lossy(&busybox_output.stdout).trim());

    for (index, (name, sysrq_b)) in [("sysrq-c", false), ("sysrq-b", true)].into_iter().enumerate()
    {
        let work = WorkDir::create(&staging, &format!("reset-exit-{name}")).expect("work dir");
        let mut image_spec = GuestImageSpec::scripted();
        image_spec.forwarder_module = false;
        image_spec.extra.push(GuestFile {
            host: busybox.clone(),
            guest: PathBuf::from("/bin/busybox"),
            with_shared_libraries: false,
        });
        let image = kit.image(work.path(), name, &image_spec).expect("scripted reset image");
        let offset = 0x0407 + u32::try_from(index).expect("two cases");
        let guest_cid = cid(offset).get();
        let control = GuestPeerControl::bind(VMADDR_PORT_ANY).expect("bind control listener");
        let control_arg = format!("{CONTROL_PORT_PARAM}{}", control.port());
        let config = launch_config_on(
            &fixture,
            &image.rootfs,
            &work.path().join("run"),
            &format!("reset-exit-{name}"),
            &[&control_arg],
        );
        let vmm = CloudHypervisorVmm::new().with_binary(fixture.cloud_hypervisor_bin.clone());
        let mut process = vmm.create(&config, cid(offset)).await.expect("the reset case launches");
        let pid = process.control.pid;
        let _kill = KillOnDrop(pid);
        let mut peer = control
            .accept_peer(guest_cid, SCRIPTED_BOOT_BOUND)
            .expect("the post-boot guest control connection is live");

        let port = 50_700 + u32::try_from(index).expect("two cases");
        peer.vsock_listen("reset-listener", port).expect("guest listens for the held flow");
        let host_flow = VsockStream::connect_retrying(guest_cid, port, Duration::from_secs(10))
            .expect("host opens a live flow to the guest");
        let accepted = peer
            .vsock_accept("reset-flow", "reset-listener", Duration::from_secs(5))
            .expect("guest accepts the held flow")
            .expect("the host connected");
        assert_eq!(accepted.0, VMADDR_CID_HOST, "the held flow belongs to the host");

        if sysrq_b {
            let _closed_by_reset = peer.exec(
                &["/bin/busybox", "sh", "-c", "echo b > /proc/sysrq-trigger"],
                Duration::from_secs(5),
            );
        } else {
            peer.crash_kernel().expect("send the existing sysrq-c stimulus");
        }

        assert!(
            ends_within(&mut process, RESET_EXIT_BOUND).await,
            "{name}: the first reset ends the VMM"
        );
        let mut byte = [0_u8; 1];
        let reset = host_flow
            .read_within(&mut byte, Duration::from_secs(5))
            .expect_err("the device release resets the live host flow");
        assert_eq!(reset.raw_os_error(), Some(libc::ECONNRESET), "{name}: host flow reset");
        assert_eq!(vhost_vsock_fds(pid), None, "{name}: the VMM and device are gone");
        assert_eq!(kernel_boots(&config), 1, "{name}: no second guest boot");

        let next_config = launch_config(&fixture, work.path(), &format!("{name}-next"), &[]);
        let next = vmm.create(&next_config, cid(offset)).await.expect("the CID is reusable");
        let _kill_next = KillOnDrop(next.control.pid);
        assert_eq!(wait_until_vmm_holds_its_device(next.control.pid).await, 1);
        vmm.terminate(&next.control, Duration::ZERO).await.expect("terminate the reuse witness");
    }
}

/// RESET-EXIT (ADR-0182): a guest PID-1 crash under `panic=1` ends the VMM
/// instead of rebooting the guest in place — the guest kernel booted once —
/// and the device is released with it (the CID is free for the next
/// launch).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(vhost_vsock)]
async fn a_guest_pid1_crash_under_panic_1_ends_the_vmm() {
    let (fixture, run_root) = fixture_and_run_root("pid1-crash");
    let vmm = CloudHypervisorVmm::new();
    // No init can run: the kernel panics, `panic=1` resets after one second.
    let config = launch_config(&fixture, &run_root, "vsock-dev-pid1", &["init=/nonexistent"]);

    let mut process = vmm.create(&config, cid(0x0403)).await.expect("the fork launches");
    let pid = process.control.pid;
    let _kill = KillOnDrop(pid);
    wait_until_vmm_holds_its_device(pid).await;

    assert!(
        ends_within(&mut process, RESET_EXIT_BOUND).await,
        "the guest reset ended the VMM within {RESET_EXIT_BOUND:?} (no in-place reboot)"
    );
    assert_eq!(vhost_vsock_fds(pid), None, "the VMM process is gone with its device");
    assert_eq!(kernel_boots(&config), 1, "no second guest boot ran under the same VMM");

    let next_config = launch_config(&fixture, &run_root, "vsock-dev-pid1-next", &[]);
    let next = vmm.create(&next_config, cid(0x0403)).await.expect("the next launch");
    let _kill_next = KillOnDrop(next.control.pid);
    assert_eq!(wait_until_vmm_holds_its_device(next.control.pid).await, 1, "the CID is free");
    vmm.terminate(&next.control, Duration::ZERO).await.expect("terminate");
}

/// RESET-EXIT (ADR-0182): a triple fault — the reset `reboot=t` performs —
/// ends the VMM as well, after one guest boot.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(vhost_vsock)]
async fn a_guest_triple_fault_ends_the_vmm() {
    let (fixture, run_root) = fixture_and_run_root("triple-fault");
    let vmm = CloudHypervisorVmm::new();
    let config =
        launch_config(&fixture, &run_root, "vsock-dev-triple", &["init=/nonexistent", "reboot=t"]);

    let mut process = vmm.create(&config, cid(0x0404)).await.expect("the fork launches");
    let pid = process.control.pid;
    let _kill = KillOnDrop(pid);
    wait_until_vmm_holds_its_device(pid).await;

    assert!(
        ends_within(&mut process, RESET_EXIT_BOUND).await,
        "the triple fault ended the VMM within {RESET_EXIT_BOUND:?}"
    );
    assert_eq!(vhost_vsock_fds(pid), None, "the VMM process is gone with its device");
    assert_eq!(kernel_boots(&config), 1, "no second guest boot ran under the same VMM");
}

/// RESET-EXIT (ADR-0182): a forced reboot from inside the guest — PID 1
/// calls `reboot(2)` with the restart command, the `reboot -f` path — ends
/// the VMM instead of booting the guest again under the same CID and launch.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(vhost_vsock)]
async fn a_forced_guest_reboot_ends_the_vmm() {
    let (fixture, run_root) = fixture_and_run_root("forced-reboot");
    let rootfs = rootfs_with_forced_reboot_init(&fixture, &run_root.join("forced-reboot-rootfs"));
    let vmm = CloudHypervisorVmm::new();
    let config = launch_config_on(
        &fixture,
        &rootfs,
        &run_root,
        "vsock-dev-reboot",
        &["init=/sbin/reboot-now"],
    );

    let mut process = vmm.create(&config, cid(0x0405)).await.expect("the fork launches");
    let pid = process.control.pid;
    let _kill = KillOnDrop(pid);
    wait_until_vmm_holds_its_device(pid).await;

    assert!(
        ends_within(&mut process, RESET_EXIT_BOUND).await,
        "the forced reboot ended the VMM within {RESET_EXIT_BOUND:?}"
    );
    assert_eq!(vhost_vsock_fds(pid), None, "the VMM process is gone with its device");
    assert_eq!(kernel_boots(&config), 1, "no second guest boot ran under the same VMM");
}
