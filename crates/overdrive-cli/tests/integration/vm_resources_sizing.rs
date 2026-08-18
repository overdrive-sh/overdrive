//! S-VM-69 / S-VM-70 — declared `[resources] cpu_milli` sizes the guest's
//! REAL online vCPU count (Tier-3, `@real-io` `@requires-kvm`, AC-17,
//! US-VM-5; `docs/feature/microvm-driver-cloud-hypervisor/distill/
//! test-scenarios.md` §§2236-2258).
//!
//! Driven through the real in-process `overdrive serve` + `overdrive
//! deploy` CLI handlers (per `crates/overdrive-cli/CLAUDE.md` § "Integration
//! tests — no subprocess") against a REAL Cloud Hypervisor VMM, under
//! `cargo xtask metal run --` as root on a real `x86_64+KVM` box (Lima on
//! Apple Silicon has no nested KVM).
//!
//! # Guest-observed, never config-asserted
//!
//! The AC is explicit: the vCPU count is observed **FROM INSIDE THE
//! GUEST**, never asserted against the constructed hypervisor config. The
//! mechanism reuses the proven beacon `EXIT <i32>` channel (S-VM-02): the
//! injected guest command reads its own online CPU count
//! (`std::thread::available_parallelism`, which resolves via
//! `sched_getaffinity(2)` — the guest kernel's view of its online vCPUs,
//! needing neither `/proc` nor coreutils) and exits WITH that count.
//! `overdrive-init` forwards the real exit status onto the beacon
//! (`exit_status_to_wire`), the host classifies it, and it lands in
//! `row.reason` as the operator-observable `exit_code`. Nothing here reads
//! `VmConfig.vcpus`.
//!
//! A guest that never booted (0 vCPUs would be unbootable) produces a
//! boot-deadline failure with NO `exit_code` (S-VM-03's shape), never a
//! `WorkloadCrashedImmediately { exit_code: Some(_) }`. So a terminal row
//! carrying the exit code IS the proof the guest reached Running and
//! executed on real vCPUs — which is exactly S-VM-70's "reaches Running"
//! claim.
//!
//! # Port-to-port litmus
//!
//! Delete the `cpu_milli -> vcpus` wiring in `VmDriver::start` (revert to
//! the fixed `VmHostLayout.vcpus` template of 1) and S-VM-69 stays RED:
//! `cpu_milli = 2000` would boot 1 vCPU, the guest would exit 1, and the
//! expected `exit_code` of 2 would never arrive.
//!
//! `#[serial(cgroup)]` — every test here boots a real `overdrive serve`
//! against the machine-global `/sys/fs/cgroup/overdrive.slice` tree; the
//! rationale is `vm_walking_skeleton.rs`'s own module doc.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::missing_panics_doc, clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, deploy};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, WorkloadDescribeOutput, describe};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::TransitionReason;
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tempfile::TempDir;

// ---------------------------------------------------------------------
// Fixture staging — copied file-local (sibling Tier-3 VM test modules
// cannot see each other's private items, per vm_walking_skeleton.rs).
// ---------------------------------------------------------------------

/// The shared staging root every Tier-3 VM test file provisions against
/// (safe under concurrent nextest processes, per `vm_fixture`'s AC5).
fn shared_staging_root() -> PathBuf {
    overdrive_testing::vm_fixture::default_staging_root()
}

/// A server-side tempdir (holding `data/` + `conf/`) on the
/// reflink-capable staging root, NOT the system tmpdir — each per-launch
/// rootfs clone is FICLONE'd into `clone_staging_dir(data_dir)`, and
/// FICLONE is intra-filesystem (co-locating `data_dir` with the masters
/// respects the one-VM-data-partition production invariant).
fn server_tmp_on_staging_root() -> TempDir {
    tempfile::Builder::new()
        .prefix("vm-resources-")
        .tempdir_in(shared_staging_root())
        .expect("server tempdir on the reflink-capable staging root")
}

/// Cross-builds a tiny static-musl binary that reads its OWN online vCPU
/// count and `exit`s with it — the guest-observed reporting channel. Uses
/// `std::thread::available_parallelism` (backed by `sched_getaffinity(2)`
/// on Linux): the guest kernel's own count of online vCPUs, requiring
/// neither `/proc` (the minimal rootfs mounts none) nor coreutils (`nproc`
/// is absent). Mirrors `vm_walking_skeleton.rs`'s `build_exit_code_binary`
/// cross-build shape — a direct `rustc` invocation, no throwaway Cargo
/// project, `x86_64-unknown-linux-musl` (the only target the fixture's
/// kernel staging supports).
fn build_report_cpus_binary(tmp: &Path) -> PathBuf {
    let src = tmp.join("report_cpus.rs");
    std::fs::write(
        &src,
        // available_parallelism is NonZeroUsize; the guest's online vCPU
        // count is always >= 1 and (here) <= 2, well within an exit code.
        "fn main() {\n    let n = std::thread::available_parallelism()\n        \
         .map(|v| v.get())\n        .unwrap_or(0);\n    std::process::exit(n as i32);\n}\n",
    )
    .expect("write the report-cpus guest source");
    let out = tmp.join("report_cpus");
    let status = Command::new("rustc")
        .arg("--edition")
        .arg("2021")
        .arg("-C")
        .arg("opt-level=0")
        .arg("-C")
        .arg("target-feature=+crt-static")
        .arg("--target")
        .arg("x86_64-unknown-linux-musl")
        .arg("-o")
        .arg(&out)
        .arg(&src)
        .status()
        .expect("spawn rustc for the report-cpus guest binary");
    assert!(status.success(), "rustc must build the tiny static-musl report-cpus binary");
    out
}

/// Stages a PER-TEST COPY of the shared fixture's rootfs image with an
/// additional static binary injected at `/sbin/<guest_name>`, via a
/// loopback mount on the HOST (before the guest ever boots) — the shared
/// fixture artifact is never mutated, so concurrent Tier-3 VM test files
/// reusing the same staging root (per AC5) are unaffected. Runs as root
/// (the whole suite runs under `cargo xtask metal run --`).
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
        .arg("--find")
        .arg("--show")
        .arg(&rootfs_copy)
        .output()
        .expect("spawn losetup --find --show");
    assert!(
        losetup_out.status.success(),
        "losetup --find --show failed: {}",
        String::from_utf8_lossy(&losetup_out.stderr)
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
    // Best-effort loop-device detach — a leaked loop device affects host
    // hygiene only, not this test's assertions.
    let _ = Command::new("losetup").arg("-d").arg(&loop_dev).status();

    rootfs_copy
}

// ---------------------------------------------------------------------
// Server composition + spec authoring + polling (file-local copies).
// ---------------------------------------------------------------------

/// Spawns a real in-process `overdrive serve` with `SimDataplane` injected
/// (these functional-sizing scenarios do not need the real `EbpfDataplane`
/// / mTLS composition), through the UNGATED `run_with_dataplane`
/// entrypoint — no node-level artifact seam. Every VM allocation sources
/// its kernel and rootfs from its own `[vm]` spec.
async fn spawn_vm_server() -> (ServeHandle, TempDir) {
    let tmp = server_tmp_on_staging_root();
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = ServeArgs { bind, data_dir, config_dir };
    let handle = overdrive_cli::commands::serve::run_with_dataplane(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
    )
    .await
    .expect("serve::run_with_dataplane");
    (handle, tmp)
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

/// A `[job]`+`[vm]`+`[resources]` TOML with a caller-chosen `cpu_milli`.
/// The `[resources]` block is the single source of truth for VM size
/// (feature-delta §263 — `[vm]` carries no cpu/memory field).
fn vm_job_toml(id: &str, command: &str, kernel: &Path, rootfs: &Path, cpu_milli: u32) -> String {
    format!(
        "[job]\nid = \"{id}\"\n\n[vm]\ncommand = \"{command}\"\nargs = []\n\
         kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = {cpu_milli}\n\
         memory_bytes = 268435456\n",
        kernel.display(),
        rootfs.display(),
    )
}

fn write_toml(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write toml");
    path
}

/// Polls `workload describe` every 500ms until the workload's first
/// allocation row reaches a terminal state (`Terminated` or `Failed`),
/// returning the final snapshot. Real wall-clock polling (Tier-3, no
/// `SimClock` at this layer).
async fn poll_until_terminal(
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
        if out.snapshot.rows.first().is_some_and(|row| {
            matches!(row.state, AllocStateWire::Terminated | AllocStateWire::Failed)
        }) {
            return out;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workload {workload_id} did not reach a terminal state within {max_wait:?}; \
             last observed row: {:?}",
            out.snapshot.rows.first(),
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// The guest-observed online vCPU count carried by a terminal row's
/// `WorkloadCrashedImmediately { exit_code }` — the guest reports its own
/// `available_parallelism` as the process exit code, which rides the
/// beacon `EXIT` back to the operator. Panics with the actual row if the
/// terminal shape is anything else (a boot-deadline failure with no exit
/// code means the guest never reached Running).
fn guest_reported_cpu_count(out: &WorkloadDescribeOutput) -> i32 {
    let row = out.snapshot.rows.first().expect("one allocation row for the deployed VM job");
    match row.reason {
        Some(TransitionReason::WorkloadCrashedImmediately { exit_code: Some(code), .. }) => code,
        ref other => panic!(
            "the guest must report its online vCPU count as an exit code (proving it reached \
             Running and executed on real vCPUs), got state={:?} reason={other:?}",
            row.state,
        ),
    }
}

// ---------------------------------------------------------------------
// S-VM-69 — declared 2000 cpu_milli yields exactly two guest vCPUs.
// ---------------------------------------------------------------------

/// S-VM-69 — A VM declaring `cpu_milli = 2000` boots a guest that reports
/// exactly TWO online CPUs, observed from inside the guest (never against
/// the constructed hypervisor config). `round_up(2000 / 1000) = 2`.
#[tokio::test]
#[serial(cgroup)]
async fn declared_cpu_milli_2000_yields_two_guest_online_cpus() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-resources-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs)");
    let report_bin = build_report_cpus_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &report_bin, "reportcpus");

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-cpu-2000.toml",
        &vm_job_toml("vm-cpu-2000", "/sbin/reportcpus", &fixture.kernel_path, &rootfs, 2000),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the 2000-cpu_milli [vm] spec");

    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    assert_eq!(
        guest_reported_cpu_count(&out),
        2,
        "cpu_milli=2000 must derive 2 vCPUs (round_up(2000/1000)) and the guest must observe \
         exactly 2 online CPUs FROM INSIDE (available_parallelism), never VmConfig.vcpus",
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-70 — a sub-core request still yields ONE usable vCPU + Running.
// ---------------------------------------------------------------------

/// S-VM-70 — A VM declaring `cpu_milli = 250` (sub-core) still boots a
/// usable guest with exactly ONE online CPU (floor at 1 — no fractional
/// vCPU) and the allocation reaches Running. The guest reporting
/// `exit_code = 1` proves BOTH: it observed exactly one online CPU, AND it
/// reached Running and executed (a 0-vCPU VM would be unbootable and would
/// fail the boot deadline with no exit code — S-VM-03's shape).
#[tokio::test]
#[serial(cgroup)]
async fn sub_core_cpu_milli_250_yields_one_guest_online_cpu_and_reaches_running() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-resources-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs)");
    let report_bin = build_report_cpus_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &report_bin, "reportcpus");

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-cpu-250.toml",
        &vm_job_toml("vm-cpu-250", "/sbin/reportcpus", &fixture.kernel_path, &rootfs, 250),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the 250-cpu_milli [vm] spec");

    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    assert_eq!(
        guest_reported_cpu_count(&out),
        1,
        "cpu_milli=250 must floor at 1 vCPU (max(1, round_up(250/1000))) and the guest must \
         observe exactly 1 online CPU FROM INSIDE; the exit code arriving at all proves the \
         allocation reached Running and executed (a 0-vCPU VM would be unbootable)",
    );

    handle.shutdown().await.expect("clean shutdown");
}
