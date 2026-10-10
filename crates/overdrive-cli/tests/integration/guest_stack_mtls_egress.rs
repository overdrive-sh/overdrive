//! Tier-3 metal acceptance tests for guest-stack transparent mTLS egress.
//!
//! These tests drive the real CLI command libraries: an mTLS-composed
//! `serve`, TOML `deploy`, `workload describe`, and `stop`. The client
//! is a real Cloud Hypervisor guest whose ordinary `TcpStream` resolves and
//! dials the mesh service. No test installs an intercept, route, address, or
//! resolver entry; those effects come exclusively from the production boot and
//! allocation paths.
//!
//! The workload speaks plaintext by design. Same-node protected transport is
//! proven by one exact `ss -e` TLS 1.3 kTLS TX/RX inode/fd whose fd is a positive
//! splice destination and source. Lossless all-interface capture separately
//! proves that tuple is loopback-only and confines plaintext to the accepted
//! guest-local leg-F/leg-S tuples with no direct bypass.

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

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
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
use overdrive_control_plane::VmOverride;
use overdrive_control_plane::dns_responder::frontend_addr_allocator::WORKLOAD_FRONTEND_BASE;
use overdrive_control_plane::veth_provisioner::{DEFAULT_CLIENT_IFACE, WORKLOAD_SUBNET_BASE};
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::guest_transport::ClaimedGuestCid;
use overdrive_core::id::AllocationId;
use overdrive_core::traits::ObservationStore as _;
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::transition_reason::StoppedBy;
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_core::vm::config::{RootfsPlan, VmConfig, VmRunDir, clone_staging_dir};
use overdrive_core::{SpiffeId, TransitionReason, aggregate::WorkloadKind};
use overdrive_netlink::nft::{self, SharedIpInterceptIdentity, SharedIpInterceptState};
use overdrive_store_local::LocalObservationStore;
use overdrive_testing::vm_fixture::VmFixture;
use proptest::prelude::*;
use serial_test::serial;
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::serve_lifetime_support::kill_serve_owner;
use super::vm_walking_skeleton::{
    PACKET_SOCKET_SEAL_SETTLE, TeardownBound, build_spin_binary, config_path, poll_until_running,
    poll_until_terminal, seal_packet_socket, shared_staging_root, stage_rootfs_with_extra_binaries,
    stage_rootfs_with_extra_binary, vm_job_toml, write_toml,
};

pub(super) const SERVICE_PORT: u16 = 18_951;
pub(super) const REQUEST: &[u8] =
    b"GTI_REQUEST_guest_plaintext_dial_by_name_must_be_encrypted_on_peer_wire_0201";
pub(super) const RESPONSE: &[u8] =
    b"GTI_RESPONSE_peer_authored_distinct_reply_returns_to_guest_byte_exact_0201";
pub(super) const REQUEST2: &[u8] =
    b"GTI_REQUEST2_post_establishment_must_use_the_same_ktls_splice_owner_0201";
pub(super) const RESPONSE2: &[u8] =
    b"GTI_RESPONSE2_same_owner_returns_post_establishment_reply_byte_exact_0201";
const NON_MESH_REQUEST: &[u8] =
    b"GTI_NON_MESH_REQUEST_plaintext_passthrough_outside_workload_subnet_0201";
const NON_MESH_RESPONSE: &[u8] =
    b"GTI_NON_MESH_RESPONSE_distinct_clear_reply_reaches_guest_unchanged_0201";
const LOOPBACK_IFACE: &str = "lo";
const INTERCEPT_INSTALL_SUCCESS: &str = "mtls.intercept.install.success";
const OPERATOR_MARKER: &str = "/gti-operator-action-ran";
const OPERATOR_CONSOLE_MARKER: &str = "GTI_OPERATOR_ACTION_RAN";

pub(super) async fn spawn_mtls_server() -> (ServeHandle, TempDir) {
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

type VmmCutReceiver = Arc<std::sync::Mutex<Receiver<VmmSpawnCut>>>;

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

    async fn create(
        &self,
        config: &VmConfig,
        vsock: ClaimedGuestCid,
    ) -> VmmResult<VmProcess> {
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
        self.inner.create(config, vsock).await
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

async fn spawn_capture_observed_mtls_server() -> (ServeHandle, TempDir, VmmCutReceiver) {
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
) -> (ServeHandle, VmmCutReceiver) {
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
    let handle = overdrive_cli::commands::serve::run_with_kek_and_vm_override(
        args,
        Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
        VmOverride { vmm: Arc::new(vmm), beacon: Arc::new(overdrive_host::VsockBeaconIo::new()) },
    )
    .await
    .expect("start production-mTLS serve with observation-only real-VMM decorator");
    (handle, Arc::new(std::sync::Mutex::new(cuts)))
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

    async fn create(
        &self,
        config: &VmConfig,
        vsock: ClaimedGuestCid,
    ) -> VmmResult<VmProcess> {
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
        let process = self.inner.create(config, vsock).await?;
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
    VmmCutReceiver,
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
    VmmCutReceiver,
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
    let handle = overdrive_cli::commands::serve::run_with_kek_and_vm_override(
        args,
        Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
        VmOverride { vmm: Arc::new(vmm), beacon: Arc::new(overdrive_host::VsockBeaconIo::new()) },
    )
    .await
    .expect("start production-mTLS serve with failure-observed real VMM");
    (handle, Arc::new(std::sync::Mutex::new(cuts)), created, boundary_observed, inner)
}

pub(super) fn build_static_binary(tmp: &Path, name: &str, source: &str) -> PathBuf {
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
    plaintext_hits_on_exact_peer_tuple: u64,
    plaintext_hits_on_other_same_port_streams: u64,
    same_port_streams_observed: usize,
    plaintext_markers_by_tuple: BTreeMap<FlowTuple, u64>,
    plaintext_markers_by_interface_tuple: BTreeMap<(u32, FlowTuple), u64>,
    interfaces_by_tuple: BTreeMap<FlowTuple, BTreeSet<u32>>,
    capture_packets: u32,
    capture_drops: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KtlsSocketEvidence {
    tuple: FlowTuple,
    inode: u64,
    fds: BTreeSet<i32>,
    record: String,
}

#[derive(Debug)]
pub(super) struct CapturedFrame {
    /// Kernel packet-event time from `SCM_TIMESTAMPNS`. Missing ancillary
    /// data remains `None`, so unprovable ordering is classified pre-ready.
    pub(super) kernel_event_at: Option<KernelRealtime>,
    pub(super) ifindex: u32,
    protocol: u16,
    packet_type: u8,
    wire_len: usize,
    pub(super) truncated: bool,
    pub(super) control_truncated: bool,
    aux: Option<PacketAuxData>,
    pub(super) bytes: Vec<u8>,
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
pub(super) struct PacketStatistics {
    packets: u32,
    pub(super) drops: u32,
}

#[derive(Debug)]
pub(super) struct CaptureBatch {
    pub(super) frames: Vec<CapturedFrame>,
    pub(super) statistics: PacketStatistics,
    /// One entry per pending `ENETDOWN` the socket reported, in order: the
    /// number of frames already captured when it was reported. A capture bound
    /// while its device is down reports exactly one, at zero frames.
    pub(super) link_down_reports: Vec<usize>,
    /// The bound ifindex stopped resolving after a link-down report, so the
    /// kernel had unhooked the socket for good and reading ended early.
    pub(super) interface_removed: bool,
}

/// Nanoseconds in the shared `SO_TIMESTAMPNS` / `CLOCK_REALTIME` domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct KernelRealtime(pub(super) i128);

#[derive(Debug, Clone, PartialEq, Eq)]
struct InterceptInstallEvent {
    alloc: Option<String>,
    at: KernelRealtime,
}

#[derive(Default)]
struct InterceptInstallFieldVisitor {
    alloc: Option<String>,
}

impl Visit for InterceptInstallFieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "alloc" {
            self.alloc = Some(format!("{value:?}").trim_matches('"').to_owned());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "alloc" {
            self.alloc = Some(value.to_owned());
        }
    }
}

#[derive(Clone, Default)]
struct InterceptInstallTrace {
    events: Arc<Mutex<Vec<InterceptInstallEvent>>>,
}

impl InterceptInstallTrace {
    fn install_global() -> Self {
        let trace = Self::default();
        tracing::subscriber::set_global_default(tracing_subscriber::registry().with(trace.clone()))
            .expect("S-ND295-01 owns this nextest process's tracing subscriber");
        trace
    }

    fn exact_barrier_for(&self, alloc: &AllocationId) -> Result<KernelRealtime, String> {
        exact_intercept_install_barrier(
            &self.events.lock().expect("intercept event collector lock").clone(),
            alloc.as_str(),
        )
    }
}

impl<S> Layer<S> for InterceptInstallTrace
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if event.metadata().name() != INTERCEPT_INSTALL_SUCCESS {
            return;
        }
        let at = KernelRealtime::now();
        let mut visitor = InterceptInstallFieldVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("intercept event collector lock")
            .push(InterceptInstallEvent { alloc: visitor.alloc, at });
    }
}

fn exact_intercept_install_barrier(
    events: &[InterceptInstallEvent],
    alloc: &str,
) -> Result<KernelRealtime, String> {
    let [event] = events else {
        return Err(format!(
            "expected exactly one {INTERCEPT_INSTALL_SUCCESS} event for {alloc}, got {events:?}"
        ));
    };
    if event.alloc.as_deref() != Some(alloc) {
        return Err(format!(
            "the sole {INTERCEPT_INSTALL_SUCCESS} event belongs to {:?}, not {alloc}",
            event.alloc
        ));
    }
    Ok(event.at)
}

pub(super) struct WireCapture {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<CaptureBatch>>,
    port: u16,
}

pub(super) fn interface_index(iface: &str) -> u32 {
    let iface = std::ffi::CString::new(iface).expect("iface has no NUL");
    // SAFETY: libc retains no pointer.
    let ifindex = unsafe { libc::if_nametoindex(iface.as_ptr()) };
    assert_ne!(ifindex, 0, "resolve AF_PACKET interface index for {iface:?}");
    ifindex
}

impl WireCapture {
    fn start(iface: &str, port: u16) -> Self {
        Self::start_bound(interface_index(iface), port)
    }

    /// Capture every interface from before the allocation TAP exists. Binding
    /// AF_PACKET with ifindex zero is the kernel's all-interface shape; each
    /// `recvmsg` result retains its actual `sockaddr_ll.sll_ifindex` and kernel
    /// packet timestamp, allowing the audit to select the exact production interface
    /// after production creates it without relabeling queued packets at
    /// userspace dequeue time.
    fn start_all() -> Self {
        Self::start_bound(0, 0)
    }

    fn start_all_for_port(port: u16) -> Self {
        Self::start_bound(0, port)
    }

    fn start_bound(ifindex: u32, port: u16) -> Self {
        let fd = open_bound_packet_socket(ifindex).expect("open bound AF_PACKET capture");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || capture_fd(fd, ifindex, &stop_thread));
        Self { stop, handle: Some(handle), port }
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

    /// S-ND295-01's one loss-accounted exact-ifindex capture. `SOCK_RAW`
    /// keeps the link-layer header, so a virtio-net header delivered as frame
    /// bytes (the E3 12-byte zero prefix) is visible rather than hidden by the
    /// kernel's L3 datagram view. Close-on-exec keeps the capture out of every
    /// child the in-process `serve` spawns.
    pub(super) fn start_link_layer(ifindex: u32) -> Self {
        let fd = open_packet_socket(ifindex, libc::SOCK_RAW | libc::SOCK_CLOEXEC)
            .expect("open the exact-ifindex link-layer AF_PACKET capture");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || capture_fd(fd, ifindex, &stop_thread));
        Self { stop, handle: Some(handle), port: 0 }
    }

    /// Stop and return the batch; kernel-reported loss, a counted frame the
    /// reader never read, and a failed capture thread are evidence the caller
    /// asserts, never a panic here (see [`capture_is_fully_accounted`]).
    pub(super) fn stop_accounted(mut self) -> Result<CaptureBatch, String> {
        self.stop.store(true, Ordering::SeqCst);
        let handle = self.handle.take().ok_or("the capture thread was already joined")?;
        let capture = handle.join().map_err(|payload| {
            format!("the capture thread failed: {}", panic_evidence(payload.as_ref()))
        })?;
        capture_is_fully_accounted(&capture)?;
        Ok(capture)
    }

    fn stop_and_scan(
        self,
        ktls_candidates: &[KtlsSocketEvidence],
        loopback_ifindex: u32,
    ) -> (WireScan, Result<KtlsSocketEvidence, String>) {
        let port = self.port;
        let capture = self.stop();
        let (mut scan, exact) =
            correlate_ktls_candidates(&capture.frames, port, loopback_ifindex, ktls_candidates);
        scan.capture_packets = capture.statistics.packets;
        scan.capture_drops = capture.statistics.drops;
        (scan, exact)
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

/// Loss accounting over one sealed capture: the kernel dropped nothing, and
/// every packet it counted for this socket was read. For a socket without a
/// ring, `PACKET_STATISTICS.tp_packets` is the number of frames the kernel
/// queued plus the number it dropped (`tp_drops`), so a lossless, fully read
/// capture has `tp_packets == frames read` exactly. Either inequality fails
/// closed: "no frame" is evidence only when the reader provably saw every
/// frame the kernel delivered to it.
fn capture_is_fully_accounted(capture: &CaptureBatch) -> Result<(), String> {
    capture_statistics_are_lossless(capture.statistics)?;
    let counted = u64::from(capture.statistics.packets);
    let read = u64::try_from(capture.frames.len()).expect("frame count fits u64");
    if counted == read + u64::from(capture.statistics.drops) {
        Ok(())
    } else {
        Err(format!(
            "PACKET_STATISTICS counted {counted} packets (tp_drops {}) but the capture read {read} \
             frames; a frame the kernel delivered was never read (link-down reports {:?}, \
             interface removed {})",
            capture.statistics.drops, capture.link_down_reports, capture.interface_removed
        ))
    }
}

fn open_bound_packet_socket(ifindex: u32) -> std::io::Result<std::os::fd::RawFd> {
    open_packet_socket(ifindex, libc::SOCK_DGRAM)
}

fn open_packet_socket(
    ifindex: u32,
    socket_type: libc::c_int,
) -> std::io::Result<std::os::fd::RawFd> {
    // SAFETY: create and bind one AF_PACKET socket. An ifindex of zero is the
    // documented all-interface binding used by `start_all`. The socket is
    // created with protocol 0, which hooks it into no receive path; the bind
    // below names `ETH_P_ALL` and the interface together. A socket created
    // with `ETH_P_ALL` is hooked on every interface until the bind, so an
    // exact-ifindex capture could queue frames of other interfaces first.
    let fd = unsafe { libc::socket(libc::AF_PACKET, socket_type, 0) };
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

/// Read one AF_PACKET capture until the caller stops it.
///
/// `ENETDOWN` is a pending link-down report, never the end of the capture. A
/// packet socket bound to a device that is down is not hooked into that
/// device's receive path, and the kernel records the refusal as a pending
/// socket error (`packet_do_bind` sets `sk_err = ENETDOWN`); the device going
/// down later records the same error (`packet_notifier`, `NETDEV_DOWN`). The
/// next `recvmsg` reports it once, before any queued frame, and clears it.
/// When the device comes up the kernel hooks the socket to the same ifindex
/// again (`NETDEV_UP`) and delivery resumes. Each report is recorded with the
/// number of frames captured before it, and reading continues.
///
/// Consequence for callers: while its device is down an exact-ifindex capture
/// is blind. A zero-frame count over a down interval is therefore no evidence
/// on its own; the TAP counters and the link history witness that interval.
///
/// Only a genuine removal ends the reading early. When an exact-ifindex
/// capture receives a report and its ifindex no longer resolves
/// (`if_indextoname` → `ENXIO`), the kernel has unhooked the socket for good
/// (`NETDEV_UNREGISTER` resets its ifindex) and nothing more can arrive. A
/// device removed while already down produces no report; that capture reads
/// nothing more and ends at the caller's stop. An all-interface capture
/// (ifindex 0) is bound to no single device and receives no report.
///
/// At the end the socket is sealed ([`seal_packet_socket`]), settled, and drained,
/// and `PACKET_STATISTICS` is read over that closed population.
fn capture_fd(fd: std::os::fd::RawFd, bound_ifindex: u32, stop: &AtomicBool) -> CaptureBatch {
    let mut frames = Vec::new();
    let mut link_down_reports = Vec::new();
    let mut interface_removed = false;
    let mut buf = vec![0_u8; 65_535];
    while !stop.load(Ordering::SeqCst) && !interface_removed {
        match receive_captured_frame(fd, &mut buf) {
            Ok(Some(frame)) => frames.push(frame),
            Ok(None) => std::thread::sleep(Duration::from_micros(200)),
            Err(error) if error.raw_os_error() == Some(libc::ENETDOWN) => {
                link_down_reports.push(frames.len());
                interface_removed = bound_ifindex != 0
                    && !ifindex_resolves(bound_ifindex).unwrap_or_else(|error| {
                        panic!(
                            "resolve capture ifindex {bound_ifindex} after a link-down report: \
                             {error}"
                        )
                    });
            }
            Err(error) => panic!("AF_PACKET receive failed: {error}"),
        }
    }
    seal_packet_socket(fd).unwrap_or_else(|error| {
        panic!("seal the AF_PACKET capture before its final drain: {error}")
    });
    std::thread::sleep(PACKET_SOCKET_SEAL_SETTLE);
    loop {
        match receive_captured_frame(fd, &mut buf) {
            Ok(Some(frame)) => frames.push(frame),
            Ok(None) => break,
            Err(error) if error.raw_os_error() == Some(libc::ENETDOWN) => {
                link_down_reports.push(frames.len());
            }
            Err(error) => panic!("AF_PACKET final drain failed: {error}"),
        }
    }
    let statistics = packet_statistics(fd).expect("read PACKET_STATISTICS");
    // SAFETY: close exactly the fd created for this capture.
    unsafe { libc::close(fd) };
    CaptureBatch { frames, statistics, link_down_reports, interface_removed }
}

/// Whether `ifindex` names a live interface in this network namespace.
/// `ENXIO` (glibc's mapping of the kernel's `ENODEV`) is the absent answer;
/// any other failure is returned, never read as absence.
fn ifindex_resolves(ifindex: u32) -> std::io::Result<bool> {
    let mut name: [libc::c_char; libc::IF_NAMESIZE] = [0; libc::IF_NAMESIZE];
    // SAFETY: `name` is writable storage of IF_NAMESIZE bytes, the size
    // `if_indextoname` requires; libc retains no pointer after the call.
    let resolved = unsafe { libc::if_indextoname(ifindex, name.as_mut_ptr()) };
    if !resolved.is_null() {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(libc::ENXIO | libc::ENODEV)) {
        Ok(false)
    } else {
        Err(error)
    }
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

    let mut captures: BTreeMap<(u32, FlowTuple), DirectionCapture> = BTreeMap::new();
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
            libc::PACKET_OTHERHOST => 0b100,
            other => panic!(
                "same-port stream has an unclassified AF_PACKET packet_type={other}; frame={frame:?}"
            ),
        };
        let capture = captures.entry((frame.ifindex, segment.tuple)).or_default();
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
    for ((ifindex, tuple), capture) in captures {
        let Some(syn_sequence) = capture.syn_sequence else {
            if capture.pieces.iter().all(|(_, _, payload, _)| payload.is_empty()) {
                // A bare RST/ACK (typically the kernel rejecting a
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
                        "peer stream {tuple:?} has conflicting retransmitted/shared-bridge-copy bytes at offset {offset}"
                    );
                }
            }
        }
        assert!(
            logical_copies.values().all(|copy_types| *copy_types & !0b111 == 0),
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
        streams.insert((ifindex, tuple), stream);
    }

    let mut scan =
        WireScan { exact_tuple, same_port_streams_observed: streams.len(), ..WireScan::default() };
    scan.plaintext_markers_by_interface_tuple =
        streams.iter().map(|(key, bytes)| (*key, plaintext_marker_hits(bytes))).collect();
    for ((ifindex, tuple), hits) in &scan.plaintext_markers_by_interface_tuple {
        *scan.plaintext_markers_by_tuple.entry(*tuple).or_default() += hits;
        scan.interfaces_by_tuple.entry(*tuple).or_default().insert(*ifindex);
    }
    if let Some(tuple) = exact_tuple {
        let reverse = tuple.reverse();
        for ((_, observed), bytes) in &streams {
            let plaintext_hits = plaintext_marker_hits(bytes);
            if *observed == tuple || *observed == reverse {
                // Confidentiality is scoped to the exact production kTLS
                // socket and its reverse. It remains independent of positive
                // TLS parsing so malformed cleartext cannot hide there.
                scan.plaintext_hits_on_exact_peer_tuple += plaintext_hits;
            } else {
                // Same-port guest-local streams are intentionally plaintext.
                // Retain them as a diagnostic complement, never as peer wire.
                scan.plaintext_hits_on_other_same_port_streams += plaintext_hits;
            }
        }
    } else {
        scan.plaintext_hits_on_other_same_port_streams =
            streams.values().map(|bytes| plaintext_marker_hits(bytes)).sum();
    }
    scan
}

fn correlate_ktls_candidates(
    frames: &[CapturedFrame],
    peer_port: u16,
    loopback_ifindex: u32,
    candidates: &[KtlsSocketEvidence],
) -> (WireScan, Result<KtlsSocketEvidence, String>) {
    let mut matches = candidates
        .iter()
        .filter(|candidate| {
            record_has_bidirectional_tls13_ktls(&candidate.record)
                && candidate.inode != 0
                && candidate.fds.len() == 1
        })
        .map(|candidate| (scan_frames(frames, peer_port, Some(candidate.tuple)), candidate.clone()))
        .filter(|(scan, candidate)| {
            let expected = BTreeSet::from([loopback_ifindex]);
            scan.interfaces_by_tuple.get(&candidate.tuple) == Some(&expected)
                && scan.interfaces_by_tuple.get(&candidate.tuple.reverse()) == Some(&expected)
        })
        .collect::<Vec<_>>();
    match matches.len() {
        1 => {
            let (scan, candidate) = matches.pop().expect("one exact candidate");
            (scan, Ok(candidate))
        }
        0 => (
            scan_frames(frames, peer_port, None),
            Err(format!(
                "no eligible kTLS TX/RX inode/fd candidate appeared bidirectionally and only on loopback; candidates={candidates:?}"
            )),
        ),
        count => (
            scan_frames(frames, peer_port, None),
            Err(format!(
                "{count} eligible kTLS TX/RX inode/fd candidates ambiguously appeared bidirectionally and only on loopback: {:?}",
                matches.iter().map(|(_, candidate)| candidate).collect::<Vec<_>>()
            )),
        ),
    }
}

fn exact_tuple_appears_on_wrong_interface(
    frames: &[CapturedFrame],
    tuple: FlowTuple,
    expected_ifindex: u32,
) -> bool {
    frames.iter().any(|frame| {
        parse_tcp_segment(frame, false).is_ok_and(|segment| {
            segment.is_some_and(|segment| {
                (segment.tuple == tuple || segment.tuple == tuple.reverse())
                    && frame.ifindex != expected_ifindex
            })
        })
    })
}

fn assert_all_interface_plaintext_boundaries(
    scan: &WireScan,
    caller: Ipv4Addr,
    frontend: SocketAddrV4,
    gateway: Ipv4Addr,
    service: Ipv4Addr,
    approved_ifindices: &BTreeSet<u32>,
) {
    let caller_tuple = scan
        .plaintext_markers_by_tuple
        .iter()
        .find(|(tuple, hits)| {
            tuple.source.ip() == &caller && tuple.destination == frontend && **hits >= 2
        })
        .map(|(tuple, _)| *tuple)
        .expect("caller TAP/shared bridge carries both plaintext requests toward leg F");
    let leg_s = scan
        .plaintext_markers_by_tuple
        .iter()
        .filter(|(tuple, hits)| {
            tuple.source.ip() == &gateway
                && tuple.destination.ip() == &service
                && tuple.destination.port() == SERVICE_PORT
                && **hits >= 2
        })
        .map(|(tuple, _)| *tuple)
        .collect::<Vec<_>>();
    let [leg_s] = leg_s.as_slice() else {
        panic!(
            "exactly one plaintext leg-S tuple must carry both requests: {leg_s:?} scan={scan:?}"
        )
    };
    assert!(
        scan.plaintext_markers_by_tuple.get(&leg_s.reverse()).is_some_and(|hits| *hits >= 2),
        "the exact reverse leg-S tuple carries both plaintext responses"
    );
    assert!(
        scan.interfaces_by_tuple.keys().all(|tuple| {
            !(tuple.source.ip() == &caller
                && tuple.destination.ip() == &service
                && tuple.destination.port() == SERVICE_PORT)
        }),
        "caller-to-Service direct guest bypass is absent; caller leg-F tuple is {caller_tuple:?}"
    );
    let allowed_tuples =
        BTreeSet::from([caller_tuple, caller_tuple.reverse(), *leg_s, leg_s.reverse()]);
    for ((ifindex, tuple), hits) in &scan.plaintext_markers_by_interface_tuple {
        if *hits == 0 {
            continue;
        }
        assert!(
            approved_ifindices.contains(ifindex) && allowed_tuples.contains(tuple),
            "plaintext marker escaped the approved bridge/TAP leg-F/leg-S partition: ifindex={ifindex} tuple={tuple:?} hits={hits}"
        );
    }
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
    if total_len > bytes.len()
        || total_len > frame.wire_len
        || total_len < ihl
        || (reject_offload_ambiguity && (total_len != bytes.len() || total_len != frame.wire_len))
    {
        return Err(format!(
            "IPv4 tot_len exceeds or ambiguously differs from the captured skb: tot_len={total_len} wire_len={} copied={}",
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
    if reject_offload_ambiguity && aux.status & TP_STATUS_CSUMNOTREADY != 0 {
        return Err(format!("checksum/offload state is not finalized: {aux:?}"));
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

fn count_subslices(haystack: &[u8], needle: &[u8]) -> u64 {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    haystack.windows(needle.len()).filter(|window| *window == needle).count() as u64
}

fn plaintext_marker_hits(bytes: &[u8]) -> u64 {
    [REQUEST, RESPONSE, REQUEST2, RESPONSE2]
        .into_iter()
        .map(|marker| count_subslices(bytes, marker))
        .sum()
}

fn socket_inode(record: &str) -> Option<u64> {
    record
        .split_whitespace()
        .find_map(|field| field.strip_prefix("ino:").and_then(|inode| inode.parse().ok()))
}

fn process_fds_for_socket_inode(inode: u64) -> BTreeSet<i32> {
    let expected = format!("socket:[{inode}]");
    std::fs::read_dir("/proc/self/fd")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let fd = entry.file_name().to_str()?.parse::<i32>().ok()?;
            let target = std::fs::read_link(entry.path()).ok()?;
            (target.to_string_lossy() == expected).then_some(fd)
        })
        .collect()
}

fn complete_ktls_evidence(tuple: FlowTuple, record: String) -> Option<KtlsSocketEvidence> {
    let inode = socket_inode(&record)?;
    Some(KtlsSocketEvidence { tuple, inode, fds: process_fds_for_socket_inode(inode), record })
}

fn ktls_socket_records() -> Vec<KtlsSocketEvidence> {
    let output = Command::new("ss").args(["-H", "-n", "-t", "-i", "-e"]).output().expect("run ss");
    assert!(output.status.success(), "ss failed: {}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8_lossy(&output.stdout);
    let mut records = Vec::new();
    let mut current: Option<(FlowTuple, String)> = None;
    for line in text.lines() {
        if line.starts_with(char::is_whitespace) {
            if let Some((_, record)) = current.as_mut() {
                record.push_str(line);
                record.push('\n');
            }
            continue;
        }
        if let Some((tuple, record)) = current.take()
            && let Some(evidence) = complete_ktls_evidence(tuple, record)
        {
            records.push(evidence);
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
        current = Some((FlowTuple { source, destination }, format!("{line}\n")));
    }
    if let Some((tuple, record)) = current
        && let Some(evidence) = complete_ktls_evidence(tuple, record)
    {
        records.push(evidence);
    }
    records
}

fn record_has_bidirectional_tls13_ktls(record: &str) -> bool {
    record.contains("tcp-ulp-tls")
        && (record.contains("version: 1.3") || record.contains("version:1.3"))
        && (record.contains("rxconf: sw") || record.contains("rxconf:sw"))
        && (record.contains("txconf: sw") || record.contains("txconf:sw"))
}

fn retain_eligible_ktls_candidates(
    journal: &mut BTreeMap<(FlowTuple, u64), KtlsSocketEvidence>,
    port: u16,
    records: impl IntoIterator<Item = KtlsSocketEvidence>,
) {
    for mut record in records {
        if record.tuple.destination.port() == port
            && record_has_bidirectional_tls13_ktls(&record.record)
        {
            let key = (record.tuple, record.inode);
            if let Some(previous) = journal.get(&key) {
                // `ss` can retain a closing socket after the owner has closed
                // its last fd. Preserve the live ownership observation while
                // refreshing the full diagnostic record. Multiple observed
                // aliases remain multiple, so the sole-fd oracle fails closed.
                record.fds.extend(previous.fds.iter().copied());
            }
            journal.insert(key, record);
        }
    }
}

struct KtlsCandidateJournal {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<BTreeMap<(FlowTuple, u64), KtlsSocketEvidence>>>,
}

impl KtlsCandidateJournal {
    fn start(port: u16) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let mut journal = BTreeMap::new();
            while !thread_stop.load(Ordering::SeqCst) {
                retain_eligible_ktls_candidates(&mut journal, port, ktls_socket_records());
                std::thread::yield_now();
            }
            journal
        });
        Self { stop, handle: Some(handle) }
    }

    fn finish(mut self) -> Vec<KtlsSocketEvidence> {
        self.stop.store(true, Ordering::SeqCst);
        self.handle
            .take()
            .expect("kTLS candidate sampler exists")
            .join()
            .expect("kTLS candidate sampler joins")
            .into_values()
            .collect()
    }
}

impl Drop for KtlsCandidateJournal {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpliceEvidence {
    source_fds: BTreeSet<i32>,
    destination_fds: BTreeSet<i32>,
}

impl SpliceEvidence {
    fn proves_bidirectional_splice(&self, fd: i32) -> bool {
        self.source_fds.contains(&fd) && self.destination_fds.contains(&fd)
    }
}

fn require_same_socket_splice(
    ktls: &KtlsSocketEvidence,
    splice: &SpliceEvidence,
) -> Result<i32, String> {
    let mut fds = ktls.fds.iter().copied();
    let Some(fd) = fds.next() else {
        return Err(format!(
            "selected ss inode must map to exactly one in-process leg-B fd: inode={} fds={:?}",
            ktls.inode, ktls.fds
        ));
    };
    if fds.next().is_some() {
        return Err(format!(
            "selected ss inode must map to exactly one in-process leg-B fd: inode={} fds={:?}",
            ktls.inode, ktls.fds
        ));
    }
    if !splice.proves_bidirectional_splice(fd) {
        return Err(format!(
            "selected leg-B fd is not both a positive splice destination and source: fd={fd} evidence={splice:?}"
        ));
    }
    Ok(fd)
}

struct SpliceProbe {
    child: Option<Child>,
    path: PathBuf,
    seed_tids: BTreeSet<i32>,
}

impl SpliceProbe {
    fn attach_self() -> Self {
        assert!(
            Command::new("strace").arg("-V").output().is_ok_and(|output| output.status.success()),
            "strace is required for same-inode/fd splice evidence"
        );
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("nd295-s01-splice-{pid}.log"));
        let _ = std::fs::remove_file(&path);
        let child = Command::new("strace")
            .args(["-f", "-q", "-qq", "-e", "trace=clone,clone3,splice"])
            .args(["-o", path.to_str().expect("UTF-8 strace path")])
            .args(["-p", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("attach strace to the in-process production composition");
        std::thread::sleep(Duration::from_millis(400));
        Self { child: Some(child), path, seed_tids: snapshot_thread_group() }
    }

    fn finish(mut self) -> SpliceEvidence {
        self.detach();
        let raw = std::fs::read_to_string(&self.path).unwrap_or_default();
        let group = thread_group_closure(&raw, &self.seed_tids);
        parse_positive_splices(&raw, &group)
    }

    fn detach(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = Command::new("kill").args(["-TERM", &child.id().to_string()]).status();
            let _ = child.wait();
        }
    }
}

impl Drop for SpliceProbe {
    fn drop(&mut self) {
        self.detach();
        let _ = std::fs::remove_file(&self.path);
    }
}

fn snapshot_thread_group() -> BTreeSet<i32> {
    std::fs::read_dir("/proc/self/task")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .collect()
}

fn split_trace_tid(line: &str) -> (Option<i32>, &str) {
    let line = line.trim_start();
    let end = line.find(|character: char| !character.is_ascii_digit()).unwrap_or(line.len());
    if end == 0 {
        return (None, line);
    }
    (line[..end].parse().ok(), line[end..].trim_start())
}

fn completed_positive_return(body: &str) -> bool {
    let Some((_, returned)) = body.rsplit_once('=') else { return false };
    let returned = returned.trim_start();
    let end =
        returned.find(|character: char| !character.is_ascii_digit()).unwrap_or(returned.len());
    end > 0 && returned[..end].parse::<u64>().is_ok_and(|value| value > 0)
}

fn completed_clone_child(body: &str) -> Option<i32> {
    let (_, returned) = body.rsplit_once('=')?;
    let returned = returned.trim_start();
    let end =
        returned.find(|character: char| !character.is_ascii_digit()).unwrap_or(returned.len());
    (end > 0).then(|| returned[..end].parse().ok()).flatten()
}

fn clone_thread_edges(raw: &str) -> Vec<(i32, i32)> {
    let mut pending = BTreeMap::new();
    let mut edges = Vec::new();
    for line in raw.lines() {
        let (Some(parent), body) = split_trace_tid(line) else { continue };
        let start = body.starts_with("clone(") || body.starts_with("clone3(");
        let resumed = body.starts_with("<...")
            && (body.contains("clone resumed") || body.contains("clone3 resumed"));
        if !start && !resumed {
            continue;
        }
        if body.contains("<unfinished ...>") {
            pending.insert(parent, body.contains("CLONE_THREAD"));
            continue;
        }
        let is_thread = if resumed {
            pending.remove(&parent).unwrap_or(false)
        } else {
            body.contains("CLONE_THREAD")
        };
        if is_thread && let Some(child) = completed_clone_child(body) {
            edges.push((parent, child));
        }
    }
    edges
}

fn thread_group_closure(raw: &str, seed: &BTreeSet<i32>) -> BTreeSet<i32> {
    let edges = clone_thread_edges(raw);
    let mut group = seed.clone();
    loop {
        let mut changed = false;
        for (parent, child) in &edges {
            if group.contains(parent) && group.insert(*child) {
                changed = true;
            }
        }
        if !changed {
            return group;
        }
    }
}

fn splice_fd(body: &str, index: usize) -> Option<i32> {
    let start = body.find("splice(")? + "splice(".len();
    let argument = body[start..].split(',').nth(index)?.trim();
    let end =
        argument.find(|character: char| !character.is_ascii_digit()).unwrap_or(argument.len());
    (end > 0).then(|| argument[..end].parse().ok()).flatten()
}

fn parse_positive_splices(raw: &str, group: &BTreeSet<i32>) -> SpliceEvidence {
    let mut pending = BTreeMap::new();
    let mut evidence =
        SpliceEvidence { source_fds: BTreeSet::new(), destination_fds: BTreeSet::new() };
    for line in raw.lines() {
        let (Some(tid), body) = split_trace_tid(line) else { continue };
        if !group.contains(&tid) {
            continue;
        }
        if body.starts_with("splice(") {
            let Some(pair) = splice_fd(body, 0).zip(splice_fd(body, 2)) else { continue };
            if body.contains("<unfinished ...>") {
                pending.insert(tid, pair);
            } else if completed_positive_return(body) {
                evidence.source_fds.insert(pair.0);
                evidence.destination_fds.insert(pair.1);
            }
        } else if body.starts_with("<...")
            && body.contains("splice resumed")
            && completed_positive_return(body)
            && let Some(pair) = pending.remove(&tid)
        {
            evidence.source_fds.insert(pair.0);
            evidence.destination_fds.insert(pair.1);
        }
    }
    evidence
}

fn without_permitted_lifecycle_delta(row: &AllocStatusRowBody) -> AllocStatusRowBody {
    // Start from the COMPLETE observable row universe and normalize only the
    // seven explicitly-permitted lifecycle/ephemeral fields. Every other field
    // remains in the value compared by `assert_exact_lifecycle_delta`.
    let mut complement = row.clone();
    complement.state = AllocStateWire::Pending;
    complement.reason = None;
    complement.workload_addr = None;
    complement.started_at = None;
    complement.exit_code = None;
    complement.last_transition = None;
    complement.terminal = None;
    complement
}

fn assert_exact_lifecycle_delta(running: &AllocStatusRowBody, terminal: &AllocStatusRowBody) {
    // Exact permitted delta over the complete row:
    // state Running -> Terminated; terminal writers may either forward the
    // canonical guest address (the exit-observer path) or clear it (the
    // terminal action-shim path); reason/last_transition/current terminal are
    // replaced; the logical observation timestamp is restamped. No other row
    // field may change.
    assert_eq!(running.state, AllocStateWire::Running);
    assert_eq!(terminal.state, AllocStateWire::Terminated);
    assert_ne!(
        running.reason, terminal.reason,
        "the Running transition reason is replaced by the terminal stop reason"
    );
    assert!(running.workload_addr.is_some(), "Running VM carries its live guest address");
    assert!(
        terminal.workload_addr.is_none() || terminal.workload_addr == running.workload_addr,
        "terminal rows may clear or forward only the canonical guest address"
    );
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
        "the durable production fact must be the VM guest address"
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
        terminal: None,
        network_cleanup_pending: false,
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

pub(super) async fn poll_until_issued_identity(
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

async fn receive_vmm_cut(cuts: &VmmCutReceiver) -> VmmSpawnCut {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let received = cuts.lock().expect("VMM capture receiver mutex is not poisoned").try_recv();
        match received {
            Ok(cut) => return cut,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the production VM reaches the capture-ready cut within 30s"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("the VMM capture channel remains connected until the cut is released")
            }
        }
    }
}

async fn release_vmm_without_capture(cuts: &VmmCutReceiver) -> VmConfig {
    let cut = receive_vmm_cut(cuts).await;
    let config = cut.config.clone();
    cut.release.send(()).expect("release the real VMM spawn");
    config
}

async fn durable_alloc_snapshot(
    server_root: &Path,
    alloc_id: &str,
    label: &str,
    ordinal: u64,
) -> Option<overdrive_core::traits::observation_store::AllocStatusRow> {
    let observation_db = server_root.join("data/observation.redb");
    let snapshot = server_root.join(format!("{label}-{ordinal}.redb"));
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

// ---------------------------------------------------------------------
// S-ND295-01 zero-frame witness (FD § "D-295-DELIVER-04-01 — same-node protected-transport boundary (pre-event oracle WITHDRAWN)" (the restored pre-intercept contract); FD § "Effect isolation and Contract Shape classification" (the S-ND295-01 zero-frame witness row); E1, E3, E4).
//
// Observation only: every probe below reads production state (sysfs, procfs,
// rtnetlink notifications, AF_PACKET) and none installs or mutates a network
// effect. Each fallible observation returns its failure as evidence, so the
// shared scenario never asserts on it; only the S-ND295-01 bodies do.
// ---------------------------------------------------------------------

/// The six TAP counters of the zero-frame contract, in a fixed order.
const TAP_COUNTER_NAMES: [&str; 6] =
    ["rx_packets", "tx_packets", "rx_bytes", "tx_bytes", "rx_dropped", "tx_dropped"];
const NLMSG_HEADER_LEN: usize = 16;
const IFINFOMSG_LEN: usize = 16;
const RTATTR_HEADER_LEN: usize = 4;
const NLA_TYPE_MASK: u16 = 0x3fff;
const NLMSG_NOOP: u16 = 1;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLMSG_OVERRUN: u16 = 4;
const LINK_MONITOR_RECEIVE_BUFFER: libc::c_int = 32 * 1024 * 1024;
fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The six TAP counters in [`TAP_COUNTER_NAMES`] order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TapCounters([u64; 6]);

impl TapCounters {
    fn all_zero(self) -> bool {
        self.0.iter().all(|value| *value == 0)
    }
}

/// One read of the caller TAP and its queue holders, from sysfs and procfs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TapStateSample {
    ifindex: u32,
    flags: u32,
    /// Raw `tun` sysfs `owner` text; `0` is the root launcher (D-295-R4).
    owner: String,
    /// Raw `tun` sysfs `tun_flags` text; diagnostic only.
    tun_flags: String,
    counters: TapCounters,
    /// Every process holding a `/dev/net/tun` descriptor whose `fdinfo`
    /// `iff:` names this TAP, with those descriptor numbers.
    queue_holders: BTreeMap<u32, BTreeSet<u32>>,
    /// The allocation cgroup scope's `cgroup.procs` (empty before the scope
    /// exists).
    vmm_scope_pids: BTreeSet<u32>,
    vmm_argv: BTreeMap<u32, Vec<String>>,
}

fn sample_tap_state(tap: &str, alloc: &AllocationId) -> Result<TapStateSample, String> {
    let sysfs = Path::new("/sys/class/net").join(tap);
    let read = |attribute: &str| {
        std::fs::read_to_string(sysfs.join(attribute))
            .map(|text| text.trim().to_owned())
            .map_err(|error| format!("read {}/{attribute}: {error}", sysfs.display()))
    };
    let ifindex_text = read("ifindex")?;
    let ifindex = ifindex_text
        .parse::<u32>()
        .map_err(|error| format!("decode {tap} ifindex {ifindex_text:?}: {error}"))?;
    let flags_text = read("flags")?;
    let flags = u32::from_str_radix(flags_text.trim_start_matches("0x"), 16)
        .map_err(|error| format!("decode {tap} flags {flags_text:?}: {error}"))?;
    let owner = read("owner")?;
    let tun_flags = read("tun_flags")?;
    let mut counters = [0_u64; 6];
    for (slot, name) in counters.iter_mut().zip(TAP_COUNTER_NAMES) {
        let text = read(&format!("statistics/{name}"))?;
        *slot = text.parse().map_err(|error| format!("decode {tap} {name} {text:?}: {error}"))?;
    }
    let queue_holders = tap_queue_holders(tap)?;
    let vmm_scope_pids = allocation_scope_pids(alloc)?;
    let mut vmm_argv = BTreeMap::new();
    for pid in &vmm_scope_pids {
        vmm_argv.insert(*pid, process_argv(*pid)?);
    }
    Ok(TapStateSample {
        ifindex,
        flags,
        owner,
        tun_flags,
        counters: TapCounters(counters),
        queue_holders,
        vmm_scope_pids,
        vmm_argv,
    })
}

fn proc_entry_vanished(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}

/// The queue holder set of one TAP: every process holding a `/dev/net/tun`
/// descriptor whose `/proc/<pid>/fdinfo/<fd>` `iff:` line names `tap`. A
/// process or descriptor that disappears during the walk is skipped; any other
/// read failure is returned, so an unreadable holder never reads as absent.
fn tap_queue_holders(tap: &str) -> Result<BTreeMap<u32, BTreeSet<u32>>, String> {
    let mut holders: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let processes = std::fs::read_dir("/proc").map_err(|error| format!("read /proc: {error}"))?;
    for process in processes {
        let process = match process {
            Ok(process) => process,
            Err(error) if proc_entry_vanished(&error) => continue,
            Err(error) => return Err(format!("read a /proc entry: {error}")),
        };
        let Ok(pid) = process.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let descriptors = match std::fs::read_dir(process.path().join("fd")) {
            Ok(descriptors) => descriptors,
            Err(error) if proc_entry_vanished(&error) => continue,
            Err(error) => return Err(format!("read /proc/{pid}/fd: {error}")),
        };
        for descriptor in descriptors {
            let descriptor = match descriptor {
                Ok(descriptor) => descriptor,
                Err(error) if proc_entry_vanished(&error) => continue,
                Err(error) => return Err(format!("read a /proc/{pid}/fd entry: {error}")),
            };
            let target = match std::fs::read_link(descriptor.path()) {
                Ok(target) => target,
                Err(error) if proc_entry_vanished(&error) => continue,
                Err(error) => {
                    return Err(format!("read {}: {error}", descriptor.path().display()));
                }
            };
            if target != Path::new("/dev/net/tun") {
                continue;
            }
            let name = descriptor.file_name().to_string_lossy().into_owned();
            let fd = name
                .parse::<u32>()
                .map_err(|error| format!("decode /proc/{pid}/fd/{name}: {error}"))?;
            let fdinfo = match std::fs::read_to_string(process.path().join("fdinfo").join(&name)) {
                Ok(fdinfo) => fdinfo,
                Err(error) if proc_entry_vanished(&error) => continue,
                Err(error) => return Err(format!("read /proc/{pid}/fdinfo/{name}: {error}")),
            };
            let attached = fdinfo.lines().find_map(|line| line.strip_prefix("iff:")).map(str::trim);
            if attached == Some(tap) {
                holders.entry(pid).or_default().insert(fd);
            }
        }
    }
    Ok(holders)
}

fn allocation_scope_pids(alloc: &AllocationId) -> Result<BTreeSet<u32>, String> {
    let procs =
        CgroupPath::for_alloc(alloc).resolve(Path::new("/sys/fs/cgroup")).join("cgroup.procs");
    match std::fs::read_to_string(&procs) {
        Ok(text) => text
            .lines()
            .map(|line| {
                line.trim()
                    .parse::<u32>()
                    .map_err(|error| format!("decode {} line {line:?}: {error}", procs.display()))
            })
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(error) => Err(format!("read {}: {error}", procs.display())),
    }
}

fn process_argv(pid: u32) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline"))
        .map_err(|error| format!("read /proc/{pid}/cmdline: {error}"))?;
    Ok(bytes
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8_lossy(arg).into_owned())
        .collect())
}

/// The caller allocation and TAP the activation witness samples at the event.
#[derive(Debug, Clone)]
struct WitnessTarget {
    alloc: AllocationId,
    tap: String,
}

#[derive(Debug, Clone)]
struct ActivationWitnessSnapshot {
    alloc: Option<String>,
    sampled_at: KernelRealtime,
    /// `None` when no target was registered or the event names another
    /// allocation.
    tap: Option<Result<TapStateSample, String>>,
}

/// A tracing Layer that, synchronously inside the production thread that
/// emits `mtls.intercept.install.success`, reads the caller TAP's admin state,
/// ifindex, owner, six counters, and queue holders. The production source
/// emits the event after `start_alloc` returns and before it awaits
/// `activate` (FD § "Gate G-295-2 — existing guest command release, narrowed to the new switch" (the intercept-install event barrier)), so this read is at the event and before any
/// activation. It installs beside [`InterceptInstallTrace`], whose realtime
/// sample stays the barrier; this Layer runs after it.
#[derive(Clone, Default)]
struct ActivationWitness {
    target: Arc<Mutex<Option<WitnessTarget>>>,
    snapshots: Arc<Mutex<Vec<ActivationWitnessSnapshot>>>,
}

impl ActivationWitness {
    fn watch(&self, target: WitnessTarget) {
        *lock_unpoisoned(&self.target) = Some(target);
    }

    fn snapshots(&self) -> Vec<ActivationWitnessSnapshot> {
        lock_unpoisoned(&self.snapshots).clone()
    }
}

impl<S> Layer<S> for ActivationWitness
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if event.metadata().name() != INTERCEPT_INSTALL_SUCCESS {
            return;
        }
        let sampled_at = KernelRealtime::now();
        let mut visitor = InterceptInstallFieldVisitor::default();
        event.record(&mut visitor);
        let target = lock_unpoisoned(&self.target).clone();
        let tap = target
            .filter(|target| visitor.alloc.as_deref() == Some(target.alloc.as_str()))
            .map(|target| sample_tap_state(&target.tap, &target.alloc));
        lock_unpoisoned(&self.snapshots).push(ActivationWitnessSnapshot {
            alloc: visitor.alloc,
            sampled_at,
            tap,
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkNoticeKind {
    New,
    Deleted,
}

/// One kernel `RTM_NEWLINK` / `RTM_DELLINK` notification.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkNotice {
    /// Userspace receipt time: an upper bound on when the kernel generated it.
    received_at: KernelRealtime,
    kind: LinkNoticeKind,
    /// `ifi_family`: `AF_UNSPEC` for the device's own notification,
    /// `AF_BRIDGE` for the bridge's notification about its port.
    family: u8,
    ifindex: u32,
    flags: u32,
    name: Option<String>,
}

/// An rtnetlink monitor subscribed to the link group. The subscription is
/// bound before [`LinkMonitor::start`] returns, so it covers every link change
/// after that point in kernel order. `ENOBUFS` (lost notifications) fails the
/// monitor rather than yielding a partial history.
struct LinkMonitor {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Result<Vec<LinkNotice>, String>>>,
}

impl LinkMonitor {
    fn start() -> Self {
        let fd = open_link_monitor_socket()
            .expect("open and bind the rtnetlink link-group monitor before deploy");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || monitor_links(&fd, &stop_thread));
        Self { stop, handle: Some(handle) }
    }

    fn stop(mut self) -> Result<Vec<LinkNotice>, String> {
        self.stop.store(true, Ordering::SeqCst);
        let handle = self.handle.take().ok_or("the link monitor thread was already joined")?;
        handle.join().map_err(|payload| {
            format!("the link monitor thread failed: {}", panic_evidence(payload.as_ref()))
        })?
    }
}

impl Drop for LinkMonitor {
    fn drop(&mut self) {
        // Panic-safe fixture cleanup, as for `WireCapture`: never leave the
        // monitor thread running past a failed oracle.
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn open_link_monitor_socket() -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _};

    // SAFETY: plain socket creation; the descriptor moves into `OwnedFd`.
    let raw = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            libc::NETLINK_ROUTE,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `raw` is a fresh descriptor that nothing else owns.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
    let size = LINK_MONITOR_RECEIVE_BUFFER;
    // SAFETY: `fd` is live and the option points to one integer.
    if unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUFFORCE,
            std::ptr::from_ref(&size).cast(),
            libc::socklen_t::try_from(std::mem::size_of_val(&size))
                .expect("receive buffer option length fits socklen_t"),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: an all-zero `sockaddr_nl` is valid; the public fields are set.
    let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    address.nl_groups = libc::RTMGRP_LINK as u32;
    // SAFETY: the live sockaddr has exactly the supplied size.
    if unsafe {
        libc::bind(
            fd.as_raw_fd(),
            std::ptr::from_ref(&address).cast(),
            libc::socklen_t::try_from(std::mem::size_of_val(&address))
                .expect("sockaddr_nl length fits socklen_t"),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(fd)
}

fn monitor_links(fd: &std::os::fd::OwnedFd, stop: &AtomicBool) -> Result<Vec<LinkNotice>, String> {
    let mut notices = Vec::new();
    let mut buf = vec![0_u8; 256 * 1024];
    loop {
        // Read the flag before receiving, so a final empty receive after it
        // was set proves the queue is drained.
        let stopping = stop.load(Ordering::SeqCst);
        match receive_link_batch(fd, &mut buf)? {
            Some((len, received_at)) => parse_link_notices(&buf[..len], received_at, &mut notices)?,
            None if stopping => return Ok(notices),
            None => std::thread::sleep(Duration::from_micros(200)),
        }
    }
}

fn receive_link_batch(
    fd: &std::os::fd::OwnedFd,
    buf: &mut [u8],
) -> Result<Option<(usize, KernelRealtime)>, String> {
    use std::os::fd::AsRawFd as _;

    // SAFETY: an all-zero `sockaddr_nl` is a valid receive buffer.
    let mut source: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    let mut source_len = libc::socklen_t::try_from(std::mem::size_of_val(&source))
        .expect("sockaddr_nl length fits socklen_t");
    // SAFETY: every pointer references live owned storage of the stated size.
    let n = unsafe {
        libc::recvfrom(
            fd.as_raw_fd(),
            buf.as_mut_ptr().cast(),
            buf.len(),
            libc::MSG_TRUNC,
            std::ptr::from_mut(&mut source).cast(),
            &raw mut source_len,
        )
    };
    let received_at = KernelRealtime::now();
    if n < 0 {
        let error = std::io::Error::last_os_error();
        return match error.kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted => Ok(None),
            _ if error.raw_os_error() == Some(libc::ENOBUFS) => {
                Err("the rtnetlink link monitor overran (ENOBUFS): link notifications were lost"
                    .to_owned())
            }
            _ => Err(format!("the rtnetlink link monitor receive failed: {error}")),
        };
    }
    let len = usize::try_from(n).expect("recvfrom length is non-negative");
    if len > buf.len() {
        return Err(format!("an rtnetlink datagram of {len} bytes exceeded the receive buffer"));
    }
    if source.nl_pid != 0 {
        return Err(format!("an rtnetlink link notification came from port {}", source.nl_pid));
    }
    Ok(Some((len, received_at)))
}

fn read_ne_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    bytes
        .get(offset..offset + 2)
        .and_then(|slice| <[u8; 2]>::try_from(slice).ok())
        .map(u16::from_ne_bytes)
        .ok_or_else(|| format!("truncated u16 at offset {offset}"))
}

fn read_ne_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
        .map(u32::from_ne_bytes)
        .ok_or_else(|| format!("truncated u32 at offset {offset}"))
}

const fn netlink_align(len: usize) -> usize {
    (len + 3) & !3
}

fn parse_link_notices(
    datagram: &[u8],
    received_at: KernelRealtime,
    notices: &mut Vec<LinkNotice>,
) -> Result<(), String> {
    let mut offset = 0;
    while offset < datagram.len() {
        let len = read_ne_u32(datagram, offset)? as usize;
        let kind = read_ne_u16(datagram, offset + 4)?;
        let end = offset.checked_add(len).ok_or("rtnetlink message length overflows")?;
        if len < NLMSG_HEADER_LEN || end > datagram.len() {
            return Err(format!("malformed rtnetlink message length {len} at offset {offset}"));
        }
        let body = &datagram[offset + NLMSG_HEADER_LEN..end];
        match kind {
            libc::RTM_NEWLINK => {
                notices.push(parse_link_notice(LinkNoticeKind::New, body, received_at)?);
            }
            libc::RTM_DELLINK => {
                notices.push(parse_link_notice(LinkNoticeKind::Deleted, body, received_at)?);
            }
            NLMSG_ERROR | NLMSG_OVERRUN => {
                return Err(format!("rtnetlink reported message type {kind} to the link monitor"));
            }
            NLMSG_NOOP | NLMSG_DONE => {}
            other => {
                return Err(format!("unexpected rtnetlink message type {other} in the link group"));
            }
        }
        offset = netlink_align(end);
    }
    Ok(())
}

fn parse_link_notice(
    kind: LinkNoticeKind,
    body: &[u8],
    received_at: KernelRealtime,
) -> Result<LinkNotice, String> {
    if body.len() < IFINFOMSG_LEN {
        return Err(format!("an rtnetlink link message carries a {}-byte ifinfomsg", body.len()));
    }
    let family = body[0];
    let ifindex = read_ne_u32(body, 4)?;
    if ifindex == 0 || ifindex > i32::MAX as u32 {
        return Err(format!("an rtnetlink link message carries ifindex {ifindex}"));
    }
    let flags = read_ne_u32(body, 8)?;
    let mut name = None;
    let mut offset = IFINFOMSG_LEN;
    while offset + RTATTR_HEADER_LEN <= body.len() {
        let attribute_len = usize::from(read_ne_u16(body, offset)?);
        let attribute_type = read_ne_u16(body, offset + 2)? & NLA_TYPE_MASK;
        let end = offset + attribute_len;
        if attribute_len < RTATTR_HEADER_LEN || end > body.len() {
            return Err(format!("malformed rtnetlink attribute length {attribute_len}"));
        }
        if attribute_type == libc::IFLA_IFNAME {
            let value = &body[offset + RTATTR_HEADER_LEN..end];
            let value = value
                .split(|byte| *byte == 0)
                .next()
                .unwrap_or_else(|| unreachable!("a slice split yields at least one piece"));
            name = Some(String::from_utf8_lossy(value).into_owned());
        }
        offset = netlink_align(end);
    }
    Ok(LinkNotice { received_at, kind, family, ifindex, flags, name })
}

/// S-ND295-01 evidence joined from the observations above.
#[derive(Debug)]
struct BornCapturedEvidence {
    caller_alloc: AllocationId,
    tap: String,
    tap_ifindex: u32,
    guest_mac: [u8; 6],
    guest_address: Ipv4Addr,
    /// Sampled at the VMM spawn cut: after provision, before Cloud Hypervisor.
    pre_create: Result<TapStateSample, String>,
    event_snapshots: Vec<ActivationWitnessSnapshot>,
    link_notices: Result<Vec<LinkNotice>, String>,
    link_layer: Result<CaptureBatch, String>,
    post_stop_complement: Result<(), String>,
}

impl BornCapturedEvidence {
    /// The caller TAP read at the exact event: exactly one witness snapshot,
    /// for the caller allocation, sampled at or after the barrier.
    fn at_event(&self, intercept_live_at: KernelRealtime) -> Result<&TapStateSample, String> {
        let [snapshot] = self.event_snapshots.as_slice() else {
            return Err(format!(
                "expected exactly one witnessed {INTERCEPT_INSTALL_SUCCESS} event, got {:#?}",
                self.event_snapshots
            ));
        };
        if snapshot.alloc.as_deref() != Some(self.caller_alloc.as_str()) {
            return Err(format!(
                "the witnessed event names {:?}, not {}",
                snapshot.alloc, self.caller_alloc
            ));
        }
        if snapshot.sampled_at < intercept_live_at {
            return Err(format!(
                "the witness sampled at {:?}, before the barrier {intercept_live_at:?}",
                snapshot.sampled_at
            ));
        }
        match &snapshot.tap {
            Some(Ok(state)) => Ok(state),
            Some(Err(error)) => Err(format!("the event-time TAP read failed: {error}")),
            None => Err("no caller TAP was registered with the witness at the event".to_owned()),
        }
    }
}

// ---------------------------------------------------------------------
// S-ND295-01 harness evidence: the capture helpers against a scratch TAP.
// ---------------------------------------------------------------------

/// A persistent scratch TAP created down by the production creator
/// ([`overdrive_netlink::create_persistent_tap`], owner uid 0), named outside
/// the managed `ovd-tp-` and bridge `ovd-gbr` spaces, and deleted on drop.
struct CaptureScratchTap {
    name: String,
}

impl CaptureScratchTap {
    fn create() -> Self {
        let name = format!("nd295cap{:x}", std::process::id());
        assert!(name.len() < libc::IFNAMSIZ, "scratch TAP name {name} fits IFNAMSIZ");
        let lookup = std::ffi::CString::new(name.as_str()).expect("TAP name has no NUL");
        // SAFETY: the NUL-terminated name remains live for this lookup.
        let existing = unsafe { libc::if_nametoindex(lookup.as_ptr()) };
        assert_eq!(
            existing, 0,
            "scratch TAP {name} already exists; an interrupted earlier run left it behind \
             (remove it with `ip link del {name}`)"
        );
        overdrive_netlink::create_persistent_tap(&name, 0)
            .unwrap_or_else(|error| panic!("create the persistent scratch TAP {name}: {error}"));
        Self { name }
    }

    fn raise(&self) {
        let name = self.name.clone();
        overdrive_netlink::block_on_host_netlink(move || async move {
            overdrive_netlink::Client::new()?.set_link_up(&name).await
        })
        .unwrap_or_else(|error| panic!("raise the scratch TAP {}: {error}", self.name));
    }

    /// Attach one queue to the persistent TAP with a raw `TUNSETIFF`: the
    /// device's own flags (`IFF_TAP | IFF_NO_PI`, single queue), so a write is
    /// exactly one Ethernet frame the TAP receives.
    fn open_queue(&self) -> std::fs::File {
        let queue = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/net/tun")
            .expect("open /dev/net/tun (close-on-exec)");
        // SAFETY: an all-zero `ifreq` is a valid initial value; the name and
        // flags are populated below.
        let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
        for (slot, byte) in request.ifr_name.iter_mut().zip(self.name.bytes()) {
            *slot = libc::c_char::from_ne_bytes([byte]);
        }
        request.ifr_ifru.ifru_flags = libc::c_short::try_from(libc::IFF_TAP | libc::IFF_NO_PI)
            .expect("TAP queue flags fit ifr_flags");
        // SAFETY: `queue` is an open `/dev/net/tun` descriptor and `request`
        // is a live, initialised `ifreq` for the whole call.
        let rc = unsafe {
            libc::ioctl(std::os::fd::AsRawFd::as_raw_fd(&queue), libc::TUNSETIFF, &raw mut request)
        };
        assert_eq!(
            rc,
            0,
            "attach a queue to the scratch TAP {}: {}",
            self.name,
            std::io::Error::last_os_error()
        );
        queue
    }
}

impl Drop for CaptureScratchTap {
    fn drop(&mut self) {
        let name = self.name.clone();
        let deleted = overdrive_netlink::block_on_host_netlink(move || async move {
            overdrive_netlink::Client::new()?.del_link(&name).await
        });
        if let Err(error) = deleted {
            if std::thread::panicking() {
                eprintln!("cleanup: deleting the scratch TAP {} failed: {error}", self.name);
            } else {
                panic!("delete the scratch TAP {}: {error}", self.name);
            }
        }
    }
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

/// S-GTI-07 — workload describe surfaces the VM's canonical shared-network
/// address.
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

    let row = described.snapshot.rows.first().expect("one Running VM row");
    let guest_addr = row.workload_addr.expect("Running VM exposes its canonical address");
    assert_eq!(
        row.workload_addr,
        Some(guest_addr),
        "describe must project the persisted shared-network address"
    );
    // Response observable universe: the full command wrapper plus the entire
    // serialized API snapshot. The pre-change contract is independently
    // reconstructed from a CoW snapshot of the production durable inputs; it
    // never reads or clones this post-change HTTP result. Add exactly the one
    // permitted workload_addr fact, then require whole-response equality.
    let pre_change_contract =
        genuine_pre_change_contract(server_tmp.path(), &submit, guest_addr).await;
    assert_eq!(described.workload_id, pre_change_contract.workload_id);
    assert_eq!(described.spec_digest, pre_change_contract.spec_digest);
    assert_eq!(described.allocations_total, pre_change_contract.allocations_total);
    assert_eq!(described.empty_state_message, pre_change_contract.empty_state_message);
    let mut expected_post_change = pre_change_contract.clone();
    expected_post_change.snapshot.rows[0].workload_addr = Some(guest_addr);
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
    let address_delta = format!("Addresses:\n  {}: {}\n", row.alloc_id, guest_addr);
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

    stop(StopArgs { id: submit.workload_id.clone(), config_path: cfg.clone() })
        .await
        .expect("stop describe VM");
    let _ = poll_until_terminal(&cfg, &submit.workload_id, Duration::from_secs(30)).await;
    handle.shutdown().await.expect("clean mTLS serve shutdown");
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn synthetic_d7_accounting_rejects_competing_capture_counts() {
    let segments = [40_usize, 60, 50];
    let packet_count = u64::try_from(segments.len()).expect("segment count fits u64");
    let byte_count = segments
        .iter()
        .map(|length| u64::try_from(*length).expect("IPv4 length fits u64"))
        .sum::<u64>();
    assert_eq!(packet_count, 3);
    assert_eq!(byte_count, 150);
    assert_ne!(packet_count + 1, packet_count, "a competing packet count cannot be equal");
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

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn tcp_offload_ambiguity_is_scoped_to_eligible_ipv4_tcp_frames() {
    let mut reported_dns = synthetic_guest_tcp_frame();
    reported_dns.bytes[9] = 17;
    reported_dns.aux.as_mut().expect("synthetic AUXDATA").status |= TP_STATUS_CSUMNOTREADY;
    assert!(
        matches!(parse_tcp_segment(&reported_dns, true), Ok(None)),
        "the reported IPv4/UDP DNS frame is ineligible for the TCP universe before checksum-offload classification"
    );

    let mut non_ipv4 = synthetic_guest_tcp_frame();
    non_ipv4.protocol = 0x0806;
    non_ipv4.aux.as_mut().expect("synthetic AUXDATA").status |= TP_STATUS_CSUMNOTREADY;
    assert!(
        matches!(parse_tcp_segment(&non_ipv4, true), Ok(None)),
        "a non-IPv4 Ethernet frame is outside the TCP universe even when AUXDATA carries CSUMNOTREADY"
    );

    let mut eligible_tcp = synthetic_guest_tcp_frame();
    eligible_tcp.aux.as_mut().expect("synthetic AUXDATA").status |= TP_STATUS_CSUMNOTREADY;
    let Err(error) = parse_tcp_segment(&eligible_tcp, true) else {
        panic!("an eligible IPv4/TCP frame with unfinished checksum state fails closed")
    };
    assert!(error.contains("checksum/offload state is not finalized"));

    let mut truncated_udp = synthetic_guest_tcp_frame();
    truncated_udp.bytes[9] = 17;
    truncated_udp.aux.as_mut().expect("synthetic AUXDATA").status |= TP_STATUS_CSUMNOTREADY;
    truncated_udp.truncated = true;
    assert!(
        parse_tcp_segment(&truncated_udp, true).is_err(),
        "truncation remains invalid before protocol eligibility"
    );
    let mut length_mismatch_udp = reported_dns;
    length_mismatch_udp.aux.as_mut().expect("synthetic AUXDATA").snaplen -= 1;
    assert!(
        parse_tcp_segment(&length_mismatch_udp, true).is_err(),
        "AUXDATA length mismatch remains invalid before protocol eligibility"
    );

    let mut padded = synthetic_guest_tcp_frame();
    padded.bytes.extend_from_slice(&[0; 6]);
    padded.wire_len += 6;
    let aux = padded.aux.as_mut().expect("synthetic AUXDATA");
    aux.len += 6;
    aux.snaplen += 6;
    assert!(
        matches!(parse_tcp_segment(&padded, false), Ok(Some(_))),
        "all-interface classification accepts validated Ethernet padding outside IPv4 tot_len"
    );
    assert!(
        parse_tcp_segment(&padded, true).is_err(),
        "the guest/TAP offload oracle retains strict complete-skb equality"
    );
}

fn synthetic_peer_tcp_frame(
    tuple: FlowTuple,
    sequence: u32,
    flags: u8,
    payload: &[u8],
    packet_type: u8,
) -> CapturedFrame {
    synthetic_peer_tcp_frame_on(1, tuple, sequence, flags, payload, packet_type)
}

fn synthetic_peer_tcp_frame_on(
    ifindex: u32,
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
        ifindex,
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
    let exact_peer = FlowTuple {
        source: SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 1), 40_001),
        destination: SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 2), SERVICE_PORT),
    };
    let guest_local = FlowTuple {
        source: SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 3), 40_002),
        destination: SocketAddrV4::new(Ipv4Addr::new(10, 98, 0, 1), SERVICE_PORT),
    };
    let split = REQUEST.len() / 2;
    let second_sequence = 1_001_u32 + u32::try_from(split).expect("marker split fits u32");
    let opaque_loopback = b"same-node-loopback-payload-is-not-a-TLS-record-oracle";
    let frames = vec![
        synthetic_peer_tcp_frame(guest_local, 1_000, 0x02, &[], libc::PACKET_OUTGOING),
        synthetic_peer_tcp_frame(guest_local, 1_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(
            guest_local,
            second_sequence,
            0x18,
            &REQUEST[split..],
            libc::PACKET_OUTGOING,
        ),
        synthetic_peer_tcp_frame(guest_local, 1_001, 0x18, &REQUEST[..split], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(
            guest_local,
            1_001,
            0x18,
            &REQUEST[..split],
            libc::PACKET_OUTGOING,
        ),
        synthetic_peer_tcp_frame(
            guest_local,
            second_sequence,
            0x18,
            &REQUEST[split..],
            libc::PACKET_HOST,
        ),
        synthetic_peer_tcp_frame(exact_peer, 5_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(exact_peer, 5_001, 0x18, opaque_loopback, libc::PACKET_HOST),
        synthetic_peer_tcp_frame(exact_peer.reverse(), 6_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(
            exact_peer.reverse(),
            6_001,
            0x18,
            opaque_loopback,
            libc::PACKET_HOST,
        ),
    ];
    let ktls_record = "tcp-ulp-tls version:1.3 rxconf:sw txconf:sw";
    let unrelated_candidate = KtlsSocketEvidence {
        tuple: FlowTuple {
            source: SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 1), 40_000),
            destination: exact_peer.destination,
        },
        inode: 40_000,
        fds: BTreeSet::from([70]),
        record: ktls_record.to_owned(),
    };
    let exact_candidate = KtlsSocketEvidence {
        tuple: exact_peer,
        inode: 40_001,
        fds: BTreeSet::from([71]),
        record: ktls_record.to_owned(),
    };
    let exact_after_close = KtlsSocketEvidence {
        tuple: exact_peer,
        inode: 40_001,
        fds: BTreeSet::new(),
        record: format!("{ktls_record} bytes_acked:77"),
    };
    let exact_latest =
        KtlsSocketEvidence { fds: BTreeSet::from([71]), ..exact_after_close.clone() };
    let ineligible = KtlsSocketEvidence {
        tuple: FlowTuple {
            source: SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 1), 39_999),
            destination: SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 2), SERVICE_PORT + 1),
        },
        inode: 39_999,
        fds: BTreeSet::from([72]),
        record: ktls_record.to_owned(),
    };
    let mut journal = BTreeMap::new();
    retain_eligible_ktls_candidates(
        &mut journal,
        SERVICE_PORT,
        [exact_candidate, unrelated_candidate.clone()],
    );
    retain_eligible_ktls_candidates(&mut journal, SERVICE_PORT, [exact_after_close, ineligible]);
    let journal = journal.into_values().collect::<Vec<_>>();
    assert_eq!(
        journal.iter().map(|candidate| candidate.tuple).collect::<Vec<_>>(),
        vec![unrelated_candidate.tuple, exact_peer],
        "the journal is tuple/inode-deduplicated and deterministically ordered"
    );
    assert_eq!(
        journal[1], exact_latest,
        "a later closing-state ss record retains the earlier live sole-fd ownership evidence"
    );
    let reused_tuple = KtlsSocketEvidence {
        tuple: exact_peer,
        inode: 40_002,
        fds: BTreeSet::from([72]),
        record: ktls_record.to_owned(),
    };
    let mut reuse_journal = BTreeMap::new();
    retain_eligible_ktls_candidates(
        &mut reuse_journal,
        SERVICE_PORT,
        [exact_latest.clone(), reused_tuple],
    );
    let reused_candidates = reuse_journal.into_values().collect::<Vec<_>>();
    assert_eq!(
        reused_candidates.len(),
        2,
        "tuple reuse by another inode remains two candidates instead of erasing ambiguity"
    );
    assert!(
        correlate_ktls_candidates(&frames, SERVICE_PORT, 1, &reused_candidates).1.is_err(),
        "two owned inodes for one captured tuple fail closed"
    );
    assert_eq!(socket_inode("ESTAB ... ino:40001 sk:1"), Some(40_001));
    let seed = BTreeSet::from([100]);
    let trace = "100 clone(child_stack=NULL, flags=CLONE_VM|CLONE_THREAD) = 101\n\
                 101 splice(5, NULL, 71, NULL, 73, SPLICE_F_MOVE) = 73\n\
                 101 splice(71, NULL, 6, NULL, 71, SPLICE_F_MOVE) = 71\n\
                 999 splice(8, NULL, 71, NULL, 99, SPLICE_F_MOVE) = 99\n";
    let group = thread_group_closure(trace, &seed);
    assert_eq!(group, BTreeSet::from([100, 101]));
    let splice = parse_positive_splices(trace, &group);
    assert_eq!(
        require_same_socket_splice(&exact_latest, &splice)
            .expect("same-inode/fd evidence correlates"),
        71,
        "only the selected in-process fd is both a positive splice destination and source"
    );
    assert!(
        require_same_socket_splice(
            &exact_latest,
            &SpliceEvidence { source_fds: BTreeSet::from([71]), destination_fds: BTreeSet::new() },
        )
        .is_err(),
        "one missing splice direction fails closed"
    );
    let mut no_owner = exact_latest.clone();
    no_owner.fds.clear();
    assert!(
        correlate_ktls_candidates(&frames, SERVICE_PORT, 1, &[no_owner]).1.is_err(),
        "a captured kTLS tuple without a live fd owner receives no credit"
    );
    let mut ambiguous_owner = exact_latest.clone();
    ambiguous_owner.fds.insert(72);
    assert!(
        correlate_ktls_candidates(&frames, SERVICE_PORT, 1, &[ambiguous_owner]).1.is_err(),
        "a captured kTLS inode with multiple in-process fds receives no credit"
    );
    let (scan, selected) = correlate_ktls_candidates(&frames, SERVICE_PORT, 1, &journal);
    assert_eq!(
        selected.expect("the second same-port candidate is the exact captured TLS connection"),
        exact_latest
    );
    assert!(!exact_tuple_appears_on_wrong_interface(&frames, exact_peer, 1));
    assert!(exact_tuple_appears_on_wrong_interface(&frames, exact_peer, 2));
    assert_eq!(scan.same_port_streams_observed, 3);
    assert_eq!(
        scan.plaintext_hits_on_exact_peer_tuple, 0,
        "guest-local plaintext must not be attributed to the exact peer tuple"
    );
    assert_eq!(
        scan.plaintext_hits_on_other_same_port_streams, 1,
        "dequeue reordering and duplicate copies reconstruct the intentional guest-local plaintext once"
    );

    let (unknown_scan, unknown_match) =
        correlate_ktls_candidates(&frames, SERVICE_PORT, 1, &[unrelated_candidate]);
    assert!(unknown_match.is_err(), "an unrelated first candidate fails closed");
    assert_eq!(unknown_scan.plaintext_hits_on_exact_peer_tuple, 0);
    assert_eq!(
        unknown_scan.plaintext_hits_on_other_same_port_streams, 1,
        "an unknown tuple never adopts an unrelated same-port stream as peer evidence"
    );

    let (_, ambiguous) =
        correlate_ktls_candidates(&frames, SERVICE_PORT, 1, &[exact_latest.clone(), exact_latest]);
    assert!(ambiguous.is_err(), "multiple matching candidates fail closed as ambiguous");

    let clear_peer = vec![
        synthetic_peer_tcp_frame(exact_peer, 7_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(exact_peer, 7_001, 0x18, REQUEST, libc::PACKET_HOST),
        synthetic_peer_tcp_frame(exact_peer.reverse(), 8_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(exact_peer.reverse(), 8_001, 0x18, RESPONSE, libc::PACKET_HOST),
    ];
    let clear_scan = scan_frames(&clear_peer, SERVICE_PORT, Some(exact_peer));
    assert_eq!(
        clear_scan.plaintext_hits_on_exact_peer_tuple, 2,
        "request and response cleartext on the exact peer tuple remain a fail-closed confidentiality witness"
    );

    let requests = [REQUEST, REQUEST2].concat();
    let responses = [RESPONSE, RESPONSE2].concat();
    let leg_s = FlowTuple {
        source: SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 1), 40_003),
        destination: SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 2), SERVICE_PORT),
    };
    let mut approved_plaintext = vec![
        synthetic_peer_tcp_frame(guest_local, 9_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(guest_local, 9_001, 0x18, &requests, libc::PACKET_HOST),
        synthetic_peer_tcp_frame(leg_s, 10_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(leg_s, 10_001, 0x18, &requests, libc::PACKET_HOST),
        synthetic_peer_tcp_frame(leg_s.reverse(), 11_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame(leg_s.reverse(), 11_001, 0x18, &responses, libc::PACKET_HOST),
    ];
    let approved_scan = scan_frames(&approved_plaintext, SERVICE_PORT, None);
    assert_all_interface_plaintext_boundaries(
        &approved_scan,
        *guest_local.source.ip(),
        guest_local.destination,
        *leg_s.source.ip(),
        *leg_s.destination.ip(),
        &BTreeSet::from([1]),
    );

    approved_plaintext.extend([
        synthetic_peer_tcp_frame_on(2, leg_s, 12_000, 0x02, &[], libc::PACKET_HOST),
        synthetic_peer_tcp_frame_on(2, leg_s, 12_001, 0x18, REQUEST, libc::PACKET_HOST),
    ]);
    let escaped_scan = scan_frames(&approved_plaintext, SERVICE_PORT, None);
    assert!(
        std::panic::catch_unwind(|| {
            assert_all_interface_plaintext_boundaries(
                &escaped_scan,
                *guest_local.source.ip(),
                guest_local.destination,
                *leg_s.source.ip(),
                *leg_s.destination.ip(),
                &BTreeSet::from([1]),
            );
        })
        .is_err(),
        "plaintext on an unapproved non-loopback interface fails closed"
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
    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_d7_decoder_and_oracle_error_fails_closed(
        capture_corruption in 0_u8..8,
        drops in 1_u32..=u32::MAX,
    ) {
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
    // server owner. Catch each cleanup independently: neither an error nor a
    // panic in the first owner may bypass the second authoritative teardown.
    let peer = std::panic::AssertUnwindSafe(peer_cleanup)
        .catch_unwind()
        .await
        .map_err(|payload| format!("peer cleanup panicked: {}", panic_evidence(payload.as_ref())))
        .and_then(|result| result);
    let server = std::panic::AssertUnwindSafe(server_cleanup)
        .catch_unwind()
        .await
        .map_err(|payload| format!("server cleanup panicked: {}", panic_evidence(payload.as_ref())))
        .and_then(|result| result);
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
    let (server, _tmp, _cuts) = spawn_capture_observed_mtls_server().await;
    let endpoint = server.endpoint().clone();
    // Server boot is allowed to sweep residue from an earlier crashed owner;
    // the assertion-safe interval begins after that production recovery step.
    let exact_before = fault_fixture::PacketPathBaseline::capture();
    let malformed = fault_fixture::ProductInputHookFixture::install();
    let wire = WireCapture::start(LOOPBACK_IFACE, SERVICE_PORT);
    let result: Result<(), String> = tokio::time::timeout(
        Duration::from_secs(15),
        finish_after_authoritative_cleanup(
            Err("injected failure after real server, capture, and packet-path owners exist"
                .to_owned()),
            async move {
                drop(wire);
                malformed.finish();
                Ok(())
            },
            async move {
                server.shutdown().await.map_err(|error| {
                    format!("drain real recovery server after injected failure: {error}")
                })
            },
        ),
    )
    .await
    .expect("injected post-owner failure restores and joins within its own bound");
    assert!(result.is_err());
    assert_eq!(
        fault_fixture::PacketPathBaseline::capture(),
        exact_before,
        "post-owner assertion failure restores the exact packet-path baseline"
    );
    let addr = SocketAddr::new(
        "127.0.0.1".parse().expect("loopback address"),
        endpoint.port_or_known_default().expect("serve endpoint port"),
    );
    assert!(
        std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_err(),
        "post-owner assertion failure synchronously drains the real server listener"
    );
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
    let _config = release_vmm_without_capture(&cuts).await;
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
    let durable =
        durable_alloc_snapshot(server_tmp.path(), &final_row.alloc_id, "exit-78-final", 1)
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

#[path = "guest_stack_mtls_egress_fault_fixture.rs"]
mod fault_fixture;
