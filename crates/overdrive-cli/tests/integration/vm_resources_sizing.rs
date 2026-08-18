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

/// Cross-builds a tiny static-musl binary that reads its OWN total RAM and
/// `exit`s with it in 16-MiB units — the guest-observed memory reporting
/// channel for S-VM-71. Uses `sysinfo(2)` (statically linked from musl via
/// `+crt-static`): the guest kernel's own view of total usable RAM,
/// requiring neither `/proc` (the minimal rootfs mounts none — the same
/// constraint that drove `build_report_cpus_binary` to
/// `available_parallelism` rather than `/proc/meminfo`) nor coreutils. The
/// exit-code channel is 8-bit, so bytes are reported in units of 16 MiB
/// (2 GiB = 128 units), which the host band-checks. Same direct-`rustc`
/// cross-build shape as `build_report_cpus_binary`.
fn build_report_mem_binary(tmp: &Path) -> PathBuf {
    let src = tmp.join("report_mem.rs");
    std::fs::write(
        &src,
        // `struct sysinfo` (x86_64): a #[repr(C)] mirror of the kernel's
        // layout. Only `totalram` (offset 32) and `mem_unit` (offset 104)
        // are read; the trailing pad keeps the buffer >= the kernel's
        // 112-byte write. The guest exits with total RAM in 16-MiB units,
        // capped to a u8 exit code (2 GiB => 128).
        "#[repr(C)]\nstruct SysInfo {\n    uptime: i64,\n    loads: [u64; 3],\n    \
         totalram: u64,\n    freeram: u64,\n    sharedram: u64,\n    bufferram: u64,\n    \
         totalswap: u64,\n    freeswap: u64,\n    procs: u16,\n    pad: u16,\n    \
         totalhigh: u64,\n    freehigh: u64,\n    mem_unit: u32,\n    _tail: [u8; 16],\n}\n\
         extern \"C\" {\n    fn sysinfo(info: *mut SysInfo) -> i32;\n}\n\
         fn main() {\n    \
         let mut si: SysInfo = unsafe { std::mem::zeroed() };\n    \
         let rc = unsafe { sysinfo(&mut si as *mut SysInfo) };\n    \
         if rc != 0 {\n        std::process::exit(255);\n    }\n    \
         let unit = if si.mem_unit == 0 { 1u64 } else { si.mem_unit as u64 };\n    \
         let total_bytes = si.totalram.saturating_mul(unit);\n    \
         let units_16mib = (total_bytes / (16u64 * 1024 * 1024)).min(255);\n    \
         std::process::exit(units_16mib as i32);\n}\n",
    )
    .expect("write the report-mem guest source");
    let out = tmp.join("report_mem");
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
        .expect("spawn rustc for the report-mem guest binary");
    assert!(status.success(), "rustc must build the tiny static-musl report-mem binary");
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

/// A `[job]`+`[vm]`+`[resources]` TOML with a caller-chosen `cpu_milli`,
/// memory fixed at 256 MiB. The sibling vCPU cases (S-VM-69/70) vary only
/// `cpu_milli`; S-VM-71 varies `memory_bytes` via [`vm_job_toml_sized`].
fn vm_job_toml(id: &str, command: &str, kernel: &Path, rootfs: &Path, cpu_milli: u32) -> String {
    vm_job_toml_sized(id, command, kernel, rootfs, cpu_milli, 268_435_456)
}

/// A `[job]`+`[vm]`+`[resources]` TOML with a caller-chosen `cpu_milli`
/// AND `memory_bytes`. The `[resources]` block is the single source of
/// truth for VM size (feature-delta §263 — `[vm]` carries no cpu/memory
/// field).
fn vm_job_toml_sized(
    id: &str,
    command: &str,
    kernel: &Path,
    rootfs: &Path,
    cpu_milli: u32,
    memory_bytes: u64,
) -> String {
    format!(
        "[job]\nid = \"{id}\"\n\n[vm]\ncommand = \"{command}\"\nargs = []\n\
         kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = {cpu_milli}\n\
         memory_bytes = {memory_bytes}\n",
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

/// The guest-observed total RAM (in 16-MiB units) carried by a terminal
/// row's `WorkloadCrashedImmediately { exit_code }` — the guest reports its
/// own `sysinfo(2)` `totalram` as the process exit code, which rides the
/// beacon `EXIT` back to the operator. Panics with the actual row if the
/// terminal shape is anything else (a boot-deadline failure with no exit
/// code means the guest never reached Running on real guest memory).
fn guest_reported_mem_units_16mib(out: &WorkloadDescribeOutput) -> i32 {
    let row = out.snapshot.rows.first().expect("one allocation row for the deployed VM job");
    match row.reason {
        Some(TransitionReason::WorkloadCrashedImmediately { exit_code: Some(code), .. }) => code,
        ref other => panic!(
            "the guest must report its observed total RAM (in 16-MiB units) as an exit code \
             (proving it booted and executed on real guest memory), got state={:?} reason={other:?}",
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

// ---------------------------------------------------------------------
// S-VM-71 — declared 2 GiB is what the guest gets, AND workload describe
// reports the same declared figure (single private memory backing; the
// shared=on/volume half was withdrawn with Slice 04 — volumes cut
// 2026-08-18, deferred to GH #97/#43/#22).
// ---------------------------------------------------------------------

/// S-VM-71 — A VM declaring `memory_bytes = 2147483648` (2 GiB) boots a
/// guest that observes approximately 2 GiB of RAM FROM INSIDE (its own
/// `sysinfo(2)` `totalram`, never the generated hypervisor config — which
/// would only prove we wrote what we wrote), AND `overdrive workload
/// describe` reports the SAME declared figure for the VM allocation.
///
/// # Guest-observed memory channel (criterion 1)
///
/// The injected guest command reads its own total RAM via `sysinfo(2)`
/// (statically linked from musl; no `/proc`, which the minimal rootfs does
/// not mount, and no coreutils) and exits WITH that figure in units of
/// 16 MiB — the 8-bit exit-code channel cannot carry raw bytes, and 2 GiB
/// is exactly 128 such units. The host asserts a tolerance band
/// `[MIN_UNITS, DECLARED_UNITS]`: "approximately 2 GiB, a little below the
/// declared figure (the guest kernel reserves some), never above" — a
/// sensible band, NOT exact equality (kernel-reserved memory makes exact
/// equality wrong). A guest that never booted produces a boot-deadline
/// failure with no exit code, so a terminal row carrying the units IS the
/// proof the guest reached Running on real guest memory.
///
/// # Describe parity (criterion 2)
///
/// `overdrive workload describe` renders the declared `memory_bytes` for
/// the VM allocation, sourced from the same per-row `resources.memory_bytes`
/// the describe handler populates from the workload intent for EVERY kind
/// (`handlers` builds it from `job.resources` / `svc.resources` /
/// `sched.job.resources`). A VM is a Job-kind workload, so it reports the
/// declared figure through the SAME render surface a process allocation
/// does — never a parallel VM-only renderer.
///
/// # Port-to-port litmus
///
/// Delete the `Memory:` line in `render::workload_describe` and the
/// describe-parity half stays RED (the declared figure never reaches the
/// operator surface), while the guest-observed half still passes — the two
/// halves fail independently.
#[tokio::test]
#[serial(cgroup)]
async fn declared_memory_2gib_is_guest_observed_and_reported_by_describe() {
    // 2 GiB — the declared operator figure, and the RAM the guest observes.
    const DECLARED_MEMORY_BYTES: u64 = 2_147_483_648;
    // 16-MiB report granularity: the 8-bit exit-code channel cannot carry
    // raw bytes; 2 GiB / 16 MiB = 128 fits a u8 with room to spare.
    const UNIT_BYTES: u64 = 16 * 1024 * 1024;
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    const DECLARED_UNITS: i32 = (DECLARED_MEMORY_BYTES / UNIT_BYTES) as i32; // 128
    // Allow up to 128 MiB (8 units) of guest-kernel reservation below the
    // declared figure; never above (the guest cannot see more RAM than CH
    // gave it). "Approximately 2 GiB", not exact.
    const MIN_UNITS: i32 = DECLARED_UNITS - 8; // 120

    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-resources-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs)");
    let report_bin = build_report_mem_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &report_bin, "reportmem");

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-mem-2gib.toml",
        &vm_job_toml_sized(
            "vm-mem-2gib",
            "/sbin/reportmem",
            &fixture.kernel_path,
            &rootfs,
            1000,
            DECLARED_MEMORY_BYTES,
        ),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the 2 GiB [vm] spec");

    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(90)).await;

    // Criterion 1 — the guest observes approximately 2 GiB FROM INSIDE
    // (sysinfo totalram), never the generated config.
    let observed_units = guest_reported_mem_units_16mib(&out);
    assert!(
        (MIN_UNITS..=DECLARED_UNITS).contains(&observed_units),
        "declared memory_bytes=2 GiB must be observed as ~2 GiB inside the guest \
         (sysinfo totalram, never the generated hypervisor config): expected \
         {MIN_UNITS}..={DECLARED_UNITS} units of 16 MiB (a little below 2 GiB for kernel \
         reservation, never above), got {observed_units} ({} MiB)",
        observed_units * 16,
    );

    // Criterion 2 — workload describe reports the SAME declared figure,
    // through the same render surface a process allocation uses.
    let rendered = overdrive_cli::render::workload_describe(&out);
    let memory_line_reports_declared = rendered.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("Memory:") && trimmed.contains(&DECLARED_MEMORY_BYTES.to_string())
    });
    assert!(
        memory_line_reports_declared,
        "overdrive workload describe must report the declared memory_bytes={DECLARED_MEMORY_BYTES} \
         for the VM allocation (the same resources.memory_bytes a process allocation carries, \
         rendered the same way); got:\n{rendered}",
    );
    // The render reads the typed snapshot; pin that it carries the declared
    // figure verbatim (the source of the rendered line above).
    assert_eq!(
        out.snapshot
            .rows
            .first()
            .expect("one allocation row for the deployed VM job")
            .resources
            .memory_bytes,
        DECLARED_MEMORY_BYTES,
        "the describe snapshot row must carry the declared memory_bytes verbatim",
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-72 — guest-observed vCPU count AND memory size both match the
// declared figures for ONE VM allocation shape, on the single (private)
// memory backing. The `shared=on` / volume backing half was withdrawn
// with Slice 04 (volumes cut 2026-08-18 → GH #97/#43/#22), so this
// reduces to a single-backing sizing-parity case that asserts BOTH
// dimensions together — the "AND" AC-17 requires, which neither
// S-VM-69/70 (cpu only, 256 MiB) nor S-VM-71 (memory only, cpu_milli=1000)
// makes on one shape. The typed resize-rejection half of S-VM-72 lives
// in `overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs`
// (port-to-port at `Driver::resize`, which `overdrive-cli` cannot reach —
// it has no `overdrive-worker` dependency).
// ---------------------------------------------------------------------

/// S-VM-72 (sizing-parity half) — a VM declaring `cpu_milli = 2000` AND
/// `memory_bytes = 2 GiB` (2147483648) boots a guest that observes BOTH
/// exactly two online vCPUs AND ~2 GiB of RAM, FROM INSIDE the guest
/// (`available_parallelism` / `sysinfo(2)` totalram — never the generated
/// hypervisor config), on the single private memory backing.
///
/// The 8-bit beacon `EXIT` channel carries one figure per boot, so the
/// SAME fixed spec size is deployed twice against one `overdrive serve`:
/// once reporting online vCPUs (expect 2 = `round_up(2000/1000)`), once
/// reporting total RAM (expect ~2 GiB, a little below for kernel
/// reservation, never above). Both deploys carry the identical
/// `[resources]` (`cpu_milli=2000`, `memory_bytes=2` GiB), so the assertion
/// is "for one VM shape, both sizing dimensions land correctly".
///
/// # Port-to-port litmus
///
/// Revert either the `cpu_milli -> vcpus` wiring or the `MemoryPlan`
/// guest-RAM sizing in `VmDriver::start` and exactly one dimension goes
/// RED while the other stays GREEN — the two halves fail independently.
#[tokio::test]
#[serial(cgroup)]
async fn declared_cpu_and_memory_match_guest_observed_on_single_private_backing() {
    const CPU_MILLI: u32 = 2000; // round_up(2000 / 1000) = 2 vCPUs
    const DECLARED_MEMORY_BYTES: u64 = 2_147_483_648; // 2 GiB
    const UNIT_BYTES: u64 = 16 * 1024 * 1024;
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    const DECLARED_UNITS: i32 = (DECLARED_MEMORY_BYTES / UNIT_BYTES) as i32; // 128
    // ~2 GiB, a little below the declared figure (guest-kernel reservation),
    // never above — same tolerance band as S-VM-71.
    const MIN_UNITS: i32 = DECLARED_UNITS - 8; // 120

    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    // Two distinct tempdirs so each dimension's staged rootfs (each
    // `stage_rootfs_with_extra_binary` writes `rootfs.ext4` in its tmp)
    // is independent and neither deploy races the other's staging.
    let cpu_tmp = tempfile::Builder::new()
        .prefix("vm-resources-cpu-")
        .tempdir_in(shared_staging_root())
        .expect("cpu tempdir on the XFS-backed reflink-capable staging root (never tmpfs)");
    let mem_tmp = tempfile::Builder::new()
        .prefix("vm-resources-mem-")
        .tempdir_in(shared_staging_root())
        .expect("mem tempdir on the XFS-backed reflink-capable staging root (never tmpfs)");

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());

    // Dimension 1 — the derived vCPU count, guest-observed.
    let cpu_bin = build_report_cpus_binary(cpu_tmp.path());
    let cpu_rootfs =
        stage_rootfs_with_extra_binary(cpu_tmp.path(), &fixture, &cpu_bin, "reportcpus");
    let cpu_spec = write_toml(
        server_tmp.path(),
        "vm-parity-cpu.toml",
        &vm_job_toml_sized(
            "vm-parity-cpu",
            "/sbin/reportcpus",
            &fixture.kernel_path,
            &cpu_rootfs,
            CPU_MILLI,
            DECLARED_MEMORY_BYTES,
        ),
    );
    let cpu_submit = deploy(DeployArgs { spec: cpu_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the cpu-report [vm] spec (cpu_milli=2000, memory=2 GiB)");
    let cpu_out = poll_until_terminal(&cfg, &cpu_submit.workload_id, Duration::from_secs(90)).await;
    assert_eq!(
        guest_reported_cpu_count(&cpu_out),
        2,
        "cpu_milli=2000 (with memory=2 GiB) must derive 2 vCPUs and the guest must observe \
         exactly 2 online CPUs FROM INSIDE, on the single private backing",
    );

    // Dimension 2 — the declared memory, guest-observed, SAME spec size.
    let mem_bin = build_report_mem_binary(mem_tmp.path());
    let mem_rootfs =
        stage_rootfs_with_extra_binary(mem_tmp.path(), &fixture, &mem_bin, "reportmem");
    let mem_spec = write_toml(
        server_tmp.path(),
        "vm-parity-mem.toml",
        &vm_job_toml_sized(
            "vm-parity-mem",
            "/sbin/reportmem",
            &fixture.kernel_path,
            &mem_rootfs,
            CPU_MILLI,
            DECLARED_MEMORY_BYTES,
        ),
    );
    let mem_submit = deploy(DeployArgs { spec: mem_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the mem-report [vm] spec (cpu_milli=2000, memory=2 GiB)");
    let mem_out = poll_until_terminal(&cfg, &mem_submit.workload_id, Duration::from_secs(90)).await;
    let observed_units = guest_reported_mem_units_16mib(&mem_out);
    assert!(
        (MIN_UNITS..=DECLARED_UNITS).contains(&observed_units),
        "declared memory_bytes=2 GiB (with cpu_milli=2000) must be observed as ~2 GiB inside the \
         guest on the single private backing: expected {MIN_UNITS}..={DECLARED_UNITS} units of \
         16 MiB, got {observed_units} ({} MiB)",
        observed_units * 16,
    );

    handle.shutdown().await.expect("clean shutdown");
}
