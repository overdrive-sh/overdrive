//! Tier-3 metal acceptance tests for guest-stack transparent mTLS egress.
//!
//! These tests drive the real CLI command libraries: an mTLS-composed
//! `serve`, TOML `deploy`, `workload describe`, and `stop`. The client
//! is a real Cloud Hypervisor guest whose ordinary `TcpStream` resolves and
//! dials the mesh service. No test installs an intercept, route, address, or
//! resolver entry; those effects come exclusively from the production boot and
//! allocation paths.
//!
//! The workload speaks plaintext by design. Encryption is proven on the
//! inter-agent leg-B/leg-C wire: an AF_PACKET capture observes TLS application
//! records in both directions with neither byte-distinct plaintext marker, and
//! `ss -tie` observes the kernel TLS ULP on the live connection.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::unwrap_used,
    reason = "Tier-3 fixtures fail fast, and Contract Shape declarations use exact mandated tokens"
)]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::os::fd::AsRawFd as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::FutureExt as _;
use overdrive_cli::commands::deploy::{DeployArgs, DeployOutput, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, WorkloadDescribeOutput, describe};
use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, IssuedCertSummary, ResourcesBody,
    RestartBudget, StopOutcome, TransitionRecord, TransitionSource,
};
use overdrive_control_plane::dns_responder::frontend_addr_allocator::WORKLOAD_FRONTEND_BASE;
use overdrive_control_plane::veth_provisioner::{
    DEFAULT_CLIENT_IFACE, NetSlot, WORKLOAD_SUBNET_BASE, derive_vm_tap_plan,
    derive_workload_netns_plan, responder_addr_for_slot,
};
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::ObservationStore as _;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::transition_reason::StoppedBy;
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_core::vm::config::{RootfsPlan, VmConfig, VmRunDir, clone_staging_dir};
use overdrive_core::{SpiffeId, TransitionReason, aggregate::WorkloadKind};
use overdrive_netlink::nft;
use overdrive_store_local::LocalObservationStore;
use overdrive_testing::vm_fixture::VmFixture;
use proptest::prelude::*;
use serial_test::serial;
use tempfile::TempDir;

use super::vm_walking_skeleton::{
    build_spin_binary, config_path, poll_until_running, poll_until_terminal, shared_staging_root,
    stage_rootfs_with_extra_binary, vm_job_toml, write_toml,
};

const SERVICE_PORT: u16 = 18_951;
const MESH_NAME: &str = "server.svc.overdrive.local";
const REQUEST: &[u8] =
    b"GTI_REQUEST_guest_plaintext_dial_by_name_must_be_encrypted_on_peer_wire_0201";
const RESPONSE: &[u8] =
    b"GTI_RESPONSE_peer_authored_distinct_reply_returns_to_guest_byte_exact_0201";
const NON_MESH_REQUEST: &[u8] =
    b"GTI_NON_MESH_REQUEST_plaintext_passthrough_outside_workload_subnet_0201";
const NON_MESH_RESPONSE: &[u8] =
    b"GTI_NON_MESH_RESPONSE_distinct_clear_reply_reaches_guest_unchanged_0201";
const LOOPBACK_IFACE: &str = "lo";
const OPERATOR_MARKER: &str = "/gti-operator-action-ran";
const OPERATOR_CONSOLE_MARKER: &str = "GTI_OPERATOR_ACTION_RAN";

async fn spawn_mtls_server() -> (ServeHandle, TempDir) {
    let tmp = tempfile::Builder::new()
        .prefix("gti-serve-")
        .tempdir_in(shared_staging_root())
        .expect("serve tempdir on the reflink-capable metal staging root");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create serve data dir");
    std::fs::create_dir_all(&config_dir).expect("create serve config dir");

    let args = ServeArgs {
        bind: "127.0.0.1:0".parse().expect("parse loopback bind"),
        data_dir,
        config_dir,
    };
    let handle = overdrive_cli::commands::serve::run_with_kek(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
    )
    .await
    .expect("start mTLS-composed serve through the CLI composition root");
    (handle, tmp)
}

struct VmmSpawnCut {
    config: VmConfig,
    release: tokio::sync::oneshot::Sender<()>,
}

struct CaptureReadyVmm {
    inner: Arc<dyn Vmm>,
    spawn_cut: Sender<VmmSpawnCut>,
}

#[async_trait]
impl Vmm for CaptureReadyVmm {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        let (release, wait) = tokio::sync::oneshot::channel();
        self.spawn_cut.send(VmmSpawnCut { config: config.clone(), release }).map_err(|_| {
            overdrive_core::traits::vmm::VmmError::create(
                "capture observer disappeared before VMM spawn",
            )
        })?;
        wait.await.map_err(|_| {
            overdrive_core::traits::vmm::VmmError::create(
                "capture observer did not acknowledge capture-ready before VMM spawn",
            )
        })?;
        self.inner.create(config).await
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

async fn spawn_capture_observed_mtls_server() -> (ServeHandle, TempDir, Receiver<VmmSpawnCut>) {
    let tmp = tempfile::Builder::new()
        .prefix("gti-serve-")
        .tempdir_in(shared_staging_root())
        .expect("serve tempdir on the reflink-capable metal staging root");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create serve data dir");
    std::fs::create_dir_all(&config_dir).expect("create serve config dir");
    let (handle, cuts) = spawn_capture_observed_mtls_server_at(&data_dir, &config_dir).await;
    (handle, tmp, cuts)
}

async fn spawn_capture_observed_mtls_server_at(
    data_dir: &Path,
    config_dir: &Path,
) -> (ServeHandle, Receiver<VmmSpawnCut>) {
    std::fs::create_dir_all(data_dir).expect("create serve data dir");
    std::fs::create_dir_all(config_dir).expect("create serve config dir");
    let args = ServeArgs {
        bind: "127.0.0.1:0".parse().expect("parse loopback bind"),
        data_dir: data_dir.to_path_buf(),
        config_dir: config_dir.to_path_buf(),
    };
    let (spawn_cut, cuts) = std::sync::mpsc::channel();
    let vmm =
        CaptureReadyVmm { inner: Arc::new(overdrive_host::CloudHypervisorVmm::new()), spawn_cut };
    let handle = overdrive_cli::commands::serve::run_with_kek_and_vmm_override(
        args,
        Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
        Arc::new(vmm),
    )
    .await
    .expect("start production-mTLS serve with observation-only real-VMM decorator");
    (handle, cuts)
}

struct FailureObservedVmm {
    inner: Arc<dyn Vmm>,
    spawn_cut: Sender<VmmSpawnCut>,
    created: Sender<VmControl>,
    boundary_observed: Sender<GuestBoundaryObservation>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct GuestBeaconTrace {
    ready: usize,
    exec: usize,
    exit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GuestBoundaryObservation {
    alloc: AllocationId,
    operator_action: bool,
    beacon: GuestBeaconTrace,
}

fn guest_beacon_trace(console: &str) -> GuestBeaconTrace {
    GuestBeaconTrace {
        ready: console.matches("overdrive-init: beacon boundary tx READY").count(),
        exec: console.matches("overdrive-init: beacon boundary rx EXEC").count(),
        exit: console.matches("overdrive-init: beacon boundary tx EXIT").count(),
    }
}

#[async_trait]
impl Vmm for FailureObservedVmm {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig) -> VmmResult<VmProcess> {
        let (release, wait) = tokio::sync::oneshot::channel();
        self.spawn_cut.send(VmmSpawnCut { config: config.clone(), release }).map_err(|_| {
            overdrive_core::traits::vmm::VmmError::create(
                "failure observer disappeared before VMM spawn",
            )
        })?;
        wait.await.map_err(|_| {
            overdrive_core::traits::vmm::VmmError::create(
                "failure observer did not acknowledge capture-ready before VMM spawn",
            )
        })?;
        let process = self.inner.create(config).await?;
        self.created.send(process.control.clone()).map_err(|_| {
            overdrive_core::traits::vmm::VmmError::create(
                "failure observer disappeared after VMM spawn",
            )
        })?;
        let VmProcess { control, mut exit, diagnostics } = process;
        let alloc = config.alloc.clone();
        let rootfs = config.rootfs.clone_dest().to_path_buf();
        let console = config.run_dir.console_log();
        let boundary_observed = self.boundary_observed.clone();
        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if let Some(ending) = exit.recv().await {
                let console = std::fs::read_to_string(console).unwrap_or_default();
                let _ = boundary_observed.send(GuestBoundaryObservation {
                    alloc,
                    operator_action: console.contains(OPERATOR_CONSOLE_MARKER)
                        || ext4_path_exists(&rootfs, OPERATOR_MARKER),
                    beacon: guest_beacon_trace(&console),
                });
                let _ = exit_tx.send(ending);
            }
        });
        Ok(VmProcess {
            control,
            exit: overdrive_core::traits::vmm::VmExitWatch::new(exit_rx),
            diagnostics,
        })
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

fn ext4_path_exists(rootfs: &Path, path: &str) -> bool {
    let output = Command::new("debugfs")
        .args(["-R", &format!("stat {path}")])
        .arg(rootfs)
        .output()
        .expect("inspect guest rootfs with debugfs");
    output.status.success()
        && !String::from_utf8_lossy(&output.stderr).contains("File not found")
        && String::from_utf8_lossy(&output.stdout).contains("Inode:")
}

fn assert_guest_boundary(
    observations: &Receiver<GuestBoundaryObservation>,
    expected_alloc: &str,
    expected_operator_action: bool,
    expected_beacon: GuestBeaconTrace,
) {
    let observed = observations
        .recv_timeout(Duration::from_secs(30))
        .expect("VMM cleanup emits the exact guest-boundary observation before clone deletion");
    assert_eq!(observed.alloc.to_string(), expected_alloc);
    assert_eq!(
        observed.operator_action, expected_operator_action,
        "operator marker complement for allocation {expected_alloc}",
    );
    assert_eq!(
        observed.beacon, expected_beacon,
        "the real guest boundary must expose the exact READY/EXEC/EXIT history",
    );
}

async fn spawn_failure_observed_mtls_server() -> (
    ServeHandle,
    TempDir,
    Receiver<VmmSpawnCut>,
    Receiver<VmControl>,
    Receiver<GuestBoundaryObservation>,
    Arc<dyn Vmm>,
) {
    let tmp = tempfile::Builder::new()
        .prefix("gti-failure-serve-")
        .tempdir_in(shared_staging_root())
        .expect("serve tempdir on the reflink-capable metal staging root");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create serve data dir");
    std::fs::create_dir_all(&config_dir).expect("create serve config dir");
    let (handle, cuts, created, boundary_observed, inner) =
        spawn_failure_observed_mtls_server_at(&data_dir, &config_dir).await;
    (handle, tmp, cuts, created, boundary_observed, inner)
}

async fn spawn_failure_observed_mtls_server_at(
    data_dir: &Path,
    config_dir: &Path,
) -> (
    ServeHandle,
    Receiver<VmmSpawnCut>,
    Receiver<VmControl>,
    Receiver<GuestBoundaryObservation>,
    Arc<dyn Vmm>,
) {
    std::fs::create_dir_all(data_dir).expect("create serve data dir");
    std::fs::create_dir_all(config_dir).expect("create serve config dir");
    let args = ServeArgs {
        bind: "127.0.0.1:0".parse().expect("parse loopback bind"),
        data_dir: data_dir.to_path_buf(),
        config_dir: config_dir.to_path_buf(),
    };
    let (spawn_cut, cuts) = std::sync::mpsc::channel();
    let (created_tx, created) = std::sync::mpsc::channel();
    let (boundary_tx, boundary_observed) = std::sync::mpsc::channel();
    let inner: Arc<dyn Vmm> = Arc::new(overdrive_host::CloudHypervisorVmm::new());
    let vmm = FailureObservedVmm {
        inner: Arc::clone(&inner),
        spawn_cut,
        created: created_tx,
        boundary_observed: boundary_tx,
    };
    let handle = overdrive_cli::commands::serve::run_with_kek_and_vmm_override(
        args,
        Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
        Arc::new(vmm),
    )
    .await
    .expect("start production-mTLS serve with failure-observed real VMM");
    (handle, cuts, created, boundary_observed, inner)
}

fn build_static_binary(tmp: &Path, name: &str, source: &str) -> PathBuf {
    let src = tmp.join(format!("{name}.rs"));
    std::fs::write(&src, source).expect("write static test fixture source");
    let out = tmp.join(name);
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
        .expect("spawn rustc for static test fixture");
    assert!(status.success(), "rustc must build {name}");
    out
}

struct RootfsMountFixture {
    watchdog: Option<std::process::Child>,
    dir: TempDir,
    mountpoint: PathBuf,
    ready: PathBuf,
    stop: PathBuf,
    loop_file: PathBuf,
    fault: Option<RootfsWatchdogFault>,
    expected_exit: RootfsWatchdogExit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootfsWatchdogFault {
    Stop,
    Signal,
    Wait,
    Verify,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RootfsWatchdogExit {
    #[default]
    Clean,
    Signalled,
}

impl RootfsMountFixture {
    fn mount(rootfs: &Path, tmp: &Path) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("resolver-rootfs-watchdog-")
            .tempdir_in(tmp)
            .expect("create resolver watchdog dir before mutation");
        let mountpoint = dir.path().join("mnt");
        let ready = dir.path().join("ready");
        let stop = dir.path().join("stop");
        let loop_file = dir.path().join("loop-device");
        std::fs::create_dir(&mountpoint).expect("create guarded rootfs mountpoint");
        let script = r#"
set -eu
parent="$1"; rootfs="$2"; mountpoint="$3"; ready="$4"; stop="$5"; loop_file="$6"
loopdev=""
cleanup() {
  set +e
  if mountpoint -q "$mountpoint"; then umount "$mountpoint"; fi
  if [ -n "$loopdev" ]; then losetup -d "$loopdev"; fi
}
trap cleanup EXIT
trap 'exit 128' HUP INT TERM
loopdev=$(losetup --find --show "$rootfs")
printf '%s\n' "$loopdev" > "$loop_file"
mount "$loopdev" "$mountpoint"
: > "$ready"
while kill -0 "$parent" 2>/dev/null && [ ! -e "$stop" ]; do sleep 0.05; done
"#;
        let watchdog = Command::new("sh")
            .arg("-c")
            .arg(script)
            .arg("rootfs-watchdog")
            .arg(std::process::id().to_string())
            .arg(rootfs)
            .arg(&mountpoint)
            .arg(&ready)
            .arg(&stop)
            .arg(&loop_file)
            .spawn()
            .expect("spawn rootfs cleanup watchdog before loop attachment");
        let mut fixture = Self {
            watchdog: Some(watchdog),
            dir,
            mountpoint,
            ready,
            stop,
            loop_file,
            fault: None,
            expected_exit: RootfsWatchdogExit::Clean,
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !fixture.ready.exists() {
            if let Some(status) = fixture
                .watchdog
                .as_mut()
                .expect("watchdog present")
                .try_wait()
                .expect("poll rootfs watchdog")
            {
                let detached = fixture.verify_detached();
                if detached.is_ok() {
                    fixture.watchdog = None;
                }
                panic!("rootfs watchdog exited before mutation: {status}; detached={detached:?}");
            }
            assert!(std::time::Instant::now() < deadline, "rootfs watchdog became ready");
            std::thread::sleep(Duration::from_millis(10));
        }
        fixture
    }

    fn finish(mut self) {
        self.restore().unwrap_or_else(|error| panic!("authoritative rootfs restoration: {error}"));
    }

    fn inject_fault(&mut self, fault: RootfsWatchdogFault) {
        self.fault = Some(fault);
    }

    fn take_fault(&mut self, expected: RootfsWatchdogFault) -> bool {
        if self.fault == Some(expected) {
            self.fault = None;
            true
        } else {
            false
        }
    }

    fn validate_exit(&self, status: std::process::ExitStatus) -> Result<(), String> {
        match self.expected_exit {
            RootfsWatchdogExit::Clean if status.success() => Ok(()),
            RootfsWatchdogExit::Signalled if status.code() == Some(128) => Ok(()),
            expected => Err(format!("watchdog exited {status}; expected {expected:?}")),
        }
    }

    fn signal_and_wait(&mut self, signal: i32) -> Result<(), String> {
        if self.take_fault(RootfsWatchdogFault::Signal) {
            return Err("injected watchdog signal failure".to_owned());
        }
        let watchdog = self.watchdog.as_mut().ok_or_else(|| "watchdog absent".to_owned())?;
        // SAFETY: the PID belongs to the live child retained by this fixture.
        if unsafe { libc::kill(watchdog.id().cast_signed(), signal) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        self.expected_exit = RootfsWatchdogExit::Signalled;
        if self.take_fault(RootfsWatchdogFault::Wait) {
            return Err("injected watchdog wait failure".to_owned());
        }
        let status = self
            .watchdog
            .as_mut()
            .expect("watchdog ownership retained after signal")
            .wait()
            .map_err(|error| error.to_string())?;
        self.validate_exit(status)?;
        if self.take_fault(RootfsWatchdogFault::Verify) {
            return Err("injected watchdog detached-state verification failure".to_owned());
        }
        self.verify_detached()?;
        self.watchdog = None;
        Ok(())
    }

    fn restore(&mut self) -> Result<(), String> {
        if self.watchdog.is_none() {
            return Ok(());
        }
        if self.take_fault(RootfsWatchdogFault::Stop) {
            return Err("injected watchdog stop-file failure".to_owned());
        }
        std::fs::write(&self.stop, b"").map_err(|error| error.to_string())?;
        if self.take_fault(RootfsWatchdogFault::Wait) {
            return Err("injected watchdog wait failure".to_owned());
        }
        let status = self
            .watchdog
            .as_mut()
            .expect("watchdog ownership retained through stop")
            .wait()
            .map_err(|error| error.to_string())?;
        self.validate_exit(status)?;
        if self.take_fault(RootfsWatchdogFault::Verify) {
            return Err("injected watchdog detached-state verification failure".to_owned());
        }
        self.verify_detached()?;
        self.watchdog = None;
        Ok(())
    }

    fn verify_detached(&self) -> Result<(), String> {
        if Command::new("mountpoint")
            .args(["-q"])
            .arg(&self.mountpoint)
            .status()
            .map_err(|error| error.to_string())?
            .success()
        {
            return Err(format!("{} remains mounted", self.mountpoint.display()));
        }
        let loop_device =
            std::fs::read_to_string(&self.loop_file).map_err(|error| error.to_string())?;
        if Command::new("losetup")
            .arg(loop_device.trim())
            .output()
            .map_err(|error| error.to_string())?
            .status
            .success()
        {
            return Err(format!("{} remains attached", loop_device.trim()));
        }
        Ok(())
    }
}

impl Drop for RootfsMountFixture {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!(
                "authoritative rootfs fixture restoration failed in {}: {error}",
                self.dir.path().display()
            );
            if std::thread::panicking() {
                std::process::abort();
            }
            panic!("authoritative rootfs fixture restoration failed: {error}");
        }
    }
}

fn replace_resolver_with_directory(rootfs: &Path, tmp: &Path) {
    let fixture = RootfsMountFixture::mount(rootfs, tmp);
    let mountpoint = fixture.mountpoint.clone();

    let resolver = mountpoint.join("etc/resolv.conf");
    match std::fs::symlink_metadata(&resolver) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            std::fs::remove_dir_all(&resolver).expect("remove prior resolver directory");
        }
        Ok(_) => std::fs::remove_file(&resolver).expect("remove prior resolver entry"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("inspect prior resolver entry: {error}"),
    }
    std::fs::create_dir_all(resolver.parent().expect("resolver has /etc parent"))
        .expect("create guest /etc directory");
    std::fs::create_dir(&resolver).expect("make /etc/resolv.conf a directory");
    fixture.finish();
}

fn assert_no_loop_device_for(rootfs: &Path, context: &str) {
    let output =
        Command::new("losetup").arg("-j").arg(rootfs).output().expect("query loop ownership");
    assert!(output.status.success(), "losetup -j failed");
    assert!(
        output.stdout.is_empty(),
        "{context}: rootfs remains attached: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(clippy::too_many_lines)]
#[test]
#[serial(cgroup)]
fn resolver_rootfs_mutator_restores_mount_and_loop_on_panic_signal_and_parent_death() {
    const CHILD_MODE: &str = "OVERDRIVE_GTI_ROOTFS_PARENT_DEATH";
    const ROOTFS_ENV: &str = "OVERDRIVE_GTI_ROOTFS_PARENT_DEATH_IMAGE";
    const THIS_TEST: &str = "integration::guest_stack_mtls_egress::resolver_rootfs_mutator_restores_mount_and_loop_on_panic_signal_and_parent_death";

    if std::env::var_os(CHILD_MODE).is_some() {
        let rootfs = PathBuf::from(std::env::var_os(ROOTFS_ENV).expect("child rootfs path"));
        let tmp = TempDir::new().expect("child watchdog tempdir");
        let _fixture = RootfsMountFixture::mount(&rootfs, tmp.path());
        std::process::abort();
    }

    let tmp = TempDir::new().expect("rootfs watchdog test tempdir");
    let rootfs = tmp.path().join("watchdog-rootfs.ext4");
    let truncate = Command::new("truncate")
        .args(["-s", "8M"])
        .arg(&rootfs)
        .status()
        .expect("size rootfs fixture");
    assert!(truncate.success());
    let mkfs = Command::new("mkfs.ext4")
        .args(["-F", "-q"])
        .arg(&rootfs)
        .status()
        .expect("format rootfs fixture");
    assert!(mkfs.success());

    let panic_result = std::panic::catch_unwind(|| {
        let _fixture = RootfsMountFixture::mount(&rootfs, tmp.path());
        panic!("intentional rootfs fixture panic");
    });
    assert!(panic_result.is_err());
    assert_no_loop_device_for(&rootfs, "panic/drop restoration");

    let mut signalled = RootfsMountFixture::mount(&rootfs, tmp.path());
    signalled.signal_and_wait(libc::SIGTERM).expect("signal restoration is authoritative");
    assert_no_loop_device_for(&rootfs, "signal restoration");

    for fault in [RootfsWatchdogFault::Stop, RootfsWatchdogFault::Wait, RootfsWatchdogFault::Verify]
    {
        let mut partitioned = RootfsMountFixture::mount(&rootfs, tmp.path());
        partitioned.inject_fault(fault);
        assert!(partitioned.restore().is_err(), "{fault:?} partition must be observed");
        assert!(
            partitioned.watchdog.is_some(),
            "{fault:?} cannot discard watchdog cleanup authority",
        );
        partitioned.restore().expect("retained watchdog retries restoration");
        assert_no_loop_device_for(&rootfs, &format!("{fault:?} retry restoration"));
    }

    let mut signal_failed = RootfsMountFixture::mount(&rootfs, tmp.path());
    signal_failed.inject_fault(RootfsWatchdogFault::Signal);
    assert!(signal_failed.signal_and_wait(libc::SIGTERM).is_err());
    assert!(signal_failed.watchdog.is_some());
    signal_failed.restore().expect("signal failure retains ordinary stop recovery");
    assert_no_loop_device_for(&rootfs, "signal-error retry restoration");

    for fault in [RootfsWatchdogFault::Wait, RootfsWatchdogFault::Verify] {
        let mut partitioned = RootfsMountFixture::mount(&rootfs, tmp.path());
        partitioned.inject_fault(fault);
        assert!(partitioned.signal_and_wait(libc::SIGTERM).is_err());
        assert!(
            partitioned.watchdog.is_some(),
            "signal-path {fault:?} cannot discard watchdog cleanup authority",
        );
        partitioned.restore().expect("signal-path retained watchdog retries restoration");
        assert_no_loop_device_for(&rootfs, &format!("signal-path {fault:?} retry restoration"));
    }

    let child = Command::new(std::env::current_exe().expect("locate integration test binary"))
        .args(["--exact", THIS_TEST, "--nocapture"])
        .env(CHILD_MODE, "1")
        .env(ROOTFS_ENV, &rootfs)
        .status()
        .expect("run aborting fixture parent");
    assert!(!child.success(), "parent-death child must abort");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let output = Command::new("losetup")
            .arg("-j")
            .arg(&rootfs)
            .output()
            .expect("poll parent-death restoration");
        if output.stdout.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "parent-death watchdog detaches within 10s");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_no_loop_device_for(&rootfs, "parent-death restoration");
}

fn build_forbidden_exec_probe(tmp: &Path, name: &str) -> PathBuf {
    build_static_binary(
        tmp,
        name,
        r#"
use std::fs::OpenOptions;
use std::io::Write;
use std::net::{TcpStream, SocketAddr};
use std::time::Duration;

fn main() {
    eprintln!("GTI_OPERATOR_ACTION_RAN");
    let mut marker = OpenOptions::new().create_new(true).write(true)
        .open("/gti-operator-action-ran").unwrap();
    marker.write_all(b"EXEC reached operator action\n").unwrap();
    marker.sync_all().unwrap();
    let destination: SocketAddr = "198.51.100.1:9".parse().unwrap();
    let _ = TcpStream::connect_timeout(&destination, Duration::from_secs(2));
    std::process::exit(47);
}
"#,
    )
}

fn build_exit_78_binary(tmp: &Path) -> PathBuf {
    build_static_binary(
        tmp,
        "gti-exit-78",
        r#"
use std::time::Duration;
use std::fs::OpenOptions;
use std::io::Write;

fn main() {
    eprintln!("GTI_OPERATOR_ACTION_RAN");
    let mut marker = OpenOptions::new().create_new(true).write(true)
        .open("/gti-operator-action-ran").unwrap();
    marker.write_all(b"READY then EXEC reached operator action\n").unwrap();
    marker.sync_all().unwrap();
    std::thread::sleep(Duration::from_secs(2));
    std::process::exit(78);
}
"#,
    )
}

fn build_mesh_peer(tmp: &Path) -> PathBuf {
    let source = format!(
        r#"
use std::io::{{Read, Write}};
use std::net::TcpListener;
use std::time::Duration;

fn main() {{
    let listener = TcpListener::bind(("0.0.0.0", {SERVICE_PORT})).unwrap();
    loop {{
        let (mut stream, _) = listener.accept().unwrap();
        std::thread::spawn(move || {{
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut buf = [0_u8; 4096];
            let Ok(n) = stream.read(&mut buf) else {{ return }};
            if &buf[..n] != {REQUEST:?} {{
                return;
            }}
            stream.write_all(&{RESPONSE:?}).unwrap();
            stream.flush().unwrap();
            std::thread::sleep(Duration::from_secs(20));
        }});
    }}
}}
"#,
    );
    build_static_binary(tmp, "gti-peer", &source)
}

fn build_mesh_guest(tmp: &Path) -> PathBuf {
    build_mesh_guest_with_delay(tmp, "gti-mesh-guest", 0)
}

fn build_mesh_guest_with_delay(tmp: &Path, name: &str, initial_delay_secs: u64) -> PathBuf {
    build_mesh_guest_with_timing(tmp, name, initial_delay_secs, 12)
}

fn build_mesh_guest_with_timing(
    tmp: &Path,
    name: &str,
    initial_delay_secs: u64,
    authenticated_hold_secs: u64,
) -> PathBuf {
    let response_len = RESPONSE.len();
    let source = format!(
        r#"
use std::io::{{Read, Write}};
use std::net::{{TcpStream, ToSocketAddrs}};
use std::time::{{Duration, Instant}};

fn main() {{
    std::thread::sleep(Duration::from_secs({initial_delay_secs}));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {{
        if let Ok(addrs) = ("{MESH_NAME}", {SERVICE_PORT}).to_socket_addrs() {{
            for addr in addrs {{
                if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) {{
                    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    if stream.write_all(&{REQUEST:?}).is_ok()
                        && stream.flush().is_ok()
                    {{
                        let mut got = vec![0_u8; {response_len}];
                        if stream.read_exact(&mut got).is_ok() && got == {RESPONSE:?} {{
                            // Keep the authenticated data socket alive long enough for the
                            // host-side exact-tuple kTLS oracle to inspect both directions.
                            std::thread::sleep(Duration::from_secs({authenticated_hold_secs}));
                            return;
                        }}
                    }}
                }}
            }}
        }}
        if Instant::now() >= deadline {{
            std::process::exit(41);
        }}
        std::thread::sleep(Duration::from_millis(100));
    }}
}}
"#,
    );
    build_static_binary(tmp, name, &source)
}

fn build_persistent_operator_marker(tmp: &Path) -> PathBuf {
    build_static_binary(
        tmp,
        "gti-persistent-operator-marker",
        r#"
use std::fs::OpenOptions;
use std::io::Write as _;
use std::time::Duration;

fn main() {
    let mut marker = OpenOptions::new().create_new(true).write(true)
        .open("/gti-operator-action-ran").unwrap();
    marker.write_all(b"EXEC reached operator action\n").unwrap();
    marker.sync_all().unwrap();
    loop { std::thread::sleep(Duration::from_secs(60)); }
}
"#,
    )
}

fn build_non_mesh_guest(tmp: &Path, destination: SocketAddr) -> PathBuf {
    let response_len = NON_MESH_RESPONSE.len();
    let source = format!(
        r#"
use std::io::{{Read, Write}};
use std::net::TcpStream;
use std::time::Duration;

fn main() {{
    // Give the operator-side test time to observe Running before the dial.
    std::thread::sleep(Duration::from_secs(2));
    let mut stream = TcpStream::connect("{destination}").unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    stream.write_all(&{NON_MESH_REQUEST:?}).unwrap();
    stream.flush().unwrap();
    let mut got = vec![0_u8; {response_len}];
    if stream.read_exact(&mut got).is_err() || got != {NON_MESH_RESPONSE:?} {{
        std::process::exit(43);
    }}
}}
"#,
    );
    build_static_binary(tmp, "gti-non-mesh-guest", &source)
}

fn service_toml(peer: &Path) -> String {
    let command = toml::Value::String(peer.display().to_string()).to_string();
    format!(
        "[service]\nid = \"server\"\nreplicas = 1\n\n[[listener]]\nport = {SERVICE_PORT}\n\
         protocol = \"tcp\"\n\n[exec]\ncommand = {command}\nargs = []\n\n[resources]\n\
         cpu_milli = 100\nmemory_bytes = 67108864\n"
    )
}

const TLS_APPLICATION_DATA: u8 = 0x17;
const TLS_RECORD_HEADER_LEN: usize = 5;
const IPV4_HEADER_LEN: usize = 20;
const ETH_P_ALL: std::os::raw::c_int = 0x0003;
const ETH_P_IP: u16 = 0x0800;
const PACKET_AUXDATA: libc::c_int = 8;
const PACKET_STATISTICS: libc::c_int = 6;
const TP_STATUS_CSUMNOTREADY: u32 = 1 << 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FlowTuple {
    source: SocketAddrV4,
    destination: SocketAddrV4,
}

struct ParsedTcpSegment<'a> {
    tuple: FlowTuple,
    sequence: u32,
    flags: u8,
    payload: &'a [u8],
    total_len: usize,
}

impl FlowTuple {
    fn reverse(self) -> Self {
        Self { source: self.destination, destination: self.source }
    }
}

#[derive(Debug, Clone, Default)]
struct WireScan {
    exact_tuple: Option<FlowTuple>,
    exact_records_to_peer: u64,
    exact_records_from_peer: u64,
    plaintext_hits_on_any_peer_stream: u64,
    peer_streams_observed: usize,
    capture_packets: u32,
    capture_drops: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KtlsSocketEvidence {
    tuple: FlowTuple,
    record: String,
}

#[derive(Debug)]
struct CapturedFrame {
    /// Kernel packet-event time from `SCM_TIMESTAMPNS`. Missing ancillary
    /// data remains `None`, so unprovable ordering is classified pre-ready.
    kernel_event_at: Option<KernelRealtime>,
    ifindex: u32,
    protocol: u16,
    packet_type: u8,
    wire_len: usize,
    truncated: bool,
    control_truncated: bool,
    aux: Option<PacketAuxData>,
    bytes: Vec<u8>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct PacketAuxData {
    status: u32,
    len: u32,
    snaplen: u32,
    mac: u16,
    net: u16,
    vlan_tci: u16,
    vlan_tpid: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct PacketStatistics {
    packets: u32,
    drops: u32,
}

#[derive(Debug)]
struct CaptureBatch {
    frames: Vec<CapturedFrame>,
    statistics: PacketStatistics,
}

/// Nanoseconds in the shared `SO_TIMESTAMPNS` / `CLOCK_REALTIME` domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct KernelRealtime(i128);

#[derive(Debug)]
struct InterceptReadiness {
    kernel_barrier_at: KernelRealtime,
    host_veth_ifindex: u32,
    host_veth: String,
    accounting: D7Accounting,
}

#[derive(Debug)]
struct LiveInterceptReadiness {
    kernel_barrier_at: KernelRealtime,
    host_veth_ifindex: u32,
    host_veth: String,
    before: [nft::RuleSnapshot; 2],
    target_userdata: Vec<u8>,
    observer: nft::NftRuleObserver,
}

#[derive(Debug)]
struct D7Accounting {
    before: [nft::RuleSnapshot; 2],
    after: [nft::RuleSnapshot; 2],
    target_userdata: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GuestBoundarySegment {
    kernel_event_at: Option<KernelRealtime>,
    tuple: FlowTuple,
    flags: u8,
    payload_len: usize,
    ipv4_total_len: usize,
}

#[derive(Debug)]
struct GuestEgressAudit {
    /// Complete frame-derived universe on the exact allocation host-veth for
    /// guest-source TCP headed to the exact mesh peer address and port.
    segments: Vec<GuestBoundarySegment>,
    /// Diagnostic complement: every parsed TCP segment on that exact
    /// interface, retained so a tuple-correlation failure reports what the
    /// independent boundary actually observed.
    interface_tcp: Vec<GuestBoundarySegment>,
    first_syn: Option<GuestBoundarySegment>,
    plaintext_request_hits: u64,
    packet_count: u64,
    byte_count: u64,
}

struct WireCapture {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<CaptureBatch>>,
    port: u16,
}

impl WireCapture {
    fn start(iface: &str, port: u16) -> Self {
        let iface = std::ffi::CString::new(iface).expect("iface has no NUL");
        // SAFETY: libc retains neither pointer and the returned fd is owned here.
        let ifindex = unsafe { libc::if_nametoindex(iface.as_ptr()) };
        assert!(ifindex != 0, "resolve AF_PACKET interface index");

        Self::start_bound(ifindex, port)
    }

    /// Capture every interface from before the allocation veth exists. Binding
    /// AF_PACKET with ifindex zero is the kernel's all-interface shape; each
    /// `recvmsg` result retains its actual `sockaddr_ll.sll_ifindex` and kernel
    /// packet timestamp, allowing the audit to select the exact host-veth
    /// after production creates it without relabeling queued packets at
    /// userspace dequeue time.
    fn start_all() -> Self {
        Self::start_bound(0, 0)
    }

    fn start_bound(ifindex: u32, port: u16) -> Self {
        let fd = open_bound_packet_socket(ifindex).expect("open bound AF_PACKET capture");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || capture_fd(fd, &stop_thread));
        Self { stop, handle: Some(handle), port }
    }

    fn start_in_netns(netns: &str, iface: &str) -> (Self, u32) {
        let namespace = File::open(Path::new("/var/run/netns").join(netns))
            .expect("open allocation network namespace");
        let iface = std::ffi::CString::new(iface).expect("tap name has no NUL");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let (ready, readiness) = std::sync::mpsc::sync_channel(1);
        let handle = std::thread::spawn(move || {
            // SAFETY: this dedicated capture thread has not opened sockets or
            // spawned children. `setns` changes only this thread's network
            // namespace, and the namespace fd remains live for the call.
            let switched = unsafe { libc::setns(namespace.as_raw_fd(), libc::CLONE_NEWNET) };
            assert_eq!(
                switched,
                0,
                "enter allocation netns for tap capture: {}",
                std::io::Error::last_os_error()
            );
            // SAFETY: the NUL-terminated name remains live for this lookup.
            let ifindex = unsafe { libc::if_nametoindex(iface.as_ptr()) };
            assert_ne!(ifindex, 0, "resolve tap ifindex inside allocation netns");
            let fd = open_bound_packet_socket(ifindex).expect("bind exact tap capture");
            ready.send(ifindex).expect("report tap capture-ready");
            capture_fd(fd, &stop_thread)
        });
        let ifindex = readiness.recv().expect("tap capture thread reports readiness");
        (Self { stop, handle: Some(handle), port: 0 }, ifindex)
    }

    fn stop(mut self) -> CaptureBatch {
        self.stop.store(true, Ordering::SeqCst);
        let capture =
            self.handle.take().expect("wire capture thread").join().expect("wire capture join");
        assert!(
            capture_statistics_are_lossless(capture.statistics).is_ok(),
            "lossless AF_PACKET capture requires PACKET_STATISTICS.tp_drops == 0; stats={:?}",
            capture.statistics,
        );
        capture
    }

    fn stop_and_scan(self, exact_tuple: Option<FlowTuple>) -> WireScan {
        let port = self.port;
        let capture = self.stop();
        let mut scan = scan_frames(&capture.frames, port, exact_tuple);
        scan.capture_packets = capture.statistics.packets;
        scan.capture_drops = capture.statistics.drops;
        scan
    }
}

impl Drop for WireCapture {
    fn drop(&mut self) {
        // Panic-safe fixture cleanup: a failed oracle must not detach a
        // forever-running AF_PACKET thread and turn the useful assertion into
        // nextest's later process timeout.
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn capture_statistics_are_lossless(statistics: PacketStatistics) -> Result<(), String> {
    if statistics.drops == 0 {
        Ok(())
    } else {
        Err(format!("AF_PACKET reported {} dropped packets", statistics.drops))
    }
}

fn open_bound_packet_socket(ifindex: u32) -> std::io::Result<std::os::fd::RawFd> {
    // SAFETY: create and bind one AF_PACKET socket. An ifindex of zero is the
    // documented all-interface binding used by `start_all`.
    let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_DGRAM, ETH_P_ALL.to_be()) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let configure = || -> std::io::Result<()> {
        for (level, option) in
            [(libc::SOL_SOCKET, libc::SO_TIMESTAMPNS), (libc::SOL_PACKET, PACKET_AUXDATA)]
        {
            let enabled: libc::c_int = 1;
            // SAFETY: `fd` is live and the option points to one integer.
            let result = unsafe {
                libc::setsockopt(
                    fd,
                    level,
                    option,
                    std::ptr::from_ref(&enabled).cast(),
                    libc::socklen_t::try_from(std::mem::size_of_val(&enabled))
                        .expect("packet option length fits socklen_t"),
                )
            };
            if result != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        addr.sll_family = libc::AF_PACKET as u16;
        addr.sll_protocol = (ETH_P_ALL as u16).to_be();
        addr.sll_ifindex = i32::try_from(ifindex).expect("AF_PACKET ifindex fits i32");
        // SAFETY: the live sockaddr has the exact supplied size.
        if unsafe {
            libc::bind(
                fd,
                std::ptr::from_ref(&addr).cast(),
                std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: query and set flags on the live capture fd.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    };
    if let Err(error) = configure() {
        // SAFETY: close the fd exactly once on configuration failure.
        unsafe { libc::close(fd) };
        return Err(error);
    }
    Ok(fd)
}

fn capture_fd(fd: std::os::fd::RawFd, stop: &AtomicBool) -> CaptureBatch {
    let mut frames = Vec::new();
    let mut buf = vec![0_u8; 65_535];
    while !stop.load(Ordering::SeqCst) {
        match receive_captured_frame(fd, &mut buf) {
            Ok(Some(frame)) => frames.push(frame),
            Ok(None) => std::thread::sleep(Duration::from_micros(200)),
            Err(error) if capture_interface_was_removed(&error) => break,
            Err(error) => panic!("AF_PACKET receive failed: {error}"),
        }
    }
    loop {
        match receive_captured_frame(fd, &mut buf) {
            Ok(Some(frame)) => frames.push(frame),
            Ok(None) => break,
            Err(error) if capture_interface_was_removed(&error) => break,
            Err(error) => panic!("AF_PACKET final drain failed: {error}"),
        }
    }
    let statistics = packet_statistics(fd).expect("read PACKET_STATISTICS");
    // SAFETY: close exactly the fd created for this capture.
    unsafe { libc::close(fd) };
    CaptureBatch { frames, statistics }
}

fn capture_interface_was_removed(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::ENETDOWN | libc::ENODEV | libc::ENXIO))
}

fn packet_statistics(fd: std::os::fd::RawFd) -> std::io::Result<PacketStatistics> {
    let mut statistics = PacketStatistics { packets: 0, drops: 0 };
    let mut len = libc::socklen_t::try_from(std::mem::size_of::<PacketStatistics>())
        .expect("packet statistics size fits socklen_t");
    // SAFETY: `statistics` and `len` are writable storage of the declared size.
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_PACKET,
            PACKET_STATISTICS,
            std::ptr::from_mut(&mut statistics).cast(),
            std::ptr::from_mut(&mut len),
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if len as usize != std::mem::size_of::<PacketStatistics>() {
        return Err(std::io::Error::other("partial PACKET_STATISTICS response"));
    }
    Ok(statistics)
}

fn receive_captured_frame(
    fd: std::os::fd::RawFd,
    buf: &mut [u8],
) -> std::io::Result<Option<CapturedFrame>> {
    let mut packet_addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    let mut iov = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
    // Native cmsghdr storage supplies both capacity and alignment for one
    // `SCM_TIMESTAMPNS` record.
    let mut control: [libc::cmsghdr; 8] = unsafe { std::mem::zeroed() };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_name = std::ptr::from_mut(&mut packet_addr).cast();
    message.msg_namelen = libc::socklen_t::try_from(std::mem::size_of_val(&packet_addr))
        .expect("packet address length fits socklen_t");
    message.msg_iov = std::ptr::from_mut(&mut iov);
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = std::mem::size_of_val(&control);
    // SAFETY: every pointer in `message` references live owned storage for the
    // duration of this nonblocking receive.
    let n = unsafe { libc::recvmsg(fd, &raw mut message, libc::MSG_TRUNC) };
    if n < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(None);
        }
        return Err(error);
    }
    if n == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "AF_PACKET returned EOF",
        ));
    }
    let kernel_event_at = if message.msg_flags & libc::MSG_CTRUNC == 0 {
        // SAFETY: the kernel populated the cmsghdr chain inside `control`; the
        // decoder validates record identity and length before reading it.
        unsafe { kernel_timestamp_from_message(&message) }
    } else {
        None
    };
    let aux = if message.msg_flags & libc::MSG_CTRUNC == 0 {
        // SAFETY: the message's ancillary chain remains live for this call.
        unsafe { packet_auxdata_from_message(&message) }
    } else {
        None
    };
    let wire_len = usize::try_from(n).expect("recvmsg length is non-negative");
    let copied = wire_len.min(buf.len());
    Ok(Some(CapturedFrame {
        kernel_event_at,
        ifindex: u32::try_from(packet_addr.sll_ifindex).expect("packet ifindex is non-negative"),
        protocol: u16::from_be(packet_addr.sll_protocol),
        packet_type: packet_addr.sll_pkttype,
        wire_len,
        truncated: wire_len > buf.len() || message.msg_flags & libc::MSG_TRUNC != 0,
        control_truncated: message.msg_flags & libc::MSG_CTRUNC != 0,
        aux,
        bytes: buf[..copied].to_vec(),
    }))
}

unsafe fn packet_auxdata_from_message(message: &libc::msghdr) -> Option<PacketAuxData> {
    // SAFETY: the caller keeps the kernel-populated control buffer live.
    let mut header = unsafe { libc::CMSG_FIRSTHDR(message) };
    while !header.is_null() {
        // SAFETY: yielded CMSG headers are valid within the live control chain.
        let current = unsafe { &*header };
        let expected_len = unsafe {
            libc::CMSG_LEN(
                u32::try_from(std::mem::size_of::<PacketAuxData>())
                    .expect("auxdata control length fits u32"),
            )
        } as usize;
        if current.cmsg_level == libc::SOL_PACKET
            && current.cmsg_type == PACKET_AUXDATA
            && current.cmsg_len >= expected_len
        {
            // SAFETY: the validated record contains one complete auxdata value.
            return Some(unsafe {
                std::ptr::read_unaligned(libc::CMSG_DATA(header).cast::<PacketAuxData>())
            });
        }
        // SAFETY: advance within the same live control chain.
        header = unsafe { libc::CMSG_NXTHDR(message, header) };
    }
    None
}

unsafe fn kernel_timestamp_from_message(message: &libc::msghdr) -> Option<KernelRealtime> {
    // SAFETY: the caller keeps the kernel-populated control buffer alive for
    // this entire CMSG traversal.
    let mut header = unsafe { libc::CMSG_FIRSTHDR(message) };
    while !header.is_null() {
        // SAFETY: non-null headers are yielded by the libc CMSG helpers.
        let current = unsafe { &*header };
        let expected_len = unsafe {
            libc::CMSG_LEN(
                u32::try_from(std::mem::size_of::<libc::timespec>())
                    .expect("timespec control length fits u32"),
            )
        } as usize;
        if current.cmsg_level == libc::SOL_SOCKET
            && current.cmsg_type == libc::SCM_TIMESTAMPNS
            && current.cmsg_len >= expected_len
        {
            // SAFETY: the validated record contains a complete timespec.
            let timestamp = unsafe {
                std::ptr::read_unaligned(libc::CMSG_DATA(header).cast::<libc::timespec>())
            };
            return KernelRealtime::from_timespec(timestamp);
        }
        // SAFETY: advance within the same live, kernel-populated chain.
        header = unsafe { libc::CMSG_NXTHDR(message, header) };
    }
    None
}

impl KernelRealtime {
    fn from_timespec(timestamp: libc::timespec) -> Option<Self> {
        const NANOS_PER_SECOND: i128 = 1_000_000_000;
        if timestamp.tv_sec < 0 || !(0..1_000_000_000).contains(&timestamp.tv_nsec) {
            return None;
        }
        i128::from(timestamp.tv_sec)
            .checked_mul(NANOS_PER_SECOND)
            .and_then(|seconds| seconds.checked_add(i128::from(timestamp.tv_nsec)))
            .map(Self)
    }

    fn now() -> Self {
        let mut timestamp: libc::timespec = unsafe { std::mem::zeroed() };
        // SAFETY: `timestamp` points to writable storage for one timespec.
        let result = unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &raw mut timestamp) };
        assert_eq!(result, 0, "read CLOCK_REALTIME: {}", std::io::Error::last_os_error());
        Self::from_timespec(timestamp).expect("CLOCK_REALTIME yields a valid non-negative timespec")
    }
}

fn scan_frames(
    frames: &[CapturedFrame],
    peer_port: u16,
    exact_tuple: Option<FlowTuple>,
) -> WireScan {
    #[derive(Default)]
    struct DirectionCapture {
        syn_sequence: Option<u32>,
        pieces: Vec<(u32, bool, Vec<u8>, u8)>,
    }

    let mut captures: BTreeMap<FlowTuple, DirectionCapture> = BTreeMap::new();
    for frame in frames {
        let Some(segment) =
            parse_tcp_segment(frame, false).unwrap_or_else(|error| {
                panic!("peer-wire capture contains a truncated/malformed/ambiguous L3 frame: {error}; frame={frame:?}")
            })
        else {
            continue;
        };
        if segment.tuple.source.port() != peer_port && segment.tuple.destination.port() != peer_port
        {
            continue;
        }
        let packet_type_bit = match frame.packet_type {
            libc::PACKET_HOST => 0b01,
            libc::PACKET_OUTGOING => 0b10,
            other => panic!(
                "peer loopback stream has an unexpected AF_PACKET packet_type={other}; frame={frame:?}"
            ),
        };
        let capture = captures.entry(segment.tuple).or_default();
        let syn = segment.flags & 0x02 != 0;
        if syn {
            match capture.syn_sequence {
                Some(sequence) => assert_eq!(
                    sequence, segment.sequence,
                    "one directional peer stream cannot carry competing SYN sequence anchors"
                ),
                None => capture.syn_sequence = Some(segment.sequence),
            }
        }
        capture.pieces.push((segment.sequence, syn, segment.payload.to_vec(), packet_type_bit));
    }

    let mut streams = BTreeMap::new();
    for (tuple, capture) in captures {
        let Some(syn_sequence) = capture.syn_sequence else {
            if capture.pieces.iter().all(|(_, _, payload, _)| payload.is_empty()) {
                // A bare loopback RST/ACK (typically the kernel rejecting a
                // readiness probe) has no application byte stream to scan.
                continue;
            }
            panic!(
                "peer stream {tuple:?} has payload observations without a captured SYN; pieces={:?}",
                capture.pieces
            );
        };
        let first_payload_sequence = syn_sequence.wrapping_add(1);
        let mut bytes_by_offset = BTreeMap::<u32, u8>::new();
        let mut logical_copies = BTreeMap::<(u32, bool, Vec<u8>), u8>::new();
        for (sequence, syn, payload, packet_type_bit) in capture.pieces {
            let logical = (sequence, syn, payload.clone());
            *logical_copies.entry(logical).or_default() |= packet_type_bit;
            if payload.is_empty() {
                continue;
            }
            let payload_sequence = sequence.wrapping_add(u32::from(syn));
            let start = payload_sequence.wrapping_sub(first_payload_sequence);
            assert!(
                start < (1_u32 << 31),
                "peer stream {tuple:?} has ambiguous TCP sequence ordering around wrap: syn={syn_sequence} payload={payload_sequence}"
            );
            for (index, byte) in payload.into_iter().enumerate() {
                let index = u32::try_from(index).expect("one captured TCP payload fits u32");
                let offset = start.checked_add(index).unwrap_or_else(|| {
                    panic!("peer stream {tuple:?} sequence space overflows its half-window")
                });
                assert!(
                    offset < (1_u32 << 31),
                    "peer stream {tuple:?} exceeds the unambiguous TCP sequence half-window"
                );
                if let Some(previous) = bytes_by_offset.insert(offset, byte) {
                    assert_eq!(
                        previous, byte,
                        "peer stream {tuple:?} has conflicting retransmitted/loopback-copy bytes at offset {offset}"
                    );
                }
            }
        }
        assert!(
            logical_copies.values().all(|copy_types| *copy_types & !0b11 == 0),
            "peer stream {tuple:?} retained an unclassified AF_PACKET copy"
        );
        let mut stream = Vec::with_capacity(bytes_by_offset.len());
        for (expected, (observed, byte)) in (0_u32..).zip(bytes_by_offset) {
            assert_eq!(
                observed, expected,
                "peer stream {tuple:?} has a capture/reassembly gap before TCP offset {observed}"
            );
            stream.push(byte);
        }
        streams.insert(tuple, stream);
    }

    let mut scan =
        WireScan { exact_tuple, peer_streams_observed: streams.len(), ..WireScan::default() };
    for bytes in streams.values() {
        // Confidentiality is checked across every byte stream touching the
        // peer port. It is intentionally independent of whether the stream can
        // first be parsed as TLS, so a clear or malformed escape cannot hide
        // outside the positive TLS classifier.
        scan.plaintext_hits_on_any_peer_stream += count_subslices(bytes, REQUEST);
        scan.plaintext_hits_on_any_peer_stream += count_subslices(bytes, RESPONSE);
    }
    if let Some(tuple) = exact_tuple {
        scan.exact_records_to_peer =
            streams.get(&tuple).map_or(0, |bytes| count_tls_application_records(bytes));
        scan.exact_records_from_peer =
            streams.get(&tuple.reverse()).map_or(0, |bytes| count_tls_application_records(bytes));
    }
    scan
}

fn parse_tcp_segment(
    frame: &CapturedFrame,
    reject_offload_ambiguity: bool,
) -> Result<Option<ParsedTcpSegment<'_>>, String> {
    if frame.truncated || frame.control_truncated || frame.bytes.len() != frame.wire_len {
        return Err("MSG_TRUNC/MSG_CTRUNC or wire-length mismatch".to_owned());
    }
    let Some(aux) = frame.aux else {
        return Err("PACKET_AUXDATA is missing".to_owned());
    };
    if usize::try_from(aux.len).ok() != Some(frame.wire_len)
        || usize::try_from(aux.snaplen).ok() != Some(frame.bytes.len())
    {
        return Err(format!("PACKET_AUXDATA length mismatch: {aux:?}"));
    }
    if reject_offload_ambiguity && aux.status & TP_STATUS_CSUMNOTREADY != 0 {
        return Err(format!("checksum/offload state is not finalized: {aux:?}"));
    }
    if frame.protocol != ETH_P_IP {
        return Ok(None);
    }
    if frame.bytes.len() < IPV4_HEADER_LEN {
        return Err("IPv4 packet is shorter than its fixed header".to_owned());
    }
    let bytes = &frame.bytes;
    let ihl = usize::from(bytes[0] & 0x0f) * 4;
    if bytes[0] >> 4 != 4 || ihl < IPV4_HEADER_LEN || ihl > bytes.len() {
        return Err("invalid IPv4 version or IHL".to_owned());
    }
    let total_len = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
    if total_len != bytes.len() || total_len != frame.wire_len || total_len < ihl {
        return Err(format!(
            "IPv4 tot_len must equal the complete skb length: tot_len={total_len} wire_len={} copied={}",
            frame.wire_len,
            bytes.len()
        ));
    }
    let fragment = u16::from_be_bytes([bytes[6], bytes[7]]);
    if fragment & 0x3fff != 0 {
        return Err(format!("fragmented IPv4 is ineligible and ambiguous: {fragment:#06x}"));
    }
    if bytes[9] != 0x06 {
        return Ok(None);
    }
    let tcp = ihl;
    if bytes.len() < tcp + 20 {
        return Err("TCP segment is shorter than its fixed header".to_owned());
    }
    let source = u16::from_be_bytes([bytes[tcp], bytes[tcp + 1]]);
    let destination = u16::from_be_bytes([bytes[tcp + 2], bytes[tcp + 3]]);
    let source_addr = Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]);
    let destination_addr = Ipv4Addr::new(bytes[16], bytes[17], bytes[18], bytes[19]);
    let tcp_header = usize::from(bytes[tcp + 12] >> 4) * 4;
    let payload = tcp + tcp_header;
    if tcp_header < 20 || payload > total_len {
        return Err("invalid TCP data offset".to_owned());
    }
    Ok(Some(ParsedTcpSegment {
        tuple: FlowTuple {
            source: SocketAddrV4::new(source_addr, source),
            destination: SocketAddrV4::new(destination_addr, destination),
        },
        sequence: u32::from_be_bytes([
            bytes[tcp + 4],
            bytes[tcp + 5],
            bytes[tcp + 6],
            bytes[tcp + 7],
        ]),
        flags: bytes[tcp + 13],
        payload: &bytes[payload..total_len],
        total_len,
    }))
}

fn outbound_rule_snapshot(host_veth: &str) -> Result<Option<nft::RuleInfo>, String> {
    let rules = nft::list_rules("overdrive-mtls", "prerouting")
        .map_err(|error| format!("strict nft rule observation failed: {error}"))?;
    outbound_rule_from_rules(&rules, host_veth)
}

fn outbound_rule_from_rules(
    rules: &[nft::RuleInfo],
    host_veth: &str,
) -> Result<Option<nft::RuleInfo>, String> {
    let tagged = d7_allocation_tagged_rules(rules, host_veth);
    match tagged.as_slice() {
        [] => Ok(None),
        [_] => exact_d7_target(rules, host_veth).map(Some),
        _ => Err(format!(
            "ambiguous D7 ownership for {host_veth}: {} allocation-tagged rules",
            tagged.len()
        )),
    }
}

async fn poll_until_outbound_rule_snapshot(host_veth: &str) -> nft::RuleInfo {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut last_error = None;
    loop {
        match outbound_rule_snapshot(host_veth) {
            Ok(Some(rule)) => return rule,
            Ok(None) => {}
            Err(error) => last_error = Some(error),
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the independent sibling's exact intercept rule must become observable; last strict observation error: {last_error:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn d7_allocation_tagged_rules<'a>(
    rules: &'a [nft::RuleInfo],
    host_veth: &str,
) -> Vec<&'a nft::RuleInfo> {
    rules
        .iter()
        .filter(|rule| {
            rule.userdata.starts_with(nft::USERDATA_MAGIC)
                && rule.userdata.get(nft::USERDATA_MAGIC.len()) == Some(&0x03)
                && rule.userdata.ends_with(host_veth.as_bytes())
        })
        .collect()
}

fn exact_d7_target(rules: &[nft::RuleInfo], host_veth: &str) -> Result<nft::RuleInfo, String> {
    let prefix_len = nft::USERDATA_MAGIC.len() + 1;
    let expected_len = prefix_len + 2 + host_veth.len();
    let matching = d7_allocation_tagged_rules(rules, host_veth)
        .into_iter()
        .filter(|rule| {
            rule.userdata.len() == expected_len
                && rule.userdata[prefix_len + 2..] == *host_veth.as_bytes()
        })
        .collect::<Vec<_>>();
    let [rule] = matching.as_slice() else {
        return Err(format!(
            "expected exactly one D7 target for {host_veth}, got {}",
            matching.len()
        ));
    };
    let agent_port = u16::from_be_bytes([rule.userdata[prefix_len], rule.userdata[prefix_len + 1]]);
    let expected_program = nft::normalized_rule_program_identity(&nft::egress_tproxy_rule_exprs(
        host_veth,
        Ipv4Addr::LOCALHOST,
        agent_port,
        0x1,
    ))
    .map_err(|error| format!("production D7 encoder did not normalize: {error}"))?;
    if rule.normalized_program != expected_program {
        return Err("D7 target normalized program differs from the production encoder".to_owned());
    }
    if rule.counter.is_none() {
        return Err("D7 target is missing its one typed anonymous counter".to_owned());
    }
    Ok((*rule).clone())
}

fn validate_stable_d7_pair(
    pair: &[nft::RuleSnapshot; 2],
    host_veth: &str,
    expected_userdata: Option<&[u8]>,
) -> Result<nft::RuleInfo, String> {
    if pair[0].generation == 0 || pair[0].generation != pair[1].generation {
        return Err("D7 pair generation is zero or changed".to_owned());
    }
    let first = exact_d7_target(&pair[0].rules, host_veth)?;
    let second = exact_d7_target(&pair[1].rules, host_veth)?;
    if first != second {
        return Err(
            "D7 target handle/program/userdata/counter changed during quiet interval".to_owned()
        );
    }
    if expected_userdata.is_some_and(|expected| first.userdata != expected) {
        return Err("D7 target userdata was replaced".to_owned());
    }
    Ok(first)
}

async fn poll_until_outbound_rule_ready(
    host_veth: String,
    budget: Duration,
) -> LiveInterceptReadiness {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let discovery = nft::list_rules("overdrive-mtls", "prerouting")
            .ok()
            .and_then(|rules| exact_d7_target(&rules, &host_veth).ok());
        if let Some(discovered) = discovery {
            let mut observer = nft::NftRuleObserver::subscribe()
                .expect("subscribe to the loss-reporting nftables notification group");
            let first = observer
                .snapshot("overdrive-mtls", "prerouting")
                .expect("strict first generation-bracketed GETRULE snapshot");
            let second = observer
                .snapshot("overdrive-mtls", "prerouting")
                .expect("strict second generation-bracketed GETRULE snapshot");
            observer
                .ensure_no_notifications()
                .expect("no nft mutation notification during baseline");
            let before = [first, second];
            let stable = validate_stable_d7_pair(&before, &host_veth, Some(&discovered.userdata))
                .expect("the D7 target is exact and stable before the observation cut");
            let iface = std::ffi::CString::new(host_veth.as_str()).expect("iface has no NUL");
            // SAFETY: libc retains no pointer; `iface` is NUL-terminated.
            let host_veth_ifindex = unsafe { libc::if_nametoindex(iface.as_ptr()) };
            assert!(host_veth_ifindex != 0, "ready nft rule names a live host-veth");
            return LiveInterceptReadiness {
                // Deliberately sampled after both the successful typed nft
                // query and exact-ifindex resolution. This later barrier is
                // conservative: every kernel packet timestamp at or before it
                // is classified pre-ready.
                kernel_barrier_at: KernelRealtime::now(),
                host_veth_ifindex,
                host_veth,
                before,
                target_userdata: stable.userdata,
                observer,
            };
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the exact outbound nft rule for {host_veth} must become observable within {budget:?}"
        );
        tokio::task::yield_now().await;
    }
}

async fn poll_until_nft_rule_observer_is_quiet(budget: Duration, minimum_allocation_rules: usize) {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let quiet = nft::NftRuleObserver::subscribe().ok().and_then(|mut observer| {
            let first = observer.snapshot("overdrive-mtls", "prerouting").ok()?;
            let second = observer.snapshot("overdrive-mtls", "prerouting").ok()?;
            let allocation_rules = second
                .rules
                .iter()
                .filter(|rule| {
                    rule.userdata.starts_with(nft::USERDATA_MAGIC)
                        && matches!(rule.userdata.get(nft::USERDATA_MAGIC.len()), Some(0x01 | 0x03))
                })
                .count();
            if allocation_rules >= minimum_allocation_rules
                && first.generation == second.generation
                && first.rules == second.rules
                && observer.ensure_no_notifications().is_ok()
            {
                Some(())
            } else {
                None
            }
        });
        if quiet.is_some() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "mTLS reconstruction must expose at least {minimum_allocation_rules} allocation rules and reach a notification-free ruleset within {budget:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn finish_d7_observation(
    mut readiness: LiveInterceptReadiness,
    budget: Duration,
) -> InterceptReadiness {
    let host_veth = readiness.host_veth.clone();
    let before =
        validate_stable_d7_pair(&readiness.before, &host_veth, Some(&readiness.target_userdata))
            .expect("revalidate exact D7 baseline");
    let before_counter = before.counter.expect("validated counter");
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let first = readiness
            .observer
            .snapshot("overdrive-mtls", "prerouting")
            .expect("strict post-flow generation-bracketed GETRULE snapshot");
        let first_target = exact_d7_target(&first.rules, &host_veth)
            .expect("post-flow snapshot retains the exact D7 target");
        let first_counter = first_target.counter.expect("validated post-flow counter");
        if first_counter.packets > before_counter.packets
            && first_counter.bytes > before_counter.bytes
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let second = readiness
                .observer
                .snapshot("overdrive-mtls", "prerouting")
                .expect("strict final generation-bracketed GETRULE snapshot");
            let after = [first, second];
            if validate_stable_d7_pair(&after, &host_veth, Some(&readiness.target_userdata))
                .is_err()
            {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the advanced D7 target must reach a stable quiet pair within {budget:?}"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
            readiness
                .observer
                .ensure_no_notifications()
                .expect("final nft notification drain is empty");
            let generations = [
                readiness.before[0].generation,
                readiness.before[1].generation,
                after[0].generation,
                after[1].generation,
            ];
            assert!(
                generations.iter().all(|generation| *generation == generations[0]),
                "every D7 bracket must retain one full non-zero generation: {generations:?}"
            );
            return InterceptReadiness {
                kernel_barrier_at: readiness.kernel_barrier_at,
                host_veth_ifindex: readiness.host_veth_ifindex,
                host_veth: readiness.host_veth,
                accounting: D7Accounting {
                    before: readiness.before,
                    after,
                    target_userdata: readiness.target_userdata,
                },
            };
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the exact production D7 counter must advance within {budget:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn audit_guest_egress_boundary(
    frames: &[CapturedFrame],
    readiness: &InterceptReadiness,
    guest_addr: Ipv4Addr,
    mesh_destination: SocketAddrV4,
) -> GuestEgressAudit {
    let mut segments = Vec::new();
    let mut interface_tcp = Vec::new();
    let mut first_syn = None;
    let mut plaintext_request_hits = 0;
    let mut packet_count = 0_u64;
    let mut byte_count = 0_u64;
    let mut exact_tuple = None;
    for frame in frames.iter().filter(|frame| {
        frame.ifindex == readiness.host_veth_ifindex && frame.packet_type != libc::PACKET_OUTGOING
    }) {
        let Some(parsed) =
            parse_tcp_segment(frame, true).unwrap_or_else(|error| {
                panic!("guest C3 capture is malformed/truncated/fragmented/offload-ambiguous: {error}; frame={frame:?}")
            })
        else {
            continue;
        };
        let tuple = parsed.tuple;
        let flags = parsed.flags;
        let payload = parsed.payload;
        let ipv4_total_len = parsed.total_len;
        let segment = GuestBoundarySegment {
            kernel_event_at: frame.kernel_event_at,
            tuple,
            flags,
            payload_len: payload.len(),
            ipv4_total_len,
        };
        interface_tcp.push(segment);
        assert_eq!(
            tuple.source.ip(),
            &guest_addr,
            "every eligible host-veth ingress TCP packet is guest-authored"
        );
        assert_eq!(
            tuple.destination, mesh_destination,
            "every eligible packet belongs to the expected mesh destination"
        );
        if first_syn.is_none() && flags & 0x02 != 0 && flags & 0x10 == 0 {
            first_syn = Some(segment);
            exact_tuple = Some(tuple);
        }
        if let Some(expected) = exact_tuple {
            assert_eq!(
                tuple, expected,
                "every eligible D7 packet has the first SYN's exact directional tuple"
            );
        }
        plaintext_request_hits += count_subslices(payload, REQUEST);
        packet_count =
            packet_count.checked_add(1).expect("captured packet count does not overflow");
        byte_count = byte_count
            .checked_add(u64::try_from(ipv4_total_len).expect("IPv4 tot_len fits u64"))
            .expect("captured IPv4 byte count does not overflow");
        segments.push(segment);
    }
    GuestEgressAudit {
        segments,
        interface_tcp,
        first_syn,
        plaintext_request_hits,
        packet_count,
        byte_count,
    }
}

fn guest_frame_precedes_capture_ready(
    frame: &CapturedFrame,
    host_veth_ifindex: u32,
    barrier: KernelRealtime,
) -> bool {
    frame.ifindex == host_veth_ifindex
        && frame.packet_type != libc::PACKET_OUTGOING
        && frame.kernel_event_at.is_none_or(|event_at| event_at <= barrier)
}

fn validate_exact_d7_accounting(
    readiness: &InterceptReadiness,
    audit: &GuestEgressAudit,
    host_veth: &str,
) -> Result<(), String> {
    let before = validate_stable_d7_pair(
        &readiness.accounting.before,
        host_veth,
        Some(&readiness.accounting.target_userdata),
    )?;
    let after = validate_stable_d7_pair(
        &readiness.accounting.after,
        host_veth,
        Some(&readiness.accounting.target_userdata),
    )?;
    let before_counter = before.counter.ok_or_else(|| "baseline counter missing".to_owned())?;
    let after_counter = after.counter.ok_or_else(|| "after counter missing".to_owned())?;
    let mut before_identity = before;
    let mut after_identity = after;
    before_identity.counter = None;
    after_identity.counter = None;
    if before_identity != after_identity {
        return Err("D7 handle/userdata/full normalized program was replaced or mutated".to_owned());
    }
    let packet_delta = after_counter
        .packets
        .checked_sub(before_counter.packets)
        .ok_or_else(|| "D7 packet counter reset/regressed/wrapped".to_owned())?;
    let byte_delta = after_counter
        .bytes
        .checked_sub(before_counter.bytes)
        .ok_or_else(|| "D7 byte counter reset/regressed/wrapped".to_owned())?;
    if packet_delta == 0 || byte_delta == 0 {
        return Err("D7 packet and byte deltas must both be non-zero".to_owned());
    }
    if packet_delta != audit.packet_count || byte_delta != audit.byte_count {
        return Err(format!(
            "D7 exact accounting mismatch: counter=({packet_delta},{byte_delta}) capture=({},{})",
            audit.packet_count, audit.byte_count
        ));
    }
    if before_counter.packets.checked_add(packet_delta) != Some(after_counter.packets)
        || before_counter.bytes.checked_add(byte_delta) != Some(after_counter.bytes)
    {
        return Err("D7 checked counter addition failed".to_owned());
    }
    Ok(())
}

fn count_tls_application_records(stream: &[u8]) -> u64 {
    let mut count = 0_u64;
    let mut cursor = 0_usize;
    while cursor + TLS_RECORD_HEADER_LEN <= stream.len() {
        let version = [stream[cursor + 1], stream[cursor + 2]];
        if version != [0x03, 0x03] && version != [0x03, 0x01] {
            break;
        }
        let len = usize::from(u16::from_be_bytes([stream[cursor + 3], stream[cursor + 4]]));
        if stream[cursor] == TLS_APPLICATION_DATA {
            count += 1;
        }
        let next = cursor + TLS_RECORD_HEADER_LEN + len;
        if next <= cursor || next > stream.len() {
            break;
        }
        cursor = next;
    }
    count
}

fn count_subslices(haystack: &[u8], needle: &[u8]) -> u64 {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    haystack.windows(needle.len()).filter(|window| *window == needle).count() as u64
}

fn ktls_socket_records() -> Vec<KtlsSocketEvidence> {
    let output = Command::new("ss").args(["-H", "-n", "-t", "-i", "-e"]).output().expect("run ss");
    assert!(output.status.success(), "ss failed: {}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8_lossy(&output.stdout);
    let mut records = Vec::new();
    let mut current: Option<KtlsSocketEvidence> = None;
    for line in text.lines() {
        if line.starts_with(char::is_whitespace) {
            if let Some(record) = current.as_mut() {
                record.record.push_str(line);
                record.record.push('\n');
            }
            continue;
        }
        if let Some(record) = current.take() {
            records.push(record);
        }
        let columns = line.split_whitespace().collect::<Vec<_>>();
        let Some((source, destination)) =
            columns.get(3).zip(columns.get(4)).and_then(|(source, destination)| {
                let source = source.parse::<SocketAddr>().ok()?;
                let destination = destination.parse::<SocketAddr>().ok()?;
                match (source, destination) {
                    (SocketAddr::V4(source), SocketAddr::V4(destination)) => {
                        Some((source, destination))
                    }
                    _ => None,
                }
            })
        else {
            continue;
        };
        current = Some(KtlsSocketEvidence {
            tuple: FlowTuple { source, destination },
            record: format!("{line}\n"),
        });
    }
    if let Some(record) = current {
        records.push(record);
    }
    records
}

fn record_has_bidirectional_tls13_ktls(record: &str) -> bool {
    record.contains("tcp-ulp-tls")
        && (record.contains("version: 1.3") || record.contains("version:1.3"))
        && (record.contains("rxconf: sw") || record.contains("rxconf:sw"))
        && (record.contains("txconf: sw") || record.contains("txconf:sw"))
}

async fn poll_until_ktls(port: u16, budget: Duration) -> Option<KtlsSocketEvidence> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if let Some(record) = ktls_socket_records().into_iter().find(|record| {
            record.tuple.destination.port() == port
                && record_has_bidirectional_tls13_ktls(&record.record)
        }) {
            return Some(record);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn without_permitted_lifecycle_delta(row: &AllocStatusRowBody) -> AllocStatusRowBody {
    // Start from the COMPLETE observable row universe and normalize only the
    // six explicitly-permitted lifecycle/ephemeral fields. Every other field
    // remains in the value compared by `assert_exact_lifecycle_delta`.
    let mut complement = row.clone();
    complement.state = AllocStateWire::Pending;
    complement.reason = None;
    complement.workload_addr = None;
    complement.started_at = None;
    complement.exit_code = None;
    complement.last_transition = None;
    complement
}

fn assert_exact_lifecycle_delta(running: &AllocStatusRowBody, terminal: &AllocStatusRowBody) {
    // Exact permitted delta over the complete row:
    // state Running -> Terminated; the live backend address is retired;
    // reason/last_transition are replaced; the logical observation timestamp
    // is restamped. No other row field may change.
    assert_eq!(running.state, AllocStateWire::Running);
    assert_eq!(terminal.state, AllocStateWire::Terminated);
    assert_ne!(
        running.reason, terminal.reason,
        "the Running transition reason is replaced by the terminal stop reason"
    );
    assert!(running.workload_addr.is_some(), "Running VM carries its live guest address");
    assert_eq!(terminal.workload_addr, None, "terminal VM is no longer a live backend");
    assert!(running.started_at.is_some(), "Running row is timestamped");
    assert!(terminal.started_at.is_some(), "terminal row is timestamped");
    assert_ne!(running.started_at, terminal.started_at, "the lifecycle observation is restamped");
    assert_eq!(running.exit_code, None, "a running allocation has no exit result");
    assert_eq!(terminal.exit_code, Some(0), "a cleanly completed Job exposes exit code zero");
    assert_ne!(
        running.last_transition, terminal.last_transition,
        "the Running transition record is replaced by the terminal transition"
    );
    assert_eq!(
        without_permitted_lifecycle_delta(terminal),
        without_permitted_lifecycle_delta(running),
        "every observable row field outside the six permitted lifecycle deltas must be equal"
    );
}

async fn genuine_pre_change_contract(
    server_dir: &Path,
    submit: &DeployOutput,
    expected_guest_addr: Ipv4Addr,
) -> WorkloadDescribeOutput {
    // Take an instantaneous CoW snapshot of the production observation DB.
    // The resulting handle is independent of the HTTP response under test and
    // cannot inherit a collateral API projection change from that response.
    let observation_db = server_dir.join("data").join("observation.redb");
    let observation_snapshot = server_dir.join("observation-pre-change-contract.redb");
    let copied = Command::new("cp")
        .arg("--reflink=always")
        .arg(&observation_db)
        .arg(&observation_snapshot)
        .status()
        .expect("snapshot the production observation database");
    assert!(copied.success(), "production observation database snapshot must succeed");

    let observations = LocalObservationStore::open(&observation_snapshot)
        .expect("open the independent observation snapshot");
    let rows = observations.alloc_status_rows().await.expect("read durable allocation rows");
    let [raw] = rows.as_slice() else {
        panic!("pre-change contract requires exactly one durable allocation row; got {rows:#?}");
    };
    assert_eq!(raw.workload_id.to_string(), submit.workload_id);
    assert_eq!(raw.state, overdrive_core::traits::observation_store::AllocState::Running);
    assert_eq!(raw.kind, WorkloadKind::Job);
    assert_eq!(
        raw.workload_addr,
        Some(expected_guest_addr),
        "the durable production fact must be the VM guest /30 address"
    );
    assert!(raw.terminal.is_none());
    assert!(raw.stderr_tail.is_none());
    assert!(raw.last_terminated.is_none());

    let logical_at = format!("(c={},w={})", raw.updated_at.counter, raw.updated_at.writer);
    let state = AllocStateWire::from(raw.state);
    let last_transition = raw.reason.clone().map(|reason| TransitionRecord {
        from: None,
        to: state,
        reason,
        source: TransitionSource::Reconciler,
        at: logical_at.clone(),
    });
    let row = AllocStatusRowBody {
        alloc_id: raw.alloc_id.to_string(),
        workload_id: raw.workload_id.to_string(),
        node_id: raw.node_id.to_string(),
        state,
        reason: raw.reason.clone(),
        resources: ResourcesBody { cpu_milli: 500, memory_bytes: 134_217_728 },
        // This is the genuine pre-step API contract: every durable input is
        // projected independently, while the one field that did not exist in
        // the pre-change response is absent.
        workload_addr: None,
        started_at: Some(logical_at),
        exit_code: None,
        last_transition,
        error: raw.detail.clone(),
        restart_count: raw.restart_count,
        last_terminated: None,
    };

    let expected_spiffe = SpiffeId::for_allocation(&raw.workload_id, &raw.alloc_id);
    let issued_rows =
        observations.issued_certificate_rows().await.expect("read durable certificate audit rows");
    let issued = issued_rows
        .iter()
        .filter(|candidate| candidate.spiffe_id == expected_spiffe)
        .max_by_key(|candidate| candidate.issuance_ordinal)
        .expect("production issuance audit contains the Running allocation identity");
    let issued_certificate = IssuedCertSummary {
        serial: issued.serial.clone(),
        spiffe_id: issued.spiffe_id.clone(),
        issuer_serial: issued.issuer_serial.clone(),
        not_after: issued.not_after,
    };

    WorkloadDescribeOutput {
        workload_id: submit.workload_id.clone(),
        spec_digest: submit.spec_digest.clone(),
        allocations_total: 1,
        empty_state_message: String::new(),
        snapshot: AllocStatusResponse {
            workload_id: Some(submit.workload_id.clone()),
            spec_digest: Some(submit.spec_digest.clone()),
            replicas_desired: 1,
            replicas_running: 1,
            rows: vec![row],
            restart_budget: Some(RestartBudget { used: 0, max: 5, exhausted: false }),
            kind: Some(WorkloadKind::Job),
            vip: None,
            listeners: Vec::new(),
            issued_certificates: vec![issued_certificate],
            probes: Vec::new(),
            probe_results: Vec::new(),
        },
    }
}

fn frozen_pre_change_render_contract(out: &WorkloadDescribeOutput) -> String {
    use std::fmt::Write as _;

    let [row] = out.snapshot.rows.as_slice() else {
        panic!("pre-change render contract requires exactly one allocation row");
    };
    let [issued] = out.snapshot.issued_certificates.as_slice() else {
        panic!("pre-change render contract requires exactly one issued certificate");
    };
    assert_eq!(out.snapshot.kind, Some(WorkloadKind::Job));
    assert_eq!(row.state, AllocStateWire::Running);
    assert_eq!(row.workload_addr, None);

    let header = format!(
        "{:<8} {:<12} {:<6} {:<20} {:<10}",
        "Attempt", "State", "Exit", "Started", "Duration"
    );
    let attempt = format!(
        "{:<8} {:<12} {:<6} {:<20} {:<10}",
        1,
        "Running",
        "\u{2014}",
        row.started_at.as_deref().expect("Running row has a logical timestamp"),
        "\u{2014}"
    );
    let mut contract = format!(
        "Job '{workload_id}' (kind: Job)\n\
         Spec digest: {spec_digest}\n\
         Verdict: In progress (no terminal yet)\n\
         \n\
         {header}\n\
         {attempt}\n\
         Memory:        {memory_bytes}\n\
         Issued certificates:\n",
        workload_id = out.workload_id,
        spec_digest = out.spec_digest,
        memory_bytes = row.resources.memory_bytes,
    );
    let _ = writeln!(contract, "  serial:        {}", issued.serial);
    let _ = writeln!(contract, "    spiffe_id:     {}", issued.spiffe_id);
    let _ = writeln!(contract, "    issuer_serial: {}", issued.issuer_serial);
    let _ = writeln!(contract, "    not_after:     {}", issued.not_after);
    contract
}

async fn poll_until_issued_identity(
    cfg: &Path,
    workload_id: &str,
    alloc_id: &str,
    budget: Duration,
) -> IssuedCertSummary {
    let expected =
        SpiffeId::new(&format!("spiffe://overdrive.local/workload/{workload_id}/alloc/{alloc_id}"))
            .expect("allocation-shaped SPIFFE ID");
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let described =
            describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_path_buf() })
                .await
                .expect("describe while waiting for production SVID audit row");
        if let Some(summary) = described
            .snapshot
            .issued_certificates
            .into_iter()
            .find(|summary| summary.spiffe_id == expected)
        {
            return summary;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "production IdentityMgr must issue and audit {expected} while the allocation is Running"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct ArmedFailureCapture {
    alloc: AllocationId,
    host_veth: String,
    netns: String,
    tap: String,
    host_wire: WireCapture,
    tap_wire: WireCapture,
    tap_ifindex: u32,
}

fn receive_vmm_cut(cuts: &Receiver<VmmSpawnCut>) -> VmmSpawnCut {
    cuts.recv_timeout(Duration::from_secs(30))
        .expect("the production VM reaches the capture-ready cut within 30s")
}

async fn wait_for_data_dir_release() {
    // An abrupt task abort drops the store owners before its future resolves;
    // retain the established bounded release bridge used by the VM
    // reclamation Tier-3 suite for filesystem-close propagation.
    tokio::time::sleep(Duration::from_millis(500)).await;
}

fn release_vmm_without_capture(cuts: &Receiver<VmmSpawnCut>) -> VmConfig {
    let cut = receive_vmm_cut(cuts);
    let config = cut.config.clone();
    cut.release.send(()).expect("release the real VMM spawn");
    config
}

fn host_veth_for_config(config: &VmConfig) -> String {
    let network = config.network.as_ref().expect("VM network attachment");
    let slot_hex = network.tap.rsplit('-').next().expect("slot-derived tap suffix");
    let slot = NetSlot::new(
        u16::from_str_radix(slot_hex, 16).expect("production tap carries a hexadecimal slot"),
    )
    .expect("observed production slot is in range");
    derive_workload_netns_plan(slot, responder_addr_for_slot(slot)).host_veth
}

fn arm_failure_capture(cuts: &Receiver<VmmSpawnCut>) -> ArmedFailureCapture {
    arm_failure_capture_from_cut(receive_vmm_cut(cuts))
}

fn arm_failure_capture_from_cut(cut: VmmSpawnCut) -> ArmedFailureCapture {
    let capture = arm_failure_capture_from_config(&cut.config);
    cut.release.send(()).expect("release the real VMM only after both failure captures are armed");
    capture
}

fn arm_failure_capture_from_config(config: &VmConfig) -> ArmedFailureCapture {
    let alloc = config.alloc.clone();
    let network = config
        .network
        .as_ref()
        .expect("failure VM reaches the VMM with a complete network attachment");
    let slot_hex = network.tap.rsplit('-').next().expect("slot-derived tap suffix");
    let slot = NetSlot::new(
        u16::from_str_radix(slot_hex, 16).expect("production tap carries a hexadecimal slot"),
    )
    .expect("observed production slot is in range");
    let responder = responder_addr_for_slot(slot);
    let workload = derive_workload_netns_plan(slot, responder);
    let guest = derive_vm_tap_plan(slot, responder);
    assert_eq!(network.netns, workload.netns);
    assert_eq!(network.tap, guest.tap);
    let host_wire = WireCapture::start(&workload.host_veth, 0);
    let (tap_wire, tap_ifindex) = WireCapture::start_in_netns(network.netns.as_str(), &network.tap);
    ArmedFailureCapture {
        alloc,
        host_veth: workload.host_veth,
        netns: network.netns.as_str().to_owned(),
        tap: network.tap.clone(),
        host_wire,
        tap_wire,
        tap_ifindex,
    }
}

fn assert_zero_guest_originated_frames(capture: ArmedFailureCapture) {
    let host = capture.host_wire.stop();
    let tap = capture.tap_wire.stop();
    assert_eq!(host.statistics.drops, 0, "host-veth failure capture is lossless");
    assert_eq!(tap.statistics.drops, 0, "tap failure capture is lossless");
    let host_guest_frames = host
        .frames
        .iter()
        .filter(|frame| frame.packet_type != libc::PACKET_OUTGOING)
        .collect::<Vec<_>>();
    let tap_guest_frames = tap
        .frames
        .iter()
        .filter(|frame| {
            frame.ifindex == capture.tap_ifindex && frame.packet_type != libc::PACKET_OUTGOING
        })
        .collect::<Vec<_>>();
    assert!(
        host_guest_frames.is_empty(),
        "the exact target host-veth observes no guest-originated frame before failure: {host_guest_frames:#?}"
    );
    assert!(
        tap_guest_frames.is_empty(),
        "the exact target tap observes no guest-originated frame before failure: {tap_guest_frames:#?}"
    );
}

fn process_is_alive(pid: u32) -> bool {
    let pid = i32::try_from(pid).expect("VMM pid fits pid_t");
    // SAFETY: signal zero performs an existence check and does not mutate the
    // process; `pid` came from the production VMM adapter.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn assert_allocation_process_is_live(alloc: &AllocationId) {
    let procs =
        CgroupPath::for_alloc(alloc).resolve(Path::new("/sys/fs/cgroup")).join("cgroup.procs");
    let pids = std::fs::read_to_string(&procs)
        .unwrap_or_else(|error| panic!("read live allocation cgroup {}: {error}", procs.display()))
        .lines()
        .map(|line| line.parse::<u32>().expect("cgroup.procs contains decimal PIDs"))
        .collect::<Vec<_>>();
    assert!(
        !pids.is_empty() && pids.iter().all(|pid| process_is_alive(*pid)),
        "the explicit abrupt-cut gate requires every observed allocation process to be live; \
         alloc={alloc}, pids={pids:?}"
    );
}

async fn wait_until_vmm_is_in_its_cgroup(alloc: &AllocationId, pid: u32) {
    let procs =
        CgroupPath::for_alloc(alloc).resolve(Path::new("/sys/fs/cgroup")).join("cgroup.procs");
    let expected = pid.to_string();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if std::fs::read_to_string(&procs)
            .is_ok_and(|content| content.lines().any(|line| line.trim() == expected))
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "real target VMM must enter its production cgroup before external termination"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn assert_failed_vm_cleanup(
    server_tmp: &TempDir,
    rootfs: &Path,
    alloc_id: &str,
    capture: &ArmedFailureCapture,
    control: &VmControl,
) {
    let alloc = AllocationId::new(alloc_id).expect("server allocation id parses");
    let master_bytes = std::fs::metadata(rootfs).expect("stat target rootfs").len();
    let rootfs_plan = RootfsPlan::for_alloc(
        rootfs.to_path_buf(),
        master_bytes,
        &alloc,
        &clone_staging_dir(&server_tmp.path().join("data")),
        Path::new("/run/overdrive/vm/clone-index"),
    );
    let run_dir = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), &alloc);
    let cgroup = CgroupPath::for_alloc(&alloc).resolve(Path::new("/sys/fs/cgroup"));
    let netns = Path::new("/var/run/netns").join(&capture.netns);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let interfaces_absent = [&capture.host_veth, &capture.tap].iter().all(|interface| {
            let name = std::ffi::CString::new(interface.as_str()).expect("interface has no NUL");
            // SAFETY: `name` is a live NUL-terminated interface name.
            unsafe { libc::if_nametoindex(name.as_ptr()) == 0 }
        });
        let clean = matches!(outbound_rule_snapshot(&capture.host_veth), Ok(None))
            && interfaces_absent
            && !netns.exists()
            && !run_dir.path().exists()
            && !cgroup.exists()
            && !rootfs_plan.clone_dest().exists()
            && !rootfs_plan.index_link().exists()
            && !process_is_alive(control.pid);
        if clean {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "failed VM cleanup must remove its VMM/cgroup/clone/index/run-dir/netns/tap/veth/route/nft residue within 30s"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn poll_until_failed_without_running(
    server_tmp: &TempDir,
    cfg: &Path,
    workload_id: &str,
    budget: Duration,
) -> (WorkloadDescribeOutput, overdrive_core::traits::observation_store::AllocStatusRow) {
    let deadline = tokio::time::Instant::now() + budget;
    let mut snapshot_ordinal = 0_u64;
    loop {
        let described =
            describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_path_buf() })
                .await
                .expect("describe while waiting for pre-READY failure");
        if let Some(row) = described.snapshot.rows.first() {
            assert_ne!(
                row.state,
                AllocStateWire::Running,
                "a pre-READY failure can never publish Running"
            );
            if row.state == AllocStateWire::Failed {
                snapshot_ordinal += 1;
                if let Some(durable) = durable_alloc_snapshot(
                    server_tmp,
                    &row.alloc_id,
                    "pre-ready-final",
                    snapshot_ordinal,
                )
                .await
                    && durable.terminal.is_some()
                {
                    let final_described = describe(DescribeArgs {
                        id: workload_id.to_owned(),
                        config_path: cfg.to_path_buf(),
                    })
                    .await
                    .expect("describe finalized pre-READY failure");
                    return (final_described, durable);
                }
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "pre-READY failure must reach Failed within {budget:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn durable_alloc_snapshot(
    server_tmp: &TempDir,
    alloc_id: &str,
    label: &str,
    ordinal: u64,
) -> Option<overdrive_core::traits::observation_store::AllocStatusRow> {
    let observation_db = server_tmp.path().join("data/observation.redb");
    let snapshot = server_tmp.path().join(format!("{label}-{ordinal}.redb"));
    let copied = Command::new("cp")
        .arg("--reflink=always")
        .arg(&observation_db)
        .arg(&snapshot)
        .status()
        .expect("snapshot the production observation database");
    assert!(copied.success(), "observation database snapshot succeeds");
    let observations = LocalObservationStore::open(&snapshot).expect("open observation snapshot");
    let row = observations
        .alloc_status_rows()
        .await
        .expect("read durable allocation rows")
        .into_iter()
        .find(|row| row.alloc_id.as_str() == alloc_id);
    drop(observations);
    std::fs::remove_file(snapshot).expect("remove consumed observation snapshot");
    row
}

fn assert_exact_pre_ready_failure(
    row: &AllocStatusRowBody,
    durable: &overdrive_core::traits::observation_store::AllocStatusRow,
) {
    let (vmm_exit_code, vmm_signal) = match row.reason.as_ref() {
        Some(TransitionReason::VmGuestExitUnreported { vmm_exit_code, vmm_signal }) => {
            (*vmm_exit_code, *vmm_signal)
        }
        ref other => panic!("pre-READY failure retains VmGuestExitUnreported, got {other:?}"),
    };
    assert_eq!(
        row.exit_code, vmm_exit_code,
        "the final public exit code forwards the exact VMM code"
    );
    assert_eq!(
        durable.terminal.clone(),
        Some(TerminalCondition::Failed { exit_code: vmm_exit_code }),
        "the final typed terminal forwards the exact pre-READY VMM code"
    );
    assert_eq!(row.restart_count, 0, "a Job pre-READY failure consumes no restart budget");
    assert_eq!(durable.restart_count, 0, "durable restart accounting is unchanged");
    let transition = row.last_transition.as_ref().expect("failure has an exact lifecycle edge");
    assert_eq!(transition.from, None, "Failed is the first recorded lifecycle transition");
    assert_eq!(transition.to, AllocStateWire::Failed);
    assert_eq!(
        durable.started_at, None,
        "the durable lifecycle never recorded Running/READY before the failure",
    );
    assert!(
        vmm_exit_code.is_some() || vmm_signal.is_some(),
        "the real VMM ending carries at least one concrete process fact"
    );
}

struct MeshResult {
    service_running: AllocStatusRowBody,
    vm_running: AllocStatusRowBody,
    vm_terminal: AllocStatusRowBody,
    service_identity: IssuedCertSummary,
    vm_identity: IssuedCertSummary,
    scan: WireScan,
    readiness: InterceptReadiness,
    guest_egress: GuestEgressAudit,
    ktls: Option<KtlsSocketEvidence>,
}

async fn run_mesh_guest_scenario(id: &str) -> MeshResult {
    // Enclose even composition-root startup: no peer-port socket is allowed to
    // predate the sequence-space oracle's SYN-complete observation universe.
    let peer_wire = WireCapture::start(LOOPBACK_IFACE, SERVICE_PORT);
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("gti-mesh-")
        .tempdir_in(shared_staging_root())
        .expect("mesh fixture tempdir on metal staging root");
    let peer = build_mesh_peer(tmp.path());
    let guest = build_mesh_guest(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &guest, "gti-mesh-guest");

    let (handle, server_tmp, vmm_cuts) = spawn_capture_observed_mtls_server().await;
    let cfg = config_path(server_tmp.path());
    let service_spec = write_toml(server_tmp.path(), "gti-peer.toml", &service_toml(&peer));
    let service_submit = deploy(DeployArgs { spec: service_spec, config_path: cfg.clone() })
        .await
        .expect("deploy mesh peer service through commands::deploy");
    let service_state =
        poll_until_running(&cfg, &service_submit.workload_id, Duration::from_secs(30)).await;
    // A fresh composition has exactly one declared mesh name (`server`), so
    // the production smallest-free frontend allocator assigns the first usable
    // address in its named block. This is the address DNS returns to the guest;
    // it is intentionally distinct from the Service VIP/API field.
    let mesh_destination = SocketAddrV4::new(
        Ipv4Addr::from(u32::from(WORKLOAD_FRONTEND_BASE.network()).saturating_add(1)),
        SERVICE_PORT,
    );
    let service_running =
        service_state.snapshot.rows.into_iter().next().expect("one Running service allocation");
    let service_identity = poll_until_issued_identity(
        &cfg,
        &service_submit.workload_id,
        &service_running.alloc_id,
        Duration::from_secs(10),
    )
    .await;

    // The peer wire is armed before deploy. The observation-only VMM
    // decorator below reports the exact C3 attachment and blocks the real CH
    // spawn until both guest-boundary captures and the exact-rule poller are
    // ready.
    let vm_spec = write_toml(
        server_tmp.path(),
        &format!("{id}.toml"),
        &vm_job_toml(id, "/sbin/gti-mesh-guest", &[], &fixture.kernel_path, &rootfs),
    );
    let vm_submit = deploy(DeployArgs { spec: vm_spec, config_path: cfg.clone() })
        .await
        .expect("deploy VM mesh dialer through commands::deploy");
    let spawn_cut = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::task::spawn_blocking(move || vmm_cuts.recv()),
    )
    .await
    .expect("C3 reaches the pre-VMM observation cut within 30s")
    .expect("join pre-VMM observation receiver")
    .expect("real VMM decorator reports one spawn cut");
    let network = spawn_cut
        .config
        .network
        .as_ref()
        .expect("mesh VM reaches VMM with one complete network attachment");
    let slot_hex = network.tap.rsplit('-').next().expect("slot-derived tap suffix").to_owned();
    let vm_slot = NetSlot::new(
        u16::from_str_radix(&slot_hex, 16).expect("production tap carries a hexadecimal slot"),
    )
    .expect("observed production slot is in range");
    let responder = responder_addr_for_slot(vm_slot);
    let vm_workload = derive_workload_netns_plan(vm_slot, responder);
    let vm_guest = derive_vm_tap_plan(vm_slot, responder);
    assert_eq!(network.netns, vm_workload.netns);
    assert_eq!(network.tap, vm_guest.tap);
    assert_eq!(network.mac, vm_guest.mac);
    let guest_wire = WireCapture::start(&vm_workload.host_veth, 0);
    let (tap_wire, tap_ifindex) = WireCapture::start_in_netns(network.netns.as_str(), &network.tap);
    assert_ne!(tap_ifindex, 0, "exact in-netns tap capture is armed");
    let readiness_task = tokio::spawn(poll_until_outbound_rule_ready(
        vm_workload.host_veth.clone(),
        Duration::from_secs(60),
    ));
    let capture_ready_at = KernelRealtime::now();
    spawn_cut
        .release
        .send(())
        .expect("release real Cloud Hypervisor only after both exact captures are ready");
    let live_readiness = tokio::time::timeout(Duration::from_secs(60), readiness_task)
        .await
        .expect("kernel rule observer remains bounded")
        .expect("kernel rule observer task does not panic");
    assert_eq!(
        live_readiness.host_veth, vm_workload.host_veth,
        "C3 plan identity is derived from the observed production rule"
    );
    let vm_running = poll_until_running(&cfg, &vm_submit.workload_id, Duration::from_secs(60))
        .await
        .snapshot
        .rows
        .into_iter()
        .next()
        .expect("one Running VM allocation");
    let vm_identity = poll_until_issued_identity(
        &cfg,
        &vm_submit.workload_id,
        &vm_running.alloc_id,
        Duration::from_secs(10),
    )
    .await;
    // Preserve cleanup even when the expected kTLS state never appears. RED
    // must fail as a normal assertion, not strand a VM/service until nextest's
    // process timeout kills the whole test binary.
    let ktls = poll_until_ktls(SERVICE_PORT, Duration::from_secs(25)).await;
    let readiness = finish_d7_observation(live_readiness, Duration::from_secs(20)).await;
    let guest_addr = vm_running.workload_addr.expect("Running VM carries its guest address");
    assert_eq!(guest_addr, vm_guest.guest_addr, "slot-derived guest source is exact");
    let scan = peer_wire.stop_and_scan(ktls.as_ref().map(|evidence| evidence.tuple));
    let guest_capture = guest_wire.stop();
    let tap_capture = tap_wire.stop();
    let pre_intercept_tap_frames = tap_capture
        .frames
        .iter()
        .filter(|frame| {
            guest_frame_precedes_capture_ready(frame, tap_ifindex, readiness.kernel_barrier_at)
        })
        .collect::<Vec<_>>();
    assert!(
        pre_intercept_tap_frames.is_empty(),
        "the exact in-netns tap observes zero guest-originated frames from capture-ready={capture_ready_at:?} through intercept-live={:?}: {pre_intercept_tap_frames:#?}",
        readiness.kernel_barrier_at,
    );
    let pre_intercept_host_veth_frames = guest_capture
        .frames
        .iter()
        .filter(|frame| {
            guest_frame_precedes_capture_ready(
                frame,
                readiness.host_veth_ifindex,
                readiness.kernel_barrier_at,
            )
        })
        .collect::<Vec<_>>();
    assert!(
        pre_intercept_host_veth_frames.is_empty(),
        "the exact host-veth observes zero guest-originated frames of every EtherType from capture-ready={capture_ready_at:?} through intercept-live={:?}: {pre_intercept_host_veth_frames:#?}",
        readiness.kernel_barrier_at,
    );
    let guest_egress = audit_guest_egress_boundary(
        &guest_capture.frames,
        &readiness,
        guest_addr,
        mesh_destination,
    );
    validate_exact_d7_accounting(&readiness, &guest_egress, &readiness.host_veth)
        .expect("strict D7 counter equals the complete lossless C3 capture");
    // The reply-dependent Job owns its successful terminal transition. Do not
    // manufacture that state through the public stop path: wait for the guest
    // process to return zero and for production lifecycle observation to emit
    // the real exit result. Cleanup of the independent Service is bounded and
    // remains a separate operation below.
    let terminal = poll_until_natural_job_completion(
        &cfg,
        &vm_submit.workload_id,
        &vm_running.alloc_id,
        Duration::from_secs(60),
    )
    .await;
    // Terminal publication and host-resource reclamation are separate
    // production events. Bound the cleanup observation independently instead
    // of treating the first terminal row as an instantaneous deletion fence.
    let cleanup_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let interfaces_absent = [&vm_workload.host_veth, &vm_guest.tap].iter().all(|interface| {
            let name = std::ffi::CString::new(interface.as_str()).expect("interface has no NUL");
            // SAFETY: `name` is a live NUL-terminated interface name.
            unsafe { libc::if_nametoindex(name.as_ptr()) == 0 }
        });
        let cleaned = matches!(outbound_rule_snapshot(&vm_workload.host_veth), Ok(None))
            && interfaces_absent
            && !Path::new("/var/run/netns").join(vm_workload.netns.as_str()).exists()
            && !Path::new("/run/overdrive/vm").join(&vm_running.alloc_id).exists()
            && !Path::new("/sys/fs/cgroup/overdrive.slice/workloads.slice")
                .join(format!("{}.scope", vm_running.alloc_id))
                .exists();
        if cleaned {
            break;
        }
        assert!(
            tokio::time::Instant::now() < cleanup_deadline,
            "natural Job completion must reclaim rule/interface/netns/VM-dir/cgroup within 30s"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        matches!(outbound_rule_snapshot(&vm_workload.host_veth), Ok(None)),
        "cleanup deletes the exact D7 target rule"
    );
    for interface in [&vm_workload.host_veth, &vm_guest.tap] {
        let name = std::ffi::CString::new(interface.as_str()).expect("interface has no NUL");
        // SAFETY: `name` is a live NUL-terminated interface name.
        assert_eq!(
            unsafe { libc::if_nametoindex(name.as_ptr()) },
            0,
            "cleanup deletes {interface}"
        );
    }
    assert!(
        !Path::new("/var/run/netns").join(vm_workload.netns.as_str()).exists(),
        "cleanup deletes the allocation netns"
    );
    assert!(
        !Path::new("/run/overdrive/vm").join(&vm_running.alloc_id).exists(),
        "cleanup deletes the allocation VM run directory"
    );
    assert!(
        !Path::new("/sys/fs/cgroup/overdrive.slice/workloads.slice")
            .join(format!("{}.scope", vm_running.alloc_id))
            .exists(),
        "cleanup deletes the allocation cgroup scope"
    );
    stop(StopArgs { id: service_submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop mesh peer service through commands::deploy::stop");
    let _ = poll_until_terminal(&cfg, &service_submit.workload_id, Duration::from_secs(30)).await;
    handle.shutdown().await.expect("clean mTLS serve shutdown");
    let vm_terminal = terminal.expect("guest mesh dialer reaches typed natural completion");
    assert_eq!(
        vm_terminal.state,
        AllocStateWire::Terminated,
        "guest mesh dialer must terminate cleanly; reason={:?} error={:?}",
        vm_terminal.reason,
        vm_terminal.error
    );
    // Natural Job completion is represented by Terminated + exit code zero;
    // the lifecycle assertion below distinguishes it from a public stop.
    MeshResult {
        service_running,
        vm_running,
        vm_terminal,
        service_identity,
        vm_identity,
        scan,
        readiness,
        guest_egress,
        ktls,
    }
}

/// S-GTI-01 — a real microVM resolves and dials a mesh Service by name through
/// the production guest network and transparent-mTLS path.
///
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(cgroup)]
async fn microvm_dials_a_mesh_peer_by_name_and_receives_the_reply() {
    let result = run_mesh_guest_scenario("gti-mesh-roundtrip").await;
    // Observable universe: every field of AllocStatusRowBody. The helper
    // asserts the exact lifecycle delta and compares the complete complement.
    assert_exact_lifecycle_delta(&result.vm_running, &result.vm_terminal);
    for (row, summary) in [
        (&result.service_running, &result.service_identity),
        (&result.vm_running, &result.vm_identity),
    ] {
        let expected = SpiffeId::new(&format!(
            "spiffe://overdrive.local/workload/{}/alloc/{}",
            row.workload_id, row.alloc_id
        ))
        .expect("allocation-shaped SPIFFE ID");
        assert_eq!(
            summary.spiffe_id, expected,
            "fresh production IdentityMgr composition must issue the exact per-allocation identity"
        );
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn concurrent_vm_job_deploys_preserve_distinct_c3_capture_and_rule_identity() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("gti-concurrent-")
        .tempdir_in(shared_staging_root())
        .expect("concurrent fixture tempdir on metal staging root");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "gti-spin");
    let (handle, server_tmp) = spawn_mtls_server().await;
    let cfg = config_path(server_tmp.path());

    let plans = [0_u16, 1_u16].map(|slot| {
        let slot = NetSlot::new(slot).expect("concurrent slot in range");
        let responder = responder_addr_for_slot(slot);
        (derive_workload_netns_plan(slot, responder), derive_vm_tap_plan(slot, responder))
    });
    assert_ne!(plans[0].0.host_veth, plans[1].0.host_veth);
    assert_ne!(plans[0].1.tap, plans[1].1.tap);
    assert_ne!(plans[0].1.guest_addr, plans[1].1.guest_addr);

    let capture = WireCapture::start_all();
    let specs = ["gti-concurrent-a", "gti-concurrent-b"].map(|id| {
        write_toml(
            server_tmp.path(),
            &format!("{id}.toml"),
            &vm_job_toml(id, "/sbin/gti-spin", &[], &fixture.kernel_path, &rootfs),
        )
    });
    let (first_submit, second_submit) = tokio::join!(
        deploy(DeployArgs { spec: specs[0].clone(), config_path: cfg.clone() }),
        deploy(DeployArgs { spec: specs[1].clone(), config_path: cfg.clone() }),
    );
    let submits = [
        first_submit.expect("first parallel VM deploy"),
        second_submit.expect("second parallel VM deploy"),
    ];
    let (first_running, second_running) = tokio::join!(
        poll_until_running(&cfg, &submits[0].workload_id, Duration::from_secs(60)),
        poll_until_running(&cfg, &submits[1].workload_id, Duration::from_secs(60)),
    );
    let running: [WorkloadDescribeOutput; 2] = (first_running, second_running).into();
    let observed_addresses = running
        .iter()
        .map(|state| {
            state.snapshot.rows[0]
                .workload_addr
                .expect("parallel VM Running row carries its guest address")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        observed_addresses.iter().copied().collect::<std::collections::BTreeSet<_>>().len(),
        2,
        "parallel allocations retain distinct guest identities"
    );

    let rules = nft::list_rules("overdrive-mtls", "prerouting")
        .expect("strict rule dump while both parallel VMs are Running");
    let targets = plans
        .iter()
        .map(|(workload, _)| {
            exact_d7_target(&rules, &workload.host_veth)
                .expect("one exact D7 rule per concurrent C3 host-veth")
        })
        .collect::<Vec<_>>();
    assert_ne!(targets[0].handle, targets[1].handle);
    assert_ne!(targets[0].userdata, targets[1].userdata);
    assert_ne!(targets[0].normalized_program, targets[1].normalized_program);

    let ifindices = plans.map(|(workload, _)| {
        let name = std::ffi::CString::new(workload.host_veth).expect("host-veth has no NUL");
        // SAFETY: the NUL-terminated interface name remains live for this call.
        let ifindex = unsafe { libc::if_nametoindex(name.as_ptr()) };
        assert_ne!(ifindex, 0, "parallel C3 host-veth is live");
        ifindex
    });
    assert_ne!(ifindices[0], ifindices[1], "parallel C3 captures have distinct ifindices");

    let (first_stop, second_stop) = tokio::join!(
        stop(StopArgs { id: submits[0].workload_id.clone(), config_path: cfg.clone() }),
        stop(StopArgs { id: submits[1].workload_id.clone(), config_path: cfg.clone() }),
    );
    first_stop.expect("stop first parallel VM Job");
    second_stop.expect("stop second parallel VM Job");
    let (first_terminal, second_terminal) = tokio::join!(
        poll_until_terminal(&cfg, &submits[0].workload_id, Duration::from_secs(30)),
        poll_until_terminal(&cfg, &submits[1].workload_id, Duration::from_secs(30)),
    );
    assert_eq!(first_terminal.snapshot.rows[0].state, AllocStateWire::Terminated);
    assert_eq!(second_terminal.snapshot.rows[0].state, AllocStateWire::Terminated);
    let captured = capture.stop();
    for ifindex in ifindices {
        assert!(
            captured.frames.iter().any(|frame| frame.ifindex == ifindex),
            "the pre-deploy all-interface capture retains frames under each real C3 ifindex"
        );
    }
    handle.shutdown().await.expect("clean concurrent mTLS serve shutdown");
}

/// S-GTI-03 — the guest's plaintext request/reply is TLS 1.3 on the peer wire,
/// with kTLS installed in both directions.
///
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(cgroup)]
async fn the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear() {
    let result = run_mesh_guest_scenario("gti-mesh-wire").await;
    assert_eq!(
        result.vm_terminal.state,
        AllocStateWire::Terminated,
        "wire proof is coupled to a successful guest round-trip"
    );
    let ktls = result
        .ktls
        .as_ref()
        .expect("one live outbound socket record must carry TLS 1.3 ULP plus RX and TX kTLS state");
    assert_eq!(
        result.scan.exact_tuple,
        Some(ktls.tuple),
        "wire evidence must be correlated to the exact socket tuple observed by ss"
    );
    assert!(
        result.scan.exact_records_to_peer > 0,
        "the exact kTLS socket's request direction must carry TLS application_data; got {:?}",
        result.scan
    );
    assert!(
        result.scan.exact_records_from_peer > 0,
        "the exact kTLS socket's response direction must carry TLS application_data; got {:?}",
        result.scan
    );
    assert_eq!(
        result.scan.plaintext_hits_on_any_peer_stream, 0,
        "neither plaintext litmus may occur on any stream touching the peer port; got {:?}",
        result.scan
    );
    assert!(
        result.scan.peer_streams_observed > 0,
        "the unfiltered peer-port stream universe must not be empty"
    );
    assert!(
        record_has_bidirectional_tls13_ktls(&ktls.record),
        "one ss record must itself contain tcp-ulp-tls, TLS 1.3, rxconf, and txconf; got:\n{}",
        ktls.record
    );
}

/// S-GTI-04 — a destination outside the workload mesh block is classified
/// NonMesh and reached unchanged over a clear TCP connection.
///
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(cgroup)]
async fn the_same_guest_reaches_a_non_mesh_destination_in_the_clear() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("gti-non-mesh-")
        .tempdir_in(shared_staging_root())
        .expect("non-mesh fixture tempdir on metal staging root");
    let (handle, server_tmp) = spawn_mtls_server().await;
    let cfg = config_path(server_tmp.path());

    let host_ip = overdrive_control_plane::iface::resolve_iface_ipv4(DEFAULT_CLIENT_IFACE)
        .expect("resolve production single-node client interface");
    assert!(
        !WORKLOAD_SUBNET_BASE.contains(&host_ip),
        "non-mesh fixture endpoint must be outside {WORKLOAD_SUBNET_BASE}, got {host_ip}"
    );
    let listener = TcpListener::bind((host_ip, 0)).expect("bind non-mesh plaintext endpoint");
    listener.set_nonblocking(true).expect("make non-mesh accept bounded");
    let destination = listener.local_addr().expect("non-mesh endpoint address");
    let endpoint = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
                    let mut got = vec![0_u8; NON_MESH_REQUEST.len()];
                    stream.read_exact(&mut got).unwrap();
                    stream.write_all(NON_MESH_RESPONSE).unwrap();
                    stream.flush().unwrap();
                    return got;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "guest must connect to the non-mesh endpoint within 30s"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept non-mesh endpoint: {error}"),
            }
        }
    });

    let guest = build_non_mesh_guest(tmp.path(), destination);
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &guest, "gti-non-mesh-guest");
    let vm_spec = write_toml(
        server_tmp.path(),
        "gti-non-mesh.toml",
        &vm_job_toml(
            "gti-non-mesh",
            "/sbin/gti-non-mesh-guest",
            &[],
            &fixture.kernel_path,
            &rootfs,
        ),
    );
    let submit = deploy(DeployArgs { spec: vm_spec, config_path: cfg.clone() })
        .await
        .expect("deploy non-mesh VM dialer");
    let running = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60))
        .await
        .snapshot
        .rows
        .into_iter()
        .next()
        .expect("one Running non-mesh VM row");
    let terminal = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let endpoint_result = endpoint.join();
    let terminal = terminal.snapshot.rows.into_iter().next().expect("one terminal non-mesh VM row");
    handle.shutdown().await.expect("clean mTLS serve shutdown");

    let received = endpoint_result.expect("join non-mesh plaintext endpoint");
    assert_eq!(
        received, NON_MESH_REQUEST,
        "plain endpoint must receive the guest-authored bytes unchanged"
    );
    assert_eq!(
        terminal.state,
        AllocStateWire::Terminated,
        "the guest terminates cleanly only after receiving the distinct clear response unchanged"
    );
    // Observable universe: every field of AllocStatusRowBody. The byte oracle
    // above plus the exact lifecycle delta and complete complement close the
    // non-mesh behavior surface.
    assert_exact_lifecycle_delta(&running, &terminal);
}

/// S-GTI-07 — workload describe surfaces the VM guest address, never its
/// transit forwarding hop.
///
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(cgroup)]
async fn the_operator_sees_the_microvm_workloads_own_mesh_address_not_its_transit_hop() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("gti-describe-")
        .tempdir_in(shared_staging_root())
        .expect("describe fixture tempdir on metal staging root");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "gti-spin");
    let (handle, server_tmp) = spawn_mtls_server().await;
    let cfg = config_path(server_tmp.path());

    let vm_spec = write_toml(
        server_tmp.path(),
        "gti-describe.toml",
        &vm_job_toml("gti-describe", "/sbin/gti-spin", &[], &fixture.kernel_path, &rootfs),
    );
    let submit = deploy(DeployArgs { spec: vm_spec, config_path: cfg.clone() })
        .await
        .expect("deploy long-lived VM for describe");
    let running = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let running_row = running.snapshot.rows.first().expect("one Running VM row");
    let _ = poll_until_issued_identity(
        &cfg,
        &submit.workload_id,
        &running_row.alloc_id,
        Duration::from_secs(10),
    )
    .await;
    let described =
        describe(DescribeArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
            .await
            .expect("workload describe direct command call");

    let slot = NetSlot::new(0).expect("first allocation owns slot zero");
    let responder = responder_addr_for_slot(slot);
    let guest = derive_vm_tap_plan(slot, responder);
    let transit = derive_workload_netns_plan(slot, responder);
    let row = described.snapshot.rows.first().expect("one Running VM row");
    assert_eq!(
        row.workload_addr,
        Some(guest.guest_addr),
        "describe must project the persisted guest NIC address"
    );
    assert_ne!(
        row.workload_addr,
        Some(transit.workload_addr),
        "describe must never substitute the transit-veth forwarding address"
    );
    // Response observable universe: the full command wrapper plus the entire
    // serialized API snapshot. The pre-change contract is independently
    // reconstructed from a CoW snapshot of the production durable inputs; it
    // never reads or clones this post-change HTTP result. Add exactly the one
    // permitted workload_addr fact, then require whole-response equality.
    let pre_change_contract =
        genuine_pre_change_contract(server_tmp.path(), &submit, guest.guest_addr).await;
    assert_eq!(described.workload_id, pre_change_contract.workload_id);
    assert_eq!(described.spec_digest, pre_change_contract.spec_digest);
    assert_eq!(described.allocations_total, pre_change_contract.allocations_total);
    assert_eq!(described.empty_state_message, pre_change_contract.empty_state_message);
    let mut expected_post_change = pre_change_contract.clone();
    expected_post_change.snapshot.rows[0].workload_addr = Some(guest.guest_addr);
    let post_change = serde_json::to_value(&described.snapshot).expect("post-change snapshot JSON");
    let expected_post_change = serde_json::to_value(&expected_post_change.snapshot)
        .expect("independent expected post-change snapshot JSON");
    assert_eq!(
        post_change, expected_post_change,
        "adding workload_addr must leave the complete pre-change response complement unchanged"
    );

    // Render observable universe: the entire output string. The baseline is a
    // frozen pre-step render contract over those independent durable facts,
    // not a second call derived from the post-change object. Its sole permitted
    // delta is the one canonical Addresses section.
    let pre_change_render = frozen_pre_change_render_contract(&pre_change_contract);
    assert_eq!(
        overdrive_cli::render::workload_describe(&pre_change_contract),
        pre_change_render,
        "the live renderer without the additive address must match the genuine pre-change contract"
    );
    let rendered = overdrive_cli::render::workload_describe(&described);
    let address_delta = format!("Addresses:\n  {}: {}\n", row.alloc_id, guest.guest_addr);
    assert_eq!(
        rendered.match_indices(&address_delta).count(),
        1,
        "the exact canonical-address delta must occur once; got:\n{rendered}"
    );
    assert_eq!(
        rendered.replacen(&address_delta, "", 1),
        pre_change_render,
        "all rendered output outside the one permitted Addresses section must remain byte-exact"
    );
    assert!(
        !rendered.contains(&format!("{}: {}", row.alloc_id, transit.workload_addr)),
        "live operator renderer must not show the transit address; got:\n{rendered}"
    );

    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop describe VM");
    let _ = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    handle.shutdown().await.expect("clean mTLS serve shutdown");
}

/// S-GTI-02 — the guest's first mesh connection is born intercepted.
///
/// Observable universe: every AF_PACKET frame captured from before VM deploy
/// through guest termination on the exact allocation host-veth whose source is
/// the exact guest address and whose destination is the exact mesh peer tuple,
/// plus every peer-port loopback stream and the typed kernel nft-rule snapshot.
/// Every exact-tuple packet carries its kernel event timestamp; nft readiness
/// is a conservative `CLOCK_REALTIME` barrier sampled after the successful
/// typed query. Missing/equal timestamps count as pre-ready. Assertions
/// quantify over those complete captured collections; no sampled event field,
/// userspace dequeue time, or selected unrelated socket stands in for their
/// complement.
///
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn the_guests_first_mesh_dial_is_born_intercepted_no_cleartext_escapes() {
    let result = run_mesh_guest_scenario("gti-born-captured").await;
    let first_syn = result.guest_egress.first_syn.unwrap_or_else(|| {
        panic!(
            "the exact guest-to-mesh host-veth universe contains the command's first SYN; \
             readiness={:?}; all interface TCP={:#?}",
            result.readiness, result.guest_egress.interface_tcp
        )
    });
    let generations = [
        result.readiness.accounting.before[0].generation,
        result.readiness.accounting.before[1].generation,
        result.readiness.accounting.after[0].generation,
        result.readiness.accounting.after[1].generation,
    ];
    assert!(
        generations.iter().all(|generation| *generation == generations[0] && *generation != 0),
        "the strict D7 brackets retain one full non-zero generation: {generations:?}"
    );
    assert!(
        !result.guest_egress.segments.is_empty(),
        "the exact guest escape-boundary universe must be non-empty"
    );
    assert!(
        first_syn.flags & 0x02 != 0 && first_syn.flags & 0x10 == 0,
        "the independently captured first exact-tuple packet is an initial SYN: {first_syn:?}"
    );
    assert_eq!(first_syn.payload_len, 0, "an initial SYN has no application payload");
    assert_eq!(
        result.guest_egress.interface_tcp.first().copied(),
        Some(first_syn),
        "the first eligible host-veth ingress TCP packet is the exact initial SYN"
    );
    assert!(
        first_syn.tuple.source.port() != 0,
        "the first guest SYN carries one exact ephemeral source port"
    );
    assert!(
        result.guest_egress.packet_count > 0 && result.guest_egress.byte_count > 0,
        "D7 exact packet/validated-IPv4-byte universe is non-empty"
    );
    assert_eq!(
        result
            .guest_egress
            .segments
            .iter()
            .map(|segment| u64::try_from(segment.ipv4_total_len).expect("tot_len fits u64"))
            .sum::<u64>(),
        result.guest_egress.byte_count,
        "the D7 byte universe is exactly the validated IPv4 tot_len sum"
    );
    let pre_ready = result
        .guest_egress
        .segments
        .iter()
        .filter(|segment| {
            segment
                .kernel_event_at
                .is_none_or(|event_at| event_at <= result.readiness.kernel_barrier_at)
        })
        .collect::<Vec<_>>();
    assert!(
        pre_ready.is_empty(),
        "actual kernel-rule readiness must precede every captured TCP segment on the exact guest \
         -> mesh tuple, including the causally-first SYN after EXEC; pre-ready={pre_ready:#?}, \
         readiness={:?}",
        result.readiness
    );
    assert!(
        first_syn
            .kernel_event_at
            .is_some_and(|event_at| event_at > result.readiness.kernel_barrier_at),
        "a kernel timestamp must prove the first exact guest SYN occurred strictly after the \
         conservative post-query nft-readiness barrier"
    );
    assert!(
        result.guest_egress.plaintext_request_hits > 0,
        "the exact host-veth capture must independently observe the guest-authored plaintext \
         request before TPROXY consumes it"
    );
    assert_eq!(
        result.scan.plaintext_hits_on_any_peer_stream, 0,
        "the first captured mesh connection must not expose either plaintext litmus; got {:?}",
        result.scan
    );
    assert!(
        result.scan.exact_records_to_peer > 0 && result.scan.exact_records_from_peer > 0,
        "the born-captured connection must carry TLS application_data in both directions; got {:?}",
        result.scan
    );
    assert!(
        result
            .ktls
            .as_ref()
            .is_some_and(|evidence| { record_has_bidirectional_tls13_ktls(&evidence.record) }),
        "the born-captured connection must install bidirectional TLS 1.3 kTLS"
    );
}

/// The mapped D7 supporting contract over the complete production ruleset and
/// lossless exact-ifindex capture universe. This deliberately drives a fresh
/// real guest flow; the small synthetic corruption checks below remain
/// separate pure-function identities.
/// CONTRACT_SHAPE: unbounded-preservation.
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn d7_exact_rule_hit_witness_is_loss_and_mutation_conservative() {
    let result = run_mesh_guest_scenario("gti-d7-exact-accounting").await;
    validate_exact_d7_accounting(
        &result.readiness,
        &result.guest_egress,
        &result.readiness.host_veth,
    )
    .expect("the complete production D7 counter delta equals the lossless exact-ifindex capture");
    assert!(
        result.guest_egress.segments.iter().all(|segment| {
            segment
                .kernel_event_at
                .is_some_and(|event_at| event_at > result.readiness.kernel_barrier_at)
        }),
        "every member of the complete captured D7 universe follows the conservative readiness cut"
    );
    assert!(
        result.guest_egress.packet_count > 0 && result.guest_egress.byte_count > 0,
        "the production witness cannot pass vacuously"
    );
}

fn synthetic_d7_rule(host_veth: &str, packets: u64, bytes: u64) -> nft::RuleInfo {
    let port = 36_533;
    nft::RuleInfo {
        handle: 17,
        userdata: nft::userdata_egress(host_veth, port),
        counter: Some(nft::RuleCounterSnapshot { packets, bytes }),
        normalized_program: nft::normalized_rule_program_identity(&nft::egress_tproxy_rule_exprs(
            host_veth,
            Ipv4Addr::LOCALHOST,
            port,
            0x1,
        ))
        .expect("synthetic production program normalizes"),
    }
}

fn synthetic_d7_pair(rule: nft::RuleInfo) -> [nft::RuleSnapshot; 2] {
    [
        nft::RuleSnapshot { generation: 9, rules: vec![rule.clone()] },
        nft::RuleSnapshot { generation: 9, rules: vec![rule] },
    ]
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn capture_ready_requires_the_real_c3_identity() {
    let rule = synthetic_d7_rule("ovd-veth-real", 0, 0);
    assert!(exact_d7_target(std::slice::from_ref(&rule), "ovd-veth-real").is_ok());
    assert_eq!(outbound_rule_from_rules(&[], "ovd-veth-real"), Ok(None));
    assert_eq!(
        outbound_rule_from_rules(std::slice::from_ref(&rule), "ovd-veth-real"),
        Ok(Some(rule.clone())),
    );
    assert!(
        outbound_rule_from_rules(&[rule.clone(), rule.clone()], "ovd-veth-real").is_err(),
        "duplicate allocation ownership is ambiguity, never absence",
    );
    let mut malformed = rule.clone();
    malformed.userdata.insert(nft::USERDATA_MAGIC.len() + 1, 0xff);
    assert!(
        outbound_rule_from_rules(&[malformed], "ovd-veth-real").is_err(),
        "malformed allocation-tagged ownership is ambiguity, never absence",
    );
    assert!(
        exact_d7_target(std::slice::from_ref(&rule), "ovd-veth-other").is_err(),
        "a guessed or sibling host-veth can never establish capture readiness"
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn pre_baseline_all_ethertype_guest_frame_always_invalidates_born_captured() {
    let frame = CapturedFrame {
        kernel_event_at: Some(KernelRealtime(99)),
        ifindex: 41,
        protocol: 0x0806,
        packet_type: libc::PACKET_HOST,
        wire_len: 0,
        truncated: false,
        control_truncated: false,
        aux: None,
        bytes: Vec::new(),
    };
    assert!(guest_frame_precedes_capture_ready(&frame, 41, KernelRealtime(100)));
    let mut missing_timestamp = frame;
    missing_timestamp.kernel_event_at = None;
    assert!(
        guest_frame_precedes_capture_ready(&missing_timestamp, 41, KernelRealtime(100)),
        "a non-IP frame with a missing timestamp is conservatively pre-baseline"
    );
    assert!(
        !guest_frame_precedes_capture_ready(&missing_timestamp, 42, KernelRealtime(100)),
        "a sibling ifindex is outside the exact host-veth universe"
    );
    missing_timestamp.packet_type = libc::PACKET_OUTGOING;
    assert!(
        !guest_frame_precedes_capture_ready(&missing_timestamp, 41, KernelRealtime(100)),
        "the opposite host-veth direction is not guest ingress"
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn live_intercept_is_invalidated_by_every_guard_mutation_signal() {
    let host_veth = "ovd-veth-real";
    let original = synthetic_d7_rule(host_veth, 2, 80);
    let stable = synthetic_d7_pair(original.clone());
    assert!(validate_stable_d7_pair(&stable, host_veth, Some(&original.userdata)).is_ok());

    let mut mutations = Vec::new();
    let mut generation = stable.clone();
    generation[1].generation = 10;
    mutations.push(generation);
    let mut handle = stable.clone();
    handle[1].rules[0].handle += 1;
    mutations.push(handle);
    let mut userdata = stable.clone();
    userdata[1].rules[0].userdata.push(0);
    mutations.push(userdata);
    let mut program = stable.clone();
    program[1].rules[0].normalized_program.push(0);
    mutations.push(program);
    let mut counter = stable.clone();
    counter[1].rules[0].counter = Some(nft::RuleCounterSnapshot { packets: 1, bytes: 80 });
    mutations.push(counter);
    let mut duplicate = stable;
    duplicate[1].rules.push(original);
    mutations.push(duplicate);

    for mutation in mutations {
        assert!(
            validate_stable_d7_pair(&mutation, host_veth, None).is_err(),
            "generation/handle/userdata/program/counter/uniqueness mutation must fail closed"
        );
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn synthetic_d7_accounting_rejects_competing_capture_counts() {
    let host_veth = "ovd-veth-real";
    let before = synthetic_d7_pair(synthetic_d7_rule(host_veth, 5, 100));
    let after = synthetic_d7_pair(synthetic_d7_rule(host_veth, 8, 250));
    let readiness = InterceptReadiness {
        kernel_barrier_at: KernelRealtime(1),
        host_veth_ifindex: 41,
        host_veth: host_veth.to_owned(),
        accounting: D7Accounting {
            before,
            after,
            target_userdata: nft::userdata_egress(host_veth, 36_533),
        },
    };
    let audit = GuestEgressAudit {
        segments: Vec::new(),
        interface_tcp: Vec::new(),
        first_syn: None,
        plaintext_request_hits: 0,
        packet_count: 3,
        byte_count: 150,
    };
    validate_exact_d7_accounting(&readiness, &audit, host_veth)
        .expect("checked non-zero packet and byte deltas equal the full capture");

    let mut competing = audit;
    competing.packet_count += 1;
    assert!(validate_exact_d7_accounting(&readiness, &competing, host_veth).is_err());
}

fn synthetic_guest_tcp_frame() -> CapturedFrame {
    let mut bytes = vec![0_u8; 40];
    bytes[0] = 0x45;
    bytes[2..4].copy_from_slice(&40_u16.to_be_bytes());
    bytes[8] = 64;
    bytes[9] = 6;
    bytes[12..16].copy_from_slice(&Ipv4Addr::new(10, 99, 0, 2).octets());
    bytes[16..20].copy_from_slice(&Ipv4Addr::new(10, 96, 0, 2).octets());
    bytes[20..22].copy_from_slice(&40_000_u16.to_be_bytes());
    bytes[22..24].copy_from_slice(&SERVICE_PORT.to_be_bytes());
    bytes[32] = 5 << 4;
    bytes[33] = 0x02;
    CapturedFrame {
        kernel_event_at: Some(KernelRealtime(1)),
        ifindex: 41,
        protocol: ETH_P_IP,
        packet_type: 0,
        wire_len: bytes.len(),
        truncated: false,
        control_truncated: false,
        aux: Some(PacketAuxData {
            status: 0,
            len: 40,
            snaplen: 40,
            mac: 0,
            net: 0,
            vlan_tci: 0,
            vlan_tpid: 0,
        }),
        bytes,
    }
}

fn synthetic_peer_tcp_frame(
    tuple: FlowTuple,
    sequence: u32,
    flags: u8,
    payload: &[u8],
    packet_type: u8,
) -> CapturedFrame {
    let total_len = 40_usize.checked_add(payload.len()).expect("synthetic frame length");
    let mut bytes = vec![0_u8; total_len];
    bytes[0] = 0x45;
    bytes[2..4].copy_from_slice(
        &u16::try_from(total_len).expect("synthetic IPv4 length fits u16").to_be_bytes(),
    );
    bytes[8] = 64;
    bytes[9] = 6;
    bytes[12..16].copy_from_slice(&tuple.source.ip().octets());
    bytes[16..20].copy_from_slice(&tuple.destination.ip().octets());
    bytes[20..22].copy_from_slice(&tuple.source.port().to_be_bytes());
    bytes[22..24].copy_from_slice(&tuple.destination.port().to_be_bytes());
    bytes[24..28].copy_from_slice(&sequence.to_be_bytes());
    bytes[32] = 5 << 4;
    bytes[33] = flags;
    bytes[40..].copy_from_slice(payload);
    CapturedFrame {
        kernel_event_at: Some(KernelRealtime(1)),
        ifindex: 1,
        protocol: ETH_P_IP,
        packet_type,
        wire_len: total_len,
        truncated: false,
        control_truncated: false,
        aux: Some(PacketAuxData {
            status: 0,
            len: u32::try_from(total_len).expect("synthetic length fits u32"),
            snaplen: u32::try_from(total_len).expect("synthetic length fits u32"),
            mac: 0,
            net: 0,
            vlan_tci: 0,
            vlan_tpid: 0,
        }),
        bytes,
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn peer_wire_reassembly_uses_sequence_space_and_deduplicates_loopback_copies() {
    let tuple = FlowTuple {
        source: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 40_001),
        destination: SocketAddrV4::new(Ipv4Addr::LOCALHOST, SERVICE_PORT),
    };
    let split = REQUEST.len() / 2;
    let second_sequence = 1_001_u32 + u32::try_from(split).expect("marker split fits u32");
    let frames = vec![
        synthetic_peer_tcp_frame(tuple, 1_000, 0x02, &[], libc::PACKET_OUTGOING),
        synthetic_peer_tcp_frame(tuple, 1_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(
            tuple,
            second_sequence,
            0x18,
            &REQUEST[split..],
            libc::PACKET_OUTGOING,
        ),
        synthetic_peer_tcp_frame(tuple, 1_001, 0x18, &REQUEST[..split], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(tuple, 1_001, 0x18, &REQUEST[..split], libc::PACKET_OUTGOING),
        synthetic_peer_tcp_frame(
            tuple,
            second_sequence,
            0x18,
            &REQUEST[split..],
            libc::PACKET_HOST,
        ),
    ];
    let scan = scan_frames(&frames, SERVICE_PORT, Some(tuple));
    assert_eq!(scan.peer_streams_observed, 1);
    assert_eq!(
        scan.plaintext_hits_on_any_peer_stream, 1,
        "dequeue reordering and the two loopback packet types reconstruct one byte stream"
    );
}

/// CONTRACT_SHAPE: pure-function.
#[test]
fn peer_wire_reassembly_rejects_gaps_conflicts_and_unknown_copy_directions() {
    let tuple = FlowTuple {
        source: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 40_002),
        destination: SocketAddrV4::new(Ipv4Addr::LOCALHOST, SERVICE_PORT),
    };
    let gap = vec![
        synthetic_peer_tcp_frame(tuple, 2_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(tuple, 2_002, 0x18, b"gap", libc::PACKET_HOST),
    ];
    assert!(
        std::panic::catch_unwind(|| scan_frames(&gap, SERVICE_PORT, Some(tuple))).is_err(),
        "a missing sequence byte fails closed"
    );

    let conflict = vec![
        synthetic_peer_tcp_frame(tuple, 3_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(tuple, 3_001, 0x18, b"a", libc::PACKET_HOST),
        synthetic_peer_tcp_frame(tuple, 3_001, 0x18, b"b", libc::PACKET_OUTGOING),
    ];
    assert!(
        std::panic::catch_unwind(|| scan_frames(&conflict, SERVICE_PORT, Some(tuple))).is_err(),
        "conflicting retransmission/copy bytes fail closed"
    );

    let unknown_direction = vec![
        synthetic_peer_tcp_frame(tuple, 4_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(tuple, 4_001, 0x18, b"x", libc::PACKET_BROADCAST),
    ];
    assert!(
        std::panic::catch_unwind(|| {
            scan_frames(&unknown_direction, SERVICE_PORT, Some(tuple));
        })
        .is_err(),
        "an unclassified AF_PACKET direction fails closed"
    );
}

proptest! {
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_d7_decoder_and_oracle_error_fails_closed(
        before_packets in 0_u64..(u64::MAX / 2),
        before_bytes in 0_u64..(u64::MAX / 2),
        packet_delta in 1_u64..1_000_000,
        byte_delta in 1_u64..65_535_000,
        oracle_corruption in 0_u8..15,
        capture_corruption in 0_u8..8,
        drops in 1_u32..=u32::MAX,
    ) {
        let host_veth = "ovd-veth-real";
        let before = synthetic_d7_pair(synthetic_d7_rule(
            host_veth,
            before_packets,
            before_bytes,
        ));
        let after = synthetic_d7_pair(synthetic_d7_rule(
            host_veth,
            before_packets + packet_delta,
            before_bytes + byte_delta,
        ));
        let mut readiness = InterceptReadiness {
            kernel_barrier_at: KernelRealtime(1),
            host_veth_ifindex: 41,
            host_veth: host_veth.to_owned(),
            accounting: D7Accounting {
                before,
                after,
                target_userdata: nft::userdata_egress(host_veth, 36_533),
            },
        };
        let mut audit = GuestEgressAudit {
            segments: Vec::new(),
            interface_tcp: Vec::new(),
            first_syn: None,
            plaintext_request_hits: 0,
            packet_count: packet_delta,
            byte_count: byte_delta,
        };
        prop_assert!(validate_exact_d7_accounting(&readiness, &audit, host_veth).is_ok());

        match oracle_corruption {
            0 => readiness.accounting.before[0].rules.clear(),
            1 => {
                let duplicate = readiness.accounting.before[0].rules[0].clone();
                readiness.accounting.before[0].rules.push(duplicate);
            }
            2 => readiness.accounting.before[0].rules[0].counter = None,
            3 => readiness.accounting.before[0].rules[0].normalized_program.clear(),
            4 => readiness.accounting.before[0].rules[0].normalized_program.reverse(),
            5 => readiness.accounting.after[1].generation =
                readiness.accounting.after[1].generation.wrapping_add(1),
            6 => readiness.accounting.after[0].rules[0].handle += 1,
            7 => readiness.accounting.after[0].rules[0].userdata.push(0),
            8 => {
                readiness.accounting.after[0].rules[0].counter = Some(nft::RuleCounterSnapshot {
                    packets: before_packets.saturating_sub(1),
                    bytes: before_bytes.saturating_sub(1),
                });
            }
            9 => audit.packet_count += 1,
            10 => audit.byte_count += 1,
            11 => {
                readiness.accounting.before[1].rules[0].counter =
                    Some(nft::RuleCounterSnapshot {
                        packets: before_packets + 1,
                        bytes: before_bytes,
                    });
            }
            12 => {
                let duplicate = readiness.accounting.after[1].rules[0].clone();
                readiness.accounting.after[1].rules.push(duplicate);
            }
            13 => readiness.accounting.after[1].rules[0].normalized_program.push(0),
            _ => readiness.accounting.target_userdata.push(0),
        }
        prop_assert!(
            validate_exact_d7_accounting(&readiness, &audit, host_veth).is_err(),
            "zero/duplicate targets and every generation/identity/counter/equality mutation fail closed",
        );

        let mut frame = synthetic_guest_tcp_frame();
        match capture_corruption {
            0 => frame.truncated = true,
            1 => frame.control_truncated = true,
            2 => frame.aux = None,
            3 => frame.aux.as_mut().expect("synthetic auxdata").status |= TP_STATUS_CSUMNOTREADY,
            4 => frame.aux.as_mut().expect("synthetic auxdata").len -= 1,
            5 => frame.bytes[6] = 0x20,
            6 => frame.bytes[3] -= 1,
            _ => frame.bytes[32] = 0,
        }
        prop_assert!(
            parse_tcp_segment(&frame, true).is_err(),
            "capture truncation, ancillary loss, fragment, offload, and length/header ambiguity fail closed",
        );
        prop_assert!(
            capture_statistics_are_lossless(PacketStatistics { packets: drops, drops }).is_err(),
            "every non-zero PACKET_STATISTICS drop count fails closed",
        );
    }
}

/// S-GTI-05 — an intercept-install failure refuses execution fail-closed.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn when_the_mesh_guard_cannot_be_installed_the_workload_is_refused() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("gti-install-failure-")
        .tempdir_in(shared_staging_root())
        .expect("failure fixture tempdir on metal staging root");
    let target_dir = tmp.path().join("target");
    std::fs::create_dir_all(&target_dir).expect("create target staging dir");
    let forbidden = build_forbidden_exec_probe(&target_dir, "gti-forbidden-install-exec");
    let target_rootfs = stage_rootfs_with_extra_binary(
        &target_dir,
        &fixture,
        &forbidden,
        "gti-forbidden-install-exec",
    );

    let (handle, server_tmp, cuts, created, marker_observed, _terminator) =
        spawn_failure_observed_mtls_server().await;
    let cfg = config_path(server_tmp.path());
    let malformed = fault_fixture::ProductInputHookFixture::install();
    let target_spec = write_toml(
        server_tmp.path(),
        "gti-install-failure-target.toml",
        &vm_job_toml(
            "gti-install-failure-target",
            "/sbin/gti-forbidden-install-exec",
            &[],
            &fixture.kernel_path,
            &target_rootfs,
        ),
    );
    let target_submit = deploy(DeployArgs { spec: target_spec, config_path: cfg.clone() })
        .await
        .expect("deploy VM against the production-named INPUT-hook counterexample");
    let target_capture = arm_failure_capture(&cuts);
    let target_control =
        created.recv_timeout(Duration::from_secs(30)).expect("observe target VMM creation");
    let terminal =
        poll_until_terminal(&cfg, &target_submit.workload_id, Duration::from_secs(60)).await;
    let row = terminal.snapshot.rows.first().expect("one target allocation");
    assert_eq!(row.state, AllocStateWire::Failed);
    match row.reason.as_ref() {
        Some(TransitionReason::MtlsInterceptInstallFailed { stage, detail }) => {
            assert_eq!(stage, "outbound_tproxy_install");
            assert!(detail.contains("append-egress"), "typed operation is retained: {detail}");
            assert!(
                detail.contains("append-rule"),
                "typed netlink operation is retained: {detail}"
            );
            assert!(
                detail.contains("Operation not supported") || detail.contains("os error 95"),
                "the real -EOPNOTSUPP source is retained: {detail}"
            );
        }
        other => panic!("wrong-hook install produces the typed install cause, got {other:?}"),
    }
    assert!(
        row.started_at.is_some(),
        "the superseding Failed row retains the permitted transient Running timestamp"
    );
    assert_eq!(row.restart_count, 0);
    assert_failed_vm_cleanup(
        &server_tmp,
        &target_rootfs,
        &row.alloc_id,
        &target_capture,
        &target_control,
    )
    .await;
    assert_guest_boundary(
        &marker_observed,
        &row.alloc_id,
        false,
        GuestBeaconTrace { ready: 1, exec: 0, exit: 0 },
    );
    assert_zero_guest_originated_frames(target_capture);
    malformed.finish();

    handle.shutdown().await.expect("clean failure server shutdown");
}

async fn poll_until_same_allocation_restarted(
    cfg: &Path,
    workload_id: &str,
    alloc_id: &str,
    budget: Duration,
) -> AllocStatusRowBody {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let described =
            describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_path_buf() })
                .await
                .expect("describe while waiting for same-allocation restart");
        let [row] = described.snapshot.rows.as_slice() else {
            panic!("standing one-replica intent must retain exactly one allocation row");
        };
        assert_eq!(row.alloc_id, alloc_id, "reclamation restart must reuse AllocationId");
        if row.state == AllocStateWire::Running && row.restart_count == 1 {
            return row.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "same allocation must restart within {budget:?}; last row={row:#?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn poll_until_same_allocation_restart_failed(
    cfg: &Path,
    workload_id: &str,
    alloc_id: &str,
    budget: Duration,
) -> AllocStatusRowBody {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let described =
            describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_path_buf() })
                .await
                .expect("describe while waiting for same-allocation reinstall failure");
        let [row] = described.snapshot.rows.as_slice() else {
            panic!("standing one-replica intent must retain exactly one allocation row");
        };
        assert_eq!(row.alloc_id, alloc_id, "failed reinstall must retain AllocationId");
        if row.state == AllocStateWire::Failed && row.restart_count == 1 {
            return row.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "same-allocation reinstall must fail within {budget:?}; last row={row:#?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn poll_until_natural_job_completion(
    cfg: &Path,
    workload_id: &str,
    alloc_id: &str,
    budget: Duration,
) -> Result<AllocStatusRowBody, String> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let described =
            describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_path_buf() })
                .await
                .map_err(|error| format!("describe natural completion failed: {error}"))?;
        let [row] = described.snapshot.rows.as_slice() else {
            return Err(format!(
                "standing Job must retain exactly one allocation row: {:?}",
                described.snapshot.rows
            ));
        };
        if row.alloc_id != alloc_id {
            return Err(format!(
                "natural completion changed allocation id: expected {alloc_id}, got {}",
                row.alloc_id
            ));
        }
        if row.state == AllocStateWire::Terminated && row.exit_code == Some(0) {
            return Ok(row.clone());
        }
        if row.state == AllocStateWire::Failed {
            return Err(format!("restarted Job failed instead of completing naturally: {row:#?}"));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "typed natural completion did not arrive within {budget:?}; last row={row:#?}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn assert_platform_reclamation_restart(row: &AllocStatusRowBody, alloc_id: &str) {
    assert_eq!(row.alloc_id, alloc_id);
    assert_eq!(row.restart_count, 1, "exactly one restart follows one boot reclamation");
    let last = row
        .last_terminated
        .as_ref()
        .expect("the restart preserves its immediately preceding terminal occurrence");
    assert!(
        matches!(last.reason, Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed })),
        "same-id restart must be caused by Platform Reclamation: {last:#?}"
    );
}

async fn observe_restarted_mesh_flow_unchecked(
    cut: VmmSpawnCut,
    cfg: &Path,
    workload_id: &str,
    alloc_id: &str,
    peer_workload_id: &str,
    peer_wire: WireCapture,
) -> Result<
    (
        AllocStatusRowBody,
        InterceptReadiness,
        GuestEgressAudit,
        WireScan,
        KtlsSocketEvidence,
        Vec<String>,
    ),
    String,
> {
    if cut.config.alloc.as_str() != alloc_id {
        return Err(format!(
            "VMM restart changed AllocationId: expected {alloc_id}, got {}",
            cut.config.alloc
        ));
    }
    let network = cut
        .config
        .network
        .as_ref()
        .ok_or_else(|| "restart has no complete VM network plan".to_owned())?;
    let slot_hex = network
        .tap
        .rsplit('-')
        .next()
        .ok_or_else(|| "slot-derived tap suffix is absent".to_owned())?;
    let slot = NetSlot::new(
        u16::from_str_radix(slot_hex, 16)
            .map_err(|error| format!("production tap slot is not hexadecimal: {error}"))?,
    )
    .map_err(|error| format!("observed network slot is invalid: {error}"))?;
    let responder = responder_addr_for_slot(slot);
    let workload = derive_workload_netns_plan(slot, responder);
    let guest = derive_vm_tap_plan(slot, responder);
    if network.netns != workload.netns || network.tap != guest.tap {
        return Err(format!(
            "restart network plan drifted: observed={network:?}, workload={workload:?}, guest={guest:?}"
        ));
    }

    // Reclamation and the stale-rule sweep have completed before this VMM cut.
    // The replacement guard is intentionally absent here: accepted ordering is
    // driver READY → transient Running write → intercept install → EXEC release.
    poll_until_nft_rule_observer_is_quiet(Duration::from_secs(30), 0).await;

    let guest_wire = WireCapture::start(&workload.host_veth, 0);
    let (tap_wire, tap_ifindex) = WireCapture::start_in_netns(network.netns.as_str(), &network.tap);
    cut.release.send(()).map_err(|()| "restarted VMM release receiver disappeared".to_owned())?;
    let restarted =
        poll_until_same_allocation_restarted(cfg, workload_id, alloc_id, Duration::from_secs(90))
            .await;
    if restarted.alloc_id != alloc_id || restarted.restart_count != 1 {
        return Err(format!("same-id Platform Reclamation delta is wrong: {restarted:#?}"));
    }
    let last = restarted.last_terminated.as_ref().ok_or_else(|| {
        "restart omitted its immediately preceding terminal occurrence".to_owned()
    })?;
    if !matches!(last.reason, Some(TransitionReason::Stopped { by: StoppedBy::PlatformReclaimed }))
    {
        return Err(format!("restart was not caused by Platform Reclamation: {last:#?}"));
    }
    let _ = poll_until_running(cfg, peer_workload_id, Duration::from_secs(30)).await;
    // The target guest's immutable startup delay keeps its first flow parked
    // while the fresh peer reaches Running. Arm D7 only after both production
    // guards are notification-free, so later generation movement is target
    // traffic rather than unrelated peer installation.
    poll_until_nft_rule_observer_is_quiet(Duration::from_secs(30), 2).await;
    let live =
        poll_until_outbound_rule_ready(workload.host_veth.clone(), Duration::from_secs(30)).await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let ktls = poll_until_ktls(SERVICE_PORT, Duration::from_secs(25)).await.ok_or_else(|| {
        "restarted first flow did not install bidirectional TLS 1.3 kTLS".to_owned()
    })?;
    let readiness = finish_d7_observation(live, Duration::from_secs(60)).await;
    let guest_capture = guest_wire.stop();
    let tap_capture = tap_wire.stop();
    let pre_ready = [
        ("tap", &tap_capture.frames, tap_ifindex),
        ("host-veth", &guest_capture.frames, readiness.host_veth_ifindex),
    ]
    .into_iter()
    .flat_map(|(name, frames, ifindex)| {
        frames
            .iter()
            .filter(move |frame| {
                guest_frame_precedes_capture_ready(frame, ifindex, readiness.kernel_barrier_at)
            })
            .map(move |frame| format!("{name}: {frame:?}"))
    })
    .collect::<Vec<_>>();
    let mesh_destination = SocketAddrV4::new(
        Ipv4Addr::from(u32::from(WORKLOAD_FRONTEND_BASE.network()).saturating_add(1)),
        SERVICE_PORT,
    );
    let guest_addr = restarted
        .workload_addr
        .ok_or_else(|| "restarted VM omitted its guest address".to_owned())?;
    if guest_addr != guest.guest_addr {
        return Err(format!(
            "restarted guest address drifted: observed={guest_addr}, expected={}",
            guest.guest_addr
        ));
    }
    let audit = audit_guest_egress_boundary(
        &guest_capture.frames,
        &readiness,
        guest_addr,
        mesh_destination,
    );
    validate_exact_d7_accounting(&readiness, &audit, &readiness.host_veth)
        .map_err(|error| format!("restart D7 accounting mismatch: {error}"))?;
    let scan = peer_wire.stop_and_scan(Some(ktls.tuple));
    if audit.first_syn.is_none()
        || audit.plaintext_request_hits == 0
        || scan.plaintext_hits_on_any_peer_stream != 0
        || scan.exact_records_to_peer == 0
        || scan.exact_records_from_peer == 0
    {
        return Err(format!(
            "restart first-flow oracle failed: audit={audit:?}, peer_scan={scan:?}"
        ));
    }
    Ok((restarted, readiness, audit, scan, ktls, pre_ready))
}

fn panic_evidence(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map_or_else(
        || {
            payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "non-string panic payload".to_owned())
        },
        |message| (*message).to_owned(),
    )
}

async fn observe_restarted_mesh_flow(
    cut: VmmSpawnCut,
    cfg: &Path,
    workload_id: &str,
    alloc_id: &str,
    peer_workload_id: &str,
    peer_wire: WireCapture,
) -> Result<
    (
        AllocStatusRowBody,
        InterceptReadiness,
        GuestEgressAudit,
        WireScan,
        KtlsSocketEvidence,
        Vec<String>,
    ),
    String,
> {
    std::panic::AssertUnwindSafe(observe_restarted_mesh_flow_unchecked(
        cut,
        cfg,
        workload_id,
        alloc_id,
        peer_workload_id,
        peer_wire,
    ))
    .catch_unwind()
    .await
    .map_err(|payload| {
        format!("restart observation failed before cleanup: {}", panic_evidence(payload.as_ref()))
    })?
}

async fn finish_after_authoritative_cleanup<T, P, S>(
    observation: Result<T, String>,
    peer_cleanup: P,
    server_cleanup: S,
) -> Result<T, String>
where
    P: std::future::Future<Output = Result<(), String>>,
    S: std::future::Future<Output = Result<(), String>>,
{
    // Stop the peer through the still-live control plane first, then drain the
    // server owner. Store (rather than `?`-propagate) the peer result so a peer
    // error still cannot bypass authoritative server teardown.
    let peer = peer_cleanup.await;
    let server = server_cleanup.await;
    match (observation, peer, server) {
        (Ok(observation), Ok(()), Ok(())) => Ok(observation),
        (observation, peer, server) => Err(format!(
            "observation={:?}; peer_cleanup={peer:?}; server_cleanup={server:?}",
            observation.err()
        )),
    }
}

/// CONTRACT_SHAPE: bounded-change (an early observation failure still awaits every fixture owner).
#[tokio::test]
async fn restart_observation_failure_awaits_cleanup_before_reporting() {
    let peer_cleaned = Arc::new(AtomicBool::new(false));
    let server_cleaned = Arc::new(AtomicBool::new(false));
    let result: Result<(), String> = tokio::time::timeout(
        Duration::from_secs(1),
        finish_after_authoritative_cleanup(
            Err("injected early observation assertion".to_owned()),
            {
                let peer_cleaned = Arc::clone(&peer_cleaned);
                async move {
                    peer_cleaned.store(true, Ordering::SeqCst);
                    Ok(())
                }
            },
            {
                let server_cleaned = Arc::clone(&server_cleaned);
                async move {
                    server_cleaned.store(true, Ordering::SeqCst);
                    Ok(())
                }
            },
        ),
    )
    .await
    .expect("injected early observation failure returns within its own one-second bound");
    assert!(result.is_err());
    assert!(peer_cleaned.load(Ordering::SeqCst));
    assert!(server_cleaned.load(Ordering::SeqCst));
}

/// S-GTI-06a — an unclean `serve` restart with standing intent reclaims and
/// restarts the same allocation, then reinstalls the exact guard before EXEC.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_restarted_microvm_workload_is_re_enrolled_in_the_mesh_before_it_runs_again() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision VM fixture");
    let server_tmp = tempfile::Builder::new()
        .prefix("gti-restart-reinstall-")
        .tempdir_in(shared_staging_root())
        .expect("restart fixture tempdir on metal staging root");
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");
    let peer = build_mesh_peer(server_tmp.path());
    // The same immutable guest image is used on both boots. Its bounded
    // startup delay leaves boot one enough time to lose ownership before the
    // first dial and leaves boot two enough time to arm the exact D7 witness
    // after the production reinstall releases EXEC.
    let guest = build_mesh_guest_with_timing(server_tmp.path(), "gti-restart-mesh-guest", 15, 12);
    let rootfs =
        stage_rootfs_with_extra_binary(server_tmp.path(), &fixture, &guest, "gti-restart-guest");

    let (boot_one, boot_one_cuts) =
        spawn_capture_observed_mtls_server_at(&data_dir, &config_dir).await;
    let cfg = config_path(server_tmp.path());
    let vm_spec = write_toml(
        server_tmp.path(),
        "gti-restart-vm.toml",
        &vm_job_toml(
            "zzzz-gti-restart-vm",
            "/sbin/gti-restart-guest",
            &[],
            &fixture.kernel_path,
            &rootfs,
        ),
    );
    let vm = deploy(DeployArgs { spec: vm_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the VM exactly once");
    let boot_one_config = release_vmm_without_capture(&boot_one_cuts);
    let first = poll_until_running(&cfg, &vm.workload_id, Duration::from_secs(60)).await;
    let first_row = first.snapshot.rows.first().expect("one first-boot allocation");
    assert_eq!(first_row.alloc_id, boot_one_config.alloc.as_str());
    assert_eq!(first_row.restart_count, 0);
    assert_eq!(first_row.last_terminated, None);
    let alloc_id = first_row.alloc_id.clone();

    // The first boot owns only the target VM. The peer is deliberately created
    // by the replacement control plane, so this scenario cannot accidentally
    // pass by reconstructing a live survivor from the dead process's userspace
    // state. The unchanged target intent and durable Running row are the only
    // inputs boot two may use.
    poll_until_nft_rule_observer_is_quiet(Duration::from_secs(30), 1).await;
    assert_allocation_process_is_live(&boot_one_config.alloc);

    // Deliberately do not issue stop/restart/deploy. Abruptly revoking the only
    // serve owner models process ownership loss. Boot two must reclaim the
    // unsupervised VM, durably record Platform Reclamation, and let ordinary
    // reconciliation restart the same allocation id.
    let _ = boot_one.abort_for_test().await;
    wait_for_data_dir_release().await;

    let peer_wire = WireCapture::start(LOOPBACK_IFACE, SERVICE_PORT);
    let (boot_two, boot_two_cuts) =
        spawn_capture_observed_mtls_server_at(&data_dir, &config_dir).await;
    let boot_two_ready = std::panic::AssertUnwindSafe(async {
        let restart_cut = boot_two_cuts
            .recv_timeout(Duration::from_secs(30))
            .map_err(|error| format!("observe boot-two VMM cut: {error}"))?;
        let service_spec =
            write_toml(server_tmp.path(), "gti-restart-peer.toml", &service_toml(&peer));
        let service = deploy(DeployArgs { spec: service_spec, config_path: cfg.clone() })
            .await
            .map_err(|error| format!("deploy fresh boot-two mesh peer: {error}"))?;
        Ok((restart_cut, service))
    })
    .catch_unwind()
    .await
    .map_err(|payload| format!("boot-two readiness panicked: {}", panic_evidence(payload.as_ref())))
    .and_then(|result| result);
    let observation = match boot_two_ready {
        Ok((restart_cut, service)) => observe_restarted_mesh_flow(
            restart_cut,
            &cfg,
            &vm.workload_id,
            &alloc_id,
            &service.workload_id,
            peer_wire,
        )
        .await
        .map(|observation| (observation, service)),
        Err(error) => Err(error),
    };

    // The same guest command returns naturally after its authenticated reply;
    // no stop manufactures the Job result.
    let terminal = if observation.is_ok() {
        poll_until_natural_job_completion(
            &cfg,
            &vm.workload_id,
            &alloc_id,
            Duration::from_secs(120),
        )
        .await
    } else {
        Err("natural completion skipped after observation failure".to_owned())
    };

    let peer_workload_id = observation
        .as_ref()
        .map_or_else(|_| "server".to_owned(), |(_, service)| service.workload_id.clone());
    let cleanup_result = finish_after_authoritative_cleanup(
        observation.map(|(observation, _)| observation),
        std::panic::AssertUnwindSafe(async {
            stop(StopArgs { id: peer_workload_id, config_path: cfg.clone() })
                .await
                .map_err(|error| format!("stop independent mesh peer: {error}"))?;
            Ok(())
        })
        .catch_unwind()
        .map(|result| {
            result.unwrap_or_else(|payload| {
                Err(format!("peer cleanup panicked: {}", panic_evidence(payload.as_ref())))
            })
        }),
        async {
            boot_two
                .shutdown()
                .await
                .map_err(|error| format!("clean boot-two serve shutdown: {error}"))
        },
    )
    .await;

    // Assert only after every fixture owner has been synchronously drained, so
    // a future regression reports promptly instead of leaking into nextest's
    // 240-second timeout.
    let (restarted, _readiness, _audit, _scan, _ktls, pre_readiness_frames) =
        cleanup_result.expect("restart observation and authoritative cleanup must converge");
    let row = terminal.expect("restarted Job must reach typed natural completion");
    assert_eq!(row.state, AllocStateWire::Terminated);
    assert_eq!(row.exit_code, Some(0));
    assert_eq!(row.alloc_id, alloc_id);
    assert_eq!(row.restart_count, 1);
    assert_eq!(restarted.alloc_id, alloc_id);
    assert!(
        pre_readiness_frames.is_empty(),
        "restarted guest emits no frame before exact reinstall: {pre_readiness_frames:#?}"
    );
}

/// S-GTI-06b — reinstall failure on the real INPUT-hook rejection is terminal
/// and never releases EXEC or a guest-originated frame.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn failed_re_enrolment_after_platform_reclamation_stays_closed() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision VM fixture");
    let server_tmp = tempfile::Builder::new()
        .prefix("gti-restart-reinstall-failure-")
        .tempdir_in(shared_staging_root())
        .expect("restart-failure fixture tempdir on metal staging root");
    let data_dir = server_tmp.path().join("data");
    let config_dir = server_tmp.path().join("conf");
    let marker = build_persistent_operator_marker(server_tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(
        server_tmp.path(),
        &fixture,
        &marker,
        "gti-persistent-operator-marker",
    );

    let (boot_one, boot_one_cuts) =
        spawn_capture_observed_mtls_server_at(&data_dir, &config_dir).await;
    let cfg = config_path(server_tmp.path());
    let spec = write_toml(
        server_tmp.path(),
        "gti-restart-reinstall-failure.toml",
        &vm_job_toml(
            "gti-restart-reinstall-failure",
            "/sbin/gti-persistent-operator-marker",
            &[],
            &fixture.kernel_path,
            &rootfs,
        ),
    );
    let submit = deploy(DeployArgs { spec, config_path: cfg.clone() })
        .await
        .expect("deploy restart-failure VM exactly once");
    let first_config = release_vmm_without_capture(&boot_one_cuts);
    let first = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    let alloc_id = first.snapshot.rows[0].alloc_id.clone();
    assert_eq!(first_config.alloc.as_str(), alloc_id);
    assert_allocation_process_is_live(&first_config.alloc);
    let _ = boot_one.abort_for_test().await;
    wait_for_data_dir_release().await;

    let exact_before = fault_fixture::PacketPathBaseline::capture();
    let malformed = fault_fixture::ProductInputHookFixture::install();
    let (boot_two, cuts, created, boundary, _terminator) =
        spawn_failure_observed_mtls_server_at(&data_dir, &config_dir).await;
    // Reclamation runs before ordinary reconciliation. The replacement VM
    // therefore reaches the production VMM boundary, emits READY, and only
    // then encounters the real post-Running intercept-install rejection.
    let capture = arm_failure_capture(&cuts);
    let control = created
        .recv_timeout(Duration::from_secs(30))
        .expect("replacement VMM is created before post-READY guard installation");
    let row = poll_until_same_allocation_restart_failed(
        &cfg,
        &submit.workload_id,
        &alloc_id,
        Duration::from_secs(90),
    )
    .await;
    assert_eq!(row.alloc_id, alloc_id);
    assert_eq!(row.state, AllocStateWire::Failed);
    assert_platform_reclamation_restart(&row, &alloc_id);
    match row.reason.as_ref() {
        Some(TransitionReason::MtlsInterceptInstallFailed { stage, detail }) => {
            assert_eq!(stage, "outbound_tproxy_install");
            assert!(detail.contains("append-egress") && detail.contains("append-rule"));
            assert!(detail.contains("Operation not supported") || detail.contains("os error 95"));
        }
        other => panic!("same-id reinstall preserves the typed INPUT-hook cause: {other:?}"),
    }
    assert_failed_vm_cleanup(&server_tmp, &rootfs, &alloc_id, &capture, &control).await;
    assert_guest_boundary(
        &boundary,
        &alloc_id,
        false,
        GuestBeaconTrace { ready: 1, exec: 0, exit: 0 },
    );
    assert_zero_guest_originated_frames(capture);
    malformed.finish();
    assert_eq!(
        fault_fixture::PacketPathBaseline::capture(),
        exact_before,
        "fixture and failed reinstall restore the complete exact target-filtered packet path"
    );
    boot_two.shutdown().await.expect("clean failed-restart server shutdown");
}

async fn run_resolver_failure_closure(label: &str) {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(shared_staging_root())
        .expect("resolver-failure tempdir on metal staging root");
    let sibling_dir = tmp.path().join("sibling");
    let target_dir = tmp.path().join("target");
    std::fs::create_dir_all(&sibling_dir).expect("create sibling staging dir");
    std::fs::create_dir_all(&target_dir).expect("create target staging dir");
    let spin = build_spin_binary(&sibling_dir);
    let sibling_rootfs =
        stage_rootfs_with_extra_binary(&sibling_dir, &fixture, &spin, "gti-resolver-sibling");
    let forbidden = build_forbidden_exec_probe(&target_dir, "gti-forbidden-resolver-exec");
    let target_rootfs = stage_rootfs_with_extra_binary(
        &target_dir,
        &fixture,
        &forbidden,
        "gti-forbidden-resolver-exec",
    );
    replace_resolver_with_directory(&target_rootfs, &target_dir);

    let (handle, server_tmp, cuts, created, marker_observed, _terminator) =
        spawn_failure_observed_mtls_server().await;
    let cfg = config_path(server_tmp.path());
    let sibling_spec = write_toml(
        server_tmp.path(),
        "gti-resolver-sibling.toml",
        &vm_job_toml(
            "gti-resolver-sibling",
            "/sbin/gti-resolver-sibling",
            &[],
            &fixture.kernel_path,
            &sibling_rootfs,
        ),
    );
    let sibling_submit = deploy(DeployArgs { spec: sibling_spec, config_path: cfg.clone() })
        .await
        .expect("deploy independent resolver sibling");
    let sibling_config = release_vmm_without_capture(&cuts);
    let _sibling_control =
        created.recv_timeout(Duration::from_secs(30)).expect("observe sibling VMM creation");
    let sibling_before =
        poll_until_running(&cfg, &sibling_submit.workload_id, Duration::from_secs(60)).await;
    let sibling_row_before = sibling_before.snapshot.rows[0].clone();
    let sibling_host_veth = host_veth_for_config(&sibling_config);
    let sibling_rule_before = poll_until_outbound_rule_snapshot(&sibling_host_veth).await;
    let packet_path_before = fault_fixture::PacketPathBaseline::capture();

    let target_spec = write_toml(
        server_tmp.path(),
        "gti-resolver-target.toml",
        &vm_job_toml(
            "gti-resolver-target",
            "/sbin/gti-forbidden-resolver-exec",
            &[],
            &fixture.kernel_path,
            &target_rootfs,
        ),
    );
    let target_submit = deploy(DeployArgs { spec: target_spec, config_path: cfg.clone() })
        .await
        .expect("deploy resolver-failure VM");
    let target_capture = arm_failure_capture(&cuts);
    let target_control = created
        .recv_timeout(Duration::from_secs(30))
        .expect("observe resolver-failure VMM creation");
    let (failed, durable) = poll_until_failed_without_running(
        &server_tmp,
        &cfg,
        &target_submit.workload_id,
        Duration::from_secs(90),
    )
    .await;
    let row = failed.snapshot.rows.first().expect("one resolver-failure allocation");
    assert_exact_pre_ready_failure(row, &durable);
    let detail = row.error.as_deref().expect("guest-console diagnostic is projected");
    assert!(detail.contains("write /etc/resolv.conf"), "resolver stage is retained: {detail}");
    assert!(
        detail.contains("Is a directory") || detail.contains("os error 21"),
        "the real resolver errno is retained: {detail}"
    );
    assert_failed_vm_cleanup(
        &server_tmp,
        &target_rootfs,
        &row.alloc_id,
        &target_capture,
        &target_control,
    )
    .await;
    assert_guest_boundary(
        &marker_observed,
        &row.alloc_id,
        false,
        GuestBeaconTrace { ready: 0, exec: 0, exit: 0 },
    );
    assert_zero_guest_originated_frames(target_capture);
    assert_eq!(
        fault_fixture::PacketPathBaseline::capture(),
        packet_path_before,
        "resolver failure removes exactly the target delta and preserves every pre-existing nft/FIB object, order, program, handle, userdata, and counter",
    );

    let first_stop =
        stop(StopArgs { id: target_submit.workload_id.clone(), config_path: cfg.clone() })
            .await
            .expect("first stop records intent even after terminal pre-READY failure");
    assert_eq!(first_stop.outcome, StopOutcome::Stopped);
    let second_stop =
        stop(StopArgs { id: target_submit.workload_id.clone(), config_path: cfg.clone() })
            .await
            .expect("second stop is idempotent");
    assert_eq!(second_stop.outcome, StopOutcome::AlreadyStopped);
    assert_eq!(
        fault_fixture::PacketPathBaseline::capture(),
        packet_path_before,
        "the explicit Stopped then AlreadyStopped replay neither recreates nor re-deletes the absent guard",
    );

    let sibling_after =
        describe(DescribeArgs { id: sibling_submit.workload_id.clone(), config_path: cfg.clone() })
            .await
            .expect("describe independent sibling after resolver failure");
    assert_eq!(sibling_after.snapshot.rows[0], sibling_row_before);
    assert_eq!(
        outbound_rule_snapshot(&sibling_host_veth),
        Ok(Some(sibling_rule_before)),
        "resolver failure and cleanup preserve the independent exact rule"
    );
    stop(StopArgs { id: sibling_submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop resolver sibling");
    let _ = poll_until_terminal(&cfg, &sibling_submit.workload_id, Duration::from_secs(30)).await;
    handle.shutdown().await.expect("clean resolver server shutdown");
}

/// S-GTI-08a — guest resolver failure is a classified pre-READY refusal.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_microvm_that_cannot_address_its_network_is_refused_as_a_boot_failure() {
    run_resolver_failure_closure("gti-resolver-refusal-").await;
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn failed_start_cleanup_removes_all_residue_and_preserves_the_independent_allocation() {
    run_resolver_failure_closure("gti-resolver-cleanup-").await;
}

/// S-GTI-08b — status 78 after READY and EXEC is an ordinary Job result.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn operator_exit_78_after_ready_is_an_ordinary_result() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("gti-exit-78-")
        .tempdir_in(shared_staging_root())
        .expect("exit-78 tempdir on metal staging root");
    let exit_78 = build_exit_78_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &exit_78, "gti-exit-78");
    let (handle, server_tmp, cuts, created, marker_observed, _terminator) =
        spawn_failure_observed_mtls_server().await;
    let cfg = config_path(server_tmp.path());
    let spec = write_toml(
        server_tmp.path(),
        "gti-exit-78.toml",
        &vm_job_toml("gti-exit-78", "/sbin/gti-exit-78", &[], &fixture.kernel_path, &rootfs),
    );
    let submit =
        deploy(DeployArgs { spec, config_path: cfg.clone() }).await.expect("deploy exit-78 VM");
    let _config = release_vmm_without_capture(&cuts);
    let _control = created.recv_timeout(Duration::from_secs(30)).expect("observe exit-78 VMM");
    let running = poll_until_running(&cfg, &submit.workload_id, Duration::from_secs(60)).await;
    assert_eq!(running.snapshot.rows[0].restart_count, 0);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let final_row = loop {
        let described =
            describe(DescribeArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
                .await
                .expect("describe exit-78 VM");
        if let Some(row) = described.snapshot.rows.first()
            && row.state == AllocStateWire::Failed
            && row.exit_code == Some(78)
        {
            break row.clone();
        }
        assert!(tokio::time::Instant::now() < deadline, "exit 78 must finalize within 60s");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(
        matches!(
            final_row.reason,
            Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(78),
                signal: None,
                ..
            })
        ),
        "post-READY EXIT 78 remains the guest's ordinary operator result: {:?}",
        final_row.reason
    );
    assert_eq!(final_row.exit_code, Some(78));
    let durable = durable_alloc_snapshot(&server_tmp, &final_row.alloc_id, "exit-78-final", 1)
        .await
        .expect("durable exit-78 allocation row");
    assert_eq!(durable.terminal, Some(TerminalCondition::Failed { exit_code: Some(78) }));
    assert_eq!(final_row.restart_count, 0);
    assert!(final_row.started_at.is_some(), "READY produced the prior Running row");
    assert_guest_boundary(
        &marker_observed,
        &final_row.alloc_id,
        true,
        GuestBeaconTrace { ready: 1, exec: 1, exit: 1 },
    );
    handle.shutdown().await.expect("clean exit-78 server shutdown");
}

/// M-GTI-INTERRUPT-BOOT — externally terminating the real VMM before READY
/// follows the same fail-closed classification and total cleanup path.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn interrupting_the_real_vmm_before_ready_fails_closed_and_cleans_up() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("gti-interrupt-boot-")
        .tempdir_in(shared_staging_root())
        .expect("interruption tempdir on metal staging root");
    let sibling_dir = tmp.path().join("sibling");
    let target_dir = tmp.path().join("target");
    std::fs::create_dir_all(&sibling_dir).expect("create sibling staging dir");
    std::fs::create_dir_all(&target_dir).expect("create target staging dir");
    let spin = build_spin_binary(&sibling_dir);
    let sibling_rootfs =
        stage_rootfs_with_extra_binary(&sibling_dir, &fixture, &spin, "gti-interrupt-sibling");
    let forbidden = build_forbidden_exec_probe(&target_dir, "gti-forbidden-interrupt-exec");
    let target_rootfs = stage_rootfs_with_extra_binary(
        &target_dir,
        &fixture,
        &forbidden,
        "gti-forbidden-interrupt-exec",
    );
    let (handle, server_tmp, cuts, created, marker_observed, terminator) =
        spawn_failure_observed_mtls_server().await;
    let cfg = config_path(server_tmp.path());
    let sibling_spec = write_toml(
        server_tmp.path(),
        "gti-interrupt-sibling.toml",
        &vm_job_toml(
            "gti-interrupt-sibling",
            "/sbin/gti-interrupt-sibling",
            &[],
            &fixture.kernel_path,
            &sibling_rootfs,
        ),
    );
    let sibling_submit = deploy(DeployArgs { spec: sibling_spec, config_path: cfg.clone() })
        .await
        .expect("deploy interruption sibling");
    let sibling_config = release_vmm_without_capture(&cuts);
    let _sibling_control =
        created.recv_timeout(Duration::from_secs(30)).expect("observe sibling VMM creation");
    let sibling_before =
        poll_until_running(&cfg, &sibling_submit.workload_id, Duration::from_secs(60)).await;
    let sibling_row_before = sibling_before.snapshot.rows[0].clone();
    let sibling_host_veth = host_veth_for_config(&sibling_config);
    let sibling_rule_before = poll_until_outbound_rule_snapshot(&sibling_host_veth).await;
    let packet_path_before = fault_fixture::PacketPathBaseline::capture();

    let target_spec = write_toml(
        server_tmp.path(),
        "gti-interrupt-target.toml",
        &vm_job_toml(
            "gti-interrupt-target",
            "/sbin/gti-forbidden-interrupt-exec",
            &[],
            &fixture.kernel_path,
            &target_rootfs,
        ),
    );
    let target_submit = deploy(DeployArgs { spec: target_spec, config_path: cfg.clone() })
        .await
        .expect("deploy externally interrupted VM");
    let target_capture = arm_failure_capture(&cuts);
    let target_control = created
        .recv_timeout(Duration::from_secs(30))
        .expect("observe real target VMM before external termination");
    wait_until_vmm_is_in_its_cgroup(&target_capture.alloc, target_control.pid).await;
    terminator
        .terminate(&target_control, Duration::ZERO)
        .await
        .expect("externally terminate the real VMM before READY");
    let (failed, durable) = poll_until_failed_without_running(
        &server_tmp,
        &cfg,
        &target_submit.workload_id,
        Duration::from_secs(90),
    )
    .await;
    let row = failed.snapshot.rows.first().expect("one interrupted allocation");
    assert_exact_pre_ready_failure(row, &durable);
    assert_failed_vm_cleanup(
        &server_tmp,
        &target_rootfs,
        &row.alloc_id,
        &target_capture,
        &target_control,
    )
    .await;
    assert_guest_boundary(
        &marker_observed,
        &row.alloc_id,
        false,
        GuestBeaconTrace { ready: 0, exec: 0, exit: 0 },
    );
    assert_zero_guest_originated_frames(target_capture);
    assert_eq!(
        fault_fixture::PacketPathBaseline::capture(),
        packet_path_before,
        "pre-READY interruption removes exactly the target delta and preserves every pre-existing nft/FIB object, order, program, handle, userdata, and counter",
    );
    let sibling_after =
        describe(DescribeArgs { id: sibling_submit.workload_id.clone(), config_path: cfg.clone() })
            .await
            .expect("describe sibling after target interruption");
    assert_eq!(sibling_after.snapshot.rows[0], sibling_row_before);
    assert_eq!(outbound_rule_snapshot(&sibling_host_veth), Ok(Some(sibling_rule_before)));
    stop(StopArgs { id: sibling_submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop interruption sibling");
    let _ = poll_until_terminal(&cfg, &sibling_submit.workload_id, Duration::from_secs(30)).await;
    handle.shutdown().await.expect("clean interruption server shutdown");
}

fn stable_full_rule_snapshot() -> nft::RuleSnapshot {
    let mut observer = nft::NftRuleObserver::subscribe().expect("subscribe strict nft observer");
    let first = observer
        .snapshot("overdrive-mtls", "prerouting")
        .expect("first strict full-chain snapshot");
    let second = observer
        .snapshot("overdrive-mtls", "prerouting")
        .expect("second strict full-chain snapshot");
    assert_ne!(first.generation, 0, "full ruleset generation is non-zero");
    assert_eq!(first.generation, second.generation, "quiet full-chain generation is stable");
    assert_eq!(first.rules, second.rules, "quiet full ordered rule sequence is stable");
    second
}

/// S-GTI-12a — stop removes exactly one allocation-owned guard and preserves
/// the complete ordered full-chain complement, including the sibling rule.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn a_stopped_microvm_workloads_egress_mesh_guard_is_torn_down_never_left_behind() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("gti-stop-exact-")
        .tempdir_in(shared_staging_root())
        .expect("stop fixture tempdir on metal staging root");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "gti-stop-spin");
    let (handle, server_tmp, cuts) = spawn_capture_observed_mtls_server().await;
    let cfg = config_path(server_tmp.path());

    let target_spec = write_toml(
        server_tmp.path(),
        "gti-stop-target.toml",
        &vm_job_toml("gti-stop-target", "/sbin/gti-stop-spin", &[], &fixture.kernel_path, &rootfs),
    );
    let target = deploy(DeployArgs { spec: target_spec, config_path: cfg.clone() })
        .await
        .expect("deploy stop target");
    let target_config = release_vmm_without_capture(&cuts);
    let _ = poll_until_running(&cfg, &target.workload_id, Duration::from_secs(60)).await;

    let sibling_spec = write_toml(
        server_tmp.path(),
        "gti-stop-sibling.toml",
        &vm_job_toml("gti-stop-sibling", "/sbin/gti-stop-spin", &[], &fixture.kernel_path, &rootfs),
    );
    let sibling = deploy(DeployArgs { spec: sibling_spec, config_path: cfg.clone() })
        .await
        .expect("deploy independent stop sibling");
    let sibling_config = release_vmm_without_capture(&cuts);
    let sibling_before =
        poll_until_running(&cfg, &sibling.workload_id, Duration::from_secs(60)).await.snapshot.rows
            [0]
        .clone();

    let target_host_veth = host_veth_for_config(&target_config);
    let sibling_host_veth = host_veth_for_config(&sibling_config);
    let before = stable_full_rule_snapshot();
    let target_rule = exact_d7_target(&before.rules, &target_host_veth)
        .expect("target has exactly one typed allocation rule");
    let sibling_rule = exact_d7_target(&before.rules, &sibling_host_veth)
        .expect("sibling has exactly one typed allocation rule");
    assert_ne!(target_rule.handle, sibling_rule.handle);

    let stopped = stop(StopArgs { id: target.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("drive the real job stop command library");
    assert_eq!(stopped.outcome, StopOutcome::Stopped);
    let _ = poll_until_terminal(&cfg, &target.workload_id, Duration::from_secs(30)).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let after = loop {
        let snapshot = stable_full_rule_snapshot();
        if snapshot.rules.iter().all(|rule| rule.handle != target_rule.handle) {
            break snapshot;
        }
        assert!(tokio::time::Instant::now() < deadline, "target guard is removed within 30s");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let expected = before
        .rules
        .into_iter()
        .filter(|rule| rule.handle != target_rule.handle)
        .collect::<Vec<_>>();
    assert_eq!(
        after.rules, expected,
        "the ordered full after-snapshot equals before filtered only by the exact target handle"
    );
    assert_eq!(
        outbound_rule_snapshot(&sibling_host_veth),
        Ok(Some(sibling_rule)),
        "sibling handle/userdata/program/counter remain byte-for-byte exact"
    );
    let sibling_after =
        describe(DescribeArgs { id: sibling.workload_id.clone(), config_path: cfg.clone() })
            .await
            .expect("describe sibling after exact target stop");
    assert_eq!(sibling_after.snapshot.rows[0], sibling_before);

    stop(StopArgs { id: sibling.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop independent sibling");
    let _ = poll_until_terminal(&cfg, &sibling.workload_id, Duration::from_secs(30)).await;
    handle.shutdown().await.expect("clean stop server shutdown");
}

/// S-GTI-12b — a terminal pre-READY VM accepts one stop intent, then reports
/// AlreadyStopped without recreating its absent guard on later convergence.
/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
async fn job_stop_without_a_guest_egress_guard_is_idempotent() {
    run_resolver_failure_closure("gti-terminal-stop-idempotent-").await;
}

#[path = "guest_stack_mtls_egress_fault_fixture.rs"]
mod fault_fixture;
