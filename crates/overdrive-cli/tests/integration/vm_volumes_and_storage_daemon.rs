//! Volumes + virtiofs storage-daemon Tier-3 suite for
//! `microvm-driver-cloud-hypervisor` (GH #42, Slice 04).
//!
//! Per `docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
//! § Slice 04. Step 05-01 lands the FIRST scenario in this file:
//!
//!   * **S-VM-57** (`@guardrail`, `@ac-14`) — a VM job declaring NO volume
//!     behaves EXACTLY as before volumes existed: it boots, runs its guest
//!     command, and reaches the SAME terminal state (Terminated) and exit
//!     code (0) as Slice 01's walking skeleton (S-VM-01). This is the
//!     Slice-04 regression guard: the `[[vm.volume]]` operator surface and
//!     the derived-`shared=on` machinery (`MemoryBacking::for_volumes`,
//!     step 05-01) do NOT touch the boot path, so a volume-free VM's
//!     end-to-end behaviour is unchanged. `--memory shared=on` is NOT
//!     derived (a volume-free VM derives `MemoryBacking::Private`, whose
//!     `--memory` suffix is empty) and no storage daemon is started
//!     (virtiofsd supervision does not exist until step 05-02).
//!
//! Driven through `overdrive_cli::commands::{serve,deploy,workload}` direct
//! handler calls (per `crates/overdrive-cli/CLAUDE.md` § "Integration tests
//! — no subprocess") against a REAL Cloud Hypervisor VMM, run under
//! `cargo xtask metal run --` as root on a real `x86_64+KVM` box (Lima on
//! Apple Silicon cannot provide nested KVM). System constraint 1: no test
//! here installs, binds, programs, or supplies anything `run_server` does
//! not supply itself — `DriverRegistry` composition happens inside
//! `overdrive serve`'s own boot sequence.
//!
//! `#[serial(cgroup)]` — every test here boots a REAL `overdrive serve`
//! against the machine-global `/sys/fs/cgroup/overdrive.slice` tree; the
//! same serialization discipline `vm_walking_skeleton.rs` documents in
//! full applies here (two concurrent boots race the same
//! `subtree_control` write and tear the delegation).
//!
//! The remaining Slice-04 scenarios (S-VM-55/56/58/59/60/61/64/65/66/68)
//! land here in later Slice-04 steps, per `distill/wave-decisions.md`.

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
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tempfile::TempDir;

// ---------------------------------------------------------------------
// Fixture staging — real kernel/rootfs + a per-test injected guest
// command binary. Mirrors `vm_walking_skeleton.rs`'s helper set (sibling
// Tier-3 test modules cannot see each other's private items).
// ---------------------------------------------------------------------

/// The shared staging root every Tier-3 VM test file provisions against
/// (per `vm_fixture`'s own AC5 concurrency contract — safe under
/// concurrent nextest processes).
fn shared_staging_root() -> PathBuf {
    overdrive_testing::vm_fixture::default_staging_root()
}

/// A server-side tempdir (holding `data/` + `conf/`) on the reflink-capable
/// staging root, NOT the system tmpdir. Each per-launch rootfs clone is
/// FICLONE'd into `clone_staging_dir(data_dir)`, and FICLONE is
/// intra-filesystem — with `data_dir` on tmpfs and the master on the xfs
/// staging root, the clone would fail `EXDEV`.
fn server_tmp_on_staging_root() -> TempDir {
    tempfile::Builder::new()
        .prefix("vm-vol-serve-")
        .tempdir_in(shared_staging_root())
        .expect("server tempdir on the reflink-capable staging root")
}

/// Cross-builds a tiny static-musl binary that does nothing but
/// `std::process::exit(exit_code)`, via a direct `rustc` invocation. The
/// same `x86_64-unknown-linux-musl` cross-build `vm_walking_skeleton.rs`
/// uses (the only target this fixture's kernel staging supports).
fn build_exit_code_binary(tmp: &Path, exit_code: u8) -> PathBuf {
    let src = tmp.join(format!("exit{exit_code}.rs"));
    std::fs::write(&src, format!("fn main() {{ std::process::exit({exit_code}); }}"))
        .expect("write tiny exit-code source");
    let out = tmp.join(format!("exit{exit_code}"));
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
        .expect("spawn rustc for the tiny static exit-code binary");
    assert!(status.success(), "rustc must build the tiny static-musl exit-code binary");
    out
}

/// Stages a PER-TEST COPY of the shared fixture's rootfs image with an
/// additional static binary injected at `/sbin/<guest_name>`, via a
/// loopback mount on the HOST (before the guest ever boots) — the shared
/// fixture's own artifact is never mutated. Runs as root (this whole
/// suite runs under `cargo xtask metal run --`).
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
    // Best-effort loop-device detach — a leaked loop device does not
    // affect correctness of THIS test's assertions, only host hygiene.
    let _ = Command::new("losetup").arg("-d").arg(&loop_dev).status();

    rootfs_copy
}

// ---------------------------------------------------------------------
// Server composition
// ---------------------------------------------------------------------

/// Spawns a real in-process `overdrive serve` through the UNGATED
/// `run_with_dataplane` entrypoint with `SimDataplane` — S-VM-57 is a
/// functional-correctness regression guard and does not need the real
/// `EbpfDataplane` / mTLS composition.
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

// ---------------------------------------------------------------------
// Spec authoring + polling
// ---------------------------------------------------------------------

/// A `[job]`+`[vm]`+`[resources]` TOML with NO `[[vm.volume]]` — the
/// exact shape Slice 01 booted, unchanged.
fn vm_job_toml_no_volume(id: &str, command: &str, kernel: &Path, rootfs: &Path) -> String {
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

/// Polls `workload describe` every 500ms until the workload's first
/// allocation row reaches a terminal state, returning the final snapshot.
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

// ---------------------------------------------------------------------
// S-VM-57 — a VM job declaring no volume behaves exactly as before
// volumes existed (Slice 01 regression guard).
// ---------------------------------------------------------------------

/// S-VM-57 — A VM job with no `[[vm.volume]]` declared boots, runs its
/// guest command, and reaches the SAME terminal state (Terminated) and
/// exit code (0) as before the Slice-04 volume surface existed. The
/// derived-`shared=on` machinery (step 05-01) does not touch the boot
/// path, so a volume-free VM's end-to-end behaviour is byte-identical to
/// S-VM-01.
#[tokio::test]
#[serial(cgroup)]
async fn vm_job_with_no_volume_behaves_exactly_as_before_volumes_existed() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-vol-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let exit0 = build_exit_code_binary(tmp.path(), 0);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit0, "exit0");

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-no-volume.toml",
        &vm_job_toml_no_volume("vm-no-volume", "/sbin/exit0", &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the volume-free [vm] spec");

    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let row = out.snapshot.rows.first().expect("one allocation row for a freshly-deployed job");
    assert_eq!(
        row.state,
        AllocStateWire::Terminated,
        "a volume-free guest that exits 0 must reach the SAME terminal state as Slice 01 \
         (Terminated, classify()'s CleanExit branch), got {:?} (reason={:?})",
        row.state,
        row.reason,
    );

    handle.shutdown().await.expect("clean shutdown");
}
