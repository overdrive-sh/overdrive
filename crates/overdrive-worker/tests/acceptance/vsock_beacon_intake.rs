//! D17 / DG-4 / QPROJ-4 — the node beacon intake and the VM driver's beacon
//! observation (GH #295, vsock attachment replacement).
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md` DG-4
//! (`BeaconIntake::bind`, `BeaconIntakeError`, the core `BeaconIo` port) and
//! QPROJ-4 (`VmDriver::beacon_observation`, `BeaconObservation`). Scenario
//! spec: `docs/feature/netns-density-295/distill/test-scenarios-p2-owner-flows.md`
//! P2-S14 (the intake half that needs no replaced signature).
//!
//! Driving ports: `BeaconIntake::bind` and `VmDriver::beacon_observation`.
//! Driven port: `BeaconIo`, supplied by `SimBeaconIo` (the QPROJ-7 arbiter of
//! the beacon accept queue), coupled to a `SimVmm`.
//!
//! Not here: the P2-S14 attribution cells (READY of the live launch takes the
//! claim; a stale queued READY; a read after `stop`; a second concurrent
//! connection; a non-READY first line). They drive a `VmDriver` built with
//! `VmDriver::new(.., beacon)` (DG-2 row 15) and READY lines carrying
//! `launch=` (`BeaconMessage::Ready { launch }`, DG-2 row 13), so they are
//! authored at the attachment cut.
//!
//! Fixture: [`fresh_driver`] builds the driver through the attachment cut's
//! `VmDriver::new(.., BeaconIntake)` (DG-2 row 15), binding the intake over a
//! `SimBeaconIo` coupled to the same `SimVmm`.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test fixtures assert their preconditions; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use overdrive_core::id::NodeId;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::vm::beacon::{BEACON_VSOCK_PORT, BeaconIo};
use overdrive_core::vm::config::{Gid, HostArch, VmConfinement, VmmIdentity};
use overdrive_sim::adapters::beacon_io::SimBeaconIo;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::vm_driver::VmHostLayout;
use overdrive_worker::{BeaconIntake, BeaconIntakeError, BeaconObservation, VmDriver};

fn sim_beacon(vmm: &SimVmm) -> Arc<SimBeaconIo> {
    Arc::new(SimBeaconIo::new(vmm.clone()))
}

/// A `VmDriver` over sim ports. Construction performs no I/O beyond binding
/// the sim beacon listener; the layout's paths are never touched because
/// nothing is started.
async fn fresh_driver(vmm: SimVmm) -> VmDriver {
    let beacon = BeaconIntake::bind(sim_beacon(&vmm) as Arc<dyn BeaconIo>)
        .await
        .expect("the sim beacon listener binds");
    let clock = SimClock::new();
    let wiring =
        overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(clock.clone()));
    let probe_runner = Arc::new(ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        Arc::new(SimHttpProber::new()),
        Arc::new(SimClock::new()),
        Arc::new(SimObservationStore::single_peer(
            NodeId::new("vsock-beacon-intake").expect("valid node ID"),
            0,
        )) as Arc<dyn ObservationStore>,
    ));
    let root = PathBuf::from("/nonexistent/vsock-beacon-intake");
    VmDriver::new(
        Arc::new(vmm),
        Arc::new(clock),
        Arc::new(SimCgroupFs::new()),
        Arc::new(SimCgroupAccounting::new()),
        probe_runner,
        wiring.gate(),
        VmHostLayout {
            cgroup_root: root.join("cgroup"),
            run_dir_root: root.join("run"),
            clone_index_dir: root.join("clone-index"),
            clone_staging_dir: root.join("clone-staging"),
            arch: HostArch::X86_64,
            confinement: VmConfinement::confined(
                VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] },
                1024,
            ),
        },
        beacon,
    )
}

/// DG-4: binding the intake binds the listener through the port, once.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 6, beacon relocation, D17)"]
async fn binding_the_intake_binds_the_beacon_listener() {
    let vmm = SimVmm::new();
    let io = sim_beacon(&vmm);
    assert!(!io.is_bound(), "precondition: nothing bound");

    let Ok(_intake) = BeaconIntake::bind(Arc::clone(&io) as Arc<dyn BeaconIo>).await else {
        panic!("a healthy port binds");
    };
    assert!(io.is_bound(), "the intake bound the listener");
    assert!(io.beacon_socket_observation().queued.is_empty(), "binding accepts nothing on its own");
}

/// DG-4: a bind failure is `BeaconIntakeError::Bind` naming
/// `BEACON_VSOCK_PORT` and the port's error; nothing is bound.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 6, beacon relocation, D17)"]
async fn a_bind_failure_is_typed_and_binds_nothing() {
    let vmm = SimVmm::new();
    let io = sim_beacon(&vmm);
    io.fail_next_bind(std::io::ErrorKind::AddrInUse);

    let Err(err) = BeaconIntake::bind(Arc::clone(&io) as Arc<dyn BeaconIo>).await else {
        panic!("the scripted bind failure must surface");
    };
    let BeaconIntakeError::Bind { port, source } = &err;
    assert_eq!(*port, BEACON_VSOCK_PORT);
    assert_eq!(source.kind(), std::io::ErrorKind::AddrInUse);
    assert!(!io.is_bound(), "nothing bound");
    assert!(
        err.to_string().starts_with("binding the beacon vsock listener on port 1234 failed: "),
        "{err}"
    );
}

/// The typed refusal renders its pinned message (the `health.startup.refused`
/// reason `vm_driver.beacon_bind` carries it).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_bind_refusal_renders_its_pinned_message() {
    let err = BeaconIntakeError::Bind {
        port: BEACON_VSOCK_PORT,
        source: std::io::Error::new(std::io::ErrorKind::AddrInUse, "in use"),
    };
    assert_eq!(err.to_string(), "binding the beacon vsock listener on port 1234 failed: in use");
}

/// QPROJ-4: a driver that has started nothing reports three empty
/// collections, and a repeated call returns an equal value.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 6, beacon relocation, D17)"]
async fn a_fresh_driver_reports_an_empty_beacon_observation() {
    let driver = fresh_driver(SimVmm::new()).await;
    let first = driver.beacon_observation();
    assert_eq!(
        first,
        BeaconObservation {
            live: BTreeMap::new(),
            claims: BTreeMap::new(),
            unread: BTreeSet::new()
        }
    );
    assert_eq!(driver.beacon_observation(), first, "a repeated call changes nothing");
}
