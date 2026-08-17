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
//!
//! Step 03-02 (S-VM-37) closes the other half of AC-09 — the unclassified
//! fallthrough. It substitutes the `Vmm` port at the same `vmm_override` seam
//! S-VM-35 established, so it needs no guest and no KVM; the file-level gate
//! over-gates it, which is a layout fact rather than a property of that
//! scenario.
//!
//! Steps 03-03 and 03-04 (S-VM-38, S-VM-39, S-VM-40) close the whole of AC-10.
//! Step 03-03 (S-VM-38) and step 03-04's S-VM-39 are activated; S-VM-40 alone
//! stays scaffolded RED at the bottom of this file — not for want of work, but
//! because no production Schedule execution path exists to drive (see its own
//! doc comment for the five refusing stages and the ADR-0051 OQ-5 / ADR-0064
//! OQ-5 deferral behind them). Their capability truth is mixed once more:
//! S-VM-39 genuinely boots a guest and reaches Running, while S-VM-38 is an
//! in-process semantic rejection that spawns no VMM at all and would run
//! anywhere — it rides here for scenario cohesion, so the `kvm-tests` gate
//! over-gates it too.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::expect_used, clippy::missing_panics_doc)]

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, WorkloadDescribeOutput, describe};
use overdrive_cli::http_client::CliError;
use overdrive_control_plane::VmBootArtifacts;
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::TransitionReason;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::vmm::Vmm;
use overdrive_core::vm::config::{HostArch, RootfsPlan, VmRunDir};
use overdrive_host::CloudHypervisorVmm;
use overdrive_sim::SimVmm;
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

/// Cross-builds a tiny static-musl binary that loops forever until it is
/// killed — the "guest that reaches Running and stays there" shape S-VM-39
/// needs. A guest command that exits promptly makes `Running` a transient
/// window the poller can legitimately miss, which would turn a real
/// regression into an intermittent one.
///
/// Deliberately a file-local copy of `vm_walking_skeleton.rs`'s helper of
/// the same name rather than a cross-module reference: sibling test modules
/// cannot see each other's private items, and this file is already
/// self-contained by convention (`shared_staging_root`, `spawn_vm_server`,
/// `config_path`, `write_toml` and `build_empty_rootfs` are all file-local
/// copies of the same shapes). Widening the walking skeleton's surface to
/// share ~50 lines of `losetup` plumbing would couple two Tier-3 files that
/// are deliberately independent.
fn build_spin_binary(tmp: &Path) -> PathBuf {
    let src = tmp.join("spin.rs");
    std::fs::write(
        &src,
        "fn main() { loop { std::thread::sleep(std::time::Duration::from_secs(3600)); } }",
    )
    .expect("write the long-lived spin source");
    let out = tmp.join("spin");
    let status = Command::new("rustc")
        .args(["--edition", "2021", "-C", "opt-level=0", "-C", "target-feature=+crt-static"])
        .args(["--target", "x86_64-unknown-linux-musl"])
        .arg("-o")
        .arg(&out)
        .arg(&src)
        .status()
        .expect("spawn rustc for the long-lived spin binary");
    assert!(status.success(), "rustc must build the long-lived spin binary");
    out
}

/// Stages a PER-TEST COPY of the shared fixture's rootfs image with an
/// additional static binary injected at `/sbin/<guest_name>`, via a
/// loopback mount on the HOST before the guest ever boots. The shared
/// fixture artifact is never mutated, so the concurrent Tier-3 VM files
/// reusing the same staging root (the fixture's AC5 contract) are
/// unaffected.
///
/// Load-bearing for S-VM-39: the shared fixture rootfs carries ONLY
/// `overdrive-init` (at `/sbin/init` and `/init`) plus empty mountpoints
/// and two device nodes — there is no `/sbin/true`, no shell, no
/// coreutils. Re-invoking `/sbin/init` as the operator command is
/// explicitly NOT a valid substitute: it would dial the beacon vsock a
/// second time, which the host never accepts, hanging the guest.
///
/// Runs as root (this suite runs under `cargo xtask metal run --`), so
/// `losetup` / `mount` / `umount` need no further escalation.
fn stage_rootfs_with_extra_binary(
    tmp: &Path,
    fixture: &VmFixture,
    host_bin: &Path,
    guest_name: &str,
) -> PathBuf {
    let rootfs_copy = tmp.join("rootfs.ext4");
    std::fs::copy(&fixture.rootfs_path, &rootfs_copy)
        .expect("copy the shared fixture rootfs into a per-test working copy");

    let mnt = tmp.join("rootfs-mnt");
    std::fs::create_dir_all(&mnt).expect("create loopback mount point");

    let losetup_out = Command::new("losetup")
        .args(["--find", "--show"])
        .arg(&rootfs_copy)
        .output()
        .expect("spawn losetup --find --show");
    assert!(
        losetup_out.status.success(),
        "losetup --find --show failed: {}",
        String::from_utf8_lossy(&losetup_out.stderr),
    );
    let loop_dev = String::from_utf8_lossy(&losetup_out.stdout).trim().to_owned();

    let mount_status =
        Command::new("mount").arg(&loop_dev).arg(&mnt).status().expect("spawn mount");
    assert!(mount_status.success(), "mount {loop_dev} {} failed", mnt.display());

    let dest = mnt.join("sbin").join(guest_name);
    std::fs::copy(host_bin, &dest).expect("copy the extra binary into the mounted rootfs");
    let mut perms = std::fs::metadata(&dest).expect("stat the copied guest binary").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&dest, perms).expect("chmod the copied guest binary executable");

    let umount_status = Command::new("umount").arg(&mnt).status().expect("spawn umount");
    assert!(umount_status.success(), "umount {} failed", mnt.display());
    // Best-effort detach — a leaked loop device affects host hygiene, not
    // the correctness of any assertion below.
    let _ = Command::new("losetup").arg("-d").arg(&loop_dev).status();

    rootfs_copy
}

/// Every allocation-scoped VM resource a rejected or aborted start must
/// leave behind: none. Named once so all six scenarios in this file
/// assert the SAME four facts (hypervisor process, per-launch rootfs
/// clone, run directory, cgroup scope) rather than drifting apart.
/// S-VM-37 (step 03-02) is the sixth caller, and it calls this once per
/// unclassified run: an unclassified rejection leaks no more than a
/// named one.
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

/// The operator-visible half of AC-09, asserted the same way by all six
/// scenarios: the cause must reach the rendered `overdrive workload
/// describe` output in the ratified `TransitionReason::human_readable()`
/// vocabulary — not merely the verbatim driver diagnostic, and not merely
/// a path that happens to appear inside it.
///
/// The second assertion is what stops this from being a tautology. Every
/// NAMED cause in this file carries a `detail` whose prose differs from
/// the cause's own wording (`open kernel image {p}: ...` vs `VM kernel not
/// found: {p}`; `stat rootfs master {p}: ...` vs `VM rootfs not found:
/// {p}`), so a renderer that emitted ONLY the detail cannot satisfy the
/// containment check. Pinning that divergence here means a future change
/// that collapses the two — making the cause's wording a substring of the
/// detail — fails loudly rather than silently hollowing out every
/// scenario's operator-facing assertion.
///
/// S-VM-37's UNCLASSIFIED cause is the one caller for which that guard is
/// satisfied structurally rather than by evidence: `DriverInternalError`'s
/// `human_readable()` EMBEDS the detail, so a detail can never contain it
/// and the check passes for free. The containment check below is still
/// worth everything there — it demands the whole `driver internal error:
/// {detail}` string, so the operator provably read the LABEL and not just
/// the diagnostic — but the divergence the guard is meant to prove is
/// asserted directly at that scenario's own call site instead.
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

/// A `[job]` + `[vm]` spec whose guest command is caller-chosen.
///
/// Every rejection scenario in this file is refused before the guest ever
/// execs anything, so the command they pass is inert — [`vm_job_toml`] fixes
/// it once for all of them. S-VM-39 is the one scenario that must actually
/// REACH the exec, so it needs a command that exists in its own staged
/// rootfs and does not return.
fn vm_job_toml_with_command(id: &str, command: &str, kernel: &Path, rootfs: &Path) -> String {
    format!(
        "[job]\nid = \"{id}\"\n\n[vm]\ncommand = \"{command}\"\nargs = []\n\
         kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\n\
         memory_bytes = 134217728\n",
        kernel.display(),
        rootfs.display(),
    )
}

fn vm_job_toml(id: &str, kernel: &Path, rootfs: &Path) -> String {
    vm_job_toml_with_command(id, "/sbin/true", kernel, rootfs)
}

/// [`vm_job_toml`]'s `[service]` twin: the SAME `[vm]` driver table and
/// `[resources]` block, with the kind section swapped and a single valid
/// `[[listener]]` added so the spec is well-formed in every respect
/// EXCEPT the `[service]` + `[vm]` combination S-VM-38 rejects. A spec
/// that were otherwise invalid could be refused for the wrong reason and
/// the scenario would prove nothing.
fn vm_service_toml(id: &str, kernel: &Path, rootfs: &Path) -> String {
    format!(
        "[service]\nid = \"{id}\"\nreplicas = 1\n\n[[listener]]\nport = 8080\n\
         protocol = \"tcp\"\n\n[vm]\ncommand = \"/sbin/true\"\nargs = []\n\
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

/// Polls `workload describe` until the workload's first allocation row
/// reaches `wanted`, returning the snapshot that observed it.
///
/// The single row-selection rule and the single timeout message every
/// poller in this file shares — the same reason
/// `assert_no_allocation_scoped_vm_residue` is named once rather than
/// inlined per scenario. A second hand-written loop would be free to drift
/// on which row it reads or how it reports a timeout.
async fn poll_until_state(
    cfg: &Path,
    workload_id: &str,
    wanted: AllocStateWire,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let out =
            describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_owned() })
                .await
                .expect("workload describe must succeed while polling");
        if let Some(row) = out.snapshot.rows.first()
            && row.state == wanted
        {
            return out;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workload {workload_id} did not reach {wanted:?} within {max_wait:?}; last row: {:?}",
            out.snapshot.rows.first(),
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn poll_until_failed(
    cfg: &Path,
    workload_id: &str,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    poll_until_state(cfg, workload_id, AllocStateWire::Failed, max_wait).await
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

// ---------------------------------------------------------------------------
// Step 03-02 (DWD-24) — AC-09's unclassified fallthrough.
//
// The inverse duty of the five named scenarios above: where they prove a
// cause is named PRECISELY, this one proves a cause with no named class stays
// honestly unclassified rather than being guessed into its nearest named
// neighbour. It is the deliberate exception to `.claude/rules/development.md`
// § Errors' distinct-failure-modes rule — an unclassified failure IS its own
// distinct mode, and mislabelling it would send an operator to check the
// wrong thing entirely.
// ---------------------------------------------------------------------------

/// The unclassified cause's own operator-facing label —
/// `TransitionReason::DriverInternalError`'s `human_readable()` prefix. Named
/// so the test-integrity guard below can state the property it protects
/// rather than restating a literal at three call sites.
const UNCLASSIFIED_LABEL: &str = "driver internal error";

/// Two upstream diagnostics that map to NO named `VmStartFailure` class,
/// differing ONLY in wording. Deliberately shaped like real hypervisor prose
/// (and deliberately NOT like any named cause's payload: no configured path,
/// no searched-path vector, no deadline, no format finding) so that anything
/// which classified them into a named cause would have to have guessed.
const UNMAPPED_DIAGNOSTIC_A: &str =
    "cloud-hypervisor: vmm control plane rejected the launch payload (response tag 0x5c)";
const UNMAPPED_DIAGNOSTIC_B: &str =
    "cloud-hypervisor: microvm refused to arm -- unexpected device-model state 0x91";

/// One S-VM-37 run: compose the real in-process `serve` with its `Vmm` port
/// bound to a [`SimVmm`] that refuses EVERY `create` with `diagnostic`
/// verbatim, deploy a `[job]` + `[vm]` workload, and poll to `Failed`.
///
/// `after_compose` runs between composition and deploy — the same
/// post-composition TOCTOU window S-VM-33/35/41 rely on, so the
/// anti-fallthrough sub-case can replace the configured kernel without a
/// second composition entry point.
///
/// The refusal is PERSISTENT rather than one-shot on purpose: an unmappable
/// substrate failure does not heal between restart attempts, and a one-shot
/// arming would let a restart succeed into a boot-deadline ending instead —
/// making which row the poller observes a race rather than a property of the
/// failure under test.
///
/// Returns the describe output plus the handles the caller must keep alive:
/// the `ServeHandle` to shut down after asserting, and the server `TempDir`
/// whose lifetime the config path rides on.
async fn deploy_against_unclassifiable_vmm(
    id: &str,
    kernel: &Path,
    rootfs: &Path,
    diagnostic: &str,
    after_compose: impl FnOnce(),
) -> (WorkloadDescribeOutput, ServeHandle, TempDir) {
    let vmm = SimVmm::new();
    vmm.inject_persistent_create_failure(diagnostic);
    let (handle, server_tmp) = spawn_vm_server_with_vmm(
        VmBootArtifacts { kernel_path: kernel.to_path_buf(), rootfs_path: rootfs.to_path_buf() },
        Arc::new(vmm),
    )
    .await;
    let cfg = config_path(server_tmp.path());
    let spec_path =
        write_toml(server_tmp.path(), &format!("{id}.toml"), &vm_job_toml(id, kernel, rootfs));

    after_compose();

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the VM workload whose start the VMM will refuse");
    let out = poll_until_failed(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    (out, handle, server_tmp)
}

/// Everything S-VM-37 asserts about ONE unclassified run, named once so the
/// verbatim-preservation and wording-independence sub-cases cannot drift
/// apart. Returns the allocation id so the caller can close on residue.
fn assert_reads_as_unclassified_carrying(
    out: &WorkloadDescribeOutput,
    diagnostic: &str,
) -> AllocationId {
    let row =
        out.snapshot.rows.first().expect("one failed allocation row for the deployed VM workload");
    let alloc = AllocationId::new(&row.alloc_id).expect("server allocation id is valid");
    let reason = row.reason.clone().expect("a failed VM allocation must carry a structured reason");
    let detail = row.error.clone().expect("the verbatim driver diagnostic must be preserved");

    // The EXISTING unclassified cause, carrying the upstream diagnostic. This
    // feature mints no variant here: `DriverStartClass::Unclassified` maps
    // onto the pre-existing `DriverInternalError` (ADR-0083 §D5, DWD-24).
    assert_eq!(
        reason,
        TransitionReason::DriverInternalError { detail: diagnostic.to_owned() },
        "S-VM-37: an unmapped VM start failure must reach the operator as the EXISTING \
         unclassified cause carrying its verbatim diagnostic",
    );

    // The anti-tautology guard, inverted from the named scenarios': being
    // "unclassified" only means something against the named vocabulary, so
    // the outcome must be NONE of the five named VM causes.
    //
    // Stated honestly: against the equality above this is defence in depth,
    // not an independent arrow — the two cannot fail apart, because a named
    // cause already contradicts the equality (verified: a litmus that routed
    // this failure to `VmHypervisorAbsent` reddened on the equality first).
    // It is kept, and kept separate, because the equality is the assertion
    // most likely to be relaxed later — a future weakening to "matches the
    // DriverInternalError variant" would silently drop the named-vocabulary
    // exclusion, and this line is what would still catch that.
    assert!(
        !matches!(
            reason,
            TransitionReason::VmKernelNotFound { .. }
                | TransitionReason::VmRootfsNotFound { .. }
                | TransitionReason::VmHypervisorAbsent { .. }
                | TransitionReason::VmBootDeadlineExceeded { .. }
                | TransitionReason::VmKernelFormatUnsupported { .. }
        ),
        "an unmapped failure must never be dressed up as a named VM cause: {reason:?}",
    );

    // Byte-for-byte on its own channel: neither truncated nor reworded.
    assert_eq!(
        detail, diagnostic,
        "the verbatim upstream diagnostic must survive untruncated and unreworded",
    );

    // ...and both halves reach the operator's actual rendered output: the
    // diagnostic verbatim, and — via the shared integrity check — the whole
    // `driver internal error: {detail}` label, so the operator provably reads
    // that this is UNCLASSIFIED and not merely an unexplained string.
    let rendered = overdrive_cli::render::workload_describe(out);
    assert!(
        rendered.contains(diagnostic),
        "the rendered operator view must carry the verbatim upstream diagnostic:\n{rendered}",
    );
    assert_named_cause_is_rendered(&rendered, &reason, &detail);

    alloc
}

/// S-VM-37 / `@contract-shape:bounded-change` — a VM start failure the
/// platform has no named variant for reaches the operator carrying its
/// verbatim upstream diagnostic under the EXISTING unclassified cause, and is
/// never dressed up as one of the named VM causes.
///
/// ```gherkin
/// Given a VM start fails for a reason the platform has no named variant for
/// When Ana reads workload describe
/// Then the reason carries the verbatim hypervisor error text
/// And it is labelled as unclassified rather than presented as a known cause
/// ```
///
/// This is the explicit TOTAL fallback of DWD-24's conversion, not an
/// exception to the distinct-failure-modes rule: a cause with no named class
/// must stay unclassified rather than be guessed into the nearest named
/// neighbour. The operator-facing variant is the pre-existing
/// `TransitionReason::DriverInternalError` — this feature mints nothing here —
/// reached through `DriverStartClass::Unclassified` for `DriverType::Vm`,
/// whose conversion copies the already-captured verbatim
/// `DriverStartFailure.detail` across unchanged (ADR-0083 §D5). No
/// compatibility parser and no classification logic exists in the action
/// shim, and none may reappear.
///
/// Three sub-cases, all example-based (Tier 3 — sad paths are enumerated,
/// never generated):
///
/// 1. *Verbatim preservation.* One run against diagnostic A.
/// 2. *Wording independence.* A second run varying ONLY the wording. This is
///    the criterion that a single run structurally cannot prove: it takes two
///    runs to show that changing the diagnostic changes only the preserved
///    detail and never which cause class is selected.
/// 3. *Anti-fallthrough.* S-VM-41's artifact — the configured kernel replaced
///    with an image the host cannot load — driven through this SAME
///    unclassified-capable override, asserting the outcome stays
///    `VmKernelFormatUnsupported`. The fallthrough must not be able to claim
///    a failure a named cause already covers.
#[tokio::test]
#[serial(cgroup)]
async fn unmapped_start_failure_reads_as_unclassified_and_preserves_its_verbatim_cause() {
    // Test integrity, asserted before anything else runs. The shared
    // `assert_named_cause_is_rendered` guard is satisfied structurally for
    // this cause (its `human_readable()` embeds the detail, so a detail can
    // never contain it), so the property that guard exists to protect is
    // asserted here directly: a diagnostic that already spelt the
    // unclassified label would make the render assertion prove nothing.
    for diagnostic in [UNMAPPED_DIAGNOSTIC_A, UNMAPPED_DIAGNOSTIC_B] {
        assert!(
            !diagnostic.contains(UNCLASSIFIED_LABEL),
            "test integrity: a diagnostic that already spells {UNCLASSIFIED_LABEL:?} would make \
             the render assertion prove nothing; diagnostic was {diagnostic:?}",
        );
    }
    assert_ne!(
        UNMAPPED_DIAGNOSTIC_A, UNMAPPED_DIAGNOSTIC_B,
        "wording independence is only provable against two DIFFERENT diagnostics",
    );

    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let artifact_tmp = tempfile::Builder::new()
        .prefix("vm-unclassified-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the shared reflink-capable staging root");

    // ---- (1) Verbatim preservation. ----
    let (out_a, handle_a, _server_a) = deploy_against_unclassifiable_vmm(
        "vm-unclassified-a",
        &fixture.kernel_path,
        &fixture.rootfs_path,
        UNMAPPED_DIAGNOSTIC_A,
        || {},
    )
    .await;
    let alloc_a = assert_reads_as_unclassified_carrying(&out_a, UNMAPPED_DIAGNOSTIC_A);
    assert_no_allocation_scoped_vm_residue(&alloc_a, &fixture.rootfs_path);
    handle_a.shutdown().await.expect("clean shutdown");

    // ---- (2) Wording independence: same class, different detail only. ----
    let (out_b, handle_b, _server_b) = deploy_against_unclassifiable_vmm(
        "vm-unclassified-b",
        &fixture.kernel_path,
        &fixture.rootfs_path,
        UNMAPPED_DIAGNOSTIC_B,
        || {},
    )
    .await;
    let alloc_b = assert_reads_as_unclassified_carrying(&out_b, UNMAPPED_DIAGNOSTIC_B);
    assert_no_allocation_scoped_vm_residue(&alloc_b, &fixture.rootfs_path);
    handle_b.shutdown().await.expect("clean shutdown");

    // The wording-independence claim, stated once as a fact rather than left
    // as a deduction the reader has to make across the two blocks above: the
    // two runs selected the SAME cause constructor, and still differ — which,
    // given each run already pinned its whole cause, can only be in the
    // preserved detail. This is the criterion one run structurally cannot
    // prove.
    let cause_of = |out: &WorkloadDescribeOutput| {
        out.snapshot
            .rows
            .first()
            .and_then(|row| row.reason.clone())
            .expect("a failed VM allocation must carry a structured reason")
    };
    let (reason_a, reason_b) = (cause_of(&out_a), cause_of(&out_b));
    assert_eq!(
        std::mem::discriminant(&reason_a),
        std::mem::discriminant(&reason_b),
        "varying ONLY the diagnostic wording must not change which cause class is selected",
    );
    assert_ne!(
        reason_a, reason_b,
        "...and the two runs must still differ, in the preserved detail — the only thing that \
         changed between them",
    );

    // ---- (3) Anti-fallthrough: a named cause is never surrendered to the
    // unclassified arm. The per-allocation verifier rejects this start before
    // the VMM is reached, so the armed diagnostic cannot claim it. ----
    let kernel = copy_kernel_for_this_test(artifact_tmp.path(), &fixture, "kernel-to-replace");
    let replacement = artifact_tmp.path().join("incompatible-image.staged");
    let (out_c, handle_c, _server_c) = deploy_against_unclassifiable_vmm(
        "vm-unclassified-not-format",
        &kernel,
        &fixture.rootfs_path,
        UNMAPPED_DIAGNOSTIC_A,
        || {
            std::fs::write(&replacement, b"NOT-A-KERNEL\n".repeat(512))
                .expect("stage an image cloud-hypervisor cannot load as a boot image");
            std::fs::rename(&replacement, &kernel)
                .expect("atomically replace the configured kernel after composition");
        },
    )
    .await;
    let row_c = out_c
        .snapshot
        .rows
        .first()
        .expect("one failed allocation row for the deployed VM workload");
    let alloc_c = AllocationId::new(&row_c.alloc_id).expect("server allocation id is valid");
    let reason_c =
        row_c.reason.clone().expect("a failed VM allocation must carry a structured reason");

    // The outcome, not the ordering — and deliberately NOT a restatement of
    // S-VM-41's own naming assertions, which that scenario already owns.
    assert!(
        matches!(reason_c, TransitionReason::VmKernelFormatUnsupported { .. }),
        "an unloadable kernel image must stay VmKernelFormatUnsupported even when the VMM behind \
         it would have produced an unclassified failure: {reason_c:?}",
    );
    assert!(
        !matches!(reason_c, TransitionReason::DriverInternalError { .. }),
        "the unclassified fallthrough must never claim a failure a named cause already covers",
    );

    // The armed diagnostic is distinctive, so its ABSENCE is direct evidence
    // the VMM was never reached rather than merely out-ranked.
    let rendered_c = overdrive_cli::render::workload_describe(&out_c);
    assert!(
        !rendered_c.contains(UNMAPPED_DIAGNOSTIC_A),
        "the VMM's own diagnostic must not reach an operator whose start never got that far:\n\
         {rendered_c}",
    );
    assert_no_allocation_scoped_vm_residue(&alloc_c, &fixture.rootfs_path);
    handle_c.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// Step 03-03 (DWD-24 / US-VM-6) — the whole of AC-10's negative half.
//
// The one scenario in this file that asserts an outcome the platform reaches
// WITHOUT a driver at all: the spec never becomes intent, so there is no
// allocation, no VMM, and no `Failed` row. Every helper above that closes over
// an `AllocationId` is therefore inapplicable here by construction rather than
// by omission.
// ---------------------------------------------------------------------------

/// S-VM-38 / `@contract-shape:bounded-change` `@error_path` `@ac-10` `@tier3`
/// `@real-io` — a spec declaring both `[service]` and `[vm]` is rejected before
/// anything is scheduled, and the rejection tells the operator which
/// capabilities are missing and where they are tracked.
///
/// ```gherkin
/// Given Ana has written a spec declaring both [service] and [vm]
/// When she runs "overdrive deploy web.toml"
/// Then the deploy is rejected before anything is scheduled
/// And the error names guest networking, guest probes and guest-stack mTLS as
///   missing, citing GH #257 and GH #222
/// ```
///
/// A semantic rejection with guidance, not a parse error — the established
/// precedent is `ParseError::ProbesNotAllowedOnKind`
/// (`crates/overdrive-core/src/aggregate/workload_spec.rs`), which rejects
/// `[[health_check.*]]` on a non-Service workload and carries per-kind
/// `guidance` text so the operator learns *why* rather than merely being
/// refused. This scenario is the mirror image of that shape: Service is the
/// kind being refused rather than the kind being required.
///
/// Unlike every other scenario in this file, nothing here needs a hypervisor,
/// a guest, or KVM: the rejection happens in-process before an intent is
/// committed. It lives here for cohesion with the rest of AC-09/AC-10 and
/// therefore inherits the file-level `integration-tests,kvm-tests` gate. That
/// over-gating is a layout consequence, not a capability claim about this
/// scenario (roadmap 03-03).
///
/// The three missing capabilities the rejection must name, asserted
/// INDEPENDENTLY below: a single `contains` on one phrase would pass against
/// a message that silently dropped the other two.
///
/// `assert_no_allocation_scoped_vm_residue` deliberately does NOT apply here
/// and is not forced: no `AllocationId` is ever minted, which is precisely
/// what this scenario proves. Asserting on residue for an allocation that does
/// not exist would be vacuous.
#[tokio::test]
#[serial(cgroup)]
async fn service_plus_vm_spec_is_rejected_before_anything_is_scheduled() {
    const MISSING_CAPABILITIES: [&str; 3] =
        ["guest networking", "guest-reachable probes", "guest-stack mtls interception"];
    const TRACKING_ISSUES: [&str; 2] = ["#257", "#222"];
    const WORKLOAD_ID: &str = "vm-service-rejected";

    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    // Both artifacts are VALID and PRESENT throughout: the rejection is
    // provably about the [service] + [vm] combination, never about a
    // broken or absent artifact (which is what S-VM-33/34/41 cover).
    assert!(
        fixture.kernel_path.exists() && fixture.rootfs_path.exists(),
        "S-VM-38 precondition: both configured artifacts must be present, so nothing but the \
         kind/driver combination can explain the rejection",
    );

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: fixture.rootfs_path.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-service-rejected.toml",
        &vm_service_toml(WORKLOAD_ID, &fixture.kernel_path, &fixture.rootfs_path),
    );

    // ---- The scoping control. The IDENTICAL [vm] driver table under
    // [job] still parses, so a failure below is the [service] rejection
    // firing and not this fixture's [vm] block being unacceptable
    // anywhere. Steps 03-04's two scenarios prove the positive paths
    // reach Running; this line only fixes the blame for THIS one. ----
    overdrive_core::aggregate::WorkloadSpecInput::from_toml_str(&vm_job_toml(
        WORKLOAD_ID,
        &fixture.kernel_path,
        &fixture.rootfs_path,
    ))
    .expect("the same [vm] driver table under [job] must still parse — the rejection is scoped");

    // ---- The rejection, off the production deploy surface. ----
    let rejection = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect_err("a spec declaring both [service] and [vm] must be rejected");
    let message = rejection.to_string();
    let lowered = message.to_lowercase();

    for capability in MISSING_CAPABILITIES {
        assert!(
            lowered.contains(capability),
            "the rejection must name {capability:?} as a missing capability:\n{message}",
        );
    }
    for issue in TRACKING_ISSUES {
        assert!(
            message.contains(issue),
            "the rejection must cite GH {issue} by number so the operator can find the tracking \
             issue:\n{message}",
        );
    }

    // ---- Nothing was scheduled. The half a parser-level unit test
    // cannot reach, and the reason this scenario is Tier 3 at all: the
    // describe surface reports the workload as unknown, so no intent was
    // committed and no allocation row exists to describe. ----
    let described =
        describe(DescribeArgs { id: WORKLOAD_ID.to_owned(), config_path: cfg.clone() }).await;
    match described {
        Err(CliError::HttpStatus { status, body }) => {
            assert_eq!(
                status, 404,
                "a rejected deploy must commit no intent, so the workload must be unknown",
            );
            assert_eq!(body.error, "not_found", "error class must be `not_found`");
        }
        Ok(out) => panic!(
            "a rejected deploy must schedule nothing, but describe found the workload with {} \
             allocation row(s)",
            out.allocations_total,
        ),
        Err(other) => {
            panic!("expected HTTP 404 for the never-committed workload, got {other:?}")
        }
    }

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// Step 03-04 — RED scaffolds (S-VM-39, S-VM-40): AC-10's positive half.
//
// Shape per `.claude/rules/testing.md` § "RED scaffolds and intentionally-
// failing commits": `#[should_panic(expected = "RED scaffold")]` plus a panic
// body naming the scenario. The bar stays green while every pending scenario
// stays discoverable via `grep -rn 'should_panic.*RED scaffold' crates/`, and
// deleting the `panic!` without writing the assertions trips the attribute
// rather than passing silently.
//
// Both carry `#[test]` TODAY, deliberately: their bodies are a single panic,
// so they await nothing and touch no cgroup. `#[tokio::test]` and
// `#[serial(cgroup)]` are claims about what a body DOES, and neither body
// makes them yet. Each rustdoc names the attributes its activated form must
// carry, so the swap happens with the assertions and not before.
//
// The seven activated scenarios above (S-VM-33/34/35/36/37/38/41) are
// untouched.
// ---------------------------------------------------------------------------

/// S-VM-39 / `@contract-shape:bounded-change` `@happy_path` `@ac-10` `@tier3`
/// `@real-io` `@requires-kvm` — a spec declaring both `[job]` and `[vm]` is
/// accepted, scheduled, and its VM allocation reaches Running through the
/// production `VmDriver` path.
///
/// ```gherkin
/// Given Ana has written a spec declaring both [job] and [vm]
/// When she runs "overdrive deploy render.toml"
/// Then the workload is accepted and scheduled
/// And its VM allocation reaches Running through the production VmDriver path
/// ```
///
/// One of the two regression guards proving S-VM-38's rejection stayed scoped
/// to `[service]`. That guard only holds if this allocation genuinely RUNS: a
/// spec that merely parses proves the rejection did not fire, but says nothing
/// about whether the positive path still reaches a guest. DWD-24 made this
/// explicit — "the test must drive real serve composition and the production
/// `VmDriver` path; a parser-only acceptance assertion is insufficient" — which
/// is why `@requires-kvm` here is a real capability claim rather than the
/// file-level layout gate.
///
/// # Activation plan (step 03-04)
///
/// **Attributes**: swap `#[test]` for `#[tokio::test]` + `#[serial(cgroup)]`.
/// This one really does create an allocation-scoped cgroup scope, so the
/// serialisation is load-bearing rather than merely conventional.
///
/// **Reuse as-is**: `shared_staging_root`, `VmFixture::provision` (the shared
/// kernel/rootfs pair that actually boots — do NOT reach for
/// `build_empty_rootfs`, whose whole purpose is a guest that never beacons),
/// `spawn_vm_server`, `config_path`, `write_toml`, `deploy`, and `vm_job_toml`
/// unchanged: it already emits `[job]` + `[vm]`, so this scenario's spec needs
/// no new builder at all.
///
/// **Still to build**: a `poll_until_running` sibling of `poll_until_failed`.
/// Prefer extracting a shared `poll_until_state(cfg, id, wanted, max_wait)`
/// that both delegate to, so the two pollers cannot drift in their timeout
/// message or their row-selection rule — the same reason
/// `assert_no_allocation_scoped_vm_residue` is named once for every scenario
/// here rather than inlined five times.
///
/// **Assertions**:
///
/// * `deploy(..)` succeeds — the workload is admitted, not rejected.
/// * The allocation reaches `AllocStateWire::Running` within a ceiling
///   comfortably above the boot deadline, so a pass means the guest booted
///   rather than the poll being generous.
/// * The row carries no failure reason while Running. A Running row with a
///   populated reason would mean the state and the cause disagree.
/// * The allocation-scoped VM resources DO exist while Running — the run
///   directory and cgroup scope `assert_no_allocation_scoped_vm_residue`
///   asserts the ABSENCE of on every rejected start. Asserting their presence
///   here is what distinguishes "the production `VmDriver` path ran" from "the
///   parser accepted the spec", and it is the assertion that makes this a
///   genuine regression guard for 03-03.
/// * Close with `handle.shutdown()`; the Running guest is this scenario's own
///   and must not outlive it.
///
/// # Two departures from that plan, both deliberate
///
/// **The guest command is not `vm_job_toml`'s inert `/sbin/true`.** That path
/// does not exist in the shared fixture rootfs (which carries only
/// `overdrive-init`, empty mountpoints and two device nodes), so the guest
/// would beacon `READY`, fail its exec, and power down — making `Running` a
/// window the poller can miss and this scenario intermittent. It stages its
/// own rootfs copy carrying a binary that never returns, so `Running` is a
/// state the assertions can actually stand in.
///
/// **It stops the workload through the production `stop` verb before shutting
/// the server down.** A guest that never exits on its own is never reaped by
/// `handle.shutdown()` alone — production spawns the VMM with
/// `kill_on_drop(false)` — and the orphan then contaminates the next
/// serialized test's `/proc` scan. This is the lesson S-VM-05 learned on the
/// metal box (walking skeleton, fourth pass), applied here up front. Test
/// hygiene, not an additional acceptance claim.
#[tokio::test]
#[serial(cgroup)]
async fn job_plus_vm_spec_is_accepted_and_its_allocation_reaches_running() {
    const WORKLOAD_ID: &str = "vm-job-accepted";

    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let artifact_tmp = tempfile::Builder::new()
        .prefix("vm-job-accepted-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the shared reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let spin = build_spin_binary(artifact_tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(artifact_tmp.path(), &fixture, &spin, "spinjobvm");

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-job-accepted.toml",
        &vm_job_toml_with_command(WORKLOAD_ID, "/sbin/spinjobvm", &fixture.kernel_path, &rootfs),
    );

    // ---- Accepted, not rejected. The `[service]` + `[vm]` refusal S-VM-38
    // proves must not reach the `[job]` family. ----
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("a spec declaring both [job] and [vm] must be ACCEPTED, never rejected");
    assert_eq!(submit.workload_id, WORKLOAD_ID, "the accepted workload keeps the declared id");

    // ---- ...and scheduled, and RUN. A ceiling comfortably above the 30s
    // boot deadline, so a pass means the guest booted rather than the poll
    // being generous. ----
    let out = poll_until_state(
        &cfg,
        &submit.workload_id,
        AllocStateWire::Running,
        Duration::from_secs(90),
    )
    .await;
    let row = out.snapshot.rows.first().expect("one allocation row for the running VM workload");
    let alloc = AllocationId::new(&row.alloc_id).expect("server allocation id is valid");
    // The affirmative form of "carries no failure cause", and a stronger
    // claim than the negative: `Started` is the progress marker production
    // writes when the driver returned `Ok(handle)` — which, per `start`'s
    // three-way boot race, happens ONLY on the beacon-win arm. Pinning it
    // therefore states that a real guest dialled the beacon, and
    // structurally excludes every named failure cause at the same time.
    assert_eq!(
        row.reason,
        Some(TransitionReason::Started),
        "a Running VM allocation must carry the driver's own `Started` progress marker — any \
         other cause would mean the state and the cause disagree",
    );

    // ---- The claim this scenario exists to make: the production `VmDriver`
    // path RAN, not merely that the parser accepted the spec. Asserted as
    // the exact inverse of `assert_no_allocation_scoped_vm_residue`'s four
    // facts, so "ran" and "left nothing behind" are stated against the same
    // resource set and cannot drift apart. ----
    let rootfs_plan = RootfsPlan::for_alloc(rootfs.clone(), 0, &alloc);
    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), &alloc);
    let scope_dir = CgroupPath::for_alloc(&alloc).resolve(Path::new("/sys/fs/cgroup"));

    assert!(
        !no_cloud_hypervisor_process_for_alloc(run_dir.path()),
        "a Running VM allocation must have a live hypervisor process for {}",
        run_dir.path().display(),
    );
    assert!(
        rootfs_plan.clone_dest().exists(),
        "a Running VM allocation must have its per-launch rootfs clone at {}",
        rootfs_plan.clone_dest().display(),
    );
    assert!(
        run_dir.path().exists(),
        "a Running VM allocation must have its run directory at {}",
        run_dir.path().display(),
    );
    assert!(
        scope_dir.exists(),
        "a Running VM allocation must have its cgroup scope at {}",
        scope_dir.display(),
    );

    // ---- Hygiene: drive the never-self-terminating guest to Terminated
    // through the production stop verb, so no real hypervisor outlives this
    // test (see the doc comment above). ----
    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop the long-lived VM workload before shutdown");
    // `poll_until_state` IS the assertion here — it panics if the workload
    // does not reach Terminated in time. A further assert on the returned
    // row's state would be tautological.
    let _terminated = poll_until_state(
        &cfg,
        &submit.workload_id,
        AllocStateWire::Terminated,
        Duration::from_secs(60),
    )
    .await;

    handle.shutdown().await.expect("clean shutdown");
}

/// S-VM-40 / `@contract-shape:bounded-change` `@happy_path` `@ac-10` `@tier3`
/// `@real-io` `@requires-kvm` — a spec declaring a cron `[schedule]` and `[vm]`
/// is accepted and scheduled, and when its first firing becomes due its VM
/// allocation reaches Running through the production `VmDriver` path.
///
/// ```gherkin
/// Given Ana has written a spec declaring [schedule] with a cron expression and [vm]
/// When she runs "overdrive deploy nightly.toml"
/// Then the workload is accepted and scheduled
/// And when its first firing becomes due, its VM allocation reaches Running through
///   the production VmDriver path
/// ```
///
/// The second regression guard for S-VM-38's scoping, and the one that closes
/// Slice 02's "accepted and run" promise for the Schedule kind. DWD-24 promoted
/// this scenario's Then from "accepted" to "reaches Running", which is what
/// makes it the 46th `@requires-kvm` scenario — the guest boot is the point,
/// not an incidental cost.
///
/// # Activation plan (step 03-04)
///
/// **Attributes**: swap `#[test]` for `#[tokio::test]` + `#[serial(cgroup)]`,
/// for the same reason as S-VM-39 — a real fired allocation, a real scope.
///
/// **Reuse as-is**: `shared_staging_root`, `VmFixture::provision`,
/// `spawn_vm_server`, `config_path`, `write_toml`, `deploy`, and the
/// `poll_until_running` helper S-VM-39 introduces.
///
/// **Still to build**: a `vm_schedule_toml` builder emitting `[schedule]` with
/// a cron expression plus `[job]` + `[vm]` — the Schedule kind composes a job
/// inner spec, so this is `vm_job_toml` with a `[schedule]` table added rather
/// than a separate shape. Confirm the exact table layout against the Schedule
/// deploy surface already exercised by `tests/integration/deploy_schedule.rs`
/// before writing it; do not infer the cron field name from this note.
///
/// **Assertions**:
///
/// * `deploy(..)` succeeds and the workload is recorded as scheduled.
/// * When the first firing becomes due, the fired allocation reaches
///   `AllocStateWire::Running`.
/// * The poll ceiling must comfortably exceed the cron granularity, the same
///   way S-VM-36's 120s ceiling comfortably exceeds its 30s boot deadline. A
///   ceiling near the firing interval turns a real pass into a coin flip and,
///   worse, turns a real regression into an intermittent one. Pick the cron
///   expression and the ceiling together so that a timeout means the firing
///   did not happen, never that the test was impatient.
/// * As in S-VM-39, assert the allocation-scoped run directory and cgroup scope
///   EXIST while Running — reaching Running through the production `VmDriver`
///   path is the claim, and a state field alone does not evidence it.
/// * Close with `handle.shutdown()`.
///
/// # BLOCKED at step 03-04 — no Schedule execution path exists to drive
///
/// The activation plan above cannot be executed as written. It assumes a
/// production path from `overdrive deploy <schedule-spec>` to a fired
/// allocation; **no such path exists**, and building one is not this step's
/// scope. The parser half is fine — `SectionPresence::validated` deliberately
/// leaves the job family untouched, so `[schedule]` + `[job]` + `[vm]` parses
/// to `WorkloadSpecInput::Schedule` — but every stage after it refuses:
///
/// 1. `commands::deploy::deploy` (the JSON-ack lane this file calls) has no
///    Schedule arm; a Schedule body falls through to the legacy flat
///    `JobSpecInput` parser, which fails on the `[job]`-nested `id`.
/// 2. `commands::deploy::deploy_streaming` rejects `WorkloadSpecInput::
///    Schedule(_)` outright — "schedule submission is not yet implemented
///    (ADR-0051 OQ-5)".
/// 3. The server's submit handler rejects `SubmitSpecInput::Schedule(_)` with
///    the same message (`overdrive-control-plane/src/handlers.rs`).
/// 4. `ScheduleV2::from_submit` and `ScheduleV2::to_describe`
///    (`overdrive-core/src/aggregate/mod.rs`) are both `todo!()` RED
///    scaffolds — no Schedule intent can be persisted or described.
/// 5. Nothing evaluates a `CronExpr` at runtime. There is no schedule
///    reconciler in `overdrive-core/src/reconcilers/`, and no `Action` that
///    mints a per-firing allocation — so "when its first firing becomes due"
///    has no producer at all.
///
/// Reaching `Running` therefore requires an entire Schedule execution
/// subsystem: a CLI arm, a wire arm, a validating aggregate constructor,
/// intent persistence, a describe projection, and a cron-firing reconciler
/// plus its action. That work is a documented cross-feature deferral
/// (ADR-0051 OQ-5 / ADR-0064 OQ-5, tracked by the CLI's own
/// `SCHEDULE_EXECUTION_TRACKING_URL` = GH #166), and the approved design for
/// this feature names none of those primitives. Per `CLAUDE.md` §
/// "Implement to the design", the crafter returns a blocker rather than
/// inventing the surface.
///
/// The scaffold therefore stays RED and discoverable
/// (`grep -rn 'should_panic.*RED scaffold' crates/`) pending a scope ruling.
/// S-VM-38's scoping is NOT left unguarded meanwhile: S-VM-39 above is the
/// regression guard, and it is litmus-proven — widening the `[service]`
/// rejection to the job family reddens it at the acceptance assertion.
#[test]
#[should_panic(expected = "RED scaffold")]
fn scheduled_vm_workload_reaches_running_when_its_first_firing_becomes_due() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-40 / step 03-04 -- a spec declaring a cron \
         [schedule] and [vm] must be accepted and scheduled, and its first firing must reach \
         Running through the production VmDriver path)"
    );
}
