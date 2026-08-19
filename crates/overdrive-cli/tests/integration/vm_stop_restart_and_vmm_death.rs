//! Slice 03 / AC-11 + AC-12 — exit classification is guest-authoritative,
//! never derived from the hypervisor's own exit status (AC-11, GH #42,
//! brief §105 / feature-delta `[D3]`), and the operator stop verb drives a
//! bounded graceful-shutdown sequence over the guest's beacon before any
//! hard kill (AC-12, ADR-0082 §D4).
//!
//! # What this file proves, and what it does NOT touch
//!
//! This is the **operator-facing Tier-3 proof** of the `[D3]` exit-
//! classification join. The classification machinery itself — the
//! guest-report-vs-VMM-signal join `classify_vm_exit`
//! (`crates/overdrive-worker/src/vm_driver.rs`), the
//! `ExitEvent → AllocStatusRow` mapping `exit_observer::classify`
//! (`crates/overdrive-control-plane/src/worker/exit_observer.rs`), and the
//! `WorkloadLifecycle` restart/backoff branch
//! (`crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`) — is
//! **REUSED UNCHANGED** (feature-delta `[D3]` reuse rows 6/7/11/15;
//! `classify_vm_exit` itself landed at step 01-07). This step drives that
//! already-wired path end-to-end through the production operator surface —
//! a real in-process `overdrive serve` + `overdrive deploy` /
//! `overdrive job stop` direct handler calls (per `crates/overdrive-cli/
//! CLAUDE.md` § "Integration tests — no subprocess") against a REAL Cloud
//! Hypervisor VMM, under `cargo xtask metal run --` as root on a real
//! `x86_64+KVM` box (Lima on Apple Silicon has no nested KVM).
//!
//! No production file is modified by this step: it is a proof, not a build.
//!
//! # The north-star lie this file refuses (`[D3]`, intake precedent #3)
//!
//! The reference implementation derived `ExitKind` from the **host**
//! `cloud-hypervisor` process's `wait()` status — and a guest that boots,
//! runs, and powers off **cleanly exits the VMM `0`** regardless of what
//! happened inside it. Every VM would then report success. `[D3]` refuses
//! that in three distinct situations, none collapsed:
//!
//! | Situation | Honest classification | Scenario |
//! |---|---|---|
//! | Agent reported the guest's exit status | guest's `CleanExit` / `Crashed` | S-VM-44 (exit 0) |
//! | VMM exited with **no** agent report | `Crashed`, cause names the un-reported death | S-VM-42 (panic), S-VM-43 (host kill) |
//! | Operator stop | `intentional_stop`, no restart budget | S-VM-45 |
//!
//! # The scenarios (AC-11: S-VM-42..45; AC-12: S-VM-46..48)
//!
//! * **S-VM-42** — a guest that exits the hypervisor cleanly (`0`) WITHOUT
//!   ever beaconing READY lands `Failed / VmGuestExitUnreported`, never
//!   `Terminated` with a completed condition. `@mandatory:mutation_target`
//!   (K1 north star): a mutation collapsing the unreported-death arm into
//!   `CleanExit` must be killed. The fixture replaces `/sbin/init` with a
//!   static binary that `sync(2)`s and `reboot(POWER_OFF)`s immediately —
//!   the honest realisation of `[D3]`'s "boots, panics, powers off cleanly,
//!   VMM exits 0" row (a real guest kernel *panic* under `panic=1` reboots
//!   in a loop and never exits, which is S-VM-36's `VmBootDeadlineExceeded`
//!   — the load-bearing fixture property here is "VMM exits 0, no agent
//!   report", which a clean power-off-before-beacon produces deterministically).
//!
//! * **S-VM-43** — a hypervisor that is killed by the host (SIGKILL — the
//!   exact signal the kernel cgroup OOM killer delivers) after the guest is
//!   Running lands `Failed / WorkloadCrashedImmediately` — the SAME
//!   classification (via the SAME `exit_observer::classify`) and SAME
//!   `WorkloadLifecycle` run-once finalisation a crashed **process** Job
//!   receives, consuming NO restart budget (K5 crash treatment parity). Two
//!   design facts shape the observable here, and BOTH narrow the scenario's
//!   literal Gherkin:
//!   1. A *real* cgroup OOM of a correctly-`MemoryPlan`-padded VM is not
//!      deterministically inducible from guest behaviour (the padding exists
//!      precisely to prevent it), so the host-delivered kill *signal* is
//!      modelled directly; the `VmOutOfMemory` *diagnosis* is S-VM-19's
//!      separate concern, whereas S-VM-43's claim is purely that a host kill
//!      is a **crash**, not a clean exit.
//!   2. A microVM is **Job-only** (`[service] + [vm]` is rejected — S-VM-38)
//!      and a Job crash **finalises without restarting** by design (the
//!      run-once contract; `workload_lifecycle.rs` Job-kind natural-exit
//!      handler, `is_natural_exit`). So the Gherkin's "same *ceiling*, same
//!      *backoff curve*" sub-clause describes the Service restart-budget
//!      branch a VM never reaches; the observable **parity** is that a
//!      host-killed VM Job is treated *identically to a crashed process Job*
//!      — Failed, `WorkloadCrashedImmediately`, finalised run-once,
//!      `restart_count == 0` — through the SAME reconciler, with no
//!      VM-specific exit path. (Surfaced to acceptance-designer as a
//!      DISTILL crafter-note candidate on S-VM-43, mirroring S-VM-44's
//!      workspace-negative-clause note.)
//!
//! * **S-VM-44** — a guest whose command exits `0` and reports it over the
//!   beacon lands `Terminated` with `Stopped { by: Process }`, which
//!   `WorkloadLifecycle::classify_natural_exit_terminal` maps to
//!   `TerminalCondition::Completed { exit_code: 0 }`. The scenario's
//!   original second Then — "this is the ONLY path in the workspace to that
//!   terminal state" — is a workspace-negative claim no port-observable
//!   assertion can make (DISTILL crafter note); it is discharged as the
//!   `@mandatory:mutation_target` annotation on `[D3]`'s §105 join, whose
//!   COMPLEMENT is asserted by S-VM-42/S-VM-43 (a non-agent-reported exit
//!   does NOT reach this state) and S-VM-02 (a non-zero agent report does
//!   not either).
//!
//! * **S-VM-45** — an operator stop lands `Terminated` attributed to the
//!   operator/reconciler, NOT a crash, and consumes NO restart budget
//!   (`restart_count == 0`, `restart_budget.used == 0`).
//!
//! * **S-VM-46** (AC-12) — stopping a running VM drives ADR-0082 §D4's
//!   graceful-shutdown sequence (the `SHUTDOWN` write on the guest's
//!   already-open beacon, then the `Vmm::terminate` escalation) to
//!   `Terminated / Stopped { by: Operator }`, the SAME driver-agnostic
//!   terminal a stopped process workload reaches. The **first real
//!   evidence for the host→guest `SHUTDOWN` write** (the spike exercised
//!   guest→host only, `findings.md:2787`) — a mechanism proof, not a
//!   regression guard.
//!
//! * **S-VM-47** (AC-12, error path) — a guest that ignores the `SHUTDOWN`
//!   request (the shipped `overdrive-init` blocked on its long-lived child,
//!   never reaching its post-command `read_shutdown_or_eof`) is still
//!   stopped within the bounded grace (`VM_SHUTDOWN_REQUEST_DEADLINE` 2s +
//!   `VM_STOP_GRACE` 10s), escalated to `Vmm::terminate`'s SIGKILL, and
//!   lands the operator-stop terminal — NEVER `WorkloadCrashedImmediately`,
//!   even though the SIGKILL'd VMM is indistinguishable in isolation from
//!   S-VM-43's host kill.
//!
//! * **S-VM-48** (AC-12, edge case) — a VM whose guest MODIFIED its rootfs
//!   clone and was then RESTARTED boots from a fresh `FICLONE` copy of the
//!   operator's read-only master (the prior modification is absent), and the
//!   master file on the host is byte-unchanged. **Restart trigger reframed
//!   (surfaced to acceptance-designer):** the DISTILL Gherkin's "crash →
//!   restart under backoff" is NOT producible for a Job-only microVM (S-VM-38
//!   rejects `[vm]+[service]`; a Job crash finalises RUN-ONCE with no
//!   restart-under-backoff path — the SAME fact S-VM-43 documents). This
//!   scenario proves the SAME observable invariant through the phase-02
//!   **platform-reclamation restart** — the boot-epoch reclaim-then-restart
//!   cycle `vm_reclamation_tier3.rs`'s S-VM-28 drives: a platform-reclaimed
//!   Job whose intent still stands is re-driven by `WorkloadLifecycle` via
//!   `Action::RestartAllocation` (DD-1), re-invoking
//!   `CloudHypervisorVmm::create`, whose per-launch `ficlone_rootfs` clones
//!   the read-only master afresh and never mutates it. A PROOF of that
//!   already-wired mechanism end to end, not a build (no production file is
//!   modified by this scenario either).
//!
//! # `#[serial(cgroup)]` + the `host-kernel-shared` nextest group
//!
//! Every test here boots a full production `run_server` against the REAL
//! host cgroupfs and scans `/proc` for the allocation's `cloud-hypervisor`
//! process. `#[serial(cgroup)]` serialises WITHIN this process; the
//! module's `host-kernel-shared` entry in `.config/nextest.toml` (added
//! for this file, mirroring `vm_walking_skeleton` / `vm_boot_failure_vocabulary`
//! / `vm_reclamation_tier3`) serialises it ACROSS nextest's per-test
//! processes, so its SIGKILL and `/proc` scans never contaminate a sibling
//! Tier-3 VM test's own allocation-scoped process view — the exact
//! cross-test contamination class `vm_walking_skeleton.rs`'s history
//! documents (S-VM-05, fourth pass).

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(
    clippy::missing_panics_doc,
    clippy::unwrap_used,
    clippy::expect_used,
    // real_uid/eff_uid/real_gid/eff_gid mirror /proc/<pid>/status's own
    // real/effective columns -- the names are intentionally parallel.
    clippy::similar_names
)]

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, WorkloadDescribeOutput, describe};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::TransitionReason;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::driver::ConfinementControl;
use overdrive_core::traits::vmm::Vmm;
use overdrive_core::transition_reason::StoppedBy;
use overdrive_core::vm::config::VmRunDir;
use overdrive_sim::SimVmm;
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tempfile::TempDir;

// ---------------------------------------------------------------------
// Fixture staging — file-local copies of the shapes the sibling Tier-3 VM
// test files settled on. Sibling test modules cannot see each other's
// private items, and each Tier-3 VM file is self-contained by convention
// (`vm_walking_skeleton.rs`, `vm_boot_failure_vocabulary.rs`).
// ---------------------------------------------------------------------

/// The shared staging root every Tier-3 VM test file provisions against
/// (per `vm_fixture`'s AC5 concurrency contract — safe under concurrent
/// nextest processes because each file stages per-test COPIES and never
/// mutates the shared artifact).
fn shared_staging_root() -> PathBuf {
    overdrive_testing::vm_fixture::default_staging_root()
}

/// A server-side tempdir (holding `data/` + `conf/`) on the reflink-capable
/// staging root, NOT the system tmpdir. Required because each per-launch rootfs
/// clone is FICLONE'd into `clone_staging_dir(data_dir)`, and FICLONE is
/// intra-filesystem (ADR-0082 2026-08-18 fourth amendment): with `data_dir` on
/// tmpfs and the master on the xfs staging root, the clone would fail `EXDEV`
/// and every VM boot would refuse. Co-locating `data_dir` with the masters
/// respects the production invariant (one VM data partition holds both).
fn server_tmp_on_staging_root() -> TempDir {
    tempfile::Builder::new()
        .prefix("vm-serve-")
        .tempdir_in(shared_staging_root())
        .expect("server tempdir on the reflink-capable staging root")
}

/// A real in-process `overdrive serve` — production `run_server` wiring
/// with only the dataplane and KEK external ports replaced by their
/// established simulation adapters (the same composition
/// `vm_walking_skeleton.rs`'s S-VM-01/02 use; these scenarios need a real
/// CH and the real exit-classification chain, not the real `EbpfDataplane`).
async fn spawn_vm_server() -> (ServeHandle, TempDir) {
    // The server's `data_dir` MUST share the rootfs master's filesystem: each
    // per-launch clone is FICLONE'd into `clone_staging_dir(data_dir)` and
    // FICLONE is intra-filesystem (ADR-0082 2026-08-18 fourth amendment). The
    // masters stage on the reflink-capable `shared_staging_root()` (xfs on the
    // metal box); the system tmpdir is tmpfs and would fail FICLONE `EXDEV`, so
    // the data_dir tempdir is created on the staging root too.
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

/// The same real in-process `serve` composition as [`spawn_vm_server`], with
/// the `Vmm` port additionally bound to a caller-supplied adapter
/// (`ServerConfig.vmm_override`, ADR-0083 §D8, step 01-09). S-VM-51 binds a
/// [`SimVmm`] armed to fail `create` CLOSED with `VmmError::ConfinementUnavailable`
/// — `.probe()` still runs unconditionally against it (a clean sim probe), so
/// the node boots and registers the VM driver exactly as in production; only
/// the port binding differs. Mirrors `vm_boot_failure_vocabulary.rs`'s helper
/// of the same name (a file-local copy — sibling test modules cannot share
/// private items).
async fn spawn_vm_server_with_vmm(vmm: std::sync::Arc<dyn Vmm>) -> (ServeHandle, TempDir) {
    // data_dir on the reflink staging root — see `server_tmp_on_staging_root`.
    let tmp = server_tmp_on_staging_root();
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
        vmm,
    )
    .await
    .expect("serve::run_with_dataplane_and_vmm_override");
    (handle, tmp)
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

/// A plain per-test COPY of the shared fixture's rootfs — no loopback mount,
/// no injected guest binary. S-VM-51's `SimVmm` never boots the guest (it
/// fails `create` closed on confinement), so the rootfs only has to exist as a
/// real file for the deploy's `RootfsPlan` staging + `SimVmm`'s `std::fs::copy`
/// clone. The shared fixture artifact is never mutated (AC5).
fn stage_plain_rootfs_copy(tmp: &Path, fixture: &VmFixture) -> PathBuf {
    let dest = tmp.join("rootfs.ext4");
    std::fs::copy(&fixture.rootfs_path, &dest)
        .expect("copy the shared fixture rootfs into a per-test working copy");
    dest
}

/// A `[job]`+`[vm]`+`[resources]` TOML — the shape a real operator writes,
/// each allocation sourcing its own kernel and rootfs from its `[vm]` block
/// (no node-level artifact seam; step 03-07 removed it).
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
    std::fs::write(&path, body).expect("write VM workload spec");
    path
}

/// Cross-builds a tiny static-musl binary that does nothing but
/// `std::process::exit(exit_code)` — S-VM-44's guest command. Mirrors the
/// `vm_walking_skeleton.rs` helper of the same name (a file-local copy;
/// sibling modules cannot share private items). `x86_64-unknown-linux-musl`
/// is the only target this fixture's kernel staging supports.
fn build_exit_code_binary(tmp: &Path, exit_code: u8) -> PathBuf {
    let src = tmp.join(format!("exit{exit_code}.rs"));
    std::fs::write(&src, format!("fn main() {{ std::process::exit({exit_code}); }}"))
        .expect("write tiny exit-code source");
    let out = tmp.join(format!("exit{exit_code}"));
    rustc_static_musl(&src, &out);
    out
}

/// Cross-builds a tiny static-musl binary that loops forever until killed —
/// the "reaches Running and stays there" shape S-VM-43 and S-VM-45 need. A
/// guest command that exits promptly makes `Running` a transient window the
/// poller can legitimately miss.
fn build_spin_binary(tmp: &Path) -> PathBuf {
    let src = tmp.join("spin.rs");
    std::fs::write(
        &src,
        "fn main() { loop { std::thread::sleep(std::time::Duration::from_secs(3600)); } }",
    )
    .expect("write the long-lived spin source");
    let out = tmp.join("spin");
    rustc_static_musl(&src, &out);
    out
}

/// Cross-builds a static-musl PID-1 replacement that `sync(2)`s then
/// `reboot(2)`s with `LINUX_REBOOT_CMD_POWER_OFF` — a clean guest power-off
/// that never dials the beacon. Booted as `/sbin/init`, it makes
/// `cloud-hypervisor` exit `0` within a couple of seconds without the guest
/// ever reaching READY, which is exactly `start`'s three-way boot race
/// resolving on its VMM-exit arm → `VmStartFailure::GuestExitUnreported`.
///
/// The raw syscalls are issued by hand because a `rustc`-only cross-build
/// (no Cargo project) cannot pull the `libc` crate. x86_64-only, which the
/// fixture already is (`stage_kernel` rejects aarch64). `sync` before the
/// power-off flushes the rootfs so the run is deterministic; the trailing
/// `loop` guarantees the binary never *returns* from `main` (a PID-1 exit
/// would panic the kernel and, under `panic=1`, reboot-loop into
/// `VmBootDeadlineExceeded` instead — the wrong terminal).
fn build_poweroff_init_binary(tmp: &Path) -> PathBuf {
    let src = tmp.join("poweroff_init.rs");
    std::fs::write(
        &src,
        r#"fn main() {
    unsafe {
        // sync(2) — __NR_sync = 162 on x86_64.
        core::arch::asm!(
            "syscall",
            inlateout("rax") 162_usize => _,
            lateout("rcx") _,
            lateout("r11") _,
        );
        // reboot(magic1, magic2, cmd, arg) — __NR_reboot = 169 on x86_64.
        // magic1 = 0xfee1dead, magic2 = 672274793 (0x28121969),
        // cmd = LINUX_REBOOT_CMD_POWER_OFF = 0x4321fedc, arg = 0.
        core::arch::asm!(
            "syscall",
            inlateout("rax") 169_usize => _,
            in("rdi") 0xfee1dead_usize,
            in("rsi") 672274793_usize,
            in("rdx") 0x4321fedc_usize,
            in("r10") 0_usize,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
    // Unreachable on success (the machine powers off); never let PID 1 exit.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
"#,
    )
    .expect("write the poweroff-init source");
    let out = tmp.join("poweroff_init");
    rustc_static_musl(&src, &out);
    out
}

/// The one `rustc` invocation every guest-binary helper here shares.
fn rustc_static_musl(src: &Path, out: &Path) {
    let status = Command::new("rustc")
        .args(["--edition", "2021", "-C", "opt-level=0", "-C", "target-feature=+crt-static"])
        .args(["--target", "x86_64-unknown-linux-musl"])
        .arg("-o")
        .arg(out)
        .arg(src)
        .status()
        .expect("spawn rustc for a static-musl guest binary");
    assert!(status.success(), "rustc must build the static-musl guest binary at {}", out.display());
}

/// Stages a PER-TEST COPY of the shared fixture's rootfs with an additional
/// static binary injected at `/sbin/<guest_name>`, via a HOST-side loopback
/// mount before the guest ever boots. The shared fixture artifact is never
/// mutated, so concurrent Tier-3 VM files reusing the same staging root
/// (AC5) are unaffected. Runs as root (this suite runs under
/// `cargo xtask metal run --`).
fn stage_rootfs_with_extra_binary(
    tmp: &Path,
    fixture: &VmFixture,
    host_bin: &Path,
    guest_name: &str,
) -> PathBuf {
    with_mounted_rootfs_copy(tmp, fixture, |mnt| {
        let dest = mnt.join("sbin").join(guest_name);
        install_guest_binary(host_bin, &dest);
    })
}

/// Stages a PER-TEST COPY of the shared fixture's rootfs with BOTH init
/// entry points (`/sbin/init` and `/init`) overwritten by `host_bin` — the
/// S-VM-42 fixture. Overwriting is required rather than injecting a new
/// name: the kernel boots the rootfs's own `/sbin/init` (no `init=` in the
/// platform cmdline), so to prevent the beaconing `overdrive-init` from ever
/// running, its two on-disk copies must be replaced.
fn stage_rootfs_replacing_init(tmp: &Path, fixture: &VmFixture, host_bin: &Path) -> PathBuf {
    with_mounted_rootfs_copy(tmp, fixture, |mnt| {
        install_guest_binary(host_bin, &mnt.join("sbin").join("init"));
        install_guest_binary(host_bin, &mnt.join("init"));
    })
}

/// Copy the shared rootfs, loopback-mount the copy, run `edit` against the
/// mount point, unmount, and return the per-test copy. The single
/// `losetup`/`mount`/`umount` plumbing both stagers above share.
fn with_mounted_rootfs_copy(tmp: &Path, fixture: &VmFixture, edit: impl FnOnce(&Path)) -> PathBuf {
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

    edit(&mnt);

    let umount_status = Command::new("umount").arg(&mnt).status().expect("spawn umount");
    assert!(umount_status.success(), "umount {} failed", mnt.display());
    // Best-effort detach — a leaked loop device affects host hygiene, not
    // the correctness of any assertion below.
    let _ = Command::new("losetup").arg("-d").arg(&loop_dev).status();

    rootfs_copy
}

/// Copy `host_bin` to `dest` and make it executable — the per-file
/// installation both stagers share.
fn install_guest_binary(host_bin: &Path, dest: &Path) {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).expect("create the guest binary's parent directory");
    }
    std::fs::copy(host_bin, dest).expect("copy the static binary into the mounted rootfs");
    let mut perms = std::fs::metadata(dest).expect("stat the copied guest binary").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(dest, perms).expect("chmod the copied guest binary executable");
}

// ---------------------------------------------------------------------
// Polling — the row-selection rule and timeout message every scenario
// shares (the shape `vm_walking_skeleton.rs` / `vm_boot_failure_vocabulary.rs`
// settled on).
// ---------------------------------------------------------------------

async fn describe_once(cfg: &Path, workload_id: &str) -> WorkloadDescribeOutput {
    describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_owned() })
        .await
        .expect("workload describe must succeed while polling")
}

async fn poll_until_state(
    cfg: &Path,
    workload_id: &str,
    wanted: AllocStateWire,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let out = describe_once(cfg, workload_id).await;
        if out.snapshot.rows.first().is_some_and(|row| row.state == wanted) {
            return out;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workload {workload_id} did not reach {wanted:?} within {max_wait:?}; last row: {:?}",
            out.snapshot.rows.first(),
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn poll_until_running(
    cfg: &Path,
    workload_id: &str,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    poll_until_state(cfg, workload_id, AllocStateWire::Running, max_wait).await
}

async fn poll_until_terminated(
    cfg: &Path,
    workload_id: &str,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    poll_until_state(cfg, workload_id, AllocStateWire::Terminated, max_wait).await
}

/// Polls until the first allocation row is `Failed`, recording EVERY state
/// observed along the way so a scenario can assert on the states the
/// allocation passed THROUGH (e.g. "never Running") and not merely on where
/// it landed.
async fn poll_until_failed_recording_states(
    cfg: &Path,
    workload_id: &str,
    max_wait: Duration,
) -> (WorkloadDescribeOutput, Vec<AllocStateWire>) {
    let deadline = tokio::time::Instant::now() + max_wait;
    let mut observed = Vec::new();
    loop {
        let out = describe_once(cfg, workload_id).await;
        if let Some(row) = out.snapshot.rows.first() {
            if observed.last() != Some(&row.state) {
                observed.push(row.state);
            }
            if row.state == AllocStateWire::Failed {
                return (out, observed);
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workload {workload_id} did not reach Failed within {max_wait:?}; observed: {observed:?}",
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// The pid of the live `cloud-hypervisor` process serving THIS allocation,
/// located by matching the allocation's own `VmRunDir` path against each
/// process's `argv` (never the `TASK_COMM_LEN`-truncated `/proc/<pid>/comm`,
/// which caps at 15 chars and can never equal the 16-char `cloud-hypervisor`).
///
/// Allocation-scoped, not host-global: sibling Tier-3 scenarios spawn their
/// own `cloud-hypervisor` processes, and (per the `host-kernel-shared`
/// nextest group this file joins) never concurrently, but the scoped match
/// is the same discipline `vm_boot_failure_vocabulary.rs` proved out and
/// keeps the SIGKILL aimed at exactly this test's hypervisor.
fn cloud_hypervisor_pid_for_alloc(alloc: &AllocationId) -> u32 {
    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), alloc);
    let needle = run_dir.path().to_string_lossy().into_owned();
    for entry in std::fs::read_dir("/proc").expect("read /proc") {
        let Ok(entry) = entry else { continue };
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let argv0 = cmdline.split(|&byte| byte == 0).next().unwrap_or(&[]);
        let argv0 = String::from_utf8_lossy(argv0);
        if Path::new(argv0.as_ref()).file_name() != Some(std::ffi::OsStr::new("cloud-hypervisor")) {
            continue;
        }
        if String::from_utf8_lossy(&cmdline).contains(&needle) {
            return pid;
        }
    }
    panic!("no live cloud-hypervisor process found whose argv references {needle}")
}

fn alloc_id_of(out: &WorkloadDescribeOutput) -> AllocationId {
    let row = out.snapshot.rows.first().expect("one allocation row for the deployed workload");
    AllocationId::new(&row.alloc_id).expect("server allocation id is valid")
}

// ---------------------------------------------------------------------------
// S-VM-48 helpers — the reclaim-then-restart cycle needs two boots against
// the SAME data_dir (unlike `spawn_vm_server`, which makes its own tempdir
// per call), plus the marker guest, the restart poller, and the operator-
// artifact fingerprint. These mirror `vm_reclamation_tier3.rs`'s shapes;
// sibling test modules cannot see each other's private items.
// ---------------------------------------------------------------------------

/// A real in-process `overdrive serve` bound to CALLER-CHOSEN `data_dir` /
/// `config_dir`, so two boots can run against the SAME `data_dir` — the
/// reclaim-then-restart cycle (S-VM-28) needs boot #2 to read boot #1's
/// durable state. Same composition as [`spawn_vm_server`] (`SimDataplane` +
/// `SimKek`), only the directories differ.
async fn spawn_vm_server_at(data_dir: &Path, config_dir: &Path) -> ServeHandle {
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

/// Bridges the narrow race between `ServeHandle::shutdown` returning and the
/// `redb` file descriptors actually closing, so a reboot against the SAME
/// `data_dir` does not observe `"Database already open"`. Mirrors
/// `vm_reclamation_tier3.rs::wait_for_data_dir_release`.
async fn wait_for_data_dir_release() {
    tokio::time::sleep(Duration::from_millis(500)).await;
}

/// Polls until the single row is `Running` again with `restart_count >= 1`
/// — the reclaim-then-restart postcondition (S-VM-28). A restart REUSES the
/// same `alloc_id` (`Action::RestartAllocation`), so `Running` alone cannot
/// distinguish the original boot from a recovered restart; the restart count
/// pins it. Mirrors `vm_reclamation_tier3.rs::poll_until_restarted`.
async fn poll_until_restarted(
    cfg: &Path,
    workload_id: &str,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let out = describe_once(cfg, workload_id).await;
        if out
            .snapshot
            .rows
            .first()
            .is_some_and(|row| row.state == AllocStateWire::Running && row.restart_count >= 1)
        {
            return out;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "workload {workload_id} did not restart (Running with restart_count>=1) within \
             {max_wait:?}; last row: {:?}",
            out.snapshot.rows.first(),
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// A compact `(length, content-hash)` fingerprint of the operator's rootfs
/// artifact, STREAMED through a hasher so a 64 MiB ext4 image never lands two
/// full copies in memory. Dependency-free (`DefaultHasher`) — a real mutation
/// to the image changes the fingerprint; a byte-identical file preserves it.
/// This is the direct host-side proof of S-VM-48's second Then (the master is
/// byte-unchanged); FICLONE only ever READS the master, and the guest's
/// writes land on its own copy-on-write clone.
fn rootfs_fingerprint(path: &Path) -> (u64, u64) {
    use std::hash::Hasher;
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).expect("open the operator rootfs artifact to fingerprint");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut buf = [0u8; 8192];
    let mut len: u64 = 0;
    loop {
        let read =
            file.read(&mut buf).expect("read the operator rootfs artifact while fingerprinting");
        if read == 0 {
            break;
        }
        len = len.saturating_add(read as u64);
        hasher.write(&buf[..read]);
    }
    (len, hasher.finish())
}

/// Cross-builds a static-musl guest command that MODIFIES its rootfs on a
/// fresh clone and then spins — the S-VM-48 guest. On boot it checks for a
/// marker file at the rootfs root:
///
/// * marker PRESENT — a prior life's write survived, i.e. this boot adopted a
///   MUTATED clone. The clean-copy invariant is violated; exit `66` so the
///   allocation lands a crash terminal `poll_until_restarted` never reaches.
///   This branch must NEVER fire on a fresh clone.
/// * marker ABSENT — a fresh clone. WRITE the marker (the "modified its
///   rootfs" Given), then RE-READ it to confirm the write actually landed on
///   the mounted block device — a silent drop exits `77`/`78` loudly, so
///   boot #1 never reaches `Running` and the test fails at its baseline
///   rather than passing vacuously. Then spin forever so the allocation stays
///   `Running` (reclaimable + restartable + observable — the long-lived shape
///   S-VM-28's cycle needs).
///
/// The guest runs as PID-1's child (root) with the rootfs mounted `rw`
/// (`root=/dev/vda rw`, `KernelCmdline::platform_default`), so the write to
/// `/` genuinely mutates the clone's ext4. Same static-musl cross-build every
/// guest helper in this file uses.
fn build_marker_or_spin_binary(tmp: &Path) -> PathBuf {
    let src = tmp.join("marker_or_spin.rs");
    std::fs::write(
        &src,
        r#"fn main() {
    let marker = std::path::Path::new("/overdrive-rootfs-clean-marker");
    if marker.exists() {
        // Booted from a MUTATED clone: a prior life's write survived. The
        // clean-copy invariant is violated -- crash so the restart poller
        // (Running + restart_count>=1) never reaches this allocation.
        std::process::exit(66);
    }
    // Fresh clone: MODIFY the rootfs, then confirm the modification really
    // landed on the mounted block device. A rootfs that silently drops the
    // write would void the premise, so make that failure loud -- boot #1
    // would then never reach Running and the test fails at its baseline.
    if std::fs::write(marker, b"modified-by-a-prior-life").is_err() {
        std::process::exit(77);
    }
    if !marker.exists() {
        std::process::exit(78);
    }
    // Stay alive so the allocation stays Running (reclaimable + restartable).
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
"#,
    )
    .expect("write the marker-or-spin guest source");
    let out = tmp.join("marker_or_spin");
    rustc_static_musl(&src, &out);
    out
}

// ---------------------------------------------------------------------------
// S-VM-42 — a guest that exits the VMM 0 without an agent report is a crash.
// ---------------------------------------------------------------------------

/// S-VM-42 / `@mandatory:mutation_target` (K1) — a guest whose `/sbin/init`
/// powers the machine off cleanly BEFORE ever beaconing READY makes
/// `cloud-hypervisor` exit `0`, yet the allocation lands
/// `Failed / VmGuestExitUnreported`, never `Terminated` with a completed
/// condition. This is the north-star refusal: `ExitKind` is NOT derived from
/// the hypervisor's own `0` exit.
///
/// ```gherkin
/// Given Ana has deployed a VM workload whose guest exits the VMM cleanly
///   without reporting an exit status over the beacon
/// When the hypervisor process exits with status 0
/// Then the allocation is Failed with TransitionReason::VmGuestExitUnreported
/// And the allocation is NOT Terminated with a completed condition
/// ```
#[tokio::test]
#[serial(cgroup)]
async fn guest_exit_without_agent_report_is_unreported_crash_never_completed() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-unreported-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let poweroff = build_poweroff_init_binary(tmp.path());
    let rootfs = stage_rootfs_replacing_init(tmp.path(), &fixture, &poweroff);

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-unreported.toml",
        // The command never runs: the replaced init powers off before exec.
        &vm_job_toml("vm-unreported", "/sbin/never", &fixture.kernel_path, &rootfs),
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the VM workload whose guest exits the VMM without reporting");

    // Comfortably exceeds VM_BOOT_DEADLINE (30s): a clean power-off resolves
    // start's boot race on its VMM-exit arm within seconds, so a 90s ceiling
    // proves the VMM EXITED (this arm) rather than the deadline elapsing.
    let (out, observed) =
        poll_until_failed_recording_states(&cfg, &submit.workload_id, Duration::from_secs(90))
            .await;
    let row = out.snapshot.rows.first().expect("one failed allocation row");

    // Never Running: the guest never beaconed READY.
    assert!(
        !observed.contains(&AllocStateWire::Running),
        "a guest that never beacons must NEVER pass through Running; observed: {observed:?}",
    );

    // Failed, not Terminated — the hypervisor's clean 0 did NOT become a
    // completed terminal.
    assert_eq!(
        row.state,
        AllocStateWire::Failed,
        "a VMM exit with no agent report must be Failed, never Terminated; reason={:?}",
        row.reason,
    );

    let reason = row.reason.clone().expect("a failed VM allocation must carry a structured reason");
    assert!(
        matches!(reason, TransitionReason::VmGuestExitUnreported { .. }),
        "the un-reported guest death must be named VmGuestExitUnreported, got {reason:?}",
    );
    // The hypervisor exited cleanly (0) — the signal path is untaken, which
    // is what makes this the "VMM exits 0, yet still a crash" refusal.
    assert!(
        matches!(reason, TransitionReason::VmGuestExitUnreported { vmm_signal: None, .. }),
        "the hypervisor exited (status 0), not signalled: {reason:?}",
    );

    // The complement of S-VM-44: this terminal is NOT the completed one.
    assert_ne!(
        row.state,
        AllocStateWire::Terminated,
        "a VMM exit with no agent report must never reach the completed terminal",
    );
    assert!(
        row.last_terminated.is_none()
            || !matches!(
                row.last_terminated.as_ref().and_then(|t| t.terminal.clone()),
                Some(overdrive_core::TerminalCondition::Completed { .. })
            ),
        "no completed condition may be attached to an un-reported guest death: {:?}",
        row.last_terminated,
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-VM-43 — a host-killed hypervisor is a crash and restarts like a process.
// ---------------------------------------------------------------------------

/// S-VM-43 / `@kpi:K5` — a hypervisor SIGKILL'd by the host (the exact
/// signal the cgroup OOM killer delivers) after the guest is Running lands
/// `Failed / WorkloadCrashedImmediately` — the SAME crash classification
/// `exit_observer::classify` gives a crashed **process**, finalised by the
/// SAME `WorkloadLifecycle` reconciler under the run-once Job contract,
/// consuming NO restart budget. This is the K5/[D3] north-star refusal from
/// the other direction to S-VM-42: a host kill of the VMM is a CRASH, never
/// swallowed as the VMM's clean exit and never an operator stop.
///
/// ```gherkin
/// Given Ana has deployed a VM workload and its hypervisor process is
///   killed by the host
/// When the platform observes the hypervisor exit without an agent report
/// Then the allocation is Failed
/// And the crash is treated by the same reconciler as a crashed process
///   workload (finalised run-once, no restart budget consumed)
/// ```
///
/// NOTE (design-vs-Gherkin, surfaced to acceptance-designer): the scenario's
/// original second Then said "same *ceiling*, same *backoff curve*", which
/// describes the Service restart-budget branch. A microVM is Job-only
/// (`[service] + [vm]` is rejected by S-VM-38) and a Job crash finalises
/// without restart by design, so that sub-clause is unobservable for a VM.
/// The observable **parity** asserted here is that a host-killed VM Job is
/// treated identically to a crashed process Job — Failed,
/// `WorkloadCrashedImmediately`, `restart_count == 0` — through the SAME
/// classifier and reconciler, with no VM-specific exit path.
#[tokio::test]
#[serial(cgroup)]
async fn host_killed_hypervisor_is_a_crash_treated_like_a_crashed_process() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-hostkill-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-hostkill.toml",
        &vm_job_toml("vm-hostkill", "/sbin/spin", &fixture.kernel_path, &rootfs),
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the long-lived VM workload");
    let running = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(90)).await;
    let alloc = alloc_id_of(&running);

    // The host kills the hypervisor with SIGKILL — the signal a cgroup OOM
    // kill delivers. `kill -KILL` needs no libc dep and runs as root here.
    let pid = cloud_hypervisor_pid_for_alloc(&alloc);
    let kill_status = Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .status()
        .expect("spawn kill -KILL against this allocation's hypervisor");
    assert!(kill_status.success(), "kill -KILL {pid} must succeed");

    // The platform observes the hypervisor exit without an agent report and
    // classifies it as a crash through the SAME exit_observer as a process
    // crash — Failed, not a clean exit and not an intentional stop.
    let (out, observed) =
        poll_until_failed_recording_states(&cfg, &submit.workload_id, Duration::from_secs(90))
            .await;
    let row = out.snapshot.rows.first().expect("one failed allocation row");

    assert_eq!(
        row.state,
        AllocStateWire::Failed,
        "a host-killed hypervisor must be Failed (a crash), never a clean terminal; observed: \
         {observed:?}, reason={:?}",
        row.reason,
    );

    let reason =
        row.reason.clone().expect("a crashed VM allocation must carry a structured reason");
    // The SAME reason a crashed PROCESS produces (`exit_observer::classify`'s
    // `Crashed` arm → `WorkloadCrashedImmediately`). The `signal` is left
    // unpinned: whether the drained VMM exit or the broken guest connection
    // resolves first in the watcher's biased race decides whether the signal
    // survives onto the row, and both orderings are a crash — the load-bearing
    // fact this scenario defends is the classification, not the signal byte.
    assert!(
        matches!(reason, TransitionReason::WorkloadCrashedImmediately { .. }),
        "a host-killed hypervisor must be classified WorkloadCrashedImmediately — the same crash \
         reason a killed process produces — got {reason:?}",
    );
    // Never swallowed as the boot-race un-reported death (S-VM-42's arm, which
    // only `start` produces before the guest is Running) and never counted as
    // an operator stop (S-VM-45's inverse).
    assert!(
        !matches!(
            reason,
            TransitionReason::VmGuestExitUnreported { .. } | TransitionReason::Stopped { .. }
        ),
        "a host-killed running guest is a crash, not a boot-race unreported death nor a stop: \
         {reason:?}",
    );

    // Treated identically to a crashed process Job: the run-once contract
    // finalises the crash and consumes NO restart budget — exactly what a
    // crashed process Job (which also does not restart) would show.
    assert_eq!(
        row.restart_count, 0,
        "a crashed VM Job is finalised run-once like a crashed process Job, consuming no restart \
         budget; got restart_count={}",
        row.restart_count,
    );
    assert!(
        out.snapshot.restart_budget.as_ref().is_none_or(|b| b.used == 0),
        "a crashed VM Job consumes no restart budget: {:?}",
        out.snapshot.restart_budget,
    );

    // No live hypervisor remains: the SIGKILL'd process is dead and the Job
    // did not restart, so there is nothing to reap. The crashed generation's
    // run-dir / clone residue is disposed by a later serve's VM-reclamation
    // boot pass (the same GC the sibling Tier-3 files rely on), not by this
    // test — mirroring production, where the crashed VMM's own watcher
    // performs no teardown.
    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-VM-44 — only an agent-reported exit produces the completed terminal.
// ---------------------------------------------------------------------------

/// S-VM-44 — a guest whose command exits `0` and reports it over the beacon
/// lands `Terminated` with `Stopped { by: Process }`, which
/// `WorkloadLifecycle::classify_natural_exit_terminal` maps to
/// `TerminalCondition::Completed { exit_code: 0 }`. `Stopped { by: Process }`
/// is produced by `exit_observer::classify` ONLY on the guest's
/// `ExitKind::CleanExit`, so its presence proves the completed terminal came
/// from the AGENT's report — never the hypervisor's own status.
///
/// ```gherkin
/// Given Ana has deployed a VM workload whose guest command exits 0 and
///   reports it
/// When the VM shuts down
/// Then the allocation is Terminated with completed exit code 0
/// ```
///
/// The original second Then — "this is the ONLY path in the workspace to
/// that state" — is a workspace-negative claim discharged as the
/// `@mandatory:mutation_target` on §105's join (module doc), whose COMPLEMENT
/// S-VM-42 (un-reported death is Failed) and S-VM-02 (a non-zero report is
/// Failed) assert.
#[tokio::test]
#[serial(cgroup)]
async fn agent_reported_exit_zero_reaches_the_completed_terminal() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-exit0-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let exit0 = build_exit_code_binary(tmp.path(), 0);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit0, "exit0");

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-exit0.toml",
        &vm_job_toml("vm-exit0", "/sbin/exit0", &fixture.kernel_path, &rootfs),
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the VM workload whose guest exits 0 and reports it");
    let out = poll_until_terminated(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let row = out.snapshot.rows.first().expect("one terminated allocation row");

    assert_eq!(
        row.state,
        AllocStateWire::Terminated,
        "an agent-reported clean exit must reach Terminated, got {:?} (reason={:?})",
        row.state,
        row.reason,
    );
    // The CleanExit-branch reason — `classify_natural_exit_terminal` maps
    // exactly this (`Terminated` + `Stopped { by: Process }`) to
    // `Completed { exit_code: 0 }`. A non-zero guest exit would be
    // `WorkloadCrashedImmediately` (S-VM-02) and a VMM death would be
    // `VmGuestExitUnreported` (S-VM-42) — neither reaches this reason.
    assert_eq!(
        row.reason,
        Some(TransitionReason::Stopped { by: StoppedBy::Process }),
        "only the guest's own EXIT 0 produces the completed terminal (Stopped by Process)",
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-VM-45 — an operator stop is never counted as a crash.
// ---------------------------------------------------------------------------

/// S-VM-45 — an operator stop lands `Terminated` attributed to the
/// operator/reconciler (NOT a crash) and consumes NO restart budget.
///
/// ```gherkin
/// Given Ana has a running VM workload
/// When she stops it with the operator stop verb
/// Then the allocation is Terminated as operator-stopped
/// And no restart budget is consumed
/// ```
#[tokio::test]
#[serial(cgroup)]
async fn operator_stop_is_terminated_and_consumes_no_restart_budget() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-opstop-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-opstop.toml",
        &vm_job_toml("vm-opstop", "/sbin/spin", &fixture.kernel_path, &rootfs),
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the long-lived VM workload");
    poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(90)).await;

    // The operator stop verb (`overdrive job stop`, per crates/overdrive-cli
    // CLAUDE.md the same `commands::deploy::stop` handler an [exec] workload
    // uses).
    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop the running VM workload with the operator stop verb");

    let out = poll_until_terminated(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    let row = out.snapshot.rows.first().expect("one terminated allocation row");

    assert_eq!(
        row.state,
        AllocStateWire::Terminated,
        "an operator stop must reach Terminated, got {:?} (reason={:?})",
        row.state,
        row.reason,
    );
    // Attributed to the operator/reconciler stop, never a crash.
    assert!(
        matches!(
            row.reason,
            Some(TransitionReason::Stopped { by: StoppedBy::Operator | StoppedBy::Reconciler }),
        ),
        "an operator stop must be attributed to Operator/Reconciler, never a crash: {:?}",
        row.reason,
    );
    assert!(
        !matches!(row.reason, Some(TransitionReason::WorkloadCrashedImmediately { .. })),
        "an operator stop is never a crash: {:?}",
        row.reason,
    );

    // No restart budget consumed — the reconciler exempts intentional stops
    // from the restart branch, so nothing ever incremented this allocation's
    // count or the workload-level budget.
    assert_eq!(
        row.restart_count, 0,
        "an operator stop must consume no restart budget (restart_count), got {}",
        row.restart_count,
    );
    assert!(
        out.snapshot.restart_budget.as_ref().is_none_or(|b| b.used == 0),
        "an operator stop must leave the restart budget unused: {:?}",
        out.snapshot.restart_budget,
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-VM-46 — an operator stop drives the graceful-shutdown sequence to the
// operator-stop terminal, the SAME terminal a process workload reaches.
// ---------------------------------------------------------------------------

/// S-VM-46 / `@ac-12` — stopping a running VM workload drives the ADR-0082
/// §D4 graceful-shutdown sequence — `VmDriver::stop` writes `SHUTDOWN` on
/// the guest's already-open beacon connection BEFORE escalating to
/// `Vmm::terminate` (AC-1) — and the allocation lands
/// `Terminated / Stopped { by: Operator }`, the SAME driver-agnostic
/// terminal a stopped process (`ExecDriver`) workload reaches. The parity
/// in the scenario title is structural: there is no VM-specific stop
/// reason — `TransitionReason::Stopped { by: Operator }` is the one reason
/// every driver's operator stop produces, so a VM reaching it IS parity.
///
/// **First real evidence for the host→guest `SHUTDOWN` write** (ADR-0082
/// §D4, `findings.md:2787` — the spike exercised the vsock connection
/// guest→host only). This is the first scenario to run the real `stop()`
/// — which writes `SHUTDOWN\n` on a real, live beacon connection held to a
/// real guest — end to end against a real Cloud Hypervisor VMM. It is a
/// mechanism proof, not a regression guard.
///
/// With the shipped `overdrive-init`, a guest running a long-lived command
/// is blocked in `exec_operator_command` and never reaches its
/// post-command `read_shutdown_or_eof`, so the best-effort `SHUTDOWN` write
/// is not consumed by the busy guest and the `Vmm::terminate` escalation is
/// what stops the VM (concurrent-`SHUTDOWN`-during-execution is deferred —
/// `overdrive-init` module doc). The observable is invariant across that
/// detail: the allocation lands the operator-stop terminal, never a crash.
///
/// ```gherkin
/// Given Ana has a running VM workload
/// When she runs the operator stop verb
/// Then the guest is asked to shut down gracefully over its open vsock
///   connection
/// And the allocation reaches Terminated as operator-stopped
/// ```
#[tokio::test]
#[serial(cgroup)]
async fn stopping_a_vm_reaches_the_operator_stop_terminal_like_a_process() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-stop46-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-stop46.toml",
        &vm_job_toml("vm-stop46", "/sbin/spin", &fixture.kernel_path, &rootfs),
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the long-lived VM workload");
    poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(90)).await;

    // The operator stop verb drives VmDriver::stop's §D4 sequence: the
    // SHUTDOWN write on the beacon, then VM_SHUTDOWN_REQUEST_DEADLINE, then
    // Vmm::terminate. Same `commands::deploy::stop` handler an [exec]
    // workload uses (crates/overdrive-cli CLAUDE.md).
    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop the running VM workload with the operator stop verb");

    let out = poll_until_terminated(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    let row = out.snapshot.rows.first().expect("one terminated allocation row");

    assert_eq!(
        row.state,
        AllocStateWire::Terminated,
        "an operator stop must reach Terminated, got {:?} (reason={:?})",
        row.state,
        row.reason,
    );
    // The operator-stop terminal — the SAME reason a stopped process
    // workload produces (there is no VM-specific stop reason). Reconciler
    // is accepted alongside Operator for the reason S-VM-45 accepts it: the
    // terminal is authored on the operator-stop intent, not a VM path.
    assert!(
        matches!(
            row.reason,
            Some(TransitionReason::Stopped { by: StoppedBy::Operator | StoppedBy::Reconciler }),
        ),
        "a VM operator stop must reach the operator-stop terminal, never a crash: {:?}",
        row.reason,
    );
    assert!(
        !matches!(row.reason, Some(TransitionReason::WorkloadCrashedImmediately { .. })),
        "the graceful-then-escalate stop sequence is never a crash: {:?}",
        row.reason,
    );
    // Converges like any other stopped workload: no restart, no budget
    // consumed (the reconciler exempts the operator-stop intent from the
    // restart branch), exactly as a stopped process workload shows.
    assert_eq!(
        row.restart_count, 0,
        "an operator-stopped VM consumes no restart budget (restart_count), got {}",
        row.restart_count,
    );
    assert!(
        out.snapshot.restart_budget.as_ref().is_none_or(|b| b.used == 0),
        "an operator-stopped VM leaves the restart budget unused: {:?}",
        out.snapshot.restart_budget,
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-VM-47 — an unresponsive guest is stopped within the bounded grace and
// is never classified a crash.
// ---------------------------------------------------------------------------

/// S-VM-47 / `@ac-12` `@error_path` — a running VM whose guest ignores the
/// `SHUTDOWN` request is still stopped within the bounded grace
/// (`VM_SHUTDOWN_REQUEST_DEADLINE` 2s + `VM_STOP_GRACE` 10s), escalated to
/// `Vmm::terminate`'s SIGKILL, and lands `Terminated / Stopped { by:
/// Operator }` — NEVER `WorkloadCrashedImmediately`, even though the VMM
/// dies to a SIGKILL that in isolation looks exactly like the host kill
/// S-VM-43 classifies as a crash. The operator-stop intent is what refuses
/// the crash classification.
///
/// The shipped `overdrive-init` IS the "guest ignores shutdown requests"
/// Given: it is blocked in `exec_operator_command` running the long-lived
/// `/sbin/spin` child and never reaches its post-command
/// `read_shutdown_or_eof`, so the host's best-effort `SHUTDOWN` write is
/// unconsumed and the `Vmm::terminate` escalation is the only thing that
/// stops the VM. Without `VM_SHUTDOWN_REQUEST_DEADLINE` bounding step 1 and
/// `VM_STOP_GRACE` bounding step 2, such a guest would let `stop` hang
/// indefinitely — the bound is exactly what this scenario defends (ADR-0082
/// §D4, "Without the step-1 deadline this ADR's own claim has no
/// mechanism").
///
/// ```gherkin
/// Given Ana has a running VM workload whose guest ignores shutdown requests
/// When she runs the operator stop verb
/// Then the allocation reaches Terminated as operator-stopped within the
///   grace period
/// And it is NOT classified as a crash
/// ```
#[tokio::test]
#[serial(cgroup)]
async fn unresponsive_guest_is_stopped_within_bounded_grace_never_a_crash() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-stop47-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-stop47.toml",
        &vm_job_toml("vm-stop47", "/sbin/spin", &fixture.kernel_path, &rootfs),
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the long-lived VM workload whose guest ignores shutdown");
    poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(90)).await;

    // VM_SHUTDOWN_REQUEST_DEADLINE (2s) + VM_STOP_GRACE (10s) = 12s is the
    // driver-side escalation bound (both private consts in vm_driver.rs).
    // The operator-observable terminal adds the reconciler observe -> emit
    // StopAllocation -> action-shim -> author-terminal latency on top, so
    // the end-to-end ceiling is padded to 30s (the window S-VM-45 already
    // proves sufficient for this same spin-guest stop path). A hang — the
    // failure mode the two constants exist to prevent — would blow past
    // both this ceiling and the 60s poll window below.
    let bounded_grace_ceiling = Duration::from_secs(30);

    let started = tokio::time::Instant::now();
    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop the VM workload whose guest ignores the shutdown request");
    let out = poll_until_terminated(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let elapsed = started.elapsed();
    let row = out.snapshot.rows.first().expect("one terminated allocation row");

    assert_eq!(
        row.state,
        AllocStateWire::Terminated,
        "an unresponsive guest's operator stop must reach Terminated, got {:?} (reason={:?})",
        row.state,
        row.reason,
    );
    // Bounded grace: the escalation lands the terminal well within the
    // constant-derived ceiling rather than hanging on the unresponsive
    // guest — the property VM_SHUTDOWN_REQUEST_DEADLINE + VM_STOP_GRACE
    // exist to guarantee.
    assert!(
        elapsed <= bounded_grace_ceiling,
        "an unresponsive guest must reach Terminated within the bounded grace \
         (2s deadline + 10s grace + reconciler slack, ceiling {bounded_grace_ceiling:?}); \
         took {elapsed:?}",
    );
    // Still an operator stop, never a crash — even though the VMM died to a
    // SIGKILL indistinguishable in isolation from S-VM-43's host kill. The
    // operator-stop intent wins the classification.
    assert!(
        matches!(
            row.reason,
            Some(TransitionReason::Stopped { by: StoppedBy::Operator | StoppedBy::Reconciler }),
        ),
        "an unresponsive guest's stop is still an operator stop, never a crash: {:?}",
        row.reason,
    );
    assert!(
        !matches!(row.reason, Some(TransitionReason::WorkloadCrashedImmediately { .. })),
        "the SIGKILL escalation of an operator stop must NOT be classified a crash: {:?}",
        row.reason,
    );
    // No restart budget consumed — an operator stop is exempt from the
    // restart branch regardless of how the VMM ultimately died.
    assert_eq!(
        row.restart_count, 0,
        "an operator-stopped VM consumes no restart budget (restart_count), got {}",
        row.restart_count,
    );
    assert!(
        out.snapshot.restart_budget.as_ref().is_none_or(|b| b.used == 0),
        "an operator-stopped VM leaves the restart budget unused: {:?}",
        out.snapshot.restart_budget,
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-VM-48 — a restarted VM boots from a clean, unmodified rootfs copy.
// ---------------------------------------------------------------------------

/// S-VM-48 / `@ac-12` `@edge_case` — a VM workload that MODIFIED its rootfs
/// clone and was then RESTARTED boots from a fresh `FICLONE` copy of the
/// operator's original artifact (the prior modification is absent), and the
/// operator's artifact file on the host is byte-unchanged across the whole
/// restart cycle.
///
/// # Design-real restart trigger (reframed from the DISTILL Gherkin)
///
/// The DISTILL Gherkin's trigger — "a VM workload CRASHED … the platform
/// RESTARTS the allocation under backoff" — is NOT producible for a microVM:
/// a microVM is Job-only (`[service] + [vm]` is rejected, S-VM-38) and a Job
/// crash finalises RUN-ONCE with no restart-under-backoff path
/// (`workload_lifecycle.rs`'s Job-kind natural-exit handler; the SAME fact
/// S-VM-43 documents from the other direction). This scenario proves the SAME
/// observable invariant through a restart path that DOES exist for a Job-kind
/// VM: the phase-02 **platform-reclamation restart** (a platform-reclaimed
/// Job whose intent still stands is re-driven by `WorkloadLifecycle` via
/// `Action::RestartAllocation`, DD-1) — the exact boot-epoch
/// reclaim-then-restart cycle `vm_reclamation_tier3.rs`'s S-VM-28 drives. That
/// restart re-invokes `CloudHypervisorVmm::create`, whose per-launch
/// `ficlone_rootfs` clones the read-only master afresh (removing any prior
/// clone first) and never mutates the master. This is a PROOF of that
/// already-wired mechanism end to end through the operator surface, not a
/// build — no production file is modified.
///
/// # Why the assertions are non-vacuous
///
/// The guest MODIFIES its rootfs on a fresh clone (writes a marker, then
/// re-reads it to confirm the write actually landed on the mounted block
/// device — a silent drop exits loudly, so boot #1 reaching `Running` PROVES
/// the modification is real), then spins. On the RESTART boot the guest sees
/// NO marker (fresh clone) and spins again → `Running` with
/// `restart_count == 1`. Had the restart booted the MUTATED clone, the guest
/// would find the marker and exit `66` → a crash terminal, and
/// `poll_until_restarted` would time out. So the restart reaching `Running`
/// with a bumped restart count IS the proof the boot was from a clean copy.
///
/// ```gherkin
/// Given a VM workload modified its rootfs, then its allocation was
///   platform-reclaimed while its intent still stood
/// When the platform restarts the allocation
/// Then the new allocation boots from an unmodified copy of the operator's
///   original artifact (the prior modification is absent)
/// And the operator's artifact file on the host is byte-unchanged
/// ```
#[tokio::test]
#[serial(cgroup)]
async fn restarted_vm_boots_from_a_clean_unmodified_rootfs_copy() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-clean-rootfs-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let marker = build_marker_or_spin_binary(tmp.path());
    // The operator's rootfs artifact = a per-test COPY of the fixture rootfs
    // with the marker guest injected at /sbin/marker. This staged file IS the
    // FICLONE master; the clone lands beside it (RootfsPlan::for_alloc).
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &marker, "marker");

    // The operator's artifact, fingerprinted BEFORE any launch.
    let fingerprint_before = rootfs_fingerprint(&rootfs);

    // data_dir on the reflink staging root — the per-launch clone is FICLONE'd
    // into `clone_staging_dir(data_dir)` and FICLONE is intra-filesystem
    // (ADR-0082 2026-08-18 fourth amendment); tmpfs would fail `EXDEV`.
    let server_tmp = server_tmp_on_staging_root();
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");
    let cfg = config_path(server_tmp.path());

    // Boot #1 -- deploy the marker guest; it modifies its rootfs clone and
    // reaches Running. Reaching Running PROVES the modification landed (a
    // failed/dropped write exits 66/77/78 -> the alloc never reaches Running).
    let handle = spawn_vm_server_at(&data_dir, &config_dir).await;
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-clean-rootfs.toml",
        &vm_job_toml("vm-clean-rootfs", "/sbin/marker", &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the VM workload whose guest modifies its rootfs then spins");
    let baseline = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(90)).await;
    assert_eq!(
        baseline.snapshot.rows.first().expect("one running row").restart_count,
        0,
        "sanity: a first start is not a restart",
    );

    // Unclean shutdown -- NEVER stop(). The workload's intent still stands
    // (DD-1); the real cloud-hypervisor process survives (kill_on_drop(false))
    // and the row stays non-terminal, so boot #2's boot-epoch VmReclamation
    // reclaims it and the SAME live serve session's WorkloadLifecycle
    // re-drives it (S-VM-28's reclaim-then-restart cycle).
    handle.shutdown().await.expect("shutdown boot #1 without stopping the workload");
    wait_for_data_dir_release().await;

    // Boot #2 -- SAME data_dir. The boot-epoch reclaim discards the mutated
    // clone; the intent-still-stands re-drive restarts the allocation,
    // re-invoking create() -> a FRESH FICLONE of the read-only master.
    let handle2 = spawn_vm_server_at(&data_dir, &config_dir).await;

    // The restarted guest booted from a fresh clone: it found NO marker and
    // spun again -> Running with restart_count == 1. Had it booted the mutated
    // clone, it would exit 66 (crash) and this poll would time out.
    let restarted = poll_until_restarted(&cfg, &submit.workload_id, Duration::from_secs(120)).await;
    let restarted_row = restarted.snapshot.rows.first().expect("one row after the restart");
    assert_eq!(
        restarted_row.restart_count, 1,
        "the reclaim-then-restart cycle must bump restart_count to exactly 1; got {restarted_row:?}",
    );
    // Explicit complement: the restarted allocation is Running (it booted the
    // clean copy), NEVER a crash terminal (which a stale-marker boot produces).
    assert_eq!(
        restarted_row.state,
        AllocStateWire::Running,
        "the restart must boot the clean copy and reach Running, never a crash; reason={:?}",
        restarted_row.reason,
    );
    assert!(
        !matches!(restarted_row.reason, Some(TransitionReason::WorkloadCrashedImmediately { .. })),
        "a restart booting a clean rootfs is never a crash (a stale-marker boot would be): {:?}",
        restarted_row.reason,
    );

    // S-VM-48's second Then, host-direct: the operator's original artifact is
    // byte-unchanged across the whole modify-then-restart cycle. FICLONE only
    // ever READS the master (copy-on-write); the guest's writes land on its
    // clone, never the master.
    let fingerprint_after = rootfs_fingerprint(&rootfs);
    assert_eq!(
        fingerprint_after, fingerprint_before,
        "the operator's rootfs artifact must be byte-unchanged (len,hash) before and after the \
         restart cycle; before={fingerprint_before:?} after={fingerprint_after:?}",
    );

    // Reap the restarted (still-live) spin VM via the production stop path
    // before shutdown: kill_on_drop(false) means nothing kills a still-Running
    // VM merely because this process exits (same leak class the sibling
    // long-lived-spin scenarios guard against).
    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop the restarted marker workload before shutdown to avoid leaking the VMM");
    poll_until_terminated(&cfg, &submit.workload_id, Duration::from_secs(30)).await;

    handle2.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// AC-13 (US-VM-7) — the hypervisor process is confined, or it does not run.
// S-VM-49 / S-VM-50 / S-VM-53. These prove ADR-0082's confinement application
// (the `prlimit … -- setpriv … --` launch wrapper + `--landlock` ruleset)
// end-to-end through the production `overdrive serve` + `overdrive deploy`
// path against a REAL Cloud Hypervisor VMM. No production file is modified by
// the tests themselves; they read the confined process's own `/proc` surface.
// ---------------------------------------------------------------------------

/// `/proc/<pid>/status`'s `Uid:` / `Gid:` lines → `(real_uid, eff_uid,
/// real_gid, eff_gid)`. The thread-group LEADER's status reports the whole
/// PROCESS's uid/gid — uid-drop is process-wide, unlike seccomp which installs
/// per-thread (spike P5 correction 1) — so the leader is the correct read for
/// the uid/gid assertions here.
fn proc_status_ids(pid: u32) -> (u32, u32, u32, u32) {
    let status =
        std::fs::read_to_string(format!("/proc/{pid}/status")).expect("read /proc/<pid>/status");
    let cols = |rest: &str| -> Vec<u32> {
        rest.split_whitespace().filter_map(|c| c.parse::<u32>().ok()).collect()
    };
    let mut uid: Option<(u32, u32)> = None;
    let mut gid: Option<(u32, u32)> = None;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let c = cols(rest);
            uid = Some((c[0], c[1]));
        } else if let Some(rest) = line.strip_prefix("Gid:") {
            let c = cols(rest);
            gid = Some((c[0], c[1]));
        }
    }
    let (real_uid, eff_uid) = uid.expect("/proc status carries a Uid: line");
    let (real_gid, eff_gid) = gid.expect("/proc status carries a Gid: line");
    (real_uid, eff_uid, real_gid, eff_gid)
}

/// A named `/proc/<pid>/limits` row → `(soft, hard)`, with `unlimited` mapped
/// to `u64::MAX`. `limit_name` matches the row label exactly (`"Max file
/// size"`, `"Max open files"`); the two columns after the label are the soft
/// and hard limits.
fn proc_limit(pid: u32, limit_name: &str) -> (u64, u64) {
    let limits =
        std::fs::read_to_string(format!("/proc/{pid}/limits")).expect("read /proc/<pid>/limits");
    let parse = |tok: &str| -> u64 {
        if tok == "unlimited" {
            u64::MAX
        } else {
            tok.parse::<u64>().expect("numeric limit column")
        }
    };
    for line in limits.lines() {
        if let Some(rest) = line.strip_prefix(limit_name) {
            let c: Vec<&str> = rest.split_whitespace().collect();
            return (parse(c[0]), parse(c[1]));
        }
    }
    panic!("no {limit_name:?} row in /proc/{pid}/limits");
}

/// `/proc/<pid>/cmdline` split into argv tokens (NUL-separated, empties
/// dropped). After the `prlimit → setpriv → cloud-hypervisor` execve chain
/// this is the FINAL `cloud-hypervisor` argv — the wrapper images are
/// replaced in place (same pid), so the hypervisor's own flags (including
/// `--landlock` / `--landlock-rules`) are what remain.
fn proc_cmdline_args(pid: u32) -> Vec<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).expect("read /proc/<pid>/cmdline");
    raw.split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Every value that immediately FOLLOWS a `--landlock-rules` token in `args`
/// — the EXPLICIT Landlock grants the platform passed (CH's auto-derived
/// kernel/disk/serial/api grants are internal and never appear here).
fn explicit_landlock_rules(args: &[String]) -> Vec<String> {
    args.windows(2).filter(|w| w[0] == "--landlock-rules").map(|w| w[1].clone()).collect()
}

/// Deploy a long-lived spin VM and return `(handle, server_tmp, cfg, workload,
/// alloc, vmm_pid)` once it is Running — the shared setup S-VM-49/50/53 need to
/// observe the confined hypervisor's live `/proc` surface. `rootfs_prefix`
/// controls the per-test tempdir prefix so S-VM-50 can stage its rootfs at a
/// distinctly-named (operator-declared) path.
async fn deploy_running_spin_vm(
    fixture: &VmFixture,
    rootfs_prefix: &str,
    workload_id: &str,
) -> (ServeHandle, TempDir, PathBuf, AllocationId, u32, PathBuf, TempDir) {
    let tmp = tempfile::Builder::new()
        .prefix(rootfs_prefix)
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), fixture, &spin, "spin");

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        &format!("{workload_id}.toml"),
        &vm_job_toml(workload_id, "/sbin/spin", &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the long-lived VM workload");
    let running = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(90)).await;
    let alloc = alloc_id_of(&running);
    let vmm_pid = cloud_hypervisor_pid_for_alloc(&alloc);
    // M3: the rootfs-staging tempdir guard (`tmp`) is RETURNED, not
    // `.keep()`-leaked — the caller holds it for the test's lifetime and it is
    // RAII-cleaned on drop, so staged rootfs copies never accumulate on the
    // shared metal box across runs.
    (handle, server_tmp, cfg, alloc, vmm_pid, rootfs, tmp)
}

/// Reap the still-Running spin VM through the production stop path, then shut
/// the server down — `kill_on_drop(false)` means nothing kills a Running VM
/// merely because this process exits (the same leak class the sibling
/// long-lived-spin scenarios guard against).
async fn stop_and_shutdown(handle: ServeHandle, cfg: &Path, workload_id: &str) {
    stop(StopArgs { id: workload_id.to_owned(), config_path: cfg.to_owned() })
        .await
        .expect("stop the running VM workload before shutdown to avoid leaking the VMM");
    poll_until_terminated(cfg, workload_id, Duration::from_secs(30)).await;
    handle.shutdown().await.expect("clean shutdown");
}

/// S-VM-49 / `@kpi:K7` — an untrusted VM workload runs with a bounded,
/// non-root, Landlock-confined hypervisor. The confined process reports a
/// non-zero real AND effective uid/gid, resource limits strictly below the
/// NAMED `overdrive serve` process (by explicit numeric pid, never
/// `/proc/self`), and a Landlock ruleset whose ONLY explicit grant is the
/// run-directory read-write grant (C-4 — the vsock socket CH does not
/// auto-derive a rule for).
///
/// ```gherkin
/// Given Ana has deployed a VM workload on a host that supports the required
///   confinement
/// When the allocation reaches Running
/// Then /proc/<vmm-pid>/status reports a non-zero real AND effective Uid and Gid
/// And /proc/<vmm-pid>/limits reports Max file size and Max open files strictly
///   below the SAME fields on the overdrive serve process
/// And the hypervisor was launched under a Landlock ruleset naming that
///   allocation's own kernel, rootfs copy and API socket by CH's auto-derived
///   grants, PLUS a directory read-write grant on that allocation's own run
///   directory, and nothing outside those grants
/// ```
#[tokio::test]
#[serial(cgroup)]
async fn hypervisor_runs_bounded_nonroot_and_landlock_confined() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let (handle, _server_tmp, cfg, alloc, vmm_pid, _rootfs, _rootfs_tmp) =
        deploy_running_spin_vm(&fixture, "vm-confined-", "vm-confined").await;

    // --- non-zero real AND effective uid/gid (never root) ---
    let (real_uid, eff_uid, real_gid, eff_gid) = proc_status_ids(vmm_pid);
    assert!(
        real_uid != 0 && eff_uid != 0 && real_gid != 0 && eff_gid != 0,
        "the confined hypervisor must run non-root real AND effective uid/gid; got \
         ruid={real_uid} euid={eff_uid} rgid={real_gid} egid={eff_gid}",
    );

    // --- limits strictly below the NAMED serve process (never /proc/self) ---
    // This in-process `overdrive serve` runs inside the test process, so the
    // serve process IS `std::process::id()`; its limits are read by explicit
    // numeric pid, never the `/proc/self` symlink (US-VM-7 AC note).
    let serve_pid = std::process::id();
    let (_, vmm_fsize_hard) = proc_limit(vmm_pid, "Max file size");
    let (_, serve_fsize_hard) = proc_limit(serve_pid, "Max file size");
    assert!(
        vmm_fsize_hard < serve_fsize_hard,
        "confined Max file size must be strictly below serve's; vmm={vmm_fsize_hard} \
         serve={serve_fsize_hard}",
    );
    let (_, vmm_nofile_hard) = proc_limit(vmm_pid, "Max open files");
    let (_, serve_nofile_hard) = proc_limit(serve_pid, "Max open files");
    assert!(
        vmm_nofile_hard < serve_nofile_hard,
        "confined Max open files must be strictly below serve's; vmm={vmm_nofile_hard} \
         serve={serve_nofile_hard}",
    );

    // --- Landlock: --landlock + EXACTLY the run-directory rw grant ---
    let args = proc_cmdline_args(vmm_pid);
    assert!(
        args.iter().any(|a| a == "--landlock"),
        "the hypervisor must be launched with --landlock; argv={args:?}",
    );
    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), &alloc);
    let expected_rule = format!("path={},access=rw", run_dir.path().display());
    assert_eq!(
        explicit_landlock_rules(&args),
        vec![expected_rule],
        "the ONLY explicit Landlock grant must be the run-directory read-write grant (C-4), and \
         nothing outside it; argv={args:?}",
    );

    stop_and_shutdown(handle, &cfg, "vm-confined").await;
}

/// S-VM-50 — the confinement ruleset follows the operator's declared artifact
/// paths, never a hardcoded directory. A rootfs declared OUTSIDE the default
/// artifact location still boots under confinement: were the disk Landlock
/// grant hardcoded to a default dir, the confined hypervisor could not reach
/// THIS path and the boot would fail. CH auto-derives the `--disk` grant from
/// the actual disk path, so a spec-declared path just works — the falsifiable
/// half of "derived, not hardcoded".
///
/// ```gherkin
/// Given Ana's rootfs lives outside the default artifact directory
/// When the allocation starts
/// Then the VM boots successfully
/// And the hypervisor can reach the declared kernel and rootfs and nothing else
/// ```
#[tokio::test]
#[serial(cgroup)]
async fn confinement_ruleset_follows_declared_rootfs_path_not_a_hardcoded_dir() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    // Staged under a distinctly-named ("outside the default artifact dir")
    // subtree on the same reflink-capable filesystem (FICLONE is
    // intra-filesystem). Reaching Running IS the assertion: the confined boot
    // succeeded despite the non-default path.
    let (handle, server_tmp, cfg, _alloc, vmm_pid, rootfs, _rootfs_tmp) =
        deploy_running_spin_vm(&fixture, "vm-outside-artifact-dir-", "vm-outside").await;

    // The boot is genuinely confined (not an unconfined root fall-through that
    // would boot any path): --landlock is present, and the --disk clone lives in
    // the PLATFORM-OWNED staging dir (derived from the node's data_dir), NEVER
    // beside the operator's declared rootfs. That the VM booted (reached
    // Running) from a rootfs declared OUTSIDE the default artifact dir proves
    // the rootfs SOURCE is derived from the spec, not hardcoded; that the clone
    // sits in the platform staging dir is the ADR-0082 fourth-amendment
    // relocation (the confined identity never traverses the operator's dir).
    let args = proc_cmdline_args(vmm_pid);
    assert!(
        args.iter().any(|a| a == "--landlock"),
        "the boot must be confined (--landlock present), not an unconfined root boot; argv={args:?}",
    );
    let declared_dir = rootfs.parent().expect("declared rootfs has a parent directory");
    let staging_dir =
        overdrive_core::vm::config::clone_staging_dir(&server_tmp.path().join("data"));
    let disk_arg = args
        .windows(2)
        .find(|w| w[0] == "--disk")
        .map(|w| w[1].clone())
        .expect("a --disk argument in the hypervisor argv");
    assert!(
        disk_arg.contains(&staging_dir.display().to_string()),
        "the --disk clone must live in the platform-owned staging dir {staging_dir:?} (derived from \
         the node data_dir), proving the ruleset follows the actual disk path; disk={disk_arg}",
    );
    assert!(
        !disk_arg.contains(&declared_dir.display().to_string()),
        "the --disk clone must NOT sit beside the operator's declared rootfs {declared_dir:?} -- the \
         B1 fix stages it in a platform dir so the confined identity never traverses the operator's \
         directory; disk={disk_arg}",
    );

    // The confined hypervisor is not root — the same bound S-VM-49 pins, here
    // proving the non-default path did not silently disable confinement.
    let (real_uid, eff_uid, ..) = proc_status_ids(vmm_pid);
    assert!(
        real_uid != 0 && eff_uid != 0,
        "a non-default rootfs path must not disable confinement; ruid={real_uid} euid={eff_uid}",
    );

    stop_and_shutdown(handle, &cfg, "vm-outside").await;
}

/// S-VM-53 / `@correction:C-4` — the vsock socket's Landlock grant is a
/// DIRECTORY grant, scoped to nothing else. The run directory holds nothing
/// but this VM's own sockets, logs, and its own kernel copy (ADR-0082
/// 2026-08-18 fourth amendment (c-fix.1) copies the operator kernel into the
/// run dir), and the ruleset grants read-write on that directory (CH does NOT
/// auto-derive a rule for the vsock socket it binds itself, unlike `--kernel`
/// / `--disk` / `--serial file=` / `--api-socket`). The directory-exclusivity
/// property (SD-2) is what makes the grant derivable rather than a list a
/// crafter must remember.
///
/// ```gherkin
/// Given Ana has deployed a VM workload
/// When the hypervisor is launched
/// Then the run directory holds nothing but this VM's own sockets and logs
/// And the Landlock ruleset grants read-write on that directory
/// ```
#[tokio::test]
#[serial(cgroup)]
async fn vsock_landlock_grant_is_the_run_directory_scoped_to_nothing_else() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let (handle, _server_tmp, cfg, alloc, vmm_pid, _rootfs, _rootfs_tmp) =
        deploy_running_spin_vm(&fixture, "vm-vsock-grant-", "vm-vsock-grant").await;

    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), &alloc);

    // The run directory holds NOTHING but this VM's own sockets and logs.
    let entries: Vec<String> = std::fs::read_dir(run_dir.path())
        .expect("read the allocation's run directory")
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !entries.is_empty(),
        "the run directory must hold this VM's own sockets/logs, it is empty: {}",
        run_dir.path().display(),
    );
    for name in &entries {
        // This VM's own files: CH's main vsock UDS (`vsock`), API socket
        // (`api`) and the API-socket lock CH writes beside it (`api.lock`),
        // the serial capture (`console.log`), the driver-bound beacon socket
        // (`vsock_<port>`), and THIS allocation's own `kernel` copy — ADR-0082
        // 2026-08-18 fourth amendment (c-fix.1) refines SD-2's "holds nothing
        // else" to "sockets, console log, and this allocation's own kernel
        // copy". All belong to THIS allocation — nothing foreign shares the
        // directory (SD-2).
        let is_own =
            matches!(name.as_str(), "vsock" | "api" | "api.lock" | "console.log" | "kernel")
                || name.starts_with("vsock_");
        assert!(
            is_own,
            "the run directory must hold ONLY this VM's own sockets and logs; found a foreign \
             entry {name:?} among {entries:?}",
        );
    }

    // The ONLY explicit Landlock grant is a read-write DIRECTORY grant on that
    // run directory (C-4). CH auto-derives kernel/disk/serial/api grants; the
    // vsock socket it binds itself is the one it omits, so the platform grants
    // the CONTAINING DIRECTORY (CH rejects a not-yet-existent socket path).
    let args = proc_cmdline_args(vmm_pid);
    let expected_rule = format!("path={},access=rw", run_dir.path().display());
    assert_eq!(
        explicit_landlock_rules(&args),
        vec![expected_rule],
        "the ONLY explicit Landlock grant must be a read-write directory grant on the run dir \
         (C-4), scoped to nothing else; argv={args:?}",
    );

    stop_and_shutdown(handle, &cfg, "vm-vsock-grant").await;
}

// ---------------------------------------------------------------------------
// B1 regression guard (ADR-0082 2026-08-18 fourth amendment) — a confined
// deploy MUST NOT mutate the operator's OWN kernel/rootfs artifacts, in bytes
// OR in permission mode. Closes the gap S-VM-48 left open: S-VM-48 fingerprints
// the rootfs BYTES only, so the prior 04-04 impl's `o+r` on the operator kernel
// and `o+x` on its directories (a world-widening DAC regression that leaked
// across allocations and survived teardown) shipped green. This guard is RED
// against that regression and GREEN once the kernel is COPIED into the run dir
// and the rootfs clone is staged in a platform-owned directory.
// ---------------------------------------------------------------------------

/// `(mode_bits, len, content_hash)` of a file — the permission mode PLUS the
/// [`rootfs_fingerprint`] byte identity, so a guard proves an operator artifact
/// is unchanged in BOTH its permission bits and its bytes.
fn artifact_mode_and_bytes(path: &Path) -> (u32, u64, u64) {
    let mode =
        std::fs::metadata(path).expect("stat operator artifact").permissions().mode() & 0o7777;
    let (len, hash) = rootfs_fingerprint(path);
    (mode, len, hash)
}

/// The permission mode bits of a path (file or directory).
fn path_mode(path: &Path) -> u32 {
    std::fs::metadata(path).expect("stat path for mode").permissions().mode() & 0o7777
}

/// B1 regression guard (`@security` / ADR-0082 fourth amendment) — a confined
/// deploy cycle leaves the operator's OWN kernel and rootfs masters, AND their
/// containing directories, byte-identical AND mode-identical. The prior 04-04
/// impl reached the uid-dropped hypervisor's artifacts by adding `o+r` to the
/// operator kernel FILE and `o+x` to the operator kernel/image DIRECTORIES — a
/// world-widening DAC regression that leaked across allocations and survived
/// teardown, exposing adjacent rootfs masters and secrets by name. The
/// amendment forbids ALL operator-artifact mutation: the kernel is COPIED into
/// the per-alloc run dir and `chown`'d there, and the rootfs clone is FICLONE'd
/// into a platform-owned staging dir — so the operator's own files are only
/// ever OPENED READ-ONLY by root. This guard is RED against the regression
/// (the operator kernel's mode changes `0o600 → 0o604`) and GREEN after the
/// fix. Per-test operator artifacts (not the shared fixture) at the common
/// `0o600` posture keep the before/after capture clean and isolated.
///
/// ```gherkin
/// Given Ana's kernel and rootfs are 0o600 operator-owned files
/// When she deploys a VM workload that reaches Running under confinement
/// Then her kernel and rootfs files, and their directories, are unchanged in
///   both bytes and permission mode
/// ```
#[tokio::test]
#[serial(cgroup)]
async fn confined_deploy_leaves_operator_kernel_and_rootfs_mode_and_bytes_unchanged() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-operator-artifacts-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");

    // Per-test OPERATOR artifacts at the common `0o600` operator posture: a
    // byte-for-byte kernel copy and a rootfs copy with the spin guest injected.
    // These are the operator-owned masters the confined hypervisor must reach
    // WITHOUT the platform touching their mode or bytes.
    let kernel = tmp.path().join("operator-kernel");
    std::fs::copy(&fixture.kernel_path, &kernel).expect("stage a per-test operator kernel copy");
    std::fs::set_permissions(&kernel, std::fs::Permissions::from_mode(0o600))
        .expect("set the operator kernel to the common 0o600 posture");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");
    std::fs::set_permissions(&rootfs, std::fs::Permissions::from_mode(0o600))
        .expect("set the operator rootfs to the common 0o600 posture");

    let kernel_before = artifact_mode_and_bytes(&kernel);
    let rootfs_before = artifact_mode_and_bytes(&rootfs);
    let kernel_dir_before = path_mode(kernel.parent().expect("kernel has a parent dir"));
    let rootfs_dir_before = path_mode(rootfs.parent().expect("rootfs has a parent dir"));

    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-operator-artifacts.toml",
        &vm_job_toml("vm-operator-artifacts", "/sbin/spin", &kernel, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the confined VM workload sourcing per-test operator artifacts");
    // Reaching Running proves the confined hypervisor DID reach its kernel and
    // rootfs -- via platform-owned copies/clones, not by widening the operator's.
    poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(90)).await;

    assert_eq!(
        artifact_mode_and_bytes(&kernel),
        kernel_before,
        "the operator's KERNEL master must be byte- AND mode-unchanged across a confined deploy \
         (the fourth amendment COPIES it into the run dir; the prior impl widened it o+r)",
    );
    assert_eq!(
        artifact_mode_and_bytes(&rootfs),
        rootfs_before,
        "the operator's ROOTFS master must be byte- AND mode-unchanged across a confined deploy \
         (FICLONE only READS it; the clone is staged in a platform dir, never beside it)",
    );
    assert_eq!(
        path_mode(kernel.parent().expect("kernel dir")),
        kernel_dir_before,
        "the operator's KERNEL directory mode must be unchanged (the prior impl widened it o+x)",
    );
    assert_eq!(
        path_mode(rootfs.parent().expect("rootfs dir")),
        rootfs_dir_before,
        "the operator's ROOTFS directory mode must be unchanged (the prior impl widened it o+x)",
    );

    stop_and_shutdown(handle, &cfg, "vm-operator-artifacts").await;
}

// ---------------------------------------------------------------------------
// AC-13 (US-VM-7) fail-closed — a host that cannot confine REFUSES the
// workload rather than starting it degraded, and confinement adds nothing to
// the operator's deploy surface. S-VM-51 (SimVmm-driven fail-closed) +
// S-VM-52 (real-boot no-new-operator-surface).
// ---------------------------------------------------------------------------

/// S-VM-51 / `@mandatory:mutation_target` — a host that cannot supply a
/// required `ConfinementControl` (here: `--landlock` below the version floor)
/// makes the allocation `Failed / VmConfinementUnavailable` NAMING that
/// control, and the hypervisor is NEVER started unconfined.
///
/// # Injected at the `Vmm` port, not organic (system constraint 1)
///
/// The whole Lima/metal test envelope runs ONE kernel, so no genuinely
/// Landlock-less host exists in it — a confinement capability is a
/// fixed-kernel-shape property, not something a fixture can toggle. So the
/// unavailable-control condition is injected at the `Vmm` port boundary via the
/// SAME `ServerConfig.vmm_override` seam step 01-09 wired (ADR-0083 §D8): a
/// [`SimVmm`] whose `create` fails CLOSED with
/// `VmmError::ConfinementUnavailable { control: Landlock, .. }`. `.probe()`
/// still runs unconditionally against it (a clean sim probe), so a REAL
/// in-process `overdrive serve` boots, registers the VM driver, and drives the
/// already-wired fail-closed producer path end to end —
/// `VmDriver::start` → `classify_vmm_error`'s `ConfinementUnavailable` arm →
/// `TransitionReason::VmConfinementUnavailable` → allocation `Failed`. No real
/// KVM boot is needed (the `SimVmm` never launches a hypervisor), so this
/// scenario runs under Lima; it inherits the file's `kvm-tests` gate only at
/// file granularity.
///
/// # Why the assertion kills the warn-and-continue mutation
///
/// The `@mandatory:mutation_target` is that fail-closed must never degrade to
/// warn-and-continue (start the hypervisor unconfined and proceed). This test
/// fails on BOTH failure shapes of that mutation:
///
/// * **It started at all.** A `SimVmm` create that returned `Ok(VmProcess)`
///   instead of the typed refusal — the shape a warn-and-continue mutation
///   produces — would drive the allocation to `Running`. The recorded-states
///   assertion (`never Running`) reddens on exactly that.
/// * **It reached the wrong terminal / wrong cause.** The allocation must land
///   `Failed` with `VmConfinementUnavailable` naming the *specific* control; a
///   mutation collapsing the `ConfinementUnavailable` classification arm into
///   the unclassified path (`DriverInternalError`) reddens on the reason
///   equality.
///
/// ```gherkin
/// Given Ana has deployed a VM workload on a host that cannot supply the
///   required confinement (e.g. no --landlock support below the version floor)
/// When the platform attempts to start the allocation
/// Then the allocation is Failed with TransitionReason::VmConfinementUnavailable
///   naming the unavailable ConfinementControl
/// And the hypervisor is NEVER started unconfined
/// ```
#[tokio::test]
#[serial(cgroup)]
async fn host_that_cannot_confine_refuses_the_workload_and_never_starts_unconfined() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-confine-unavail-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    // SimVmm never boots the guest, so a plain rootfs copy (no injected guest
    // binary, no loopback mount) is all the deploy's staging + the sim clone need.
    let rootfs = stage_plain_rootfs_copy(tmp.path(), &fixture);

    // The Vmm port, bound to a SimVmm that fails create CLOSED because the host
    // cannot supply the Landlock control (the "no --landlock below the version
    // floor" case). `.probe()` stays clean, so the node boots normally.
    let vmm = SimVmm::new();
    vmm.inject_persistent_confinement_unavailable(
        ConfinementControl::Landlock,
        "sim-injected: cloud-hypervisor below the --landlock version floor",
    );
    let (handle, server_tmp) = spawn_vm_server_with_vmm(std::sync::Arc::new(vmm)).await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-confine-unavail.toml",
        // The command never runs: create is refused before any guest launches.
        &vm_job_toml("vm-confine-unavail", "/sbin/never", &fixture.kernel_path, &rootfs),
    );

    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the VM workload the confinement-less host will refuse");

    let (out, observed) =
        poll_until_failed_recording_states(&cfg, &submit.workload_id, Duration::from_secs(30))
            .await;
    let row = out.snapshot.rows.first().expect("one failed allocation row");

    // The hypervisor was NEVER started unconfined: fail-closed, not
    // warn-and-continue. A warn-and-continue mutation (create returns
    // Ok(VmProcess)) would drive the allocation THROUGH Running — this reddens.
    assert!(
        !observed.contains(&AllocStateWire::Running),
        "a host that cannot confine must NEVER start the hypervisor (fail-closed, not \
         warn-and-continue); the allocation passed through Running: {observed:?}",
    );

    // Failed, not degraded.
    assert_eq!(
        row.state,
        AllocStateWire::Failed,
        "an unconfinable host must Fail the allocation, never start it degraded; observed \
         {observed:?}, reason={:?}",
        row.reason,
    );

    // The cause NAMES the unavailable control — not the unclassified fallback.
    let reason = row.reason.clone().expect("a failed VM allocation must carry a structured reason");
    assert!(
        matches!(
            reason,
            TransitionReason::VmConfinementUnavailable {
                control: ConfinementControl::Landlock,
                ..
            }
        ),
        "the refusal must be VmConfinementUnavailable naming the unavailable control (Landlock), \
         never a generic/unclassified cause: {reason:?}",
    );
    // Explicit complement: never swallowed as the unclassified driver-internal
    // cause a collapse of the ConfinementUnavailable classification arm produces.
    assert!(
        !matches!(reason, TransitionReason::DriverInternalError { .. }),
        "an unavailable confinement control must be named, never demoted to the unclassified \
         driver-internal cause: {reason:?}",
    );

    // The complement terminal is never reached: an unconfinable host does not
    // produce a completed/terminated outcome.
    assert_ne!(
        row.state,
        AllocStateWire::Terminated,
        "an unconfinable host must never reach a Terminated/completed terminal",
    );

    handle.shutdown().await.expect("clean shutdown");
}

/// S-VM-52 / `@contract-shape:unbounded-preservation` — deploying a VM on a
/// host that DOES support confinement requires no new flag, table or verb, and
/// the terminal state + exit code the operator reads are UNCHANGED from
/// Slice 01. Confinement (always active since step 04-04) is invisible to the
/// operator surface.
///
/// This is the preservation half of US-VM-7: S-VM-51 proves the fail-closed
/// refusal; this proves the success path grew no operator-facing surface. It
/// runs a REAL confined boot on metal (`@requires-kvm`).
///
/// # What "no new surface" is proved by
///
/// * **No new flag / verb.** The deploy is the SAME `DeployArgs { spec,
///   config_path }` an `[exec]` workload uses (there is no confinement
///   parameter to pass — a compile-time fact), and the `[job]`+`[vm]` spec
///   carries NO confinement stanza (`vm_job_toml` emits none).
/// * **Terminal + exit code unchanged.** A guest that exits 0 and reports it
///   reaches `Terminated` / `Stopped { by: Process }` → `Completed { exit_code:
///   0 }` — byte-identical to the Slice-01 exit-classification terminal
///   (S-VM-44), despite the hypervisor now running fully confined.
/// * **No new table / render field.** The rendered `workload describe` the
///   operator reads carries no confinement-specific field (no landlock /
///   seccomp / confinement / uid-drop vocabulary leaked into the operator view).
///
/// ```gherkin
/// Given Ana already deploys VM jobs with "overdrive deploy <spec>"
/// When she deploys a workload on a host that supports the required confinement
/// Then no new flag, table or verb is required
/// And the terminal state and exit code she reads are unchanged
/// ```
#[tokio::test]
#[serial(cgroup)]
async fn confinement_adds_no_new_operator_surface() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-nosurface-")
        .tempdir_in(shared_staging_root())
        .expect("tempdir on the reflink-capable staging root (never tmpfs -- cloud-hypervisor disk I/O needs O_DIRECT, which tmpfs cannot support)");
    let exit0 = build_exit_code_binary(tmp.path(), 0);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit0, "exit0");

    // The SAME real confined boot every other Slice-03 VM scenario uses — no
    // confinement-specific composition, no operator-facing knob.
    let (handle, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());
    let spec_path = write_toml(
        server_tmp.path(),
        "vm-nosurface.toml",
        // No confinement stanza — vm_job_toml emits only [job]/[vm]/[resources].
        &vm_job_toml("vm-nosurface", "/sbin/exit0", &fixture.kernel_path, &rootfs),
    );

    // The SAME DeployArgs an [exec] workload uses — there is no confinement
    // parameter to pass (a compile-time fact: no new flag/verb).
    let submit = deploy(DeployArgs { spec: spec_path, config_path: cfg.clone() })
        .await
        .expect("deploy the confined VM workload with the unchanged operator verb");
    let out = poll_until_terminated(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let row = out.snapshot.rows.first().expect("one terminated allocation row");

    // Terminal + exit code UNCHANGED from Slice 01: a guest exit 0, reported,
    // reaches the SAME completed terminal despite the hypervisor now running
    // fully confined.
    assert_eq!(
        row.state,
        AllocStateWire::Terminated,
        "a confined guest's clean exit must reach the SAME Terminated the unconfined Slice-01 \
         path did, got {:?} (reason={:?})",
        row.state,
        row.reason,
    );
    // The exit-0 terminal the operator reads is carried by
    // `Stopped { by: Process }` — the guest-authoritative clean-exit
    // classification `classify_natural_exit_terminal` maps to
    // `Completed { exit_code: 0 }`. This is the SAME terminal + exit
    // classification S-VM-44 pins for the Slice-01 path, unchanged now that the
    // hypervisor runs fully confined (04-04). Asserting exactly the two fields
    // S-VM-44 does keeps this a faithful preservation proof rather than
    // over-reaching into the separate exit-code-rendering concern.
    assert_eq!(
        row.reason,
        Some(TransitionReason::Stopped { by: StoppedBy::Process }),
        "the completed exit-0 terminal (Stopped by Process) the operator reads must be unchanged \
         by confinement",
    );
    // Never a crash: confinement did not turn a clean exit into a failure.
    assert!(
        !matches!(row.reason, Some(TransitionReason::WorkloadCrashedImmediately { .. })),
        "a confined clean guest exit must not be read as a crash: {:?}",
        row.reason,
    );

    // No new table / render field: the operator's rendered view carries no
    // confinement vocabulary — confinement did not grow the surface she reads.
    let rendered = overdrive_cli::render::workload_describe(&out).to_lowercase();
    for leaked in ["landlock", "seccomp", "confinement", "uid-drop", "uid_drop"] {
        assert!(
            !rendered.contains(leaked),
            "confinement must add no new operator-facing field; the rendered workload describe \
             leaked {leaked:?}:\n{rendered}",
        );
    }

    handle.shutdown().await.expect("clean shutdown");
}
