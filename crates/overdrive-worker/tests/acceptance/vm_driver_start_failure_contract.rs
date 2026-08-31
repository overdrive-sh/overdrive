//! 03-05 component-scope acceptance for `VmDriver`'s typed start-failure
//! join (DWD-24).
//!
//! `@contract-shape:bounded-change` — every scenario enters through the
//! public `Driver::start` port against a `Vmm` double and observes only
//! the returned cause/detail pair plus the allocation-scoped cleanup
//! surface. Nothing here reads driver internals.
//!
//! The `Vmm` doubles below are built entirely from the port's own public
//! surface (`VmProcess` / `VmExitWatch::new` / `VmmDiagnostics::new`), so
//! no production API exists for testing's sake.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::os::unix::fs::FileTypeExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::SpiffeId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::{AllocationId, NetnsName};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, Driver, DriverError, DriverPayload, DriverStartClass,
    DriverType, Resources, VmPayload, VmStartFailure,
};
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmExitWatch, VmProcess, VmTermination, Vmm, VmmDiagnostics,
    VmmError, VmmExit, VmmProbeError,
};
use overdrive_core::vm::beacon::BEACON_VSOCK_PORT;
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, RootfsPlan, VmConfig, VmConfinement, VmRunDir, VmmIdentity,
};
use overdrive_sim::adapters::cgroup_fs::SimOp;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_worker::VmDriver;
use overdrive_worker::vm_driver::VmHostLayout;
use tempfile::TempDir;
use tokio::io::AsyncWriteExt as _;
use tokio::net::UnixStream;
use tokio::sync::{Mutex, Semaphore, oneshot};

// ---------------------------------------------------------------------
// Fixtures — mirror `vm_driver_stop_totality.rs`'s established shapes.
// ---------------------------------------------------------------------

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

fn build_layout(tmp: &TempDir) -> VmHostLayout {
    stage_fixture_artifacts(tmp);
    VmHostLayout {
        cgroup_root: tmp.path().join("cgroup"),
        run_dir_root: tmp.path().join("run"),
        clone_index_dir: tmp.path().join("clone-index"),
        clone_staging_dir: tmp.path().join("clone-staging"),
        arch: HostArch::X86_64,
        confinement: VmConfinement::confined(
            VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
            1024,
        ),
    }
}

fn build_spec(alloc: &AllocationId, tmp: &TempDir) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/workload/vm-fail/alloc/x")
            .expect("valid spiffe id"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/sbin/init".to_owned(),
            args: vec![],
            kernel: fixture_kernel_path(tmp),
            rootfs: fixture_rootfs_path(tmp),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: Some(NetnsName::from_hex4("0001").expect("valid fixture netns")),
        host_veth: Some("ovd-hv-0001".to_owned()),
        service_ports: Vec::new(),
        workload_addr: Some("100.96.0.6".parse().expect("valid fixture guest address")),
        guest_tap: Some("ovd-tap-0001".to_owned()),
        guest_mac: Some([0x02, 0, 0, 0, 0, 1]),
        guest_gateway: Some("100.96.0.5".parse().expect("valid fixture gateway")),
        guest_prefix_len: Some(30),
        guest_dns: Some("100.96.0.5".parse().expect("valid fixture DNS")),
    }
}

fn build_driver(vmm: Arc<dyn Vmm>, layout: VmHostLayout) -> (VmDriver, SimClock, SimCgroupFs) {
    let clock = SimClock::new();
    let cgroup_fs = SimCgroupFs::new();
    let fs: Arc<dyn overdrive_core::traits::CgroupFs> = Arc::new(cgroup_fs.clone());
    let accounting: Arc<dyn overdrive_core::traits::cgroup_accounting::CgroupAccounting> =
        Arc::new(SimCgroupAccounting::new());
    let driver = VmDriver::new(vmm, Arc::new(clock.clone()), fs, accounting, layout);
    (driver, clock, cgroup_fs)
}

async fn connect_beacon(run_dir_root: &Path, alloc: &AllocationId) -> UnixStream {
    let socket = VmRunDir::for_alloc(run_dir_root, alloc).beacon_socket(BEACON_VSOCK_PORT);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match UnixStream::connect(&socket).await {
            Ok(stream) => return stream,
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            Err(error) => panic!(
                "beacon listener did not become connectable at {}: {error}",
                socket.display()
            ),
        }
    }
}

async fn start_ready(
    driver: &VmDriver,
    spec: &AllocationSpec,
    run_dir_root: &Path,
) -> (AllocationHandle, UnixStream) {
    let task_driver = driver.clone();
    let spec = spec.clone();
    let alloc = spec.alloc.clone();
    let task = tokio::spawn(async move { task_driver.start(&spec).await });
    let mut beacon = connect_beacon(run_dir_root, &alloc).await;
    beacon
        .write_all(b"READY pid=1 port=1234\n")
        .await
        .expect("send READY through the production beacon parser");
    let handle = task.await.expect("start task did not panic").expect("READY completes the start");
    driver.release_for_exit_emission(&handle).await;
    (handle, beacon)
}

async fn stop_ready(driver: &VmDriver, handle: &AllocationHandle, clock: &SimClock) {
    let driver = driver.clone();
    let handle = handle.clone();
    let task = tokio::spawn(async move { driver.stop(&handle).await });
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    clock.tick(Duration::from_secs(2));
    task.await.expect("stop task did not panic").expect("stop the live VM owner");
}

/// Extract the typed failure from a rejected start.
fn expect_rejection(err: DriverError) -> overdrive_core::traits::driver::DriverStartFailure {
    match err {
        DriverError::StartRejected { failure } => failure,
        other => panic!("expected StartRejected, got {other:?}"),
    }
}

fn expect_vm_class(err: DriverError) -> (VmStartFailure, String) {
    let failure = expect_rejection(err);
    let detail = failure.detail.clone();
    match failure.class {
        DriverStartClass::Vm(vm) => (vm, detail),
        other => panic!("expected a VM class, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// `Vmm` doubles, built from the port's own public surface.
// ---------------------------------------------------------------------

/// Fails `create` with a caller-chosen typed `VmmError`, so the driver's
/// STRUCTURAL join can be observed independently of any prose.
struct FailsCreate {
    make: Box<dyn Fn() -> VmmError + Send + Sync>,
}

struct CreateBarrierVmm {
    inner: SimVmm,
    target: AllocationId,
    entered: Mutex<Option<oneshot::Sender<()>>>,
    release: Semaphore,
    create_calls: AtomicUsize,
}

#[async_trait]
impl Vmm for CreateBarrierVmm {
    fn kind(&self) -> &'static str {
        "create-barrier-sim"
    }

    async fn probe(&self) -> std::result::Result<(), VmmProbeError> {
        Ok(())
    }

    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        self.create_calls.fetch_add(1, Ordering::SeqCst);
        if config.alloc == self.target {
            let entered = self.entered.lock().await.take();
            if let Some(entered) = entered {
                let _ = entered.send(());
            }
            self.release.acquire().await.expect("barrier stays open").forget();
        }
        self.inner.create(config).await
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

fn artifact_tree(root: &Path) -> Vec<(PathBuf, String, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<(PathBuf, String, Vec<u8>)>) {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return;
        };
        let relative = path.strip_prefix(root).unwrap_or(path).to_path_buf();
        if metadata.file_type().is_symlink() {
            out.push((
                relative,
                "symlink".to_owned(),
                std::fs::read_link(path)
                    .expect("read fixture symlink")
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec(),
            ));
        } else if metadata.is_dir() {
            out.push((relative, "dir".to_owned(), Vec::new()));
            let mut children = std::fs::read_dir(path)
                .expect("read fixture directory")
                .map(|entry| entry.expect("read fixture entry").path())
                .collect::<Vec<_>>();
            children.sort();
            for child in children {
                visit(root, &child, out);
            }
        } else if metadata.file_type().is_socket() {
            out.push((relative, "socket".to_owned(), Vec::new()));
        } else {
            out.push((
                relative,
                "file".to_owned(),
                std::fs::read(path).expect("read fixture file"),
            ));
        }
    }

    let mut out = Vec::new();
    visit(root, root, &mut out);
    out
}

#[async_trait]
impl Vmm for FailsCreate {
    fn kind(&self) -> &'static str {
        "fails-create"
    }
    async fn probe(&self) -> std::result::Result<(), VmmProbeError> {
        Ok(())
    }
    async fn create(&self, _config: &VmConfig) -> VmmResult<VmProcess> {
        Err((self.make)())
    }
    async fn terminate(&self, _c: &VmControl, _g: Duration) -> VmmResult<VmTermination> {
        Ok(VmTermination::Killed)
    }
}

/// Reports a scripted hypervisor ending immediately, before any guest
/// could beacon — the §D3 `exit.recv()` arm.
struct ExitsBeforeBeacon {
    exit: VmmExit,
}

#[async_trait]
impl Vmm for ExitsBeforeBeacon {
    fn kind(&self) -> &'static str {
        "exits-before-beacon"
    }
    async fn probe(&self) -> std::result::Result<(), VmmProbeError> {
        Ok(())
    }
    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        let (diagnostics, writer) = VmmDiagnostics::new();
        if let Some(tail) = self.exit.stderr_tail.as_deref() {
            writer.append(tail.as_bytes());
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = tx.send(self.exit.clone());
        Ok(VmProcess {
            control: VmControl { pid: 424_242, api_socket: config.run_dir.api_socket() },
            exit: VmExitWatch::new(rx),
            diagnostics,
        })
    }
    async fn terminate(&self, _c: &VmControl, _g: Duration) -> VmmResult<VmTermination> {
        Ok(VmTermination::Killed)
    }
}

/// Never beacons and never exits, but DOES emit console output — the
/// §D3 deadline arm, which can only read a LIVE capture.
struct SilentButNoisy {
    console: &'static str,
}

#[async_trait]
impl Vmm for SilentButNoisy {
    fn kind(&self) -> &'static str {
        "silent-but-noisy"
    }
    async fn probe(&self) -> std::result::Result<(), VmmProbeError> {
        Ok(())
    }
    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        let (diagnostics, writer) = VmmDiagnostics::new();
        writer.append(self.console.as_bytes());
        // Deliberately leak the sender: a dropped sender resolves
        // `recv()` to `None`, which is itself an "exited" signal.
        let (tx, rx) = tokio::sync::oneshot::channel::<VmmExit>();
        std::mem::forget(tx);
        Ok(VmProcess {
            control: VmControl { pid: 424_243, api_socket: config.run_dir.api_socket() },
            exit: VmExitWatch::new(rx),
            diagnostics,
        })
    }
    async fn terminate(&self, _c: &VmControl, _g: Duration) -> VmmResult<VmTermination> {
        Ok(VmTermination::Killed)
    }
}

// ---------------------------------------------------------------------
// Scenarios.
// ---------------------------------------------------------------------

/// Changing only a `VmmError`'s rendered prose cannot change the VM start
/// class selected from its structured facts.
#[tokio::test]
async fn vm_start_failure_class_is_independent_of_vmm_diagnostic_wording() {
    // Same structured absence, three unrelated diagnostics.
    let prose = ["cloud-hypervisor is gone", "binaire introuvable", "spawn ...: whatever"];
    let mut observed = Vec::new();

    for text in prose {
        let tmp = TempDir::new().expect("tempdir");
        let layout = build_layout(&tmp);
        let vmm = FailsCreate {
            make: Box::new(move || VmmError::HypervisorAbsent {
                searched: vec!["/usr/bin/cloud-hypervisor".to_owned()],
                source: std::io::Error::new(std::io::ErrorKind::NotFound, text),
            }),
        };
        let (driver, _clock, _fs) = build_driver(Arc::new(vmm), layout);
        let alloc = AllocationId::new("alloc-wording").expect("valid alloc id");

        let err =
            driver.start(&build_spec(&alloc, &tmp)).await.expect_err("create failure rejects");
        let (class, detail) = expect_vm_class(err);
        observed.push((class, detail));
    }

    for (class, _) in &observed {
        assert_eq!(
            class,
            &VmStartFailure::HypervisorAbsent {
                searched: vec!["/usr/bin/cloud-hypervisor".to_owned()],
            },
            "the class must come from the VmmError VARIANT, never its prose",
        );
    }
    // The diagnostics really did differ — the invariance above is not
    // vacuous.
    assert_ne!(observed[0].1, observed[1].1, "the fixture must vary the diagnostic");
}

/// The low-level diagnostic crosses `VmDriver` unchanged and separately
/// from the structured VM cause.
#[tokio::test]
async fn vm_start_failure_preserves_verbatim_vmm_diagnostic_detail() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let vmm = FailsCreate {
        make: Box::new(|| VmmError::RootfsNotFound {
            path: std::path::PathBuf::from("/srv/vm/root.ext4"),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No such file or directory (os error 2)",
            ),
        }),
    };
    let (driver, _clock, _fs) = build_driver(Arc::new(vmm), layout);
    let alloc = AllocationId::new("alloc-verbatim").expect("valid alloc id");

    let err = driver.start(&build_spec(&alloc, &tmp)).await.expect_err("create failure rejects");
    let (class, detail) = expect_vm_class(err);

    assert_eq!(
        class,
        VmStartFailure::RootfsNotFound { path: "/srv/vm/root.ext4".to_owned() },
        "the structured cause names the exact configured path",
    );
    assert_eq!(
        detail, "No such file or directory (os error 2)",
        "the adapter's own diagnostic must cross the driver unchanged",
    );
}

/// When the VMM exits before READY, the start rejection retains the
/// process exit code, terminating signal, and final stderr tail.
#[tokio::test]
async fn vm_early_exit_failure_preserves_exit_code_signal_and_final_stderr_tail() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let vmm = ExitsBeforeBeacon {
        exit: VmmExit {
            exit_code: Some(17),
            signal: Some(6),
            stderr_tail: Some("cloud-hypervisor: fatal: bad disk\nabort".to_owned()),
        },
    };
    let (driver, _clock, _fs) = build_driver(Arc::new(vmm), layout);
    let alloc = AllocationId::new("alloc-early-exit").expect("valid alloc id");

    let err =
        driver.start(&build_spec(&alloc, &tmp)).await.expect_err("early VMM exit rejects start");
    let (class, detail) = expect_vm_class(err);

    assert_eq!(
        class,
        VmStartFailure::GuestExitUnreported { vmm_exit_code: Some(17), vmm_signal: Some(6) },
        "the resolved VmmExit's code and signal must survive as structured facts",
    );
    assert_eq!(
        detail, "cloud-hypervisor: fatal: bad disk\nabort",
        "the hypervisor's final stderr tail must be the preserved diagnostic",
    );
}

/// When the boot deadline wins, the rejection retains the configured
/// deadline in milliseconds and the LIVE console tail — the only source
/// available, since no `VmmExit` exists on this arm.
#[tokio::test]
async fn vm_boot_deadline_failure_preserves_deadline_milliseconds_and_live_console_tail() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let vmm =
        SilentButNoisy { console: "[    0.78] Run /sbin/init as init process\npanic: no init" };
    let (driver, clock, _fs) = build_driver(Arc::new(vmm), layout);
    let alloc = AllocationId::new("alloc-deadline-tail").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    let driver_task = driver.clone();
    let spec_owned = spec.clone();
    let start = tokio::spawn(async move { driver_task.start(&spec_owned).await });

    // Advance logical time until the deadline arm resolves. Mirrors the
    // established pattern in `vm_driver_stop_totality.rs`.
    for _ in 0..200 {
        if start.is_finished() {
            break;
        }
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
        clock.tick(Duration::from_secs(30));
    }

    let err = start
        .await
        .expect("start task did not panic")
        .expect_err("boot deadline rejects start when nothing beacons");
    let (class, detail) = expect_vm_class(err);

    match class {
        VmStartFailure::BootDeadlineExceeded { deadline_ms, console_tail } => {
            assert_eq!(deadline_ms, 30_000, "the real configured deadline, in milliseconds");
            assert_eq!(
                console_tail.as_deref(),
                Some("[    0.78] Run /sbin/init as init process\npanic: no init"),
                "the LIVE console tail must be captured, not reconstructed from timeout text",
            );
        }
        other => panic!("expected BootDeadlineExceeded, got {other:?}"),
    }
    assert!(
        !detail.contains("elapsed with no beacon"),
        "the captured console output is the diagnostic, not the timeout sentence: {detail}",
    );

    // The deadline arm still cleans up (§D3's most leak-prone arm).
    let run_dir = VmRunDir::for_alloc(&run_dir_root, &alloc);
    assert!(!run_dir.path().exists(), "the deadline arm must remove the run directory");
}

/// The same rejected start driven first as an initial start and then as a
/// restart exposes byte-identical cause/detail pairs.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn initial_and_restart_start_expose_identical_cause_and_detail_pairs() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    // Delete the configured rootfs so BOTH attempts fail identically.
    std::fs::remove_file(fixture_rootfs_path(&tmp)).expect("remove the configured rootfs master");
    let configured = fixture_rootfs_path(&tmp).display().to_string();
    let (driver, _clock, _fs) = build_driver(Arc::new(SimVmm::new()), layout);
    let alloc = AllocationId::new("alloc-restart-parity").expect("valid alloc id");
    let spec = build_spec(&alloc, &tmp);

    let first = expect_vm_class(driver.start(&spec).await.expect_err("initial start rejects"));
    driver.release_supervision(&alloc);
    let second = expect_vm_class(driver.start(&spec).await.expect_err("restart start rejects"));
    driver.release_supervision(&alloc);

    assert_eq!(
        first.0,
        VmStartFailure::RootfsNotFound { path: configured },
        "the initial start names the exact configured rootfs path",
    );
    assert_eq!(first, second, "initial and restart starts must persist identical cause/detail");
}

/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn duplicate_start_request_is_rejected_without_replacement_cross_ownership_or_leak() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let clone_staging = layout.clone_staging_dir.clone();
    let clone_index = layout.clone_index_dir.clone();
    let sim = SimVmm::new();
    let target = AllocationId::new("alloc-duplicate-target").expect("valid target id");
    let sibling = AllocationId::new("alloc-duplicate-sibling").expect("valid sibling id");
    let (entered_tx, entered_rx) = oneshot::channel();
    let barrier_vmm = Arc::new(CreateBarrierVmm {
        inner: sim.clone(),
        target: target.clone(),
        entered: Mutex::new(Some(entered_tx)),
        release: Semaphore::new(0),
        create_calls: AtomicUsize::new(0),
    });
    let (driver, clock, cgroup_fs) = build_driver(barrier_vmm.clone(), layout);
    let target_spec = build_spec(&target, &tmp);
    let sibling_spec = build_spec(&sibling, &tmp);

    let (sibling_handle, _sibling_beacon) =
        start_ready(&driver, &sibling_spec, &run_dir_root).await;
    let sibling_pid = sibling_handle.pid.expect("sibling start returns a VMM pid");
    let target_task_driver = driver.clone();
    let target_task_spec = target_spec.clone();
    let target_task =
        tokio::spawn(async move { target_task_driver.start(&target_task_spec).await });
    entered_rx.await.expect("target start reaches the deterministic create barrier");

    let starting_claims = driver.live_allocations().expect("VM driver reports claims");
    let starting_cgroups = cgroup_fs.snapshot();
    let starting_run_tree = artifact_tree(&run_dir_root);
    let starting_clone_tree = artifact_tree(&clone_staging);
    let starting_index_tree = artifact_tree(&clone_index);
    assert_eq!(barrier_vmm.create_calls.load(Ordering::SeqCst), 2);
    assert!(sim.is_live(sibling_pid), "the sibling is the sole created process at the barrier");
    assert!(
        !sim.is_live(sibling_pid + 1),
        "the target process is not created before the barrier releases"
    );

    let duplicate = driver
        .start(&target_spec)
        .await
        .expect_err("a second start while the first is Starting is rejected");
    let rejection = expect_rejection(duplicate);
    assert!(
        matches!(
            rejection.class,
            DriverStartClass::Vm(VmStartFailure::AllocationAlreadyOwned { ref alloc })
                if alloc == &target
        ),
        "duplicate ownership must carry the allocation identity structurally: {:?}",
        rejection.class
    );
    assert!(
        rejection.detail.contains("already has an active VM"),
        "the stable detail identifies the ownership conflict: {}",
        rejection.detail
    );
    assert_eq!(
        driver.live_allocations().expect("VM driver reports claims"),
        starting_claims,
        "the Starting duplicate cannot replace either claim"
    );
    assert_eq!(cgroup_fs.snapshot(), starting_cgroups, "Starting cgroups are byte-exact");
    assert_eq!(artifact_tree(&run_dir_root), starting_run_tree, "Starting run tree is exact");
    assert_eq!(artifact_tree(&clone_staging), starting_clone_tree, "Starting clone tree is exact");
    assert_eq!(artifact_tree(&clone_index), starting_index_tree, "Starting index tree is exact");
    assert_eq!(barrier_vmm.create_calls.load(Ordering::SeqCst), 2, "no replacement create");
    assert!(sim.is_live(sibling_pid), "the independent sibling remains live");

    let mut target_beacon = connect_beacon(&run_dir_root, &target).await;
    target_beacon.write_all(b"READY pid=1 port=1234\n").await.expect("queue target READY");
    barrier_vmm.release.add_permits(1);
    let target_handle = target_task
        .await
        .expect("target start task did not panic")
        .expect("original target start completes");
    driver.release_for_exit_emission(&target_handle).await;
    let target_pid = target_handle.pid.expect("VM start returns a VMM pid");

    let live_claims = driver.live_allocations().expect("VM driver reports claims");
    let live_cgroups = cgroup_fs.snapshot();
    let live_run_tree = artifact_tree(&run_dir_root);
    let live_clone_tree = artifact_tree(&clone_staging);
    let live_index_tree = artifact_tree(&clone_index);
    let live_duplicate = expect_rejection(
        driver.start(&target_spec).await.expect_err("a duplicate Live start is rejected"),
    );
    assert!(matches!(
        live_duplicate.class,
        DriverStartClass::Vm(VmStartFailure::AllocationAlreadyOwned { ref alloc }) if alloc == &target
    ));
    assert_eq!(driver.live_allocations(), Some(live_claims));
    assert_eq!(cgroup_fs.snapshot(), live_cgroups, "Live cgroups are byte-exact");
    assert_eq!(artifact_tree(&run_dir_root), live_run_tree, "Live run tree is exact");
    assert_eq!(artifact_tree(&clone_staging), live_clone_tree, "Live clone tree is exact");
    assert_eq!(artifact_tree(&clone_index), live_index_tree, "Live index tree is exact");
    assert_eq!(
        barrier_vmm.create_calls.load(Ordering::SeqCst),
        2,
        "Live duplicate does not create"
    );
    assert!(sim.is_live(target_pid), "the original target remains live");
    assert!(sim.is_live(sibling_pid), "the sibling remains live");

    stop_ready(&driver, &target_handle, &clock).await;
    stop_ready(&driver, &sibling_handle, &clock).await;
    driver.release_supervision(&target);
    driver.release_supervision(&sibling);
    assert_eq!(driver.live_allocations(), Some(Vec::new()));
    assert!(!sim.is_live(target_pid));
    assert!(!sim.is_live(sibling_pid));
    let cgroups = cgroup_fs.snapshot();
    assert!(
        !cgroups.contains_key(&CgroupPath::for_alloc(&target).resolve(&tmp.path().join("cgroup"))),
        "ordinary stop removes the target cgroup scope"
    );
    assert!(
        !cgroups.contains_key(&CgroupPath::for_alloc(&sibling).resolve(&tmp.path().join("cgroup"))),
        "ordinary stop removes the sibling cgroup scope"
    );
    assert!(!VmRunDir::for_alloc(&run_dir_root, &target).path().exists());
    assert!(!VmRunDir::for_alloc(&run_dir_root, &sibling).path().exists());
}

/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn failed_start_cleanup_twice_converges_to_the_same_residue_free_state() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let run_dir_root = layout.run_dir_root.clone();
    let clone_staging = layout.clone_staging_dir.clone();
    let clone_index = layout.clone_index_dir.clone();
    let cgroup_root = layout.cgroup_root.clone();
    let sim = SimVmm::new();
    let (driver, clock, cgroup_fs) = build_driver(Arc::new(sim.clone()), layout);
    let sibling = AllocationId::new("alloc-cleanup-twice-sibling").expect("valid sibling id");
    let target = AllocationId::new("alloc-cleanup-twice-target").expect("valid target id");
    let sibling_spec = build_spec(&sibling, &tmp);
    let target_spec = build_spec(&target, &tmp);
    let (sibling_handle, _sibling_beacon) =
        start_ready(&driver, &sibling_spec, &run_dir_root).await;
    let sibling_pid = sibling_handle.pid.expect("sibling VMM pid");
    let sibling_claims = driver.live_allocations().expect("VM driver reports claims");
    let sibling_cgroups = cgroup_fs.snapshot();

    std::fs::remove_file(fixture_rootfs_path(&tmp)).expect("remove target rootfs master");
    let target_plan =
        RootfsPlan::for_alloc(fixture_rootfs_path(&tmp), 0, &target, &clone_staging, &clone_index);
    let target_scope = CgroupPath::for_alloc(&target).resolve(&cgroup_root);

    let mut residue_snapshots = Vec::new();
    for attempt in 1..=2 {
        let failure = expect_rejection(
            driver
                .start(&target_spec)
                .await
                .expect_err("the missing rootfs rejects every fresh start"),
        );
        assert!(
            matches!(failure.class, DriverStartClass::Vm(VmStartFailure::RootfsNotFound { .. })),
            "attempt {attempt} retains the production rootfs classification"
        );
        let residue = (
            VmRunDir::for_alloc(&run_dir_root, &target).path().exists(),
            target_plan.clone_dest().exists(),
            target_plan.index_link().exists(),
            cgroup_fs.snapshot().contains_key(&target_scope),
            driver.live_allocations().expect("VM driver reports claims").contains(&target),
        );
        assert_eq!(
            residue,
            (false, false, false, false, true),
            "attempt {attempt} removes every VM resource while retaining only the disposition-authorship claim"
        );
        driver.release_supervision(&target);
        assert_eq!(
            driver.live_allocations().expect("VM driver reports claims"),
            sibling_claims,
            "attempt {attempt} preserves independent supervision"
        );
        assert_eq!(
            cgroup_fs.snapshot(),
            sibling_cgroups,
            "attempt {attempt} preserves the independent cgroup exactly"
        );
        assert!(sim.is_live(sibling_pid), "attempt {attempt} preserves the sibling VMM");
        residue_snapshots.push(residue);
    }
    assert_eq!(
        residue_snapshots[0], residue_snapshots[1],
        "replaying failed-start cleanup is convergent"
    );

    stop_ready(&driver, &sibling_handle, &clock).await;
    driver.release_supervision(&sibling);
}

/// Every non-OK VM start arm leaves no VMM process, cgroup scope, per-launch
/// rootfs clone, or run directory. Its supervision claim remains only until
/// the action shim resolves the Failed disposition write.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn every_vm_start_rejection_leaves_no_vm_resources() {
    // Three genuinely different rejection arms: pre-provision preflight,
    // `Vmm::create` refusal, and the boot-race VMM-exit arm.
    for arm in ["preflight", "create", "early-exit"] {
        let tmp = TempDir::new().expect("tempdir");
        let layout = build_layout(&tmp);
        let run_dir_root = layout.run_dir_root.clone();
        let cgroup_root = layout.cgroup_root.clone();
        let rootfs_master = fixture_rootfs_path(&tmp);

        let sim = SimVmm::new();
        let vmm: Arc<dyn Vmm> = match arm {
            "preflight" => {
                std::fs::remove_file(fixture_rootfs_path(&tmp)).expect("remove configured rootfs");
                Arc::new(sim.clone())
            }
            "create" => {
                sim.inject_create_failure();
                Arc::new(sim.clone())
            }
            _ => Arc::new(ExitsBeforeBeacon {
                exit: VmmExit { exit_code: Some(1), signal: None, stderr_tail: None },
            }),
        };

        let (driver, _clock, cgroup_fs) = build_driver(vmm, layout);
        let alloc = AllocationId::new("alloc-cleanup").expect("valid alloc id");
        let err = driver.start(&build_spec(&alloc, &tmp)).await.expect_err("start rejects");
        assert!(matches!(err, DriverError::StartRejected { .. }), "[{arm}] must reject");

        assert_eq!(
            driver.live_allocations(),
            Some(vec![alloc.clone()]),
            "[{arm}] the disposition-authorship claim remains held",
        );

        let run_dir = VmRunDir::for_alloc(&run_dir_root, &alloc);
        assert!(!run_dir.path().exists(), "[{arm}] the run directory must be removed");

        let master_bytes = std::fs::metadata(&rootfs_master).map_or(0, |m| m.len());
        let rootfs_plan = RootfsPlan::for_alloc(
            rootfs_master,
            master_bytes,
            &alloc,
            // Same staging dir the layout gives `VmDriver::start` (B1 fix): the
            // clone lands here, and the cleanup path must remove it from here.
            &tmp.path().join("clone-staging"),
            std::path::Path::new("/run/overdrive/vm/clone-index"),
        );
        assert!(
            !rootfs_plan.clone_dest().exists(),
            "[{arm}] the per-launch rootfs clone must be removed",
        );

        let scope_dir = CgroupPath::for_alloc(&alloc).resolve(Path::new(&cgroup_root));
        assert!(
            !cgroup_fs.snapshot().contains_key(&scope_dir),
            "[{arm}] the cgroup scope must be removed",
        );

        if arm != "preflight" {
            assert!(!sim.is_live(1_000_000), "[{arm}] no hypervisor process may survive");
        }
        driver.release_supervision(&alloc);
        assert_eq!(driver.live_allocations(), Some(Vec::new()));
    }
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn run_directory_create_failure_attempts_the_complete_known_cleanup_partition() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    std::fs::write(&layout.run_dir_root, b"not a directory")
        .expect("make the run-directory root non-traversable");
    let alloc = AllocationId::new("alloc-run-dir-create-failure").expect("valid alloc id");
    let scope_dir = CgroupPath::for_alloc(&alloc).resolve(&layout.cgroup_root);
    let (driver, _clock, cgroup_fs) = build_driver(Arc::new(SimVmm::new()), layout.clone());
    cgroup_fs.inject_error(
        SimOp::Write,
        scope_dir.join("cgroup.kill"),
        std::io::ErrorKind::PermissionDenied,
    );
    cgroup_fs.inject_error(SimOp::RemoveDir, scope_dir, std::io::ErrorKind::DirectoryNotEmpty);

    let failure = expect_rejection(
        driver
            .start(&build_spec(&alloc, &tmp))
            .await
            .expect_err("run-directory creation must reject"),
    );

    assert_eq!(failure.class, DriverStartClass::Unclassified { driver: DriverType::Vm });
    for expected in [
        "primary rejection:",
        "create VM run directory:",
        "cgroup kill:",
        "cgroup remove:",
        "run directory remove:",
    ] {
        assert!(failure.detail.contains(expected), "missing `{expected}` in {}", failure.detail);
    }
    assert_eq!(
        driver.live_allocations(),
        Some(vec![alloc.clone()]),
        "the disposition claim survives total cleanup until Failed is authored",
    );
    driver.release_supervision(&alloc);
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test]
async fn cgroup_create_failure_attempts_cleanup_and_removes_the_run_directory() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let alloc = AllocationId::new("alloc-cgroup-create-failure").expect("valid alloc id");
    let scope_dir = CgroupPath::for_alloc(&alloc).resolve(&layout.cgroup_root);
    let run_dir = VmRunDir::for_alloc(&layout.run_dir_root, &alloc);
    let sim = SimVmm::new();
    let (driver, _clock, cgroup_fs) = build_driver(Arc::new(sim.clone()), layout);
    cgroup_fs.inject_error(
        SimOp::CreateDir,
        scope_dir.clone(),
        std::io::ErrorKind::PermissionDenied,
    );
    cgroup_fs.inject_error(
        SimOp::Write,
        scope_dir.join("cgroup.kill"),
        std::io::ErrorKind::PermissionDenied,
    );
    cgroup_fs.inject_error(SimOp::RemoveDir, scope_dir, std::io::ErrorKind::DirectoryNotEmpty);

    let failure = expect_rejection(
        driver.start(&build_spec(&alloc, &tmp)).await.expect_err("cgroup creation must reject"),
    );

    assert_eq!(failure.class, DriverStartClass::Unclassified { driver: DriverType::Vm });
    for expected in
        ["primary rejection:", "create workload scope:", "cgroup kill:", "cgroup remove:"]
    {
        assert!(failure.detail.contains(expected), "missing `{expected}` in {}", failure.detail);
    }
    assert!(!run_dir.path().exists(), "cleanup removes the pre-cgroup run directory");
    assert!(!sim.is_live(1_000_000), "the cgroup-create arm never starts a VMM");
    assert_eq!(driver.live_allocations(), Some(vec![alloc.clone()]));
    driver.release_supervision(&alloc);
}

/// A start that is rejected for a NON-absence reason must not be
/// relabelled as absence. Guards the `.claude/rules/development.md`
/// § Errors discipline at the one boundary where a wrong diagnosis sends
/// an operator to check the wrong thing entirely.
#[tokio::test]
async fn a_permission_error_is_never_reported_as_a_missing_artifact() {
    let tmp = TempDir::new().expect("tempdir");
    let layout = build_layout(&tmp);
    let vmm = FailsCreate {
        make: Box::new(|| {
            VmmError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Permission denied (os error 13)",
            ))
        }),
    };
    let (driver, _clock, _fs) = build_driver(Arc::new(vmm), layout);
    let alloc = AllocationId::new("alloc-eacces").expect("valid alloc id");

    let err = driver.start(&build_spec(&alloc, &tmp)).await.expect_err("io failure rejects start");
    let failure = expect_rejection(err);

    assert!(
        matches!(failure.class, DriverStartClass::Unclassified { driver: DriverType::Vm }),
        "a permission failure must reach the unknown fallback, never an absence class: {:?}",
        failure.class,
    );
    assert!(
        failure.detail.contains("Permission denied"),
        "the real cause must survive in the diagnostic: {}",
        failure.detail,
    );
}
