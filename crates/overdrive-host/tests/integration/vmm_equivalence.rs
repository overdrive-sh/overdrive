//! Tier-3 acceptance — `Vmm` adapter equivalence (S-VM-90) plus the
//! `VmFixture` fail-loudly / idempotent-re-run smoke assertions (roadmap
//! step 01-06 AC6 — these have no test anywhere else in the roadmap).
//!
//! Gated `integration-tests,kvm-tests` (see `tests/integration.rs`) —
//! real Cloud Hypervisor boot needs `x86_64` + nested KVM, which this
//! workspace's Lima (arm64) cannot provide. Run via:
//!
//! ```text
//! cargo xtask metal run -- cargo nextest run -p overdrive-host \
//!   --features integration-tests,kvm-tests -E 'binary(integration)'
//! ```
//!
//! Per Mandate 9 (`nw-tdd-methodology`): this is a FIXED, hand-enumerated
//! call sequence at layer 3+ — `@example`, not `@property`.

#![allow(clippy::unused_async)]

use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::time::Duration;

use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::traits::vmm::{VmTermination, Vmm};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, KernelCmdline, KernelImage, MemoryPlan, RootfsPlan,
    VmConfig, VmConfinement, VmRunDir, VmmIdentity,
};
use overdrive_host::CloudHypervisorVmm;
use overdrive_sim::SimVmm;
use overdrive_testing::vm_fixture::{VmFixture, VmFixtureError, default_staging_root};
use serial_test::serial;

/// Small guest RAM — keeps every real boot in this suite light. Matches
/// the smallest point `MemoryPlan::derive`'s own measurement table
/// covers (128 MiB).
const GUEST_BYTES: u64 = 128 * 1024 * 1024;

fn read_kernel_header(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let file = std::fs::File::open(path).expect("open staged kernel for header read");
    let mut buf = Vec::new();
    file.take(KERNEL_MAGIC_WINDOW as u64).read_to_end(&mut buf).expect("read kernel header");
    buf
}

fn validated_kernel(fixture: &VmFixture) -> KernelImage {
    let header = read_kernel_header(&fixture.kernel_path);
    KernelImage::validate(fixture.kernel_path.clone(), HostArch::X86_64, &header)
        .expect("fixture-staged kernel validates for x86_64")
}

const fn sample_confinement() -> VmConfinement {
    VmConfinement::confined(
        VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
        1024,
    )
}

/// Build a real, fully-resolved `VmConfig` sharing `fixture`'s staged
/// kernel/rootfs. `alloc_suffix` controls the allocation id (and
/// therefore the derived rootfs clone destination + cgroup scope +
/// run-dir path) — callers reuse the SAME suffix across two calls to
/// exercise the "replace a stale clone destination" edge case.
fn sample_vm_config(fixture: &VmFixture, run_root: &Path, alloc_suffix: &str) -> VmConfig {
    let alloc = AllocationId::new(alloc_suffix).expect("valid alloc id");
    let master_bytes = std::fs::metadata(&fixture.rootfs_path).expect("stat staged rootfs").len();
    VmConfig {
        alloc: alloc.clone(),
        kernel: validated_kernel(fixture),
        rootfs: RootfsPlan::for_alloc(
            fixture.rootfs_path.clone(),
            master_bytes,
            &alloc,
            // Vmm-level test: the clone (FICLONE) is what's exercised. Stage it
            // on the master's OWN filesystem — `run_root` is under the fixture's
            // reflink-capable staging root, so FICLONE is intra-fs (a foreign-fs
            // staging dir is the EXDEV fail-closed path, not this sequence).
            // ADR-0082 2026-08-18 fourth amendment: the clone lives in a
            // platform-owned staging dir, never beside the operator's master.
            run_root,
            // The index link is `VmDriver`'s concern, so this dir is inert here.
            std::path::Path::new("/run/overdrive/vm/clone-index"),
        ),
        cmdline: KernelCmdline::platform_default(HostArch::X86_64),
        memory: MemoryPlan::derive(GUEST_BYTES),
        vcpus: NonZeroU8::new(1).expect("1 is nonzero"),
        run_dir: VmRunDir::for_alloc(run_root, &alloc),
        confinement: sample_confinement(),
        netns: None,
        cgroup_scope: CgroupPath::for_alloc(&alloc),
    }
}

/// The shared call sequence S-VM-90 drives against BOTH adapters: probe
/// (idempotent) -> create with `netns == None` (not an error) ->
/// terminate with a real grace window -> terminate again on the
/// now-dead VMM (idempotent `Ok(Killed)`) -> create again over the SAME
/// alloc id (replaces the stale clone destination, never adopts it) ->
/// terminate with `grace == ZERO` (immediate kill).
async fn drive_shared_sequence(vmm: &dyn Vmm, fixture: &VmFixture, run_root: &Path) {
    vmm.probe().await.expect("probe succeeds against a capable substrate");
    vmm.probe().await.expect("probe is idempotent -- second call also succeeds, no residue");

    let config_a = sample_vm_config(fixture, run_root, "vmmeq-a");
    std::fs::create_dir_all(config_a.run_dir.path()).expect("create run dir");
    let clone_dest = config_a.rootfs.clone_dest().to_path_buf();

    let proc1 = vmm.create(&config_a).await.expect("create succeeds with netns == None");
    assert!(clone_dest.exists(), "the rootfs clone destination must exist after create()");

    // At this (process-half) layer EITHER outcome is valid: the guest never
    // beacons in step 01-06's scope, so a real, un-beaconed CH process may
    // legitimately land in `ExitedWithinGrace` or `Killed` within the grace
    // window, and this call site cannot predict which. This step only
    // asserts that `terminate()` SUCCEEDS (see the `.expect(...)` below);
    // the discriminating assertions on WHICH variant comes back live on
    // `outcome_again` and `zero_grace_outcome` further down (both assert
    // `Killed`).
    vmm.terminate(&proc1.control, Duration::from_secs(5))
        .await
        .expect("terminate succeeds against a live VMM");

    let outcome_again = vmm
        .terminate(&proc1.control, Duration::from_secs(5))
        .await
        .expect("terminate on an already-dead VMM is Ok, not an error");
    assert!(
        matches!(outcome_again, VmTermination::Killed),
        "an already-dead VMM must ALWAYS report Killed (idempotent), got {outcome_again:?}"
    );

    // Second create() over the SAME alloc id -- clone_dest is IDENTICAL to
    // config_a's, and still exists on disk from the run above. The edge
    // case: create() must REPLACE it, not adopt it untouched.
    let config_b = sample_vm_config(fixture, run_root, "vmmeq-a");
    assert_eq!(
        config_b.rootfs.clone_dest(),
        clone_dest.as_path(),
        "same alloc id must derive the same clone destination -- precondition for this edge case"
    );
    std::fs::create_dir_all(config_b.run_dir.path()).expect("create run dir (idempotent)");
    let proc2 = vmm.create(&config_b).await.expect("create replaces a stale clone destination");
    assert!(clone_dest.exists(), "the replaced clone destination must exist");

    let zero_grace_outcome = vmm
        .terminate(&proc2.control, Duration::ZERO)
        .await
        .expect("terminate with grace == ZERO kills immediately, with no await");
    assert!(
        matches!(zero_grace_outcome, VmTermination::Killed),
        "grace == ZERO must resolve to Killed, got {zero_grace_outcome:?}"
    );
}

#[tokio::test]
async fn vmm_equivalence_sim_vmm() {
    let staging_root = default_staging_root();
    let fixture = VmFixture::provision(&staging_root).expect("fixture provisions on this host");
    let run_root = staging_root.join("run-sim");

    let vmm = SimVmm::new();
    drive_shared_sequence(&vmm, &fixture, &run_root).await;

    // create() removes its clone if the spawn fails -- SimVmm's injection
    // hook exercises the edge case the real adapter proves via a broken
    // binary path in the companion CH test below.
    let config = sample_vm_config(&fixture, &run_root, "vmmeq-fail");
    std::fs::create_dir_all(config.run_dir.path()).expect("create run dir");
    let clone_dest = config.rootfs.clone_dest().to_path_buf();
    vmm.inject_create_failure();
    let result = vmm.create(&config).await;
    assert!(result.is_err(), "an injected spawn failure must surface as Err");
    assert!(
        !clone_dest.exists(),
        "create() must remove its clone when the spawn fails -- no partial artifact may remain"
    );
}

#[tokio::test]
async fn vmm_equivalence_cloud_hypervisor_vmm() {
    let staging_root = default_staging_root();
    let fixture = VmFixture::provision(&staging_root).expect("fixture provisions on this host");
    let run_root = staging_root.join("run-ch");

    let vmm = CloudHypervisorVmm::new()
        .with_image_dir(staging_root.join("probe-image-dir"))
        .with_run_dir_root(staging_root.join("probe-run-dir-root"));
    drive_shared_sequence(&vmm, &fixture, &run_root).await;

    // create() removes its clone if the spawn fails -- exercised here via
    // a deliberately broken `cloud-hypervisor` binary path so
    // `Command::spawn` itself fails, after the FICLONE clone has already
    // succeeded.
    let broken =
        CloudHypervisorVmm::new().with_binary(PathBuf::from("/nonexistent/cloud-hypervisor"));
    let config = sample_vm_config(&fixture, &run_root, "vmmeq-fail-ch");
    std::fs::create_dir_all(config.run_dir.path()).expect("create run dir");
    let clone_dest = config.rootfs.clone_dest().to_path_buf();
    let result = broken.create(&config).await;
    assert!(result.is_err(), "a broken cloud-hypervisor binary path must surface as Err");
    assert!(
        !clone_dest.exists(),
        "create() must remove its clone when the spawn fails -- no partial artifact may remain"
    );
}

// ---------------------------------------------------------------------
// VmFixture smoke -- fail-loudly-on-missing-prerequisite + idempotent
// re-run (roadmap step 01-06 AC6; no other test in the roadmap covers
// either behavior).
// ---------------------------------------------------------------------

/// Every directory in `PATH` EXCEPT the one containing `cloud-hypervisor`
/// — leaves `systemd-detect-virt` / `uname` / `mkfs.ext4` / `which` (every
/// OTHER tool `VmFixture::provision` needs) resolvable, so the ONLY
/// prerequisite this test deliberately breaks is `cloud-hypervisor`
/// itself.
fn path_without_cloud_hypervisor() -> String {
    let ch_dir = std::process::Command::new("which")
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

#[tokio::test]
#[serial(env)]
async fn vm_fixture_provision_fails_loudly_on_a_broken_cloud_hypervisor_path() {
    let original_path = std::env::var_os("PATH");
    let broken_path = path_without_cloud_hypervisor();
    // SAFETY: `#[serial(env)]` guarantees exclusive access to `PATH` for
    // the duration of this test.
    unsafe {
        std::env::set_var("PATH", &broken_path);
    }

    let staging_root = std::env::temp_dir().join("overdrive-vmm-fail-loudly-test");
    let result = tokio::task::spawn_blocking(move || VmFixture::provision(&staging_root))
        .await
        .expect("blocking task did not panic");

    // SAFETY: restoring the pre-test PATH; still inside the
    // `#[serial(env)]` window.
    unsafe {
        match &original_path {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }
    }

    let err =
        result.expect_err("provision must fail when cloud-hypervisor is unresolvable on PATH");
    assert!(
        matches!(err, VmFixtureError::CloudHypervisorMissing { .. }),
        "a broken cloud-hypervisor path must fail with a NAMED, actionable error -- never a \
         downstream timeout. got: {err:?}"
    );
}

#[tokio::test]
async fn vm_fixture_provision_is_idempotent_and_reuses_staged_artifacts() {
    let staging_root = default_staging_root();

    let first = VmFixture::provision(&staging_root).expect("first provision succeeds");
    let rootfs_mtime_before =
        std::fs::metadata(&first.rootfs_path).expect("stat rootfs").modified().expect("mtime");
    let kernel_mtime_before =
        std::fs::metadata(&first.kernel_path).expect("stat kernel").modified().expect("mtime");

    let second = VmFixture::provision(&staging_root)
        .expect("second provision succeeds (re-verify, not re-bake)");

    assert_eq!(first.kernel_path, second.kernel_path, "re-run must resolve the SAME kernel path");
    assert_eq!(first.rootfs_path, second.rootfs_path, "re-run must resolve the SAME rootfs path");

    let rootfs_mtime_after =
        std::fs::metadata(&second.rootfs_path).expect("stat rootfs").modified().expect("mtime");
    let kernel_mtime_after =
        std::fs::metadata(&second.kernel_path).expect("stat kernel").modified().expect("mtime");
    assert_eq!(
        rootfs_mtime_before, rootfs_mtime_after,
        "a second provision() must REUSE the staged rootfs image, not rebuild it"
    );
    assert_eq!(
        kernel_mtime_before, kernel_mtime_after,
        "a second provision() must REUSE the staged kernel, not re-stage it"
    );
}

// ---------------------------------------------------------------------
// Step 03-05 / DWD-24 — bounded VMM diagnostics equivalence.
// ---------------------------------------------------------------------

/// `@contract-shape:pure-function` — repeated live reads and the terminal
/// `VmmExit` projection observe ONE bounded diagnostic stream: the live
/// console tail and the final stderr tail are coherent for both adapters.
///
/// This is what makes the boot-deadline arm trustworthy. If the live
/// snapshot and the terminal projection were assembled separately, an
/// operator reading a deadline failure and an operator reading the
/// eventual exit row would be told different things about one process.
#[tokio::test]
async fn vmm_live_console_tail_is_coherent_with_final_exit_stderr_tail_for_both_adapters() {
    let staging_root = default_staging_root();
    let fixture = VmFixture::provision(&staging_root).expect("fixture provisions on this host");

    // --- SimVmm: scripted output, observed live and then finally. ---
    let sim_run_root = staging_root.join("run-sim-tail");
    let sim = SimVmm::new();
    let scripted = "cloud-hypervisor: warn: vsock ready\ncloud-hypervisor: fatal: giving up";
    sim.inject_console_output(scripted.as_bytes().to_vec());
    sim.inject_exit(Some(3), None);

    let config = sample_vm_config(&fixture, &sim_run_root, "vmmeq-tail-sim");
    std::fs::create_dir_all(config.run_dir.path()).expect("create run dir");
    let mut process = sim.create(&config).await.expect("sim create succeeds");

    // Repeated live reads are stable with no intervening append.
    let live_first = process.diagnostics.console_tail();
    let live_second = process.diagnostics.console_tail();
    assert_eq!(live_first, live_second, "two live reads with no append must be equal");
    assert_eq!(
        live_first.as_deref(),
        Some(scripted),
        "the live tail must render exactly what was captured",
    );

    sim.terminate(&process.control, Duration::ZERO).await.expect("terminate the sim process");
    let sim_exit = process.exit.recv().await.expect("the sim process reports its ending");
    assert_eq!(
        sim_exit.stderr_tail, live_first,
        "SimVmm's final stderr tail must be the SAME capture the live reader observed",
    );

    // --- CloudHypervisorVmm: a real process that really writes stderr. ---
    let ch_run_root = staging_root.join("run-ch-tail");
    let ch_config = sample_vm_config(&fixture, &ch_run_root, "vmmeq-tail-ch");
    std::fs::create_dir_all(ch_config.run_dir.path()).expect("create run dir");
    let ch = CloudHypervisorVmm::new();
    let mut ch_process = ch.create(&ch_config).await.expect("real CH spawns");

    let ch_exit = ch_process.exit.recv().await.expect("the real CH process reports its ending");
    let ch_live = ch_process.diagnostics.console_tail();
    assert_eq!(
        ch_exit.stderr_tail, ch_live,
        "CloudHypervisorVmm's final stderr tail must be the SAME capture the live reader \
         observes — a separately assembled tail is exactly the incoherence this guards",
    );
    let _ = ch.terminate(&ch_process.control, Duration::ZERO).await;
}

/// `@contract-shape:bounded-change` — an early process exit retains its
/// exact exit code, terminating signal, and final stderr tail through
/// both the `SimVmm` and real Cloud Hypervisor adapters.
#[tokio::test]
async fn vmm_early_exit_preserves_code_signal_and_stderr_tail_for_both_adapters() {
    let staging_root = default_staging_root();
    let fixture = VmFixture::provision(&staging_root).expect("fixture provisions on this host");

    // --- SimVmm: a scripted ending with an exact code and signal. ---
    let sim_run_root = staging_root.join("run-sim-early");
    let sim = SimVmm::new();
    sim.inject_console_output(b"cloud-hypervisor: fatal: bad disk".to_vec());
    sim.inject_exit(Some(101), Some(11));

    let config = sample_vm_config(&fixture, &sim_run_root, "vmmeq-early-sim");
    std::fs::create_dir_all(config.run_dir.path()).expect("create run dir");
    let mut process = sim.create(&config).await.expect("sim create succeeds");
    sim.terminate(&process.control, Duration::ZERO).await.expect("terminate the sim process");

    let sim_exit = process.exit.recv().await.expect("the sim process reports its ending");
    assert_eq!(sim_exit.exit_code, Some(101), "the scripted exit code must survive");
    assert_eq!(sim_exit.signal, Some(11), "the scripted terminating signal must survive");
    assert_eq!(
        sim_exit.stderr_tail.as_deref(),
        Some("cloud-hypervisor: fatal: bad disk"),
        "the captured stderr must survive alongside the code and signal",
    );

    // --- CloudHypervisorVmm: a REAL process ending, really observed. ---
    let ch_run_root = staging_root.join("run-ch-early");
    let ch_config = sample_vm_config(&fixture, &ch_run_root, "vmmeq-early-ch");
    std::fs::create_dir_all(ch_config.run_dir.path()).expect("create run dir");
    let ch = CloudHypervisorVmm::new();
    let mut ch_process = ch.create(&ch_config).await.expect("real CH spawns");

    // Kill it and observe the REAL reported ending — the adapter must
    // report a code or a signal, never silence.
    let _ = ch.terminate(&ch_process.control, Duration::ZERO).await;
    let ch_exit = ch_process.exit.recv().await.expect("the real CH process reports its ending");
    assert!(
        ch_exit.exit_code.is_some() || ch_exit.signal.is_some(),
        "a real ending must report a code or a signal, not silence: {ch_exit:?}",
    );
}
