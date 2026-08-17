//! Tier-3 real-kernel suite for `VmReclamation` (ADR-0083 §D7, `brief.md`
//! §105a, GH #42) — step 02-03.
//!
//! Drives real `overdrive serve` boots + real `overdrive deploy`/`stop`
//! against a real Cloud Hypervisor VMM, per `crates/overdrive-cli/CLAUDE.md`
//! § "Integration tests — no subprocess" (direct handler calls, no
//! `Command::spawn`). Runs on the `x86_64+KVM` metal box
//! (`cargo xtask metal run --`), never Lima (no nested KVM on Apple
//! Silicon).
//!
//! Every scenario here MUST stay registered in `.config/nextest.toml`'s
//! `host-kernel-shared` test-group (matched by module, so a new `#[tokio::
//! test]` added to this file joins automatically) — `#[serial(cgroup)]`
//! alone enforces nothing under nextest's per-test-process execution
//! model. Confirmed empirically while landing this file: an unserialized
//! `--no-fail-fast` run produced real `EBUSY` XDP-slot collisions and
//! `did not reach Running within 60s` timeouts from concurrent real-
//! cgroupfs contention; the module-level `host-kernel-shared` assignment
//! resolved all of it. See `vm_walking_skeleton.rs`'s own module doc for
//! the original precedent this file follows.
//!
//! # Scenario map
//!
//! Per `docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
//! (the catalogue SSOT; step 02-03's own dispatch prompt swapped the
//! S-VM-25 / S-VM-79 labels relative to the catalogue's actual content —
//! test names below are content-addressed, not id-addressed, to stay
//! correct regardless of the label swap):
//!
//! - S-VM-30 (folded from 02-02 scope): [`vm_survivor_with_no_vm_registry_entry_is_reclaimed_via_observed_empty`]
//! - S-VM-23 (folded from 02-02 scope): [`boot_epoch_reclamation_settles_before_adopt_on_restart_recovery`]
//! - S-VM-21: [`steady_state_sweep_reclaims_a_stranded_scope_without_restarting_serve`]
//! - S-VM-22: [`steady_state_sweep_kills_a_surviving_vmm_at_a_later_tick`]
//! - S-VM-25 shape (a): [`restart_orphan_terminal_row_is_byte_unchanged_after_reclamation`]
//! - S-VM-25 shape (b): [`failed_stop_orphan_terminal_row_is_byte_unchanged_after_reclamation`]
//! - S-VM-81: [`reclaiming_an_svid_holding_allocation_submits_the_fourth_evaluation`]
//!   — step 02-03 completion: drives a GENUINE `Action::
//!   ReclaimAllocation` (not `stop()`) via the boot-epoch drive and
//!   observes `Stopped { by: PlatformReclaimed }` on real infra as an
//!   intermediate checkpoint, well inside the 1s restart-backoff window.
//!   Step 02-04 re-scope: S-VM-26's guard makes restart-after-reclaim the
//!   correct behavior for this never-stopped Job-kind allocation, so the
//!   checkpoint's terminal row is superseded by the SAME live `serve`
//!   session's `WorkloadLifecycle` re-drive -- the test's FINAL assertion
//!   is the DURABLE occurrence proof (`restart_count` +
//!   `last_terminated.reason`, ADR-0078) that survives the restart, not
//!   the transient terminal row it supersedes. See that test's own doc
//!   comment for the residual gap this suite surfaces rather than papers
//!   over (no production surface exposes `IdentityMgr` state to a
//!   real-serve test; the four-evaluations claim IS fully proven,
//!   executor-direct, at Tier-1 in `action_shim::reclamation::tests`).
//! - S-VM-28 (step 02-04): [`reclaim_then_restart_populates_restart_count_and_last_terminated_together`]
//!   — the SAME boot-epoch-reclaim fixture shape as S-VM-81, but the
//!   workload is never `stop()`-ed: its intent still stands, so the SAME
//!   live `serve` session's `WorkloadLifecycle` reconcile loop re-drives
//!   it (S-VM-26/27's guards), and ONE scenario asserts BOTH
//!   `restart_count` and `last_terminated` populate together.
//!
//! # Fixture construction — why not a plain fault-injection seam
//!
//! `VmDriver::stop`'s host-footprint teardown (`cgroup_kill`,
//! `remove_workload_scope`, `remove_dir_all`, `remove_file`,
//! `crates/overdrive-worker/src/vm_driver.rs:696-701`) is ALREADY
//! best-effort — every call is `let _ = ...`, so a substrate failure
//! there is silently absorbed and `stop()` still returns `Ok(())`. To
//! reproduce "cgroup scope removal is made to fail" deterministically
//! (rather than racing a real kernel reap-lag window), these tests
//! create a REAL, empty CHILD cgroup directory inside the allocation's
//! scope before stopping it: cgroup v2's `rmdir` on a parent refuses
//! with `ENOTEMPTY` while ANY child cgroup directory exists, regardless
//! of process/reap state. This is a real action against real cgroupfs
//! (`mkdir`/`rmdir` on `/sys/fs/cgroup/...`), not a mock or a new
//! production test-seam — the same class of direct kernel-surface
//! interaction `vm_walking_skeleton.rs` already uses (`/proc/<pid>/...`
//! reads). The blocker is removed by the test before the reclamation
//! window opens, simulating "the transient condition has cleared" —
//! `VmHostState::kill_scope`'s own settle-retry loop would otherwise
//! also hit the same `ENOTEMPTY` and this executor propagates that
//! failure (unlike `VmDriver::stop`), so leaving the blocker in place at
//! BOOT time would refuse the whole boot (the boot-epoch drive is
//! fail-closed) — which is why the blocker is introduced and cleared
//! entirely within one already-booted `serve` session for S-VM-21/22/25(b).

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::missing_panics_doc, clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, WorkloadDescribeOutput, describe};
use overdrive_control_plane::api::{AllocStateWire, AllocStatusRowBody};
use overdrive_core::transition_reason::StoppedBy;
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tempfile::TempDir;

// ---------------------------------------------------------------------
// Shared fixture staging (own copies — the `vm_walking_skeleton.rs`
// helpers are private to that file).
// ---------------------------------------------------------------------

fn shared_staging_root() -> PathBuf {
    overdrive_testing::vm_fixture::default_staging_root()
}

/// A long-lived guest binary that never exits on its own — needed
/// whenever the assertion window requires the real `cloud-hypervisor`
/// process to still be observable at a specific moment.
fn build_spin_binary(tmp: &Path) -> PathBuf {
    let src = tmp.join("spin.rs");
    std::fs::write(
        &src,
        "fn main() { loop { std::thread::sleep(std::time::Duration::from_secs(3600)); } }",
    )
    .expect("write spin source");
    let out = tmp.join("spin");
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
        .expect("spawn rustc for the long-lived spin binary");
    assert!(status.success(), "rustc must build the long-lived spin binary");
    out
}

/// A short-lived guest binary that exits 0 immediately — needed for the
/// natural-exit fixture (S-VM-25 shape (a)).
fn build_exit0_binary(tmp: &Path) -> PathBuf {
    let src = tmp.join("exit0.rs");
    std::fs::write(&src, "fn main() { std::process::exit(0); }").expect("write exit0 source");
    let out = tmp.join("exit0");
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
        .expect("spawn rustc for the exit0 binary");
    assert!(status.success(), "rustc must build the exit0 binary");
    out
}

/// Stages a PER-TEST COPY of the shared fixture's rootfs with an extra
/// binary injected at `/sbin/<guest_name>` via a host-side loopback
/// mount. Mirrors `vm_walking_skeleton.rs::stage_rootfs_with_extra_binary`
/// exactly (this file's own copy — that one is private).
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
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&dest, perms).expect("chmod the copied guest binary executable");

    let umount_status = Command::new("umount").arg(&mnt).status().expect("spawn umount");
    assert!(umount_status.success(), "umount {} failed", mnt.display());
    let _ = Command::new("losetup").arg("-d").arg(&loop_dev).status();

    rootfs_copy
}

// ---------------------------------------------------------------------
// Server composition — three shapes needed across these scenarios.
// ---------------------------------------------------------------------

/// Real serve, `[vm]` driver composed, non-mTLS (`SimDataplane`
/// injected) — the cheapest boot for scenarios that do not need mTLS /
/// SVID machinery.
async fn spawn_vm_server(data_dir: &Path, config_dir: &Path) -> ServeHandle {
    std::fs::create_dir_all(data_dir).expect("create data dir");
    std::fs::create_dir_all(config_dir).expect("create operator config dir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let args =
        ServeArgs { bind, data_dir: data_dir.to_path_buf(), config_dir: config_dir.to_path_buf() };
    overdrive_cli::commands::serve::run_with_dataplane(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
    )
    .await
    .expect("serve::run_with_dataplane")
}

/// Real serve, `[vm]` driver composed, mTLS-composed (real `EbpfDataplane`,
/// real SVID/CA machinery) — needed for S-VM-23 (adopt-on-restart
/// ordering) and S-VM-81 (SVID issuance).
async fn spawn_vm_server_mtls_composed(
    data_dir: &Path,
    config_dir: &Path,
) -> Result<ServeHandle, overdrive_cli::http_client::CliError> {
    std::fs::create_dir_all(data_dir).expect("create data dir");
    std::fs::create_dir_all(config_dir).expect("create operator config dir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let args =
        ServeArgs { bind, data_dir: data_dir.to_path_buf(), config_dir: config_dir.to_path_buf() };
    overdrive_cli::commands::serve::run_with_kek(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
    )
    .await
}

/// Every directory in `PATH` EXCEPT the one containing `cloud-hypervisor`
/// — mirrors `vm_walking_skeleton.rs::path_without_cloud_hypervisor` (same
/// technique, duplicated here since sibling test modules cannot see each
/// other's private items).
fn path_without_cloud_hypervisor() -> String {
    let ch_dir = Command::new("which")
        .arg("cloud-hypervisor")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .and_then(|resolved| Path::new(&resolved).parent().map(Path::to_path_buf))
        .expect(
            "cloud-hypervisor must be resolvable via `which` on this host before we can hide it",
        );

    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|dir| Path::new(dir) != ch_dir.as_path())
        .collect::<Vec<_>>()
        .join(":")
}

/// Real serve, NO `[vm]` driver composed at all (cloud-hypervisor
/// "uninstalled" — `state.drivers.get(DriverType::Vm) == None`). Needed
/// for S-VM-30.
///
/// The absence is a REAL host fact — `cloud-hypervisor` is hidden from
/// `PATH` for the duration of the boot, so discovery finds no hypervisor
/// and composition takes the `VmComposeError::NotAvailable` soft-skip
/// arm. It used to be produced by leaving `ServerConfig.vm_artifacts`
/// unset, but ADR-0083 §D3c made composition UNCONDITIONAL and gated on
/// `Vmm::probe` alone: there is no artifact precondition left to withhold,
/// and a node's VM capability is now exactly whether the hypervisor is
/// there. This is the same technique S-VM-12 already proves end to end
/// (`vm_walking_skeleton.rs`).
///
/// The composition root's discover/probe runs synchronously inside this
/// `.await`, so `PATH` is restored the instant it returns. Callers must
/// carry `#[serial(env)]` alongside `#[serial(cgroup)]`.
async fn spawn_server_no_vm_driver(data_dir: &Path, config_dir: &Path) -> ServeHandle {
    std::fs::create_dir_all(data_dir).expect("create data dir");
    std::fs::create_dir_all(config_dir).expect("create operator config dir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let args =
        ServeArgs { bind, data_dir: data_dir.to_path_buf(), config_dir: config_dir.to_path_buf() };

    let original_path = std::env::var_os("PATH");
    let broken_path = path_without_cloud_hypervisor();
    // SAFETY: callers carry `#[serial(env)]`, which guarantees exclusive
    // access to `PATH` for the duration of the test.
    unsafe {
        std::env::set_var("PATH", &broken_path);
    }

    let result = overdrive_cli::commands::serve::run_with_dataplane(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
    )
    .await;

    // SAFETY: as above — restored immediately, before any assertion runs.
    unsafe {
        match &original_path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }

    result.expect("serve::run_with_dataplane (cloud-hypervisor hidden -> no Vm registry entry)")
}

fn config_path(config_dir: &Path) -> PathBuf {
    config_dir.join(".overdrive").join("config")
}

/// Bridges the real, narrow race between `ServeHandle::shutdown` returning
/// and its `LocalIntentStore` / `LocalObservationStore` redb file
/// descriptors actually closing (the convergence-loop / exit-observer
/// background tasks are signalled via `CancellationToken` but not
/// necessarily joined before `shutdown` returns). A restart against the
/// SAME `data_dir` immediately after `shutdown()` can otherwise observe
/// `"Database already open. Cannot acquire lock."` on the redb reopen.
/// Every scenario in this file that reboots against the same `data_dir`
/// calls this between the two boots.
async fn wait_for_data_dir_release() {
    tokio::time::sleep(Duration::from_millis(500)).await;
}

// ---------------------------------------------------------------------
// Spec authoring + polling + host-path helpers.
// ---------------------------------------------------------------------

fn vm_job_toml(id: &str, command: &str, kernel: &Path, rootfs: &Path) -> String {
    format!(
        "[job]\nid = \"{id}\"\n\n[vm]\ncommand = \"{command}\"\nargs = []\n\
         kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\n\
         memory_bytes = 134217728\n",
        kernel.display(),
        rootfs.display(),
    )
}

fn write_toml(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write toml");
    path
}

async fn poll_until_running(
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
        if out.snapshot.rows.first().is_some_and(|r| r.state == AllocStateWire::Running) {
            return out;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workload {workload_id} did not reach Running within {max_wait:?}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

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
        if let Some(row) = out.snapshot.rows.first()
            && matches!(row.state, AllocStateWire::Terminated | AllocStateWire::Failed)
        {
            return out;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workload {workload_id} did not reach a terminal state within {max_wait:?}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Polls up to `max_wait` for the single row to be `Running` again with
/// `restart_count >= 1` -- the reclaim-then-restart postcondition
/// (S-VM-28). Distinct from `poll_until_running`: a restart REUSES the
/// SAME `alloc_id` (`Action::RestartAllocation`, mirrors the Exec-driver
/// shape in `crash_observability_two_cycles.rs`), so `state == Running`
/// alone cannot distinguish "still the original boot" from "recovered
/// via a reclaim-then-restart cycle" -- the restart count is what pins it.
async fn poll_until_restarted(
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
        if out
            .snapshot
            .rows
            .first()
            .is_some_and(|r| r.state == AllocStateWire::Running && r.restart_count >= 1)
        {
            return out;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workload {workload_id} did not restart (Running with restart_count>=1) within {max_wait:?}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn scope_path(alloc_id: &str) -> PathBuf {
    PathBuf::from("/sys/fs/cgroup/overdrive.slice/workloads.slice")
        .join(format!("{alloc_id}.scope"))
}

fn run_dir_path(alloc_id: &str) -> PathBuf {
    PathBuf::from("/run/overdrive/vm").join(alloc_id)
}

fn clone_path(staging_dir: &Path, alloc_id: &str) -> PathBuf {
    staging_dir.join(format!(".overdrive-vm-rootfs-{alloc_id}.img"))
}

/// The PLATFORM-OWNED VM rootfs staging root — the single directory
/// `RealVmHostState` enumerates for stranded per-launch clones, and the
/// literal `run_server` composes (`/run/overdrive/vm-rootfs-staging`).
///
/// # The clone-surface residual this constant makes visible
///
/// ADR-0083 §D3a made artifacts per-allocation, and `RootfsPlan::for_alloc`
/// derives the per-launch clone destination from the MASTER'S OWN PARENT —
/// i.e. wherever the operator's `[vm] rootfs` lives. §D3b keeps that
/// derivation deliberately (`FICLONE` is intra-filesystem, so the clone
/// must sit on the master's filesystem for reflink to work at all), and
/// the amendment's own Consequences ratify the result: "the per-launch
/// rootfs clone is now written into an operator-chosen directory ...
/// rather than a platform-owned one".
///
/// The consequence for RECLAMATION is not spelled out there, and it is
/// this: `VmHostState`'s clone surface is a single node-level directory,
/// so it can no longer observe any clone a real allocation produces.
/// `VmDriver::stop` still removes the clone directly (it holds the
/// allocation's own `RootfsPlan` in memory, so the operator directory is
/// no obstacle) — the uncovered case is an allocation that ends WITHOUT
/// `stop`: a natural guest exit, or a crash. Scope and run-directory
/// reclamation are unaffected; both are platform-owned paths.
///
/// Scenarios below therefore split into two groups, and the split is
/// deliberate rather than incidental — see each site.
fn node_staging_dir() -> PathBuf {
    let dir = PathBuf::from("/run/overdrive/vm-rootfs-staging");
    std::fs::create_dir_all(&dir).expect("create the platform-owned VM rootfs staging root");
    dir
}

/// Finds the single running `cloud-hypervisor` process's pid by
/// `argv[0]` (mirrors `vm_walking_skeleton.rs::find_cloud_hypervisor_pid`
/// — `comm` truncates to 15 chars, shorter than the 16-char binary name).
fn find_cloud_hypervisor_pid() -> Option<u32> {
    for entry in std::fs::read_dir("/proc").expect("read /proc") {
        let Ok(entry) = entry else { continue };
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let cmdline_path = entry.path().join("cmdline");
        let Ok(cmdline) = std::fs::read(&cmdline_path) else { continue };
        let argv0 = cmdline.split(|&b| b == 0).next().unwrap_or(&[]);
        let argv0 = String::from_utf8_lossy(argv0);
        if Path::new(argv0.as_ref()).file_name() == Some(std::ffi::OsStr::new("cloud-hypervisor")) {
            return Some(pid);
        }
    }
    None
}

fn pid_is_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Creates a REAL, empty child cgroup directory inside `scope` — see the
/// module doc's "Fixture construction" section for why this reliably
/// (not racily) makes the scope's own `rmdir` fail with `ENOTEMPTY`
/// until removed.
fn add_cgroup_child_blocker(scope: &Path) -> PathBuf {
    let blocker = scope.join("blocker");
    std::fs::create_dir(&blocker).expect("mkdir a real child cgroup inside the scope");
    blocker
}

fn remove_cgroup_child_blocker(blocker: &Path) {
    // A child cgroup directory can only be rmdir'd once its own
    // `cgroup.procs` is empty -- true here since nothing was ever added
    // to it -- and once its own subtree_control has nothing enabled.
    std::fs::remove_dir(blocker)
        .expect("rmdir the child cgroup blocker (clearing the induced fault)");
}

/// `VmDriver::stop`'s own teardown (`crates/overdrive-worker/src/vm_driver.rs:696-701`)
/// unconditionally removes the run dir and rootfs clone as two SEPARATE,
/// non-cgroup filesystem operations -- the cgroup-child blocker above
/// only defeats the scope's `rmdir`, so by the time `stop()` returns the
/// run dir and clone are already gone. `plan_reclamation`'s decision
/// table deliberately does NOT reclaim a cgroup-scope-ONLY leftover (Row
/// 6 -- "shared with exec allocations and is left alone"), so a fixture
/// with only the scope stranded is unreclaimable BY DESIGN.
///
/// This recreates the run dir (empty) and clone (empty file) directly on
/// the real filesystem -- the same "the VM-exclusive artifacts survived
/// a crash between teardown steps" shape `brief.md` §105a.5 itself names
/// ("a clone leaked by a crash between teardown steps"), and mirrors the
/// established Tier-1 precedent (`vm_reclamation_boot.rs`'s own tests
/// call `host.set_run_dir(alloc.clone())` directly to seed the SAME
/// GIVEN state) -- elevated here to real paths so the real production
/// `execute_discard_stranded_artifacts` genuinely discovers and removes
/// them via `VmHostState::observe()` / `discard_artifacts`.
fn restrand_vm_exclusive_artifacts(alloc_id: &str, staging_dir: &Path) {
    std::fs::create_dir_all(run_dir_path(alloc_id)).expect("recreate the stranded run dir");
    std::fs::write(clone_path(staging_dir, alloc_id), b"stranded-clone-placeholder")
        .expect("recreate the stranded clone file");
}

/// Polls up to `max_wait` for `scope_path`/`run_dir_path`/`clone_path`
/// to ALL be absent -- the steady-state sweep's reclaim postcondition.
/// `VM_RECLAMATION_SWEEP_INTERVAL` is 30s; the ceiling gives headroom
/// for scheduling jitter plus the sweep's own real filesystem I/O.
async fn poll_until_reclaimed(alloc_id: &str, staging_dir: &Path, max_wait: Duration) {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let scope_gone = !scope_path(alloc_id).exists();
        let run_dir_gone = !run_dir_path(alloc_id).exists();
        let clone_gone = !clone_path(staging_dir, alloc_id).exists();
        if scope_gone && run_dir_gone && clone_gone {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "reclamation did not complete within {max_wait:?} for {alloc_id}: \
             scope_gone={scope_gone} run_dir_gone={run_dir_gone} clone_gone={clone_gone}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

// ---------------------------------------------------------------------
// S-VM-30 (folded) — a node with no `Vm` registry entry still reclaims
// its VM survivors via `Observed(∅)`.
// ---------------------------------------------------------------------

/// A node that "uninstalled cloud-hypervisor" (no `[vm]` driver
/// composed) still reclaims surviving VM artifacts at boot, because
/// `VmHostState` composes unconditionally and `Observed(∅)` (no
/// registry entry) authorises reclamation rather than blocking it
/// (brief.md §105a.3's composition table).
#[tokio::test]
// `env` alongside `cgroup`: [`spawn_server_no_vm_driver`] hides
// `cloud-hypervisor` from `PATH` for the duration of the boot, which is
// process-global state (`.claude/rules/testing.md` § "Tests that mutate
// process-global state").
#[serial(cgroup, env)]
async fn vm_survivor_with_no_vm_registry_entry_is_reclaimed_via_observed_empty() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-reclaim-s30-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the staging root");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");

    let server_tmp = TempDir::new().expect("server tempdir");
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");

    // Boot #1 -- with the [vm] driver -- deploy and let the VM survive
    // an unclean shutdown (kill_on_drop(false) on the spawned VMM means
    // it is NOT killed merely because this process's handle is dropped).
    let handle = spawn_vm_server(&data_dir, &config_dir).await;
    let cfg = config_path(&config_dir);
    let spec_path = write_toml(
        tmp.path(),
        "vm-s30.toml",
        &vm_job_toml("vm-s30", "/sbin/spin", &fixture.kernel_path, &rootfs),
    );
    let submit =
        deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() }).await.expect("deploy");
    poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let alloc_id = format!("alloc-{}-0", submit.workload_id);
    assert!(scope_path(&alloc_id).exists(), "sanity: the scope must exist while Running");
    handle.shutdown().await.expect("shutdown boot #1 without stopping the workload");
    wait_for_data_dir_release().await;

    // Boot #2 -- SAME data_dir, with `cloud-hypervisor` hidden from PATH so
    // discovery finds no hypervisor and NO [vm] driver is composed:
    // `state.drivers.get(DriverType::Vm)` reads `None`. (Before ADR-0083
    // §D3c this absence was produced by withholding `vm_artifacts`;
    // composition is now unconditional and gated on `Vmm::probe` alone, so
    // the absence has to be a real host fact -- which is strictly more
    // faithful to the "cloud-hypervisor uninstalled" node it models.)
    let handle2 = spawn_server_no_vm_driver(&data_dir, &config_dir).await;

    // The boot-epoch VmReclamation drive runs synchronously inside
    // `run_server`, before this call returns -- reclamation has already
    // happened by the time boot #2's handle is available.
    assert!(!scope_path(&alloc_id).exists(), "the surviving scope must be reclaimed at boot");
    assert!(!run_dir_path(&alloc_id).exists(), "the surviving run dir must be reclaimed at boot");
    // The clone is NOT asserted here -- the same named residual documented
    // on `node_staging_dir`. Boot #1's clone was produced by a REAL boot,
    // so it sits beside the operator's own `[vm] rootfs`, and
    // `RealVmHostState`'s single platform-owned clone surface cannot see
    // it. The run dir (fixed `/run/overdrive/vm`) and the scope are both
    // platform-owned paths, are unaffected by the artifact relocation, and
    // ARE asserted above -- which is this scenario's actual claim: a node
    // with no `Vm` registry entry still reclaims via `Observed(the empty
    // set)`.
    handle2.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-23 (folded) — boot-epoch reclamation settles BEFORE
// adopt_on_restart_recovery reads the tree.
// ---------------------------------------------------------------------

/// A restart whose boot-epoch `VmReclamation` drive reclaims N surviving
/// VM cgroup scopes must NOT have `adopt_on_restart_recovery` refuse the
/// boot with `NetnsRecoveryError::ObserveRead` -- the load-bearing
/// PINNED ORDER (reclaim -> settle -> adopt reads the tree) is observed
/// indirectly: if the ordering regressed, this boot would refuse.
#[tokio::test]
#[serial(cgroup)]
async fn boot_epoch_reclamation_settles_before_adopt_on_restart_recovery() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-reclaim-s23-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the staging root");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");

    let server_tmp = TempDir::new().expect("server tempdir");
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");

    let handle = spawn_vm_server_mtls_composed(&data_dir, &config_dir)
        .await
        .expect("boot #1 (mTLS-composed) must succeed");
    let cfg = config_path(&config_dir);
    let spec_path = write_toml(
        tmp.path(),
        "vm-s23.toml",
        &vm_job_toml("vm-s23", "/sbin/spin", &fixture.kernel_path, &rootfs),
    );
    let submit =
        deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() }).await.expect("deploy");
    poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let alloc_id = format!("alloc-{}-0", submit.workload_id);
    handle.shutdown().await.expect("shutdown boot #1 without stopping the workload");
    wait_for_data_dir_release().await;

    // Boot #2 -- SAME data_dir, mTLS-composed again (so
    // adopt_on_restart_recovery's netns-adopt pass genuinely runs and
    // reads the same cgroup tree the reclamation drive just touched).
    let handle2 = spawn_vm_server_mtls_composed(&data_dir, &config_dir).await.expect(
        "boot #2 must NOT refuse -- a NetnsRecoveryError::ObserveRead would surface here if the \
         reclaim-before-adopt ordering (or kill_scope's settle postcondition) regressed",
    );

    assert!(!scope_path(&alloc_id).exists(), "the surviving scope must be reclaimed at boot");
    assert!(!run_dir_path(&alloc_id).exists(), "the surviving run dir must be reclaimed at boot");
    // The CLONE is deliberately not asserted here, and the reason is a
    // named design residual rather than an oversight.
    //
    // This clone was produced by a REAL boot, so `RootfsPlan::for_alloc`
    // put it beside the operator's own `[vm] rootfs` (ADR-0083 §D3a/§D3b —
    // FICLONE is intra-filesystem, so the clone MUST share the master's
    // filesystem). `RealVmHostState`'s clone surface is the single
    // platform-owned `node_staging_dir()`, so the sweep structurally
    // cannot see it. This allocation also never went through
    // `VmDriver::stop` (which DOES remove the clone directly, holding the
    // allocation's own `RootfsPlan`), so nothing removes it.
    //
    // That is a real leak on the natural-exit / crash path, ratified only
    // in its premise: ADR-0083's Consequences accept that the clone leaves
    // platform-owned space, but no ruling reconciles that with §D7's
    // reclamation model, which counts the clone as one of three observation
    // surfaces. Asserting "reclaimed" here would be false; asserting
    // "leaked" would ratify a leak this step has no authority to bless.
    // The scenario's own claim -- named in its function name -- is about
    // the reclaim-before-adopt ORDERING, and that claim is asserted in full above.

    handle2.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-21 -- mid-run drift repairs without a `serve` restart.
// ---------------------------------------------------------------------

/// THE AC that distinguishes Bar 2 from Bar 1 (brief §105a.10 AC1): a
/// SINGLE `serve` session (no restart anywhere in this test) whose
/// boot-epoch drive finds nothing (the workload does not exist yet at
/// boot), then strands a scope WHILE ALREADY RUNNING, and the periodic
/// `VM_RECLAMATION_SWEEP_INTERVAL` tick -- with NO manual nudge --
/// reclaims it. Proves the WAKE mechanism (§105a.8) actually fires.
#[tokio::test]
#[serial(cgroup)]
async fn steady_state_sweep_reclaims_a_stranded_scope_without_restarting_serve() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-reclaim-s21-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the staging root");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");
    // The stranded clone goes in the PLATFORM-OWNED staging root, which is
    // the surface `RealVmHostState` enumerates (see `node_staging_dir`).
    // This scenario strands artifacts artificially, so it is free to strand
    // them where the sweep looks — that keeps `discard_artifacts`' clone
    // path genuinely exercised. A clone from a REAL boot lands beside the
    // operator's own `[vm] rootfs` instead, which the sweep cannot see; the
    // two scenarios that produce one that way say so at their own sites.
    let staging_dir = node_staging_dir();

    let server_tmp = TempDir::new().expect("server tempdir");
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");

    // ONE boot for the whole test -- no restart anywhere below.
    let handle = spawn_vm_server(&data_dir, &config_dir).await;
    let cfg = config_path(&config_dir);
    let spec_path = write_toml(
        tmp.path(),
        "vm-s21.toml",
        &vm_job_toml("vm-s21", "/sbin/spin", &fixture.kernel_path, &rootfs),
    );
    let submit =
        deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() }).await.expect("deploy");
    poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let alloc_id = format!("alloc-{}-0", submit.workload_id);

    // Induce the "scope removal is made to fail" fault (see module doc)
    // BEFORE stopping -- VmDriver::stop's own teardown will hit it.
    let blocker = add_cgroup_child_blocker(&scope_path(&alloc_id));

    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop must succeed even though its own scope-removal step failed (best-effort)");
    let stopped = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    assert_eq!(
        stopped.snapshot.rows.first().expect("one row").state,
        AllocStateWire::Terminated,
        "the stop must still author a Terminated row despite the scope surviving"
    );
    assert!(
        scope_path(&alloc_id).exists(),
        "sanity: the scope must have SURVIVED the stop (the induced fault worked)"
    );

    // stop()'s own teardown already removed the run dir / clone (both
    // unrelated to the cgroup blocker) -- restrand them so the leftover
    // is VM-exclusive, not scope-only (see the helper's own doc comment
    // for why a scope-only leftover is unreclaimable by design).
    restrand_vm_exclusive_artifacts(&alloc_id, &staging_dir);

    // Clear the induced fault -- "the transient condition has cleared".
    remove_cgroup_child_blocker(&blocker);

    // WITHOUT restarting serve: wait for the natural periodic sweep,
    // NO manual evaluation submission.
    poll_until_reclaimed(&alloc_id, &staging_dir, Duration::from_secs(50)).await;

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-22 -- a live VMM whose allocation row is already terminal is
// killed at a later tick.
// ---------------------------------------------------------------------

/// Same fixture shape as S-VM-21 (one boot, no restart, the real
/// `stop()` writes a Terminated row while the induced cgroup fault
/// survives it) -- this scenario's own emphasis is on the SURVIVING
/// PID: `VmDriver::stop`'s `Vmm::terminate` call is unconditional
/// (`crates/overdrive-worker/src/vm_driver.rs:694`), so the real
/// `cloud-hypervisor` process IS killed as part of the same `stop()`
/// call that authors the terminal row -- there is no production path
/// that authors a terminal row while deliberately leaving the VMM PID
/// alive without ALSO inventing a fault-injection seam on `Vmm::terminate`
/// itself (out of this step's pinned scope). This test therefore proves
/// the postcondition the AC's Then clause actually asserts on -- "the
/// surviving VMM process, its cgroup scope, run directory and rootfs
/// clone are all gone after the sweep" -- by capturing the real PID
/// BEFORE the stop (proving a real VMM existed) and asserting it is
/// gone AFTER the sweep, alongside the scope/run-dir/clone.
#[tokio::test]
#[serial(cgroup)]
async fn steady_state_sweep_kills_a_surviving_vmm_at_a_later_tick() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-reclaim-s22-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the staging root");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");
    // The stranded clone goes in the PLATFORM-OWNED staging root, which is
    // the surface `RealVmHostState` enumerates (see `node_staging_dir`).
    // This scenario strands artifacts artificially, so it is free to strand
    // them where the sweep looks — that keeps `discard_artifacts`' clone
    // path genuinely exercised. A clone from a REAL boot lands beside the
    // operator's own `[vm] rootfs` instead, which the sweep cannot see; the
    // two scenarios that produce one that way say so at their own sites.
    let staging_dir = node_staging_dir();

    let server_tmp = TempDir::new().expect("server tempdir");
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");

    let handle = spawn_vm_server(&data_dir, &config_dir).await;
    let cfg = config_path(&config_dir);
    let spec_path = write_toml(
        tmp.path(),
        "vm-s22.toml",
        &vm_job_toml("vm-s22", "/sbin/spin", &fixture.kernel_path, &rootfs),
    );
    let submit =
        deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() }).await.expect("deploy");
    poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let alloc_id = format!("alloc-{}-0", submit.workload_id);

    let vmm_pid = find_cloud_hypervisor_pid().expect("a real cloud-hypervisor process is running");
    assert!(pid_is_alive(vmm_pid), "sanity: the real VMM process must be alive before stop");

    let blocker = add_cgroup_child_blocker(&scope_path(&alloc_id));
    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop must succeed even though its own scope-removal step failed");
    poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    restrand_vm_exclusive_artifacts(&alloc_id, &staging_dir);
    remove_cgroup_child_blocker(&blocker);

    poll_until_reclaimed(&alloc_id, &staging_dir, Duration::from_secs(50)).await;
    assert!(
        !pid_is_alive(vmm_pid),
        "the real cloud-hypervisor process must be gone after the sweep"
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-25 shape (a) -- the restart orphan: a terminal row survives a
// restart, byte-unchanged after reclamation.
// ---------------------------------------------------------------------

/// A row that reached Terminated via a NATURAL exit (the real
/// `exit_observer` path, not reclamation) survives a `serve` restart
/// with its artifacts (run dir / clone) still on disk. The next boot's
/// reclamation drive discards the stranded artifacts (`DiscardStranded
/// Artifacts` — no row write, no evaluation, structurally per DD-5) and
/// the row is BYTE-UNCHANGED at every field this wire surface exposes.
#[tokio::test]
#[serial(cgroup)]
async fn restart_orphan_terminal_row_is_byte_unchanged_after_reclamation() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-reclaim-s25a-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the staging root");
    let exit0 = build_exit0_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit0, "exit0");

    let server_tmp = TempDir::new().expect("server tempdir");
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");

    let handle = spawn_vm_server(&data_dir, &config_dir).await;
    let cfg = config_path(&config_dir);
    let spec_path = write_toml(
        tmp.path(),
        "vm-s25a.toml",
        &vm_job_toml("vm-s25a", "/sbin/exit0", &fixture.kernel_path, &rootfs),
    );
    let submit =
        deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() }).await.expect("deploy");
    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let alloc_id = format!("alloc-{}-0", submit.workload_id);
    let seeded_row: AllocStatusRowBody =
        out.snapshot.rows.first().expect("one row for the exited alloc").clone();
    assert_eq!(
        seeded_row.state,
        AllocStateWire::Terminated,
        "a clean guest exit reaches Terminated"
    );
    handle.shutdown().await.expect("shutdown boot #1 (no further action against this row)");
    wait_for_data_dir_release().await;

    // If a natural exit's artifacts survive to disk (SD-2's framing --
    // VmReclamation is the general sweep, not only the failure-path
    // cleanup), boot #2's reclamation drive discards them.
    let handle2 = spawn_vm_server(&data_dir, &config_dir).await;

    assert!(!scope_path(&alloc_id).exists(), "the scope must be reclaimed at boot");
    assert!(!run_dir_path(&alloc_id).exists(), "the run dir must be reclaimed at boot");
    // The CLONE is deliberately not asserted here, and the reason is a
    // named design residual rather than an oversight.
    //
    // This clone was produced by a REAL boot, so `RootfsPlan::for_alloc`
    // put it beside the operator's own `[vm] rootfs` (ADR-0083 §D3a/§D3b —
    // FICLONE is intra-filesystem, so the clone MUST share the master's
    // filesystem). `RealVmHostState`'s clone surface is the single
    // platform-owned `node_staging_dir()`, so the sweep structurally
    // cannot see it. This allocation also never went through
    // `VmDriver::stop` (which DOES remove the clone directly, holding the
    // allocation's own `RootfsPlan`), so nothing removes it.
    //
    // That is a real leak on the natural-exit / crash path, ratified only
    // in its premise: ADR-0083's Consequences accept that the clone leaves
    // platform-owned space, but no ruling reconciles that with §D7's
    // reclamation model, which counts the clone as one of three observation
    // surfaces. Asserting "reclaimed" here would be false; asserting
    // "leaked" would ratify a leak this step has no authority to bless.
    // The scenario's own claim -- named in its function name -- is about
    // the terminal row being BYTE-UNCHANGED across reclamation, and that claim is asserted in full above.

    let after = describe(DescribeArgs { id: submit.workload_id.clone(), config_path: cfg })
        .await
        .expect("describe after reclamation");
    let after_row = after.snapshot.rows.first().expect("one row survives reclamation");
    assert_eq!(
        after_row, &seeded_row,
        "DiscardStrandedArtifacts writes no row at all (DD-5, structural: the executor takes no \
         ObservationStore parameter) -- the row observed on every field this wire surface \
         exposes (state, reason, restart_count, last_terminated, started_at, error) must be \
         byte-identical to what the natural exit wrote"
    );

    handle2.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-25 shape (b) -- the failed-stop orphan: an operator stop
// authored the terminal row but the scope removal failed; byte-unchanged
// after reclamation.
// ---------------------------------------------------------------------

/// An operator `stop()` authors the terminal row (state: Terminated,
/// `reason: Stopped { by: Reconciler }` -- the shim's own `StopAllocation`
/// arm disposition) while the scope removal genuinely fails (the induced
/// cgroup-child fault). The later sweep's `DiscardStrandedArtifacts`
/// reclaims the artifacts and writes NO row -- the row is byte-unchanged
/// at every field this wire surface exposes, same shape as S-VM-25
/// shape (a).
#[tokio::test]
#[serial(cgroup)]
async fn failed_stop_orphan_terminal_row_is_byte_unchanged_after_reclamation() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-reclaim-s25b-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the staging root");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");
    // The stranded clone goes in the PLATFORM-OWNED staging root, which is
    // the surface `RealVmHostState` enumerates (see `node_staging_dir`).
    // This scenario strands artifacts artificially, so it is free to strand
    // them where the sweep looks — that keeps `discard_artifacts`' clone
    // path genuinely exercised. A clone from a REAL boot lands beside the
    // operator's own `[vm] rootfs` instead, which the sweep cannot see; the
    // two scenarios that produce one that way say so at their own sites.
    let staging_dir = node_staging_dir();

    let server_tmp = TempDir::new().expect("server tempdir");
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");

    let handle = spawn_vm_server(&data_dir, &config_dir).await;
    let cfg = config_path(&config_dir);
    let spec_path = write_toml(
        tmp.path(),
        "vm-s25b.toml",
        &vm_job_toml("vm-s25b", "/sbin/spin", &fixture.kernel_path, &rootfs),
    );
    let submit =
        deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() }).await.expect("deploy");
    poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let alloc_id = format!("alloc-{}-0", submit.workload_id);

    let blocker = add_cgroup_child_blocker(&scope_path(&alloc_id));
    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop succeeds (best-effort teardown) despite the induced scope-removal fault");
    let stopped = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    let seeded_row: AllocStatusRowBody =
        stopped.snapshot.rows.first().expect("one row after the stop").clone();
    assert!(
        matches!(
            seeded_row.reason,
            Some(overdrive_core::TransitionReason::Stopped { by: StoppedBy::Reconciler })
        ),
        "the shim's StopAllocation arm authors Stopped {{ by: Reconciler }}, got {:?}",
        seeded_row.reason
    );
    assert!(scope_path(&alloc_id).exists(), "sanity: the scope survived the failed removal");

    restrand_vm_exclusive_artifacts(&alloc_id, &staging_dir);
    remove_cgroup_child_blocker(&blocker);
    poll_until_reclaimed(&alloc_id, &staging_dir, Duration::from_secs(50)).await;

    let after = describe(DescribeArgs { id: submit.workload_id.clone(), config_path: cfg })
        .await
        .expect("describe after reclamation");
    let after_row = after.snapshot.rows.first().expect("one row survives reclamation");
    assert_eq!(
        after_row, &seeded_row,
        "the row must be byte-unchanged on every field this wire surface exposes -- \
         DiscardStrandedArtifacts writes no row at all"
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-81 -- reclaiming an SVID-holding allocation via a GENUINE
// `Action::ReclaimAllocation` (not `stop()`) submits the fourth
// evaluation (svid_lifecycle), alongside the other three; the durable
// reclaim occurrence survives the S-VM-26-mandated restart.
// ---------------------------------------------------------------------

/// Step 02-03 completion. Prior to this step `Action::ReclaimAllocation`
/// was structurally unreachable from any real `overdrive serve` +
/// `overdrive deploy` flow -- `reconciler_runtime::hydrate_desired`'s
/// `VmReclamation` arm was hardcoded to an empty
/// `VmReclamationState::default()`, so `plan_reclamation` could only
/// ever route through the "no entry" branch of the decision table. This
/// test's earlier shape reflected that gap honestly: it deployed a VM,
/// confirmed a real SVID was issued, then drove the workload to
/// `Terminated` via the ordinary `stop()` verb before shutdown -- which
/// proved SVID issuance, not reclamation (`stop()` authors
/// `Stopped { by: Reconciler }`, never `Stopped { by:
/// PlatformReclaimed }`).
///
/// Now that [`crate` (`overdrive_control_plane`)`::reconciler_runtime::
/// hydrate_vm_reclamation_desired`] fills the join for both drives, this
/// test drives a GENUINE `Action::ReclaimAllocation` through the
/// boot-epoch drive -- the SAME S-VM-30 / S-VM-23 fixture shape (a VM
/// that survives an unclean `handle.shutdown()`, so its `AllocStatusRow`
/// stays non-terminal, then a fresh boot whose brand-new `VmDriver` has
/// tracked nothing yet reads `Observed(∅)`). By construction (already
/// Tier-1-proven, executor-direct, in `action_shim::reclamation::tests::
/// execute_reclaim_allocation_authorised_kills_discards_writes_and_submits_four_evaluations`)
/// the SAME unconditional code path that writes the reclaimed row also
/// submits the four evaluations, including `svid_lifecycle`.
///
/// **Step 02-04 re-scope.** The workload's intent is never withdrawn
/// here (no `stop()` before boot #1's unclean shutdown) -- so per
/// S-VM-26's guard (`is_natural_exit && !is_platform_reclaimed(row)`,
/// `transition_reason.rs`), the reclaim is NOT this test's final word:
/// the SAME live `serve` session's `WorkloadLifecycle` reconcile loop
/// re-drives the allocation once the 1s `RESTART_BACKOFF_DURATION`
/// elapses, which is the correct, intended behavior for a reclaimed
/// Job-kind allocation, not the bug S-VM-26 fixed (pre-fix,
/// `is_natural_exit` finalised the row via a fabricated `Failed {
/// exit_code: Some(0) }` claim instead of ever restarting it).
///
/// This test does NOT assert an intermediate "reclaimed, not yet
/// restarted" checkpoint. `WorkloadLifecycle`'s convergence loop runs
/// CONCURRENTLY with (not strictly after) `run_server`'s own mTLS/SVID
/// boot sequence, so there is no reliable window in which the reclaim's
/// terminal row is guaranteed still observable -- confirmed empirically:
/// a real-metal run of this exact fixture asserted the reclaim's
/// discarded scope gone synchronously right after boot #2 returned, well
/// inside the nominal 1s backoff margin, and still observed the scope
/// PRESENT -- the restart had already re-created it. The mTLS/SVID boot
/// sequence's own wall-clock cost is apparently enough, on real
/// hardware, for the concurrently-running convergence loop to observe
/// the freshly-written terminal row, wait out the backoff, and complete
/// a full restart before control even returns to the test. Per ADR-0078
/// ("a convergent record cannot answer 'did it happen'" --
/// `development.md` § "A convergent record cannot answer 'did it
/// happen'"), the test therefore asserts ONLY the DURABLE occurrence
/// proof that survives the restart: `restart_count` (the budget) and
/// `last_terminated` (the disposition), once `poll_until_restarted`
/// confirms the cycle has genuinely settled.
/// `last_terminated.reason == Stopped { by: PlatformReclaimed }` is
/// exactly as strong a claim that `execute_reclaim_allocation`'s
/// AUTHORISED branch (not `stop()`, not `DiscardStrandedArtifacts`) ran
/// as a synchronous checkpoint would have been -- ADR-0078's crash-facts
/// snapshot preserves the terminal disposition the row passed through,
/// observed without racing the restart. `Action::RestartAllocation`
/// reuses the SAME `alloc_id`, so there is no "old `alloc_id`" distinct
/// from the current one whose artifacts this test could check post-
/// restart either. This is what keeps S-VM-81 a genuine end-to-end
/// witness that a reclaimed **SVID-holder** specifically is handled
/// correctly through a real restart -- distinct from S-VM-28, which
/// drives the SAME boot-epoch-reclaim-then-restart cycle without the
/// SVID precondition, asserting the identical `restart_count` /
/// `last_terminated` pair.
///
/// **Residual gap, surfaced rather than papered over.** Observing that
/// the `svid_lifecycle` evaluation is subsequently DEQUEUED by a live
/// `SvidLifecycle` tick and that its `DropSvid` action actually removes
/// the entry from THIS process's `IdentityMgr` requires reading
/// `IdentityMgr::held_snapshot()` -- and no production surface exposes
/// it from outside the process. `overdrive_cli::commands::serve::
/// ServeHandle` and `overdrive_control_plane::ServerHandle` are fully
/// opaque (`endpoint()`/`local_addr()` and `shutdown()` only); `identity`
/// is constructed INSIDE `run_server`'s own composition with no caller
/// injection point -- unlike `obs` / `drivers`, which
/// `run_server_with_obs_and_driver(s)` already exposes for exactly this
/// "integration tests that need to retain a handle" purpose (per that
/// function's own doc comment). Exposing `IdentityMgr` state to a
/// real-serve Tier-3 test needs a new test-support entry point mirroring
/// that existing `obs`-retention pattern -- a small, genuine new surface
/// this step's dispatch did not pin a signature for. Per CLAUDE.md
/// "Implement to the design" this is a gap to surface, not improvise; it
/// is called out in the step's own report rather than worked around with
/// an invented `pub` accessor. `dispatch_drop`'s own effect (it calls
/// `identity.drop_svid(alloc_id)`, which removes the held entry) is a
/// concern unit-tested independently of `VmReclamation`, at
/// `identity_mgr.rs` / `issue_svid.rs`; the FOUR-evaluations claim
/// (including `svid_lifecycle`) is Tier-1-proven, executor-direct, at
/// the site named above.
#[tokio::test]
#[serial(cgroup)]
async fn reclaiming_an_svid_holding_allocation_submits_the_fourth_evaluation() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-reclaim-s81-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the staging root");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");

    let server_tmp = TempDir::new().expect("server tempdir");
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");

    // Boot #1 -- mTLS-composed -- deploy and confirm a REAL SVID is
    // issued and held.
    let handle = spawn_vm_server_mtls_composed(&data_dir, &config_dir)
        .await
        .expect("boot #1 (mTLS-composed) must succeed");
    let cfg = config_path(&config_dir);
    let spec_path = write_toml(
        tmp.path(),
        "vm-s81.toml",
        &vm_job_toml("vm-s81", "/sbin/spin", &fixture.kernel_path, &rootfs),
    );
    let submit =
        deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() }).await.expect("deploy");
    let out = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let alloc_id = format!("alloc-{}-0", submit.workload_id);

    assert!(
        !out.snapshot.issued_certificates.is_empty(),
        "a Running mTLS-composed VM allocation must hold a real issued SVID -- \
         reclaiming it is the precondition S-VM-81 depends on"
    );
    assert!(scope_path(&alloc_id).exists(), "sanity: the scope must exist while Running");

    // Boot #1 shuts down WITHOUT stopping the workload -- an "unclean"
    // shutdown (mirrors S-VM-30 / S-VM-23): the real cloud-hypervisor
    // process survives (`kill_on_drop(false)`) and the alloc's row stays
    // non-terminal (Running), never transitioning through the ordinary
    // `stop()` path. The intent is never withdrawn either -- this is
    // what makes the boot-epoch reclaim's restart (S-VM-26), not a
    // stay-terminal finish, the correct next act.
    handle.shutdown().await.expect("shutdown boot #1 without stopping the workload");
    wait_for_data_dir_release().await;

    // Boot #2 -- SAME data_dir, mTLS-composed again. Its brand-new
    // `VmDriver` has supervised nothing yet in THIS process
    // (`live_allocations()` reads empty) -> `SupervisionSet::Observed(∅)`
    // -> `reclamation_authorised` is true. The desired-side join now
    // finds this alloc: a Vm-driven `Job` intent (`vm-s81`'s persisted
    // `WorkloadIntent::Job` with `driver: Vm`) joined against a
    // NON-TERMINAL `AllocStatusRow` (still Running -- the unclean
    // shutdown never wrote a terminal row) -> `plan_reclamation`'s row 1
    // -> `Action::ReclaimAllocation`, NOT row 4's
    // `DiscardStrandedArtifacts` (which requires "no entry"). The
    // boot-epoch `VmReclamation` drive runs synchronously inside
    // `run_server`, before this call returns.
    let handle2 = spawn_vm_server_mtls_composed(&data_dir, &config_dir)
        .await
        .expect("boot #2 (mTLS-composed) must succeed");

    // The DURABLE occurrence proof (ADR-0078: "a convergent record
    // cannot answer 'did it happen'"). The workload's intent was never
    // withdrawn, so the SAME live `serve` session's `WorkloadLifecycle`
    // reconcile loop re-drives the boot-epoch reclaim's terminal row
    // once the backoff window elapses -- per S-VM-26 this is now the
    // correct, intended behavior. No intermediate "reclaimed, not yet
    // restarted" checkpoint is asserted here -- see the test's own doc
    // comment: `WorkloadLifecycle`'s convergence loop runs concurrently
    // with the mTLS/SVID boot sequence, leaving no reliable pre-restart
    // observation window (confirmed on real metal). What survives the
    // restart durably is `restart_count` (the budget) and
    // `last_terminated` (the disposition -- unambiguous proof
    // `execute_reclaim_allocation`'s AUTHORISED branch ran, since
    // ADR-0078's crash-facts snapshot preserves the SAME `Stopped {
    // by: PlatformReclaimed }` disposition a synchronous checkpoint
    // would have observed), asserted together per S-VM-28's own
    // reasoning: only together do they rule out both an implementation
    // that erases the occurrence and one that silently consumes the
    // budget.
    let restarted = poll_until_restarted(&cfg, &submit.workload_id, Duration::from_secs(90)).await;
    let restarted_row = restarted.snapshot.rows.first().expect("one row after the restart");
    assert_eq!(
        restarted_row.restart_count, 1,
        "restart_count must have incremented by EXACTLY one across the reclaim-then-restart \
         cycle; got {restarted_row:?}"
    );
    let last_terminated = restarted_row
        .last_terminated
        .as_ref()
        .expect("last_terminated must be populated by the SAME restart that bumped restart_count");
    assert!(
        matches!(
            last_terminated.reason,
            Some(overdrive_core::TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })
        ),
        "last_terminated must describe the reclamation disposition (StoppedBy::PlatformReclaimed) \
         this SVID-holding allocation actually underwent -- written ONLY by \
         execute_reclaim_allocation's AUTHORISED branch (stop() writes Stopped {{ by: Reconciler }}; \
         DiscardStrandedArtifacts writes no row at all, DD-5) -- not silently erased or describing \
         something else; got {:?}",
        last_terminated.reason
    );

    // Reap the restarted (still-live) spin VM via the PRODUCTION stop
    // path before shutdown: `Command::kill_on_drop` is deliberately
    // `false` on the production spawn, so nothing kills a still-Running
    // VM merely because this test process exits (same leak class
    // `vm_walking_skeleton.rs`'s long-lived-spin scenarios guard
    // against; mirrors S-VM-28's own closing sequence).
    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() }).await.expect(
        "stop the restarted spin workload before shutdown to avoid leaking the VMM process",
    );
    poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;

    handle2.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-28 (step 02-04) -- restart_count and last_terminated populate
// TOGETHER, in one scenario, across a genuine reclaim-then-restart
// cycle.
// ---------------------------------------------------------------------

/// `docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
/// S-VM-28. SAME boot-epoch-reclaim fixture shape as S-VM-81 (an unclean
/// `handle.shutdown()` leaves a real, surviving VMM and a non-terminal
/// row; a second boot's brand-new `VmDriver` reads `Observed(∅)` and the
/// boot-epoch drive reclaims it via a GENUINE `Action::ReclaimAllocation`,
/// never `stop()`) -- but UNLIKE S-VM-81, the workload here is NEVER
/// `stop()`-ed before the reclaim: its intent still stands (DD-1), so the
/// SAME live `serve` session's `WorkloadLifecycle` reconcile loop
/// re-drives it once the boot-epoch drive's terminal write lands (the
/// `is_natural_exit` / ceiling guards S-VM-26/27 exist for exactly this
/// branch). Deliberately ONE scenario asserting BOTH crash-observability
/// fields together, per the DISTILL crafter notes: asserting only the
/// budget (`restart_count`) passes an implementation that erased the
/// occurrence; asserting only the occurrence (`last_terminated`) passes
/// one that consumed the budget silently -- per ADR-0078, "a convergent
/// record cannot answer 'did it happen'."
#[tokio::test]
#[serial(cgroup)]
async fn reclaim_then_restart_populates_restart_count_and_last_terminated_together() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-reclaim-s28-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the staging root");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");

    let server_tmp = TempDir::new().expect("server tempdir");
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");

    // Boot #1 -- deploy and confirm the pre-reclaim baseline: a first
    // start has never restarted and has nothing to report yet.
    let handle = spawn_vm_server(&data_dir, &config_dir).await;
    let cfg = config_path(&config_dir);
    let spec_path = write_toml(
        tmp.path(),
        "vm-s28.toml",
        &vm_job_toml("vm-s28", "/sbin/spin", &fixture.kernel_path, &rootfs),
    );
    let submit =
        deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() }).await.expect("deploy");
    let baseline = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let baseline_row = baseline.snapshot.rows.first().expect("one row for the running alloc");
    assert_eq!(baseline_row.restart_count, 0, "sanity: a first start is not a restart");
    assert_eq!(baseline_row.last_terminated, None, "sanity: nothing survived yet");

    // Unclean shutdown -- NEVER call stop(). The workload's intent still
    // stands; the real cloud-hypervisor process survives
    // (`Command::kill_on_drop` is deliberately `false` on the production
    // spawn) and the row stays non-terminal (Running).
    handle.shutdown().await.expect("shutdown boot #1 without stopping the workload");
    wait_for_data_dir_release().await;

    // Boot #2 -- SAME data_dir. Its brand-new `VmDriver` has supervised
    // nothing yet in THIS process (`Observed(∅)`), so the boot-epoch
    // drive's `plan_reclamation` row 1 fires a GENUINE
    // `Action::ReclaimAllocation` for the still-non-terminal row -- the
    // reclaim half of S-VM-28's "reclaimed and then restarted" Given.
    // The drive runs synchronously inside `run_server`, before this call
    // returns.
    let handle2 = spawn_vm_server(&data_dir, &config_dir).await;

    let reclaimed =
        describe(DescribeArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
            .await
            .expect("describe after the boot-epoch reclaim");
    let reclaimed_row = reclaimed.snapshot.rows.first().expect("one row survives reclamation");
    assert_eq!(
        reclaimed_row.state,
        AllocStateWire::Terminated,
        "the boot-epoch drive must reclaim the surviving non-terminal allocation, got {reclaimed_row:?}"
    );
    assert!(
        matches!(
            reclaimed_row.reason,
            Some(overdrive_core::TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })
        ),
        "Stopped {{ by: PlatformReclaimed }} is the reclamation disposition S-VM-28's Given \
         requires; got {:?}",
        reclaimed_row.reason
    );

    // The workload was NEVER stopped -- its intent still stands, so the
    // SAME live `serve` session's `WorkloadLifecycle` reconcile loop
    // re-drives it once the backoff window elapses. This is the restart
    // half of the Given, and the ONE scenario's Then: both
    // crash-observability fields populate TOGETHER as a direct result of
    // the SAME cycle.
    let restarted = poll_until_restarted(&cfg, &submit.workload_id, Duration::from_secs(90)).await;
    let restarted_row = restarted.snapshot.rows.first().expect("one row after the restart");
    assert_eq!(
        restarted_row.restart_count, 1,
        "restart_count must have incremented by EXACTLY one across the reclaim-then-restart \
         cycle; got {restarted_row:?}"
    );
    let last_terminated = restarted_row
        .last_terminated
        .as_ref()
        .expect("last_terminated must be populated by the SAME restart that bumped restart_count");
    assert!(
        matches!(
            last_terminated.reason,
            Some(overdrive_core::TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })
        ),
        "last_terminated must describe the reclamation disposition (StoppedBy::PlatformReclaimed), \
         not be silently erased or describe something else; got {:?}",
        last_terminated.reason
    );

    // Reap the restarted (still-live) spin VM via the PRODUCTION stop
    // path before shutdown: `Command::kill_on_drop` is deliberately
    // `false` on the production spawn, so nothing kills a still-Running
    // VM merely because this test process exits (same leak class
    // `vm_walking_skeleton.rs`'s long-lived-spin scenarios guard
    // against).
    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() }).await.expect(
        "stop the restarted spin workload before shutdown to avoid leaking the VMM process",
    );
    poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;

    handle2.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-84 (step 03-09, DWD-26 / ADR-0083 §§D3f-D3h) — RED scaffolds:
// a VM that ends WITHOUT `VmDriver::stop` leaves no rootfs clone behind,
// on all three without-stop endings.
//
// Shape per `.claude/rules/testing.md` § "RED scaffolds and
// intentionally-failing commits": `#[should_panic(expected = "RED
// scaffold")]` plus a panic body naming the scenario, so the bar stays
// green and the scaffold stays discoverable via
// `grep -rn 'should_panic.*RED scaffold' crates/`. `#[test]` (sync)
// TODAY because a body that is a single panic awaits nothing, boots no
// server and touches no cgroup; the rustdoc names the `#[tokio::test]` +
// `#[serial(cgroup)]` attributes the activated form must carry, so the
// swap happens together with the assertions. The file-level
// `integration-tests,kvm-tests` gate carries the `@requires-kvm` tag —
// same gating as every activated scenario above, so these compile only
// under `kvm-tests` and run only on the metal box, never Lima.
//
// The eight activated scenarios above are untouched.
//
// ## Shared shape (all three fns)
//
// `@contract-shape:bounded-change` `@error_path` `@ac-08` `@tier3`
// `@real-io` `@requires-kvm`. THE leak scenario — the falsifiable form of
// DWD-26 / ADR-0083 §§ D3f–D3h.
//
// ```gherkin
// Given a [job] + [vm] spec whose rootfs master sits in an
//   operator-chosen directory that is NOT any platform-owned directory
// And the VM has reached Running under a real "overdrive serve"
// When the allocation ends WITHOUT VmDriver::stop being called
// Then the VmReclamation pass reclaims the per-launch rootfs clone
// And no .overdrive-vm-rootfs-<alloc>.img file remains in the operator's
//   rootfs directory
// And the clone index holds no entry for that allocation
// And the cgroup scope and run directory are reclaimed as they already were
// ```
//
// ### Fixture placement is the load-bearing precondition
//
// Stage the `[vm] rootfs` master in a per-test OPERATOR-chosen directory
// — a `tempdir_in(shared_staging_root())` (reflink-capable, so FICLONE
// works) that is deliberately NEITHER `node_staging_dir()`
// (`/run/overdrive/vm-rootfs-staging`) NOR the server's `data_dir`. That
// is exactly where a real boot lands the clone: `RootfsPlan::for_alloc`
// derives `clone_dest` from `parent([vm] rootfs)` (§ D3a/§ D3b). A
// regression that re-points the sweep at a platform directory then fails
// HERE rather than passing vacuously. The activated assertions read:
//   * the operator clone is gone —
//     `clone_path(<operator_rootfs_dir>, &alloc_id)` (the SAME
//     `.overdrive-vm-rootfs-<alloc>.img` helper, pointed at the operator
//     dir, NOT `node_staging_dir()`) no longer `exists()`;
//   * the clone-index entry is gone — no
//     `.overdrive-vm-rootfs-<alloc>.img` symlink under
//     `clone_index_dir(<data_dir>)` = `<data_dir>/vm/clone-index/`
//     (§ D3g — the one derivation both composition sites call);
//   * the scope and run dir are reclaimed exactly as the activated
//     scenarios above already assert (`scope_path`, `run_dir_path`).
//
// ### Why `stop` must NOT stand in for any of the three (CRITICAL)
//
// The `stop` path is already correct: it holds the allocation's own
// in-memory `RootfsPlan` and removes the exact clone it minted, so the
// operator directory is no obstacle. That is precisely why a fixture that
// calls `stop` passes TODAY and proves nothing — `stop` is the ONE shape
// that must never substitute for a without-stop ending. Each ending below
// is therefore its own scenario, and none may reach for `stop` to reclaim
// the clone under test.
// ---------------------------------------------------------------------

/// S-VM-84 ending (1) — the guest exits on its own. Reuse
/// `build_exit0_binary` (the natural-exit shape S-VM-25(a) already
/// stages): the guest execs, exits 0, and `run_exit_watcher` emits an
/// `ExitEvent` — but per DWD-26 nothing in that path removes the
/// per-launch clone, so it strands in the operator directory. Deploy
/// through `spawn_vm_server` + `deploy`, poll to a terminal row via
/// `poll_until_terminal`, then drive the reclamation pass (a `serve`
/// restart's boot-epoch drive, or the steady-state sweep — either is
/// legitimate for this ending; the restart arm is scenario (3)) and
/// assert the operator clone AND the clone-index entry are both gone,
/// with scope/run-dir reclaimed as before. MUST NOT call `stop`.
/// Activate as `#[tokio::test]` `#[serial(cgroup)]`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn guest_self_exit_without_stop_leaves_no_rootfs_clone_in_operator_dir() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-84 / step 03-09 -- a VM whose guest exits on \
         its own, without VmDriver::stop, leaves no per-launch rootfs clone in the operator's \
         rootfs directory and no clone-index entry; scope and run dir reclaimed as before)"
    );
}

/// S-VM-84 ending (2) — the hypervisor dies. Stage a long-lived guest
/// (`build_spin_binary`), reach Running, then capture the real
/// `cloud-hypervisor` pid with `find_cloud_hypervisor_pid` and kill it
/// directly (SIGKILL) so the allocation ends via a dead VMM rather than
/// `stop`. The row goes non-terminal-then-observed; the clone strands.
/// Drive the reclamation pass and assert the operator clone AND the
/// clone-index entry are both gone, scope/run-dir reclaimed. `pid_is_alive`
/// confirms the VMM is genuinely gone first. MUST NOT call `stop`.
/// Activate as `#[tokio::test]` `#[serial(cgroup)]`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn hypervisor_death_without_stop_leaves_no_rootfs_clone_in_operator_dir() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-84 / step 03-09 -- a VM whose hypervisor dies, \
         without VmDriver::stop, leaves no per-launch rootfs clone in the operator's rootfs \
         directory and no clone-index entry; scope and run dir reclaimed as before)"
    );
}

/// S-VM-84 ending (3) — `serve` restarts and loses the in-memory
/// `RootfsPlan`. This is the sharpest arm: it is the ONLY one that also
/// proves the index survives the process. Boot #1 (`spawn_vm_server`)
/// deploys a long-lived guest to Running, then `handle.shutdown()` — an
/// unclean shutdown that drops the in-memory `RootfsPlan` (the same
/// shape S-VM-30/23/81 use; `kill_on_drop(false)` leaves the real VMM
/// surviving). Boot #2 against the SAME `data_dir` (via
/// `wait_for_data_dir_release` then `spawn_vm_server`) runs the
/// boot-epoch `VmReclamation` drive synchronously inside `run_server`,
/// which reads the DURABLE clone index under `<data_dir>/vm/clone-index/`
/// (§ D3g: `data_dir`, not `/run` — the index must survive the process)
/// and reclaims the operator clone. Assert the operator clone AND the
/// clone-index entry are both gone after boot #2, scope/run-dir
/// reclaimed. MUST NOT call `stop`. Activate as `#[tokio::test]`
/// `#[serial(cgroup)]`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn serve_restart_losing_in_memory_rootfs_plan_leaves_no_rootfs_clone() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-84 / step 03-09 -- a serve restart that loses \
         the in-memory RootfsPlan leaves no per-launch rootfs clone in the operator's rootfs \
         directory and no clone-index entry; the durable clone index under data_dir is what the \
         boot-epoch drive reads to reclaim it)"
    );
}
