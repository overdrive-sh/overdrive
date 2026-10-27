//! The production owner over the seam's sim (DG-3): one composition every
//! source-local body and every conformance driver of the host guest-flow
//! owner stands on.
//!
//! The owner is built exactly as `run_server` builds it, by
//! `compose_guest_network` (D-UP-16, shaped by LEGS-DIAL): the owner and its
//! mTLS worker in one construction, the inbound enforcement dialling leg S
//! through the composition's one `OwnerDial` (bound to the owner) and the
//! owner holding the worker, with every port explicit: `SimClock` for every deadline and
//! the audit cadence, `SimMtlsResolve` for the single resolution (the owner's
//! and the worker's), sim enforcement and the sim intercept, the
//! configured VIP range, and the production node prefix and gateway (DG-9).
//! The pool is the production pool with seeded `SimEntropy` for launch tokens
//! (ATTR-TOKEN). Only the owner seam is simulated (`SimGuestFlowIo`, QPROJ-1
//! rule 3), and through it the forwarder (`SimFlowForwarder`, the named
//! arbiter of the forwarder's kernel facts, QPROJ-7 / QPROJ-8). Nothing here
//! writes owner state: every lease, session, flow and dial is produced by the
//! owner's own driving ports from stimuli the sim delivers, and every flow
//! end is a cause scripted on a socket (the guest's `0x2` / `0x3` / `RST`,
//! the destination's FIN or reset, the device release), classified by the
//! sim forwarder — never an end record a test writes.
//!
//! Seeds: every composition is built from a seed (launch tokens, the sim's
//! accept order); the seed is printed when a body starts so a failure
//! reproduces bit-for-bit (`.claude/rules/testing.md` § "Seeding and
//! reproducibility").

#![allow(
    clippy::redundant_pub_crate,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    reason = "test fixtures assert their own preconditions"
)]
#![allow(
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    reason = "test support and conformance drivers: fixture preconditions are asserted with expect, and handler signatures follow the action map"
)]
#![allow(
    dead_code,
    reason = "shared by the default-lane bodies and the integration-tests conformance drivers; each lane uses a subset"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use ipnet::Ipv4Net;
use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial,
};
use overdrive_core::guest_transport::forwarder::ForwardId;
use overdrive_core::guest_transport::wire::{
    ControlMessage, ControlOpen, FlowId, FlowOrigin, FlowRequest,
};
use overdrive_core::guest_transport::{
    GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX, GuestCid, GuestFlowPorts, LaunchToken, VsockListenPort,
};
use overdrive_core::id::AllocationId;
use overdrive_core::net::ListenPort;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{AllocationSpec, DriverPayload, Resources, VmPayload};
use overdrive_core::traits::mtls_enforcement::MtlsLimits;
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_core::vm::config::VsockPort;
use overdrive_core::{SpiffeId, WorkloadId};
use overdrive_dataplane::allocators::VipRange;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsIntercept, SimMtlsResolve};
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
use parking_lot::Mutex;
use tokio::task::JoinHandle;

use crate::guest_network::flow_io::sim::{SimEffectKind, SimGuestFlowIo};
use crate::guest_network::flow_io::{FlowListener, GuestFlowIo};
use crate::guest_network::{
    GuestAddressPool, GuestConnKey, GuestFlowPhase, GuestNetworkPlan, GuestNetworkProvisioner,
    GuestOwnerObservation, HostSharedGuestNetworkOwner, SharedGuestNetworkOwner,
};
use crate::guest_network_composition::OwnerDial;

/// The configured service VIP range of every composition here: disjoint
/// from the guest prefix (`100.95.0.0/16`), so `probe_startup` passes.
pub(crate) fn vip_range() -> Ipv4Net {
    "10.96.0.0/20".parse().expect("a valid CIDR")
}

/// A current-thread runtime whose time is paused: `SimGuestFlowIo::settled`
/// detects an idle owner through its auto-advance (the drivers' runtime).
pub(crate) fn paused_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("a current-thread runtime builds")
}

/// One lease the pool's `assign` produced and the owner provisioned.
#[derive(Debug, Clone)]
pub(crate) struct Lease {
    /// The allocation.
    pub(crate) alloc: AllocationId,
    /// The plan `assign` returned.
    pub(crate) plan: GuestNetworkPlan,
    /// `GUEST_CID_BASE + lease offset`.
    pub(crate) cid: GuestCid,
    /// The lease's launch token.
    pub(crate) launch: LaunchToken,
    /// The lease's workload address.
    pub(crate) workload_addr: Ipv4Addr,
}

impl Lease {
    /// `workload_addr:port`.
    pub(crate) fn target(&self, port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(self.workload_addr, port)
    }
}

/// The production owner and pool over the seam's sim.
pub(crate) struct Composition {
    pub(crate) seed: u64,
    pub(crate) io: Arc<SimGuestFlowIo>,
    pub(crate) owner: Arc<HostSharedGuestNetworkOwner>,
    pub(crate) pool: Arc<GuestAddressPool>,
    pub(crate) clock: SimClock,
    pub(crate) resolve: Arc<SimMtlsResolve>,
    /// The mTLS worker the owner registers forwarded outbound flows with
    /// (D9): the same instance the owner holds (`GuestFlowOwnerPorts::mtls`).
    pub(crate) mtls: Arc<MtlsInterceptWorker>,
    /// The composition's one `OwnerDial` (D-13): bound to `owner` by
    /// `compose_guest_network`, and the `guest_dial` the composition's
    /// `SimMtlsEnforcement` was built with (LEGS-DIAL), so a leg-S dial
    /// through the enforcement reaches `owner`.
    pub(crate) dial: Arc<OwnerDial>,
}

impl Composition {
    /// The composition, not booted, with `resolve` at the resolution port
    /// and launch tokens drawn from `SimEntropy` seeded with `seed`.
    pub(crate) fn new(resolve: SimMtlsResolve, seed: u64) -> Self {
        Self::with(resolve, vip_range(), seed)
    }

    /// As [`Self::new`], with `vip` as the configured service VIP range.
    pub(crate) fn with(resolve: SimMtlsResolve, vip: Ipv4Net, seed: u64) -> Self {
        Self::with_prefix(resolve, vip, GUEST_NODE_PREFIX, GUEST_NODE_GATEWAY, seed)
    }

    /// As [`Self::with`], over the guest prefix `prefix` with `gateway` (the
    /// same values for the owner and the pool, as `run_server` passes them).
    /// A `/30` leaves the pool one usable offset, so every lease reuses one
    /// CID — the conformance drivers' one-CID abstraction (`owner_flows`).
    pub(crate) fn with_prefix(
        resolve: SimMtlsResolve,
        vip: Ipv4Net,
        prefix: Ipv4Net,
        gateway: Ipv4Addr,
        seed: u64,
    ) -> Self {
        eprintln!("guest-flow owner composition seed = {seed}");
        let io = SimGuestFlowIo::new(seed);
        io.auto_release(SimEffectKind::ProbeForwarder);
        let clock = SimClock::new();
        let resolve = Arc::new(resolve);
        // `run_server`'s construction of the owner and its mTLS worker
        // (D-UP-16, as changed under LEGS-DIAL): the boot's one unbound
        // `OwnerDial` is the enforcement's `guest_dial`, and
        // `compose_guest_network` binds it to the owner it builds. The
        // caller's precondition of `compose_guest_network` — `dial` is the
        // same `Arc` the enforcement was built with — holds by construction:
        // both are built here from the one `dial`.
        let dial = Arc::new(OwnerDial::unbound());
        let ports = crate::GuestNetworkCompositionPorts {
            clock: Arc::new(clock.clone()) as Arc<dyn Clock>,
            resolve: Arc::clone(&resolve) as Arc<dyn MtlsResolve>,
            enforcement: Arc::new(SimMtlsEnforcement::new(
                Arc::new(SimIdentityRead::new(BTreeMap::new(), None)),
                MtlsLimits::default(),
                Arc::clone(&dial) as Arc<dyn GuestStreamDial>,
            )),
            intercept: Arc::new(SimMtlsIntercept::new()),
            vip_range: VipRange::new(vec![vip], BTreeSet::new()).expect("valid VIP range"),
            node_prefix: prefix,
            gateway,
            // PORTS-CONFIG: the listeners are kernel-assigned (the seam
            // chooses); every connection key reads the bound ports back from
            // the owner (`flow_ports`), never a number.
            stream_port: VsockListenPort::KernelAssigned,
            control_port: VsockListenPort::KernelAssigned,
        };
        let (owner, mtls) = crate::compose_guest_network(
            ports,
            Arc::clone(&io) as Arc<dyn GuestFlowIo>,
            Arc::clone(&dial),
        );
        let pool =
            Arc::new(GuestAddressPool::new(prefix, gateway, Arc::new(SimEntropy::new(seed))));
        Self { seed, io, owner, pool, clock, resolve, mtls, dial }
    }

    /// The composition with every non-mesh destination resolved `NonMesh`.
    pub(crate) fn non_mesh(seed: u64) -> Self {
        Self::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh), seed)
    }

    /// Boot steps 1, 7 and 9 of § *Composition* on the owner:
    /// `probe_startup`, `sweep_stale`, `converge_shared` (the forwarder
    /// context opened and probed, the listeners bound, the gateway route
    /// converged).
    pub(crate) async fn boot(&self) {
        self.owner.probe_startup().await.expect("probe_startup passes on a clean node");
        self.owner.sweep_stale().await.expect("no stale owner object");
        self.owner.converge_shared().await.expect("converge_shared opens the forwarder context");
        self.settle().await;
    }

    /// A booted composition resolving every destination `NonMesh`.
    pub(crate) async fn booted(seed: u64) -> Self {
        let composition = Self::non_mesh(seed);
        composition.boot().await;
        composition
    }

    /// The ports the owner's listeners bound (PORTS-CONFIG): `Some` from
    /// its first `Ok` `converge_shared` ([`Self::boot`]), constant after.
    pub(crate) fn flow_ports(&self) -> GuestFlowPorts {
        self.owner
            .flow_ports()
            .expect("boot first: converge_shared binds the owner's flow and control listeners")
    }

    /// Every owner task is parked with nothing undelivered.
    pub(crate) async fn settle(&self) {
        self.io.settled().await;
    }

    /// QPROJ-3, after the owner settled.
    pub(crate) async fn observe(&self) -> GuestOwnerObservation {
        self.settle().await;
        self.owner.flow_observation().await
    }

    /// Advances the owner's clock by `by`, then settles.
    pub(crate) async fn advance(&self, by: Duration) {
        self.clock.tick(by);
        self.settle().await;
    }

    /// `pool.assign` with `ports` declared and the owner's bound flow ports
    /// (PORTS-CONFIG, carried into the assignment), then `owner.provision`.
    pub(crate) async fn lease(&self, name: &str, ports: &[u16]) -> Lease {
        let alloc = AllocationId::new(name).expect("a valid allocation id");
        let listen_ports: Vec<ListenPort> =
            ports.iter().map(|port| ListenPort::new(*port).expect("non-zero port")).collect();
        let plan = self
            .pool
            .assign(alloc.clone(), &listen_ports, self.flow_ports())
            .expect("the pool admits the lease");
        self.owner.provision(&plan).await.expect("provision registers the CID");
        let assignment = plan.assignment();
        let lease = Lease {
            alloc,
            cid: assignment.cid(),
            launch: assignment.launch(),
            workload_addr: assignment.workload_addr(),
            plan: plan.clone(),
        };
        self.settle().await;
        lease
    }

    /// The guest of `lease` opens its control session from `peer_port`,
    /// carrying `launch` in its `ControlOpen`.
    pub(crate) fn open_session_with(
        &self,
        lease: &Lease,
        peer_port: u32,
        launch: LaunchToken,
    ) -> GuestConnKey {
        let key = conn_key(self.flow_ports().control(), lease.cid, peer_port);
        self.io.queue_connection(
            FlowListener::Control,
            key,
            Some(ControlOpen { launch }.encode().to_vec()),
            Some(lease.launch),
        );
        key
    }

    /// The guest of `lease` opens its control session with the lease's
    /// token.
    pub(crate) fn open_session(&self, lease: &Lease, peer_port: u32) -> GuestConnKey {
        self.open_session_with(lease, peer_port, lease.launch)
    }

    /// A lease whose guest opened its control session and which the owner
    /// activated (`Activated`).
    pub(crate) async fn active_lease(&self, name: &str, ports: &[u16]) -> (Lease, GuestConnKey) {
        let lease = self.lease(name, ports).await;
        let session = self.open_session(&lease, 1024);
        self.settle().await;
        let activation = self.owner.activate(&lease.plan).await.expect("activation succeeds");
        assert_eq!(activation, crate::guest_network::ForwardingActivation::Activated);
        self.settle().await;
        (lease, session)
    }

    /// The guest of `lease` opens a TcpConnect flow `seq` toward
    /// `destination` from vsock port `peer_port`, its request buffered.
    pub(crate) fn open_tcp_flow(
        &self,
        lease: &Lease,
        seq: u32,
        destination: SocketAddrV4,
        peer_port: u32,
    ) -> (GuestConnKey, FlowId) {
        self.open_tcp_flow_with(lease, seq, destination, peer_port, lease.launch)
    }

    /// As [`Self::open_tcp_flow`], carrying `launch`.
    pub(crate) fn open_tcp_flow_with(
        &self,
        lease: &Lease,
        seq: u32,
        destination: SocketAddrV4,
        peer_port: u32,
        launch: LaunchToken,
    ) -> (GuestConnKey, FlowId) {
        let flow = guest_flow(seq);
        let key = conn_key(self.flow_ports().stream(), lease.cid, peer_port);
        let request = FlowRequest::TcpConnect { flow, destination, launch };
        self.io.queue_connection(
            FlowListener::Stream,
            key,
            Some(request.encode().to_vec()),
            Some(lease.launch),
        );
        (key, flow)
    }

    /// The guest of `lease` opens datagram association `seq` toward
    /// `destination`: its one flow-stream connection (request kind 2,
    /// EVMOD-DGRAM) with its `DatagramStream` request buffered.
    pub(crate) fn open_datagram(
        &self,
        lease: &Lease,
        seq: u32,
        destination: SocketAddrV4,
        peer_port: u32,
    ) -> (GuestConnKey, FlowId) {
        let flow = guest_flow(seq);
        let key = conn_key(self.flow_ports().stream(), lease.cid, peer_port);
        let request = FlowRequest::DatagramStream { flow, destination, launch: lease.launch };
        self.io.queue_connection(
            FlowListener::Stream,
            key,
            Some(request.encode().to_vec()),
            Some(lease.launch),
        );
        (key, flow)
    }

    /// The guest of `lease` opens a flow-stream connection whose first 32
    /// bytes are `record` (not a canonical request).
    pub(crate) fn open_raw_flow(
        &self,
        lease: &Lease,
        record: Vec<u8>,
        peer_port: u32,
    ) -> GuestConnKey {
        let key = conn_key(self.flow_ports().stream(), lease.cid, peer_port);
        self.io.queue_connection(FlowListener::Stream, key, Some(record), Some(lease.launch));
        key
    }

    /// The guest writes `message` on its control session `session`.
    pub(crate) fn guest_sends(&self, session: GuestConnKey, message: ControlMessage) {
        self.io.send_on_connection(session, &message.encode());
    }

    /// The dialer calls `GuestStreamDial::dial(target, purpose)` on a task.
    pub(crate) fn spawn_dial(
        &self,
        target: SocketAddrV4,
        purpose: DialPurpose,
    ) -> JoinHandle<Result<GuestStream, GuestDialError>> {
        let owner = Arc::clone(&self.owner);
        tokio::spawn(async move { owner.dial(target, purpose).await })
    }

    /// The host-origin flow id the owner wrote in the `index`-th dial's
    /// TcpAccept request.
    pub(crate) fn dial_flow(&self, index: usize) -> FlowId {
        self.io
            .dials()
            .get(index)
            .and_then(|dial| dial.request)
            .map(|request| request.flow())
            .expect("the dial's TcpAccept request was written")
    }
}

impl Composition {
    /// `lease`'s allocation becomes intercept-live on the composition's mTLS
    /// worker, as the action shim's `start_alloc` makes it (D9: the owner's
    /// forwarded-outbound registration requires it). Call once per
    /// composition.
    pub(crate) async fn intercept_live(&self, lease: &Lease) {
        self.mtls.start_shared_owner().await.expect("the shared mTLS owner starts");
        let spec = AllocationSpec {
            identity: SpiffeId::for_allocation(
                &WorkloadId::new("guest-flow-owner").expect("valid workload id"),
                &lease.alloc,
            ),
            alloc: lease.alloc.clone(),
            driver: DriverPayload::Vm(VmPayload {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
                kernel: std::path::PathBuf::from("/nonexistent/kernel"),
                rootfs: std::path::PathBuf::from("/nonexistent/rootfs"),
            }),
            resources: Resources { cpu_milli: 1, memory_bytes: 1 },
            probe_descriptors: Vec::new(),
            transport: Some(lease.plan.assignment().clone()),
            service_ports: Vec::new(),
        };
        self.mtls.start_alloc(&spec).await.expect("the allocation becomes intercept-live");
    }

    /// A guest-opened TcpConnect flow `seq` toward `destination`, admitted,
    /// read, connected, attached and `Paired` by the owner; its connection
    /// key, flow id and pair id.
    pub(crate) async fn paired_tcp(
        &self,
        lease: &Lease,
        seq: u32,
        destination: SocketAddrV4,
        peer_port: u32,
    ) -> (GuestConnKey, FlowId, ForwardId) {
        let (key, flow) = self.open_tcp_flow(lease, seq, destination, peer_port);
        let observed = self.observe().await;
        let pair = match observed.cids[&lease.cid].guest_flows.get(&key) {
            Some(GuestFlowPhase::Paired { pair, .. }) => *pair,
            other => panic!("flow {seq} did not pair: {other:?}"),
        };
        assert!(sent_to(self, lease.cid).contains(&ControlMessage::Paired { flow }));
        (key, flow, pair)
    }

    /// A dial of `target` for `purpose`, paired by the guest's `Paired` on
    /// `session`: the stream and its host-origin flow id. The dial is the
    /// `index`-th the owner opened.
    pub(crate) async fn paired_dial(
        &self,
        session: GuestConnKey,
        target: SocketAddrV4,
        purpose: DialPurpose,
        index: usize,
    ) -> (GuestStream, FlowId) {
        let dial = self.spawn_dial(target, purpose);
        self.settle().await;
        let flow = self.dial_flow(index);
        self.guest_sends(session, ControlMessage::Paired { flow });
        self.settle().await;
        let stream = dial.await.expect("the dial task does not panic").expect("the dial pairs");
        (stream, flow)
    }
}

impl Drop for Composition {
    /// The `serve` process of this composition ends with it: every seam
    /// handle the owner still holds dies with the process, as at a real
    /// process exit (the seam sim then does not report them as leaked).
    fn drop(&mut self) {
        self.io.exit_process();
    }
}

/// A guest-origin flow id.
pub(crate) fn guest_flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Guest, seq).expect("seq below 2^31")
}

/// The kernel-reported key of a guest-opened host vsock connection.
pub(crate) fn conn_key(listener: VsockPort, cid: GuestCid, peer_port: u32) -> GuestConnKey {
    GuestConnKey { listener, peer_cid: cid, peer_port: VsockPort::new(peer_port) }
}

/// A token no lease of these compositions carries (all `0xAB`).
pub(crate) fn foreign_token() -> LaunchToken {
    LaunchToken::from_bytes([0xAB; 16]).expect("non-zero token")
}

/// A non-mesh destination outside every host-internal class.
pub(crate) fn remote(port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 10), port)
}

/// The control messages the owner wrote to `cid`, without consuming them.
pub(crate) fn sent_to(composition: &Composition, cid: GuestCid) -> Vec<ControlMessage> {
    composition.io.outbox(cid)
}

// ---------------------------------------------------------------------------
// Owner telemetry (structured tracing; § *Owner and provisioner*,
// "Telemetry") — the pinned event names and fields are part of the contract
// ---------------------------------------------------------------------------

/// One structured event the owner emitted: its pinned name, level and
/// fields (by name, as their `Display` / `Debug` text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnerEvent {
    pub(crate) name: String,
    pub(crate) level: tracing::Level,
    pub(crate) fields: BTreeMap<String, String>,
}

impl OwnerEvent {
    /// The value of field `name`.
    pub(crate) fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

/// Captures every event emitted on this thread while its guard lives (the
/// bodies run on a current-thread runtime, so every owner task emits here).
#[derive(Debug, Clone, Default)]
pub(crate) struct EventCapture {
    events: Arc<Mutex<Vec<OwnerEvent>>>,
}

impl EventCapture {
    /// Installs the capture as this thread's default subscriber.
    pub(crate) fn install() -> (Self, tracing::subscriber::DefaultGuard) {
        use tracing_subscriber::layer::SubscriberExt as _;
        let capture = Self::default();
        let guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(capture.clone()));
        (capture, guard)
    }

    /// Every captured event named `name`, in emission order.
    pub(crate) fn named(&self, name: &str) -> Vec<OwnerEvent> {
        self.events.lock().iter().filter(|event| event.name == name).cloned().collect()
    }
}

struct FieldText<'a>(&'a mut BTreeMap<String, String>);

impl tracing::field::Visit for FieldText<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for EventCapture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = BTreeMap::new();
        event.record(&mut FieldText(&mut fields));
        self.events.lock().push(OwnerEvent {
            name: event.metadata().name().to_owned(),
            level: *event.metadata().level(),
            fields,
        });
    }
}
