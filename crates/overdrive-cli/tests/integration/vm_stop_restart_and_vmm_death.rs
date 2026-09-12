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
//! (`crates/overdrive-reconcilers/src/workload_lifecycle.rs`) — is
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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, WorkloadDescribeOutput, describe};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::TransitionReason;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::driver::ConfinementControl;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::transition_reason::StoppedBy;
use overdrive_core::vm::config::{VmConfig, VmRunDir};
use overdrive_host::CloudHypervisorVmm;
use overdrive_sim::SimVmm;
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt as _;

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

fn write_guest_script(tmp: &Path, name: &str, body: &str) -> PathBuf {
    let path = tmp.join(name);
    std::fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n"))
        .expect("write guest supervision script");
    let mut permissions = std::fs::metadata(&path).expect("stat guest script").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("chmod guest script");
    path
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

fn install_static_guest_shell(mount: &Path) {
    let busybox = [Path::new("/bin/busybox"), Path::new("/usr/bin/busybox")]
        .into_iter()
        .find(|candidate| candidate.is_file())
        .expect("native guest-supervision fixture requires host busybox-static");
    let description =
        Command::new("file").arg(busybox).output().expect("inspect native busybox fixture");
    assert!(description.status.success(), "file must inspect busybox-static");
    assert!(
        String::from_utf8_lossy(&description.stdout).contains("statically linked"),
        "guest shell must be static: {}",
        String::from_utf8_lossy(&description.stdout)
    );
    let bin = mount.join("bin");
    std::fs::create_dir_all(&bin).expect("create guest /bin");
    install_guest_binary(busybox, &bin.join("busybox"));
    for applet in ["sh", "sleep", "setsid"] {
        let destination = bin.join(applet);
        if destination.exists() {
            std::fs::remove_file(&destination).expect("replace guest busybox applet");
        }
        std::os::unix::fs::symlink("busybox", &destination)
            .expect("install guest busybox applet symlink");
    }
}

fn stage_rootfs_with_guest_script(
    tmp: &Path,
    fixture: &VmFixture,
    script: &Path,
    guest_name: &str,
) -> PathBuf {
    with_mounted_rootfs_copy(tmp, fixture, |mount| {
        install_static_guest_shell(mount);
        install_guest_binary(script, &mount.join("sbin").join(guest_name));
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

struct StatePollOutcome {
    reached: bool,
    last: WorkloadDescribeOutput,
}

async fn poll_until_state_outcome(
    cfg: &Path,
    workload_id: &str,
    wanted: AllocStateWire,
    max_wait: Duration,
) -> StatePollOutcome {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let out = describe_once(cfg, workload_id).await;
        if out.snapshot.rows.first().is_some_and(|row| row.state == wanted) {
            return StatePollOutcome { reached: true, last: out };
        }
        if tokio::time::Instant::now() >= deadline {
            return StatePollOutcome { reached: false, last: out };
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn poll_until_state(
    cfg: &Path,
    workload_id: &str,
    wanted: AllocStateWire,
    max_wait: Duration,
) -> WorkloadDescribeOutput {
    let outcome = poll_until_state_outcome(cfg, workload_id, wanted, max_wait).await;
    assert!(
        outcome.reached,
        "workload {workload_id} did not reach {wanted:?} within {max_wait:?}; last row: {:?}",
        outcome.last.snapshot.rows.first(),
    );
    outcome.last
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

async fn poll_until_terminal(
    cfg: &Path,
    workload_id: &str,
    max_wait: Duration,
) -> Option<WorkloadDescribeOutput> {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let out = describe_once(cfg, workload_id).await;
        if out.snapshot.rows.first().is_some_and(|row| {
            matches!(row.state, AllocStateWire::Terminated | AllocStateWire::Failed)
                && row.terminal.is_some()
        }) {
            return Some(out);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn poll_until_terminal_outcome(
    cfg: &Path,
    workload_id: &str,
    max_wait: Duration,
) -> StatePollOutcome {
    let deadline = tokio::time::Instant::now() + max_wait;
    loop {
        let out = describe_once(cfg, workload_id).await;
        if out.snapshot.rows.first().is_some_and(|row| {
            matches!(row.state, AllocStateWire::Terminated | AllocStateWire::Failed)
        }) {
            return StatePollOutcome { reached: true, last: out };
        }
        if tokio::time::Instant::now() >= deadline {
            return StatePollOutcome { reached: false, last: out };
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
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

fn sha256sum_file(path: &Path) -> String {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("spawn sha256sum for native fixture inventory");
    assert!(
        output.status.success(),
        "sha256sum failed for {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .expect("sha256sum emits a digest")
        .to_owned()
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

/// The selected allocation TAP from the live hypervisor's `--net` value.
/// Reading the same process argv that the Landlock oracle observes keeps the
/// test on the production-selected identity; the test never derives or
/// hand-creates a second TAP name.
fn selected_network_tap(args: &[String]) -> &str {
    let net = args
        .windows(2)
        .find(|pair| pair[0] == "--net")
        .map(|pair| pair[1].as_str())
        .expect("a networked VM launch carries --net");
    net.split(',')
        .find_map(|field| field.strip_prefix("tap="))
        .expect("the network attachment carries its selected TAP")
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
/// `/proc/self`), and exactly the selected allocation TAP sysfs read grant
/// followed by the run-directory read-write grant.
///
/// ```gherkin
/// Given Ana has deployed a VM workload on a host that supports the required
///   confinement
/// When the allocation reaches Running
/// Then /proc/<vmm-pid>/status reports a non-zero real AND effective Uid and Gid
/// And /proc/<vmm-pid>/limits reports Max file size and Max open files strictly
///   below the SAME fields on the overdrive serve process
/// And the hypervisor was launched under a Landlock ruleset granting read-only
///   access to that allocation's selected TAP sysfs leaf first and read-write
///   access to that allocation's run directory second, with no other explicit
///   grant
/// ```
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
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

    // --- Landlock: exact selected-TAP read, then run-directory write ---
    let args = proc_cmdline_args(vmm_pid);
    assert!(
        args.iter().any(|a| a == "--landlock"),
        "the hypervisor must be launched with --landlock; argv={args:?}",
    );
    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), &alloc);
    let selected_tap = selected_network_tap(&args);
    let expected_tap_rule = format!("path=/sys/class/net/{selected_tap},access=r");
    let expected_run_dir_rule = format!("path={},access=rw", run_dir.path().display());
    assert_eq!(
        explicit_landlock_rules(&args),
        vec![expected_tap_rule, expected_run_dir_rule],
        "the explicit Landlock rules must be exactly the selected allocation TAP sysfs leaf \
         read-only first, then the allocation run directory read-write; no parent sysfs path, \
         glob, other TAP, alternate path, TAP write access, or extra rule is allowed; argv={args:?}",
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

/// S-VM-53 / `@correction:C-4` — a networked VM's explicit Landlock rules are
/// exactly the selected allocation TAP sysfs read grant followed by the run
/// directory read-write grant. The run directory holds nothing but this VM's
/// own sockets, logs, and its own kernel copy (ADR-0082
/// 2026-08-18 fourth amendment (c-fix.1) copies the operator kernel into the
/// run dir). CH needs read-only access to the selected TAP's sysfs leaf and
/// does not auto-derive a rule for the vsock socket it binds itself. The
/// directory-exclusivity property (SD-2) keeps the writable grant bounded.
///
/// ```gherkin
/// Given Ana has deployed a VM workload
/// When the hypervisor is launched
/// Then the run directory holds nothing but this VM's own sockets and logs
/// And the Landlock ruleset grants read-only on the selected TAP sysfs leaf
///   first and read-write on that run directory second, with no other rule
/// ```
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
#[serial(cgroup)]
async fn networked_vm_landlock_rules_are_exact_tap_read_then_run_dir_write() {
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

    // The complete explicit rule set is selected-TAP sysfs read-only first,
    // then the read-write DIRECTORY grant on the run directory (C-4). Exact
    // equality rejects the parent /sys/class/net path, globs, every other TAP,
    // alternate paths/aliases, TAP write access, and any extra rule.
    let args = proc_cmdline_args(vmm_pid);
    let selected_tap = selected_network_tap(&args);
    let expected_tap_path = format!("/sys/class/net/{selected_tap}");
    let expected_tap_rule = format!("path={expected_tap_path},access=r");
    let expected_run_dir_rule = format!("path={},access=rw", run_dir.path().display());
    let rules = explicit_landlock_rules(&args);
    assert_eq!(
        rules,
        vec![expected_tap_rule, expected_run_dir_rule],
        "the rules must contain only the exact selected allocation TAP sysfs leaf read-only, then \
         the allocation run directory read-write; this forbids /sys/class/net, globs, another TAP, \
         alternate paths, TAP write access, and a third rule; argv={args:?}",
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

// vm-lifecycle-latency native complements.  The control-stream decorator below
// stays at the already-existing Vmm port: Cloud Hypervisor and the rootfs's
// production overdrive-init still execute unchanged.  It merely interposes the
// Unix endpoint which implements CH's guest-to-host vsock transport so the
// acceptance matrix can deliver byte splits, coalescing, EOF and invalid
// messages that the well-behaved production BeaconWriter cannot originate.

#[derive(Clone, Copy, Debug)]
enum GuestControlScript {
    Pass,
    RepeatShutdown,
    SplitExec,
    ExecThenShutdown,
    ExecThenEof,
    ExecThenMalformed,
    ExecThenDuplicate,
    EofBeforeExec,
    MalformedBeforeExec,
    ShutdownBeforeExec,
}

#[derive(Clone, Default)]
struct GuestControlEvidence {
    guest_to_host: Arc<Mutex<Vec<u8>>>,
    host_to_guest: Arc<Mutex<Vec<u8>>>,
    proxy_dirs: Arc<Mutex<Vec<PathBuf>>>,
}

impl GuestControlEvidence {
    fn guest_text(&self) -> String {
        String::from_utf8_lossy(
            &self.guest_to_host.lock().expect("guest transcript mutex not poisoned"),
        )
        .into_owned()
    }

    fn host_text(&self) -> String {
        String::from_utf8_lossy(
            &self.host_to_guest.lock().expect("host transcript mutex not poisoned"),
        )
        .into_owned()
    }

    fn console_text(&self) -> String {
        self.proxy_dirs
            .lock()
            .expect("proxy-dir mutex not poisoned")
            .iter()
            .filter_map(|dir| std::fs::read(dir.join("console.log")).ok())
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn cleanup_proxy_dirs(&self) {
        for dir in self.proxy_dirs.lock().expect("proxy-dir mutex not poisoned").iter() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[derive(Clone)]
struct GuestControlVmm {
    inner: CloudHypervisorVmm,
    script: GuestControlScript,
    evidence: GuestControlEvidence,
}

impl GuestControlVmm {
    fn new(script: GuestControlScript) -> (Self, GuestControlEvidence) {
        let evidence = GuestControlEvidence::default();
        (Self { inner: CloudHypervisorVmm::new(), script, evidence: evidence.clone() }, evidence)
    }
}

async fn connect_control_upstream(path: &Path) -> UnixStream {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match UnixStream::connect(path).await {
            Ok(stream) => return stream,
            Err(error) if tokio::time::Instant::now() < deadline => {
                let _ = error;
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            Err(error) => panic!(
                "production VmDriver beacon listener at {} was not reachable: {error}",
                path.display()
            ),
        }
    }
}

async fn proxy_guest_control(
    listener: UnixListener,
    upstream_path: PathBuf,
    script: GuestControlScript,
    evidence: GuestControlEvidence,
) {
    let (guest, _) = listener.accept().await.expect("production init connects to proxy vsock");
    let host = connect_control_upstream(&upstream_path).await;
    let (mut guest_read, mut guest_write) = guest.into_split();
    let (host_read, mut host_write) = host.into_split();

    let guest_evidence = evidence.clone();
    let guest_to_host = async move {
        let mut buffer = [0_u8; 4096];
        loop {
            let count = guest_read.read(&mut buffer).await.expect("read production init bytes");
            if count == 0 {
                break;
            }
            guest_evidence
                .guest_to_host
                .lock()
                .expect("guest transcript mutex not poisoned")
                .extend_from_slice(&buffer[..count]);
            host_write
                .write_all(&buffer[..count])
                .await
                .expect("forward production init bytes to VmDriver");
        }
    };

    let host_evidence = evidence;
    let host_to_guest = async move {
        let mut reader = BufReader::new(host_read);
        let mut line = Vec::new();
        loop {
            line.clear();
            let count = reader.read_until(b'\n', &mut line).await.expect("read BeaconWriter line");
            if count == 0 {
                break;
            }
            host_evidence
                .host_to_guest
                .lock()
                .expect("host transcript mutex not poisoned")
                .extend_from_slice(&line);
            let is_exec = line.starts_with(b"EXEC ");
            let is_shutdown = line == b"SHUTDOWN\n";
            match (script, is_exec, is_shutdown) {
                (GuestControlScript::EofBeforeExec, true, _) => break,
                (GuestControlScript::MalformedBeforeExec, true, _) => {
                    guest_write.write_all(b"EXEC not-json\n").await.expect("write malformed EXEC");
                }
                (GuestControlScript::ShutdownBeforeExec, true, _) => {
                    guest_write.write_all(b"SHUTDOWN\n").await.expect("write unexpected SHUTDOWN");
                }
                (GuestControlScript::SplitExec, true, _) => {
                    let midpoint = line.len() / 2;
                    guest_write.write_all(&line[..midpoint]).await.expect("write EXEC prefix");
                    tokio::task::yield_now().await;
                    guest_write.write_all(&line[midpoint..]).await.expect("write EXEC suffix");
                }
                (GuestControlScript::ExecThenShutdown, true, _) => {
                    let mut coalesced = line.clone();
                    coalesced.extend_from_slice(b"SHUTDOWN\n");
                    guest_write
                        .write_all(&coalesced)
                        .await
                        .expect("write coalesced EXEC and SHUTDOWN");
                }
                (GuestControlScript::ExecThenEof, true, _) => {
                    guest_write.write_all(&line).await.expect("write EXEC before EOF");
                    break;
                }
                (GuestControlScript::ExecThenMalformed, true, _) => {
                    let mut coalesced = line.clone();
                    coalesced.extend_from_slice(b"not-a-beacon-frame\n");
                    guest_write
                        .write_all(&coalesced)
                        .await
                        .expect("write coalesced malformed frame");
                }
                (GuestControlScript::ExecThenDuplicate, true, _) => {
                    let mut coalesced = line.clone();
                    coalesced.extend_from_slice(&line);
                    guest_write.write_all(&coalesced).await.expect("write duplicate EXEC frames");
                }
                (GuestControlScript::RepeatShutdown, _, true) => {
                    guest_write
                        .write_all(b"SHUTDOWN\nSHUTDOWN\n")
                        .await
                        .expect("write repeated SHUTDOWN in one packet");
                }
                _ => guest_write.write_all(&line).await.expect("forward BeaconWriter line"),
            }
        }
        let _ = guest_write.shutdown().await;
    };

    tokio::join!(guest_to_host, host_to_guest);
}

#[async_trait]
impl Vmm for GuestControlVmm {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        static PROXY_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = PROXY_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let upstream_path =
            config.run_dir.beacon_socket(overdrive_core::vm::beacon::BEACON_VSOCK_PORT);
        let proxy_alloc_name =
            format!("{}-control-proxy-{}-{sequence}", config.alloc, std::process::id());
        let proxy_alloc =
            AllocationId::new(&proxy_alloc_name).expect("valid control-proxy allocation id");
        let proxy_run_dir = VmRunDir::for_alloc(
            config.run_dir.path().parent().expect("VM run directory has parent"),
            &proxy_alloc,
        );
        std::fs::create_dir_all(proxy_run_dir.path()).expect("create control-proxy run directory");
        let listener = UnixListener::bind(
            proxy_run_dir.beacon_socket(overdrive_core::vm::beacon::BEACON_VSOCK_PORT),
        )
        .expect("bind control-proxy beacon socket before Cloud Hypervisor starts");
        self.evidence
            .proxy_dirs
            .lock()
            .expect("proxy-dir mutex not poisoned")
            .push(proxy_run_dir.path().to_path_buf());
        tokio::spawn(proxy_guest_control(
            listener,
            upstream_path,
            self.script,
            self.evidence.clone(),
        ));
        let mut proxied = config.clone();
        proxied.run_dir = proxy_run_dir;
        self.inner.create(&proxied).await
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

async fn run_guest_control_case(
    fixture: &VmFixture,
    label: &str,
    script: GuestControlScript,
    command_body: &str,
    terminal_wait: Duration,
) -> (WorkloadDescribeOutput, GuestControlEvidence) {
    let fixture_tmp = tempfile::Builder::new()
        .prefix(&format!("vll-{label}-"))
        .tempdir_in(shared_staging_root())
        .expect("guest-control fixture dir");
    let command = write_guest_script(fixture_tmp.path(), "control-case", command_body);
    let rootfs =
        stage_rootfs_with_guest_script(fixture_tmp.path(), fixture, &command, "vll-control-case");
    let (vmm, evidence) = GuestControlVmm::new(script);
    let (server, server_tmp) = spawn_vm_server_with_vmm(Arc::new(vmm)).await;
    let cfg = config_path(server_tmp.path());
    let spec = write_toml(
        server_tmp.path(),
        &format!("{label}.toml"),
        &vm_job_toml(label, "/sbin/vll-control-case", &fixture.kernel_path, &rootfs),
    );
    let submitted = deploy(DeployArgs { spec, config_path: cfg.clone() })
        .await
        .expect("deploy guest-control case");
    let terminal = poll_until_terminal(&cfg, &submitted.workload_id, terminal_wait).await;
    let Some(terminal) = terminal else {
        let _ =
            stop(StopArgs { id: submitted.workload_id.clone(), config_path: cfg.clone() }).await;
        let _ = server.shutdown().await;
        evidence.cleanup_proxy_dirs();
        panic!("S-VLL-10b: {label} did not terminate after the controlled stream fault");
    };
    // Before EXEC, ADR-0103 deliberately starts no child and returns the
    // original receive error through init's fatal-poweroff path. That path is
    // not an operator stop: its exit watcher authors the terminal observation,
    // and `FinalizeFailed` has neither a `Driver::stop` call nor the discarded
    // `LiveVm` cleanup payload. Do not apply the during-execution host-artifact
    // oracle to those no-child cases. Once EXEC is accepted, retain the full
    // artifact complement alongside the group/session teardown oracle.
    if !matches!(
        script,
        GuestControlScript::EofBeforeExec
            | GuestControlScript::MalformedBeforeExec
            | GuestControlScript::ShutdownBeforeExec
    ) {
        let alloc = alloc_id_of(&terminal);
        if let Some(cleanup_error) =
            wait_for_vm_artifact_absence(&alloc, &rootfs, &server_tmp.path().join("data")).await
        {
            let _ = server.shutdown().await;
            evidence.cleanup_proxy_dirs();
            panic!("S-VLL-10b: {label}: {cleanup_error}");
        }
    }
    server.shutdown().await.expect("shutdown guest-control server");
    (terminal, evidence)
}

#[derive(Clone, Debug)]
struct CapturedLifecycleEvent {
    name: &'static str,
    at: std::time::Instant,
    fields: std::collections::BTreeMap<String, String>,
}

#[derive(Clone)]
struct LifecycleTraceLayer {
    events: Arc<Mutex<Vec<CapturedLifecycleEvent>>>,
    enabled: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for LifecycleTraceLayer {
    fn default() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            enabled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }
}

#[derive(Default)]
struct LifecycleFieldVisitor {
    fields: std::collections::BTreeMap<String, String>,
}

impl Visit for LifecycleFieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_owned(), value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().to_owned(), value.to_string());
    }
}

impl<S> Layer<S> for LifecycleTraceLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: tracing_subscriber::layer::Context<'_, S>) {
        if !self.enabled.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let name = event.metadata().name();
        if !matches!(
            name,
            "vm.lifecycle.create_enter"
                | "vm.lifecycle.created"
                | "vm.lifecycle.ready"
                | "vm.beacon.exec.released"
                | "vm.lifecycle.stop_enter"
                | "vm.lifecycle.writer_finished"
                | "vmm.process.reaped"
                | "vm.lifecycle.cleanup_calls_finished"
                | "convergence.evaluation.admitted"
                | "convergence.evaluation.completed"
        ) {
            return;
        }
        let mut visitor = LifecycleFieldVisitor::default();
        event.record(&mut visitor);
        self.events.lock().expect("lifecycle-event mutex not poisoned").push(
            CapturedLifecycleEvent { name, at: std::time::Instant::now(), fields: visitor.fields },
        );
    }
}

fn install_lifecycle_trace()
-> (Arc<Mutex<Vec<CapturedLifecycleEvent>>>, Arc<std::sync::atomic::AtomicBool>) {
    let layer = LifecycleTraceLayer::default();
    let events = Arc::clone(&layer.events);
    let enabled = Arc::clone(&layer.enabled);
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer))
        .expect("native lifecycle benchmark owns this nextest process's tracing subscriber");
    (events, enabled)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeLifecycleProfile {
    Ready,
    FiniteJob,
    CooperativeService,
}

#[derive(Clone, Debug)]
struct NativeTrial {
    profile: NativeLifecycleProfile,
    ordinal: usize,
    workload_id: String,
    alloc: AllocationId,
    failed: Option<String>,
    operator_stop_duration: Option<Duration>,
    stop_admission_after_stop_return: Option<Duration>,
    terminal_after_stop_return: Option<Duration>,
    observed_cleanup_at: std::time::Instant,
}

fn native_service_toml(id: &str, kernel: &Path, rootfs: &Path) -> String {
    format!(
        "[service]\nid = \"{id}\"\nreplicas = 1\n\n[[listener]]\nport = 18081\nprotocol = \"tcp\"\n\n\
         [vm]\ncommand = \"/bin/sh\"\nargs = [\"-c\", \"trap 'exit 0' TERM; /opt/overdrive/examples/svm/e08-server & child=$!; trap 'kill -TERM $child 2>/dev/null || true; wait $child || true; exit 0' TERM; wait $child\"]\n\
         kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = 500\nmemory_bytes = 134217728\n\n\
         [[health_check.startup]]\ntype = \"tcp\"\nhost = \"0.0.0.0\"\nport = 18081\ninterval_seconds = 1\ntimeout_seconds = 1\nmax_attempts = 30\n",
        kernel.display(),
        rootfs.display(),
    )
}

const fn native_profile_slug(profile: NativeLifecycleProfile) -> &'static str {
    match profile {
        NativeLifecycleProfile::Ready => "ready",
        NativeLifecycleProfile::FiniteJob => "job",
        NativeLifecycleProfile::CooperativeService => "service",
    }
}

fn native_alloc_from(out: &WorkloadDescribeOutput) -> Option<AllocationId> {
    out.snapshot.rows.first().and_then(|row| AllocationId::new(&row.alloc_id).ok())
}

// Public-stop-to-admission is a recorded distribution, not a product SLO.
// Await the required stage boundary without a fixture-local duration gate; the
// enclosing test-runner liveness boundary remains the finite guard if the event
// is genuinely absent.  Once admission is observed, the trial applies its
// independent terminal-observation bound and stage quantile contract.
async fn await_stop_admission(
    events: Option<&Arc<Mutex<Vec<CapturedLifecycleEvent>>>>,
    alloc: Option<&AllocationId>,
    public_stop_returned_at: std::time::Instant,
) -> Option<Duration> {
    let (Some(events), Some(alloc)) = (events, alloc) else { return Some(Duration::ZERO) };
    let alloc = alloc.to_string();
    loop {
        let observed_at = {
            events
                .lock()
                .expect("lifecycle-event mutex not poisoned")
                .iter()
                .find(|event| {
                    event.name == "vm.lifecycle.stop_enter"
                        && event.fields.get("alloc") == Some(&alloc)
                })
                .map(|event| event.at)
        };
        if let Some(observed_at) = observed_at {
            return Some(observed_at.saturating_duration_since(public_stop_returned_at));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the retained trial record keeps fixture cleanup, timing and failure evidence together"
)]
async fn finish_native_trial(
    profile: NativeLifecycleProfile,
    ordinal: usize,
    workload_id: &str,
    alloc: Option<AllocationId>,
    rootfs: &Path,
    data_dir: &Path,
    operator_stop_duration: Option<Duration>,
    stop_admission_after_stop_return: Option<Duration>,
    terminal_after_stop_return: Option<Duration>,
    mut failures: Vec<String>,
) -> NativeTrial {
    let alloc = if let Some(alloc) = alloc {
        if let Some(error) = wait_for_vm_artifact_absence(&alloc, rootfs, data_dir).await {
            failures.push(error);
        }
        alloc
    } else {
        failures.push("allocation row unavailable for cleanup and stage correlation".to_owned());
        AllocationId::new(&format!("missing-{}-{ordinal}", native_profile_slug(profile)))
            .expect("valid missing-allocation trial placeholder")
    };
    NativeTrial {
        profile,
        ordinal,
        workload_id: workload_id.to_owned(),
        alloc,
        failed: (!failures.is_empty()).then(|| failures.join("; ")),
        operator_stop_duration,
        stop_admission_after_stop_return,
        terminal_after_stop_return,
        observed_cleanup_at: std::time::Instant::now(),
    }
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one trial retains the complete three-profile result ledger and its shared stage-event oracle instead of splitting failure accounting across helpers"
)]
async fn exercise_native_trial(
    profile: NativeLifecycleProfile,
    ordinal: usize,
    events: Option<&Arc<Mutex<Vec<CapturedLifecycleEvent>>>>,
    cfg: PathBuf,
    spec_dir: PathBuf,
    kernel: PathBuf,
    rootfs: PathBuf,
    data_dir: PathBuf,
) -> NativeTrial {
    let id = match profile {
        NativeLifecycleProfile::Ready => format!("vll-ready-{ordinal:04}"),
        NativeLifecycleProfile::FiniteJob => format!("vll-job-{ordinal:04}"),
        NativeLifecycleProfile::CooperativeService => format!("vll-service-{ordinal:04}"),
    };
    let body = match profile {
        NativeLifecycleProfile::Ready => vm_job_toml(&id, "/sbin/vll-spin", &kernel, &rootfs),
        NativeLifecycleProfile::FiniteJob => vm_job_toml(&id, "/sbin/vll-exit0", &kernel, &rootfs),
        NativeLifecycleProfile::CooperativeService => native_service_toml(&id, &kernel, &rootfs),
    };
    let spec = write_toml(&spec_dir, &format!("{id}.toml"), &body);
    let submitted = match deploy(DeployArgs { spec, config_path: cfg.clone() }).await {
        Ok(submitted) => submitted,
        Err(error) => {
            return NativeTrial {
                profile,
                ordinal,
                workload_id: id,
                alloc: AllocationId::new(&format!(
                    "deploy-{}-{ordinal}",
                    native_profile_slug(profile)
                ))
                .expect("valid failed-trial allocation placeholder"),
                failed: Some(format!("deploy: {error}")),
                operator_stop_duration: None,
                stop_admission_after_stop_return: None,
                terminal_after_stop_return: None,
                observed_cleanup_at: std::time::Instant::now(),
            };
        }
    };

    match profile {
        NativeLifecycleProfile::Ready => {
            let running = poll_until_state_outcome(
                &cfg,
                &submitted.workload_id,
                AllocStateWire::Running,
                Duration::from_secs(90),
            )
            .await;
            let mut alloc = native_alloc_from(&running.last);
            let mut failures = Vec::new();
            if !running.reached {
                failures.push(format!(
                    "Running timeout; last row: {:?}",
                    running.last.snapshot.rows.first()
                ));
            }
            let stop_started = std::time::Instant::now();
            let stop_result =
                stop(StopArgs { id: submitted.workload_id.clone(), config_path: cfg.clone() })
                    .await;
            let operator_stop_duration = stop_started.elapsed();
            let stop_returned_at = std::time::Instant::now();
            if let Err(error) = stop_result {
                failures.push(format!("stop: {error}"));
            }
            let stop_admission_after_stop_return =
                await_stop_admission(events, alloc.as_ref(), stop_returned_at).await;
            let terminal =
                poll_until_terminal_outcome(&cfg, &submitted.workload_id, Duration::from_secs(30))
                    .await;
            alloc = alloc.or_else(|| native_alloc_from(&terminal.last));
            if !terminal.reached {
                failures.push(format!(
                    "terminal timeout after stop; last row: {:?}",
                    terminal.last.snapshot.rows.first()
                ));
            } else if terminal.last.snapshot.rows.first().map(|row| row.state)
                != Some(AllocStateWire::Terminated)
            {
                failures.push(format!(
                    "READY stop terminal was {:?}",
                    terminal.last.snapshot.rows.first()
                ));
            }
            let terminal_after_stop_return = terminal.reached.then(|| stop_returned_at.elapsed());
            finish_native_trial(
                profile,
                ordinal,
                &submitted.workload_id,
                alloc,
                &rootfs,
                &data_dir,
                Some(operator_stop_duration),
                stop_admission_after_stop_return,
                terminal_after_stop_return,
                failures,
            )
            .await
        }
        NativeLifecycleProfile::FiniteJob => {
            let terminal =
                poll_until_terminal_outcome(&cfg, &submitted.workload_id, Duration::from_secs(90))
                    .await;
            let mut alloc = native_alloc_from(&terminal.last);
            let mut failures = Vec::new();
            if terminal.reached {
                let row = terminal.last.snapshot.rows.first().expect("terminal trial row");
                let exit_projection_matches_terminal = matches!(
                    (&row.terminal, row.exit_code),
                    (None, None)
                        | (
                            Some(overdrive_core::TerminalCondition::Completed { exit_code: 0 }),
                            Some(0)
                        )
                );
                if row.state != AllocStateWire::Terminated
                    || row.reason != Some(TransitionReason::Stopped { by: StoppedBy::Process })
                    || !exit_projection_matches_terminal
                {
                    failures.push(format!(
                        "finite Job terminal was {:?}, exit {:?}, reason {:?}, terminal {:?}",
                        row.state, row.exit_code, row.reason, row.terminal
                    ));
                }
            } else {
                failures.push(format!(
                    "terminal timeout; last row: {:?}",
                    terminal.last.snapshot.rows.first()
                ));
                if let Err(error) =
                    stop(StopArgs { id: submitted.workload_id.clone(), config_path: cfg.clone() })
                        .await
                {
                    failures.push(format!("timeout cleanup stop: {error}"));
                }
                let stopped = poll_until_terminal_outcome(
                    &cfg,
                    &submitted.workload_id,
                    Duration::from_secs(30),
                )
                .await;
                alloc = alloc.or_else(|| native_alloc_from(&stopped.last));
                if !stopped.reached {
                    failures.push(format!(
                        "terminal timeout after cleanup stop; last row: {:?}",
                        stopped.last.snapshot.rows.first()
                    ));
                }
            }
            finish_native_trial(
                profile,
                ordinal,
                &submitted.workload_id,
                alloc,
                &rootfs,
                &data_dir,
                None,
                None,
                None,
                failures,
            )
            .await
        }
        NativeLifecycleProfile::CooperativeService => {
            let running = poll_until_state_outcome(
                &cfg,
                &submitted.workload_id,
                AllocStateWire::Running,
                Duration::from_secs(90),
            )
            .await;
            let mut alloc = native_alloc_from(&running.last);
            let mut failures = Vec::new();
            if running.reached {
                let row = running.last.snapshot.rows.first().expect("service allocation row");
                match row.workload_addr {
                    Some(address) => {
                        match tokio::time::timeout(Duration::from_secs(5), async {
                            let mut stream =
                                tokio::net::TcpStream::connect((address, 18081)).await?;
                            stream.write_all(b"native-lifecycle-probe").await?;
                            let mut reply = [0_u8; 64];
                            let count = stream.read(&mut reply).await?;
                            if &reply[..count] == b"SVM-E08-GUEST-OK" {
                                Ok::<(), std::io::Error>(())
                            } else {
                                Err(std::io::Error::other("unexpected cooperative service reply"))
                            }
                        })
                        .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                failures.push(format!("operator-visible request: {error}"));
                            }
                            Err(_) => failures
                                .push("operator-visible request timed out after 5s".to_owned()),
                        }
                    }
                    None => failures.push("Running service lacks workload_addr".to_owned()),
                }
            } else {
                failures.push(format!(
                    "Running timeout; last row: {:?}",
                    running.last.snapshot.rows.first()
                ));
            }
            let stop_started = std::time::Instant::now();
            let stop_result =
                stop(StopArgs { id: submitted.workload_id.clone(), config_path: cfg.clone() })
                    .await;
            let operator_stop_duration = stop_started.elapsed();
            let stop_returned_at = std::time::Instant::now();
            if let Err(error) = stop_result {
                failures.push(format!("stop: {error}"));
            }
            let stop_admission_after_stop_return =
                await_stop_admission(events, alloc.as_ref(), stop_returned_at).await;
            let terminal =
                poll_until_terminal_outcome(&cfg, &submitted.workload_id, Duration::from_secs(30))
                    .await;
            alloc = alloc.or_else(|| native_alloc_from(&terminal.last));
            if !terminal.reached {
                failures.push(format!(
                    "terminal timeout after stop; last row: {:?}",
                    terminal.last.snapshot.rows.first()
                ));
            } else if terminal.last.snapshot.rows.first().map(|row| row.state)
                != Some(AllocStateWire::Terminated)
            {
                failures.push(format!(
                    "cooperative Service terminal was {:?}",
                    terminal.last.snapshot.rows.first()
                ));
            }
            let terminal_after_stop_return = terminal.reached.then(|| stop_returned_at.elapsed());
            finish_native_trial(
                profile,
                ordinal,
                &submitted.workload_id,
                alloc,
                &rootfs,
                &data_dir,
                Some(operator_stop_duration),
                stop_admission_after_stop_return,
                terminal_after_stop_return,
                failures,
            )
            .await
        }
    }
}

async fn wait_for_vm_artifact_absence(
    alloc: &AllocationId,
    rootfs: &Path,
    data_dir: &Path,
) -> Option<String> {
    let plan = overdrive_core::vm::config::RootfsPlan::for_alloc(
        rootfs.to_path_buf(),
        std::fs::metadata(rootfs).map(|meta| meta.len()).unwrap_or_default(),
        alloc,
        &overdrive_core::vm::config::clone_staging_dir(data_dir),
        &overdrive_core::vm::config::clone_index_dir(data_dir),
    );
    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), alloc);
    let cgroup = overdrive_core::cgroup::CgroupPath::for_alloc(alloc)
        .resolve(Path::new(overdrive_control_plane::cgroup_preflight::DEFAULT_CGROUP_ROOT));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let present = [
            run_dir.path().to_path_buf(),
            cgroup.clone(),
            plan.clone_dest().to_path_buf(),
            plan.index_link().to_path_buf(),
        ]
        .into_iter()
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
        if present.is_empty() {
            return None;
        }
        if tokio::time::Instant::now() >= deadline {
            return Some(format!("driver artifacts remain after cleanup calls: {present:?}"));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn nearest_rank(samples: &mut [Duration], numerator: usize, denominator: usize) -> Duration {
    samples.sort_unstable();
    let rank = samples.len().saturating_mul(numerator).div_ceil(denominator).max(1);
    samples[rank - 1]
}

#[derive(Debug)]
struct DurationDistribution {
    n: usize,
    min: Duration,
    p50: Duration,
    p95: Duration,
    p99: Duration,
    max: Duration,
}

fn duration_distribution(samples: &[Duration], label: &str) -> DurationDistribution {
    assert!(!samples.is_empty(), "{label} distribution must retain at least one sample");
    DurationDistribution {
        n: samples.len(),
        min: *samples.iter().min().expect("non-empty duration distribution"),
        p50: nearest_rank(&mut samples.to_vec(), 50, 100),
        p95: nearest_rank(&mut samples.to_vec(), 95, 100),
        p99: nearest_rank(&mut samples.to_vec(), 99, 100),
        max: *samples.iter().max().expect("non-empty duration distribution"),
    }
}

fn assert_normal_vmm_reap(events: &Arc<Mutex<Vec<CapturedLifecycleEvent>>>, alloc: &AllocationId) {
    let events = events.lock().expect("lifecycle-event mutex not poisoned");
    let alloc_text = alloc.to_string();
    let created = events
        .iter()
        .find(|event| {
            event.name == "vm.lifecycle.created" && event.fields.get("alloc") == Some(&alloc_text)
        })
        .unwrap_or_else(|| panic!("missing vm.lifecycle.created for {alloc}"));
    let pid = created.fields.get("pid").expect("created event carries pid").clone();
    let reaped = events
        .iter()
        .find(|event| event.name == "vmm.process.reaped" && event.fields.get("pid") == Some(&pid))
        .unwrap_or_else(|| panic!("missing vmm.process.reaped for {alloc}"))
        .clone();
    drop(events);
    assert!(
        reaped.fields.get("exit_code").is_some_and(|value| value == "0" || value == "Some(0)"),
        "VMM must exit normally after guest poweroff: {reaped:?}"
    );
    assert!(
        reaped.fields.get("signal").is_none_or(|value| value == "None"),
        "a forced VMM exit cannot pass the healthy observation: {reaped:?}"
    );
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-10a. Real production init in a guest: cooperative direct child plus
/// same-group descendant; direct exits first; natural exit races SHUTDOWN;
/// ignored TERM reaches one 5s grace then SIGKILL/reap; repeated SHUTDOWN never
/// resets it. Characterize deliberate group escape only until poweroff.
/// Observe guest-recorded process-group/reap status, exact direct-child EXIT
/// once, normal/forced VMM exit and complete allocation artifact complement.
#[allow(clippy::doc_markdown, reason = "exact per-test contract declaration")]
#[tokio::test]
#[serial(cgroup)]
async fn guest_supervisor_reaps_its_command_group_and_preserves_direct_child_status() {
    let (events, _trace_enabled) = install_lifecycle_trace();
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision VM fixture");

    // Direct child exits first while a cooperative same-group descendant is
    // still alive.  The supervisor must retain 23, terminate/reap the
    // descendant, emit one EXIT, and power off without waiting for SHUTDOWN.
    let natural_tmp = tempfile::Builder::new()
        .prefix("vll-s10a-natural-")
        .tempdir_in(shared_staging_root())
        .expect("natural-exit fixture dir");
    let natural_script = write_guest_script(
        natural_tmp.path(),
        "direct-first",
        "( trap 'exit 0' TERM; while :; do /bin/sleep 1; done ) &\nexit 23",
    );
    let natural_rootfs = stage_rootfs_with_guest_script(
        natural_tmp.path(),
        &fixture,
        &natural_script,
        "vll-direct-first",
    );
    let (natural_vmm, natural_evidence) = GuestControlVmm::new(GuestControlScript::Pass);
    let (natural_server, natural_server_tmp) =
        spawn_vm_server_with_vmm(Arc::new(natural_vmm)).await;
    let natural_cfg = config_path(natural_server_tmp.path());
    let natural_spec = write_toml(
        natural_server_tmp.path(),
        "vll-s10a-natural.toml",
        &vm_job_toml(
            "vll-s10a-natural",
            "/sbin/vll-direct-first",
            &fixture.kernel_path,
            &natural_rootfs,
        ),
    );
    let natural_submit =
        deploy(DeployArgs { spec: natural_spec, config_path: natural_cfg.clone() })
            .await
            .expect("deploy direct-child-first VM");
    let Some(natural_terminal) =
        poll_until_terminal(&natural_cfg, &natural_submit.workload_id, Duration::from_secs(15))
            .await
    else {
        let _ = stop(StopArgs {
            id: natural_submit.workload_id.clone(),
            config_path: natural_cfg.clone(),
        })
        .await;
        let _ = natural_server.shutdown().await;
        natural_evidence.cleanup_proxy_dirs();
        panic!(
            "S-VLL-10a: production init waited for post-EXIT SHUTDOWN instead of powering off after group completion"
        );
    };
    let natural_row = natural_terminal.snapshot.rows.first().expect("natural terminal row");
    assert_eq!(
        natural_row.exit_code,
        Some(23),
        "the direct child's exact status survives descendant teardown"
    );
    assert_eq!(
        natural_evidence.guest_text().matches("EXIT 23\n").count(),
        1,
        "successful supervision emits the saved direct-child EXIT exactly once"
    );
    let natural_console = natural_evidence.console_text();
    assert!(
        natural_console.contains("group-complete"),
        "missing group-complete: {natural_console}"
    );
    assert!(
        natural_console.contains("poweroff-requested"),
        "missing poweroff-requested: {natural_console}"
    );
    let natural_alloc = alloc_id_of(&natural_terminal);
    assert_normal_vmm_reap(&events, &natural_alloc);
    assert_eq!(
        wait_for_vm_artifact_absence(
            &natural_alloc,
            &natural_rootfs,
            &natural_server_tmp.path().join("data")
        )
        .await,
        None
    );
    natural_server.shutdown().await.expect("shutdown natural-exit server");
    natural_evidence.cleanup_proxy_dirs();

    // EXEC and SHUTDOWN arrive in one transport read while the direct child
    // is live.  Whether its natural timer or SIGTERM handling wins, the direct
    // child deliberately returns 29; a late/repeated control message cannot
    // replace that saved status or cause a second EXIT.
    let (raced_terminal, raced_evidence) = run_guest_control_case(
        &fixture,
        "vll-s10a-natural-shutdown-race",
        GuestControlScript::ExecThenShutdown,
        "trap 'exit 29' TERM\n/bin/sleep 1\nexit 29",
        Duration::from_secs(15),
    )
    .await;
    let raced_row = raced_terminal.snapshot.rows.first().expect("raced terminal row");
    assert_eq!(raced_row.exit_code, Some(29), "direct status survives the control race");
    assert_eq!(raced_evidence.guest_text().matches("EXIT 29\n").count(), 1);
    let raced_console = raced_evidence.console_text();
    assert!(raced_console.contains("shutdown-received"), "missing race receipt: {raced_console}");
    assert!(raced_console.contains("group-complete"), "missing raced reap: {raced_console}");
    assert_normal_vmm_reap(&events, &alloc_id_of(&raced_terminal));
    raced_evidence.cleanup_proxy_dirs();

    // Both the direct child and its descendant ignore TERM.  Two coalesced
    // SHUTDOWN frames must establish only one five-second deadline.  The guest
    // then SIGKILLs/reaps the group and powers off early enough to remain
    // inside the host's single ten-second grace.
    let forced_tmp = tempfile::Builder::new()
        .prefix("vll-s10a-forced-")
        .tempdir_in(shared_staging_root())
        .expect("forced fixture dir");
    let forced_script = write_guest_script(
        forced_tmp.path(),
        "ignore-term",
        "trap '' TERM\n( trap '' TERM; while :; do /bin/sleep 1; done ) &\nwhile :; do /bin/sleep 1; done",
    );
    let forced_rootfs = stage_rootfs_with_guest_script(
        forced_tmp.path(),
        &fixture,
        &forced_script,
        "vll-ignore-term",
    );
    let (forced_vmm, forced_evidence) = GuestControlVmm::new(GuestControlScript::RepeatShutdown);
    let (forced_server, forced_server_tmp) = spawn_vm_server_with_vmm(Arc::new(forced_vmm)).await;
    let forced_cfg = config_path(forced_server_tmp.path());
    let forced_spec = write_toml(
        forced_server_tmp.path(),
        "vll-s10a-forced.toml",
        &vm_job_toml(
            "vll-s10a-forced",
            "/sbin/vll-ignore-term",
            &fixture.kernel_path,
            &forced_rootfs,
        ),
    );
    let forced_submit = deploy(DeployArgs { spec: forced_spec, config_path: forced_cfg.clone() })
        .await
        .expect("deploy TERM-resistant group");
    poll_until_running(&forced_cfg, &forced_submit.workload_id, Duration::from_secs(90)).await;
    let stop_started = std::time::Instant::now();
    stop(StopArgs { id: forced_submit.workload_id.clone(), config_path: forced_cfg.clone() })
        .await
        .expect("operator stop TERM-resistant group");
    let forced_terminal =
        poll_until_terminated(&forced_cfg, &forced_submit.workload_id, Duration::from_secs(20))
            .await;
    let stop_elapsed = stop_started.elapsed();
    assert!(
        stop_elapsed < Duration::from_secs(8),
        "one guest grace must fit inside the host grace; repeated SHUTDOWN cannot extend it: {stop_elapsed:?}"
    );
    assert_eq!(
        forced_evidence.host_text().matches("SHUTDOWN\n").count(),
        1,
        "the real host submits one request; the proxy duplicates it only on the guest side"
    );
    let forced_console = forced_evidence.console_text();
    assert!(forced_console.contains("shutdown-received"), "missing receipt: {forced_console}");
    assert!(forced_console.contains("group-complete"), "missing reap boundary: {forced_console}");
    assert!(
        forced_console.contains("poweroff-requested"),
        "missing poweroff boundary: {forced_console}"
    );
    let forced_guest = forced_evidence.guest_text();
    assert_eq!(
        forced_guest.lines().filter(|line| line.starts_with("EXIT ")).collect::<Vec<_>>(),
        ["EXIT 137"],
        "the TERM-resistant direct child must report the exact SIGKILL-mapped status once"
    );
    let forced_alloc = alloc_id_of(&forced_terminal);
    assert_normal_vmm_reap(&events, &forced_alloc);
    assert_eq!(
        wait_for_vm_artifact_absence(
            &forced_alloc,
            &forced_rootfs,
            &forced_server_tmp.path().join("data")
        )
        .await,
        None
    );
    forced_server.shutdown().await.expect("shutdown forced-group server");
    forced_evidence.cleanup_proxy_dirs();

    // A deliberately escaped descendant is not claimed as graceful-group
    // coverage.  It must not hold PID 1 indefinitely after the direct child
    // finishes; guest poweroff is the boundary that disposes it.
    let escape_tmp = tempfile::Builder::new()
        .prefix("vll-s10a-escape-")
        .tempdir_in(shared_staging_root())
        .expect("escape fixture dir");
    let escape_script = write_guest_script(
        escape_tmp.path(),
        "group-escape",
        "/bin/setsid /bin/sh -c 'trap \"\" TERM; while :; do /bin/sleep 1; done' &\nexit 7",
    );
    let escape_rootfs = stage_rootfs_with_guest_script(
        escape_tmp.path(),
        &fixture,
        &escape_script,
        "vll-group-escape",
    );
    let (escape_vmm, escape_evidence) = GuestControlVmm::new(GuestControlScript::Pass);
    let (escape_server, escape_server_tmp) = spawn_vm_server_with_vmm(Arc::new(escape_vmm)).await;
    let escape_cfg = config_path(escape_server_tmp.path());
    let escape_spec = write_toml(
        escape_server_tmp.path(),
        "vll-s10a-escape.toml",
        &vm_job_toml(
            "vll-s10a-escape",
            "/sbin/vll-group-escape",
            &fixture.kernel_path,
            &escape_rootfs,
        ),
    );
    let escape_submit = deploy(DeployArgs { spec: escape_spec, config_path: escape_cfg.clone() })
        .await
        .expect("deploy deliberate group escape");
    let escape_terminal =
        poll_until_terminal(&escape_cfg, &escape_submit.workload_id, Duration::from_secs(15))
            .await
            .expect("escaped descendant cannot hold guest poweroff indefinitely");
    assert_eq!(escape_terminal.snapshot.rows.first().and_then(|row| row.exit_code), Some(7));
    assert_eq!(escape_evidence.guest_text().matches("EXIT 7\n").count(), 1);
    let escape_alloc = alloc_id_of(&escape_terminal);
    assert_normal_vmm_reap(&events, &escape_alloc);
    assert_eq!(
        wait_for_vm_artifact_absence(
            &escape_alloc,
            &escape_rootfs,
            &escape_server_tmp.path().join("data")
        )
        .await,
        None
    );
    escape_server.shutdown().await.expect("shutdown escape server");
    escape_evidence.cleanup_proxy_dirs();
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-10b. Before EXEC: EOF, malformed and unexpected frame start no child.
/// During execution: partial and coalesced frames, repeated SHUTDOWN, EOF,
/// malformed frame and duplicate EXEC preserve framing and bounded teardown
/// with no false EXIT. Source-local File/process-boundary tests pin every exact
/// typed error; this qualified-native case retains group, reap and poweroff
/// evidence separately. EINTR retains deadlines; ESRCH is absence; ECHILD
/// cannot lose direct status. Real accepted host/init control path, not a
/// second guest supervisor.
#[allow(clippy::doc_markdown, reason = "exact per-test contract declaration")]
#[tokio::test]
#[serial(cgroup)]
async fn guest_control_stream_errors_keep_bounded_teardown_and_original_error() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision VM fixture");
    let never_run = "exit 0";

    for (label, script) in [
        ("vll-s10b-pre-eof", GuestControlScript::EofBeforeExec),
        ("vll-s10b-pre-malformed", GuestControlScript::MalformedBeforeExec),
        ("vll-s10b-pre-unexpected", GuestControlScript::ShutdownBeforeExec),
    ] {
        let (terminal, evidence) =
            run_guest_control_case(&fixture, label, script, never_run, Duration::from_secs(30))
                .await;
        // The source-local File-boundary test proves that the operator closure
        // is never entered.  At this native boundary, a finalized Failed row
        // is published only after the VMM exit watcher has consumed the clean
        // guest poweroff and the host has reaped the VMM.  The bounded poll
        // above and the wire transcript are the durable host observations;
        // serial console delivery is deliberately not part of this oracle.
        let row = terminal.snapshot.rows.first().expect("one finalized pre-EXEC allocation row");
        assert_eq!(
            row.state,
            AllocStateWire::Failed,
            "{label} must remain a failed no-report execution, never a completed operator command"
        );
        assert!(
            matches!(row.terminal.as_ref(), Some(overdrive_core::TerminalCondition::Failed { .. })),
            "{label} must finalize as failure, never as a completed operator command: {:?}",
            row.terminal
        );
        assert!(
            matches!(
                row.reason.as_ref(),
                Some(TransitionReason::WorkloadCrashedImmediately { signal: None, .. })
            ),
            "{label} must reach terminal observation through clean guest poweroff, not forced VMM termination: {:?}",
            row.reason
        );
        let guest_transcript = evidence.guest_text();
        assert_eq!(
            guest_transcript.lines().filter(|line| line.starts_with("READY ")).count(),
            1,
            "{label} must exercise the accepted native READY-to-first-EXEC boundary"
        );
        assert_eq!(
            guest_transcript.matches("EXIT ").count(),
            0,
            "a pre-EXEC failure cannot fabricate EXIT"
        );
        evidence.cleanup_proxy_dirs();
    }

    // After EXEC, EOF/malformed/duplicate control retain their original cause
    // only after the live group has been bounded and reaped.  A second EXIT is
    // never fabricated.  The 60-second command makes it impossible for a
    // legacy child.wait-before-control implementation to pass by natural exit.
    for (label, script) in [
        ("vll-s10b-post-eof", GuestControlScript::ExecThenEof),
        ("vll-s10b-post-malformed", GuestControlScript::ExecThenMalformed),
        ("vll-s10b-post-duplicate", GuestControlScript::ExecThenDuplicate),
    ] {
        let started = std::time::Instant::now();
        let (_terminal, evidence) = run_guest_control_case(
            &fixture,
            label,
            script,
            "trap 'exit 41' TERM\nwhile :; do /bin/sleep 1; done",
            Duration::from_secs(8),
        )
        .await;
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "{label} must terminate inside one five-second group grace"
        );
        let console = evidence.console_text();
        // Keep this qualified-native oracle on effects that survive guest
        // shutdown. The source-local File/process-boundary acceptance test
        // independently distinguishes Io(UnexpectedEof), BeaconParse and
        // UnexpectedBeaconMessage(Exec); serial console text is not that
        // typed-cause oracle.
        assert_eq!(
            evidence.guest_text().matches("EXIT ").count(),
            0,
            "failed running streams never emit a successful or duplicate EXIT"
        );
        assert!(console.contains("group-complete"), "{label} must reap before error: {console}");
        assert!(
            console.contains("poweroff-requested"),
            "{label} must reach the existing fatal poweroff path: {console}"
        );
        evidence.cleanup_proxy_dirs();
    }

    // A split valid EXEC must remain one frame.  Rapid adopted-child exits
    // exercise real SIGCHLD interruption/retry and the direct child's exit
    // races group disappearance/reaping (the naturally reachable
    // EINTR/ESRCH/ECHILD edges) without an injected syscall seam.
    let (split_terminal, split_evidence) = run_guest_control_case(
        &fixture,
        "vll-s10b-split-exec",
        GuestControlScript::SplitExec,
        "i=0; while [ $i -lt 32 ]; do (exit 0) & i=$((i + 1)); done\nwait\nexit 19",
        Duration::from_secs(15),
    )
    .await;
    assert_eq!(
        split_terminal.snapshot.rows.first().and_then(|row| row.exit_code),
        Some(19),
        "ECHILD after reaping cannot erase the direct child's saved status"
    );
    assert_eq!(split_evidence.guest_text().matches("EXIT 19\n").count(), 1);
    let split_console = split_evidence.console_text();
    assert!(split_console.contains("group-complete"), "split EXEC lost group completion");
    assert!(split_console.contains("poweroff-requested"), "split EXEC lost poweroff");
    split_evidence.cleanup_proxy_dirs();
}

/// CONTRACT_SHAPE: bounded-change.
/// S-VLL-11. Three named profiles, each 200 sequential + 200 with ten public
/// operator workers through one in-process serve: fresh READY (2/3s P95/P99),
/// immediate childless Job EXEC-release->normal VMM exit (0.5/1s), cooperative
/// TCP Service stop-entry->normal exit + complete driver artifact absence
/// (1/1.5s). Use exactly the approved warm-host-cache image/resources; retain
/// all 1200 scheduled trial records and nearest-rank quantiles. Any timeout,
/// forced kill, missing event or cleanup failure fails the whole healthy gate.
/// Guest and host clocks are never subtracted; queue, shim and cleanup stages
/// remain separate; calibrate bounded stage-event overhead against control.
#[allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    reason = "exact per-test contract declaration; the ignored native benchmark prints its retained fixture and distribution ledger"
)]
#[tokio::test]
#[serial(cgroup)]
async fn native_lifecycle_profiles_meet_stage_targets_without_dropping_trials() {
    let (events, trace_enabled) = install_lifecycle_trace();
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision VM fixture");
    let fixture_tmp = tempfile::Builder::new()
        .prefix("vll-s11-profiles-")
        .tempdir_in(shared_staging_root())
        .expect("profile fixture directory");
    let exit0 = build_exit_code_binary(fixture_tmp.path(), 0);
    let spin = build_spin_binary(fixture_tmp.path());
    let e08_server = fixture_tmp.path().join("e08-server");
    rustc_static_musl(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/service-kind-vm-workloads/guest_server.rs"),
        &e08_server,
    );
    let mut guest_init_hash = None;
    let profile_rootfs = with_mounted_rootfs_copy(fixture_tmp.path(), &fixture, |mount| {
        install_static_guest_shell(mount);
        install_guest_binary(&exit0, &mount.join("sbin/vll-exit0"));
        install_guest_binary(&spin, &mount.join("sbin/vll-spin"));
        install_guest_binary(&e08_server, &mount.join("opt/overdrive/examples/svm/e08-server"));
        guest_init_hash = Some(sha256sum_file(&mount.join("sbin/init")));
    });
    let current_test_binary = std::env::current_exe().expect("locate in-process test binary");
    let uname = Command::new("uname").arg("-a").output().expect("record native kernel inventory");
    assert!(uname.status.success(), "uname -a must describe the native fixture");
    let cpu_model = std::fs::read_to_string("/proc/cpuinfo")
        .expect("record native CPU inventory")
        .lines()
        .find(|line| line.starts_with("model name"))
        .unwrap_or("model name: unavailable")
        .to_owned();
    let memory_total = std::fs::read_to_string("/proc/meminfo")
        .expect("record native RAM inventory")
        .lines()
        .find(|line| line.starts_with("MemTotal:"))
        .unwrap_or("MemTotal: unavailable")
        .to_owned();
    eprintln!(
        "S-VLL-11 fixture: product_version={} in_process_binary_sha256={} kernel_sha256={} rootfs_sha256={} rootfs_size={} guest_init_sha256={} cooperative_server_sha256={} cloud_hypervisor_sha256={} cloud_hypervisor={} cpu={cpu_model:?} ram={memory_total:?} host_kernel={:?} cache=warm-pre-read cpu_milli=500 memory_bytes=134217728",
        env!("CARGO_PKG_VERSION"),
        sha256sum_file(&current_test_binary),
        sha256sum_file(&fixture.kernel_path),
        sha256sum_file(&profile_rootfs),
        std::fs::metadata(&profile_rootfs).expect("profile rootfs metadata").len(),
        guest_init_hash.expect("mounted profile rootfs contains production init"),
        sha256sum_file(&e08_server),
        sha256sum_file(&fixture.cloud_hypervisor_bin),
        fixture.cloud_hypervisor_version.trim(),
        String::from_utf8_lossy(&uname.stdout).trim(),
    );

    // Declared warm-host-cache lane: read each immutable launch input before
    // scheduling a cold VM.  Every trial still receives a fresh allocation,
    // run directory and rootfs clone; no snapshot/restore or VM pooling occurs.
    for input in [&fixture.kernel_path, &profile_rootfs] {
        let mut source = std::fs::File::open(input).expect("open immutable profile input");
        std::io::copy(&mut source, &mut std::io::sink()).expect("warm immutable input cache");
    }

    let (server, server_tmp) = spawn_vm_server().await;
    let cfg = config_path(server_tmp.path());
    let data_dir = server_tmp.path().join("data");
    let mut ledger = Vec::with_capacity(1_200);

    // Execute one real trial before committing the metal host to the full
    // matrix.  Missing approved stage events are an incomplete measurement,
    // not permission to fall back to deploy wall time or drop the sample.
    let instrumented_control_started = std::time::Instant::now();
    ledger.push(
        exercise_native_trial(
            NativeLifecycleProfile::Ready,
            0,
            Some(&events),
            cfg.clone(),
            server_tmp.path().to_path_buf(),
            fixture.kernel_path.clone(),
            profile_rootfs.clone(),
            data_dir.clone(),
        )
        .await,
    );
    let instrumented_control_elapsed = instrumented_control_started.elapsed();
    let first_alloc = ledger[0].alloc.to_string();
    let first_has_boundaries =
        events.lock().expect("lifecycle event mutex not poisoned").iter().any(|event| {
            event.name == "vm.lifecycle.create_enter"
                && event.fields.get("alloc") == Some(&first_alloc)
        });
    if !first_has_boundaries {
        server.shutdown().await.expect("shutdown RED preflight server");
        panic!("S-VLL-11: the first scheduled trial lacks the approved create_enter boundary");
    }

    trace_enabled.store(false, std::sync::atomic::Ordering::Relaxed);
    let uninstrumented_control_started = std::time::Instant::now();
    let uninstrumented_control = exercise_native_trial(
        NativeLifecycleProfile::Ready,
        9_999,
        None,
        cfg.clone(),
        server_tmp.path().to_path_buf(),
        fixture.kernel_path.clone(),
        profile_rootfs.clone(),
        data_dir.clone(),
    )
    .await;
    let uninstrumented_control_elapsed = uninstrumented_control_started.elapsed();
    trace_enabled.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        uninstrumented_control.failed.is_none(),
        "the uninstrumented calibration control must remain healthy: {uninstrumented_control:?}"
    );
    let bounded_event_overhead =
        instrumented_control_elapsed.saturating_sub(uninstrumented_control_elapsed);

    // The remaining sequential lane: 200 trials per named profile.  The
    // READY profile's ordinal zero above is part of this ledger, never a warmup
    // silently discarded from the distribution.
    for ordinal in 1..200 {
        ledger.push(
            exercise_native_trial(
                NativeLifecycleProfile::Ready,
                ordinal,
                Some(&events),
                cfg.clone(),
                server_tmp.path().to_path_buf(),
                fixture.kernel_path.clone(),
                profile_rootfs.clone(),
                data_dir.clone(),
            )
            .await,
        );
    }
    for profile in [NativeLifecycleProfile::FiniteJob, NativeLifecycleProfile::CooperativeService] {
        for ordinal in 0..200 {
            ledger.push(
                exercise_native_trial(
                    profile,
                    ordinal,
                    Some(&events),
                    cfg.clone(),
                    server_tmp.path().to_path_buf(),
                    fixture.kernel_path.clone(),
                    profile_rootfs.clone(),
                    data_dir.clone(),
                )
                .await,
            );
        }
    }

    // Concurrent lane: twenty cohorts of ten public operator workers for each
    // profile.  A cohort is fully joined and all ten results are appended even
    // when one reports a failure; no fail-fast join may truncate the ledger.
    for profile in [
        NativeLifecycleProfile::Ready,
        NativeLifecycleProfile::FiniteJob,
        NativeLifecycleProfile::CooperativeService,
    ] {
        for cohort in 0..20 {
            let trials = (0..10).map(|worker| {
                exercise_native_trial(
                    profile,
                    200 + cohort * 10 + worker,
                    Some(&events),
                    cfg.clone(),
                    server_tmp.path().to_path_buf(),
                    fixture.kernel_path.clone(),
                    profile_rootfs.clone(),
                    data_dir.clone(),
                )
            });
            ledger.extend(futures::future::join_all(trials).await);
        }
    }

    assert_eq!(ledger.len(), 1_200, "every scheduled trial remains in the ledger");
    for profile in [
        NativeLifecycleProfile::Ready,
        NativeLifecycleProfile::FiniteJob,
        NativeLifecycleProfile::CooperativeService,
    ] {
        let profile_trials =
            ledger.iter().filter(|trial| trial.profile == profile).collect::<Vec<_>>();
        assert_eq!(profile_trials.len(), 400, "exact per-profile scheduled cardinality");
        let ordinals = profile_trials
            .iter()
            .map(|trial| trial.ordinal)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            ordinals,
            (0..400).collect(),
            "200 sequential and 200 ten-worker trials are represented exactly once"
        );
    }

    let failures = ledger
        .iter()
        .filter_map(|trial| trial.failed.as_ref().map(|failure| (trial, failure)))
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "failed/timeout/cleanup trials remain visible: {failures:?}");

    let captured = events.lock().expect("lifecycle event mutex not poisoned").clone();
    let event_for_alloc = |alloc: &AllocationId, name: &str| {
        let alloc = alloc.to_string();
        captured
            .iter()
            .find(|event| event.name == name && event.fields.get("alloc") == Some(&alloc))
            .unwrap_or_else(|| panic!("missing {name} for allocation {alloc}"))
    };
    let reaped_for_alloc = |alloc: &AllocationId| {
        let created = event_for_alloc(alloc, "vm.lifecycle.created");
        let pid = created.fields.get("pid").expect("created event carries pid");
        captured
            .iter()
            .find(|event| {
                event.name == "vmm.process.reaped" && event.fields.get("pid") == Some(pid)
            })
            .unwrap_or_else(|| panic!("missing reaper observation for allocation {alloc}"))
    };

    let mut ready_samples = Vec::with_capacity(400);
    let mut job_samples = Vec::with_capacity(400);
    let mut service_samples = Vec::with_capacity(400);
    let mut admission_queue_samples = Vec::new();
    let mut driver_cleanup_samples = Vec::with_capacity(800);
    let mut reaper_to_cleanup_samples = Vec::with_capacity(1_200);
    let mut operator_stop_samples = Vec::with_capacity(800);
    let mut stop_admission_after_stop_samples = Vec::with_capacity(800);
    let mut terminal_after_stop_samples = Vec::with_capacity(800);
    for trial in &ledger {
        let target = format!("workload/{}", trial.workload_id);
        let target_owner_events = captured
            .iter()
            .filter(|event| event.fields.get("target") == Some(&target))
            .collect::<Vec<_>>();
        let target_admissions = target_owner_events
            .iter()
            .filter(|event| event.name == "convergence.evaluation.admitted")
            .copied()
            .collect::<Vec<_>>();
        assert!(
            !target_admissions.is_empty(),
            "missing owner admission for scheduled target {target}"
        );
        assert!(
            target_owner_events
                .iter()
                .any(|event| event.name == "convergence.evaluation.completed"),
            "missing owner-consumed completion for scheduled target {target}"
        );
        admission_queue_samples.extend(target_admissions.into_iter().map(|event| {
            Duration::from_millis(
                event
                    .fields
                    .get("queue_ms")
                    .unwrap_or_else(|| panic!("admission for {target} lacks queue_ms"))
                    .parse()
                    .unwrap_or_else(|_| panic!("admission for {target} has invalid queue_ms")),
            )
        }));

        let reaped = reaped_for_alloc(&trial.alloc);
        let created = event_for_alloc(&trial.alloc, "vm.lifecycle.created");
        let pid = created
            .fields
            .get("pid")
            .expect("created event carries the allocation-to-VMM correlation");
        assert!(
            reaped.fields.get("exit_code").is_some_and(|value| value == "0" || value == "Some(0)"),
            "healthy trial requires normal VMM exit: {reaped:?}"
        );
        assert!(
            reaped.fields.get("signal").is_none_or(|value| value == "None"),
            "healthy trial cannot include forced VMM death: {reaped:?}"
        );
        assert!(
            !Path::new("/proc").join(pid).exists(),
            "reaper observation and independent /proc absence must agree for pid {pid}"
        );
        let cleanup_calls = event_for_alloc(&trial.alloc, "vm.lifecycle.cleanup_calls_finished");
        assert!(cleanup_calls.at >= reaped.at, "driver cleanup preceded VMM reaping");
        reaper_to_cleanup_samples.push(cleanup_calls.at.duration_since(reaped.at));
        match trial.profile {
            NativeLifecycleProfile::Ready => {
                let enter = event_for_alloc(&trial.alloc, "vm.lifecycle.create_enter");
                let ready = event_for_alloc(&trial.alloc, "vm.lifecycle.ready");
                ready_samples.push(ready.at.duration_since(enter.at));
                let stop_enter = event_for_alloc(&trial.alloc, "vm.lifecycle.stop_enter");
                let writer = event_for_alloc(&trial.alloc, "vm.lifecycle.writer_finished");
                assert_eq!(
                    writer.fields.get("disposition").map(String::as_str),
                    Some("completed"),
                    "healthy READY teardown requires the accepted writer to complete"
                );
                assert!(writer.at >= stop_enter.at && cleanup_calls.at >= writer.at);
                driver_cleanup_samples.push(cleanup_calls.at.duration_since(stop_enter.at));
                operator_stop_samples
                    .push(trial.operator_stop_duration.expect("READY records public stop return"));
                stop_admission_after_stop_samples.push(
                    trial
                        .stop_admission_after_stop_return
                        .expect("READY records stop admission after public return"),
                );
                terminal_after_stop_samples.push(
                    trial
                        .terminal_after_stop_return
                        .expect("READY records terminal observation separately"),
                );
            }
            NativeLifecycleProfile::FiniteJob => {
                let released = event_for_alloc(&trial.alloc, "vm.beacon.exec.released");
                job_samples.push(reaped.at.duration_since(released.at));
            }
            NativeLifecycleProfile::CooperativeService => {
                let stop_enter = event_for_alloc(&trial.alloc, "vm.lifecycle.stop_enter");
                let writer = event_for_alloc(&trial.alloc, "vm.lifecycle.writer_finished");
                assert_eq!(
                    writer.fields.get("disposition").map(String::as_str),
                    Some("completed"),
                    "healthy cooperative Service requires the accepted writer to complete"
                );
                assert!(
                    reaped.at >= stop_enter.at,
                    "normal reaper observation must follow stop entry"
                );
                assert!(writer.at >= stop_enter.at && cleanup_calls.at >= writer.at);
                driver_cleanup_samples.push(cleanup_calls.at.duration_since(stop_enter.at));
                operator_stop_samples.push(
                    trial
                        .operator_stop_duration
                        .expect("Service records public stop return separately"),
                );
                stop_admission_after_stop_samples.push(
                    trial
                        .stop_admission_after_stop_return
                        .expect("Service records stop admission after public return"),
                );
                terminal_after_stop_samples.push(
                    trial
                        .terminal_after_stop_return
                        .expect("Service records terminal observation separately"),
                );
                assert!(trial.observed_cleanup_at >= cleanup_calls.at);
                service_samples.push(trial.observed_cleanup_at.duration_since(stop_enter.at));
            }
        }
    }

    // Nearest-rank empirical quantiles, 1-based ceil(p*n).  Median is retained
    // beside the approved P95/P99 gates so the complete distribution summary
    // is reviewable; no failed sample has been filtered above.
    let ready = duration_distribution(&ready_samples, "READY");
    let job = duration_distribution(&job_samples, "finite Job");
    let service = duration_distribution(&service_samples, "cooperative Service");
    assert!(
        ready.n == 400
            && ready.p95 <= Duration::from_secs(2)
            && ready.p99 <= Duration::from_secs(3),
        "READY n={} min={:?} p50={:?} p95={:?} p99={:?} max={:?}; bounded-stage-event control delta={bounded_event_overhead:?}",
        ready.n,
        ready.min,
        ready.p50,
        ready.p95,
        ready.p99,
        ready.max,
    );
    assert!(
        job.n == 400 && job.p95 <= Duration::from_millis(500) && job.p99 <= Duration::from_secs(1),
        "finite Job n={} min={:?} p50={:?} p95={:?} p99={:?} max={:?}",
        job.n,
        job.min,
        job.p50,
        job.p95,
        job.p99,
        job.max,
    );
    assert!(
        service.n == 400
            && service.p95 <= Duration::from_secs(1)
            && service.p99 <= Duration::from_millis(1_500),
        "cooperative Service n={} min={:?} p50={:?} p95={:?} p99={:?} max={:?}",
        service.n,
        service.min,
        service.p50,
        service.p95,
        service.p99,
        service.max,
    );

    let admission_queue = duration_distribution(&admission_queue_samples, "admission queue");
    let driver_cleanup = duration_distribution(&driver_cleanup_samples, "driver stop/cleanup");
    let reaper_to_cleanup =
        duration_distribution(&reaper_to_cleanup_samples, "reaper-to-driver-cleanup");
    let operator_stop = duration_distribution(&operator_stop_samples, "public stop return");
    let stop_admission_after_stop = duration_distribution(
        &stop_admission_after_stop_samples,
        "stop admission after public stop return",
    );
    let terminal_after_stop =
        duration_distribution(&terminal_after_stop_samples, "terminal observation after stop");
    eprintln!(
        "S-VLL-11 separate stages: admission_queue={admission_queue:?}; stop_admission_after_public_stop={stop_admission_after_stop:?}; driver_stop_to_cleanup={driver_cleanup:?}; reaper_to_driver_cleanup={reaper_to_cleanup:?}; public_stop_return={operator_stop:?}; terminal_after_public_stop={terminal_after_stop:?}"
    );

    server.shutdown().await.expect("shutdown persistent native profile server");
}
