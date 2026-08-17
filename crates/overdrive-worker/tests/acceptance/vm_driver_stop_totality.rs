//! S-VM-76 + crafter-authored race-arm examples (ADR-0082 §§D3-D4,
//! GH #42) — `VmDriver`'s component-scope acceptance suite against
//! `SimVmm`.
//!
//! Per Mandate 9 / this step's roadmap notes: FIXED, hand-enumerated
//! call sequences (`@example`), not generated input. `vmm_equivalence.rs`
//! drives the `Vmm` port only and structurally cannot reach `VmDriver`'s
//! relocated guest half (the beacon session) — this file is the
//! enforcement vehicle ADR-0082 §D4 names by name.
//!
//! S-VM-14 (deadline-arm leak) and S-VM-15 (EXIT-report priority) are
//! AC-06's Tier-3 `@real-io` evidence for this SAME race and cleanup
//! logic, committed at the walking skeleton (step 01-08) against a real
//! Cloud Hypervisor boot — deliberately NOT duplicated here as `SimVmm`
//! shadows (that is the testing-theatre shape DISTILL rejected at
//! S-VM-75's own precedent). This file's four crafter-authored
//! race-arm/EXIT-drain examples are the component-scope RED/GREEN
//! evidence for the SAME production code those Tier-3 scenarios prove
//! against a real substrate.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::SpiffeId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, Driver, ExitKind, Resources,
};
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmExitWatch, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::vm::beacon::{BEACON_VSOCK_PORT, BeaconMessage};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, RootfsPlan, VmConfig, VmConfinement, VmRunDir, VmmIdentity,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_worker::VmDriver;
use overdrive_worker::vm_driver::VmHostLayout;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

// ---------------------------------------------------------------------
// Fixture builders
// ---------------------------------------------------------------------

/// Yield to the tokio scheduler enough times that a spawned task's
/// first poll of a `clock.sleep(...)` future registers its waker
/// against the `SimClock` timer registry BEFORE the next `tick(...)`
/// fires. Matches the established pattern in
/// `probe_runner_supervised_tick.rs::yield_for_task_poll` — without
/// this, `tick` races `tokio::spawn`'s first poll and the deadline
/// computed inside `sleep` ends up relative to a LATER baseline than
/// intended, so the tick never catches it.
async fn yield_for_task_poll() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// The kernel image this fixture stages, and the path the `[vm]` spec
/// built by [`build_spec`] names. ADR-0083 §D3a: artifacts are
/// per-allocation, so the driver reads THIS path out of the spec's own
/// `VmPayload` — there is no node-level artifact anywhere to fall back to.
fn fixture_kernel_path(tmp: &TempDir) -> PathBuf {
    tmp.path().join("vmlinuz")
}

/// The master rootfs image this fixture stages, and the path the `[vm]`
/// spec built by [`build_spec`] names.
fn fixture_rootfs_path(tmp: &TempDir) -> PathBuf {
    tmp.path().join("master.img")
}

/// Stage both artifacts on disk. The bytes must REALLY be there:
/// `VmDriver::start` opens the path the spec names and re-validates the
/// kernel header for every allocation (ADR-0082 §D2.4 / ADR-0083 §D3b),
/// which is what makes a deleted or replaced artifact observable as an
/// allocation failure. Called by [`build_layout`], which every test
/// invokes before [`build_spec`].
fn stage_fixture_artifacts(tmp: &TempDir) {
    std::fs::write(fixture_rootfs_path(tmp), b"deterministic-fixture-rootfs-bytes")
        .expect("write synthetic master rootfs");
    let mut header = vec![0u8; KERNEL_MAGIC_WINDOW];
    header[..4].copy_from_slice(b"\x7fELF");
    std::fs::write(fixture_kernel_path(tmp), &header).expect("stage the synthetic kernel image");
}

/// Build a per-test [`VmHostLayout`] rooted at `tmp`: a real (tiny)
/// master rootfs file `SimVmm::create` FICLONE-copies, a
/// pre-validated synthetic-ELF [`KernelImage`], and a dedicated
/// `run`/`cgroup` subtree so parallel tests never collide.
fn build_layout(tmp: &TempDir) -> VmHostLayout {
    stage_fixture_artifacts(tmp);
    VmHostLayout {
        cgroup_root: tmp.path().join("cgroup"),
        run_dir_root: tmp.path().join("run"),
        arch: HostArch::X86_64,
        vcpus: NonZeroU8::new(1).expect("1 != 0"),
        confinement: VmConfinement::confined(
            VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
            1024,
        ),
    }
}

fn build_spec(alloc: &AllocationId, tmp: &TempDir) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/workload/vm-driver-test/alloc/x")
            .expect("valid spiffe id"),
        driver: overdrive_core::traits::driver::DriverPayload::Vm(
            overdrive_core::traits::driver::VmPayload {
                command: "/sbin/init".to_owned(),
                args: vec![],
                kernel: fixture_kernel_path(tmp),
                rootfs: fixture_rootfs_path(tmp),
                volumes: vec![],
            },
        ),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    }
}

fn build_driver(vmm: std::sync::Arc<dyn Vmm>, layout: VmHostLayout) -> (VmDriver, SimClock) {
    let clock = SimClock::new();
    let fs: std::sync::Arc<dyn overdrive_core::traits::CgroupFs> =
        std::sync::Arc::new(SimCgroupFs::new());
    let cgroup_accounting: std::sync::Arc<
        dyn overdrive_core::traits::cgroup_accounting::CgroupAccounting,
    > = std::sync::Arc::new(SimCgroupAccounting::new());
    let driver =
        VmDriver::new(vmm, std::sync::Arc::new(clock.clone()), fs, cgroup_accounting, layout);
    (driver, clock)
}

/// Like [`build_driver`], but additionally returns the concrete
/// [`SimCgroupFs`] handle so callers can assert on cgroup-scope
/// removal via [`SimCgroupFs::snapshot`] (01-07 review remediation,
/// BLOCKER D2). Mirrors the established `SimVmm`-handle-retention
/// pattern already used throughout this file: the fs is cloned before
/// being erased to the trait object, so both the driver and the test
/// observe the same `Arc<Mutex<...>>`-backed mutations.
fn build_driver_with_cgroup_fs(
    vmm: std::sync::Arc<dyn Vmm>,
    layout: VmHostLayout,
) -> (VmDriver, SimClock, SimCgroupFs) {
    let clock = SimClock::new();
    let cgroup_fs = SimCgroupFs::new();
    let fs: std::sync::Arc<dyn overdrive_core::traits::CgroupFs> =
        std::sync::Arc::new(cgroup_fs.clone());
    let cgroup_accounting: std::sync::Arc<
        dyn overdrive_core::traits::cgroup_accounting::CgroupAccounting,
    > = std::sync::Arc::new(SimCgroupAccounting::new());
    let driver =
        VmDriver::new(vmm, std::sync::Arc::new(clock.clone()), fs, cgroup_accounting, layout);
    (driver, clock, cgroup_fs)
}

fn beacon_socket_path(run_dir_root: &Path, alloc: &AllocationId) -> PathBuf {
    VmRunDir::for_alloc(run_dir_root, alloc).beacon_socket(BEACON_VSOCK_PORT)
}

/// Retry-connect to the beacon `UnixListener` — robust against the
/// listener not being bound the instant the spawned `start()` task is
/// scheduled, with no reliance on driver-internal timing.
async fn connect_with_retry(path: &Path) -> UnixStream {
    for _ in 0..2000 {
        match UnixStream::connect(path).await {
            Ok(stream) => return stream,
            Err(_) => tokio::task::yield_now().await,
        }
    }
    panic!("beacon listener never became connectable at {}", path.display());
}

/// Spawn `driver.start(&spec)`, dial the beacon, write `READY`, and
/// await the spawned task to completion — the shared "beacon wins the
/// boot race" happy path several scenarios below build on.
async fn start_with_beacon_accepted(
    driver: &VmDriver,
    spec: &AllocationSpec,
    run_dir_root: &Path,
) -> (AllocationHandle, UnixStream) {
    let beacon_path = beacon_socket_path(run_dir_root, &spec.alloc);
    let driver = driver.clone();
    let spec_owned = spec.clone();
    let start_task = tokio::spawn(async move { driver.start(&spec_owned).await });

    let mut stream = connect_with_retry(&beacon_path).await;
    stream.write_all(b"READY pid=1 port=1234\n").await.expect("write READY");

    let handle = start_task
        .await
        .expect("start task did not panic")
        .expect("start resolves Ok once the beacon accepts");
    (handle, stream)
}

/// Test-only `Vmm` decorator: right after `create()` succeeds, fires a
/// background task that immediately terminates the just-created
/// process — simulating "the VMM died before the guest could beacon"
/// for the exit-arm-wins race scenario, entirely through `SimVmm`'s
/// EXISTING public `Vmm` surface (`terminate`), with no modification to
/// `overdrive-sim` itself.
#[derive(Clone)]
struct DiesBeforeBeacon {
    inner: SimVmm,
}

#[async_trait]
impl Vmm for DiesBeforeBeacon {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        let process = self.inner.create(config).await?;
        let control = process.control.clone();
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let _ = inner.terminate(&control, Duration::ZERO).await;
        });
        Ok(process)
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

// ---------------------------------------------------------------------
// AC #1 — claim taken at step 0; every non-Ok race arm releases it and
// cleans up the scope/dir/clone/VMM.
// ---------------------------------------------------------------------

/// Crafter-authored race-arm example: `Vmm::create` itself fails (the
/// earliest possible post-claim failure point). The claim taken at
/// step 0 must be released and the run directory removed even though
/// no VMM process or rootfs clone ever came into existence.
#[tokio::test]
async fn create_failure_releases_claim_and_cleans_up_run_directory() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let rootfs_master = fixture_rootfs_path(&tmp);
    let cgroup_root = layout.cgroup_root.clone();
    let sim = SimVmm::new();
    sim.inject_create_failure();
    let (driver, _clock, cgroup_fs) = build_driver_with_cgroup_fs(std::sync::Arc::new(sim), layout);

    let alloc = AllocationId::new("alloc-create-fail").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    let err = driver.start(&spec).await.expect_err("Vmm::create failure rejects start");
    assert!(matches!(err, overdrive_core::traits::driver::DriverError::StartRejected { .. }));

    assert_eq!(
        driver.live_allocations(),
        Some(Vec::new()),
        "claim taken at step 0 must be released on a Vmm::create failure"
    );

    let run_dir = VmRunDir::for_alloc(&run_dir_root, &alloc);
    assert!(
        !run_dir.path().exists(),
        "run directory must be removed after a Vmm::create failure, still present at {}",
        run_dir.path().display()
    );

    // @mandatory:mutation_target companions (01-07 review D2) — a
    // mutant that drops either cleanup call in
    // `cleanup_after_start_failure` must not survive with only the
    // run-directory assertion above.
    let master_bytes =
        std::fs::metadata(&rootfs_master).expect("stat synthetic master rootfs").len();
    let rootfs_plan = RootfsPlan::for_alloc(rootfs_master, master_bytes, &alloc);
    assert!(
        !rootfs_plan.clone_dest().exists(),
        "rootfs clone must be removed after a Vmm::create failure, still present at {}",
        rootfs_plan.clone_dest().display()
    );

    // 01-07 review D2 closure: cgroup-scope removal via
    // `SimCgroupFs::snapshot`. `SimCgroupFs::remove_dir` now models
    // real cgroup v2 `rmdir` semantics faithfully — the
    // controller-interface files this arm already wrote
    // (`cpu.weight`, `memory.max` from `write_resource_limits`, plus
    // `cgroup.kill` from this same cleanup path) are kernel-managed
    // pseudo-files that real cgroupfs auto-reaps on `rmdir(2)` and
    // must not block removal (see the `CgroupFs::remove_dir` trait
    // rustdoc + the Tier-3 `rmdir_auto_reap` evidence).
    let scope_dir = CgroupPath::for_alloc(&alloc).resolve(&cgroup_root);
    let snap = cgroup_fs.snapshot();
    assert!(
        !snap.contains_key(&scope_dir),
        "cgroup scope must be removed after a Vmm::create failure, still present at {}",
        scope_dir.display()
    );
}

/// Crafter-authored race-arm example: the VMM exits before the guest
/// ever beacons (ADR-0082 §D3's `exit.recv()` arm winning). The claim
/// must be released, the rootfs clone and run directory removed, and
/// `Vmm::terminate` invoked (observable via `SimVmm::is_live`).
#[tokio::test]
async fn vmm_exits_before_beacon_releases_claim_and_cleans_up() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let rootfs_master = fixture_rootfs_path(&tmp);
    let cgroup_root = layout.cgroup_root.clone();
    let sim = SimVmm::new();
    let dies_before_beacon = DiesBeforeBeacon { inner: sim.clone() };
    let (driver, _clock, cgroup_fs) =
        build_driver_with_cgroup_fs(std::sync::Arc::new(dies_before_beacon), layout);

    let alloc = AllocationId::new("alloc-exit-before-beacon").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    let err = driver.start(&spec).await.expect_err("VMM exit before beacon rejects start");
    assert!(matches!(err, overdrive_core::traits::driver::DriverError::StartRejected { .. }));

    assert_eq!(
        driver.live_allocations(),
        Some(Vec::new()),
        "claim taken at step 0 must be released when the VMM exits before the guest beacons"
    );

    let run_dir = VmRunDir::for_alloc(&run_dir_root, &alloc);
    assert!(
        !run_dir.path().exists(),
        "run directory must be removed on the exit-before-beacon arm, still present at {}",
        run_dir.path().display()
    );

    // No pid was ever handed back (start failed), so confirm via the
    // sim's own live-process bookkeeping: the FIRST pid it ever minted
    // (base 1_000_000, per SimVmm's documented construction) must no
    // longer be tracked as live.
    assert!(
        !sim.is_live(1_000_000),
        "Vmm::terminate must have been invoked on the exit-before-beacon arm"
    );

    // @mandatory:mutation_target companions (01-07 review D2).
    let master_bytes =
        std::fs::metadata(&rootfs_master).expect("stat synthetic master rootfs").len();
    let rootfs_plan = RootfsPlan::for_alloc(rootfs_master, master_bytes, &alloc);
    assert!(
        !rootfs_plan.clone_dest().exists(),
        "rootfs clone must be removed on the exit-before-beacon arm, still present at {}",
        rootfs_plan.clone_dest().display()
    );

    // 01-07 review D2 closure: cgroup-scope removal — see the
    // rationale documented in
    // `create_failure_releases_claim_and_cleans_up_run_directory`
    // above (`SimCgroupFs::remove_dir` now models real cgroup v2
    // `rmdir` auto-reap of kernel-managed pseudo-files).
    let scope_dir = CgroupPath::for_alloc(&alloc).resolve(&cgroup_root);
    let snap = cgroup_fs.snapshot();
    assert!(
        !snap.contains_key(&scope_dir),
        "cgroup scope must be removed on the exit-before-beacon arm, still present at {}",
        scope_dir.display()
    );
}

/// Crafter-authored race-arm example: nothing ever beacons and nothing
/// ever exits — `VM_BOOT_DEADLINE` (30 s) elapses, driven via
/// `SimClock::tick` (never a real 30 s wait). Slice 03's "no leaked
/// hypervisor processes or rootfs copies" AC, on the arm an
/// implementation is most likely to leak on.
#[tokio::test]
async fn boot_deadline_elapses_releases_claim_and_cleans_up() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let rootfs_master = fixture_rootfs_path(&tmp);
    let cgroup_root = layout.cgroup_root.clone();
    let sim = SimVmm::new();
    let (driver, clock, cgroup_fs) =
        build_driver_with_cgroup_fs(std::sync::Arc::new(sim.clone()), layout);

    let alloc = AllocationId::new("alloc-deadline").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    let driver_for_task = driver.clone();
    let spec_owned = spec.clone();
    let start_task = tokio::spawn(async move { driver_for_task.start(&spec_owned).await });

    // Let the spawned task reach the three-way select! and register
    // its `clock.sleep(VM_BOOT_DEADLINE)` waker BEFORE advancing —
    // nothing dials the beacon and nothing calls `terminate`, so only
    // the deadline arm can ever resolve this race. Real filesystem I/O
    // (`create_dir_all`, `fs::metadata` against this test's real
    // tempdir) sits on the path to that registration; a pure
    // `yield_now()` burst never forces the runtime to poll for the
    // blocking pool's completion notification when there is always
    // other ready work to round-robin against (this task and
    // `start_task` are both perpetually ready), so it cannot robustly
    // outlast that I/O. A real (sub-millisecond-budget) sleep DOES
    // force a reactor turn — mix one into the retry: a tick issued
    // before the sleep has registered is a harmless no-op (nothing
    // pending to wake), and the moment the sleep DOES register, the
    // very next +30s tick crosses its deadline from whatever baseline
    // it captured.
    for _ in 0..200 {
        if start_task.is_finished() {
            break;
        }
        yield_for_task_poll().await;
        tokio::time::sleep(Duration::from_millis(1)).await;
        clock.tick(Duration::from_secs(30));
    }

    let err = start_task
        .await
        .expect("start task did not panic")
        .expect_err("boot deadline rejects start when nothing ever beacons");
    assert!(matches!(err, overdrive_core::traits::driver::DriverError::StartRejected { .. }));

    assert_eq!(
        driver.live_allocations(),
        Some(Vec::new()),
        "claim taken at step 0 must be released when the boot deadline elapses"
    );

    let run_dir = VmRunDir::for_alloc(&run_dir_root, &alloc);
    assert!(
        !run_dir.path().exists(),
        "run directory must be removed on the deadline arm, still present at {}",
        run_dir.path().display()
    );
    assert!(!sim.is_live(1_000_000), "Vmm::terminate must have been invoked on the deadline arm");

    // @mandatory:mutation_target companions (01-07 review D2).
    let master_bytes =
        std::fs::metadata(&rootfs_master).expect("stat synthetic master rootfs").len();
    let rootfs_plan = RootfsPlan::for_alloc(rootfs_master, master_bytes, &alloc);
    assert!(
        !rootfs_plan.clone_dest().exists(),
        "rootfs clone must be removed on the deadline arm, still present at {}",
        rootfs_plan.clone_dest().display()
    );

    // 01-07 review D2 closure: cgroup-scope removal — see the
    // rationale documented in
    // `create_failure_releases_claim_and_cleans_up_run_directory`
    // above (`SimCgroupFs::remove_dir` now models real cgroup v2
    // `rmdir` auto-reap of kernel-managed pseudo-files).
    let scope_dir = CgroupPath::for_alloc(&alloc).resolve(&cgroup_root);
    let snap = cgroup_fs.snapshot();
    assert!(
        !snap.contains_key(&scope_dir),
        "cgroup scope must be removed on the deadline arm, still present at {}",
        scope_dir.display()
    );
}

// ---------------------------------------------------------------------
// AC #2 — the guest EXIT report is drained and read to completion
// before the ExitEvent is emitted; the VMM's own subsequent teardown
// exit never overwrites it. ExitKind is derived from the guest report
// only.
// ---------------------------------------------------------------------

/// Crafter-authored example: the guest reports `EXIT 7`, and the VMM's
/// OWN process exit (the hypervisor's teardown, simulated here via a
/// direct `Vmm::terminate` call — the SAME thing a self-powered-off
/// guest triggers) follows afterward. The emitted `ExitEvent.kind` must
/// reflect the guest's report (`Crashed { exit_code: Some(7), .. }`),
/// never a fallback derived from the VMM's own exit.
#[tokio::test]
async fn guest_exit_report_is_authoritative_over_subsequent_vmm_teardown() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let sim = SimVmm::new();
    let (driver, _clock) = build_driver(std::sync::Arc::new(sim.clone()), layout);

    let alloc = AllocationId::new("alloc-exit-report").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    let mut exit_rx = driver.take_exit_receiver().expect("exit receiver available exactly once");

    let (handle, mut stream) = start_with_beacon_accepted(&driver, &spec, &run_dir_root).await;

    // The VMM's OWN teardown exit (the same event a self-powered-off
    // guest triggers on the real substrate) resolves FIRST this time
    // (01-07 review D4) — forcing the exit watcher's
    // `GUEST_REPORT_DRAIN_MAX_YIELDS` retry loop to actually iterate
    // while it waits out the guest's own report, written only after a
    // short cooperative-yield delay below. A prior version of this test
    // wrote the guest's EXIT line BEFORE calling `terminate`, which let
    // the watcher's FIRST `select!` poll win on the guest-line arm
    // outright and never exercised the retry loop at all.
    let control =
        VmControl { pid: handle.pid.expect("VmDriver populates pid"), api_socket: PathBuf::new() };
    sim.terminate(&control, Duration::ZERO).await.expect("terminate the VMM process");

    // THEN the guest reports its supervised workload's exit status,
    // deliberately delayed so the watcher's retry loop has already
    // started polling before this line becomes readable.
    yield_for_task_poll().await;
    stream.write_all(b"EXIT 7\n").await.expect("write EXIT report");

    let event = exit_rx.recv().await.expect("exit watcher emits exactly one ExitEvent");
    assert_eq!(event.alloc, alloc);
    assert_eq!(
        event.kind,
        ExitKind::Crashed { exit_code: Some(7), signal: None },
        "ExitKind must reflect the guest's EXIT report, never the VMM's own exit status"
    );
}

/// ADR-0082 §D7 amendment (GH #42, item 2 / 01-07 review remediation
/// FIX 2) — once the beacon accepts and reads `READY`, `start` writes
/// the operator's command to the guest as one `EXEC <json-argv>` line
/// BEFORE returning `Ok`. `argv` is `[spec.command, ...spec.args]` —
/// the kernel cmdline never carries it (`KernelCmdline` stays
/// platform-only, ADR-0082 §D2).
#[tokio::test]
async fn start_writes_exec_message_with_spec_command_and_args_before_returning_ok() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let sim = SimVmm::new();
    let (driver, _clock) = build_driver(std::sync::Arc::new(sim), layout);

    let alloc = AllocationId::new("alloc-exec-write").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    let (_handle, stream) = start_with_beacon_accepted(&driver, &spec, &run_dir_root).await;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    // Bounded real-wall-clock timeout, not a `SimClock` wait — this
    // test has no reason for the driver to ever hang on the EXEC
    // write, so a bug here should fail fast and cleanly rather than
    // riding nextest's own 120s slow-test kill.
    tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
        .await
        .expect("read the first host -> guest line within 5s")
        .expect("read the first host -> guest line");

    let message: BeaconMessage =
        line.trim_end().parse().expect("EXEC line parses as a BeaconMessage");
    assert_eq!(
        message,
        BeaconMessage::Exec { argv: vec!["/sbin/init".to_owned()] },
        "the first host -> guest line after READY must be EXEC with spec.command/args as argv"
    );
}

// ---------------------------------------------------------------------
// AC #3 — S-VM-76: `VmDriver::stop` is total over all four sequences.
// ---------------------------------------------------------------------

/// Test-only `Vmm` decorator: signals a oneshot the instant
/// `Vmm::create` resolves. `VmDriver::start` performs its
/// `Starting -> Live` supervision transition synchronously, with no
/// intervening `.await`, immediately after `Vmm::create` resolves — so
/// awaiting this signal is a deterministic point at which the
/// allocation is guaranteed to be `Live { beacon: None }`.
///
/// `live_allocations()` cannot provide this signal itself: per brief
/// §105a.3 it deliberately reports BOTH `Starting` and `Live` as
/// "Held", so polling it cannot distinguish the two Held sub-phases
/// sequence (a) needs to tell apart, since ADR-0082 §D4 scopes the
/// pre-beacon window precisely to "between `Vmm::create` and
/// `accept_ready`", i.e. Live-with-no-beacon, not Starting.
#[derive(Clone)]
struct SignalsOnceLive {
    inner: SimVmm,
    signal: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

#[async_trait]
impl Vmm for SignalsOnceLive {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        let process = self.inner.create(config).await?;
        let signal = self.signal.lock().expect("signal mutex not poisoned").take();
        if let Some(tx) = signal {
            let _ = tx.send(());
        }
        Ok(process)
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

/// S-VM-76 sequence (a) — stop arrives BEFORE the guest has beaconed
/// (between `Vmm::create` and `accept_ready` — `LiveVm.beacon` is
/// `None`). `stop` must skip the beacon write entirely and go straight
/// to `Vmm::terminate`; the allocation reaches a terminal disposition,
/// never a crash.
///
/// 01-07 RE-REVIEW remediation (HIGH) — this SAME interleaving is also
/// the ONLY reachable case that races `stop`'s transition 3b
/// (`Live -> EndingInFlight`, taken synchronously under the lock before
/// `Vmm::terminate` is even called) against `start`'s OWN unwind
/// cleanup: `stop`'s `Vmm::terminate` call is exactly what resolves the
/// in-flight `start`'s `exit.recv()` race arm, which then runs
/// `cleanup_after_start_failure` -> `release_claim` on the SAME entry
/// `stop` just moved to `EndingInFlight`. Pre-fix, `release_claim` was
/// an UNCONDITIONAL remove, so it silently clobbered `stop`'s hand-off
/// and stripped the allocation out of `live_allocations()` entirely — a
/// full-remove shape brief §105a.11's `EndingInFlightIsNeverReclaimed`
/// forbids (the entry must stay reported as claimed while its ending is
/// in flight, or a reclamation-shaped consumer treats an abandoned
/// entry as fair game). The retention assertion below is this fix's
/// regression test.
#[tokio::test]
async fn stop_sequence_a_pre_beacon_stop_skips_write_and_terminates() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let sim = SimVmm::new();
    let (live_tx, live_rx) = tokio::sync::oneshot::channel();
    let vmm = SignalsOnceLive {
        inner: sim,
        signal: std::sync::Arc::new(std::sync::Mutex::new(Some(live_tx))),
    };
    let (driver, _clock) = build_driver(std::sync::Arc::new(vmm), layout);

    let alloc = AllocationId::new("alloc-stop-a").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    // Nothing ever dials the beacon or ticks the clock — start() stays
    // parked in the three-way race for the whole test, landing on
    // Live { beacon: None } shortly after Vmm::create resolves.
    // `live_rx` is the readiness gate (see `SignalsOnceLive` above) —
    // polling `live_allocations()` cannot distinguish "claimed"
    // (Starting) from "Live { beacon: None }", exactly the two states
    // this sequence needs to tell apart.
    let driver_for_task = driver.clone();
    let spec_owned = spec.clone();
    let start_task = tokio::spawn(async move { driver_for_task.start(&spec_owned).await });
    live_rx.await.expect("Vmm::create resolves before start() can ever return");

    // Callers may construct a handle with `pid: None` (mirrors
    // ExecDriver's documented contract) — `stop` uses its own live
    // map, never `handle.pid`.
    let handle = AllocationHandle { alloc: alloc.clone(), pid: None };
    let stop_result = driver.stop(&handle).await;
    assert!(stop_result.is_ok(), "pre-beacon stop must return Ok, never a crash: {stop_result:?}");

    // The terminate() call inside stop() resolves the in-flight
    // start()'s exit.recv() arm — it must reject, never panic.
    let start_result = start_task.await.expect("start task did not panic");
    assert!(start_result.is_err(), "the in-flight start must reject once stop terminates the VMM");

    // 01-07 RE-REVIEW remediation (HIGH) — `stop`'s own transition 3b
    // set this entry to `EndingInFlight` BEFORE `Vmm::terminate` ever
    // ran, so it strictly happens-before `start`'s unwind cleanup
    // (which only begins once `exit.recv()` resolves, i.e. once
    // `terminate` has already fired). `start`'s `release_claim` must
    // see `EndingInFlight` here and leave it alone — `stop` owns this
    // allocation's ending, not `start`'s failure path. A full removal
    // (the pre-fix unconditional `release_claim`) would report the
    // allocation as unclaimed, reopening the second-authorship hazard
    // `EndingInFlightIsNeverReclaimed` (brief §105a.11) exists to
    // forbid.
    assert_eq!(
        driver.live_allocations(),
        Some(vec![alloc.clone()]),
        "the allocation must remain claimed (EndingInFlight) after stop's transition 3b \
         races start's own unwind cleanup -- release_claim must not clobber a \
         concurrently-set EndingInFlight entry"
    );
}

/// S-VM-76 sequence (b) — stop arrives after the guest has beaconed,
/// but the guest never reads the `SHUTDOWN` byte (an unresponsive
/// guest). `stop` escalates to `Vmm::terminate` once
/// `VM_SHUTDOWN_REQUEST_DEADLINE` (2 s) elapses on the unread write —
/// driven via `SimClock::tick`, never a real 2 s wait.
#[tokio::test]
async fn stop_sequence_b_unresponsive_guest_escalates_after_deadline() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let sim = SimVmm::new();
    let (driver, clock) = build_driver(std::sync::Arc::new(sim.clone()), layout);

    let alloc = AllocationId::new("alloc-stop-b").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    let (handle, _unresponsive_stream) =
        start_with_beacon_accepted(&driver, &spec, &run_dir_root).await;
    // `_unresponsive_stream` is held alive but never read from — the
    // guest is connected and simply does not consume the SHUTDOWN byte.

    let driver_for_task = driver.clone();
    let handle_owned = handle.clone();
    let stop_task = tokio::spawn(async move { driver_for_task.stop(&handle_owned).await });

    yield_for_task_poll().await;
    clock.tick(Duration::from_secs(2));

    let stop_result = stop_task.await.expect("stop task did not panic");
    assert!(
        stop_result.is_ok(),
        "stop against an unresponsive guest must still return Ok after escalating: {stop_result:?}"
    );
    assert!(
        !sim.is_live(handle.pid.expect("pid populated")),
        "Vmm::terminate must have force-killed the unresponsive guest's VMM"
    );
}

/// S-VM-76 sequence (c) — stop arrives after the VMM process is
/// already dead. `Vmm::terminate` observes an already-gone process and
/// returns `Ok(VmTermination::Killed)` — `stop` must return `Ok`
/// without erroring (idempotent terminate).
#[tokio::test]
async fn stop_sequence_c_already_dead_vmm_returns_ok() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let sim = SimVmm::new();
    let (driver, clock) = build_driver(std::sync::Arc::new(sim.clone()), layout);

    let alloc = AllocationId::new("alloc-stop-c").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    let (handle, _stream) = start_with_beacon_accepted(&driver, &spec, &run_dir_root).await;

    // The VMM dies on its own BEFORE the operator's stop arrives.
    let control = VmControl { pid: handle.pid.expect("pid populated"), api_socket: PathBuf::new() };
    sim.terminate(&control, Duration::ZERO).await.expect("pre-kill the VMM");
    assert!(!sim.is_live(control.pid), "precondition: the VMM is already dead");

    let driver_for_task = driver.clone();
    let handle_owned = handle.clone();
    let stop_task = tokio::spawn(async move { driver_for_task.stop(&handle_owned).await });

    // The beacon session is still `Some` (the guest connection itself
    // was never closed by this test), so `stop` still writes SHUTDOWN
    // and waits out the request deadline before calling `terminate`
    // again on the already-dead process.
    yield_for_task_poll().await;
    clock.tick(Duration::from_secs(2));

    let stop_result = stop_task.await.expect("stop task did not panic");
    assert!(
        stop_result.is_ok(),
        "stop against an already-dead VMM must return Ok without erroring: {stop_result:?}"
    );
}

/// S-VM-76 sequence (d) — stop is called twice in succession for the
/// same allocation. Neither call panics, and the allocation reaches a
/// terminal disposition — AC #3's "never a crash", not "the second
/// call is always `Ok`".
///
/// The FIRST call moves the entry `Live -> EndingInFlight`
/// synchronously, under the SAME lock as its own extraction (brief
/// §105a.3 transition 3b, 01-07 review remediation FIX 1) — no longer
/// racing the exit watcher's independent `try_begin_ending` (transition
/// 3) to decide who wins. The second call therefore deterministically
/// finds no `Live` entry and returns `Err(NotFound)` — the SAME
/// idempotent-double-stop outcome `ExecDriver::stop` returns
/// unconditionally (it removes its live entry on the FIRST call:
/// `driver.rs:590`). The assertion still accepts `Ok(())` as well —
/// AC #3's "never a crash" contract, not a claim about which arm fires
/// — but only the `Err(NotFound)` arm is reachable post-fix.
#[tokio::test]
async fn stop_sequence_d_double_stop_is_idempotent_and_never_panics() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let sim = SimVmm::new();
    let (driver, clock) = build_driver(std::sync::Arc::new(sim.clone()), layout);

    let alloc = AllocationId::new("alloc-stop-d").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    let (handle, _stream) = start_with_beacon_accepted(&driver, &spec, &run_dir_root).await;

    let driver_for_first = driver.clone();
    let handle_for_first = handle.clone();
    let first_stop = tokio::spawn(async move { driver_for_first.stop(&handle_for_first).await });
    yield_for_task_poll().await;
    clock.tick(Duration::from_secs(2));
    let first_result = first_stop.await.expect("first stop task did not panic");
    assert!(first_result.is_ok(), "first stop must return Ok: {first_result:?}");

    // Second call — deterministically Err(NotFound) post-FIX-1 (see the
    // test's doc comment above); the union match still stands as the
    // honest AC #3 contract ("never a crash"), not a claim about which
    // arm fires.
    let second_result = driver.stop(&handle).await;
    assert!(
        matches!(
            second_result,
            Ok(()) | Err(overdrive_core::traits::driver::DriverError::NotFound { .. })
        ),
        "second stop call must reach a terminal disposition, never a crash: {second_result:?}"
    );

    let run_dir = VmRunDir::for_alloc(&run_dir_root, &alloc);
    assert!(
        !run_dir.path().exists(),
        "run directory must be removed after stop, still present at {}",
        run_dir.path().display()
    );
}

/// Test-only `Vmm` decorator: hands back a [`VmExitWatch`] that NEVER
/// resolves (backed by a fresh oneshot pair whose sender is
/// deliberately leaked). Structurally rules out the SPAWNED EXIT
/// WATCHER's own independent `try_begin_ending` transition as an
/// alternative explanation for `status()` reporting `NotFound` after
/// `stop()` — without this isolation, `stop_ok_then_status_reports_not_found`
/// below observably PASSED even against the pre-fix `stop` (01-07
/// review remediation FIX 1 / BLOCKER D1), because the watcher's own
/// exit-driven transition won the race often enough to produce the
/// SAME correct-looking outcome by coincidence. A dropped sender would
/// ALSO resolve `recv()` (to `None`) — itself an observable "exited"
/// signal — so the sender must stay alive, never dropped and never
/// sent, for the rest of the process.
#[derive(Clone)]
struct NeverSignalsExit {
    inner: SimVmm,
}

#[async_trait]
impl Vmm for NeverSignalsExit {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        let process = self.inner.create(config).await?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        // Deliberate permanent leak — see the struct doc comment.
        std::mem::forget(tx);
        // Carry the inner adapter's own diagnostics reader through: this
        // decorator replaces only the exit watch, never the capture.
        Ok(VmProcess {
            control: process.control,
            exit: VmExitWatch::new(rx),
            diagnostics: process.diagnostics,
        })
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

/// The `Driver::stop` post-condition (driver.rs, binding): after
/// `stop()` returns `Ok(())`, a subsequent `status()` against the SAME
/// handle returns `Err(DriverError::NotFound)`. 01-07 review
/// remediation FIX 1 / BLOCKER D1 regression test — `stop` moves the
/// entry `Live -> EndingInFlight` synchronously (transition 3b) so
/// `status`'s existing `EndingInFlight -> NotFound` mapping applies
/// immediately. Uses [`NeverSignalsExit`] so the spawned exit watcher
/// can never independently race to the SAME observable outcome — the
/// ONLY path that can move this allocation out of `Live` here is
/// `stop`'s own synchronous transition.
#[tokio::test]
async fn stop_ok_then_status_reports_not_found() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let sim = SimVmm::new();
    let never_signals_exit = NeverSignalsExit { inner: sim };
    let (driver, clock) = build_driver(std::sync::Arc::new(never_signals_exit), layout);

    let alloc = AllocationId::new("alloc-stop-then-status").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    let (handle, _stream) = start_with_beacon_accepted(&driver, &spec, &run_dir_root).await;

    let driver_for_task = driver.clone();
    let handle_owned = handle.clone();
    let stop_task = tokio::spawn(async move { driver_for_task.stop(&handle_owned).await });
    yield_for_task_poll().await;
    clock.tick(Duration::from_secs(2));
    let stop_result = stop_task.await.expect("stop task did not panic");
    assert!(stop_result.is_ok(), "stop must return Ok: {stop_result:?}");

    let status_result = driver.status(&handle).await;
    assert!(
        matches!(status_result, Err(overdrive_core::traits::driver::DriverError::NotFound { .. })),
        "status after a successful stop must report NotFound per the Driver::stop \
         post-condition (driver.rs), got {status_result:?}"
    );
}

// ---------------------------------------------------------------------
// AC #4 — the two defaulted Driver methods, VmDriver's overrides.
// ---------------------------------------------------------------------

/// `live_allocations` reports an allocation across the boot-race window
/// and while running (both `Starting`/`Live` "Held" phases collapse to
/// membership in the reported set); `release_supervision` is idempotent
/// — a second call, or a call against an unknown id, is a no-op.
#[tokio::test]
async fn live_allocations_reports_membership_and_release_supervision_is_idempotent() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let sim = SimVmm::new();
    let (driver, _clock) = build_driver(std::sync::Arc::new(sim), layout);

    let alloc = AllocationId::new("alloc-live-report").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    assert_eq!(driver.live_allocations(), Some(Vec::new()), "nothing live before start");

    let (handle, _stream) = start_with_beacon_accepted(&driver, &spec, &run_dir_root).await;
    assert_eq!(
        driver.live_allocations(),
        Some(vec![alloc.clone()]),
        "the running allocation is reported while its supervision handle is Live"
    );

    // Unknown id: no-op, no panic.
    let unknown = AllocationId::new("alloc-never-started").expect("valid alloc id");
    driver.release_supervision(&unknown);
    assert_eq!(
        driver.live_allocations(),
        Some(vec![alloc.clone()]),
        "release_supervision against an unknown id must not disturb other entries"
    );

    driver.release_supervision(&alloc);
    assert_eq!(
        driver.live_allocations(),
        Some(Vec::new()),
        "release_supervision removes the entry"
    );

    // Idempotent — a second release against the same, now-absent id is
    // still a no-op.
    driver.release_supervision(&alloc);
    assert_eq!(driver.live_allocations(), Some(Vec::new()));

    // Cleanup — stop the underlying process/artifacts so the tempdir
    // teardown does not race a still-live SimVmm process (harmless
    // either way, but keeps the fixture tidy).
    let _ = driver.stop(&handle).await;
}
