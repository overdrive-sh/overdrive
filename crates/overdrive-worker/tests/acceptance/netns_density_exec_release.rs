//! Deterministic real-`VmDriver` EXEC-release schedules for GH #295.

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "reasoned-pending acceptance bodies use exact diagnostics and real beacon fixtures"
)]

use std::collections::BTreeSet;
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
use overdrive_core::vm::beacon::BeaconMessage;
use overdrive_core::vm::config::{Gid, HostArch, KERNEL_MAGIC_WINDOW, VmConfinement, VmmIdentity};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_worker::VmDriver;
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::vm_driver::VmHostLayout;
use tempfile::TempDir;

use super::support::{BeaconHarness, GuestSession};

fn probe_runner() -> Arc<ProbeRunner> {
    Arc::new(ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        Arc::new(SimHttpProber::new()),
        Arc::new(SimClock::new()),
        Arc::new(SimObservationStore::single_peer(
            NodeId::new("nd295-exec-release").expect("node id"),
            0,
        )) as Arc<dyn ObservationStore>,
        Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
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
        transport: Some(super::support::transport(
            "100.95.0.2".parse().expect("guest address"),
            "100.95.0.1".parse().expect("gateway"),
            0,
        )),
        service_ports: Vec::new(),
    }
}

async fn driver(
    tmp: &TempDir,
    wiring: &GuestNetworkExecWiring,
) -> (VmDriver, BeaconHarness, SimVmm) {
    let layout = layout(tmp);
    let vmm = SimVmm::new();
    let beacon = BeaconHarness::new(&vmm);
    let driver = VmDriver::new(
        beacon.vmm(Arc::new(vmm.clone())),
        Arc::new(SimClock::new()),
        Arc::new(SimCgroupFs::new()),
        Arc::new(SimCgroupAccounting::new()),
        probe_runner(),
        wiring.gate(),
        layout,
        beacon.intake().await,
    );
    (driver, beacon, vmm)
}

/// Spawn `start`, the guest connects with READY once its VMM exists, and
/// `start` admits the VM.
async fn start(
    driver: &VmDriver,
    spec: &AllocationSpec,
    beacon: &BeaconHarness,
) -> (AllocationHandle, GuestSession) {
    let owned_driver = driver.clone();
    let owned_spec = spec.clone();
    let start = tokio::spawn(async move { owned_driver.start(&owned_spec).await });
    let guest = beacon.dial_ready(spec).await;
    let handle = start.await.expect("start joins").expect("READY admits VM");
    (handle, guest)
}

async fn yields() {
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
}

async fn assert_no_line(guest: &mut GuestSession) {
    let observed = tokio::time::timeout(Duration::from_millis(50), guest.next_line()).await;
    assert!(
        observed.is_err(),
        "guest must not receive EXEC while release is closed; observed {observed:?}"
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
    let (driver, beacon, _vmm) = driver(&tmp, &wiring).await;
    let alloc = AllocationId::new("nd295-recovery-before-release").expect("allocation id");
    let assignment = spec(&tmp, &alloc, 0);
    let (handle, mut guest) = start(&driver, &assignment, &beacon).await;
    assert!(supervisor.begin_recovery(SharedGuestNetworkComponent::Forwarder));

    let release_driver = driver.clone();
    let release_handle = handle.clone();
    let release = tokio::spawn(async move {
        release_driver.release_for_exit_emission(&release_handle).await;
    });
    yields().await;
    assert!(!release.is_finished(), "Recovering waiter remains owned by the release future");
    assert_no_line(&mut guest).await;

    assert!(supervisor.complete_attempt(BTreeSet::new()), "recovered event reopens release");
    let exec = tokio::time::timeout(Duration::from_secs(1), guest.next_line())
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
    let (driver, beacon, _vmm) = driver(&tmp, &wiring).await;
    let alloc = AllocationId::new("nd295-claim-before-detection").expect("allocation id");
    let assignment = spec(&tmp, &alloc, 16 * 1024 * 1024);
    let (handle, guest) = start(&driver, &assignment, &beacon).await;
    // The guest's receive side stops draining: the 16 MiB EXEC write cannot
    // complete (the former shrunk-`SO_RCVBUF` backpressure).
    guest.backpressure();

    let release_driver = driver.clone();
    let release_handle = handle.clone();
    let release = tokio::spawn(async move {
        release_driver.release_for_exit_emission(&release_handle).await;
    });
    yields().await;
    assert!(!release.is_finished(), "writer acknowledgement is backpressured");
    assert!(supervisor.begin_recovery(SharedGuestNetworkComponent::IpSets));
    assert!(!release.is_finished(), "the pre-detection claim keeps the one writer owned");

    release.abort();
    assert!(release.await.expect_err("release is cancelled").is_cancelled());
    let observed = tokio::time::timeout(Duration::from_secs(1), guest.session_to_end())
        .await
        .expect("writer cancellation closes the beacon");
    // The Published Language defines a complete command at its existing
    // newline-framing plus typed-parser boundary. A pre-detection claim may
    // have already transmitted the whole first command before cancellation;
    // every line the host delivered must parse as a typed message.
    let complete_messages: Vec<BeaconMessage> = observed
        .iter()
        .map(|line| {
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
    let (driver, beacon, _vmm) = driver(&tmp, &wiring).await;
    let alloc = AllocationId::new("nd295-fail-stop-refusal").expect("allocation id");
    let assignment = spec(&tmp, &alloc, 0);
    let (handle, mut guest) = start(&driver, &assignment, &beacon).await;
    assert!(supervisor.begin_recovery(SharedGuestNetworkComponent::FlowListeners));

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

    assert_no_line(&mut guest).await;
    assert_eq!(driver.live_allocations(), Some(vec![alloc]));
}
