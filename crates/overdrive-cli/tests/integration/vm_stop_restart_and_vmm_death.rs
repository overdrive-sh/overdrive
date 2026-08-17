//! Slice 03 / AC-11 — exit classification is guest-authoritative, never
//! derived from the hypervisor's own exit status (GH #42, brief §105 /
//! feature-delta `[D3]`).
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
//! # The four scenarios
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
#![allow(clippy::missing_panics_doc, clippy::unwrap_used, clippy::expect_used)]

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
use overdrive_core::transition_reason::StoppedBy;
use overdrive_core::vm::config::VmRunDir;
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

/// A real in-process `overdrive serve` — production `run_server` wiring
/// with only the dataplane and KEK external ports replaced by their
/// established simulation adapters (the same composition
/// `vm_walking_skeleton.rs`'s S-VM-01/02 use; these scenarios need a real
/// CH and the real exit-classification chain, not the real `EbpfDataplane`).
async fn spawn_vm_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
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
fn with_mounted_rootfs_copy(
    tmp: &Path,
    fixture: &VmFixture,
    edit: impl FnOnce(&Path),
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
        matches!(
            reason,
            TransitionReason::VmGuestExitUnreported { vmm_signal: None, .. }
        ),
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

    let reason = row.reason.clone().expect("a crashed VM allocation must carry a structured reason");
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
            Some(TransitionReason::Stopped {
                by: StoppedBy::Operator | StoppedBy::Reconciler
            }),
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
