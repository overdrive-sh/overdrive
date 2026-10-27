//! GH #295 — the guest transport through the action shim: EXEC only after
//! forwarding is Active (P6-S12, G-V2; the G-V3 rows `QuiescenceLatched`,
//! `ControlSessionPending` and a Retiring lease's refused activation), VMM
//! death and the pre-READY exit
//! (P6-S10, G-V4 rows 4/5, ATTR-TOKEN), and the feature-disabled row
//! (P6-S12b, `AllocationSpec.transport: None`; C5a / C5b).
//!
//! Driven through the action shim's dispatch with the production guest-flow
//! owner (`HostSharedGuestNetworkOwner::with_flow_io` over `SimGuestFlowIo`),
//! the production `GuestAddressPool`, the production EXEC gate wiring and the
//! production `MtlsInterceptWorker` over `SimMtlsIntercept`. The driver is a
//! recording VM driver (the test's own `Driver` double at the driver port):
//! it reports READY at `start` and records the EXEC release hook. Faults enter
//! at driven ports only: the guest's control session and flows at the owner
//! seam, a VMM death at the seam (`vm_stop`), a pre-READY exit as the
//! driver's typed start failure. Quiescence in P6-S12 is a hold the test
//! takes through the owner's own `quiesce()` port (the test plays the
//! supervisor, which is the subject of `crate::shared_network_recovery_acceptance`).
//!
//! # Universe (port-exposed)
//!
//! The dispatch result; the recording driver's started specs and EXEC
//! releases; the owner's QPROJ-3 `flow_observation()` (CID phase, session,
//! held flows); the pool's QPROJ-2 `lease_observation()`; the seam's QPROJ-7
//! `socket_observation().closed_by_owner`; the allocation's status row.

#![allow(
    clippy::significant_drop_in_scrutinee,
    clippy::unnecessary_get_then_check,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "acceptance bodies fail at once on a broken precondition, naming it"
)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::guest_network::GuestNetworkExecWiring;
use overdrive_core::guest_transport::wire::{ControlOpen, FlowId, FlowOrigin, FlowRequest};
use overdrive_core::guest_transport::{
    GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX, GuestCid, GuestFlowPorts, LaunchToken, VsockListenPort,
};
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverPayload,
    DriverStartClass, DriverStartFailure, DriverType, Resources, VmPayload, VmStartFailure,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_core::vm::config::VsockPort;
use overdrive_dataplane::allocators::VipRange;
use overdrive_netlink::FibRouteType;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::{SimMtlsIntercept, SimMtlsResolve};
use overdrive_store_local::LocalIntentStore;
use parking_lot::Mutex;
use tempfile::TempDir;

use super::{
    ShimError, WorkloadNetworkProvisioner, dispatch_with_guest_network_provisioner_for_test,
    dispatch_with_workflow_intent_and_network_provisioner_for_test,
};
use crate::guest_network::flow_io::FlowListener;
use crate::guest_network::flow_io::sim::{SimEffectKind, SimGuestFlowIo};
use crate::guest_network::{
    CidPhase, GuestAddressPool, GuestConnKey, GuestFlowOwnerPorts, GuestNetworkError,
    GuestNetworkOperation, HostSharedGuestNetworkOwner, SharedGuestNetworkOwner,
};
use crate::shared_network_test_ports::worker_over;
use crate::veth_provisioner::{VethProvisionError, WorkloadNetnsPlan};

const LISTEN_PORT: u16 = 8080;
const DESTINATION: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 9), 443);
/// Bounded scheduler polls for the already-observable pre-activation state.
const ACTIVATION_PENDING_POLLS: usize = 256;

// ---------------------------------------------------------------------------
// The recording VM driver (the test's driver-port double)
// ---------------------------------------------------------------------------

/// How `start` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartScript {
    /// The guest reached READY.
    Ready,
    /// The hypervisor ended before the guest reported READY (the existing
    /// pre-READY VMM-exit start failure).
    ExitBeforeReady,
}

struct RecordingVmDriver {
    script: Mutex<StartScript>,
    started: Mutex<Vec<AllocationSpec>>,
    exec_released: Mutex<Vec<AllocationId>>,
    stopped: Mutex<Vec<AllocationId>>,
}

impl RecordingVmDriver {
    fn new() -> Self {
        Self {
            script: Mutex::new(StartScript::Ready),
            started: Mutex::new(Vec::new()),
            exec_released: Mutex::new(Vec::new()),
            stopped: Mutex::new(Vec::new()),
        }
    }

    fn script(&self, script: StartScript) {
        *self.script.lock() = script;
    }

    fn started(&self) -> Vec<AllocationSpec> {
        self.started.lock().clone()
    }

    fn exec_released(&self) -> Vec<AllocationId> {
        self.exec_released.lock().clone()
    }
}

#[async_trait::async_trait]
impl Driver for RecordingVmDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Vm
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        self.started.lock().push(spec.clone());
        match *self.script.lock() {
            StartScript::Ready => {
                Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(4242) })
            }
            StartScript::ExitBeforeReady => Err(DriverError::StartRejected {
                failure: DriverStartFailure {
                    class: DriverStartClass::Vm(VmStartFailure::GuestExitUnreported {
                        vmm_exit_code: None,
                        vmm_signal: Some(9),
                    }),
                    detail: "scripted: the VMM exited before READY".to_owned(),
                },
            }),
        }
    }

    async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.exec_released.lock().push(handle.alloc.clone());
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.stopped.lock().push(handle.alloc.clone());
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        Err(DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Ok(())
    }
}

/// The C3 lane's driven port for the feature-disabled body: it acknowledges
/// the derived plan and records that it was asked.
#[derive(Default)]
struct RecordingNetnsProvisioner {
    provisioned: Mutex<u32>,
}

impl WorkloadNetworkProvisioner for RecordingNetnsProvisioner {
    fn provision(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        *self.provisioned.lock() += 1;
        Ok(())
    }

    fn teardown(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The shim fixture
// ---------------------------------------------------------------------------

struct ShimFixture {
    state: crate::AppState,
    driver: Arc<RecordingVmDriver>,
    owner: Arc<HostSharedGuestNetworkOwner>,
    io: Arc<SimGuestFlowIo>,
    pool: Arc<GuestAddressPool>,
    _wiring: GuestNetworkExecWiring,
    _tmp: TempDir,
}

impl ShimFixture {
    /// The production owner booted through its own driving ports
    /// (`probe_startup`, `converge_shared`: boot steps 1 and 9), the EXEC gate
    /// opened as `run_server` does after boot (`open_after_boot`), and an
    /// `AppState` over them.
    async fn build() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let clock = Arc::new(SimClock::new());
        let intercept = Arc::new(SimMtlsIntercept::new());
        let worker = worker_over(Arc::clone(&intercept), Arc::clone(&clock));
        worker.start_shared_owner().await.expect("the shared mTLS owner starts over the sim");
        let io = SimGuestFlowIo::new(0x0295_0612);
        io.auto_release(SimEffectKind::ProbeForwarder);
        let owner = Arc::new(HostSharedGuestNetworkOwner::with_flow_io(
            GuestFlowOwnerPorts {
                clock: clock.clone(),
                resolve: Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh))
                    as Arc<dyn MtlsResolve>,
                mtls: Arc::clone(&worker),
                vip_range: VipRange::default(),
                node_prefix: GUEST_NODE_PREFIX,
                gateway: GUEST_NODE_GATEWAY,
                stream_port: VsockListenPort::KernelAssigned,
                control_port: VsockListenPort::KernelAssigned,
            },
            Arc::clone(&io) as Arc<dyn crate::guest_network::flow_io::GuestFlowIo>,
        ));
        owner.probe_startup().await.expect("the owner's startup probe passes over the sim");
        owner.converge_shared().await.expect("the owner's boot convergence");
        io.settled().await;
        let wiring = GuestNetworkExecWiring::new(clock.clone());
        assert!(wiring.supervisor().open_after_boot(), "the EXEC gate opens after boot");
        let pool = Arc::new(GuestAddressPool::new(
            GUEST_NODE_PREFIX,
            GUEST_NODE_GATEWAY,
            Arc::new(SimEntropy::new(0x0295_0012)),
        ));
        let driver = Arc::new(RecordingVmDriver::new());

        let mut runtime =
            crate::reconciler_runtime::ReconcilerRuntime::new_with_redb_view_store_for_test(
                tmp.path(),
            )
            .expect("runtime");
        runtime.register(crate::noop_heartbeat()).await.expect("register reconciler");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
        let obs: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(node_id(), 0));
        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        let state = crate::AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            Arc::clone(&driver) as Arc<dyn Driver>,
            clock,
            Arc::new(SimDataplane::new()),
            Arc::new(SimCa::new(Arc::new(SimEntropy::new(295)))),
            Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
            node_id(),
            allocator,
            crate::test_empty_listener_facts(),
            Ipv4Addr::LOCALHOST,
            worker,
            Arc::clone(&owner) as Arc<dyn SharedGuestNetworkOwner>,
            wiring.gate(),
            Arc::clone(&pool),
        );
        Self { state, driver, owner, io, pool, _wiring: wiring, _tmp: tmp }
    }

    fn tick() -> TickContext {
        let now = Instant::now();
        TickContext {
            now,
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_200)),
            tick: 1,
            deadline: now + Duration::from_secs(1),
        }
    }

    async fn dispatch(&self, action: Action) -> Result<(), ShimError> {
        Box::pin(dispatch_with_guest_network_provisioner_for_test(
            vec![action],
            &self.state,
            &Self::tick(),
            self.owner.as_ref(),
        ))
        .await
    }

    /// PORTS-CONFIG: the ports the owner's listeners bound at boot step 9;
    /// every connection key names them, never a number.
    fn flow_ports(&self) -> GuestFlowPorts {
        self.owner.flow_ports().expect("boot step 9's converge_shared bound the owner's listeners")
    }

    /// The lease the pool holds for `alloc`: its CID and launch token.
    fn lease(&self, alloc: &AllocationId) -> Option<(GuestCid, LaunchToken)> {
        self.pool.lease_observation().leases.get(alloc).map(|lease| (lease.cid, lease.launch))
    }

    /// Wait until the concurrent start has provisioned and started the VM but
    /// cannot activate it because no control session exists. This uses the
    /// fixture's existing owner/driver/pool observations rather than the Sim
    /// seam's paused-time idle signal: the production activation retry is
    /// intentionally runnable while it waits for the session.
    async fn activation_pending(&self, alloc: &AllocationId) -> (GuestCid, LaunchToken) {
        for _ in 0..ACTIVATION_PENDING_POLLS {
            if let Some((cid, launch)) = self.lease(alloc) {
                let started = self.driver.started().iter().any(|spec| &spec.alloc == alloc);
                let exec_released = self.driver.exec_released().contains(alloc);
                let observed = self.owner.flow_observation().await;
                let pending = observed
                    .cids
                    .get(&cid)
                    .is_some_and(|cid| cid.phase == CidPhase::Provisioned && cid.session.is_none());
                if started && pending && !exec_released {
                    return (cid, launch);
                }
            }
            tokio::task::yield_now().await;
        }
        panic!(
            "the concurrent start did not reach session-pending activation within \
             {ACTIVATION_PENDING_POLLS} scheduler polls; lease={:?}, started={:?}, exec={:?}",
            self.lease(alloc),
            self.driver.started(),
            self.driver.exec_released(),
        );
    }

    /// The guest of `cid` opens its control session with `launch`.
    async fn open_control_session(&self, cid: GuestCid, launch: LaunchToken) {
        let key = GuestConnKey {
            listener: self.flow_ports().control(),
            peer_cid: cid,
            peer_port: VsockPort::new(49_100),
        };
        self.io.queue_connection(
            FlowListener::Control,
            key,
            Some(ControlOpen { launch }.encode().to_vec()),
            Some(launch),
        );
        self.io.settled().await;
    }

    /// The guest of `cid` opens a TCP flow to [`DESTINATION`] carrying
    /// `launch`; returns its connection key.
    async fn open_flow(&self, cid: GuestCid, launch: LaunchToken, peer_port: u32) -> GuestConnKey {
        self.io.set_route_type(*DESTINATION.ip(), FibRouteType::Unicast);
        let key = GuestConnKey {
            listener: self.flow_ports().stream(),
            peer_cid: cid,
            peer_port: VsockPort::new(peer_port),
        };
        let request = FlowRequest::TcpConnect {
            flow: FlowId::new(FlowOrigin::Guest, peer_port).expect("guest flow id"),
            destination: DESTINATION,
            launch,
        };
        self.io.queue_connection(
            FlowListener::Stream,
            key,
            Some(request.encode().to_vec()),
            Some(launch),
        );
        self.io.settled().await;
        key
    }

    async fn cid_phase(&self, cid: GuestCid) -> Option<CidPhase> {
        self.owner.flow_observation().await.cids.get(&cid).map(|observed| observed.phase)
    }
}

fn node_id() -> NodeId {
    NodeId::new("nd295-shim-node").expect("node id")
}

fn workload_id() -> WorkloadId {
    WorkloadId::new("nd295-shim-service").expect("workload id")
}

fn alloc_id(name: &str) -> AllocationId {
    AllocationId::new(name).expect("allocation id")
}

fn spec(name: &str) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc_id(name),
        identity: SpiffeId::new(&format!("spiffe://overdrive.local/workload/nd295/alloc/{name}"))
            .expect("SPIFFE ID"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/nd295/kernel"),
            rootfs: PathBuf::from("/nd295/rootfs.ext4"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        transport: None,
        service_ports: vec![NonZeroU16::new(LISTEN_PORT).expect("non-zero listener port")],
    }
}

fn start(name: &str) -> Action {
    Action::StartAllocation {
        alloc_id: alloc_id(name),
        workload_id: workload_id(),
        node_id: node_id(),
        spec: spec(name),
        kind: WorkloadKind::Service,
    }
}

// ---------------------------------------------------------------------------
// P6-S12 — EXEC only after Active
// ---------------------------------------------------------------------------

/// P6-S12 — The workload command is released only after forwarding is Active
/// for the allocation; activation waits out quiescence and a pending control
/// session under the EXEC claim.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a recovery holds quiescence. When the allocation is started through
/// the shim, then it is leased and provisioned, the driver starts it with the
/// lease's transport (CID and launch token), and while the hold remains its
/// CID stays Provisioned and EXEC is not released. When the hold is restored
/// with no control session yet, then EXEC is still not released. When the
/// guest opens its control session with the live launch token, then the CID
/// becomes Active and EXEC is released exactly once.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn exec_is_released_only_after_the_allocation_is_active() {
    let fixture = ShimFixture::build().await;
    let (hold, _quiescence) = fixture.owner.quiesce().await;
    let name = "nd295-p6-s12";

    let (dispatched, ()) = tokio::join!(fixture.dispatch(start(name)), async {
        let (cid, launch) = fixture.activation_pending(&alloc_id(name)).await;
        let started = fixture.driver.started();
        assert_eq!(started.len(), 1, "the VM is started once");
        let transport = started[0].transport.as_ref().expect("the spec carries the transport");
        assert_eq!(
            (transport.cid(), transport.launch()),
            (cid, launch),
            "the lease's CID and token"
        );
        assert_eq!(
            fixture.cid_phase(cid).await,
            Some(CidPhase::Provisioned),
            "quiesced: not Active"
        );
        assert!(fixture.driver.exec_released().is_empty(), "no EXEC while quiesced");

        let restored = fixture.owner.restore(hold).await;
        assert_eq!(
            restored,
            Ok(crate::guest_network::ForwardingRestore::Reopened),
            "the owner's own hold is redeemed at its ledger and reopens at zero"
        );
        let (pending_cid, pending_launch) = fixture.activation_pending(&alloc_id(name)).await;
        assert_eq!((pending_cid, pending_launch), (cid, launch), "the pending lease is stable");
        assert_eq!(
            fixture.cid_phase(cid).await,
            Some(CidPhase::Provisioned),
            "no session: not Active"
        );
        assert!(fixture.driver.exec_released().is_empty(), "no EXEC before the session is live");

        fixture.open_control_session(cid, launch).await;
    });
    dispatched.expect("the start completes once activated");

    let (cid, _) = fixture.lease(&alloc_id(name)).expect("still leased");
    assert_eq!(fixture.cid_phase(cid).await, Some(CidPhase::Active));
    assert_eq!(fixture.driver.exec_released(), vec![alloc_id(name)], "EXEC released exactly once");
    fixture
        .dispatch(Action::StopAllocation { alloc_id: alloc_id(name), terminal: None })
        .await
        .expect("fixture cleanup tears down the active allocation");
}

/// P6-S12 (row 4) — An activation after teardown began never releases EXEC:
/// `activate` of a Retiring CID is refused and changes nothing.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given an Active allocation whose control session is live. When it is
/// stopped through the shim (teardown), then a later `activate` of its plan
/// is refused with `PostconditionMismatch { operation: ForwardingActivate }`,
/// the owner holds no CID for it, and no second EXEC is released.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_activation_after_teardown_is_refused_and_releases_no_exec() {
    let fixture = ShimFixture::build().await;
    let name = "nd295-p6-s12-row4";
    let (dispatched, ()) = tokio::join!(fixture.dispatch(start(name)), async {
        let (cid, launch) = fixture.activation_pending(&alloc_id(name)).await;
        fixture.open_control_session(cid, launch).await;
    });
    dispatched.expect("the start completes");
    let plan = fixture.pool.snapshot().remove(&alloc_id(name)).expect("the Admitted plan");
    let (cid, _) = fixture.lease(&alloc_id(name)).expect("leased");

    fixture
        .dispatch(Action::StopAllocation { alloc_id: alloc_id(name), terminal: None })
        .await
        .expect("the stop completes");
    let late =
        crate::guest_network::GuestNetworkProvisioner::activate(fixture.owner.as_ref(), &plan)
            .await;
    assert!(
        matches!(
            late,
            Err(GuestNetworkError::PostconditionMismatch {
                operation: GuestNetworkOperation::ForwardingActivate,
                ..
            })
        ),
        "a late activation of a torn-down CID is refused: {late:?}"
    );
    assert_eq!(fixture.cid_phase(cid).await, None, "the owner holds no CID for it");
    assert_eq!(fixture.driver.exec_released(), vec![alloc_id(name)], "no second EXEC");
}

// ---------------------------------------------------------------------------
// P6-S10 — VMM death and the pre-READY exit
// ---------------------------------------------------------------------------

/// P6-S10 — A VMM exit before READY is the existing non-terminal start
/// failure: no EXEC, the owner holds no CID for the allocation and its lease
/// is released.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given the driver's VMM exits before READY. When the allocation is started
/// through the shim, then no EXEC is released, the allocation's row is not
/// Running, the owner holds no CID for it, and the pool holds no lease for
/// it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_vmm_exit_before_ready_releases_no_exec_and_no_lease_survives() {
    let fixture = ShimFixture::build().await;
    fixture.driver.script(StartScript::ExitBeforeReady);
    let name = "nd295-p6-s10-preready";

    let _non_terminal = fixture.dispatch(start(name)).await;

    assert!(fixture.driver.exec_released().is_empty(), "no EXEC after a pre-READY exit");
    let row = fixture.state.obs.alloc_status_row(&alloc_id(name)).await.expect("row readable");
    assert!(
        row.as_ref().is_none_or(|row| row.state != AllocState::Running),
        "a pre-READY exit is never Running: {row:?}"
    );
    assert!(fixture.lease(&alloc_id(name)).is_none(), "the lease is released after the VMM exited");
    assert!(
        fixture.owner.flow_observation().await.cids.values().all(|cid| cid.alloc != alloc_id(name)),
        "the owner holds no CID for the failed allocation"
    );
}

/// P6-S10 — A VMM death aborts every flow of its CID; after teardown a flow
/// request of the dead launch is closed unattributed and registers nothing.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given an Active allocation with a live control session and one paired
/// guest-opened flow. When its VMM dies, then the owner holds no flow of the
/// CID. When the allocation is stopped through the shim and a flow request
/// carrying the dead launch's token reaches the flow listener from its CID,
/// then the owner closes it, holds no CID and registers no flow for it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_vmm_death_aborts_its_flows_and_a_stale_launch_flow_is_closed_unattributed() {
    let fixture = ShimFixture::build().await;
    let name = "nd295-p6-s10-death";
    let (dispatched, ()) = tokio::join!(fixture.dispatch(start(name)), async {
        let (cid, launch) = fixture.activation_pending(&alloc_id(name)).await;
        fixture.open_control_session(cid, launch).await;
    });
    dispatched.expect("the start completes");
    let (cid, launch) = fixture.lease(&alloc_id(name)).expect("leased");
    fixture.open_flow(cid, launch, 51_000).await;
    let flows = |observed: &crate::guest_network::GuestOwnerObservation| {
        observed.cids.get(&cid).map_or(0, |observed| observed.guest_flows.len())
    };
    assert_eq!(flows(&fixture.owner.flow_observation().await), 1, "precondition: one paired flow");

    fixture.io.vm_stop(cid);
    fixture.io.settled().await;
    assert_eq!(
        flows(&fixture.owner.flow_observation().await),
        0,
        "the VMM death aborts the CID's flows"
    );

    fixture
        .dispatch(Action::StopAllocation { alloc_id: alloc_id(name), terminal: None })
        .await
        .expect("the stop completes");
    let stale = fixture.open_flow(cid, launch, 51_001).await;
    let observed = fixture.owner.flow_observation().await;
    assert!(observed.cids.get(&cid).is_none(), "nothing registers for the torn-down CID");
    // conformance-lint: sim-arbiter-ok QPROJ-7 owner-seam socket arbiter (connections the owner closed), Tier-3 R5-16 / R5-41
    assert!(
        fixture.io.socket_observation().closed_by_owner.contains(&stale),
        "the stale launch's connection is closed by the owner"
    );
}

// ---------------------------------------------------------------------------
// P6-S12b — feature disabled (`transport: None`)
// ---------------------------------------------------------------------------

/// P6-S12b — An allocation started without the guest transport runs as
/// today: no lease, no activation, EXEC on today's path; the same allocation
/// with the guest network composed is leased and activated before EXEC.
/// CONTRACT_SHAPE: unbounded-preservation.
///
/// Given a VM allocation dispatched through the shim lane with no guest
/// network (the pre-feature `network: None` baseline). When it starts, then
/// the driver sees `transport: None`, the pool holds no lease, the owner
/// holds no CID, the C3 provisioner was asked once, the row is Running and
/// EXEC is released once. And when the same allocation shape is dispatched
/// with the guest network composed, then it is leased, its spec carries the
/// transport, and EXEC follows its activation (C5b contrast).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_allocation_without_a_guest_transport_runs_as_today() {
    let fixture = ShimFixture::build().await;
    let netns = RecordingNetnsProvisioner::default();
    let disabled = "nd295-p6-s12b-off";

    Box::pin(dispatch_with_workflow_intent_and_network_provisioner_for_test(
        vec![start(disabled)],
        &fixture.state,
        &ShimFixture::tick(),
        &netns,
    ))
    .await
    .expect("the pre-feature lane starts the allocation");

    let started = fixture.driver.started();
    assert_eq!(started.len(), 1);
    assert!(started[0].transport.is_none(), "no guest transport on the pre-feature lane");
    assert!(fixture.lease(&alloc_id(disabled)).is_none(), "no lease is taken");
    assert!(fixture.owner.flow_observation().await.cids.is_empty(), "no CID, no activate");
    assert_eq!(*netns.provisioned.lock(), 1, "today's C3 lane provisioned once");
    let row = fixture
        .state
        .obs
        .alloc_status_row(&alloc_id(disabled))
        .await
        .expect("row readable")
        .expect("a row");
    assert_eq!(row.state, AllocState::Running);
    assert_eq!(fixture.driver.exec_released(), vec![alloc_id(disabled)], "EXEC on today's path");

    let enabled = "nd295-p6-s12b-on";
    let (dispatched, ()) = tokio::join!(fixture.dispatch(start(enabled)), async {
        let (cid, launch) = fixture.activation_pending(&alloc_id(enabled)).await;
        assert!(
            !fixture.driver.exec_released().contains(&alloc_id(enabled)),
            "no EXEC before activation"
        );
        fixture.open_control_session(cid, launch).await;
    });
    dispatched.expect("the guest-network lane starts the allocation");
    let started = fixture.driver.started();
    assert!(
        started.iter().any(|spec| spec.alloc == alloc_id(enabled) && spec.transport.is_some()),
        "the composed lane carries the transport"
    );
    assert_eq!(
        fixture.driver.exec_released(),
        vec![alloc_id(disabled), alloc_id(enabled)],
        "EXEC once each, the composed one after activation"
    );
    fixture
        .dispatch(Action::StopAllocation { alloc_id: alloc_id(enabled), terminal: None })
        .await
        .expect("fixture cleanup tears down the active guest allocation");
}
