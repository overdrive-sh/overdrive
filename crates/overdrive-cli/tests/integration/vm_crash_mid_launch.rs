//! A `serve` crash just after a VMM is spawned, then a second `serve` on the same directories
//! (ADR-0352 "A VMM is born inside its scope", ADR-0350 "Boot proceeds past a stuck VM"; GH #348).
//!
//! The VMM is a **stand-in hypervisor** (`tests/fixtures/standin_hypervisor.rs`): a tiny program
//! this test builds with the guest's own `rustc` (profile `nested-launch-rustc`) and gives to
//! `CloudHypervisorVmm::with_binary`. As its first act it records the cgroup it was born into
//! (`/proc/self/cgroup`) in its run directory; it then claims the guest CID it was leased through
//! `/dev/vhost-vsock`, as a hypervisor does, and blocks holding the device. The source travels in
//! this test binary (`include_str!`), so the guest kit needs no new artifact.
//!
//! The crash is the serve lifetime port's killed mode (`serve_lifetime_support::kill_serve_owner`,
//! the one in-process process-loss mechanism): every task, socket and descriptor of the owner is
//! released as the kernel releases them at `SIGKILL`, the VMM survives. It is not a process death:
//! the owner's memory is dropped, not torn away, so a destructor that runs on a real crash-free
//! path still runs here; what it cannot model is a descriptor leaked to a child.
//!
//! Scenario map (CONTRACT_SHAPE: unbounded-preservation):
//! ```gherkin
//! Given a VM workload whose hypervisor has just been spawned and has claimed its guest address
//! When serve dies without a graceful shutdown and a second serve boots on the same directories
//! Then the crashed VM is gone and its scope removed once the second boot has run
//! And the workload is launched again and its hypervisor claims its guest address without a clash
//! And the crashed VM's first act, before anything could place it, ran inside its allocation's scope
//! ```
//! The last clause is the one a launch that places the pid after the spawn fails: the process
//! spends its first instructions outside the scope boot reclamation sweeps.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::expect_used, clippy::missing_panics_doc, clippy::doc_markdown)]

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, deploy};
use overdrive_cli::commands::serve::ServeHandle;
use overdrive_host::{CloudHypervisorVmm, VsockBeaconIo};
use overdrive_testing::overdrive_test;
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tempfile::TempDir;

const STANDIN_SOURCE: &str = include_str!("../fixtures/standin_hypervisor.rs");
const RUN_DIR_ROOT: &str = "/run/overdrive/vm";
const SCOPE_ROOT: &str = "/sys/fs/cgroup/overdrive.slice/workloads.slice";

fn staging_root() -> PathBuf {
    overdrive_testing::vm_fixture::default_staging_root()
}

/// Builds the stand-in with the guest's `rustc` into a directory the confined identity can
/// traverse (the launcher drops privileges before it executes the hypervisor).
fn build_standin(dir: &Path) -> PathBuf {
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))
        .expect("the stand-in's directory is traversable by the confined identity");
    let source = dir.join("standin_hypervisor.rs");
    std::fs::write(&source, STANDIN_SOURCE).expect("write the stand-in source");
    let binary = dir.join("cloud-hypervisor");
    let status = Command::new("rustc")
        .args(["--edition", "2021", "-C", "opt-level=0", "-C", "target-feature=+crt-static"])
        .args(["--target", "x86_64-unknown-linux-musl", "-o"])
        .arg(&binary)
        .arg(&source)
        .status()
        .expect("spawn rustc for the stand-in hypervisor");
    assert!(status.success(), "rustc must build the stand-in hypervisor at {}", binary.display());
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
        .expect("the stand-in is executable by the confined identity");
    binary
}

/// What a stand-in recorded: the allocation it served, its pid, its cgroup lines, its guest CID
/// and the outcome of claiming it.
#[derive(Debug, Clone)]
struct Record {
    alloc: String,
    pid: u32,
    cgroup: Vec<String>,
    cid: u64,
    claim: String,
}

/// Every complete record (claim written) under the run directory root.
fn records() -> Vec<Record> {
    let Ok(entries) = std::fs::read_dir(RUN_DIR_ROOT) else { return Vec::new() };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path().join("standin.record")) else {
            continue;
        };
        let mut record = Record {
            alloc: entry.file_name().to_string_lossy().into_owned(),
            pid: 0,
            cgroup: Vec::new(),
            cid: 0,
            claim: String::new(),
        };
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("cgroup=") {
                record.cgroup.push(value.to_owned());
            } else if let Some(value) = line.strip_prefix("pid=") {
                record.pid = value.parse().unwrap_or(0);
            } else if let Some(value) = line.strip_prefix("cid=") {
                record.cid = value.parse().unwrap_or(0);
            } else if let Some(value) = line.strip_prefix("claim=") {
                value.clone_into(&mut record.claim);
            }
        }
        if !record.claim.is_empty() {
            found.push(record);
        }
    }
    found
}

async fn wait_for_record_other_than(known: &[String], bound: Duration) -> Record {
    let deadline = tokio::time::Instant::now() + bound;
    loop {
        if let Some(record) = records().into_iter().find(|r| !known.contains(&r.alloc)) {
            return record;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no stand-in hypervisor recorded its launch within {bound:?}; records: {:?}",
            records(),
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

async fn serve_on(server: &TempDir, standin: &Path) -> ServeHandle {
    let data_dir = server.path().join("data");
    let config_dir = server.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let args = super::serve_fixture::serve_args(bind, data_dir, config_dir);
    let vmm = Arc::new(
        CloudHypervisorVmm::new().with_binary(standin.to_path_buf()).with_image_dir(staging_root()),
    );
    super::serve_fixture::serve_with_vm(
        args,
        super::serve_fixture::DataplaneChoice::Sim,
        super::serve_fixture::VmPorts { vmm, beacon: Arc::new(VsockBeaconIo::new()) },
    )
    .await
    .expect("serve boots with the stand-in hypervisor")
}

/// A crash just after the VMM was spawned: the second boot reclaims the crashed VM, the workload
/// is launched again without a guest-address clash, and the crashed VM was born inside its scope.
///
/// Contract: ADR-0352 (a VMM is a member of its scope from its first instruction) and ADR-0350 /
/// ADR-0156 (boot reclamation completes before the first launch of the next boot).
/// Owner path: `serve` + `deploy`, then `serve` again on the same directories.
/// Fault stimulus: the serve lifetime port's killed mode with the stand-in held after `exec`.
/// Oracle: the stand-in's own record of `/proc/self/cgroup` and of its CID claim, `/proc/<pid>`
/// and the scope directory. The membership assertion is the last, so each assertion before it is
/// proven to hold today and the first failure reached is the membership one.
/// CONTRACT_SHAPE: unbounded-preservation.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch-rustc")]
#[tokio::test]
#[serial(cgroup)]
#[should_panic(
    expected = "the hypervisor's first act must run inside its allocation's cgroup scope"
)]
async fn a_serve_crash_just_after_the_vmm_is_spawned_leaves_nothing_outside_a_scope_boot_reclaims()
{
    let fixture = VmFixture::provision(&staging_root()).expect("provision the shared VM fixture");
    let artifacts = tempfile::Builder::new()
        .prefix("vm-crash-mid-launch-")
        .tempdir_in(staging_root())
        .expect("tempdir on the staging root");
    let standin = build_standin(artifacts.path());
    let server = tempfile::Builder::new()
        .prefix("vm-crash-serve-")
        .tempdir_in(staging_root())
        .expect("server tempdir on the reflink-capable staging root");

    let handle = serve_on(&server, &standin).await;
    let cfg = server.path().join("conf").join(".overdrive").join("config");
    let spec = artifacts.path().join("crash-mid-launch.toml");
    std::fs::write(
        &spec,
        format!(
            "[job]\nid = \"crash-mid-launch\"\n\n[vm]\ncommand = \"/sbin/true\"\nargs = []\n\
             kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu = '500m'\nmemory = '128MiB'\n",
            fixture.kernel_path.display(),
            fixture.rootfs_path.display(),
        ),
    )
    .expect("write the VM workload spec");
    deploy(DeployArgs { spec, config_path: cfg }).await.expect("deploy the VM workload");

    // The hypervisor has been spawned and holds its guest address.
    let crashed = wait_for_record_other_than(&[], Duration::from_secs(90)).await;
    assert_eq!(crashed.claim, "ok", "the first hypervisor claims its leased guest address");
    assert!(process_alive(crashed.pid), "the hypervisor is running when serve dies");

    // serve dies without a graceful shutdown; the VMM survives it.
    super::serve_lifetime_support::kill_serve_owner(handle).await.expect("the killed mode");
    assert!(process_alive(crashed.pid), "the crash leaves the hypervisor running");

    // The second boot reclaims it before the first launch of the new boot.
    let second = serve_on(&server, &standin).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while process_alive(crashed.pid) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the crashed hypervisor (pid {}) was still alive 60 s after the second boot",
            crashed.pid,
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let scope = PathBuf::from(SCOPE_ROOT).join(format!("{}.scope", crashed.alloc));
    assert!(
        !scope.exists(),
        "the crashed VM's scope {} must be removed by boot reclamation",
        scope.display(),
    );

    // The workload is launched again, and its hypervisor claims its guest address without a clash.
    let relaunched =
        wait_for_record_other_than(std::slice::from_ref(&crashed.alloc), Duration::from_secs(90))
            .await;
    assert_eq!(
        relaunched.claim, "ok",
        "the relaunch must claim its guest address; a clash with the crashed VM's address is \
         EADDRINUSE; relaunched {relaunched:?}, crashed {crashed:?}",
    );
    assert_ne!(relaunched.pid, crashed.pid, "the relaunch is a new process");

    second.shutdown().await.expect("graceful shutdown of the second serve");
    if process_alive(relaunched.pid) {
        let _ = Command::new("kill").args(["-KILL", &relaunched.pid.to_string()]).status();
    }

    // Last, so the assertions above hold today: the crashed VM's first act ran in its scope.
    let expected = format!("/overdrive.slice/workloads.slice/{}.scope", crashed.alloc);
    assert!(
        crashed.cgroup.iter().any(|line| line.ends_with(&expected)),
        "the hypervisor's first act must run inside its allocation's cgroup scope {expected}; \
         it recorded {:?}",
        crashed.cgroup,
    );
}
