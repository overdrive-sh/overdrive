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
//! **S-VM-05's SECOND, DISTINCT blocker — cross-test contamination, not
//! EBUSY — is CLOSED (01-08 review remediation, fourth pass, 2026-08-14,
//! commit `28dbefdc`).** The symptom (S-VM-05's `find_cloud_hypervisor_pid()`
//! resolving to S-VM-14's `alloc-vm-deadline-0.scope` rather than its own) was
//! NOT a defect in `VmDriver::cleanup_after_start_failure` /
//! `CloudHypervisorVmm::terminate` (`crates/overdrive-worker/src/vm_driver.rs`)
//! — S-VM-14 run in true isolation reaps its VMM process, rootfs clone, run
//! directory, and cgroup scope cleanly, every time. The real causes were two
//! test-harness gaps: (a) `#[serial(cgroup)]` only synchronises WITHIN one OS
//! process, but nextest runs each test as its own process, and only 2 of this
//! file's 8 scenarios were in the `host-kernel-shared` cross-process
//! single-writer group — widened to the WHOLE module (`.config/nextest.toml`,
//! see the by-module filter this file's tests join); (b) S-VM-05's own
//! long-lived "spin" guest never exits on its own and the test never stopped
//! it before `handle.shutdown()`, so combined with production's deliberate
//! `kill_on_drop(false)` on the spawned VMM process, its own VMM was orphaned
//! every run and contaminated the NEXT serialized test's `/proc` scan — fixed
//! by having the test drive its own workload to Terminated through the
//! production `stop` verb first (test hygiene, not a new acceptance claim).
//! Also de-vacuumed S-VM-14's OWN `no_cloud_hypervisor_process_running`
//! helper, which matched the `TASK_COMM_LEN`-truncated `/proc/<pid>/comm` and
//! could never equal "cloud-hypervisor" (16 chars, `comm` caps at 15) —
//! mirrors the `argv0` fix `find_cloud_hypervisor_pid` below already applied.
//! S-VM-05 is un-ignored; all 8 scenarios pass on the metal box with zero
//! live `cloud-hypervisor` processes remaining after the full suite run.
//!
//! **S-VM-74's own assertion was a weak proxy — strengthened (01-08 review
//! remediation, fifth pass, this pass).** The original version asserted only
//! `row.state == Terminated`, which a mutant removing the `DriverType::Exec`
//! mTLS-install gate (`action_shim::mod.rs`, both call sites) would very
//! likely SURVIVE: this alloc's netns/veth is provisioned regardless of
//! driver type (ADR-0083 §D2a(c)), so a broken gate would call `start_alloc`
//! with REAL `host_veth`/`workload_addr` values, and neither the resulting
//! nft-TPROXY rule nor the two bound leg-F/leg-C listeners would, by
//! themselves, stop the guest from booting and exiting cleanly (still
//! Terminated either way). The scenario now asserts DIRECTLY, against the
//! real kernel, that neither was installed — see
//! `vm_alloc_on_mtls_composed_serve_boots_cleanly_without_mtls_install`'s own
//! doc comment for the mechanism, and for why the check runs WHILE the alloc
//! is confirmed Running rather than after Terminated.
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
//! All 8 scenarios — S-VM-01, S-VM-02, S-VM-03, S-VM-04, S-VM-05, S-VM-14,
//! S-VM-15, and S-VM-74 — are GREEN and carry no `#[ignore]`. Every blocker
//! this file's history above documents (vsock EAFNOSUPPORT, the terminal-row
//! misclassification, the XDP EBUSY race, and S-VM-05's cross-test
//! contamination) is CLOSED.
//!
//! **Step 01-10** adds a 9th scenario, S-VM-09 (per-thread seccomp
//! verification — the C-5 correction, see
//! [`vm_seccomp_is_verified_per_thread_not_on_the_thread_group_leader`]'s
//! own doc comment).
//!
//! **Step 03-07** appends a 10th scenario, S-VM-54 (DWD-25 / AC-21), as a
//! RED scaffold at the bottom of this file — the one test here that is not
//! GREEN. It carries `#[should_panic(expected = "RED scaffold")]`, never
//! `#[ignore]`, so the bar stays green and it stays discoverable via
//! `grep -rn 'should_panic.*RED scaffold' crates/`. It cannot be written
//! before 03-07 lands because every server helper above composes through
//! `VmBootArtifacts` — the node-level artifact seam DWD-25 deletes — and
//! S-VM-54's whole claim is that no such seam exists. See its own doc
//! comment for the activation plan.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::missing_panics_doc, clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, WorkloadDescribeOutput, describe};
use overdrive_control_plane::VmBootArtifacts;
use overdrive_control_plane::api::{AllocStateWire, IdempotencyOutcome};
use overdrive_core::TransitionReason;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::AllocationId;
use overdrive_core::vm::config::{MemoryPlan, RootfsPlan, VmRunDir};
use overdrive_host::CloudHypervisorVmm;
use overdrive_sim::{SimVmm, SimVmmProbeFault};
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

    // Hygiene, not a new acceptance claim: the "spin" guest loops forever
    // and never beacons EXIT, so unlike every other scenario in this file
    // its allocation cannot reach a terminal state on its own. Explicitly
    // stopping it through the production stop verb -- then confirming the
    // row actually reaches Terminated -- drives it through VmDriver::stop's
    // real teardown (guest SHUTDOWN write -> VM_SHUTDOWN_REQUEST_DEADLINE
    // -> Vmm::terminate's VM_STOP_GRACE -> forceful kill+reap,
    // crates/overdrive-worker/src/vm_driver.rs) before this test's own
    // `handle.shutdown()` tears down the server. Without this, the real
    // `cloud-hypervisor` process is orphaned: `Command::kill_on_drop` is
    // deliberately `false` on the production spawn (crates/overdrive-host/
    // src/vmm.rs), so nothing kills a still-Running VM merely because the
    // test process housing it exits -- confirmed directly on the metal box
    // (01-08 review remediation): a "PASS" run of this scenario without
    // this stop step left its own `alloc-vm-contained-0` cloud-hypervisor
    // process alive in `/proc`, which a LATER test's system-wide `/proc`
    // scan (S-VM-14's `no_cloud_hypervisor_process_running`) then read as
    // a leak. `VmDriver::stop`'s cleanup path was never at fault -- the gap
    // was this test never invoking it for a workload that cannot reach a
    // terminal state by itself.
    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop the long-lived spin workload before shutdown");
    let stopped = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    let stopped_row =
        stopped.snapshot.rows.first().expect("one allocation row for the stopped workload");
    assert_eq!(
        stopped_row.state,
        AllocStateWire::Terminated,
        "an operator stop must drive the never-self-terminating spin allocation to Terminated \
         before this test tears down the server, got {:?}",
        stopped_row.state,
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-09 — seccomp is verified per-thread, not on the thread-group leader.
// ---------------------------------------------------------------------

/// The parsed `Seccomp:` mode from one `/proc/<pid>/task/<tid>/status`
/// file. `0` is `SECCOMP_MODE_DISABLED` (no filter installed on that
/// thread); a confined thread reports `2` (`SECCOMP_MODE_FILTER`, the mode
/// `--seccomp true` installs).
fn thread_seccomp_mode(vmm_pid: u32, tid: &str) -> u32 {
    let path = format!("/proc/{vmm_pid}/task/{tid}/status");
    let status = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Seccomp:") {
            return rest
                .trim()
                .parse::<u32>()
                .unwrap_or_else(|e| panic!("parse Seccomp: field in {path}: {e}"));
        }
    }
    panic!("no Seccomp: field found in {path}")
}

/// Every thread's `comm` name for `vmm_pid`, keyed by tid, read via
/// `/proc/<pid>/task/<tid>/comm`. Cloud Hypervisor names its threads
/// distinctly (`vmm`, `http-server`, `vcpu0`, ...) -- all well under
/// `TASK_COMM_LEN`'s 15-visible-character cap, unlike `cloud-hypervisor`
/// itself (see [`find_cloud_hypervisor_pid`]'s doc comment).
fn thread_names(vmm_pid: u32) -> std::collections::BTreeMap<String, String> {
    let task_dir = format!("/proc/{vmm_pid}/task");
    let mut out = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(&task_dir).unwrap_or_else(|e| panic!("read {task_dir}: {e}")) {
        let entry = entry.unwrap_or_else(|e| panic!("read {task_dir} entry: {e}"));
        let tid = entry.file_name().to_string_lossy().into_owned();
        let comm_path = entry.path().join("comm");
        let comm = std::fs::read_to_string(&comm_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", comm_path.display()));
        out.insert(tid, comm.trim().to_owned());
    }
    out
}

/// `SECCOMP_MODE_DISABLED` — the mode a bare `/proc/<pid>/status` read
/// (the thread-group leader) correctly reports on a properly-confined CH
/// (C-5, spike P5 correction 2). Named so both assertions below read as
/// "must equal the disabled mode" / "must NOT equal the disabled mode"
/// rather than a bare magic `0`.
const SECCOMP_MODE_DISABLED: u32 = 0;

/// S-VM-09 -- Seccomp is verified per-thread, not on the thread-group
/// leader. **C-5 correction** (brief.md §102 row C-5, §113; DESIGN
/// 2026-08-11): the original Slice 01 AC read a bare `/proc/<pid>/status`,
/// which FAILS against correct cloud-hypervisor behaviour -- spike P5
/// measured `Seccomp: 0` on the thread-group leader of a *correctly*
/// confined CH, because the filters are installed per-thread, after the
/// `vmm` / `http-server` / `vcpu0` threads are spawned, never re-applied to
/// (or inherited retroactively by) the leader's own status. A regression
/// that dropped `--seccomp` from the argv renderer entirely would ALSO read
/// `Seccomp: 0` on the leader -- so a leader-only check cannot distinguish
/// "confined correctly" from "not confined at all"; this scenario is the
/// runtime regression guard S-VM-08's argv-level assertion is paired with
/// (S-VM-08 is the binding mutation-kill site; this is the `/proc`-level
/// half, satisfied by CH's own default and therefore not itself proof this
/// slice acted -- see brief.md §106 / slice-01's `[D7]` item 6).
#[tokio::test]
#[serial(cgroup)]
async fn vm_seccomp_is_verified_per_thread_not_on_the_thread_group_leader() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    // A long-lived guest (never exits on its own) -- this scenario needs
    // the real cloud-hypervisor process alive so its threads can be
    // inspected via /proc while Running. Reuses the S-VM-74 helper rather
    // than a third inline copy of the same spin.rs shape.
    let spin_bin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin_bin, "spinseccomp");

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-seccomp.toml",
        &vm_job_toml("vm-seccomp", "/sbin/spinseccomp", &[], &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the [vm] spec");

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
        assert!(tokio::time::Instant::now() < deadline, "allocation must reach Running within 60s");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let vmm_pid = find_cloud_hypervisor_pid();
    let names = thread_names(vmm_pid);

    // The thread-group leader's OWN status (tid == pid) is NEVER used as
    // the sole evidence -- confirm it, on its own, reports the DISABLED
    // mode a bare `/proc/<pid>/status` read would see, and document that
    // this alone must not fail the scenario (C-5's correction).
    let leader_mode = thread_seccomp_mode(vmm_pid, &vmm_pid.to_string());
    assert_eq!(
        leader_mode, SECCOMP_MODE_DISABLED,
        "the thread-group leader's own status is expected to report SECCOMP_MODE_DISABLED \
         on a correctly-confined cloud-hypervisor -- this is not a failure, it documents why \
         a bare /proc/<pid>/status read is the wrong evidence (C-5); observed threads: \
         {names:?}"
    );

    let vmm_tid = names
        .iter()
        .find(|(_, name)| name.as_str() == "vmm")
        .map_or_else(|| panic!("no thread named 'vmm' among {names:?}"), |(tid, _)| tid.clone());
    let http_server_tid = names.iter().find(|(_, name)| name.contains("http")).map_or_else(
        || panic!("no thread with 'http' in its name among {names:?}"),
        |(tid, _)| tid.clone(),
    );
    let vcpu0_tid = names.iter().find(|(_, name)| name.contains("vcpu0")).map_or_else(
        || panic!("no thread with 'vcpu0' in its name among {names:?}"),
        |(tid, _)| tid.clone(),
    );

    for (label, tid) in
        [("vmm", &vmm_tid), ("http-server", &http_server_tid), ("vcpu0", &vcpu0_tid)]
    {
        let mode = thread_seccomp_mode(vmm_pid, tid);
        assert_ne!(
            mode, SECCOMP_MODE_DISABLED,
            "the {label} thread (tid={tid}) must report a non-default Seccomp mode; got \
             {mode} (0 = disabled) -- observed threads: {names:?}"
        );
    }

    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop the long-lived spin workload before shutdown");
    let stopped = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    let stopped_row =
        stopped.snapshot.rows.first().expect("one allocation row for the stopped workload");
    assert_eq!(
        stopped_row.state,
        AllocStateWire::Terminated,
        "an operator stop must drive the never-self-terminating spin allocation to Terminated \
         before this test tears down the server, got {:?}",
        stopped_row.state,
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-74 — mTLS-composed VM alloc gets no listener/TPROXY install.
// ---------------------------------------------------------------------

/// Cross-builds a tiny static-musl binary that loops forever until
/// killed. The SAME "never exits on its own" shape
/// [`vm_platform_contains_the_hypervisor_it_started`]'s (S-VM-05) own
/// inline `spin.rs` uses, duplicated here (rather than shared) so this
/// file's already-GREEN S-VM-05 body stays untouched by this step's
/// review remediation (01-08 D2) — S-VM-74 needs its own long-lived
/// guest so its allocation has a reliably-observable Running window (see
/// the test's own doc comment for why).
fn build_spin_binary(tmp: &Path) -> PathBuf {
    let src = tmp.join("spinmtls.rs");
    std::fs::write(
        &src,
        "fn main() { loop { std::thread::sleep(std::time::Duration::from_secs(3600)); } }",
    )
    .expect("write spin source");
    let out = tmp.join("spinmtls");
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
        .expect("spawn rustc for the long-lived spinmtls binary");
    assert!(status.success(), "rustc must build the long-lived spinmtls binary");
    out
}

/// `nft list table ip overdrive-mtls`'s stdout, or a fixed sentinel when
/// the shared mTLS table does not exist yet (`nft` exits non-zero —
/// "No such file or directory": no `DriverType::Exec` alloc has
/// installed anything in this suite run yet). `install_outbound_tproxy`
/// / `install_inbound_tproxy`
/// (`crates/overdrive-worker/src/mtls_intercept.rs`) are the ONLY
/// writers of this table, and both are reachable ONLY through
/// `MtlsInterceptWorker::start_alloc`, itself gated on
/// `spec.driver.driver_type() == DriverType::Exec` (`action_shim::
/// mod.rs`, both call sites — confirmed by that crate's own Tier-3 AT,
/// `overdrive-worker`'s `start_alloc_installs_outbound_and_inbound_tproxy_no_cgroup`,
/// which dumps this exact table via the same `nft list ...` shape and
/// observes the egress rule appear synchronously the instant
/// `start_alloc` returns). A byte-identical snapshot taken before this
/// alloc's deploy and again while it is confirmed Running is therefore
/// direct, kernel-observable proof the gate held for this
/// `DriverType::Vm` allocation.
fn overdrive_mtls_nft_snapshot() -> String {
    let out = Command::new("nft")
        .args(["list", "table", "ip", "overdrive-mtls"])
        .output()
        .expect("spawn nft list table ip overdrive-mtls");
    if out.status.success() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        "<overdrive-mtls table absent>".to_owned()
    }
}

/// The set of `addr:port` TCP LISTEN sockets bound on loopback
/// (`127.0.0.1`), read via `ss -tlnH` (numeric, no header, TCP-only,
/// listening-only). This is exactly the bind shape
/// `MtlsInterceptWorker::start_alloc`'s leg-F / leg-C
/// `bind_transparent(SocketAddrV4::new(LOCALHOST, 0))` calls produce
/// (`crates/overdrive-worker/src/mtls_intercept_worker.rs`) — both
/// listeners are bound in the CONTROL-PLANE PROCESS's own (root) netns,
/// never inside a per-workload netns (the install code never `setns`s
/// before binding them), so this needs no `nsenter`: the server under
/// test and this test process already share one netns. A before/
/// while-Running diff of this set is the direct, kernel-observable
/// proof that neither listener was bound for this alloc.
fn loopback_tcp_listeners() -> BTreeSet<String> {
    let out = Command::new("ss").args(["-tlnH"]).output().expect("spawn ss -tlnH");
    assert!(
        out.status.success(),
        "ss -tlnH must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.trim_start().starts_with("LISTEN"))
        .filter_map(|line| line.split_whitespace().nth(3))
        .filter(|addr_port| addr_port.starts_with("127.0.0.1:"))
        .map(str::to_owned)
        .collect()
}

/// S-VM-74 — On an mTLS-composed serve boot (real `EbpfDataplane`,
/// `dataplane_override` unset), a `DriverType::Vm` allocation boots and
/// runs unaffected by the `MtlsInterceptWorker`'s `DriverType::Exec`
/// gate (`action_shim::mod.rs`, both call sites): NO TPROXY rule and NO
/// leg-F/leg-C listener is installed for it, proven DIRECTLY against the
/// real kernel — not merely inferred from reaching Terminated.
///
/// 01-08 review remediation (D2): the ORIGINAL version of this test
/// asserted only `row.state == Terminated` against an `exit0` guest — a
/// weak proxy a mutant removing the `DriverType::Exec` condition would
/// very likely SURVIVE. `provision_and_inject_netns` provisions this
/// alloc's netns/veth regardless of driver type (ADR-0083 §D2a(c)), so a
/// broken gate would call `start_alloc` with REAL `host_veth`/
/// `workload_addr` values — and neither the resulting nft-TPROXY rule
/// nor the two bound leg-F/leg-C listeners would, by themselves, stop
/// the guest from booting and exiting cleanly (still Terminated either
/// way).
///
/// The fixture is now the SAME long-lived "spin" guest S-VM-05 uses
/// (never exits on its own), so the allocation has a reliably-observable
/// Running window: `overdrive_mtls_nft_snapshot()` +
/// `loopback_tcp_listeners()` are compared before deploy against a
/// snapshot taken WHILE the alloc is confirmed Running (after a settle
/// window). This is deliberately NEVER compared against a
/// post-Terminated snapshot: `MtlsInterceptWorker::stop_alloc`
/// (`action_shim::mod.rs`, both terminal call sites) is idempotent and
/// UNGATED by driver type, so it tears down ANY intercept — including
/// one a broken gate installed — on every terminal transition. A
/// before/after-Terminated diff would be vacuous: install-then-
/// immediate-teardown happens entirely inside that window and leaves NO
/// net difference even when the gate is broken (the exact shape
/// `overdrive-worker`'s own `start_alloc_installs_outbound_and_inbound_tproxy_no_cgroup`
/// AC4 relies on — its `stop_alloc` call removes the very rule its AC1
/// just proved present). The settle sleep after first observing Running
/// absorbs the async gap between the Running row becoming externally
/// readable (`obs.write(...).await` resolving, on the SAME task that
/// then calls the synchronous `start_alloc` with no further `.await` in
/// between) and this test's own poller running on a different OS thread
/// — generous relative to both the ~500ms poll interval used throughout
/// this file and the millisecond-scale `nft`-subprocess + socket-bind
/// work `start_alloc` performs.
#[tokio::test]
#[serial(cgroup)]
async fn vm_alloc_on_mtls_composed_serve_boots_cleanly_without_mtls_install() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let spin_bin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin_bin, "spinmtls");

    let (handle, server_tmp) = spawn_vm_server_mtls_composed(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    // D2 baseline, taken BEFORE deploy: the ambient shared mTLS nft
    // state + loopback LISTEN set this suite run has accumulated so far
    // (may be non-empty if an earlier host-kernel-shared test already
    // ran). `.config/nextest.toml`'s single-writer group guarantees
    // nothing else can mutate this real-kernel state concurrently with
    // THIS test, so any growth from here on is directly attributable to
    // this alloc.
    let nft_before = overdrive_mtls_nft_snapshot();
    let listeners_before = loopback_tcp_listeners();

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-mtls-gate.toml",
        &vm_job_toml("vm-mtls-gate", "/sbin/spinmtls", &[], &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the [vm] spec on an mTLS-composed serve");

    // Poll until Running (the spin guest never exits on its own) --
    // mirrors S-VM-05's own polling loop.
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
    // Settle window -- see the doc comment above for why the check
    // cannot fire the instant Running is first observed.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let nft_while_running = overdrive_mtls_nft_snapshot();
    assert_eq!(
        nft_before, nft_while_running,
        "MtlsInterceptWorker::start_alloc must NOT install any TPROXY rule for a \
         DriverType::Vm allocation -- the shared overdrive-mtls nft table changed while this \
         alloc was Running, which can only happen if the DriverType::Exec gate \
         (action_shim::mod.rs) let this Vm allocation through"
    );
    let listeners_while_running = loopback_tcp_listeners();
    assert_eq!(
        listeners_before, listeners_while_running,
        "MtlsInterceptWorker::start_alloc must NOT bind a leg-F/leg-C intercept listener for \
         a DriverType::Vm allocation -- a new 127.0.0.1 TCP LISTEN socket appeared while this \
         alloc was Running, which can only happen if the DriverType::Exec gate let this Vm \
         allocation through"
    );

    // Hygiene, not a new acceptance claim (mirrors S-VM-05's own fix,
    // commit 28dbefdc): the spin guest never beacons EXIT, so drive it
    // to Terminated through the production stop verb before this test
    // tears down the server, rather than orphaning its cloud-hypervisor
    // process (`Command::kill_on_drop` is deliberately `false` on the
    // production spawn, `crates/overdrive-host/src/vmm.rs`).
    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop the long-lived spinmtls workload before shutdown");
    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    let row = out.snapshot.rows.first().expect("one allocation row for the stopped workload");
    assert_eq!(
        row.state,
        AllocStateWire::Terminated,
        "a Vm allocation on an mTLS-composed serve must stop cleanly, unaffected by the \
         Exec-only MtlsInterceptWorker gate, got {:?} (reason={:?})",
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
///
/// Matches on `argv[0]` via `/proc/<pid>/cmdline`, NOT `/proc/<pid>/comm`
/// -- the SAME `TASK_COMM_LEN`=16 truncation bug [`find_cloud_hypervisor_pid`]
/// above was fixed to avoid. `comm` caps at 15 visible characters and
/// `"cloud-hypervisor"` is exactly 16, so a `comm`-based match against
/// the real binary can NEVER succeed: this helper was vacuously always
/// `true` regardless of whether a real VMM process remained, silently
/// masking the deadline-arm cleanup leak this file's own module doc
/// documents (01-08 review remediation).
fn no_cloud_hypervisor_process_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else { return true };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(_pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let cmdline_path = entry.path().join("cmdline");
        let Ok(cmdline) = std::fs::read(&cmdline_path) else { continue };
        let argv0 = cmdline.split(|&b| b == 0).next().unwrap_or(&[]);
        let argv0 = String::from_utf8_lossy(argv0);
        if Path::new(argv0.as_ref()).file_name() == Some(std::ffi::OsStr::new("cloud-hypervisor")) {
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

// ---------------------------------------------------------------------
// S-VM-11 — cloud-hypervisor present and healthy composes the Vm driver.
// ---------------------------------------------------------------------

/// S-VM-11 — cloud-hypervisor present and healthy composes the `Vm`
/// driver entry: a real substrate (no injection) boot accepts a `[vm]`
/// deploy. The registry reporting `DriverType::Vm` as supported is
/// observable ONLY through deploy-acceptance at the CLI driving port —
/// had the composition gate NOT composed a `Vm` entry, this exact deploy
/// would instead reach S-VM-12's dispatch-time-fallback classification
/// (`DriverError::StartRejected` → `Failed`), never `Inserted`.
#[tokio::test]
#[serial(cgroup)]
async fn vm_registry_reports_vm_supported_when_cloud_hypervisor_present_and_healthy() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let exit0 = build_exit_code_binary(tmp.path(), 0);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit0, "exit0vm11");

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-registry-supported.toml",
        &vm_job_toml(
            "vm-registry-supported",
            "/sbin/exit0vm11",
            &[],
            &fixture.kernel_path,
            &rootfs,
        ),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() }).await.expect(
        "a [vm] deploy must be ACCEPTED when cloud-hypervisor is present and healthy -- the \
             composition gate composed a Vm driver entry",
    );
    assert_eq!(
        submit.outcome,
        IdempotencyOutcome::Inserted,
        "the driver registry reporting DriverType::Vm as supported is proven by deploy \
         acceptance -- a rejected/failed deploy would mean no Vm entry was composed"
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-12 — cloud-hypervisor absent: no Vm entry, deploy classified
// naming the capability.
// ---------------------------------------------------------------------

/// Every directory in `PATH` EXCEPT the one containing `cloud-hypervisor`
/// — mirrors `overdrive-host`'s
/// `vmm_equivalence.rs::path_without_cloud_hypervisor` exactly (same
/// technique, duplicated here rather than shared since neither crate
/// exposes it as a reusable test helper). S-VM-12's "host with no
/// cloud-hypervisor binary installed" is this REAL `PATH`-resolution
/// fact, not a `SimVmm` stand-in — `cloud-hypervisor` genuinely cannot be
/// `exec`'d by this process for the duration of the mutation.
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

/// S-VM-12 — cloud-hypervisor absent: the node boots successfully with
/// no `Vm` entry in the driver registry, and a subsequent `[vm]` deploy
/// is not silently accepted-and-hung — it is classified `Failed`, naming
/// the absent capability, not a parse error.
///
/// **Honest scope note.** This proves the CURRENTLY-SHIPPED dispatch-time
/// fallback (`action_shim::mod.rs`'s `drivers.get(driver_kind) -> None`
/// arm — itself commented "SD-5's admission-time capability gate (step
/// 01-09) is a separate, earlier check; this is the dispatch-time
/// fallback for whatever reaches here regardless"): the deploy is
/// ACCEPTED at admission (`IdempotencyOutcome::Inserted`) and the
/// allocation transitions Pending → Failed at SCHEDULING/DISPATCH time.
/// It does NOT prove a hard admission-time (pre-`Inserted`) rejection —
/// building that would require a NEW capability check in the HTTP
/// submission handler (`handlers.rs::submit_workload`, plus `AppState`
/// widening to carry the `DriverRegistry`), which sits outside this
/// step's declared `implementation_scope`. See this step's final report
/// for the explicit gap flag.
#[tokio::test]
#[serial(cgroup, env)]
async fn vm_absent_boots_node_with_no_vm_entry_and_classifies_deploy_naming_capability() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let exit0 = build_exit_code_binary(tmp.path(), 0);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit0, "exit0vm12");

    let original_path = std::env::var_os("PATH");
    let broken_path = path_without_cloud_hypervisor();
    // SAFETY: `#[serial(env)]` guarantees exclusive access to `PATH` for
    // the duration of this test.
    unsafe {
        std::env::set_var("PATH", &broken_path);
    }

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;

    // SAFETY: restoring the pre-test PATH; still inside the
    // `#[serial(env)]` window. Safe to restore NOW — the composition
    // root's discover/probe already ran SYNCHRONOUSLY inside
    // `spawn_vm_server(...).await` above (the driver registry is fully
    // decided by the time that call returns); the `deploy()` call below
    // never touches `PATH`.
    unsafe {
        match &original_path {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }
    }

    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-absent.toml",
        &vm_job_toml("vm-absent", "/sbin/exit0vm12", &[], &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() }).await.expect(
        "deploy of a [vm] spec must be ACCEPTED at admission even when no Vm driver is \
             composed -- SD-5: capability absence is not a parse error",
    );
    assert_eq!(
        submit.outcome,
        IdempotencyOutcome::Inserted,
        "today's shipped behavior: the spec parses and is admitted; absence is classified at \
         dispatch time, not rejected as a malformed spec"
    );

    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    let row = out.snapshot.rows.first().expect("one allocation row for a freshly-deployed job");
    assert_eq!(
        row.state,
        AllocStateWire::Failed,
        "a [vm] deploy on a node with NO Vm driver entry must reach Failed (never hang Pending \
         forever, never silently succeed), got {:?}",
        row.state,
    );
    let reason_text = row.reason.as_ref().map(TransitionReason::human_readable).unwrap_or_default();
    assert!(
        reason_text.contains("vm")
            && reason_text.contains("no")
            && reason_text.contains("composed"),
        "the classification must NAME the absent capability (\"no vm driver composed on this \
         node\"), not a generic/unnamed failure -- got reason={:?}",
        row.reason,
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// S-VM-13 / S-VM-75 — genuine substrate lies refuse the boot, injected
// via ServerConfig.vmm_override.
// ---------------------------------------------------------------------

/// Spawns a real in-process `overdrive serve` with the given real VM
/// boot artifacts composed AND `vmm_override` set (ADR-0083 §D8, step
/// 01-09) — the composition root's `discover -> probe -> insert`
/// sequence resolves `vmm` first, then calls `.probe()` UNCONDITIONALLY
/// against it, exactly as it does for the production
/// `CloudHypervisorVmm`. `SimDataplane`-composed (not mTLS), matching
/// [`spawn_vm_server`] — S-VM-13/S-VM-75 are boot-refusal scenarios that
/// need no mesh composition. Returns `Result` (not `.expect()`-unwrapped)
/// since both callers assert on the `Err` arm.
async fn spawn_vm_server_with_vmm_override(
    vm_artifacts: VmBootArtifacts,
    vmm_override: std::sync::Arc<dyn overdrive_core::traits::vmm::Vmm>,
) -> Result<(ServeHandle, TempDir), overdrive_cli::http_client::CliError> {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = ServeArgs { bind, data_dir, config_dir };
    let result = overdrive_cli::commands::serve::run_with_dataplane_and_vmm_override(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
        vm_artifacts,
        vmm_override,
    )
    .await;
    result.map(|handle| (handle, tmp))
}

/// S-VM-13 — cloud-hypervisor present but a capability the host cannot
/// supply is missing: the node refuses to boot with a
/// `health.startup.refused`-shaped event naming the probe. Injected via
/// `ServerConfig.vmm_override` (ADR-0083 §D8) — a `SimVmm` carrying a
/// capability-flag probe fault, declaring "cloud-hypervisor IS present"
/// so the composition root's `VmComposeError::Refused` (hard-refusal)
/// path fires, never `NotAvailable` (capability-ABSENCE soft-skip,
/// S-VM-12's path). Per S-VM-13's own crafter note, this fault class has
/// no genuinely-lying real host in the Lima/metal test envelope — the
/// injection is the sanctioned mechanism (ADR-0083 §D8's own ruling).
#[tokio::test]
#[serial(cgroup)]
async fn vm_capability_flag_probe_failure_injected_via_vmm_override_refuses_boot() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");

    let sim_vmm = SimVmm::new();
    sim_vmm.inject_probe_failure(SimVmmProbeFault::LandlockLsmAbsent);

    let result = spawn_vm_server_with_vmm_override(
        VmBootArtifacts {
            kernel_path: fixture.kernel_path.clone(),
            rootfs_path: fixture.rootfs_path.clone(),
        },
        std::sync::Arc::new(sim_vmm),
    )
    .await;

    let err = result.expect_err(
        "a boot against an INJECTED vmm carrying a capability-flag probe fault must be REFUSED \
         -- Earned Trust ran .probe() unconditionally against the injected adapter and it failed",
    );
    // `err` here is `overdrive_cli::http_client::CliError::Transport { cause:
    // String, .. }` -- `run_inner` (overdrive-cli/src/commands/serve.rs)
    // flattens EVERY `ControlPlaneError` variant (not just `VmmBoot`) into
    // that `cause: String` field before this test ever sees it, so a
    // `matches!()` on the underlying `error::VmmBootError::Probe { source:
    // VmmProbeError::LandlockLsmAbsent { .. } }` variant is unreachable at
    // this boundary without touching `overdrive-cli` production code (out
    // of step 01-09's declared scope). The STRUCTURAL proof that
    // `compose_vm_driver` wires this into a distinct typed variant (D1
    // review remediation) lives in
    // `overdrive-control-plane::tests::vm_compose_error_typing::
    // injected_vmm_probe_failure_is_refused_with_typed_probe_variant` --
    // this scenario keeps its Display-based assertion as the honest E2E
    // proof of the composed boot path at the layer where only `Display`
    // is observable.
    let rendered = err.to_string();
    assert!(
        rendered.contains("VM driver probe refused") && rendered.contains("Landlock"),
        "the boot refusal must surface a message naming the probe failure, mirroring \
         MtlsEnforcement::probe's refusal shape -- got: {rendered}"
    );
}

/// S-VM-75 — cloud-hypervisor present, capability flags all satisfied,
/// but the VM staging directory is genuinely non-reflink: the node
/// refuses to boot via an EXECUTED FICLONE ioctl, never an fstype string
/// comparison. Uses a REAL `CloudHypervisorVmm` (not `SimVmm`)
/// constructed with its own test-only `.with_image_dir(...)` builder
/// pointed at a REAL tmpfs directory (`/dev/shm`, guaranteed tmpfs on
/// Linux — never `tmpdir_in(shared_staging_root())`, which is the
/// XFS-backed reflink-capable root every OTHER scenario in this file
/// deliberately uses), injected through the SAME `vmm_override` seam
/// S-VM-13 uses — the seam is `Arc<dyn Vmm>`, adapter-agnostic, and a
/// REAL differently-configured adapter satisfies it exactly as a
/// fault-injecting `SimVmm` does.
#[tokio::test]
#[serial(cgroup)]
async fn vm_non_reflink_staging_directory_refuses_boot_via_executed_ficlone() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");

    let non_reflink_dir = tempfile::Builder::new()
        .prefix("overdrive-vm-ws-s-vm-75-")
        .tempdir_in("/dev/shm")
        .expect("create a tmpfs probe-image-dir under /dev/shm (guaranteed tmpfs on Linux)");
    let real_vmm_non_reflink =
        CloudHypervisorVmm::new().with_image_dir(non_reflink_dir.path().to_path_buf());

    let result = spawn_vm_server_with_vmm_override(
        VmBootArtifacts {
            kernel_path: fixture.kernel_path.clone(),
            rootfs_path: fixture.rootfs_path.clone(),
        },
        std::sync::Arc::new(real_vmm_non_reflink),
    )
    .await;

    let err = result.expect_err(
        "a boot whose REAL CloudHypervisorVmm probes a genuinely non-reflink tmpfs directory \
         must be REFUSED -- an executed FICLONE ioctl against tmpfs returns EOPNOTSUPP/ENOTTY, \
         never an fstype string comparison",
    );
    // Same `CliError::Transport { cause: String }` boundary as S-VM-13
    // above -- see that scenario's comment for why this stays
    // Display-based. The structural `matches!()` proof for the
    // `ReflinkUnsupported` source (via the SAME `VmmBootError::Probe`
    // variant) is out of reach here since `overdrive-cli` re-stringifies
    // every `ControlPlaneError` before this test observes it; a real
    // `CloudHypervisorVmm` probing a genuinely non-reflink directory is
    // exercised structurally at the control-plane layer only through the
    // `SimVmm`-injected sibling tests in
    // `overdrive-control-plane::tests::vm_compose_error_typing` (the
    // `VmmProbeError::ReflinkUnsupported` source itself is adapter-real
    // only — `SimVmm` cannot fabricate it — so this real-substrate E2E
    // scenario remains the sole proof of THIS specific source class; its
    // Display assertion is what is available at this boundary).
    let rendered = err.to_string();
    assert!(
        rendered.contains("VM driver probe refused") && rendered.contains("reflink"),
        "the boot refusal must name ReflinkUnsupported -- got: {rendered}"
    );
}

// ---------------------------------------------------------------------
// S-VM-19 — a genuine cgroup OOM is diagnosed as VmOutOfMemory, never a
// bare signal 9.
// ---------------------------------------------------------------------

/// Cross-builds a tiny static-musl binary that repeatedly grows and
/// touches an anonymous buffer without bound — a "memory hog" that keeps
/// consuming and dirtying real pages until killed. Mirrors
/// `build_spin_binary`'s cross-build shape. `buf.resize(..., 0xAB)`
/// forces every new byte's page to be written (genuinely committed,
/// never a lazy zero-page); the explicit stride-touch loop afterward is
/// cheap insurance against any future stdlib fast-path.
fn build_memory_hog_binary(tmp: &Path) -> PathBuf {
    let src = tmp.join("memhog.rs");
    std::fs::write(
        &src,
        "fn main() {\n\
         \x20   let mut buf: Vec<u8> = Vec::new();\n\
         \x20   let chunk: usize = 2 * 1024 * 1024;\n\
         \x20   loop {\n\
         \x20       let start = buf.len();\n\
         \x20       buf.resize(start + chunk, 0xABu8);\n\
         \x20       let mut i = start;\n\
         \x20       while i < buf.len() {\n\
         \x20           buf[i] = buf[i].wrapping_add(1);\n\
         \x20           i += 4096;\n\
         \x20       }\n\
         \x20       std::thread::sleep(std::time::Duration::from_millis(100));\n\
         \x20   }\n\
         }\n",
    )
    .expect("write memhog source");
    let out = tmp.join("memhog");
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
        .expect("spawn rustc for the memory-hog binary");
    assert!(status.success(), "rustc must build the memory-hog binary");
    out
}

/// S-VM-19 — A VM that exceeds its declared memory is diagnosed as OOM,
/// not a bare signal 9.
///
/// Forces a REAL, genuine kernel cgroup-OOM (per this step's "Ground the
/// premise" obligation — the diagnosis must observe a REAL kernel
/// `memory.events` increment, never a synthesized classification). The
/// deploy declares `memory_bytes` at the SAME 128 MiB every other
/// scenario in this file uses (empirically the smallest point this
/// kernel/rootfs/CH combination is proven to boot at — a first cut of
/// this test at 64 MiB never reached Running within 60s on the real
/// metal box: the guest kernel's OWN boot footprint, before
/// `overdrive-init` ever dials the beacon, did not fit; that failure is
/// orthogonal to this scenario's actual claim, which is about a POST-boot
/// cgroup ceiling breach). Once the allocation is confirmed Running (its
/// cgroup scope exists, and the guest has ALREADY booted successfully
/// under the full, proven 128 MiB budget), this test DIRECTLY overwrites
/// the REAL `<scope>/memory.max` pseudo-file to a much tighter value
/// than what `MemoryPlan` computed — reproducing the D-3 "wrong
/// `reserve_bytes`" state the diagnosis exists to catch (ADR-0082
/// §D2.3's own bug report: a cgroup-OOM under a wrong `reserve_bytes`
/// "surfaces as `Failed / WorkloadCrashedImmediately {signal: 9}`,
/// indistinguishable from `kill -9`"). This is a REAL host-kernel
/// perturbation (the SAME class of direct real-substrate action
/// `.claude/rules/testing.md`'s fault-injection catalogue already
/// sanctions — `tc qdisc … netem`-style, applied to cgroupfs instead of
/// the network), not a fake at any layer of the diagnosis code path
/// itself: the memory-hog guest, already Running and growing without
/// bound WITHIN its own 128 MiB guest-visible RAM ceiling (comfortably
/// under it — the artificially tightened 24 MiB host ceiling, applied
/// AFTER boot, is what actually bites), genuinely breaches the tightened
/// ceiling and the REAL kernel OOM-kills the confined `cloud-hypervisor`
/// process — `memory.events`'s `oom_kill` counter is a REAL,
/// kernel-incremented fact by the time the exit watcher reads it.
///
/// `limit_bytes` on the resulting `TransitionReason::VmOutOfMemory` is
/// asserted against `MemoryPlan::derive(declared).cgroup_max_bytes()`
/// (the ORIGINALLY-COMPUTED ceiling `VmDriver` captured at `start` — per
/// ADR-0082 §D8, "costs no I/O", never a live re-read of the
/// artificially-tightened value this test wrote afterward).
#[tokio::test]
#[serial(cgroup)]
async fn vm_that_exceeds_declared_memory_is_diagnosed_as_oom_not_bare_signal_9() {
    const DECLARED_MEMORY_BYTES: u64 = 134_217_728; // 128 MiB -- matches every other scenario's proven-to-boot budget
    const TIGHTENED_MEMORY_MAX_BYTES: u64 = 24 * 1024 * 1024; // 24 MiB

    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-ws-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let memhog = build_memory_hog_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &memhog, "memhog");

    let (handle, server_tmp) = spawn_vm_server(VmBootArtifacts {
        kernel_path: fixture.kernel_path.clone(),
        rootfs_path: rootfs.clone(),
    })
    .await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-oom.toml",
        &format!(
            "[job]\nid = \"vm-oom\"\n\n[vm]\ncommand = \"/sbin/memhog\"\nargs = []\n\
             kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\n\
             memory_bytes = {DECLARED_MEMORY_BYTES}\n",
            fixture.kernel_path.display(),
            rootfs.display(),
        ),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the [vm] spec whose guest deliberately exceeds a tightened memory.max");

    // Poll until Running -- the cgroup scope must exist before this test
    // can tighten its real memory.max file. Captures the server-echoed
    // alloc_id at the same instant, per S-VM-14's established pattern
    // (never hand-assume the "alloc-<name>-0" naming convention).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let running_alloc_id: String = loop {
        let out =
            describe(DescribeArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
                .await
                .expect("workload describe must succeed while polling");
        if let Some(row) = out.snapshot.rows.first()
            && row.state == AllocStateWire::Running
        {
            break row.alloc_id.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "allocation must reach Running within 60s so its cgroup scope exists"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let alloc = AllocationId::new(&running_alloc_id)
        .expect("server-echoed alloc_id parses as AllocationId");

    // Force a REAL undersized memory.max -- reproducing the D-3 "wrong
    // reserve_bytes" state directly against the real kernel pseudo-file
    // production already wrote (MemoryPlan::cgroup_max_bytes()).
    let scope_memory_max =
        CgroupPath::for_alloc(&alloc).resolve(Path::new("/sys/fs/cgroup")).join("memory.max");
    std::fs::write(&scope_memory_max, TIGHTENED_MEMORY_MAX_BYTES.to_string()).unwrap_or_else(
        |err| {
            panic!(
                "write a tightened real memory.max at {}: {err} -- the alloc's cgroup scope must \
                 already exist (Running was confirmed above)",
                scope_memory_max.display()
            )
        },
    );

    let out = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let row = out.snapshot.rows.first().expect("one allocation row for a freshly-deployed job");
    assert_eq!(
        row.state,
        AllocStateWire::Failed,
        "a genuinely cgroup-OOM-killed VM must reach Failed (a crash), never Terminated, got {:?} \
         (reason={:?})",
        row.state,
        row.reason,
    );
    let expected_limit_bytes = MemoryPlan::derive(DECLARED_MEMORY_BYTES).cgroup_max_bytes();
    match &row.reason {
        Some(TransitionReason::VmOutOfMemory { limit_bytes, oom_kill_count }) => {
            assert_eq!(
                *limit_bytes, expected_limit_bytes,
                "limit_bytes must be MemoryPlan::cgroup_max_bytes() -- the ORIGINALLY-COMPUTED \
                 ceiling VmDriver captured at start, never this test's artificially-tightened value"
            );
            assert!(
                *oom_kill_count > 0,
                "oom_kill_count must be a REAL, kernel-incremented positive fact -- got 0"
            );
        }
        other => panic!(
            "a VM whose real, artificially-tightened memory.max was genuinely breached must be \
             diagnosed TransitionReason::VmOutOfMemory{{limit_bytes, oom_kill_count}}, NEVER a \
             bare signal 9 / WorkloadCrashedImmediately -- got {other:?}"
        ),
    }

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// Step 03-07 — RED scaffold (S-VM-54): the artifacts are the allocation's
// own, and there is no node-level artifact seam left to supply them.
//
// Shape per `.claude/rules/testing.md` § "RED scaffolds and
// intentionally-failing commits": `#[should_panic(expected = "RED
// scaffold")]` plus a panic body naming the scenario. The bar stays green
// while the pending scenario stays discoverable via `grep -rn
// 'should_panic.*RED scaffold' crates/`, and deleting the `panic!` without
// writing the assertions trips the attribute rather than passing silently.
//
// It carries `#[test]` TODAY, deliberately: the body is a single panic, so
// it awaits nothing, boots no server and touches no cgroup.
// `#[tokio::test]` and `#[serial(cgroup)]` are claims about what a body
// DOES, and this body makes neither yet. The rustdoc below names the
// attributes the activated form must carry, so the swap happens with the
// assertions and not before.
//
// The nine activated scenarios above are untouched.
// ---------------------------------------------------------------------

/// S-VM-54 / `@contract-shape:bounded-change` `@walking_skeleton`
/// `@happy_path` `@ac-21` `@tier3` `@real-io` `@requires-kvm` `@kpi:K4` —
/// two VM jobs deployed to ONE running `overdrive serve`, naming DIFFERENT
/// rootfs images, both reach Running, and each boots from the image its own
/// spec named.
///
/// ```gherkin
/// Given a kernel and two distinct ext4 rootfs images are staged on the host in
///   separate directories, each guest identifiably reporting which image it booted
/// And "overdrive serve" is running with no kernel or rootfs configured anywhere
///   in its arguments, environment or configuration files
/// And Ana has written two specs, each declaring [job] and [vm], naming the same
///   kernel and a different one of the two rootfs images
/// When she runs "overdrive deploy" for each of them
/// Then both workloads are accepted and both VM allocations reach Running through
///   the production VmDriver path
/// And each allocation booted from the rootfs image its own spec named
/// ```
///
/// Two claims, and the second is the reason there are two specs. The
/// **structural** claim is that reaching Running at all is impossible unless
/// the spec's own paths were read: after 03-07 there is no other source of a
/// kernel or rootfs path anywhere in the process. The **regression** claim is
/// the one a single-spec test would pass vacuously against — a re-introduced
/// node-level default would still boot ONE allocation from ONE image and look
/// entirely correct; only two allocations demanding two different images can
/// tell "the driver read this allocation's payload" apart from "the node had a
/// template that happened to match".
///
/// This is the in-tree companion to verification expectation
/// `E06-vm-job-deploy-reaches-running`, which asks the strictly harder
/// question (the shipped binary's own argv, out of process, default features)
/// and is K4's instrument. Keep the two consistent: if this passes and E06
/// does not, the difference is the in-process seam and is itself the finding.
///
/// # Why this cannot be written before 03-07
///
/// Every server helper in this file — [`spawn_vm_server`],
/// [`spawn_vm_server_mtls_composed`], [`spawn_vm_server_with_vmm_override`] —
/// takes a `VmBootArtifacts`, the node-level artifact seam DWD-25 **deletes**
/// (along with `ServerConfig.vm_artifacts`,
/// `run_with_dataplane_and_vm_artifacts` and `run_with_vm_artifacts`). This
/// scenario's premise is that no such seam exists, so it has no honest
/// composition to drive today: composing through the seam would prove the
/// opposite of what it claims. The scaffold therefore stops at the panic
/// rather than reaching for the API 03-07 is going to remove — or inventing
/// the one it will leave behind.
///
/// # Activation plan (step 03-07)
///
/// **Attributes**: swap `#[test]` for `#[tokio::test]` + `#[serial(cgroup)]`.
/// This scenario boots a real `serve` against the real host cgroupfs and
/// creates TWO allocation-scoped cgroup scopes, so the serialisation is
/// load-bearing rather than conventional. No `.config/nextest.toml` change is
/// needed: that file already routes this whole module into the
/// `host-kernel-shared` single-writer group by MODULE filter
/// (`test(vm_walking_skeleton)`), and `tests/integration.rs` already declares
/// the module.
///
/// **Composition**: drive whichever **ungated** `serve` entrypoint survives
/// 03-07's deletion. 03-07 owns naming it; do not invent one here, and do not
/// reach for `run_with_dataplane_and_vm_artifacts` or `run_with_vm_artifacts`
/// — both are gone. Whatever the surviving helper is, it must take NO artifact
/// argument: that absence IS the scenario's `Given`.
///
/// **Reuse as-is**: `shared_staging_root`, `VmFixture::provision` (the shared
/// kernel — both specs name the SAME kernel, so one copy suffices),
/// [`build_spin_binary`], [`stage_rootfs_with_extra_binary`], [`write_toml`],
/// [`config_path`], [`vm_job_toml`] (it already emits `kernel = ` and
/// `rootfs = ` inside `[vm]`, so no new spec builder is needed), `deploy`,
/// `stop`, and [`poll_until_terminal`].
///
/// # The distinct-parent-directory requirement is STRUCTURAL, not stylistic
///
/// The two rootfs masters MUST be staged in two different parent directories.
/// `RootfsPlan::for_alloc` derives the per-launch clone destination as
/// `<master_dir>/.overdrive-vm-rootfs-<alloc>.img` — it does **not** encode
/// the master's own filename. Two masters sharing one parent therefore produce
/// two clone paths that differ ONLY by allocation id, which is exactly what a
/// node-level default would also produce: the assertion could not discriminate
/// "each allocation booted the image its own spec named" from "both
/// allocations booted one node-wide image", and the scenario would pass
/// vacuously against the very regression it exists to catch. Two parents make
/// the two clone paths differ in their DIRECTORY component, which no
/// single-master node default can imitate.
///
/// The fixture shape falls out of this and is enforced by a helper this file
/// already has: [`stage_rootfs_with_extra_binary`] writes its per-test copy to
/// the fixed name `tmp.join("rootfs.ext4")`, so staging two images requires two
/// `tempdir_in(shared_staging_root())` roots regardless — calling it twice
/// against one `tmp` would silently overwrite the first image with the second.
///
/// **Still to build**:
///
/// * Two staging roots, each its own `tempdir_in(shared_staging_root())` (never
///   tmpfs — cloud-hypervisor disk I/O needs `O_DIRECT`), each carrying one
///   rootfs copy.
/// * A **second, in-guest** discriminator, cheap because the fixture already
///   supports it: give each image a DIFFERENT injected guest binary name
///   (`stage_rootfs_with_extra_binary`'s `guest_name`, e.g. `/sbin/spinone`
///   and `/sbin/spintwo`) and have each spec name only its own. Each per-test
///   rootfs is a copy of the shared fixture image, which carries only
///   `overdrive-init` — so neither image contains the other's binary. If a
///   node-level default forced both allocations onto one image, the mismatched
///   allocation's guest would beacon READY, fail its exec and power down, and
///   could not STAY Running. That is the Gherkin's "each guest identifiably
///   reporting which image it booted", and it is independent of the host-side
///   path check below.
/// * A `poll_until_running(cfg, id, max_wait)` helper. This file polls for
///   Running inline in three places already (S-VM-05, S-VM-09, S-VM-74) and a
///   fourth and fifth copy for two allocations would be four hand-written loops
///   free to drift on their row-selection rule and timeout message. Prefer the
///   shape `vm_boot_failure_vocabulary.rs` settled on: one
///   `poll_until_state(cfg, id, wanted, max_wait)` that a `poll_until_running`
///   and the existing [`poll_until_terminal`] both delegate to.
/// * A **per-allocation** hypervisor finder. [`find_cloud_hypervisor_pid`]
///   CANNOT be reused: its own doc comment pins the assumption "exactly one VM
///   is booted at the time of the call", and this is the first scenario in the
///   file to run two concurrently — it would return whichever allocation's VMM
///   `/proc` yielded first. Use the allocation-scoped shape
///   `vm_boot_failure_vocabulary.rs` already proved out: scan `/proc` for the
///   `cloud-hypervisor` `argv[0]` (never the `TASK_COMM_LEN`-truncated
///   `comm`) whose full argv contains THIS allocation's
///   `VmRunDir::for_alloc(...)` path. It must be a file-local copy — sibling
///   test modules cannot see each other's private items, which is why
///   [`build_spin_binary`] is already duplicated across the two files.
///
/// **Assertions**:
///
/// * Both `deploy(..)` calls succeed against the SAME running server, and each
///   returns its own declared workload id. One `serve`, two deploys — a second
///   server would test two nodes, not one node reading two payloads.
/// * Both allocations reach `AllocStateWire::Running` within a ceiling
///   comfortably above the 30s boot deadline, so a pass means both guests
///   booted rather than the poll being generous.
/// * Each Running row carries `Some(TransitionReason::Started)` — the progress
///   marker production writes only on `start`'s beacon-win arm, so pinning it
///   states that a real guest dialled the beacon and excludes every named
///   failure cause at the same time (the form S-VM-39 settled on).
/// * **The discriminating assertion.** For each allocation independently:
///   `RootfsPlan::for_alloc(<the master THAT spec named>, <its size>,
///   &alloc).clone_dest()` exists, and the live hypervisor process found for
///   THAT allocation's run directory has that clone path in its argv. Then the
///   cross-check that makes it discriminating: allocation A's clone path is NOT
///   under image B's parent directory and vice versa, and neither allocation's
///   argv references the other's master. A node-level default cannot satisfy
///   both halves at once.
/// * Assert through the observable run directory and hypervisor argv only.
///   Never read `VmHostLayout` — after 03-07 it no longer carries the fields,
///   and reading platform-internal state would assert the implementation
///   rather than the operator-observable outcome.
///
/// **Hygiene, not an acceptance claim**: both guests are long-lived and never
/// beacon EXIT, so both workloads must be driven to Terminated through the
/// production `stop` verb (and confirmed there) BEFORE `handle.shutdown()`.
/// Production spawns the VMM with `kill_on_drop(false)`, so a still-Running
/// guest is orphaned by server teardown alone and contaminates the next
/// serialized test's `/proc` scan — the lesson S-VM-05 learned on the metal box
/// (commit `28dbefdc`), doubled here because there are two of them.
#[test]
#[should_panic(expected = "RED scaffold")]
fn two_vm_jobs_on_one_serve_each_boot_from_the_rootfs_their_own_spec_named() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-54 / step 03-07 -- two VM jobs deployed to ONE \
         running serve, naming rootfs images staged in two DIFFERENT parent directories, must \
         both reach Running and each boot from the image its own spec named, on a composition \
         with no node-level artifact seam)"
    );
}
