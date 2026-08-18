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

use std::num::NonZeroU8;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::SpiffeId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverError, DriverPayload, DriverStartClass, DriverType, Resources,
    VmPayload, VmStartFailure,
};
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmExitWatch, VmProcess, VmTermination, Vmm, VmmDiagnostics,
    VmmError, VmmExit, VmmProbeError,
};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, RootfsPlan, VmConfig, VmConfinement, VmRunDir, VmmIdentity,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_worker::VmDriver;
use overdrive_worker::vm_driver::VmHostLayout;
use tempfile::TempDir;

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
        identity: SpiffeId::new("spiffe://overdrive.local/workload/vm-fail/alloc/x")
            .expect("valid spiffe id"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/sbin/init".to_owned(),
            args: vec![],
            kernel: fixture_kernel_path(tmp),
            rootfs: fixture_rootfs_path(tmp),
            volumes: vec![],
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
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
    let second = expect_vm_class(driver.start(&spec).await.expect_err("restart start rejects"));

    assert_eq!(
        first.0,
        VmStartFailure::RootfsNotFound { path: configured },
        "the initial start names the exact configured rootfs path",
    );
    assert_eq!(first, second, "initial and restart starts must persist identical cause/detail");
}

/// Every non-OK VM start arm releases the supervision claim and leaves no
/// VMM process, cgroup scope, per-launch rootfs clone, or run directory.
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
            Some(Vec::new()),
            "[{arm}] the supervision claim must be released",
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
    }
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
