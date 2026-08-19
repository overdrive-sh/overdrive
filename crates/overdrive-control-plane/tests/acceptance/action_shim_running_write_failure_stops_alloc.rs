//! greptile PR #268 P1 — a `Running`-observation write failure must not
//! strand the started workload.
//!
//! The action shim's `StartAllocation` / `RestartAllocation` arms spawn
//! the workload via `driver.start()` (which arms an exit watcher parked
//! on the Running-confirmed gate), THEN write the `Running`
//! `AllocStatusRow`, THEN fire the gate via
//! `Driver::release_for_exit_emission`. If the `Running` write FAILS the
//! `?`-propagation used to return before the gate fire — leaving:
//!
//! 1. the exit watcher parked forever on the never-fired gate (no
//!    `ExitEvent`, so the exit observer never runs and never cleans up),
//!    and
//! 2. the workload process + host footprint alive with no terminal row —
//!    which `VmReclamation` will NOT reclaim, because the alloc is still
//!    in `live_allocations()` and `reclamation_authorised` is `false` by
//!    design (reclamation must never race a live driver).
//!
//! The fix tears the just-started workload down (`driver.stop`) before
//! propagating the write error: `stop` drops the stashed gate sender (the
//! `Driver::start` § "Sender drop (orphan path)") so the watcher unparks,
//! and reclaims the host footprint — turning an orphaned-live workload
//! into a clean failed start the reconciler re-dispatches next tick.
//!
//! # PORT-TO-PORT litmus
//!
//! Drives the production driving port `action_shim::dispatch` and asserts
//! at the driven-port boundary. TWO driver shapes are exercised, because
//! write-failure recovery has two observable halves:
//!
//! * **Exec shape** (`DriverType::Exec`) — asserts `SimDriver::live_count()`
//!   is 0 after the failure: the shim STOPPED the started workload
//!   (`SimDriver::stop` evicts the slot). The teardown half.
//! * **VM shape** (`DriverType::Vm`) — asserts `Driver::live_allocations()`
//!   no longer reports the alloc after the failure: the shim RELEASED the
//!   supervision claim `stop` left `EndingInFlight`. The release half —
//!   and it is VmDriver-SPECIFIC. `VmDriver::stop` moves its claim
//!   `Live -> EndingInFlight` (never a full remove — its double-authorship
//!   guard), so `stop` ALONE does not retire it; without the explicit
//!   `release_supervision` pairing the torn-down alloc is reported by
//!   `live_allocations()` forever, pinning `VmReclamation`
//!   (`!reclamation_authorised`) for a dead id (greptile PR #268 P1
//!   "Ending claim survives teardown"). An Exec-shaped `SimDriver` CANNOT
//!   exhibit this leak — its `stop` is a full remove — which is exactly
//!   why an Exec-only `live_count` assertion left the leak uncaught. The
//!   VM-shaped `SimDriver` faithfully models the phased claim so the
//!   pairing is actually observable here.
//!
//! Per-driver teardown internals stay covered by the driver-level suites
//! (`vm_driver_stop_totality.rs`, `driver.rs` gate tests).
//!
//! Default lane: sim adapters for every port, `mtls_worker: None`, no
//! root, no netns.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::action_shim::{AllocDriverIndex, ShimError, dispatch};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::veth_provisioner::NetSlotAllocator;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::eval_broker::EvaluationBroker;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverRegistry, DriverType, ExecPayload, Resources,
    VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, ObservationStore, ObservationStoreError,
};
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

fn alloc_id() -> AllocationId {
    AllocationId::new("alloc-writefail-0").expect("valid alloc id")
}

fn start_action() -> Action {
    Action::StartAllocation {
        alloc_id: alloc_id(),
        workload_id: WorkloadId::new("writefail").expect("valid workload id"),
        node_id: NodeId::new("node-001").expect("valid node id"),
        spec: AllocationSpec {
            alloc: alloc_id(),
            identity: SpiffeId::new("spiffe://overdrive.local/workload/writefail/alloc/0")
                .expect("valid spiffe id"),
            driver: DriverPayload::Exec(ExecPayload {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
            }),
            resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            probe_descriptors: Vec::new(),
            netns: None,
            host_veth: None,
            service_ports: Vec::new(),
            workload_addr: None,
        },
        kind: WorkloadKind::Service,
    }
}

/// The VM-shaped counterpart of [`start_action`]. Routes to a
/// `DriverType::Vm` driver (`spec.driver.driver_type() == Vm`), so the
/// shim selects the registered VM-typed `SimDriver` — the one that models
/// `VmDriver`'s phased `Live -> EndingInFlight` claim. `kernel` / `rootfs`
/// are placeholders: `SimDriver::start` boots nothing and ignores them.
fn start_action_vm() -> Action {
    Action::StartAllocation {
        alloc_id: alloc_id(),
        workload_id: WorkloadId::new("writefail").expect("valid workload id"),
        node_id: NodeId::new("node-001").expect("valid node id"),
        spec: AllocationSpec {
            alloc: alloc_id(),
            identity: SpiffeId::new("spiffe://overdrive.local/workload/writefail/alloc/0")
                .expect("valid spiffe id"),
            driver: DriverPayload::Vm(VmPayload {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
                kernel: PathBuf::from("/nonexistent/kernel"),
                rootfs: PathBuf::from("/nonexistent/rootfs"),
            }),
            resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            probe_descriptors: Vec::new(),
            netns: None,
            host_veth: None,
            service_ports: Vec::new(),
            workload_addr: None,
        },
        kind: WorkloadKind::Service,
    }
}

/// Dispatch a single `Action` against the supplied (already wired)
/// `SimDriver` + `SimObservationStore`, so the caller retains both for
/// post-dispatch assertions. Every other port is a fresh sim adapter;
/// `mtls_worker` / `workflow_engine` are `None`. The caller supplies the
/// action so the same harness drives both the Exec-shaped
/// ([`start_action`]) and VM-shaped ([`start_action_vm`]) starts.
async fn dispatch_action(
    obs: &Arc<SimObservationStore>,
    sim_driver: &Arc<SimDriver>,
    action: Action,
) -> Result<(), ShimError> {
    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
    let obs_dyn: Arc<dyn ObservationStore> = Arc::clone(obs) as Arc<dyn ObservationStore>;

    let drivers: Arc<DriverRegistry> = {
        let mut r = DriverRegistry::new();
        r.insert(Arc::clone(sim_driver) as Arc<dyn overdrive_core::traits::driver::Driver>);
        Arc::new(r)
    };
    let alloc_drivers = AllocDriverIndex::default();
    let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let writer_node = NodeId::new("writer-1").expect("NodeId");
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));
    let net_slot_allocator = NetSlotAllocator::new();
    let broker = parking_lot::Mutex::new(EvaluationBroker::new());

    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    dispatch(
        vec![action],
        drivers.as_ref(),
        &alloc_drivers,
        obs_dyn.as_ref(),
        &SimDataplane::new(),
        &SimCa::new(Arc::new(SimEntropy::new(0))),
        &SimClock::new(),
        &IdentityMgr::new(None),
        &lifecycle_tx,
        &tick,
        &writer_node,
        Arc::clone(&allocator),
        &broker,
        None,
        None,
        &net_slot_allocator,
        &SimVmHostState::new(),
    )
    .await
}

/// The defect under fix: a `Running`-write failure after a successful
/// `driver.start` must tear the started workload down, not leave it
/// orphaned-live with a stranded exit watcher.
///
/// `SimObservationStore::inject_write_failure` is a one-shot FIFO; a
/// `StartAllocation` for an Exec alloc with `netns: None` performs exactly
/// ONE observation write (the `Running` row — reads do not consume the
/// FIFO), so the injected failure lands on it deterministically.
#[tokio::test]
async fn running_write_failure_stops_the_started_alloc() {
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Exec, Arc::new(SimClock::new())));

    // Fail exactly the next observation write — the `Running` row.
    obs.inject_write_failure(ObservationStoreError::Io(io::Error::from(
        io::ErrorKind::PermissionDenied,
    )));

    let result = dispatch_action(&obs, &sim_driver, start_action()).await;

    // The write error still surfaces to the caller (behaviour unchanged
    // from the bare `?` — the fix only ADDS teardown before propagating).
    assert!(
        result.is_err(),
        "the Running-write failure must propagate as a ShimError; got {result:?}",
    );

    // THE fix assertion: the driver started the workload (SimDriver::start
    // always succeeds → live_count 1) and the shim then STOPPED it on the
    // write failure (SimDriver::stop evicts the slot → live_count 0). A
    // live_count of 1 means the started workload was left orphaned — its
    // exit watcher stranded on the never-fired gate and its footprint
    // leaked past `live_allocations()` where `VmReclamation` cannot reach
    // it. This is the state greptile PR #268 P1 flagged.
    assert_eq!(
        sim_driver.live_count(),
        0,
        "on a Running-write failure the shim must stop the started alloc; live_count == 1 means \
         the fix regressed and the workload is orphaned-live (stranded watcher + leaked footprint)",
    );

    // Corroborates that the injection hit the Running write specifically:
    // the row never committed.
    let row = obs.alloc_status_row(&alloc_id()).await.expect("read alloc row");
    assert!(
        row.is_none(),
        "the Running write failed, so no AllocStatusRow was committed; got {row:?}",
    );
}

/// The non-vacuity control: when the `Running` write SUCCEEDS, the shim
/// must NOT stop the alloc — it stays live and supervised. Pins the
/// `if let Err(..)` guard (a mutant that stops unconditionally, or drops
/// the `state == Running` guard, fails here) so the failure test above
/// cannot pass for the wrong reason.
#[tokio::test]
async fn running_write_success_keeps_the_started_alloc() {
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Exec, Arc::new(SimClock::new())));

    // No injected failure — the Running write commits.
    let result = dispatch_action(&obs, &sim_driver, start_action()).await;
    assert!(result.is_ok(), "a clean StartAllocation must succeed; got {result:?}");

    assert_eq!(
        sim_driver.live_count(),
        1,
        "on a successful Running write the shim must NOT stop the alloc — it stays live and \
         supervised so its exit watcher can report the eventual exit",
    );

    let row = obs
        .alloc_status_row(&alloc_id())
        .await
        .expect("read alloc row")
        .expect("a Running row must exist after a successful start");
    assert_eq!(row.state, AllocState::Running, "the committed row is Running");
}

/// The VM (phased-claim) shape of the same fix — the leg an Exec driver
/// cannot exhibit. `VmDriver::stop` moves its supervision entry
/// `Live -> EndingInFlight` rather than removing it, so `stop` ALONE does
/// not retire the claim: `live_allocations()` keeps reporting the
/// torn-down alloc until `release_supervision` runs. On the
/// Running-write-failure path the exit watcher emits no `ExitEvent` (it
/// finds the entry already `EndingInFlight`), so the exit-observer
/// releaser never fires, and the shim's arm returns `Err` before its
/// terminal-write `release_supervision`. The ONLY releaser left is the
/// explicit `release_supervision` this arm must call right after `stop`
/// (the "authorship abandoned as impossible" clause of the
/// `Driver::release_supervision` contract). Without it the alloc leaks
/// past `live_allocations()` and pins `VmReclamation`
/// (`!reclamation_authorised`) for a dead id forever — greptile PR #268
/// P1 "Ending claim survives teardown".
///
/// A `DriverType::Vm` `SimDriver` faithfully models the phased claim, so
/// this goes RED if the `release_supervision` pairing is dropped — a leak
/// the Exec-shaped `live_count` assertion above structurally cannot see.
#[tokio::test]
async fn vm_running_write_failure_releases_the_supervision_claim() {
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::new(SimClock::new())));

    // Fail exactly the next observation write — the `Running` row.
    obs.inject_write_failure(ObservationStoreError::Io(io::Error::from(
        io::ErrorKind::PermissionDenied,
    )));

    let result = dispatch_action(&obs, &sim_driver, start_action_vm()).await;
    assert!(
        result.is_err(),
        "the Running-write failure must propagate as a ShimError; got {result:?}",
    );

    // THE fix assertion, at the phased-claim surface: after the shim stops
    // the alloc on the write failure it must ALSO release the supervision
    // claim `stop` left `EndingInFlight`. A non-empty `live_allocations()`
    // means the claim was NOT retired — the torn-down alloc leaks and
    // `VmReclamation` is pinned `!reclamation_authorised` for a dead id
    // forever. This is the exact state greptile PR #268 P1 flagged, and it
    // is invisible to a full-remove Exec driver.
    assert_eq!(
        sim_driver.live_allocations(),
        Some(Vec::new()),
        "on a Running-write failure the shim must release the supervision claim after stopping; a \
         non-empty live_allocations() means the EndingInFlight entry leaked past reclamation",
    );

    let row = obs.alloc_status_row(&alloc_id()).await.expect("read alloc row");
    assert!(
        row.is_none(),
        "the Running write failed, so no AllocStatusRow was committed; got {row:?}",
    );
}

/// VM non-vacuity control: a SUCCESSFUL Running write must NOT release the
/// claim — the alloc stays supervised (`Live`) so its exit watcher can
/// report the eventual exit. Pins the `if let Err(..)` guard (a mutant
/// that releases unconditionally, or drops the `state == Running` guard,
/// fails here) so the failure test above cannot pass for the wrong reason.
#[tokio::test]
async fn vm_running_write_success_keeps_the_supervision_claim() {
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::new(SimClock::new())));

    // No injected failure — the Running write commits.
    let result = dispatch_action(&obs, &sim_driver, start_action_vm()).await;
    assert!(result.is_ok(), "a clean StartAllocation must succeed; got {result:?}");

    assert_eq!(
        sim_driver.live_allocations(),
        Some(vec![alloc_id()]),
        "on a successful Running write the shim must NOT release the alloc — it stays supervised \
         (Live) so its exit watcher can report the eventual exit",
    );

    let row = obs
        .alloc_status_row(&alloc_id())
        .await
        .expect("read alloc row")
        .expect("a Running row must exist after a successful start");
    assert_eq!(row.state, AllocState::Running, "the committed row is Running");
}
