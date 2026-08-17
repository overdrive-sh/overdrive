//! S-VM-34 — production port-to-port RED scaffold for the first consumer of
//! DWD-24's typed driver-start failure transport.
//!
//! Driving path: real in-process `overdrive serve` composition -> direct
//! `overdrive deploy <SPEC>` handler -> `overdrive workload describe <ID>`.
//! This matches `vm_walking_skeleton.rs` and the crate's no-subprocess rule;
//! it never calls a private classifier or hand-installs a missing production
//! effect.
//!
//! The scenario is pre-spawn real I/O, but this file deliberately inherits
//! the established microVM fixture's file-level `integration-tests,kvm-tests`
//! gate. The broader gate is a test-layout fact, not a claim that the missing
//! rootfs path boots a guest.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::expect_used, clippy::missing_panics_doc)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, deploy};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, WorkloadDescribeOutput, describe};
use overdrive_control_plane::VmBootArtifacts;
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::TransitionReason;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::AllocationId;
use overdrive_core::vm::config::{RootfsPlan, VmRunDir};
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tempfile::TempDir;

fn shared_staging_root() -> PathBuf {
    overdrive_testing::vm_fixture::default_staging_root()
}

/// The same real in-process `serve` composition used by the existing microVM
/// walking skeleton: production server wiring with only the dataplane and KEK
/// external ports replaced by their established simulation adapters.
async fn spawn_vm_server(vm_artifacts: VmBootArtifacts) -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = ServeArgs { bind, data_dir, config_dir };
    let handle = overdrive_cli::commands::serve::run_with_dataplane_and_vm_artifacts(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
        vm_artifacts,
    )
    .await
    .expect("serve::run_with_dataplane_and_vm_artifacts");
    (handle, tmp)
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

fn vm_job_toml(id: &str, kernel: &Path, rootfs: &Path) -> String {
    format!(
        "[job]\nid = \"{id}\"\n\n[vm]\ncommand = \"/sbin/true\"\nargs = []\n\
         kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\n\
         memory_bytes = 134217728\n",
        kernel.display(),
        rootfs.display(),
    )
}

fn write_toml(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write VM workload spec");
    path
}

async fn poll_until_failed(
    cfg: &Path,
    workload_id: &str,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let out =
            describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_owned() })
                .await
                .expect("workload describe must succeed while polling");
        if let Some(row) = out.snapshot.rows.first()
            && row.state == AllocStateWire::Failed
        {
            return out;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workload {workload_id} did not reach Failed within {max_wait:?}; last row: {:?}",
            out.snapshot.rows.first(),
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// `true` when no Cloud Hypervisor process is alive **for this allocation**
/// — i.e. no live `cloud-hypervisor` argv references `run_dir`, which is
/// where every per-VM socket path this allocation would use lives.
///
/// Deliberately scoped to the allocation rather than the whole host. A
/// host-global scan cannot support the claim this scenario makes: sibling
/// Tier-3 scenarios (`vmm_equivalence`) spawn REAL Cloud Hypervisor
/// processes in parallel nextest processes, so a global scan fails on
/// THEIR processes while proving nothing about this rejected start. That
/// is the same cross-test contamination the walking skeleton already hit
/// and fixed for S-VM-05 (`vm_walking_skeleton.rs`, fourth pass) — the
/// lesson is applied here up front rather than re-learned.
///
/// Matches on `argv`, not the `TASK_COMM_LEN`-truncated
/// `/proc/<pid>/comm` (which caps at 15 chars and can never equal the
/// 16-char `cloud-hypervisor`).
fn no_cloud_hypervisor_process_for_alloc(run_dir: &Path) -> bool {
    let needle = run_dir.to_string_lossy().into_owned();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return true;
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let mut argv = cmdline.split(|&byte| byte == 0);
        let argv0 = String::from_utf8_lossy(argv.next().unwrap_or(&[])).into_owned();
        if Path::new(&argv0).file_name() != Some(std::ffi::OsStr::new("cloud-hypervisor")) {
            continue;
        }
        if String::from_utf8_lossy(&cmdline).contains(&needle) {
            return false;
        }
    }
    true
}

/// `true` when no Cloud Hypervisor process is alive anywhere on the host.
/// Retained for reference; see the scoping rationale above for why the
/// allocation-scoped variant is the one this scenario asserts on.
#[allow(dead_code)]
fn no_cloud_hypervisor_process_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return true;
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(_pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let argv0 = cmdline.split(|&byte| byte == 0).next().unwrap_or(&[]);
        let argv0 = String::from_utf8_lossy(argv0);
        if Path::new(argv0.as_ref()).file_name() == Some(std::ffi::OsStr::new("cloud-hypervisor")) {
            return false;
        }
    }
    true
}

/// S-VM-34 / `@contract-shape:bounded-change` — a configured rootfs master
/// that is absent at allocation start reaches the operator as
/// `Failed/VmRootfsNotFound`, names the exact configured path, remains distinct
/// from `VmKernelNotFound`, and leaves no allocation-scoped VM resources.
#[tokio::test]
#[serial(cgroup)]
async fn missing_configured_rootfs_is_named_precisely_and_leaks_no_vm_resources() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let artifact_tmp = tempfile::Builder::new()
        .prefix("vm-rootfs-missing-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the shared reflink-capable staging root");
    let missing_rootfs = artifact_tmp.path().join("configured-rootfs-missing.ext4");
    assert!(
        !missing_rootfs.exists(),
        "S-VM-34 precondition: configured rootfs path must be absent"
    );

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: missing_rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-rootfs-missing.toml",
        &vm_job_toml("vm-rootfs-missing", &fixture.kernel_path, &missing_rootfs),
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the VM workload with the absent configured rootfs");
    let out = poll_until_failed(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    let row =
        out.snapshot.rows.first().expect("one failed allocation row for the deployed VM workload");
    let alloc = AllocationId::new(&row.alloc_id).expect("server allocation id is valid");

    // ---- The operator-visible outcome, off the production surface. ----
    let configured = missing_rootfs.display().to_string();
    let reason = row.reason.clone().expect("a failed VM allocation must carry a structured reason");

    assert_eq!(
        reason,
        TransitionReason::VmRootfsNotFound { path: configured.clone() },
        "S-VM-34: the operator must be told the rootfs is missing, and told WHICH path",
    );

    // Distinct from the kernel artifact — the two must never collapse
    // into one diagnosis, which is the whole point of naming them apart.
    assert!(
        !matches!(reason, TransitionReason::VmKernelNotFound { .. }),
        "the rootfs cause must be distinct from VmKernelNotFound",
    );
    let kernel_path = fixture.kernel_path.display().to_string();
    assert_ne!(
        configured, kernel_path,
        "fixture precondition: the two artifact paths must differ for the distinction to mean \
         anything",
    );

    // The verbatim low-level diagnostic survives on its own channel.
    let detail = row.error.clone().expect("the verbatim driver diagnostic must be preserved");
    assert!(detail.contains(&configured), "the diagnostic must name the configured path: {detail}",);

    // ...and it reaches the operator's actual rendered output.
    let rendered = overdrive_cli::render::workload_describe(&out);
    assert!(
        rendered.contains(&configured),
        "the rendered operator view must name the exact configured rootfs path:\n{rendered}",
    );

    // ---- Rejected starts leak nothing. ----
    let rootfs_plan = RootfsPlan::for_alloc(missing_rootfs.clone(), 0, &alloc);
    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), &alloc);
    let scope_dir = CgroupPath::for_alloc(&alloc).resolve(Path::new("/sys/fs/cgroup"));

    assert!(
        no_cloud_hypervisor_process_for_alloc(run_dir.path()),
        "a rejected VM start must leave no hypervisor process behind for {}",
        run_dir.path().display(),
    );
    assert!(
        !rootfs_plan.clone_dest().exists(),
        "a rejected VM start must leave no per-launch rootfs clone at {}",
        rootfs_plan.clone_dest().display(),
    );
    assert!(
        !run_dir.path().exists(),
        "a rejected VM start must leave no run directory at {}",
        run_dir.path().display(),
    );
    assert!(
        !scope_dir.exists(),
        "a rejected VM start must leave no cgroup scope at {}",
        scope_dir.display(),
    );

    handle.shutdown().await.expect("clean shutdown");
}
