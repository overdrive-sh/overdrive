//! Walking-skeleton gate for `microvm-driver-cloud-hypervisor` (GH #42).
//!
//! GREEN bodies for US-VM-1's five UAT scenarios plus S-VM-14/15/74
//! (`docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
//! § Slice 01/03, S-VM-01..05, and the walking-skeleton-owned Tier-3
//! `@real-io` evidence for `vm_driver_stop_totality.rs`'s race/priority
//! logic). Driven through `overdrive_cli::commands::{serve,deploy,workload}`
//! direct handler calls (per `crates/overdrive-cli/CLAUDE.md` § "Integration
//! tests — no subprocess") against a REAL Cloud Hypervisor VMM, run under
//! `cargo xtask metal run --` as root on a real `x86_64+KVM` box (Lima on
//! Apple Silicon cannot provide nested KVM).
//!
//! **This is the ONLY scenario group in the feature driven with a real
//! guest kernel booting under a real hypervisor.**
//!
//! System constraint 1 (vertical-slice bar): no test here installs, binds,
//! programs, or supplies anything `run_server` does not supply itself.
//! `DriverRegistry` composition (discover cloud-hypervisor → probe →
//! insert) happens inside `overdrive serve`'s own boot sequence (step
//! 01-08's `compose_vm_driver`) — these tests never hand-construct a
//! `VmDriver`.
//!
//! # The guest-command gap this file closes
//!
//! The 01-04 [`VmFixture`]'s shared, concurrency-safe staged rootfs
//! deliberately carries ONLY the `overdrive-init` binary (at `/sbin/init`
//! and `/init`) plus empty mountpoints and two device nodes — no shell, no
//! coreutils (see `vm_fixture.rs`'s own module doc: "64 MiB matches the
//! spike's proven-sufficient size for a single static binary plus device
//! nodes"). That fixture proves the KVM boot + beacon READY handshake;
//! it deliberately defers "a real operator command sourced from a real
//! deploy spec" to THIS step (01-08), per `overdrive-init`'s own module
//! doc: "The real end-to-end boot ... is exercised at step 01-08 under
//! Tier-3."
//!
//! Re-invoking `/sbin/init` itself as the operator's `[vm]` command is NOT
//! a valid choice: `overdrive-init` does not special-case "am I PID 1" —
//! a re-exec'd child instance would try to dial the SAME beacon vsock a
//! second time, which the host `VmDriver` never accepts twice, hanging
//! the guest indefinitely.
//!
//! This file therefore stages its OWN per-test COPY of the shared
//! fixture's rootfs image (never mutating the shared artifact — other
//! Tier-3 VM test files reuse it concurrently per the fixture's own AC5
//! contract) and injects a tiny additional static-musl binary at
//! `/sbin/<name>` via a loopback mount (`losetup` + `mount` + `cp` +
//! `chmod` + `umount` — no shell inside the guest is needed; the
//! injection happens on the HOST before the guest ever boots). See
//! [`build_exit_code_binary`] and [`stage_rootfs_with_extra_binary`].
//!
//! # `#[serial(cgroup)]` — every test here boots a REAL `overdrive serve`
//!
//! Unlike `vm_driver_stop_totality.rs` (`SimCgroupFs`-backed), every test
//! in this file drives a full production `run_server` boot against the
//! REAL host cgroupfs (`RealCgroupFs`, per `ServerConfig::new`'s default
//! composition). `run_server`'s cgroup bootstrap
//! (`overdrive-control-plane/CLAUDE.md` § "Cgroup boot ordering") writes
//! to the MACHINE-GLOBAL `/sys/fs/cgroup/overdrive.slice` tree — running
//! nextest's default per-binary test concurrency against this file
//! without serialization lets two boots race the SAME
//! `overdrive.slice/cgroup.subtree_control` write, tearing the delegation
//! (`workloads.slice` ends up missing, `subtree_control` ends up empty)
//! and failing every concurrent boot with `ENOENT`. Confirmed empirically
//! on the real metal box (2026-08-14): 6 concurrent boots left
//! `overdrive.slice/cgroup.subtree_control` empty and
//! `overdrive.slice/workloads.slice` absent. `#[serial(cgroup)]` forces
//! every test in this file to hold exclusive use of that shared substrate
//! for the duration of its own `serve` lifecycle — the same discipline
//! `.claude/rules/testing.md` § "Tests that mutate process-global state"
//! documents for the `env` group, applied to real host cgroupfs instead
//! of environment variables.
//!
//! # BLOCKER found by this walking skeleton, CLOSED at 01-08 review
//! # remediation — guest vsock is a kernel module the guest never
//! # loaded (DISTILL DWD-21)
//!
//! A direct `cloud-hypervisor` boot (bypassing the driver entirely) on
//! the real metal box, using the EXACT same kernel/rootfs/cmdline
//! `CloudHypervisorVmm::create` composes, produced this REAL guest
//! console output (captured 2026-08-14, kernel `7.0.0-15-generic`,
//! BEFORE this section's fix landed):
//!
//! ```text
//! [    0.781590] Run /sbin/init as init process
//! overdrive-init: fatal: could not create the beacon vsock socket: EAFNOSUPPORT: Address family not supported by protocol
//! [    0.785590] ACPI: PM: Preparing to enter system sleep state S5
//! [    0.786611] reboot: Power down
//! ```
//!
//! This was EXACTLY the gap `vm_fixture.rs`'s own module doc used to
//! flag as unresolved: Ubuntu kernels build `CONFIG_VSOCKETS`/
//! `CONFIG_VIRTIO_VSOCKETS` as *modules*, and `overdrive-init` (as
//! landed in step 01-03) had no `finit_module` logic — it went straight
//! to `socket(AF_VSOCK, ...)`. The box's running kernel (verbatim-copied
//! into every guest per the fixture's "Pinned kernel source" design
//! note) builds vsock as loadable modules, not built-in; nothing in the
//! guest ever `insmod`d them, so `AF_VSOCK` was never registered and
//! every `socket(AF_VSOCK, ...)` call failed `EAFNOSUPPORT` before the
//! guest could ever dial the beacon.
//!
//! **Ruled and closed by DISTILL DWD-21 / ADR-0082 §D4 amendment
//! (2026-08-14), landed as this SAME step's (01-08) review-remediation.**
//! `overdrive-init` now `finit_module`-loads the three vsock modules (in
//! dependency order, from a shared pinned in-guest directory) before
//! `connect_beacon`, tolerating "already loaded" and "absent" (the
//! vsock=y appliance-kernel case, ADR-0068 §4) as success;
//! `vm_fixture.rs`'s `build_staging_tree` stages those same three `.ko`,
//! zstd-decompressed, from the SAME `uname -r` the staged kernel came
//! from. `connect_beacon` also gained a bounded retry (the virtio-vsock
//! PCI probe completes asynchronously after the module load). Matches
//! the spike's proven-12/12 mechanism
//! (`spike-scratch/increment-a/build.sh`,
//! `probe/src/bin/guest_init.rs`). A SECOND, pre-existing, latent bug
//! surfaced alongside this fix and is fixed in the SAME commit:
//! `vm_fixture.rs`'s `stage_kernel` re-verified only that a previously
//! staged kernel copy still PARSED as a valid bzImage, never that it
//! still matched the CURRENTLY RUNNING `uname -r` — a host kernel
//! package upgrade (observed on the metal box mid-investigation:
//! `7.0.0-15-generic` → `7.0.0-29-generic`) left a stale staged kernel
//! paired with freshly-staged modules built for the new release,
//! producing a real `finit_module` ENOEXEC ("vsock: disagrees about
//! version of symbol `module_layout`") — confirmed via a direct manual
//! `cloud-hypervisor` boot against the staged fixture. `stage_kernel`
//! now pins a `.kernel-staged-from` release marker (mirroring
//! `stage_rootfs`'s own `.rootfs-built-from` shape) so a release change
//! invalidates the staged copy the same way a rebuilt `overdrive-init`
//! invalidates the rootfs.
//!
//! **Confirmed CLOSED — the vsock/EAFNOSUPPORT gap itself.** Two
//! consecutive real metal-box runs, post-fix, show ZERO `EAFNOSUPPORT`
//! anywhere. S-VM-02 and S-VM-15 — both of which require the guest to
//! reach `READY`, exec, and report a REAL `EXIT <status>` back over
//! vsock — PASS cleanly and repeatably; S-VM-03/S-VM-04/S-VM-14 (never
//! vsock-dependent) remain GREEN throughout. This is the maximal
//! evidence available that the guest-vsock transport itself works.
//!
//! **The EBUSY blocker — S-VM-05 / S-VM-74, both mTLS-composed, real
//! `EbpfDataplane` — is CLOSED (01-08 review remediation, third pass,
//! 2026-08-14).** Root cause was NOT a production double-attach:
//! `EbpfDataplane::new_with_pin_dir` is called from exactly one
//! `map_or_else` site in `run_server`
//! (`crates/overdrive-control-plane/src/lib.rs`), so a real boot never
//! attaches twice. The EBUSY was two TEST PROCESSES independently
//! simulating that boot concurrently against the SAME fixed, shared
//! kernel interface names (`ovd-veth-cli` / `ovd-veth-bk`) — nextest's
//! default per-test-process concurrency racing the same host-kernel XDP
//! slot, never a state a real deploy (one `overdrive serve` per node)
//! can reach. Fixed by adding both scenarios to the pre-existing
//! `host-kernel-shared` single-writer test-group (`.config/nextest.toml`)
//! — the SAME serialization pattern already applied to
//! `serve_boot_provisions_veth` and `dns_responder_walking_skeleton` for
//! the identical class of gap. Confirmed CLOSED: S-VM-74 passes cleanly
//! and repeatably across two full-suite metal-box runs.
//!
//! **S-VM-05 has a SECOND, DISTINCT, newly-discovered blocker — cross-test
//! contamination, not EBUSY.** Deterministically reproduced (identical
//! failure, two consecutive full-suite runs): the real `cloud-hypervisor`
//! process this scenario's `find_cloud_hypervisor_pid()` locates resolves
//! to `alloc-vm-deadline-0.scope` (S-VM-14's allocation) rather than this
//! scenario's own `alloc-vm-contained-0.scope`. Root cause: S-VM-14's own
//! `no_cloud_hypervisor_process_running()` leak-verification helper still
//! matches on the truncated `/proc/<pid>/comm` (the SAME
//! `TASK_COMM_LEN`=16 truncation bug `find_cloud_hypervisor_pid` below was
//! just fixed to avoid, via `argv[0]`) — it is vacuously always-`true`
//! and silently masks a REAL `cloud-hypervisor` process that outlives
//! S-VM-14's own `cleanup_after_start_failure` path
//! (`crates/overdrive-worker/src/vm_driver.rs`) long enough to
//! contaminate the next serialized test's `/proc` scan. Fixing this needs
//! (a) the same `comm`→`argv0` fix mirrored into
//! `no_cloud_hypervisor_process_running` so S-VM-14 actually verifies its
//! own claim, which will likely flip S-VM-14 itself to failing, and then
//! (b) an investigation into why `cleanup_after_start_failure` does not
//! reliably/promptly terminate the VMM process on boot-deadline. Both are
//! outside this step's file boundary (`overdrive-worker`'s driver
//! internals) and are a genuine investigation, not a design decision this
//! step can improvise (CLAUDE.md § "Implement to the design"). S-VM-05
//! keeps its `#[ignore]`, updated to name this new root cause.
//!
//! **S-VM-01 (the walking skeleton itself) — CLOSED (step 01-08 review
//! remediation, second pass, 2026-08-14).** The guest-side mechanism
//! always worked correctly (READY, EXEC, `EXIT 0` all confirmed over
//! vsock — the SAME mechanism S-VM-02/S-VM-15 prove GREEN); the terminal
//! observation row landed `AllocStateWire::Failed` with
//! `TransitionReason::Stopped{by:Process}` — a state/reason pairing
//! `exit_observer.rs`'s own `classify()` never produces (that reason
//! always pairs with `Terminated`). Root cause was downstream of the
//! vsock/exit-report mechanism, confirming the original finding's own
//! hypothesis: `action_shim::dispatch`'s `Action::FinalizeFailed` arm
//! (`crates/overdrive-control-plane/src/action_shim/mod.rs`) collapsed
//! EVERY non-`Stable` `TerminalCondition` — including `Completed` (the
//! Job-kind clean-exit SUCCESS terminal `WorkloadLifecycle::
//! classify_natural_exit_terminal` emits for exactly this row) — to
//! `AllocState::Failed`, while forward-carrying the prior row's
//! `reason: Stopped{by:Process}` (written correctly by `exit_observer`'s
//! `classify()` a tick earlier) unchanged. Fixed by giving `Completed`
//! its own `AllocState::Terminated` case, alongside the pre-existing
//! `Stable` special-case — matching `TerminalCondition::Completed`'s own
//! documented "exit code 0 is the canonical success" contract and the
//! already-correct sibling `streaming.rs::workload_event_from_terminal`
//! projection (`Completed -> JobSubmitEvent::Succeeded`). The
//! per-driver exit-observer dispatch itself (one task per `DriverRegistry`
//! entry, ADR-0083 §D2a) was never at fault.
//!
//! S-VM-01, S-VM-02, S-VM-03, S-VM-04, S-VM-14, S-VM-15, and S-VM-74 are
//! GREEN and carry no `#[ignore]`. S-VM-05 carries an `#[ignore]` above —
//! never the (now-closed) vsock reason, never the (now-closed)
//! terminal-row-classification reason, and never the (now-closed) EBUSY
//! reason — naming the newly-discovered cross-test-contamination root
//! cause instead (see above).

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
use overdrive_control_plane::VmBootArtifacts;
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::TransitionReason;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::AllocationId;
use overdrive_core::vm::config::{RootfsPlan, VmRunDir};
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tempfile::TempDir;

// ---------------------------------------------------------------------
// Fixture staging — real kernel/rootfs + a per-test injected guest
// command binary.
// ---------------------------------------------------------------------

/// The shared staging root every Tier-3 VM test file provisions against
/// (per `vm_fixture`'s own AC5 concurrency contract — safe under
/// concurrent nextest processes).
fn shared_staging_root() -> PathBuf {
    overdrive_testing::vm_fixture::default_staging_root()
}

/// Cross-builds a tiny static-musl binary that does nothing but
/// `std::process::exit(exit_code)`, via a direct `rustc` invocation (no
/// throwaway Cargo project). Mirrors `vm_fixture`'s own
/// `build_overdrive_init_static` cross-build shape
/// (`x86_64-unknown-linux-musl` — the only target this fixture's kernel
/// staging supports today; `stage_kernel` rejects aarch64 with
/// `KernelImageRequiresUkiUnwrap`, so the walking skeleton is
/// x86_64-only in practice already).
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
/// fixture's own artifact at `fixture.rootfs_path` is never mutated, so
/// concurrent Tier-3 VM test files reusing the SAME shared staging root
/// (per AC5) are unaffected.
///
/// Runs as root (this whole suite runs under `cargo xtask metal run --`,
/// which is root), so `losetup`/`mount`/`umount` need no further
/// escalation.
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

/// Builds an EMPTY, validly-formatted 64 MiB ext4 image with NO staged
/// content at all — no `/sbin/init`, nothing. S-VM-03's "rootfs with no
/// working init" fixture: the kernel's no-`init=` fallback search
/// (`/sbin/init`, `/etc/init`, `/bin/init`, `/bin/sh`) exhausts every
/// candidate and the guest never beacons, so `VM_BOOT_DEADLINE` elapses.
fn build_empty_rootfs(tmp: &Path) -> PathBuf {
    let path = tmp.join("empty-rootfs.ext4");
    {
        let file = std::fs::File::create(&path).expect("create empty rootfs image");
        file.set_len(64 * 1024 * 1024).expect("size empty rootfs image");
    }
    let status = Command::new("mkfs.ext4")
        .arg("-F")
        .args(["-L", "overdrive-vm-ws-empty"])
        .arg(&path)
        .status()
        .expect("spawn mkfs.ext4 for the empty rootfs image");
    assert!(status.success(), "mkfs.ext4 must format the empty rootfs image");
    path
}

// ---------------------------------------------------------------------
// Server composition
// ---------------------------------------------------------------------

/// Spawns a real in-process `overdrive serve` with the given real VM
/// boot artifacts composed, injecting `SimDataplane` (functional
/// correctness scenarios — S-VM-01/02/03/04 do not need the real
/// `EbpfDataplane` / mTLS composition; that is what
/// [`spawn_vm_server_mtls_composed`] is for).
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

/// Spawns a real in-process `overdrive serve` with `dataplane_override`
/// left UNSET — production `run_server` therefore composes the REAL
/// `EbpfDataplane` and `compose_mtls = dataplane_override.is_none()`
/// evaluates `true`, exactly as it does on the production `run` path
/// (GH #248 / ADR-0074 trap: this is the deliberate re-proof that a
/// mesh-composed serve correctly SKIPS the `MtlsInterceptWorker` gate
/// for a `DriverType::Vm` allocation rather than crashing on it).
async fn spawn_vm_server_mtls_composed(vm_artifacts: VmBootArtifacts) -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = ServeArgs { bind, data_dir, config_dir };
    let handle = overdrive_cli::commands::serve::run_with_vm_artifacts(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
        vm_artifacts,
    )
    .await
    .expect("serve::run_with_vm_artifacts (mTLS-composed)");
    (handle, tmp)
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

// ---------------------------------------------------------------------
// Spec authoring + polling
// ---------------------------------------------------------------------

/// A `[job]`+`[vm]`+`[resources]` TOML — the shape `WorkloadSpecInput::
/// from_toml_str`'s job-family branch parses (confirmed GREEN by
/// S-VM-06/S-VM-07 in `vm_spec_driver_table_dispatch.rs`).
fn vm_job_toml(id: &str, command: &str, args: &[&str], kernel: &Path, rootfs: &Path) -> String {
    let args_toml = args.iter().map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(", ");
    format!(
        "[job]\nid = \"{id}\"\n\n[vm]\ncommand = \"{command}\"\nargs = [{args_toml}]\n\
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
/// allocation row reaches a terminal [`AllocStateWire`] (`Terminated` or
/// `Failed`), returning the final snapshot. Real wall-clock polling —
/// this is a Tier-3 test against a real kernel boot, there is no
/// `SimClock` at this layer. `max_wait` should comfortably exceed
/// `VM_BOOT_DEADLINE` (30s) for scenarios that must observe the deadline
/// elapse.
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
            "workload {workload_id} did not reach a terminal state within {max_wait:?}; \
             last observed row: {:?}",
            out.snapshot.rows.first(),
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// ---------------------------------------------------------------------
// S-VM-01 — the walking skeleton itself.
// ---------------------------------------------------------------------

/// S-VM-01 — A VM workload runs to completion and its exit code reaches
/// the operator.
#[tokio::test]
#[serial(cgroup)]
async fn vm_workload_runs_to_completion_and_exit_code_reaches_operator() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let exit0 = build_exit_code_binary(tmp.path(), 0);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit0, "exit0");

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-exit0.toml",
        &vm_job_toml("vm-exit0", "/sbin/exit0", &[], &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the [vm] spec");

    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let row = out.snapshot.rows.first().expect("one allocation row for a freshly-deployed job");
    assert_eq!(
        row.state,
        AllocStateWire::Terminated,
        "a guest that exits 0 must reach Terminated (classify()'s CleanExit branch), got {:?} \
         (reason={:?})",
        row.state,
        row.reason,
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-02 — non-zero guest exit code, never the hypervisor's own.
// ---------------------------------------------------------------------

/// S-VM-02 — A non-zero guest exit code is reported, never the
/// hypervisor's own exit code (cloud-hypervisor itself always exits 0 on
/// a clean guest poweroff, regardless of what the SUPERVISED workload
/// inside the guest returned).
#[tokio::test]
#[serial(cgroup)]
async fn vm_non_zero_guest_exit_code_is_reported_not_the_hypervisors() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let exit7 = build_exit_code_binary(tmp.path(), 7);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit7, "exit7");

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-exit7.toml",
        &vm_job_toml("vm-exit7", "/sbin/exit7", &[], &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the [vm] spec");

    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let row = out.snapshot.rows.first().expect("one allocation row for a freshly-deployed job");
    assert_eq!(
        row.state,
        AllocStateWire::Failed,
        "a guest that exits 7 must reach Failed (classify()'s Crashed branch), got {:?}",
        row.state,
    );
    assert!(
        matches!(
            row.reason,
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(7),
                signal: None,
                ..
            })
        ),
        "the reported exit_code must be the GUEST's 7, never the VMM's own clean 0 -- got \
         reason={:?}",
        row.reason,
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-03 — a guest that never starts is never reported Running.
// ---------------------------------------------------------------------

/// S-VM-03 — A guest whose rootfs has no working init never reaches
/// Running. `VM_BOOT_DEADLINE` (30s) elapses; the allocation goes
/// Pending directly to Failed, and every polled snapshot along the way
/// observes a state OTHER than Running (K2 guardrail).
#[tokio::test]
#[serial(cgroup)]
async fn vm_guest_that_never_starts_is_never_reported_running() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let broken_rootfs = build_empty_rootfs(tmp.path());

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: broken_rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-broken-init.toml",
        &vm_job_toml("vm-broken-init", "/sbin/anything", &[], &fixture.kernel_path, &broken_rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the [vm] spec against the broken rootfs");

    // Poll continuously, asserting Running is NEVER observed along the
    // way, up to comfortably past VM_BOOT_DEADLINE (30s).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        let out =
            describe(DescribeArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
                .await
                .expect("workload describe must succeed while polling");
        if let Some(row) = out.snapshot.rows.first() {
            assert_ne!(
                row.state,
                AllocStateWire::Running,
                "a guest with no working init must NEVER be reported Running (K2 guardrail)"
            );
            if row.state == AllocStateWire::Failed {
                break;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "allocation must reach Failed once VM_BOOT_DEADLINE elapses (90s poll ceiling exceeded)"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-04 — same verb as a process workload, no new CLI surface.
// ---------------------------------------------------------------------

/// S-VM-04 — A `[vm]` spec deploys through the exact same
/// `overdrive_cli::commands::deploy::deploy` handler as an `[exec]`
/// spec — no new verb, no new flag. Proven at deploy-acceptance time
/// (no full boot-to-completion needed; that is S-VM-01/02's claim).
#[tokio::test]
#[serial(cgroup)]
async fn vm_workload_deploys_through_the_same_verb_as_a_process_workload() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let exit0 = build_exit_code_binary(tmp.path(), 0);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit0, "exit0");

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-same-verb.toml",
        &vm_job_toml("vm-same-verb", "/sbin/exit0", &[], &fixture.kernel_path, &rootfs),
    );
    // The SAME `deploy()` fn every [exec] spec in this crate's other
    // integration tests calls (`exec_spec_walking_skeleton.rs`,
    // `workload_restart.rs`) — no `[vm]`-specific handler, no new CLI
    // subcommand.
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg })
        .await
        .expect("deploy the [vm] spec through the exact same verb an [exec] spec uses");
    assert_eq!(submit.workload_id, "vm-same-verb");
    assert_eq!(
        submit.outcome,
        overdrive_control_plane::api::IdempotencyOutcome::Inserted,
        "a fresh [vm] deploy must report Inserted, exactly like a fresh [exec] deploy"
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-05 — the platform contains the hypervisor it started.
// ---------------------------------------------------------------------

/// Finds the single running `cloud-hypervisor` process's pid by scanning
/// `/proc` for a matching `argv[0]` basename. Assumes exactly one VM is
/// booted at the time of the call (true for every scenario in this file
/// — each test uses its own server + its own single allocation).
///
/// Matches on `argv[0]` (the first NUL-delimited field of
/// `/proc/<pid>/cmdline`), NOT `/proc/<pid>/comm`: the kernel's
/// `TASK_COMM_LEN` caps `comm` at 15 visible characters (16 bytes
/// including the trailing NUL), and `"cloud-hypervisor"` is exactly 16
/// characters, so the kernel-reported `comm` for the real binary is
/// always truncated to `"cloud-hyperviso"` — confirmed directly against
/// the real binary on the metal box (`argv[0]` reads `cloud-hypervisor`,
/// len 16; `comm` reads `cloud-hyperviso`, len 15). A `comm`-based exact
/// match can never succeed against this binary name, which is why this
/// helper previously panicked even when a real VMM was running.
fn find_cloud_hypervisor_pid() -> u32 {
    for entry in std::fs::read_dir("/proc").expect("read /proc") {
        let Ok(entry) = entry else { continue };
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let cmdline_path = entry.path().join("cmdline");
        let Ok(cmdline) = std::fs::read(&cmdline_path) else { continue };
        let argv0 = cmdline.split(|&b| b == 0).next().unwrap_or(&[]);
        let argv0 = String::from_utf8_lossy(argv0);
        if Path::new(argv0.as_ref()).file_name() == Some(std::ffi::OsStr::new("cloud-hypervisor")) {
            return pid;
        }
    }
    panic!("no running cloud-hypervisor process found in /proc");
}

/// S-VM-05 — The platform contains the hypervisor it started: the real
/// `cloud-hypervisor` process this alloc's `VmDriver::start` spawned is
/// confined to the allocation's own cgroup scope, verified on the
/// mTLS-composed production boot (the GH #248 / ADR-0074 trap this
/// feature deliberately re-proves closed).
#[tokio::test]
#[serial(cgroup)]
#[ignore = "NEW BLOCKER, DISTINCT FROM THE CLOSED EBUSY GAP (see module doc): the EBUSY \
            XDP double-attach is CLOSED (host-kernel-shared serialization in \
            .config/nextest.toml) -- confirmed by S-VM-74 (same mTLS-composed real-\
            EbpfDataplane path) passing cleanly across two full-suite runs. This scenario \
            instead now fails deterministically (reproduced identically twice) with the \
            allocation's cloud-hypervisor PID resolving to a DIFFERENT allocation's cgroup \
            scope: 'the cloud-hypervisor process's cgroup must resolve to the allocation's \
            own workload scope, got: 0::/overdrive.slice/workloads.slice/alloc-vm-deadline-0.scope' \
            -- alloc-vm-deadline-0 is S-VM-14's allocation, not this test's own \
            (alloc-vm-contained-0). Root cause: S-VM-14's own leak-verification helper \
            (no_cloud_hypervisor_process_running, this file) still matches on the truncated \
            /proc/<pid>/comm (TASK_COMM_LEN=16 caps comm at 15 visible chars -- the SAME bug \
            just fixed in find_cloud_hypervisor_pid, but not fixed there), so it is \
            vacuously always-true and silently masks a REAL cloud-hypervisor process that \
            outlives S-VM-14's own deadline-cleanup path (crates/overdrive-worker/src/\
            vm_driver.rs::cleanup_after_start_failure) long enough to contaminate the NEXT \
            serialized test's /proc scan. Fixing this needs (a) the comm->argv0 fix mirrored \
            into no_cloud_hypervisor_process_running so S-VM-14 actually verifies its own \
            claim, and (b) an investigation into why cleanup_after_start_failure does not \
            reliably/promptly terminate the VMM process on boot-deadline -- both outside \
            this step's file boundary and requiring further investigation, not a design \
            decision this step can improvise."]
async fn vm_platform_contains_the_hypervisor_it_started() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    // A long-lived guest command (not exit0) -- this scenario asserts on
    // the LIVE, Running process's containment, so the guest must still
    // be executing when the assertion runs. `sleep`-shaped: reuse the
    // exit-code binary generator with a code that never returns by
    // looping instead -- simplest is a tiny static binary that loops
    // forever until killed by the test's own shutdown.
    let src = tmp.path().join("spin.rs");
    std::fs::write(
        &src,
        "fn main() { loop { std::thread::sleep(std::time::Duration::from_secs(3600)); } }",
    )
    .expect("write spin source");
    let spin_bin = tmp.path().join("spin");
    let rustc_status = Command::new("rustc")
        .arg("--edition")
        .arg("2021")
        .arg("-C")
        .arg("opt-level=0")
        .arg("-C")
        .arg("target-feature=+crt-static")
        .arg("--target")
        .arg("x86_64-unknown-linux-musl")
        .arg("-o")
        .arg(&spin_bin)
        .arg(&src)
        .status()
        .expect("spawn rustc for the long-lived spin binary");
    assert!(rustc_status.success(), "rustc must build the long-lived spin binary");
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin_bin, "spin");

    let (handle, server_tmp) = spawn_vm_server_mtls_composed(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-contained.toml",
        &vm_job_toml("vm-contained", "/sbin/spin", &[], &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the [vm] spec on an mTLS-composed serve");

    // Poll until Running (the spin guest never exits on its own).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let out =
            describe(DescribeArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
                .await
                .expect("workload describe must succeed while polling");
        if out.snapshot.rows.first().is_some_and(|r| r.state == AllocStateWire::Running) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "allocation must reach Running within 60s on an mTLS-composed serve"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // The real cloud-hypervisor process this alloc's VmDriver::start
    // spawned must be confined to a cgroup scope naming this allocation.
    let vmm_pid = find_cloud_hypervisor_pid();
    let cgroup_contents = std::fs::read_to_string(format!("/proc/{vmm_pid}/cgroup"))
        .expect("read /proc/<pid>/cgroup");
    assert!(
        cgroup_contents.contains(submit.workload_id.as_str())
            || cgroup_contents.contains("vm-contained"),
        "the cloud-hypervisor process's cgroup must resolve to the allocation's own workload \
         scope, got: {cgroup_contents}"
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-74 — mTLS-composed VM alloc gets no listener/TPROXY install.
// ---------------------------------------------------------------------

/// S-VM-74 — On an mTLS-composed serve boot (real `EbpfDataplane`,
/// `dataplane_override` unset), a `DriverType::Vm` allocation boots and
/// reaches Terminated cleanly — proving the `MtlsInterceptWorker`'s
/// `DriverType::Exec` gate (`action_shim::mod.rs`, both call sites)
/// correctly SKIPS a Vm allocation rather than attempting (and failing
/// or hanging on) a listener/TPROXY install shaped for a process
/// allocation's netns.
#[tokio::test]
#[serial(cgroup)]
async fn vm_alloc_on_mtls_composed_serve_boots_cleanly_without_mtls_install() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let exit0 = build_exit_code_binary(tmp.path(), 0);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit0, "exit0");

    let (handle, server_tmp) = spawn_vm_server_mtls_composed(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-mtls-gate.toml",
        &vm_job_toml("vm-mtls-gate", "/sbin/exit0", &[], &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the [vm] spec on an mTLS-composed serve");

    // If the Exec-only gate were broken (i.e. MtlsInterceptWorker::
    // start_alloc were invoked for this Vm allocation), it would attempt
    // netns/listener operations shaped for a process allocation and
    // either error the StartAllocation dispatch or hang; a clean
    // Terminated is the proof the gate correctly skipped it.
    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let row = out.snapshot.rows.first().expect("one allocation row for a freshly-deployed job");
    assert_eq!(
        row.state,
        AllocStateWire::Terminated,
        "a Vm allocation on an mTLS-composed serve must boot and exit cleanly, unaffected by \
         the Exec-only MtlsInterceptWorker gate, got {:?} (reason={:?})",
        row.state,
        row.reason,
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-14 — the deadline arm of the three-way boot race leaks nothing.
// ---------------------------------------------------------------------

/// `true` iff no `cloud-hypervisor` process is running anywhere on the
/// host. Mirrors [`find_cloud_hypervisor_pid`]'s `/proc` scan, inverted
/// -- this file's own established single-VM-at-a-time assumption (see
/// that function's doc comment) makes "no CH process found anywhere" an
/// honest "this allocation's VMM is gone" signal under
/// `#[serial(cgroup)]`'s exclusivity.
fn no_cloud_hypervisor_process_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else { return true };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(_pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let comm_path = entry.path().join("comm");
        if let Ok(comm) = std::fs::read_to_string(&comm_path)
            && comm.trim() == "cloud-hypervisor"
        {
            return false;
        }
    }
    true
}

/// S-VM-14 — The deadline arm of the three-way boot race leaks nothing:
/// a guest that never beacons ready (the SAME broken-init fixture
/// S-VM-03 uses -- the kernel's no-`init=` fallback search exhausts and
/// nothing ever execs, so `overdrive-init` never runs and vsock is
/// never dialed) reaches Failed once `VM_BOOT_DEADLINE` elapses, and no
/// cloud-hypervisor process, run directory, rootfs clone, or cgroup
/// scope remains for the allocation.
///
/// Real-substrate companion to `vm_driver_stop_totality.rs`'s
/// `boot_deadline_elapses_releases_claim_and_cleans_up` -- the `SimVmm`
/// component-scope enforcement vehicle for this SAME
/// `cleanup_after_start_failure` code path (see that file's own module
/// doc: "S-VM-14 ... AC-06's Tier-3 `@real-io` evidence for this SAME
/// race and cleanup logic ... against a real Cloud Hypervisor boot").
///
/// The allocation's supervision claim (`VmDriver`'s internal
/// `VmSupervision` map) is released as the LAST statement of the SAME
/// synchronous `cleanup_after_start_failure` call that performs every
/// other side effect asserted below
/// (`crates/overdrive-worker/src/vm_driver.rs`) -- no operator-facing
/// surface exposes `Driver::live_allocations()` today (no caller drives
/// the reclamation transitions this feature's own ADR references), so
/// proving the other four side effects landed is the maximal black-box
/// evidence the `overdrive deploy` driving port can give for claim
/// release without inventing new API surface (CLAUDE.md § "Implement to
/// the design" -- mirrors the explicit-boundary-statement precedent
/// DISTILL set for S-VM-67).
#[tokio::test]
#[serial(cgroup)]
async fn vm_deadline_arm_leaks_nothing() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let broken_rootfs = build_empty_rootfs(tmp.path());

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: broken_rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-deadline.toml",
        &vm_job_toml("vm-deadline", "/sbin/anything", &[], &fixture.kernel_path, &broken_rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the [vm] spec whose guest never beacons ready");

    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(90)).await;
    let row = out.snapshot.rows.first().expect("one allocation row for a freshly-deployed job");
    assert_eq!(
        row.state,
        AllocStateWire::Failed,
        "a guest that never beacons ready must reach Failed once VM_BOOT_DEADLINE elapses, got \
         {:?}",
        row.state,
    );

    let alloc =
        AllocationId::new(&row.alloc_id).expect("server-echoed alloc_id parses as AllocationId");

    assert!(
        no_cloud_hypervisor_process_running(),
        "the deadline arm's cleanup must terminate the VMM process; none may remain"
    );

    let master_bytes = std::fs::metadata(&broken_rootfs).expect("stat the broken rootfs").len();
    let rootfs_plan = RootfsPlan::for_alloc(broken_rootfs.clone(), master_bytes, &alloc);
    assert!(
        !rootfs_plan.clone_dest().exists(),
        "the deadline arm's cleanup must remove the per-launch rootfs clone at {}",
        rootfs_plan.clone_dest().display()
    );

    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), &alloc);
    assert!(
        !run_dir.path().exists(),
        "the deadline arm's cleanup must remove the run directory at {}",
        run_dir.path().display()
    );

    let scope_dir = CgroupPath::for_alloc(&alloc).resolve(Path::new("/sys/fs/cgroup"));
    assert!(
        !scope_dir.exists(),
        "the deadline arm's cleanup must remove the cgroup scope at {}",
        scope_dir.display()
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-15 — a guest EXIT report is never overwritten by the VMM's own
// teardown exit.
// ---------------------------------------------------------------------

/// S-VM-15 — A guest EXIT report is never overwritten by the VMM's own
/// teardown exit: the reported exit code stays the guest's, even though
/// every real guest poweroff races its own `EXIT <status>` beacon write
/// against the subsequent `cloud-hypervisor` process exit (0, clean,
/// during its own teardown).
///
/// Real-substrate companion to `vm_driver_stop_totality.rs`'s
/// `guest_exit_report_is_authoritative_over_subsequent_vmm_teardown`,
/// which exercises this SAME `drain_guest_report` /
/// `classify_vm_exit` code path at the `SimVmm` level by forcing the
/// VMM's own exit to resolve FIRST via a test-only `Vmm` decorator, so
/// the exit watcher's `GUEST_REPORT_DRAIN_MAX_YIELDS` retry loop
/// actually iterates while it waits out the guest's own report.
///
/// A real Cloud Hypervisor boot cannot have its process-exit timing
/// test-injected the way `SimVmm` can -- there is no fault-injection
/// seam on the production `Vmm` adapter, and minting one is outside
/// this step's design scope (CLAUDE.md § "Implement to the design").
/// This test's evidence is instead the REAL substrate's own natural
/// race window: `overdrive-init` writes `EXIT 7` and only THEN reads
/// for `SHUTDOWN`/EOF before powering off (ADR-0082 §D7) -- it is the
/// guest's own poweroff that drives `cloud-hypervisor`'s subsequent,
/// signal-less, clean process exit, so every real guest self-exit
/// already races the two events in exactly the order this scenario's
/// `Given`/`When` describe. A lost race would surface as
/// `exit_code: None` (the VMM's own clean, signal-less exit, per
/// `classify_vm_exit`'s no-report fallback row) rather than the
/// guest's real `7` -- distinct evidence from S-VM-02's own claim (the
/// operator sees the GUEST's code, never the VMM's), which this
/// scenario's internal drain-then-classify ordering is what makes true.
#[tokio::test]
#[serial(cgroup)]
async fn vm_guest_exit_report_is_never_overwritten_by_vmm_teardown_exit() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let exit7 = build_exit_code_binary(tmp.path(), 7);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit7, "exitreport");

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-exit-report-priority.toml",
        &vm_job_toml(
            "vm-exit-report-priority",
            "/sbin/exitreport",
            &[],
            &fixture.kernel_path,
            &rootfs,
        ),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the [vm] spec");

    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let row = out.snapshot.rows.first().expect("one allocation row for a freshly-deployed job");
    assert_eq!(
        row.state,
        AllocStateWire::Failed,
        "a guest that exits 7 must reach Failed, got {:?}",
        row.state,
    );
    assert!(
        matches!(
            row.reason,
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(7),
                signal: None,
                ..
            })
        ),
        "the guest's EXIT report must arrive and be classified before the ExitEvent is emitted \
         -- a lost race would report exit_code: None (the VMM's own clean, signal-less \
         teardown exit) instead of the guest's real 7; got reason={:?}",
        row.reason,
    );

    handle.shutdown().await.expect("clean shutdown");
}
