//! Tier-3 acceptance: a VMM is born inside its allocation's scope, and a launch
//! that cannot join the scope names the step and leaves nothing behind (RB-9,
//! ADR-0352, P-7; feature delta § "A VMM is born inside its scope", Tests).
//!
//! # Scenario family
//!
//! Today the driver creates the scope, `Vmm::create` spawns the VMM in `serve`'s
//! own cgroup, and only afterwards does the driver write the pid into
//! `cgroup.procs`. A `serve` crash between the two leaves a VMM that holds a
//! vsock device outside any scope, which boot reclamation cannot see. The
//! contract is that `create` returns a VMM whose process was created inside
//! `config.cgroup_scope` (`clone3` with `CLONE_INTO_CGROUP`), and that a refused
//! placement fails `create` with no process, no rootfs clone left, and a
//! `detail` that names the step (`joining the cgroup failed: ...`).
//!
//! # Owner path, fault stimulus, oracle
//!
//! The production `CloudHypervisorVmm::create` over the real cgroup v2
//! filesystem and a real Cloud Hypervisor, with the cgroup root left at its
//! default (the cgroup2 mount). The fault is the scope directory: removed before
//! the call (never created), or made an invalid placement target by enabling a
//! controller in its `subtree_control`. Oracle: `/proc/<pid>/cgroup`, the
//! scope's `cgroup.procs`, the error's `detail`, the process table, the
//! rootfs clone path.
//!
//! # Tier and placement
//!
//! Tier 3, `nested-launch` guest, `kvm-tests`. Example-only (Mandate 9).
//! PENDING (RED): today `create` ignores `config.cgroup_scope`.

#![allow(clippy::doc_markdown, reason = "contract-shape tags in the scenario docs")]

use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::time::Duration;

use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::guest_transport::{GUEST_CID_BASE, GuestCid};
use overdrive_core::traits::vmm::Vmm;
use overdrive_core::vm::config::{
    HostArch, KERNEL_MAGIC_WINDOW, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan, VmConfig,
    VmConfinement, VmRunDir,
};
use overdrive_host::CloudHypervisorVmm;
use overdrive_testing::overdrive_test;
use overdrive_testing::vm_fixture::{VmFixture, default_staging_root};
use serial_test::serial;

const GUEST_BYTES: u64 = 128 * 1024 * 1024;
const CGROUP_ROOT: &str = "/sys/fs/cgroup";

fn cid(offset: u32) -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + offset).expect("an in-range guest CID")
}

fn validated_kernel(fixture: &VmFixture) -> KernelImage {
    use std::io::Read as _;
    let mut header = Vec::new();
    std::fs::File::open(&fixture.kernel_path)
        .expect("open the staged kernel")
        .take(KERNEL_MAGIC_WINDOW as u64)
        .read_to_end(&mut header)
        .expect("read the kernel header");
    KernelImage::validate(fixture.kernel_path.clone(), HostArch::X86_64, &header)
        .expect("the fixture kernel validates for x86_64")
}

fn config(fixture: &VmFixture, run_root: &Path, name: &str) -> VmConfig {
    let alloc = AllocationId::new(name).expect("valid alloc id");
    let index_dir = run_root.join("clone-index");
    let master_bytes = std::fs::metadata(&fixture.rootfs_path).expect("stat the rootfs").len();
    VmConfig {
        volume: None,
        alloc: alloc.clone(),
        kernel: validated_kernel(fixture),
        rootfs: RootfsPlan::for_alloc(
            fixture.rootfs_path.clone(),
            master_bytes,
            &alloc,
            run_root,
            &index_dir,
        ),
        cmdline: KernelCmdline::platform_default(HostArch::X86_64),
        memory: MemoryPlan::derive(GUEST_BYTES),
        vcpus: NonZeroU8::new(1).expect("1 is nonzero"),
        run_dir: VmRunDir::for_alloc(run_root, &alloc),
        confinement: VmConfinement::confined(super::production_vmm_identity(), 1024),
        cgroup_scope: CgroupPath::for_alloc(&alloc),
    }
}

fn scope_dir(config: &VmConfig) -> PathBuf {
    config.cgroup_scope.resolve(Path::new(CGROUP_ROOT))
}

/// The cgroup path a process reports for itself.
fn cgroup_of(pid: u32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default().trim().to_owned()
}

/// A Cloud Hypervisor process whose command line names `alloc`.
fn hypervisor_for(alloc: &AllocationId) -> bool {
    std::fs::read_dir("/proc").into_iter().flatten().flatten().any(|entry| {
        std::fs::read(entry.path().join("cmdline")).is_ok_and(|bytes| {
            let text = String::from_utf8_lossy(&bytes);
            text.contains("cloud-hypervisor") && text.contains(alloc.as_str())
        })
    })
}

fn stage(name: &str) -> (VmFixture, PathBuf, VmConfig) {
    let staging_root = default_staging_root();
    let fixture = VmFixture::provision(&staging_root).expect("the fixture provisions");
    let run_root = staging_root.join(format!("vmm-in-scope-{name}"));
    std::fs::create_dir_all(&run_root).expect("create the run root");
    let config = config(&fixture, &run_root, name);
    std::fs::create_dir_all(config.run_dir.path()).expect("create the run dir");
    std::fs::create_dir_all(run_root.join("clone-index")).expect("create the clone-index dir");
    (fixture, run_root, config)
}

/// PENDING (RB-9, ADR-0352). When `create` returns, the VMM process is a member
/// of `config.cgroup_scope` and the scope's `cgroup.procs` lists it. Today the
/// VMM is in `serve`'s cgroup until the driver writes its pid.
///
/// CONTRACT_SHAPE: bounded-change. `create` creates one process tree and
/// changes the membership of exactly one scope. Universe: the process's
/// cgroup path, the scope's `cgroup.procs`.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[tokio::test]
#[serial(vmm_in_scope)]
#[should_panic(expected = "the VMM is a member of its scope when create returns")]
async fn a_created_vmm_is_a_member_of_its_scope_when_create_returns() {
    let (_fixture, _run_root, config) = stage("in-scope-a");
    let scope = scope_dir(&config);
    std::fs::create_dir_all(&scope).expect("create the scope (the driver's step)");
    let vmm = CloudHypervisorVmm::default();
    vmm.probe().await.expect("Vmm::probe succeeds on this host");

    let process = vmm.create(&config, cid(0x0330)).await.expect("create boots a real VMM");

    let pid = process.control.pid;
    let membership = cgroup_of(pid);
    let procs = std::fs::read_to_string(scope.join("cgroup.procs")).unwrap_or_default();
    let in_scope = membership.ends_with(&format!("{}.scope", config.alloc))
        && procs.lines().any(|line| line.trim() == pid.to_string());
    let _killed = vmm.terminate(&process.control, Duration::ZERO).await;
    assert!(
        in_scope,
        "the VMM is a member of its scope when create returns: /proc/{pid}/cgroup = `{membership}`, scope procs = `{procs}`"
    );
}

/// PENDING (RB-9, P-7, ADR-0352 item 8). A scope that does not exist is a
/// refused placement: `create` fails with a `detail` that names the step, no
/// hypervisor process exists, and the rootfs clone is removed. Today `create`
/// ignores the scope and boots the VMM.
///
/// CONTRACT_SHAPE: bounded-change. Universe: the error, the process table, the
/// clone path.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[tokio::test]
#[serial(vmm_in_scope)]
#[should_panic(expected = "create against a scope that does not exist fails")]
async fn create_against_a_removed_scope_fails_naming_the_step_and_leaves_nothing() {
    let (_fixture, _run_root, config) = stage("in-scope-removed");
    assert!(!scope_dir(&config).exists(), "precondition: the scope was never created");
    let vmm = CloudHypervisorVmm::default();
    vmm.probe().await.expect("Vmm::probe succeeds on this host");

    let result = vmm.create(&config, cid(0x0331)).await;

    let Err(error) = result else { panic!("create against a scope that does not exist fails") };
    let text = format!("{error} / {error:?}");
    assert!(
        text.contains("joining the cgroup failed"),
        "the detail names the failed step `joining the cgroup`: {text}"
    );
    assert!(!hypervisor_for(&config.alloc), "no hypervisor process remains");
    assert!(
        !config.rootfs.clone_dest().exists(),
        "the rootfs clone is removed on a refused launch"
    );
}

/// PENDING (RB-9, ADR-0352 item 8, C3). A scope that cannot be a placement
/// target (a controller enabled in its `subtree_control`, so the scope is a
/// domain with children) refuses the launch the same way: typed, naming the
/// step, no process, no clone.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[tokio::test]
#[serial(vmm_in_scope)]
#[should_panic(expected = "create against a scope that is not a valid placement target fails")]
async fn create_against_a_scope_with_subtree_control_set_fails_naming_the_step_and_leaves_nothing()
{
    let (_fixture, _run_root, config) = stage("in-scope-subtree");
    let scope = scope_dir(&config);
    std::fs::create_dir_all(&scope).expect("create the scope");
    std::fs::write(scope.join("cgroup.subtree_control"), "+pids")
        .expect("precondition: enable a controller in the scope's subtree_control");
    let vmm = CloudHypervisorVmm::default();
    vmm.probe().await.expect("Vmm::probe succeeds on this host");

    let result = vmm.create(&config, cid(0x0332)).await;

    let Err(error) = result else {
        panic!("create against a scope that is not a valid placement target fails")
    };
    let text = format!("{error} / {error:?}");
    assert!(text.contains("joining the cgroup failed"), "the detail names the step: {text}");
    assert!(!hypervisor_for(&config.alloc), "no hypervisor process remains");
    assert!(!config.rootfs.clone_dest().exists(), "the rootfs clone is removed");
}
