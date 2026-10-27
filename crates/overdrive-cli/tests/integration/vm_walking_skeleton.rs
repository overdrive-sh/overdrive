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
//! **The historical EBUSY blocker for mTLS-composed real-`EbpfDataplane`
//! tests is CLOSED (01-08 review remediation, third pass, 2026-08-14).**
//! Root cause was NOT a production double-attach:
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
//! the identical class of gap. The remaining real-XDP scenario S-VM-05
//! stays in that cross-process single-writer group.
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
//! process, but nextest runs each test as its own process, and only a subset of
//! this file's scenarios were in the `host-kernel-shared` cross-process
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
//! S-VM-05 is un-ignored; the module passes on the metal box with zero
//! live `cloud-hypervisor` processes remaining after the full suite run.
//!
//! **Historical S-VM-74 is superseded by guest-stack transparent mTLS step
//! 02-01.** Its former “VM allocation installs no mTLS intercept” contract was
//! intentionally deleted. S-GTI-01 and S-GTI-03 now prove the opposite current
//! contract on real metal: VM traffic crosses the direct host-TAP/shared-bridge
//! path, receives the production intercept, and reaches the peer as authenticated
//! TLS 1.3.
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
//! Every pre-#295 scenario that boots a VM now carries
//! a VM start leases the guest transport and takes READY from the node
//! beacon intake, which only that step wires, so these bodies are RED until
//! it lands and are activated by it. The GH
//! #295 bridge/TAP-era bodies (S-ND295-35's direct-host-TAP/shared-bridge
//! topology and descriptor-3 bodies, S-ND295-37's TAP double loss) were
//! deleted with the vsock attachment replacement (feature-delta § *Bridge-era
//! code for DELIVER to delete*); the vsock walking-skeleton cells (P6-S13..S15)
//! are DISTILL's re-derivation. S-ND295-45's per-thread launch filter stays. Every blocker this file's history above documents (vsock
//! EAFNOSUPPORT, the terminal-row misclassification, the XDP EBUSY race, and
//! S-VM-05's cross-test contamination) is CLOSED.
//!
//! **Step 01-10** added S-VM-09 (per-thread seccomp verification, the C-5
//! correction). D-295-R22 falsifies its claim that the thread-group leader
//! reports `SECCOMP_MODE_DISABLED`; S-ND295-45 retargets it as
//! [`every_cloud_hypervisor_thread_carries_the_launch_filter_under_its_own_filters`].
//!
//! **Step 03-07** adds a 10th scenario, S-VM-54 (DWD-25 / AC-21), and
//! deletes the node-level artifact seam every server helper here used to
//! compose through. No helper below takes an artifact argument any more:
//! each scenario's kernel and rootfs reach the driver through the `[vm]`
//! block of the spec it deploys — which is what a real operator writes,
//! so these tests became MORE production-faithful, not less. S-VM-54 is
//! the scenario whose whole claim is that no node-level seam exists: two
//! specs naming rootfs images in two DIFFERENT parent directories, both
//! reaching Running on ONE `serve`, each booting the image its own spec
//! named.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::missing_panics_doc, clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::ServeHandle;
use overdrive_cli::commands::workload::{DescribeArgs, WorkloadDescribeOutput, describe};
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
pub(super) fn shared_staging_root() -> PathBuf {
    overdrive_testing::vm_fixture::default_staging_root()
}

/// A server-side tempdir (holding `data/` + `conf/`) on the reflink-capable
/// staging root, NOT the system tmpdir. Required because each per-launch rootfs
/// clone is FICLONE'd into `clone_staging_dir(data_dir)`, and FICLONE is
/// intra-filesystem (ADR-0082 2026-08-18 fourth amendment): with `data_dir` on
/// tmpfs and the master on the xfs staging root, the clone would fail `EXDEV`
/// and the VM boot would refuse. Co-locating `data_dir` with the masters
/// respects the production invariant (one VM data partition holds both).
pub(super) fn server_tmp_on_staging_root() -> TempDir {
    tempfile::Builder::new()
        .prefix("vm-serve-")
        .tempdir_in(shared_staging_root())
        .expect("server tempdir on the reflink-capable staging root")
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
pub(super) fn stage_rootfs_with_extra_binary(
    tmp: &Path,
    fixture: &VmFixture,
    host_bin: &Path,
    guest_name: &str,
) -> PathBuf {
    stage_rootfs_with_extra_binaries(tmp, fixture, &[(host_bin, guest_name)])
}

/// Stage a per-test rootfs copy with multiple static guest binaries injected
/// at `/sbin/<guest_name>`. A VM-to-VM scenario may need both a long-lived
/// Service peer and a Job caller in the same image; keeping this composition
/// in one mount/copy/unmount operation prevents the second binary from being
/// accidentally omitted or from mutating the shared fixture image.
pub(super) fn stage_rootfs_with_extra_binaries(
    tmp: &Path,
    fixture: &VmFixture,
    binaries: &[(&Path, &str)],
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

    for (host_bin, guest_name) in binaries {
        let dest = mnt.join("sbin").join(guest_name);
        std::fs::copy(host_bin, &dest).expect("copy the extra binary into the mounted rootfs");
        let mut perms =
            std::fs::metadata(&dest).expect("stat the copied guest binary").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms).expect("chmod the copied guest binary executable");
    }

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

/// Spawns a real in-process `overdrive serve` through the UNGATED
/// `serve_fixture::serve` (`DataplaneChoice::Sim`) entrypoint,
/// injecting `SimDataplane` as the Service dataplane (functional correctness
/// scenarios — S-VM-01/02/03/04 do not need the real `EbpfDataplane`; that is
/// what [`spawn_vm_server_mtls_composed`] is for). No node-level artifact
/// argument anywhere: every VM allocation booted against this handle must
/// source its kernel and rootfs from its own `[vm]` spec.
async fn spawn_vm_server() -> (ServeHandle, TempDir) {
    let tmp = server_tmp_on_staging_root();
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = super::serve_fixture::serve_args(bind, data_dir, config_dir);
    let handle = super::serve_fixture::serve(args, super::serve_fixture::DataplaneChoice::Sim)
        .await
        .expect("serve must start");
    (handle, tmp)
}

/// Spawns a real in-process `overdrive serve` through
/// `serve_fixture::serve` with `DataplaneChoice::Host`: the bundle keeps the
/// production `HostDataplaneComposer`, exactly as the production `run` path
/// composes it (GH #248 / ADR-0074 trap): it builds the REAL `EbpfDataplane` for the
/// Service dataplane. Under D-295-R16 the mTLS intercept worker, its resolver,
/// the guest DNS owner, and the shared guest-network supervisor are composed on
/// every boot, whatever the Service dataplane (FD § "[REF] Serve-boundary ports
/// (D-295-R16) — ACCEPTED 2026-09-24"). VM allocations join that worker
/// through their host TAP on the node's shared bridge; the guest-stack metal
/// scenarios (S-ND295-01) own the end-to-end behavior proof.
pub(super) async fn spawn_vm_server_mtls_composed() -> (ServeHandle, TempDir) {
    let tmp = server_tmp_on_staging_root();
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = super::serve_fixture::serve_args(bind, data_dir, config_dir);
    let handle = super::serve_fixture::serve(args, super::serve_fixture::DataplaneChoice::Host)
        .await
        .expect("serve must start (host dataplane)");
    (handle, tmp)
}

pub(super) fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

// ---------------------------------------------------------------------
// Spec authoring + polling
// ---------------------------------------------------------------------

/// A `[job]`+`[vm]`+`[resources]` TOML — the shape `WorkloadSpecInput::
/// from_toml_str`'s job-family branch parses (confirmed GREEN by
/// S-VM-06/S-VM-07 in the VM parser acceptance suite).
pub(super) fn vm_job_toml(
    id: &str,
    command: &str,
    args: &[&str],
    kernel: &Path,
    rootfs: &Path,
) -> String {
    let args_toml = args.iter().map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(", ");
    format!(
        "[job]\nid = \"{id}\"\n\n[vm]\ncommand = \"{command}\"\nargs = [{args_toml}]\n\
         kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\n\
         memory_bytes = 134217728\n",
        kernel.display(),
        rootfs.display(),
    )
}

pub(super) fn write_toml(dir: &Path, name: &str, body: &str) -> PathBuf {
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
pub(super) async fn poll_until_terminal(
    cfg: &Path,
    workload_id: &str,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    poll_until_state(
        cfg,
        workload_id,
        |state| matches!(state, AllocStateWire::Terminated | AllocStateWire::Failed),
        "a terminal state",
        max_wait,
    )
    .await
}

/// [`poll_until_state`] specialised to `Running`. Multiple VM and guest-stack
/// scenarios share it so their row-selection rule and timeout diagnostics
/// cannot drift.
pub(super) async fn poll_until_running(
    cfg: &Path,
    workload_id: &str,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    poll_until_state(
        cfg,
        workload_id,
        |state| state == AllocStateWire::Running,
        "Running",
        max_wait,
    )
    .await
}

/// The ONE row-selection rule and the ONE timeout message every poller in
/// this file shares (the shape `vm_boot_failure_vocabulary.rs` settled on).
///
/// Fails fast on a terminal first row it does not want. An allocation never
/// leaves `Terminated` or `Failed` (`AllocState::is_terminal`; a restart is a
/// fresh allocation, never this row), and this poller reads only the first
/// row, so such a row can never become wanted. Waiting out `max_wait` would
/// only delay the same verdict (a baseline guest-boot failure otherwise cost
/// each body its full 90 s).
async fn poll_until_state(
    cfg: &Path,
    workload_id: &str,
    wanted: impl Fn(AllocStateWire) -> bool,
    wanted_label: &str,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let out =
            describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_owned() })
                .await
                .expect("workload describe must succeed while polling");
        if out.snapshot.rows.first().is_some_and(|row| wanted(row.state)) {
            return out;
        }
        if let Some(row) =
            out.snapshot.rows.first().filter(|row| {
                matches!(row.state, AllocStateWire::Terminated | AllocStateWire::Failed)
            })
        {
            panic!(
                "workload {workload_id} reached the terminal state {:?} and can never reach \
                 {wanted_label}; observed row: {row:?}",
                row.state,
            );
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workload {workload_id} did not reach {wanted_label} within {max_wait:?}; \
             last observed row: {:?}",
            out.snapshot.rows.first(),
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// How long a failed native body's teardown may run before [`TeardownBound`]
/// ends the test process.
const TEARDOWN_BOUND: Duration = Duration::from_secs(30);

/// Bounds the teardown of a native body that boots an in-process `serve` and
/// then fails.
///
/// After such a body panics, `#[tokio::test]` drops its runtime, and tokio's
/// runtime drop waits without limit for blocking-pool work. The in-process
/// serve's DNS responder loop runs there and stops only on the graceful
/// `ServeHandle::shutdown` the panicking body never reaches, so the test
/// otherwise hangs until nextest's `terminate-after` (600 s for the #295
/// native-fault modules, `.config/nextest.toml`).
///
/// Declare it as a body's FIRST statement so it drops last. When it drops
/// while the thread is unwinding, it arms a watchdog thread. After
/// [`TEARDOWN_BOUND`] the watchdog writes `cgroup.kill` for every workload
/// scope the body created (so no Cloud Hypervisor outlives the run; the
/// module is in `host-kernel-shared`, so no other test's scope can be new),
/// reports what it killed, and exits the process with libtest's failure
/// status 101. The panic message is already on stderr by then. A body that
/// passes, or whose teardown ends inside the bound, is unaffected: the
/// process exits first.
pub(super) struct TeardownBound {
    baseline: BTreeSet<String>,
}

impl TeardownBound {
    const WORKLOAD_SCOPES: &'static str = "/sys/fs/cgroup/overdrive.slice/workloads.slice";

    /// Records the workload scopes that exist before the body runs.
    pub(super) fn arm() -> Self {
        Self { baseline: Self::workload_scopes() }
    }

    fn workload_scopes() -> BTreeSet<String> {
        match std::fs::read_dir(Self::WORKLOAD_SCOPES) {
            Ok(entries) => entries
                .map(|entry| {
                    entry
                        .expect("read a workload scope entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .filter(|name| {
                    name.starts_with("alloc-")
                        && Path::new(name).extension().is_some_and(|extension| extension == "scope")
                })
                .collect(),
            Err(error) if error.kind() == ErrorKind::NotFound => BTreeSet::new(),
            Err(error) => panic!("read {}: {error}", Self::WORKLOAD_SCOPES),
        }
    }
}

impl Drop for TeardownBound {
    #[allow(
        clippy::print_stderr,
        reason = "the watchdog runs after the body panicked; stderr, which the harness captures, \
                  is its only channel for saying what it killed and why the process ends"
    )]
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        let baseline = std::mem::take(&mut self.baseline);
        std::thread::spawn(move || {
            std::thread::sleep(TEARDOWN_BOUND);
            let created: Vec<String> = match std::fs::read_dir(Self::WORKLOAD_SCOPES) {
                Ok(entries) => entries
                    .filter_map(|entry| match entry {
                        Ok(entry) => Some(entry.file_name().to_string_lossy().into_owned()),
                        Err(error) => {
                            eprintln!("teardown bound: read a workload scope entry: {error}");
                            None
                        }
                    })
                    .filter(|name| {
                        name.starts_with("alloc-")
                            && Path::new(name)
                                .extension()
                                .is_some_and(|extension| extension == "scope")
                            && !baseline.contains(name)
                    })
                    .collect(),
                Err(error) => {
                    eprintln!("teardown bound: cannot list {}: {error}", Self::WORKLOAD_SCOPES);
                    Vec::new()
                }
            };
            for scope in &created {
                let kill = Path::new(Self::WORKLOAD_SCOPES).join(scope).join("cgroup.kill");
                if let Err(error) = std::fs::write(&kill, "1") {
                    eprintln!("teardown bound: write {}: {error}", kill.display());
                }
            }
            eprintln!(
                "teardown bound: the failed body's teardown exceeded {TEARDOWN_BOUND:?} (the \
                 in-process serve was never shut down); killed the workload scopes it created: \
                 {created:?}; ending the test process"
            );
            std::process::exit(101);
        });
    }
}

/// The full NUL-joined argv of the live `cloud-hypervisor` process serving
/// THIS allocation, located by the allocation's own [`VmRunDir`] path.
///
/// [`find_cloud_hypervisor_pid`] cannot be used where more than one VM is
/// booted concurrently — its own contract pins "exactly one VM is booted at
/// the time of the call", and it would return whichever allocation's
/// `/proc` entry was yielded first. This is the allocation-scoped shape
/// `vm_boot_failure_vocabulary.rs` already proved out (a file-local copy —
/// sibling test modules cannot see each other's private items).
///
/// Matches on `argv[0]`, never the `TASK_COMM_LEN`-truncated
/// `/proc/<pid>/comm` (15 chars, shorter than the 16-char binary name).
pub(super) fn hypervisor_argv_for_alloc(alloc: &AllocationId) -> String {
    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), alloc);
    let needle = run_dir.path().to_string_lossy().into_owned();
    for entry_result in std::fs::read_dir("/proc").expect("read /proc") {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("read /proc entry while locating allocation {alloc}: {error}"),
        };
        if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
            continue;
        }
        let cmdline = match std::fs::read(entry.path().join("cmdline")) {
            Ok(cmdline) => cmdline,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("read /proc cmdline while locating allocation {alloc}: {error}"),
        };
        let argv0 = cmdline.split(|&byte| byte == 0).next().unwrap_or(&[]);
        let argv0 = String::from_utf8_lossy(argv0);
        if Path::new(argv0.as_ref()).file_name() != Some(std::ffi::OsStr::new("cloud-hypervisor")) {
            continue;
        }
        let argv = String::from_utf8_lossy(&cmdline).replace('\0', " ");
        if argv.contains(&needle) {
            return argv;
        }
    }
    panic!("no live cloud-hypervisor process found whose argv references {needle}")
}

// ---------------------------------------------------------------------
// GH #295 native observation helpers.
// ---------------------------------------------------------------------

/// The managed host TAP name the shared guest-network owner derives from a
/// guest address (`ovd-tp-<low 16 bits, hex>`).
fn managed_tap_name(address: Ipv4Addr) -> String {
    let octets = address.octets();
    format!("ovd-tp-{:04x}", u16::from_be_bytes([octets[2], octets[3]]))
}

/// The pid and NUL-joined argv of the live `cloud-hypervisor` process
/// serving `alloc`, located by the allocation's own [`VmRunDir`] path (the
/// same `argv[0]` rule as [`hypervisor_argv_for_alloc`]).
fn hypervisor_process_for_alloc(alloc: &AllocationId) -> (u32, String) {
    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), alloc);
    let needle = run_dir.path().to_string_lossy().into_owned();
    for entry_result in std::fs::read_dir("/proc").expect("read /proc") {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => panic!("read /proc entry while locating allocation {alloc}: {error}"),
        };
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let cmdline = match std::fs::read(entry.path().join("cmdline")) {
            Ok(cmdline) => cmdline,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => panic!("read /proc/{pid}/cmdline for allocation {alloc}: {error}"),
        };
        let argv0 = cmdline.split(|&byte| byte == 0).next().unwrap_or(&[]);
        let argv0 = String::from_utf8_lossy(argv0);
        if Path::new(argv0.as_ref()).file_name() != Some(std::ffi::OsStr::new("cloud-hypervisor")) {
            continue;
        }
        let argv = String::from_utf8_lossy(&cmdline).replace('\0', " ");
        if argv.contains(&needle) {
            return (pid, argv);
        }
    }
    panic!("no live cloud-hypervisor process found whose argv references {needle}")
}

/// The administrative (`IFF_UP`) state of a host interface from
/// `/sys/class/net/<name>/flags`; `Ok(None)` when the interface is absent.
/// Never panics, so the supervisor trace layer can call it from inside the
/// emitting task.
fn tap_admin_up(name: &str) -> std::io::Result<Option<bool>> {
    let path = Path::new("/sys/class/net").join(name).join("flags");
    match std::fs::read_to_string(&path) {
        Ok(flags) => {
            let flags = u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16)
                .map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))?;
            let up = u32::try_from(libc::IFF_UP).expect("IFF_UP is a positive flag bit");
            Ok(Some(flags & up != 0))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
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

    // Exercise the production composition root: every admitted VM receives
    // its guest transport assignment before the guest can cross READY.
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
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

    let (handle, server_tmp) = spawn_vm_server().await;
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

    let (handle, server_tmp) = spawn_vm_server().await;
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
/// `overdrive_cli::commands::deploy::deploy` handler as any other workload
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

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());

    let spec_path = write_toml(
        server_tmp.path(),
        "vm-same-verb.toml",
        &vm_job_toml("vm-same-verb", "/sbin/exit0", &[], &fixture.kernel_path, &rootfs),
    );
    // The SAME `deploy()` fn used by the other workload integration tests —
    // no VM-specific handler and no new CLI subcommand.
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg })
        .await
        .expect("deploy the [vm] spec through the existing workload verb");
    assert_eq!(submit.workload_id, "vm-same-verb");
    assert_eq!(
        submit.outcome,
        overdrive_control_plane::api::IdempotencyOutcome::Inserted,
        "a fresh [vm] deploy must report Inserted through the existing workload verb"
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
pub(super) fn find_cloud_hypervisor_pid() -> u32 {
    for entry_result in std::fs::read_dir("/proc").expect("read /proc") {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("read /proc entry while locating Cloud Hypervisor: {error}"),
        };
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let cmdline_path = entry.path().join("cmdline");
        let cmdline = match std::fs::read(&cmdline_path) {
            Ok(cmdline) => cmdline,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("read {}: {error}", cmdline_path.display()),
        };
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

    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
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
// S-ND295-45 — every Cloud Hypervisor thread carries the launch filter
// (retargets S-VM-09, whose leader-disabled claim D-295-R22 falsifies).
// ---------------------------------------------------------------------

/// The parsed `Seccomp:` mode from one `/proc/<pid>/task/<tid>/status`
/// file. `0` is `SECCOMP_MODE_DISABLED` (no filter installed on that
/// thread); a confined thread reports `2` (`SECCOMP_MODE_FILTER`, the mode
/// `--seccomp true` installs).
pub(super) fn thread_seccomp_mode(vmm_pid: u32, tid: &str) -> u32 {
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
pub(super) fn thread_names(vmm_pid: u32) -> std::collections::BTreeMap<String, String> {
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

/// `SECCOMP_MODE_FILTER` — the `Seccomp:` mode a thread reports once at
/// least one seccomp filter confines it.
const SECCOMP_MODE_FILTER: u32 = 2;

/// One numeric field (`NoNewPrivs:`, `Seccomp_filters:`) of
/// `/proc/<pid>/task/<tid>/status`.
fn thread_status_number(vmm_pid: u32, tid: &str, field: &str) -> u32 {
    let path = format!("/proc/{vmm_pid}/task/{tid}/status");
    let status = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    status.lines().find_map(|line| line.strip_prefix(field)).map_or_else(
        || panic!("no {field} field found in {path}"),
        |value| {
            value.trim().parse::<u32>().unwrap_or_else(|e| panic!("parse {field} in {path}: {e}"))
        },
    )
}

/// The `Seccomp_filters` count D-295-R22 pins for one Cloud Hypervisor v53
/// thread: CH's own filter count for that thread plus the one launch filter
/// every thread inherits from the launch child (increment-aa control table,
/// FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (row E21, case (e))). CH never filters its thread-group leader, so the leader's
/// count is the discriminating check.
fn expected_launch_filter_count(vmm_pid: u32, tid: &str, name: &str) -> u32 {
    if tid == vmm_pid.to_string() {
        1
    } else if matches!(name, "vmm" | "http-server") {
        2
    } else {
        3
    }
}

/// Reads every thread of `vmm_pid` and asserts D-295-R22's per-thread table:
/// `NoNewPrivs: 1`, `Seccomp: 2`, and the pinned `Seccomp_filters` count.
/// Requires the leader, `vmm`, `http-server`, and `vcpu0` threads so each of
/// the three filter-count classes is observed, never vacuously absent.
fn assert_every_thread_carries_the_launch_filter(vmm_pid: u32, phase: &str) {
    let names = thread_names(vmm_pid);
    let leader = vmm_pid.to_string();
    assert!(
        names.contains_key(&leader),
        "{phase}: the thread-group leader {leader} is listed in /proc/{vmm_pid}/task: {names:?}"
    );
    for required in ["vmm", "http-server", "vcpu0"] {
        assert!(
            names.values().any(|name| name == required),
            "{phase}: Cloud Hypervisor runs a thread named {required}; observed threads: {names:?}"
        );
    }
    let observed: BTreeMap<&String, (&String, u32, u32, u32, u32)> = names
        .iter()
        .map(|(tid, name)| {
            (
                tid,
                (
                    name,
                    thread_status_number(vmm_pid, tid, "NoNewPrivs:"),
                    thread_seccomp_mode(vmm_pid, tid),
                    thread_status_number(vmm_pid, tid, "Seccomp_filters:"),
                    expected_launch_filter_count(vmm_pid, tid, name),
                ),
            )
        })
        .collect();
    let violations: Vec<_> = observed
        .iter()
        .filter(|(_, (_, no_new_privs, mode, filters, expected))| {
            *no_new_privs != 1 || *mode != SECCOMP_MODE_FILTER || filters != expected
        })
        .collect();
    assert!(
        violations.is_empty(),
        "{phase}: every Cloud Hypervisor thread must report NoNewPrivs 1, Seccomp \
         {SECCOMP_MODE_FILTER}, and Seccomp_filters = leader 1 / vmm and http-server 2 / \
         every other thread 3; violations (tid -> (name, NoNewPrivs, Seccomp, Seccomp_filters, \
         expected filters)): {violations:?}; full table: {observed:?}"
    );
}

/// A counter of `/sys/class/net/<tap>/statistics/<counter>`.
fn tap_statistic(tap: &str, counter: &str) -> u64 {
    let path = Path::new("/sys/class/net").join(tap).join("statistics").join(counter);
    let value =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    value.trim().parse::<u64>().unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Sends `count` broadcast ARP probes for `guest` onto `tap` from the host
/// through an `AF_PACKET` socket (an ARP probe carries sender address
/// 0.0.0.0, so it seeds no ARP cache). A frame the TAP transmits is read by
/// the VMM's network queue, which is what `tx_packets` counts, so this is
/// host-to-guest traffic through the Cloud Hypervisor network device.
fn send_host_broadcast_arp_probes(tap: &str, guest: Ipv4Addr, count: usize) {
    const ETH_P_ARP: u16 = 0x0806;
    let name = std::ffi::CString::new(tap).expect("TAP name has no NUL");
    // SAFETY: `name` is a live NUL-terminated string for this call.
    let ifindex = unsafe { libc::if_nametoindex(name.as_ptr()) };
    assert_ne!(ifindex, 0, "host-to-guest traffic TAP {tap} exists");
    // SAFETY: a send-only AF_PACKET raw socket (protocol 0 registers no
    // receive hook); ownership moves into the OwnedFd immediately.
    let raw = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW | libc::SOCK_CLOEXEC, 0) };
    assert!(raw >= 0, "open AF_PACKET sender: {}", std::io::Error::last_os_error());
    // SAFETY: `raw` is a freshly created descriptor owned by nothing else.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    let source_mac = [0x02, 0x95, 0x02, 0x95, 0x00, 0x45];
    let mut frame = Vec::with_capacity(42);
    frame.extend_from_slice(&[0xff; 6]);
    frame.extend_from_slice(&source_mac);
    frame.extend_from_slice(&ETH_P_ARP.to_be_bytes());
    frame.extend_from_slice(&1_u16.to_be_bytes());
    frame.extend_from_slice(&0x0800_u16.to_be_bytes());
    frame.extend_from_slice(&[6, 4]);
    frame.extend_from_slice(&1_u16.to_be_bytes());
    frame.extend_from_slice(&source_mac);
    frame.extend_from_slice(&[0, 0, 0, 0]);
    frame.extend_from_slice(&[0; 6]);
    frame.extend_from_slice(&guest.octets());
    // SAFETY: zero is a valid initialization for sockaddr_ll before fields are populated.
    let mut address: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    address.sll_family = u16::try_from(libc::AF_PACKET).expect("AF_PACKET fits u16");
    address.sll_protocol = ETH_P_ARP.to_be();
    address.sll_ifindex = i32::try_from(ifindex).expect("ifindex fits i32");
    address.sll_halen = 6;
    address.sll_addr[..6].copy_from_slice(&[0xff; 6]);
    for _ in 0..count {
        // SAFETY: `frame` and `address` are live, fully initialized buffers of the
        // supplied lengths, and `socket` is an owned open descriptor.
        let sent = unsafe {
            libc::sendto(
                socket.as_raw_fd(),
                frame.as_ptr().cast(),
                frame.len(),
                0,
                std::ptr::from_ref(&address).cast(),
                libc::socklen_t::try_from(std::mem::size_of::<libc::sockaddr_ll>())
                    .expect("sockaddr_ll length fits socklen_t"),
            )
        };
        assert_eq!(
            usize::try_from(sent).ok(),
            Some(frame.len()),
            "send host broadcast ARP probe on {tap}: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-45 — Every Cloud Hypervisor thread carries the launch filter under its own filters
/// CONTRACT_SHAPE: bounded-change.
///
/// E21 native (e), per-thread half (D-295-R22, FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (the testability boundary: architecture gating); FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (row E21)). A VM
/// launched through `serve` and `deploy` runs with the launch seccomp filter
/// the launch child installs before its first exec, so every thread, the
/// thread-group leader included, reports `NoNewPrivs: 1` and `Seccomp: 2`,
/// and each thread's `Seccomp_filters` is the audited Cloud Hypervisor
/// build's own count plus one: leader 1, `vmm` and `http-server` 2, every
/// other thread 3. D-295-R22 makes the retired S-VM-09 claim (the leader
/// reports `SECCOMP_MODE_DISABLED`) false.
///
/// The body activates at DELIVER step 05-03, not 05-02 where the launch
/// filter lands: its precondition is a guest that reaches Running, which the
/// 05-03 descriptor handoff makes possible (before it, the named-TAP launch
/// fails with `Failed to open taps` / `EPERM`).
///
/// The table is read at READY (the allocation reports `Running` only after
/// the guest's READY beacon) and again after traffic, as E21 (e) requires:
/// the guest program transmits frames on its NIC and the host sends
/// broadcast ARP probes onto the TAP, and the read repeats only once the
/// TAP's `rx_packets` (guest to host) and `tx_packets` (host to guest, read
/// by the VMM's queue) have both advanced. Traffic delivery itself is
/// S-ND295-01's oracle.
#[expect(
    clippy::doc_markdown,
    reason = "CONTRACT_SHAPE is an exact repository-mandated machine-read declaration"
)]
#[tokio::test]
#[serial(cgroup)]
async fn every_cloud_hypervisor_thread_carries_the_launch_filter_under_its_own_filters() {
    let _teardown = TeardownBound::arm();
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-s45-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    // A long-lived guest that keeps transmitting frames on its NIC, so the
    // after-traffic read has guest-to-host traffic to wait for.
    let traffic = build_identifiable_datagram_emitter(tmp.path(), Ipv4Addr::new(100, 95, 0, 1));
    let rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &traffic, "nd295-s45-traffic");

    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "nd295-s45.toml",
        &vm_job_toml("nd295-s45", "/sbin/nd295-s45-traffic", &[], &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the [vm] spec through the production handler");
    let running = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(90)).await;
    let row = running.snapshot.rows.first().expect("one Running allocation row");
    let alloc = AllocationId::new(&row.alloc_id).expect("server-echoed alloc_id parses");
    let guest = row.workload_addr.expect("a Running VM publishes its address");
    let tap = managed_tap_name(guest);
    let (vmm_pid, _) = hypervisor_process_for_alloc(&alloc);

    assert_every_thread_carries_the_launch_filter(vmm_pid, "at READY");

    // Traffic: wait for the owner's activation to raise the TAP, then drive
    // host-to-guest frames until both directions have crossed the VMM queue.
    let activation_deadline = Instant::now() + Duration::from_secs(30);
    while tap_admin_up(&tap).expect("read managed TAP flags") != Some(true) {
        assert!(
            Instant::now() < activation_deadline,
            "the managed TAP {tap} is raised by activation within 30 s of Running"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let rx_before = tap_statistic(&tap, "rx_packets");
    let tx_before = tap_statistic(&tap, "tx_packets");
    let traffic_deadline = Instant::now() + Duration::from_secs(30);
    loop {
        send_host_broadcast_arp_probes(&tap, guest, 3);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let rx = tap_statistic(&tap, "rx_packets");
        let tx = tap_statistic(&tap, "tx_packets");
        if rx > rx_before && tx > tx_before {
            break;
        }
        assert!(
            Instant::now() < traffic_deadline,
            "guest-to-host (rx_packets {rx_before} -> {rx}) and host-to-guest (tx_packets \
             {tx_before} -> {tx}) frames both cross {tap} within 30 s"
        );
    }
    let (vmm_pid_after_traffic, _) = hypervisor_process_for_alloc(&alloc);
    assert_eq!(
        vmm_pid_after_traffic, vmm_pid,
        "the same Cloud Hypervisor process serves the allocation after traffic"
    );

    assert_every_thread_carries_the_launch_filter(vmm_pid, "after traffic");

    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop the long-lived guest before shutdown");
    let stopped = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    assert_eq!(
        stopped.snapshot.rows.first().expect("one allocation row for the stopped workload").state,
        AllocStateWire::Terminated,
        "an operator stop drives the never-self-terminating guest to Terminated before shutdown"
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------
// Shared long-lived guest fixture.
// ---------------------------------------------------------------------

/// Cross-builds a tiny static-musl binary that loops forever until killed.
/// The reliably-observable Running window is shared by scenarios that must
/// inspect a live Cloud Hypervisor process or a live allocation surface.
pub(super) fn build_spin_binary(tmp: &Path) -> PathBuf {
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

#[allow(
    clippy::too_many_lines,
    reason = "the emitted guest program is one inline source template; splitting the template across helpers would obscure the exact frames the native bodies count"
)]
pub(super) fn build_identifiable_datagram_emitter(
    tmp: &Path,
    target: std::net::Ipv4Addr,
) -> PathBuf {
    let src = tmp.join("shared-guest-network-emitter.rs");
    let target = target.octets();
    std::fs::write(
        &src,
        format!(
            r#"use std::ffi::CString;
use std::mem::size_of;
use std::thread;
use std::time::Duration;

#[repr(C)]
struct SockaddrLl {{
    family: u16,
    protocol: u16,
    ifindex: i32,
    hatype: u16,
    pkttype: u8,
    halen: u8,
    addr: [u8; 8],
}}

extern "C" {{
    fn if_nametoindex(name: *const i8) -> u32;
    fn socket(domain: i32, kind: i32, protocol: i32) -> i32;
    fn sendto(
        fd: i32,
        bytes: *const u8,
        length: usize,
        flags: i32,
        address: *const SockaddrLl,
        address_length: u32,
    ) -> isize;
}}

fn ipv4_udp(source_mac: [u8; 6], source_ip: [u8; 4], target_ip: [u8; 4]) -> Vec<u8> {{
    let marker = b"ND295-S37-IDENTIFIABLE";
    let total = 20 + 8 + marker.len();
    let mut frame = Vec::with_capacity(14 + total);
    frame.extend_from_slice(&[0x02, 0x00, {t0}, {t1}, {t2}, {t3}]);
    frame.extend_from_slice(&source_mac);
    frame.extend_from_slice(&0x0800_u16.to_be_bytes());
    frame.extend_from_slice(&[0x45, 0]);
    frame.extend_from_slice(&(total as u16).to_be_bytes());
    frame.extend_from_slice(&[0, 0, 0, 0, 64, 17, 0, 0]);
    frame.extend_from_slice(&source_ip);
    frame.extend_from_slice(&target_ip);
    frame.extend_from_slice(&40_037_u16.to_be_bytes());
    frame.extend_from_slice(&19_037_u16.to_be_bytes());
    frame.extend_from_slice(&((8 + marker.len()) as u16).to_be_bytes());
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(marker);
    frame
}}

fn arp_spoof(source_mac: [u8; 6], source_ip: [u8; 4], target_ip: [u8; 4]) -> Vec<u8> {{
    let mut frame = Vec::new();
    frame.extend_from_slice(&[0xff; 6]);
    frame.extend_from_slice(&source_mac);
    frame.extend_from_slice(&0x0806_u16.to_be_bytes());
    frame.extend_from_slice(&1_u16.to_be_bytes());
    frame.extend_from_slice(&0x0800_u16.to_be_bytes());
    frame.extend_from_slice(&[6, 4]);
    frame.extend_from_slice(&1_u16.to_be_bytes());
    frame.extend_from_slice(&source_mac);
    frame.extend_from_slice(&source_ip);
    frame.extend_from_slice(&[0_u8; 6]);
    frame.extend_from_slice(&target_ip);
    frame.extend_from_slice(b"ND295-S37-IDENTIFIABLE");
    frame
}}

fn main() {{
    const AF_PACKET: i32 = 17;
    const SOCK_RAW: i32 = 3;
    const ETH_P_ALL: u16 = 0x0003;
    let interface = CString::new("eth0").unwrap();
    let ifindex = unsafe {{ if_nametoindex(interface.as_ptr()) }};
    assert_ne!(ifindex, 0);
    let fd = unsafe {{ socket(AF_PACKET, SOCK_RAW, i32::from(ETH_P_ALL.to_be())) }};
    assert!(fd >= 0);
    let destination = [0x02, 0x00, {t0}, {t1}, {t2}, {t3}];
    let good_mac = [0x02, 0x00, 100, 95, 0, 3];
    let good_ip = [100, 95, 0, 3];
    let mut mac_spoof = ipv4_udp([0x02, 0, 1, 2, 3, 4], good_ip, [{t0}, {t1}, {t2}, {t3}]);
    let ip_spoof = ipv4_udp(good_mac, [100, 95, 0, 99], [{t0}, {t1}, {t2}, {t3}]);
    let udp_bypass = ipv4_udp(good_mac, good_ip, [{t0}, {t1}, {t2}, {t3}]);
    let arp_source_spoof = arp_spoof(good_mac, [100, 95, 0, 99], [{t0}, {t1}, {t2}, {t3}]);
    let mut non_ipv4 = udp_bypass.clone();
    non_ipv4[12..14].copy_from_slice(&0x86dd_u16.to_be_bytes());
    let malformed = udp_bypass[..18].to_vec();
    // Make the MAC-spoof case carry a byte-distinct marker copy while retaining
    // a well-formed IPv4/UDP envelope.
    mac_spoof.extend_from_slice(b"-MAC");
    let frames = [mac_spoof, ip_spoof, udp_bypass, arp_source_spoof, non_ipv4, malformed];
    let address = SockaddrLl {{
        family: AF_PACKET as u16,
        protocol: ETH_P_ALL.to_be(),
        ifindex: ifindex as i32,
        hatype: 0,
        pkttype: 0,
        halen: 6,
        addr: [destination[0], destination[1], destination[2], destination[3], destination[4], destination[5], 0, 0],
    }};
    thread::sleep(Duration::from_secs(5));
    loop {{
        for frame in &frames {{
            let sent = unsafe {{
                sendto(
                    fd,
                    frame.as_ptr(),
                    frame.len(),
                    0,
                    &address,
                    size_of::<SockaddrLl>() as u32,
                )
            }};
            assert_eq!(sent, frame.len() as isize);
        }}
        thread::sleep(Duration::from_millis(10));
    }}
}}"#,
            t0 = target[0],
            t1 = target[1],
            t2 = target[2],
            t3 = target[3],
        ),
    )
    .expect("write identifiable guest emitter source");
    let out = tmp.join("shared-guest-network-emitter");
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
        .expect("spawn rustc for identifiable guest emitter");
    assert!(status.success(), "rustc builds the identifiable guest emitter");
    out
}

/// How long a sealed capture waits before its final drain. A delivery that
/// passed the socket's filter before the seal completes inside the receive
/// path's RCU read section; the settle lets such a frame land in the queue so
/// the final drain reads it. A frame that still lands later is counted but
/// unread, and the reconciliation fails it closed; it never passes.
pub(super) const PACKET_SOCKET_SEAL_SETTLE: Duration = Duration::from_millis(50);

/// Seals an `AF_PACKET` socket before its final drain: attach a
/// one-instruction classic BPF program that accepts nothing. `packet_rcv`
/// rejects a frame the filter refuses before it counts or queues it, so after
/// the seal the queue and `PACKET_STATISTICS.tp_packets` describe the same
/// closed population.
pub(super) fn seal_packet_socket(fd: RawFd) -> std::io::Result<()> {
    let mut reject_all = [libc::sock_filter {
        code: u16::try_from(libc::BPF_RET | libc::BPF_K).expect("BPF_RET|BPF_K fits u16"),
        jt: 0,
        jf: 0,
        k: 0,
    }];
    let program = libc::sock_fprog { len: 1, filter: reject_all.as_mut_ptr() };
    // SAFETY: `program` points at one live instruction for the duration of the
    // call; the kernel copies the program before returning.
    let result = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ATTACH_FILTER,
            std::ptr::from_ref(&program).cast(),
            libc::socklen_t::try_from(std::mem::size_of::<libc::sock_fprog>())
                .expect("sock_fprog size fits socklen_t"),
        )
    };
    if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
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
pub(super) fn no_cloud_hypervisor_process_running() -> bool {
    let entries = std::fs::read_dir("/proc").expect("read /proc");
    for entry_result in entries {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("read /proc entry while checking Cloud Hypervisor: {error}"),
        };
        let Ok(_pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let cmdline_path = entry.path().join("cmdline");
        let cmdline = match std::fs::read(&cmdline_path) {
            Ok(cmdline) => cmdline,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("read {}: {error}", cmdline_path.display()),
        };
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

    let (handle, server_tmp) = spawn_vm_server().await;
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
    let rootfs_plan = RootfsPlan::for_alloc(
        broken_rootfs.clone(),
        master_bytes,
        &alloc,
        // The clone lands in the platform staging dir under the server data_dir
        // (ADR-0082 fourth amendment B1 fix); the deadline arm must remove it
        // FROM THERE -- `clone_dest` derives from this staging dir.
        &overdrive_core::vm::config::clone_staging_dir(&server_tmp.path().join("data")),
        std::path::Path::new("/run/overdrive/vm/clone-index"),
    );
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

    let (handle, server_tmp) = spawn_vm_server().await;
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

    let (handle, server_tmp) = spawn_vm_server().await;
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

    let (handle, server_tmp) = spawn_vm_server().await;

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
// via the `NodeAdapters` bundle's `VmAdapters` (ADR-0219, `Required`).
// ---------------------------------------------------------------------

/// Spawns a real in-process `overdrive serve` with the given real VM
/// boot artifacts composed AND the VM ports replaced in the bundle (ADR-0083 §D8, step
/// 01-09) — the composition root's `discover -> probe -> insert`
/// sequence resolves `vmm` first, then calls `.probe()` UNCONDITIONALLY
/// against it, exactly as it does for the production
/// `CloudHypervisorVmm`. `SimDataplane`-composed (not mTLS), matching
/// [`spawn_vm_server`] — S-VM-13/S-VM-75 are boot-refusal scenarios that
/// need no mesh composition. Returns `Result` (not `.expect()`-unwrapped)
/// since both callers assert on the `Err` arm.
async fn spawn_vm_server_with_vmm_override(
    vm_ports: super::serve_fixture::VmPorts,
) -> Result<(ServeHandle, TempDir), overdrive_cli::http_client::CliError> {
    let tmp = server_tmp_on_staging_root();
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = super::serve_fixture::serve_args(bind, data_dir, config_dir);
    let result = super::serve_fixture::serve_with_vm(
        args,
        super::serve_fixture::DataplaneChoice::Sim,
        vm_ports,
    )
    .await;
    result.map(|handle| (handle, tmp))
}

/// S-VM-13 — cloud-hypervisor present but a capability the host cannot
/// supply is missing: the node refuses to boot with a
/// `health.startup.refused`-shaped event naming the probe. Injected via
/// the `NodeAdapters` bundle's `VmAdapters` (ADR-0219, `Required`;
/// ADR-0083 §D8) — a `SimVmm` carrying a
/// capability-flag probe fault, declaring "cloud-hypervisor IS present"
/// so the composition root's `VmComposeError::Refused` (hard-refusal)
/// path fires, never `NotAvailable` (capability-ABSENCE soft-skip,
/// S-VM-12's path). Per S-VM-13's own crafter note, this fault class has
/// no genuinely-lying real host in the Lima/metal test envelope — the
/// injection is the sanctioned mechanism (ADR-0083 §D8's own ruling).
#[tokio::test]
#[serial(cgroup)]
async fn vm_capability_flag_probe_failure_injected_via_vmm_override_refuses_boot() {
    let sim_vmm = SimVmm::new();
    sim_vmm.inject_probe_failure(SimVmmProbeFault::LandlockLsmAbsent);

    let beacon = std::sync::Arc::new(overdrive_sim::adapters::SimBeaconIo::new(sim_vmm.clone()));
    let result = spawn_vm_server_with_vmm_override(super::serve_fixture::VmPorts {
        vmm: std::sync::Arc::new(sim_vmm),
        beacon,
    })
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
/// deliberately uses), injected through the SAME `VmAdapters` ports
/// S-VM-13 uses — the seam is `Arc<dyn Vmm>`, adapter-agnostic, and a
/// REAL differently-configured adapter satisfies it exactly as a
/// fault-injecting `SimVmm` does.
#[tokio::test]
#[serial(cgroup)]
async fn vm_non_reflink_staging_directory_refuses_boot_via_executed_ficlone() {
    let non_reflink_dir = tempfile::Builder::new()
        .prefix("overdrive-vm-ws-s-vm-75-")
        .tempdir_in("/dev/shm")
        .expect("create a tmpfs probe-image-dir under /dev/shm (guaranteed tmpfs on Linux)");
    let real_vmm_non_reflink =
        CloudHypervisorVmm::new().with_image_dir(non_reflink_dir.path().to_path_buf());

    // The probe refuses before the beacon is bound; the production beacon
    // port pairs with the real VMM (DG-4: a VMM and its beacon travel together).
    let result = spawn_vm_server_with_vmm_override(super::serve_fixture::VmPorts {
        vmm: std::sync::Arc::new(real_vmm_non_reflink),
        beacon: std::sync::Arc::new(overdrive_host::beacon_io::VsockBeaconIo::new()),
    })
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

    let (handle, server_tmp) = spawn_vm_server().await;
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
/// # The two discriminators, and why each is load-bearing
///
/// **Host-side.** The two rootfs masters are staged in two DIFFERENT parent
/// directories. `RootfsPlan::for_alloc` derives the per-launch clone
/// destination as `<master_dir>/.overdrive-vm-rootfs-<alloc>.img` and does
/// NOT encode the master's own filename, so two masters sharing one parent
/// would produce two clone paths differing only by allocation id — which is
/// exactly what a re-introduced node-level default would also produce. Two
/// parents make the two clone paths differ in their DIRECTORY component,
/// which no single-master node default can imitate.
///
/// **In-guest.** Each image carries a different injected guest binary
/// (`/sbin/spinone`, `/sbin/spintwo`) and each spec names only its own. Every
/// per-test rootfs is a copy of the shared fixture image, which carries only
/// `overdrive-init` — so neither image contains the other's binary. Were a
/// node-level default to force both allocations onto one image, the
/// mismatched guest would beacon READY, fail its exec, power down, and be
/// unable to STAY Running. That is the Gherkin's "each guest identifiably
/// reporting which image it booted", independent of the host-side check.
///
/// [`find_cloud_hypervisor_pid`] is deliberately NOT used: its own contract
/// pins "exactly one VM is booted at the time of the call", and this is the
/// first scenario in the file to run two concurrently. It uses the
/// allocation-scoped [`hypervisor_argv_for_alloc`] instead.
///
/// **Hygiene, not an acceptance claim**: both guests are long-lived and never
/// beacon EXIT, so both workloads are driven to Terminated through the
/// production `stop` verb (and confirmed there) BEFORE `handle.shutdown()`.
/// Production spawns the VMM with `kill_on_drop(false)`, so a still-Running
/// guest is orphaned by server teardown alone and contaminates the next
/// serialized test's `/proc` scan — the lesson S-VM-05 learned on the metal
/// box (commit `28dbefdc`), doubled here because there are two of them.
#[tokio::test]
#[serial(cgroup)]
async fn two_vm_jobs_on_one_serve_each_boot_from_the_rootfs_their_own_spec_named() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");

    // TWO staging roots, never one. `RootfsPlan::for_alloc` derives the
    // per-launch clone destination as
    // `<master_dir>/.overdrive-vm-rootfs-<alloc>.img` and does NOT encode
    // the master's own filename, so two masters sharing one parent would
    // produce two clone paths differing only by allocation id -- exactly
    // what a re-introduced node-level default would also produce, and the
    // discriminating assertion below could not tell the two apart.
    // (`stage_rootfs_with_extra_binary` also writes to the fixed name
    // `tmp.join("rootfs.ext4")`, so two calls against one root would
    // silently overwrite the first image regardless.)
    let tmp_one = tempfile::Builder::new()
        .prefix("vm-ws-s54-one-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let tmp_two = tempfile::Builder::new()
        .prefix("vm-ws-s54-two-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the XFS-backed reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");

    // The SECOND, in-guest discriminator: each image carries a DIFFERENT
    // long-lived guest binary, and each spec names only its own. Every
    // per-test rootfs is a copy of the shared fixture image, which carries
    // ONLY `overdrive-init` -- so neither image contains the other's
    // binary. Were a node-level default to force both allocations onto one
    // image, the mismatched guest would beacon READY, fail its exec, power
    // down, and be unable to STAY Running.
    let spin_one = build_spin_binary(tmp_one.path());
    let spin_two = build_spin_binary(tmp_two.path());
    let rootfs_one = stage_rootfs_with_extra_binary(tmp_one.path(), &fixture, &spin_one, "spinone");
    let rootfs_two = stage_rootfs_with_extra_binary(tmp_two.path(), &fixture, &spin_two, "spintwo");

    // ONE serve, with NO artifact argument anywhere -- that absence IS the
    // scenario's `Given`.
    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());

    let spec_one = write_toml(
        server_tmp.path(),
        "vm-s54-one.toml",
        &vm_job_toml("vm-s54-one", "/sbin/spinone", &[], &fixture.kernel_path, &rootfs_one),
    );
    let spec_two = write_toml(
        server_tmp.path(),
        "vm-s54-two.toml",
        &vm_job_toml("vm-s54-two", "/sbin/spintwo", &[], &fixture.kernel_path, &rootfs_two),
    );

    let submit_one = deploy(DeployArgs { spec: spec_one, config_path: cfg.clone() })
        .await
        .expect("deploy the first [vm] spec");
    let submit_two = deploy(DeployArgs { spec: spec_two, config_path: cfg.clone() })
        .await
        .expect("deploy the second [vm] spec against the SAME running serve");
    assert_eq!(submit_one.workload_id, "vm-s54-one");
    assert_eq!(submit_two.workload_id, "vm-s54-two");

    // 90s: comfortably above the 30s VM_BOOT_DEADLINE for two concurrent
    // real guest boots, so a pass means both booted rather than the poll
    // being generous.
    let running_one =
        poll_until_running(&cfg, &submit_one.workload_id, Duration::from_secs(90)).await;
    let running_two =
        poll_until_running(&cfg, &submit_two.workload_id, Duration::from_secs(90)).await;

    let row_one = running_one.snapshot.rows.first().expect("one allocation row for the first job");
    let row_two = running_two.snapshot.rows.first().expect("one allocation row for the second job");
    assert_eq!(
        row_one.reason,
        Some(TransitionReason::Started),
        "a Running VM allocation carries the beacon-win progress marker; got {:?}",
        row_one.reason,
    );
    assert_eq!(
        row_two.reason,
        Some(TransitionReason::Started),
        "a Running VM allocation carries the beacon-win progress marker; got {:?}",
        row_two.reason,
    );

    let alloc_one = AllocationId::new(&row_one.alloc_id)
        .expect("server-echoed alloc_id parses as AllocationId");
    let alloc_two = AllocationId::new(&row_two.alloc_id)
        .expect("server-echoed alloc_id parses as AllocationId");

    // The discriminating assertion. Each allocation derives its OWN distinct
    // per-launch clone (a distinct alloc-named file in the platform staging
    // dir, ADR-0082 2026-08-18 fourth amendment B1 fix -- NEVER beside either
    // operator's master), and the live hypervisor for THAT allocation's run
    // directory must carry that clone path in its argv. A single node-level
    // master would collapse the two onto one image.
    let master_bytes_one =
        std::fs::metadata(&rootfs_one).expect("stat the first rootfs master").len();
    let master_bytes_two =
        std::fs::metadata(&rootfs_two).expect("stat the second rootfs master").len();
    let staging_dir =
        overdrive_core::vm::config::clone_staging_dir(&server_tmp.path().join("data"));
    let plan_one = RootfsPlan::for_alloc(
        rootfs_one.clone(),
        master_bytes_one,
        &alloc_one,
        &staging_dir,
        tmp_one.path(),
    );
    let plan_two = RootfsPlan::for_alloc(
        rootfs_two.clone(),
        master_bytes_two,
        &alloc_two,
        &staging_dir,
        tmp_two.path(),
    );
    let clone_one = plan_one.clone_dest();
    let clone_two = plan_two.clone_dest();

    assert!(
        clone_one.exists(),
        "the first allocation's per-launch clone must exist in the platform staging dir, at {}",
        clone_one.display(),
    );
    assert!(
        clone_two.exists(),
        "the second allocation's per-launch clone must exist in the platform staging dir, at {}",
        clone_two.display(),
    );
    assert_ne!(
        clone_one,
        clone_two,
        "each allocation must derive its OWN distinct per-launch clone; got the same path {}",
        clone_one.display(),
    );
    assert!(
        !clone_one.starts_with(tmp_one.path())
            && !clone_one.starts_with(tmp_two.path())
            && !clone_two.starts_with(tmp_one.path())
            && !clone_two.starts_with(tmp_two.path()),
        "the per-launch clones must live in the platform staging dir, NEVER under either operator's \
         own image directory (tmp_one={}, tmp_two={}) -- a node-level artifact default (or the pre-B1 \
         beside-the-master placement) is exactly what that would look like",
        tmp_one.path().display(),
        tmp_two.path().display(),
    );

    let argv_one = hypervisor_argv_for_alloc(&alloc_one);
    let argv_two = hypervisor_argv_for_alloc(&alloc_two);
    assert!(
        argv_one.contains(&clone_one.to_string_lossy().into_owned()),
        "the first allocation's live hypervisor argv must name ITS OWN clone {}; got: {argv_one}",
        clone_one.display(),
    );
    assert!(
        argv_two.contains(&clone_two.to_string_lossy().into_owned()),
        "the second allocation's live hypervisor argv must name ITS OWN clone {}; got: {argv_two}",
        clone_two.display(),
    );
    assert!(
        !argv_one.contains(&rootfs_two.to_string_lossy().into_owned()),
        "the first allocation's hypervisor argv must not reference the OTHER spec's master {}",
        rootfs_two.display(),
    );
    assert!(
        !argv_two.contains(&rootfs_one.to_string_lossy().into_owned()),
        "the second allocation's hypervisor argv must not reference the OTHER spec's master {}",
        rootfs_one.display(),
    );

    // Hygiene, not an acceptance claim: both guests spin forever and never
    // beacon EXIT, so neither allocation can reach a terminal state on its
    // own. Production spawns the VMM with `kill_on_drop(false)`, so a
    // still-Running guest is orphaned by server teardown alone and
    // contaminates the next serialized test's `/proc` scan (the lesson
    // S-VM-05 learned on the metal box, commit `28dbefdc`) -- doubled here.
    for workload_id in [&submit_one.workload_id, &submit_two.workload_id] {
        stop(StopArgs { id: workload_id.clone(), config_path: cfg.clone() })
            .await
            .expect("stop the long-lived spin workload before shutdown");
        let stopped = poll_until_terminal(&cfg, workload_id, Duration::from_secs(30)).await;
        let stopped_row =
            stopped.snapshot.rows.first().expect("one allocation row for the stopped workload");
        assert_eq!(
            stopped_row.state,
            AllocStateWire::Terminated,
            "an operator stop must drive the never-self-terminating spin allocation {workload_id} \
             to Terminated before this test tears down the server, got {:?}",
            stopped_row.state,
        );
    }

    handle.shutdown().await.expect("clean shutdown");
}
