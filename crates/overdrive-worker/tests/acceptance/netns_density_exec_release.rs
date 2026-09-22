//! Deterministic real-`VmDriver` EXEC-release schedules for GH #295.

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "reasoned-pending acceptance bodies use exact diagnostics and real beacon fixtures"
)]

use std::os::fd::AsRawFd as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::SpiffeId;
use overdrive_core::guest_network::{
    GuestNetworkExecSupervisor, GuestNetworkExecWiring, SharedGuestNetworkComponent,
    SharedGuestNetworkFailStopCause,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, Driver, DriverPayload, Resources, VmPayload,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::vm::beacon::{BEACON_VSOCK_PORT, BeaconMessage};
use overdrive_core::vm::config::{
    Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfinement, VmRunDir, VmmIdentity,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_worker::VmDriver;
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::vm_driver::VmHostLayout;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

fn probe_runner() -> Arc<ProbeRunner> {
    Arc::new(ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        Arc::new(SimHttpProber::new()),
        Arc::new(SimClock::new()),
        Arc::new(SimObservationStore::single_peer(
            NodeId::new("nd295-exec-release").expect("node id"),
            0,
        )) as Arc<dyn ObservationStore>,
    ))
}

fn layout(tmp: &TempDir) -> VmHostLayout {
    let kernel = tmp.path().join("vmlinuz");
    let mut header = vec![0_u8; KERNEL_MAGIC_WINDOW];
    header[..4].copy_from_slice(b"\x7fELF");
    std::fs::write(&kernel, header).expect("stage kernel");
    std::fs::write(tmp.path().join("rootfs.img"), b"nd295-rootfs").expect("stage rootfs");
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

fn spec(tmp: &TempDir, alloc: &AllocationId, argument_bytes: usize) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/workload/nd295/alloc/exec")
            .expect("SPIFFE ID"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: vec!["x".repeat(argument_bytes)],
            kernel: tmp.path().join("vmlinuz"),
            rootfs: tmp.path().join("rootfs.img"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        network: Some(overdrive_core::traits::driver::GuestNetworkAssignment {
            address: "100.95.0.2".parse().expect("guest address"),
            tap: "ovd-tp-0002".to_owned(),
            mac: [0x02, 0, 100, 95, 0, 2],
            gateway: "100.95.0.1".parse().expect("gateway"),
            prefix: 16,
            dns: "100.95.0.1".parse().expect("DNS"),
        }),
        service_ports: Vec::new(),
    }
}

fn driver(tmp: &TempDir, wiring: &GuestNetworkExecWiring) -> (VmDriver, PathBuf, SimVmm) {
    let layout = layout(tmp);
    let run_root = layout.run_dir_root.clone();
    let vmm = SimVmm::new();
    let driver = VmDriver::new(
        Arc::new(vmm.clone()),
        Arc::new(SimClock::new()),
        Arc::new(SimCgroupFs::new()),
        Arc::new(SimCgroupAccounting::new()),
        probe_runner(),
        wiring.gate(),
        layout,
    );
    (driver, run_root, vmm)
}

async fn connect(path: &Path) -> UnixStream {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match UnixStream::connect(path).await {
            Ok(stream) => return stream,
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            Err(error) => panic!("beacon did not bind at {}: {error}", path.display()),
        }
    }
}

async fn start(
    driver: &VmDriver,
    spec: &AllocationSpec,
    run_root: &Path,
) -> (AllocationHandle, UnixStream) {
    let socket = VmRunDir::for_alloc(run_root, &spec.alloc).beacon_socket(BEACON_VSOCK_PORT);
    let owned_driver = driver.clone();
    let owned_spec = spec.clone();
    let start = tokio::spawn(async move { owned_driver.start(&owned_spec).await });
    let mut guest = connect(&socket).await;
    guest.write_all(b"READY pid=1 port=1234\n").await.expect("write READY");
    let handle = start.await.expect("start joins").expect("READY admits VM");
    (handle, guest)
}

async fn yields() {
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
}

async fn assert_no_line(guest: &mut BufReader<UnixStream>) {
    let mut line = String::new();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), guest.read_line(&mut line)).await.is_err(),
        "guest must not receive EXEC while release is closed; observed {line:?}"
    );
}

/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn recovering_waiter_keeps_pending_exec_untaken_until_recovered_event_precedes_ack() {
    let tmp = TempDir::new().expect("tempdir");
    let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
    let supervisor = wiring.supervisor();
    assert!(supervisor.open_after_boot());
    let (driver, run_root, _vmm) = driver(&tmp, &wiring);
    let alloc = AllocationId::new("nd295-recovery-before-release").expect("allocation id");
    let assignment = spec(&tmp, &alloc, 0);
    let (handle, guest) = start(&driver, &assignment, &run_root).await;
    assert!(supervisor.begin_recovery(SharedGuestNetworkComponent::TcxLink));

    let release_driver = driver.clone();
    let release_handle = handle.clone();
    let release = tokio::spawn(async move {
        release_driver.release_for_exit_emission(&release_handle).await;
    });
    let mut guest = BufReader::new(guest);
    yields().await;
    assert!(!release.is_finished(), "Recovering waiter remains owned by the release future");
    assert_no_line(&mut guest).await;

    assert!(supervisor.complete_attempt(None), "recovered event reopens release");
    let mut exec = String::new();
    tokio::time::timeout(Duration::from_secs(1), guest.read_line(&mut exec))
        .await
        .expect("recovered release is bounded")
        .expect("read EXEC");
    assert!(exec.starts_with("EXEC "), "actual beacon acknowledgement follows recovery: {exec}");
    release.await.expect("release future remains owned");
}

/// A claim linearized before detection may transmit bytes or one complete EXEC
/// before cancellation wins. The observable universe is the structured release
/// task, the production beacon stream through EOF, and newline-framed messages
/// accepted by the existing [`BeaconMessage`] parser. Cancellation must end the
/// task-owned claim lifetime, close the transferred writer, and preserve the
/// complement of no second complete EXEC command.
///
/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn claim_before_detection_backpressure_and_cancellation_do_not_create_a_second_writer() {
    let tmp = TempDir::new().expect("tempdir");
    let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
    let supervisor = wiring.supervisor();
    assert!(supervisor.open_after_boot());
    let (driver, run_root, _vmm) = driver(&tmp, &wiring);
    let alloc = AllocationId::new("nd295-claim-before-detection").expect("allocation id");
    let assignment = spec(&tmp, &alloc, 16 * 1024 * 1024);
    let (handle, mut guest) = start(&driver, &assignment, &run_root).await;
    let receive_bytes: libc::c_int = 4 * 1024;
    // SAFETY: `guest` owns a live Unix socket and the option points to one integer.
    let configured = unsafe {
        libc::setsockopt(
            guest.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            std::ptr::from_ref(&receive_bytes).cast(),
            libc::socklen_t::try_from(std::mem::size_of_val(&receive_bytes)).expect("socklen"),
        )
    };
    assert_eq!(configured, 0);

    let release_driver = driver.clone();
    let release_handle = handle.clone();
    let release = tokio::spawn(async move {
        release_driver.release_for_exit_emission(&release_handle).await;
    });
    yields().await;
    assert!(!release.is_finished(), "writer acknowledgement is backpressured");
    assert!(supervisor.begin_recovery(SharedGuestNetworkComponent::BridgeGuard));
    assert!(!release.is_finished(), "the pre-detection claim keeps the one writer owned");

    release.abort();
    assert!(release.await.expect_err("release is cancelled").is_cancelled());
    let mut observed = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(1),
        tokio::io::AsyncReadExt::read_to_end(&mut guest, &mut observed),
    )
    .await
    .expect("writer cancellation closes the beacon")
    .expect("read through EOF");
    // The Published Language defines a complete command at its existing
    // newline-framing plus typed-parser boundary. A pre-detection claim may
    // have already transmitted any prefix (or the whole first command) before
    // cancellation; a raw `EXEC ` byte prefix is therefore not a message and
    // is not evidence of a second writer.
    let complete_messages: Vec<BeaconMessage> = observed
        .split_inclusive(|byte| *byte == b'\n')
        .filter(|frame| frame.ends_with(b"\n"))
        .map(|frame| {
            let line = std::str::from_utf8(frame).unwrap_or_else(|error| {
                panic!(
                    "every newline-complete beacon frame must be valid UTF-8: {error}; \
                     frame={frame:?}"
                )
            });
            line.parse::<BeaconMessage>().unwrap_or_else(|error| {
                panic!(
                    "every newline-complete beacon frame must parse through the Published \
                     Language: {error}; frame={line:?}"
                )
            })
        })
        .collect();
    assert!(
        matches!(complete_messages.as_slice(), [] | [BeaconMessage::Exec { .. }]),
        "cancellation stream must contain zero or one newline-complete typed EXEC and no other \
         complete frame; observed {} complete frame(s)",
        complete_messages.len()
    );
}

/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn fail_stop_wakes_the_real_vm_driver_waiter_without_writing_exec() {
    let tmp = TempDir::new().expect("tempdir");
    let wiring = GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
    let supervisor: Arc<GuestNetworkExecSupervisor> = wiring.supervisor();
    assert!(supervisor.open_after_boot());
    let (driver, run_root, _vmm) = driver(&tmp, &wiring);
    let alloc = AllocationId::new("nd295-fail-stop-refusal").expect("allocation id");
    let assignment = spec(&tmp, &alloc, 0);
    let (handle, guest) = start(&driver, &assignment, &run_root).await;
    assert!(supervisor.begin_recovery(SharedGuestNetworkComponent::EndpointMap));

    let release_driver = driver.clone();
    let release_handle = handle.clone();
    let release = tokio::spawn(async move {
        release_driver.release_for_exit_emission(&release_handle).await;
    });
    yields().await;
    assert!(!release.is_finished());
    assert!(
        supervisor.fail_stop(SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded).is_some()
    );
    release.await.expect("FailStop wakes the real VmDriver waiter");

    let mut guest = BufReader::new(guest);
    assert_no_line(&mut guest).await;
    assert_eq!(driver.live_allocations(), Some(vec![alloc]));
}
