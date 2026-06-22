//! `ExecDriver::start` with `spec.netns = Some(<name>)` enters the
//! target netns before `execve` (the `with_netns_path` builder was
//! deleted single-cut; the netns NAME now travels on `AllocationSpec`,
//! JOIN-2). Real-kernel test: spawn `/bin/sh -c 'readlink
//! /proc/self/ns/net >>/tmp/...; sleep 60'` with `spec.netns` pointing
//! at a freshly-created netns, and assert the child's
//! `/proc/self/ns/net` symlink resolves to the target netns's inode
//! (NOT the test process's inode).
//!
//! Linux-only; requires `CAP_SYS_ADMIN` (for `ip netns add`) +
//! `CAP_NET_ADMIN` and a writable `/sys/fs/cgroup`. Runs via
//! `cargo xtask lima run --` in the project's standard inner-loop
//! shape per `.claude/rules/testing.md` § "Running tests — Lima VM".
//!
//! Port-to-port: enters via `Driver::start`; asserts on the observable
//! filesystem side effect (`/proc/<pid>/ns/net` symlink target) and
//! on the absence of a `DriverError::NetnsEntry` failure. Does NOT
//! verify the internals of `pre_exec` directly — that would be
//! testing the implementation.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::missing_panics_doc,
    reason = "Test bodies; skip messages go to stderr; failures must panic with informative messages"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverError, Resources};
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::ExecDriver;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;

use super::cleanup::AllocCleanup;

/// RAII netns guard for tests. Creates `ip netns add <name>`; drops
/// `ip netns del <name>` best-effort.
struct TestNetns {
    name: String,
}

impl TestNetns {
    fn create(name: &str) -> Self {
        // Best-effort cleanup of leftover state.
        let _ = Command::new("ip").args(["netns", "del", name]).output();
        let out =
            Command::new("ip").args(["netns", "add", name]).output().expect("spawn ip netns add");
        assert!(
            out.status.success(),
            "ip netns add {name} failed: {}",
            String::from_utf8_lossy(&out.stderr),
        );
        Self { name: name.to_owned() }
    }

    fn path(&self) -> PathBuf {
        PathBuf::from(format!("/var/run/netns/{}", self.name))
    }

    /// Read the inode of the netns file at `/var/run/netns/<name>`.
    /// `setns(CLONE_NEWNET)` on this FD lands the calling thread in
    /// the namespace whose `proc/<pid>/ns/net` symlink resolves to
    /// `net:[<inode>]`.
    fn inode(&self) -> u64 {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(self.path()).expect("stat netns path").ino()
    }
}

impl Drop for TestNetns {
    fn drop(&mut self) {
        let _ = Command::new("ip").args(["netns", "del", &self.name]).output();
    }
}

/// Pre-flight: are we running as root with CAP_NET_ADMIN / CAP_SYS_ADMIN?
fn require_root_or_skip(test_name: &str) -> bool {
    // SAFETY: `geteuid` has no preconditions; reads a kernel-managed
    // numeric.
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        eprintln!("[skip] {test_name} needs root for `ip netns add`; euid={euid}");
        return false;
    }
    true
}

/// Read the netns inode from a `/proc/<pid>/ns/net` symlink target.
/// The symlink resolves to `net:[<inode>]`; we parse out the inode.
fn read_proc_netns_inode(pid: u32) -> Option<u64> {
    let link = std::fs::read_link(format!("/proc/{pid}/ns/net")).ok()?;
    let s = link.to_string_lossy();
    // Form is e.g. `net:[4026532001]`.
    let inner = s.strip_prefix("net:[")?.strip_suffix(']')?;
    inner.parse::<u64>().ok()
}

#[tokio::test]
#[serial(cgroup)]
async fn exec_driver_with_spec_netns_spawns_child_inside_target_netns() {
    if !require_root_or_skip("exec_driver_with_spec_netns") {
        return;
    }

    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    // Two distinct netns — the target the driver enters, and the
    // test process's own. The assertion that the child is in the
    // target (not the test's) requires the inodes to differ.
    let target_ns = TestNetns::create("ovd-edns-target");
    let target_inode = target_ns.inode();
    let test_inode =
        read_proc_netns_inode(std::process::id()).expect("test process /proc/self/ns/net readable");
    assert_ne!(
        target_inode, test_inode,
        "freshly-created netns must have a distinct inode from the test process's netns",
    );

    let driver: Arc<dyn Driver> =
        Arc::new(ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(SimClock::new()), fs));

    let alloc = AllocationId::new("alloc-netns-entry").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/netns/alloc/01")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // The netns NAME (not a path) — the production C3 channel (JOIN-2).
        // `start` joins it onto `/var/run/netns/<name>` (where `ip netns add`
        // placed it) and enters it via `setns(CLONE_NEWNET)`.
        netns: Some(target_ns.name.clone()),
        host_veth: None,
    };

    let handle = driver
        .start(&spec)
        .await
        .expect("ExecDriver::start with spec.netns succeeds for /bin/sleep");

    let pid = handle.pid.expect("ExecDriver populates pid on start");

    // The child is in the target netns iff `/proc/<pid>/ns/net`
    // resolves to the same inode as `/var/run/netns/<target>`.
    let child_inode =
        read_proc_netns_inode(pid).expect("child /proc/<pid>/ns/net symlink readable");
    assert_eq!(
        child_inode, target_inode,
        "child netns inode ({child_inode}) must match target netns inode ({target_inode}); \
         test process inode is {test_inode}",
    );

    driver.stop(&handle).await.expect("stop succeeds");
}

#[tokio::test]
#[serial(cgroup)]
async fn exec_driver_with_missing_netns_path_returns_netns_entry_error() {
    if !require_root_or_skip("exec_driver_missing_netns") {
        return;
    }

    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let missing_name = format!("ovd-edns-missing-{}", std::process::id());
    // Make sure it really doesn't exist.
    let _ = std::fs::remove_file(format!("/var/run/netns/{missing_name}"));

    let driver: Arc<dyn Driver> =
        Arc::new(ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(SimClock::new()), fs));

    let alloc = AllocationId::new("alloc-netns-missing").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/netns/alloc/missing")
            .expect("valid spiffe id"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // A netns NAME that does not exist under `/var/run/netns/` — the
        // open() pre-flight fails → `DriverError::NetnsEntry`.
        netns: Some(missing_name.clone()),
        host_veth: None,
    };

    let err = driver.start(&spec).await.expect_err("start must fail for missing netns");

    match err {
        DriverError::NetnsEntry { netns_path, .. } => {
            assert!(
                netns_path.contains("ovd-edns-missing"),
                "NetnsEntry must carry the offending path; got {netns_path}",
            );
        }
        other => panic!("expected DriverError::NetnsEntry; got {other:?}"),
    }

    // Give the kernel a moment to settle — no child was actually
    // spawned (open() failed pre-fork) so there's nothing to reap;
    // this is just defense in depth against ordering flakes between
    // the cleanup-on-error path's `remove_workload_scope` and the
    // next test's mkdir.
    tokio::time::sleep(Duration::from_millis(20)).await;
}
