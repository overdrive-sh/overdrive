//! Named VM boot-failure vocabulary — the operator-visible half of AC-09.
//!
//! S-VM-34 (step 03-05) — production port-to-port proof for the first consumer
//! of DWD-24's typed driver-start failure transport.
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
//!
//! Step 03-06 (DWD-24) extends that same production path across the remaining
//! four named Slice-02 causes — S-VM-33, S-VM-35, S-VM-36 and S-VM-41.
//! Capability truth stays mixed and the file gate stays a layout fact rather
//! than a capability claim: only S-VM-36 requires a real guest boot; S-VM-33,
//! S-VM-35 and S-VM-41 are all rejected before a guest-booting hypervisor
//! process exists.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::expect_used, clippy::missing_panics_doc)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, deploy};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, WorkloadDescribeOutput, describe};
use overdrive_control_plane::VmBootArtifacts;
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::TransitionReason;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::vmm::Vmm;
use overdrive_core::vm::config::{HostArch, RootfsPlan, VmRunDir};
use overdrive_host::CloudHypervisorVmm;
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

/// The same real in-process `serve` composition as [`spawn_vm_server`],
/// with the `Vmm` port additionally bound to a caller-supplied adapter
/// (`ServerConfig.vmm_override`, ADR-0083 §D8). S-VM-35 binds a REAL
/// `CloudHypervisorVmm` pointed at a hypervisor binary the test owns, so
/// removing that binary removes only THIS allocation's hypervisor and
/// never mutates the host-shared `cloud-hypervisor` sibling Tier-3
/// scenarios (`vmm_equivalence`) spawn in parallel nextest processes.
async fn spawn_vm_server_with_vmm(
    vm_artifacts: VmBootArtifacts,
    vmm: Arc<dyn Vmm>,
) -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = ServeArgs { bind, data_dir, config_dir };
    let handle = overdrive_cli::commands::serve::run_with_dataplane_and_vmm_override(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
        vm_artifacts,
        vmm,
    )
    .await
    .expect("serve::run_with_dataplane_and_vmm_override");
    (handle, tmp)
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

/// The architecture the production composition root derives for THIS
/// binary (`compose_vm_driver`'s own `cfg!(target_arch)` branch). Read
/// the same way here so the arch assertion is pinned to what production
/// computed rather than to a literal the test restates.
const fn host_arch() -> HostArch {
    if cfg!(target_arch = "aarch64") { HostArch::Aarch64 } else { HostArch::X86_64 }
}

/// Copies the shared fixture's kernel into a per-test directory so a
/// scenario may delete or replace ITS copy without disturbing the shared
/// artifact every other Tier-3 VM scenario reuses (AC5).
fn copy_kernel_for_this_test(tmp: &Path, fixture: &VmFixture, name: &str) -> PathBuf {
    let dest = tmp.join(name);
    std::fs::copy(&fixture.kernel_path, &dest)
        .expect("copy the shared fixture kernel into a per-test working copy");
    dest
}

/// Builds an EMPTY, validly-formatted 64 MiB ext4 image with NO staged
/// content at all — no `/sbin/init`, nothing. The kernel's no-`init=`
/// fallback search (`/sbin/init`, `/etc/init`, `/bin/init`, `/bin/sh`)
/// exhausts every candidate, so the guest boots the kernel and then
/// never beacons: S-VM-36's "guest that never signals READY" fixture.
fn build_empty_rootfs(tmp: &Path, name: &str) -> PathBuf {
    let path = tmp.join(name);
    {
        let file = std::fs::File::create(&path).expect("create empty rootfs image");
        file.set_len(64 * 1024 * 1024).expect("size empty rootfs image");
    }
    let status = Command::new("mkfs.ext4")
        .arg("-F")
        .args(["-L", "overdrive-vm-noinit"])
        .arg(&path)
        .status()
        .expect("spawn mkfs.ext4 for the empty rootfs image");
    assert!(status.success(), "mkfs.ext4 must format the empty rootfs image");
    path
}

/// Every allocation-scoped VM resource a rejected or aborted start must
/// leave behind: none. Named once so all five scenarios in this file
/// assert the SAME four facts (hypervisor process, per-launch rootfs
/// clone, run directory, cgroup scope) rather than drifting apart.
fn assert_no_allocation_scoped_vm_residue(alloc: &AllocationId, rootfs_master: &Path) {
    let rootfs_plan = RootfsPlan::for_alloc(rootfs_master.to_path_buf(), 0, alloc);
    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), alloc);
    let scope_dir = CgroupPath::for_alloc(alloc).resolve(Path::new("/sys/fs/cgroup"));

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
}

/// The operator-visible half of AC-09, asserted the same way by all five
/// scenarios: the NAMED cause must reach the rendered `overdrive workload
/// describe` output in the ratified `TransitionReason::human_readable()`
/// vocabulary — not merely the verbatim driver diagnostic, and not merely
/// a path that happens to appear inside it.
///
/// The second assertion is what stops this from being a tautology. Every
/// cause in this file carries a `detail` whose prose differs from the
/// cause's own wording (`open kernel image {p}: ...` vs `VM kernel not
/// found: {p}`; `stat rootfs master {p}: ...` vs `VM rootfs not found:
/// {p}`), so a renderer that emitted ONLY the detail cannot satisfy the
/// containment check. Pinning that divergence here means a future change
/// that collapses the two — making the cause's wording a substring of the
/// detail — fails loudly rather than silently hollowing out every
/// scenario's operator-facing assertion.
fn assert_named_cause_is_rendered(rendered: &str, reason: &TransitionReason, detail: &str) {
    let named = reason.human_readable();
    assert!(
        !detail.contains(&named),
        "test integrity: the verbatim driver detail must NOT already contain the named cause \
         {named:?}, or the render assertion below proves nothing; detail was {detail:?}",
    );
    assert!(
        rendered.contains(&named),
        "the rendered operator view must name the typed cause in the ratified vocabulary \
         ({named:?}) — the verbatim diagnostic alone is not the named cause:\n{rendered}",
    );
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

/// [`poll_until_failed`] that additionally records EVERY allocation state
/// it observed on the way, so a scenario can assert on the states the
/// allocation passed THROUGH and not merely on where it landed.
async fn poll_until_failed_recording_states(
    cfg: &Path,
    workload_id: &str,
    max_wait: Duration,
) -> (WorkloadDescribeOutput, Vec<AllocStateWire>) {
    let deadline = tokio::time::Instant::now() + max_wait;
    let mut observed = Vec::new();
    loop {
        let out =
            describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_owned() })
                .await
                .expect("workload describe must succeed while polling");
        if let Some(row) = out.snapshot.rows.first() {
            observed.push(row.state);
            if row.state == AllocStateWire::Failed {
                return (out, observed);
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workload {workload_id} did not reach Failed within {max_wait:?}; observed states: \
             {observed:?}",
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
    assert!(detail.contains(&configured), "the diagnostic must name the configured path: {detail}");

    // ...and it reaches the operator's actual rendered output.
    let rendered = overdrive_cli::render::workload_describe(&out);
    assert!(
        rendered.contains(&configured),
        "the rendered operator view must name the exact configured rootfs path:\n{rendered}",
    );
    assert_named_cause_is_rendered(&rendered, &reason, &detail);

    // ---- Rejected starts leak nothing. ----
    assert_no_allocation_scoped_vm_residue(&alloc, &missing_rootfs);

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// Step 03-06 (DWD-24) — the remaining four named Slice-02 causes.
//
// Every scenario below now carries `#[tokio::test]` + `#[serial(cgroup)]`,
// matching S-VM-34: each awaits a real in-process `serve` composition and each
// contends for the same host cgroup hierarchy, so both attributes are claims
// the bodies actually make.
//
// Three of the four (S-VM-33, S-VM-35, S-VM-41) use post-composition TOCTOU
// fixtures because an artifact that is missing or invalid at `serve` boot
// refuses the driver outright — no allocation is created, so no allocation-
// level `Failed` row can exist to assert on. Valid at composition, broken at
// this allocation's start is the only reachable producer of these rows.
//
// Field preservation is the point of the whole step: the exact configured
// path, the full searched-path vector, `deadline_ms`, the live console tail,
// the arch and the validator's own diagnosis must each survive at their
// source. Every scenario is therefore asserted through the rendered
// `overdrive workload describe` output, never by matching an enum
// discriminant alone (roadmap 03-06, fifth criterion).
// ---------------------------------------------------------------------------

/// S-VM-33 / `@contract-shape:bounded-change` — a kernel image that was present
/// when `overdrive serve` composed the Vm driver, then deleted before this
/// allocation starts, reaches the operator as `Failed/VmKernelNotFound` naming
/// the exact configured path, stays distinct from a missing rootfs, and spawns
/// no hypervisor process.
///
/// ```gherkin
/// Given "overdrive serve" booted while the configured VM kernel path held a
///   valid image, so the Vm driver composed successfully
/// And that exact kernel path is then DELETED after composition but before this
///   allocation's start performs its per-allocation verification
/// When the platform attempts to start the allocation
/// Then the allocation is Failed with VmKernelNotFound naming the exact path
/// And the reason is distinct from a missing rootfs
/// And no hypervisor process is spawned for this allocation
/// ```
///
/// The post-composition deletion is load-bearing, not incidental (DWD-24): a
/// kernel absent at `serve` boot cannot produce the allocation-level `Failed`
/// row this scenario asserts, because the validated kernel and the Vm
/// capability never compose and no allocation is ever created. Valid at
/// composition, absent at this allocation's start is the only reachable
/// producer of this row.
#[tokio::test]
#[serial(cgroup)]
async fn kernel_deleted_after_composition_is_named_precisely_and_spawns_no_hypervisor() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let artifact_tmp = tempfile::Builder::new()
        .prefix("vm-kernel-deleted-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the shared reflink-capable staging root");
    let kernel = copy_kernel_for_this_test(artifact_tmp.path(), &fixture, "kernel-to-delete");
    assert!(
        kernel.exists(),
        "S-VM-33 precondition: the configured kernel must be PRESENT when serve composes",
    );

    // Composition must SUCCEED against the valid copy — an absent kernel
    // at serve boot refuses the driver outright and creates no allocation.
    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: kernel.clone(),
        rootfs_path: fixture.rootfs_path.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-kernel-deleted.toml",
        &vm_job_toml("vm-kernel-deleted", &kernel, &fixture.rootfs_path),
    );

    // ---- The TOCTOU window: valid at composition, gone at start. ----
    std::fs::remove_file(&kernel).expect("delete the configured kernel after composition");
    assert!(
        !kernel.exists(),
        "S-VM-33 precondition: the configured kernel must be ABSENT before this start",
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the VM workload whose configured kernel was just deleted");
    let out = poll_until_failed(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    let row =
        out.snapshot.rows.first().expect("one failed allocation row for the deployed VM workload");
    let alloc = AllocationId::new(&row.alloc_id).expect("server allocation id is valid");

    let configured = kernel.display().to_string();
    let reason = row.reason.clone().expect("a failed VM allocation must carry a structured reason");

    assert_eq!(
        reason,
        TransitionReason::VmKernelNotFound { path: configured.clone() },
        "S-VM-33: the operator must be told the kernel is missing, and told WHICH path",
    );

    // Distinct from the rootfs artifact — the two name different files
    // and must never collapse into one diagnosis.
    assert!(
        !matches!(reason, TransitionReason::VmRootfsNotFound { .. }),
        "the kernel cause must be distinct from VmRootfsNotFound",
    );
    let rootfs_path = fixture.rootfs_path.display().to_string();
    assert_ne!(
        configured, rootfs_path,
        "fixture precondition: the two artifact paths must differ for the distinction to mean \
         anything",
    );
    assert!(
        fixture.rootfs_path.exists(),
        "the rootfs is present throughout — only the kernel went missing",
    );

    let detail = row.error.clone().expect("the verbatim driver diagnostic must be preserved");
    assert!(detail.contains(&configured), "the diagnostic must name the configured path: {detail}");

    let rendered = overdrive_cli::render::workload_describe(&out);
    assert!(
        rendered.contains(&configured),
        "the rendered operator view must name the exact configured kernel path:\n{rendered}",
    );
    assert_named_cause_is_rendered(&rendered, &reason, &detail);

    // The preflight runs before anything is provisioned, so nothing was
    // spawned and nothing is left behind.
    assert_no_allocation_scoped_vm_residue(&alloc, &fixture.rootfs_path);

    handle.shutdown().await.expect("clean shutdown");
}

/// S-VM-35 / `@contract-shape:bounded-change` — a cloud-hypervisor binary that
/// was present when `overdrive serve` composed the Vm driver, then removed
/// before this allocation spawns it, reaches the operator as
/// `Failed/VmHypervisorAbsent` naming every path searched, with the spawn
/// diagnostic preserved verbatim and the cause distinct from a missing kernel
/// or rootfs artifact.
///
/// ```gherkin
/// Given "overdrive serve" booted with cloud-hypervisor present -- the Vm
///   driver is composed and a [vm] deploy is admitted
/// And the cloud-hypervisor binary is then REMOVED from the host, after
///   admission but before this specific allocation's start actually spawns it
/// When the platform attempts to start the allocation
/// Then the allocation is Failed with VmHypervisorAbsent naming the paths
///   searched
/// And the low-level spawn diagnostic is preserved verbatim
/// And the reason is distinct from a missing kernel or rootfs artifact
/// ```
///
/// The TOCTOU window is real, not contrived: the composition-time presence gate
/// (ADR-0083 §D2/§D4) probes cloud-hypervisor ONCE, at `serve` boot, and no
/// per-deploy re-probe exists. A host with no hypervisor at boot is instead
/// S-VM-12 — rejected at admission with no allocation created at all, which
/// cannot satisfy this scenario's `Failed`-row assertion.
#[tokio::test]
#[serial(cgroup)]
async fn hypervisor_removed_after_composition_names_every_searched_path() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let artifact_tmp = tempfile::Builder::new()
        .prefix("vm-hypervisor-removed-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the shared reflink-capable staging root");

    // A hypervisor binary THIS test owns: removing it removes only this
    // allocation's hypervisor, never the host-shared `cloud-hypervisor`
    // that sibling Tier-3 scenarios spawn in parallel nextest processes.
    let hypervisor = artifact_tmp.path().join("cloud-hypervisor");
    std::fs::copy(&fixture.cloud_hypervisor_bin, &hypervisor)
        .expect("copy the host cloud-hypervisor into a per-test working copy");
    assert!(
        hypervisor.exists(),
        "S-VM-35 precondition: the hypervisor must be PRESENT when serve composes",
    );

    // Composition must SUCCEED — `Vmm::probe()` runs unconditionally
    // against this REAL adapter, so the copy proves itself capable before
    // any deploy is admitted. A host with NO hypervisor at boot is
    // S-VM-12 instead: rejected at admission, no allocation created.
    let vmm: Arc<dyn Vmm> = Arc::new(
        CloudHypervisorVmm::new()
            .with_binary(hypervisor.clone())
            .with_image_dir(shared_staging_root()),
    );
    let (handle, server_tmp) = spawn_vm_server_with_vmm(
        VmBootArtifacts {
            kernel_path: fixture.kernel_path.clone(),
            rootfs_path: fixture.rootfs_path.clone(),
        },
        vmm,
    )
    .await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-hypervisor-removed.toml",
        &vm_job_toml("vm-hypervisor-removed", &fixture.kernel_path, &fixture.rootfs_path),
    );

    // ---- The TOCTOU window: probed at composition, gone before spawn. ----
    std::fs::remove_file(&hypervisor).expect("remove the hypervisor after composition");
    assert!(
        !hypervisor.exists(),
        "S-VM-35 precondition: the hypervisor must be ABSENT before this start spawns it",
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the VM workload whose hypervisor was just removed");
    let out = poll_until_failed(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    let row =
        out.snapshot.rows.first().expect("one failed allocation row for the deployed VM workload");
    let alloc = AllocationId::new(&row.alloc_id).expect("server allocation id is valid");

    let searched_path = hypervisor.display().to_string();
    let reason = row.reason.clone().expect("a failed VM allocation must carry a structured reason");

    // EXACT vector equality, not `contains`: "names every path searched"
    // is only proven by pinning the whole vector — a first-path-only
    // report would pass a containment check.
    assert_eq!(
        reason,
        TransitionReason::VmHypervisorAbsent { searched: vec![searched_path.clone()] },
        "S-VM-35: the operator must be told the hypervisor is absent, and told WHERE we looked",
    );

    // Distinct from either missing-artifact cause — both artifacts are
    // present here; only the hypervisor went away.
    assert!(
        !matches!(
            reason,
            TransitionReason::VmKernelNotFound { .. } | TransitionReason::VmRootfsNotFound { .. }
        ),
        "the hypervisor cause must be distinct from a missing kernel or rootfs artifact",
    );
    assert!(
        fixture.kernel_path.exists() && fixture.rootfs_path.exists(),
        "both configured artifacts stay present throughout — only the hypervisor was removed",
    );

    // The low-level spawn diagnostic survives verbatim on its own channel.
    let detail = row.error.clone().expect("the verbatim spawn diagnostic must be preserved");
    assert!(
        !detail.trim().is_empty(),
        "the spawn diagnostic must reach the operator, not be dropped: {detail:?}",
    );

    let rendered = overdrive_cli::render::workload_describe(&out);
    assert!(
        rendered.contains(&searched_path),
        "the rendered operator view must name every searched path:\n{rendered}",
    );
    assert_named_cause_is_rendered(&rendered, &reason, &detail);

    // A spawn that never happened leaves nothing behind.
    assert_no_allocation_scoped_vm_residue(&alloc, &fixture.rootfs_path);

    handle.shutdown().await.expect("clean shutdown");
}

/// S-VM-36 / `@contract-shape:bounded-change` — a guest that never beacons
/// reaches the operator as `Failed/VmBootDeadlineExceeded` carrying the deadline
/// in milliseconds and the console tail captured live, and the allocation never
/// passes through Running.
///
/// ```gherkin
/// Given Ana has deployed a VM workload whose rootfs init hangs forever
/// When the boot deadline elapses
/// Then the allocation is Failed with VmBootDeadlineExceeded naming the
///   deadline in milliseconds and the captured console tail
/// And the allocation never passes through Running
/// ```
///
/// The only one of step 03-06's four scenarios that genuinely requires a real
/// guest boot: S-VM-33, S-VM-35 and S-VM-41 are all rejected before a
/// guest-booting hypervisor process exists, whereas this one must actually
/// start a guest and then wait it out. `@requires-kvm` is therefore a real
/// capability claim here, not the file-level layout gate.
///
/// Field-source discipline (DWD-24, roadmap 03-06): the shape the design pins
/// is `VmBootDeadlineExceeded { deadline_ms: 30000, console_tail }`, and both
/// fields must come from their live source. `deadline_ms` is the boot clock's
/// own configured budget, not a literal restated in the test. `console_tail`
/// comes from the live `VmmDiagnostics` read handle — deliberately NOT from
/// `VmmExit.stderr_tail`, which is available only after termination and so
/// cannot supply the deadline arm at all (ADR-0082 §D1.1).
#[tokio::test]
#[serial(cgroup)]
async fn guest_that_never_beacons_reports_the_boot_deadline_and_console_tail() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let artifact_tmp = tempfile::Builder::new()
        .prefix("vm-never-beacons-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the shared reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    // A real guest that really boots and really never beacons: the
    // kernel's no-`init=` fallback search finds no init at all.
    let silent_rootfs = build_empty_rootfs(artifact_tmp.path(), "never-beacons-rootfs.ext4");

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: silent_rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-never-beacons.toml",
        &vm_job_toml("vm-never-beacons", &fixture.kernel_path, &silent_rootfs),
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the VM workload whose guest never beacons");

    // The ceiling comfortably exceeds VM_BOOT_DEADLINE (30s), so this
    // asserts the DEADLINE fired rather than the poll giving up first.
    let (out, observed) =
        poll_until_failed_recording_states(&cfg, &submit.workload_id, Duration::from_secs(120))
            .await;
    let row =
        out.snapshot.rows.first().expect("one failed allocation row for the deployed VM workload");
    let alloc = AllocationId::new(&row.alloc_id).expect("server allocation id is valid");

    assert!(
        !observed.contains(&AllocStateWire::Running),
        "a guest that never beacons must NEVER pass through Running; observed: {observed:?}",
    );

    let reason = row.reason.clone().expect("a failed VM allocation must carry a structured reason");
    let detail = row.error.clone().expect("the verbatim driver diagnostic must be preserved");

    // The deadline is the boot clock's own configured budget, restated
    // here only as the value the design pins (ADR-0082 §D3 / DWD-24).
    let TransitionReason::VmBootDeadlineExceeded { deadline_ms, console_tail } = reason.clone()
    else {
        panic!(
            "S-VM-36: expected VmBootDeadlineExceeded, got {reason:?}; observed states \
             {observed:?}; verbatim driver detail: {detail}",
        );
    };
    assert_eq!(
        deadline_ms, 30_000,
        "the reported deadline must be the driver's own configured boot budget",
    );

    // Field-source proof. The deadline arm's ONLY tail source is the LIVE
    // `VmmDiagnostics` read handle — `VmmExit.stderr_tail` exists only
    // after termination and cannot supply this arm at all. So:
    //   * a captured tail must reach the row's verbatim detail unchanged,
    //     and must NOT be text reconstructed from the timeout itself;
    //   * an empty capture must say exactly that, never fabricate one.
    let no_capture_fallback = format!(
        "boot deadline ({deadline_ms}ms) elapsed with no beacon; no console output captured"
    );
    match console_tail.as_deref() {
        Some(tail) => {
            assert!(!tail.trim().is_empty(), "a captured tail must not be blank");
            assert_ne!(
                tail, no_capture_fallback,
                "the console tail must be the LIVE capture, never text derived from the timeout",
            );
            assert_eq!(
                detail, tail,
                "the row's verbatim detail must be the SAME live capture the cause carries",
            );
        }
        None => assert_eq!(
            detail, no_capture_fallback,
            "with nothing captured the operator must be told exactly that, never a fabricated tail",
        ),
    }

    // ...and the operator reads the deadline in the rendered view.
    let rendered = overdrive_cli::render::workload_describe(&out);
    assert!(
        rendered.contains("30000"),
        "the rendered operator view must name the deadline in milliseconds:\n{rendered}",
    );
    assert_named_cause_is_rendered(&rendered, &reason, &detail);
    if let Some(tail) = console_tail.as_deref() {
        assert!(
            rendered.contains(tail),
            "the rendered operator view must carry the captured console tail:\n{rendered}",
        );
    }

    // An aborted boot leaves nothing behind, hypervisor included.
    assert_no_allocation_scoped_vm_residue(&alloc, &silent_rootfs);

    handle.shutdown().await.expect("clean shutdown");
}

/// S-VM-41 / `@contract-shape:bounded-change` — a kernel path that held a valid
/// aarch64 raw PE Image when `overdrive serve` composed the Vm driver, then was
/// replaced with an image cloud-hypervisor cannot load, reaches the operator as
/// `Failed/VmKernelFormatUnsupported` naming the exact configured path, the
/// arch, and a stable format diagnosis — never a firmware size cap.
///
/// ```gherkin
/// Given "overdrive serve" booted on an aarch64 host while the configured VM
///   kernel path contained a valid raw PE Image, so the Vm driver composed
/// And that same path is then atomically REPLACED after composition but before
///   this allocation's start with an image cloud-hypervisor cannot load as a
///   raw PE Image
/// When the platform attempts to start the allocation
/// Then the allocation is Failed with VmKernelFormatUnsupported naming the
///   exact configured path and arch "aarch64"
/// And the reported cause reads as a format problem -- never a firmware size
///   cap and never "UefiTooBig"
/// And the reason is distinct from VmKernelNotFound (the path exists; only its
///   format is wrong)
/// ```
///
/// This is the classification-JOIN half of contradiction C-7. Do not duplicate
/// S-VM-17, which already proves the pure `KernelImage::validate(path, arch,
/// header)` half at the function boundary against the identical artifact. What
/// is proven here is one layer up: the per-allocation verifier's typed
/// `VmKernelFormatUnsupported { path, arch, detail }` survives the exhaustive
/// conversion (ADR-0083 §D5 row 5) and reaches the operator's rendered view
/// without any parse of validator or hypervisor prose.
///
/// The anti-assertion is the load-bearing one: cloud-hypervisor's own framing
/// of an unloadable image is `VmBoot(UefiLoad(UefiTooBig))`, which tells an
/// operator to shrink a firmware that is not the problem. That wording is
/// exactly the lie this variant exists to keep off the operator's screen. If it
/// appears at all it belongs only in the free-form `detail`, never in the
/// cause's meaning — so this scenario asserts on the rendered cause text, not
/// on the discriminant.
///
/// The post-composition replacement is load-bearing for the same reason as
/// S-VM-33 (DWD-24, checkpoint `3222f030`): booting `serve` against the bad
/// image fails before any allocation exists, which cannot satisfy this
/// scenario's `Failed`-row assertion.
#[tokio::test]
#[serial(cgroup)]
async fn kernel_replaced_with_an_incompatible_image_reads_as_a_format_problem() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let artifact_tmp = tempfile::Builder::new()
        .prefix("vm-kernel-format-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the shared reflink-capable staging root");
    let kernel = copy_kernel_for_this_test(artifact_tmp.path(), &fixture, "kernel-to-replace");

    // Composition must SUCCEED against the valid image — booting `serve`
    // against the bad one fails before any allocation exists, which
    // cannot produce the Failed row this scenario asserts on.
    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: kernel.clone(),
        rootfs_path: fixture.rootfs_path.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-kernel-format.toml",
        &vm_job_toml("vm-kernel-format", &kernel, &fixture.rootfs_path),
    );

    // ---- The TOCTOU window: atomically REPLACE, never delete. The path
    // stays present; only its content stops being loadable. ----
    let replacement = artifact_tmp.path().join("incompatible-image.staged");
    std::fs::write(&replacement, b"NOT-A-KERNEL\n".repeat(512))
        .expect("stage an image cloud-hypervisor cannot load as a boot image");
    std::fs::rename(&replacement, &kernel)
        .expect("atomically replace the configured kernel after composition");
    assert!(
        kernel.exists(),
        "S-VM-41 precondition: the configured path must still EXIST — only its format is wrong",
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the VM workload whose configured kernel was just replaced");
    let out = poll_until_failed(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    let row =
        out.snapshot.rows.first().expect("one failed allocation row for the deployed VM workload");
    let alloc = AllocationId::new(&row.alloc_id).expect("server allocation id is valid");

    let configured = kernel.display().to_string();
    let arch = host_arch().to_string();
    let reason = row.reason.clone().expect("a failed VM allocation must carry a structured reason");
    let detail = row.error.clone().expect("the verbatim driver diagnostic must be preserved");

    let TransitionReason::VmKernelFormatUnsupported {
        path: reported_path,
        arch: reported_arch,
        detail: diagnosis,
    } = reason.clone()
    else {
        panic!("S-VM-41: expected VmKernelFormatUnsupported, got {reason:?}");
    };
    assert_eq!(reported_path, configured, "the exact configured path must be named");
    assert_eq!(reported_arch, arch, "the host architecture must be named");

    // The diagnosis is the pure validator's OWN stable wording — never
    // the hypervisor's misleading firmware-size framing (contradiction
    // C-7). It must be a format statement about this arch.
    assert!(
        diagnosis.contains(&arch) && diagnosis.contains("magic"),
        "the diagnosis must be the validator's own stable format finding: {diagnosis:?}",
    );

    // Distinct from absence — the file is right there.
    assert!(
        !matches!(reason, TransitionReason::VmKernelNotFound { .. }),
        "a present-but-unloadable kernel must never read as VmKernelNotFound",
    );
    assert!(
        kernel.exists(),
        "the configured path is still present AT ASSERTION TIME, so 'present but wrong format' is \
         proven rather than assumed",
    );

    // ---- The anti-assertion: the operator never reads a firmware size
    // cap for an unloadable kernel image. ----
    let rendered = overdrive_cli::render::workload_describe(&out);
    let lowered = rendered.to_lowercase();
    for lie in ["uefitoobig", "uefi", "firmware", "too big"] {
        assert!(
            !lowered.contains(lie),
            "the rendered cause must never tell the operator to shrink a firmware that is not the \
             problem (found {lie:?}):\n{rendered}",
        );
    }
    assert!(
        lowered.contains("format"),
        "the rendered cause must read as a FORMAT problem:\n{rendered}",
    );
    assert!(
        rendered.contains(&configured) && rendered.contains(&arch),
        "the rendered operator view must name the exact configured path and the arch:\n{rendered}",
    );
    assert_named_cause_is_rendered(&rendered, &reason, &detail);

    // The preflight rejects before any hypervisor is spawned.
    assert_no_allocation_scoped_vm_residue(&alloc, &fixture.rootfs_path);

    handle.shutdown().await.expect("clean shutdown");
}
