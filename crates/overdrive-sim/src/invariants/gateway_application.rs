//! Gateway Application lifecycle invariants (ADR-0222; invariants
//! `GatewayApplicationGenerationLifecycleIsSafe` and
//! `GatewayApplicationShutdownConverges`).
//!
//! Both evaluators drive the production owners only: Route commands through
//! `GatewayRouteCommands::routes`, restart through `UnboundGatewayBuilder` ->
//! `GatewayBuilder::start` over the unchanged durable stores, retain/release
//! through a real loopback connection to the bound public listener served by
//! `SimGatewayRelay`, and shutdown through `GatewayHandle::shutdown`. The only
//! fault is the Sim connect-probe hold/release (a driven-port fault carrying the
//! real `ServiceFrontend`, parked at an awaited operation the gate owns: the
//! once-per-boot connect dataplane probe for the first candidate of a boot,
//! which the owner cannot cancel, and the identity wait for every later one;
//! route activation does not depend on backend authentication, so the gate
//! never reaches `authenticate`, R-65). The
//! oracles read only the redacted `GatewayApplicationStatusRowV1`, the
//! non-secret shutdown projections and the dataplane's live intent/receipt
//! counts; there is no parallel owner model and no handcrafted pass.
//!
//! # Addresses (R-6)
//!
//! Every composition owns two `127.0.0.0/8` addresses: the public bind address
//! `127.K.x.y` and the Service frontend `127.(K+2).x.y`, where the second octet
//! `K` is the kind (`120` generation lifecycle, `121` shutdown). The slot
//! `x.y` is claimed atomically: `Rig::start` binds the Service frontend
//! listener at the slot, starting from a slot mixed from the seed and the
//! process id, and moves to the next slot on `AddrInUse`. The held listener is
//! the claim, and the public bind address is derived from the same slot, so a
//! composition that owns the frontend listener owns the pair; there is no
//! probe-bind-and-drop. Parallel processes that run the same seed therefore
//! never share an address. Neither address appears in the trace.
//!
//! # The trace
//!
//! A passing result carries the digest of the ordered observation trace in
//! `InvariantResult::cause` (`ordered-trace len=N digest=0x..`): the pinned
//! `InvariantResult` has no other field a twin-run comparison can read. A failing
//! result carries the seed, the schedule and the offending observation.

#![allow(
    clippy::large_futures,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    reason = "long owner-driving evaluators; doc comments name pinned identifiers"
)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::SpiffeId;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, ServiceV1, VmInput, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::ca::kek::KEK_LEN;
use overdrive_core::ca::root_key_envelope::KekId;
use overdrive_core::dataplane::{Proto, ServiceFrontend};
use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};
use overdrive_core::gateway_identity::{GatewayIdentityDesired, GatewayIdentityEpoch};
use overdrive_core::id::{AllocationId, ContentHash, NodeId, ServiceVip, WorkloadId};
use overdrive_core::public_ingress::{
    GatewayApplicationStatusRowV1, GatewayListenerStatus, PublicCertifiedKeyId, PublicHostname,
    PublicRouteInput, Route, RouteGeneration, RouteId, ServiceListenerReferenceInput,
};
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::traits::ca::{Ca, SvidRequest};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::{Backend, Dataplane};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_gateway::application::{
    GatewayHandle, GatewayRouteCommands, GatewayRuntimeDependencies, GatewayShutdownReport,
    GatewayStatusReads, UnboundGatewayBuilder,
};
use overdrive_gateway::ports::{
    GatewayConnectCleanup, GatewayConnectDataplane, GatewayIdentityActionTarget,
};
use overdrive_gateway::runtime::GatewayLimits;
use overdrive_gateway::{GatewayConfig, ManualCertifiedKeyConfig};
use overdrive_host::ca::PublicCertifiedKeyAeadCodec;
use overdrive_store_local::LocalIntentStore;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom as _;
use tempfile::TempDir;

use crate::adapters::ca::SimCa;
use crate::adapters::clock::SimClock;
use crate::adapters::dataplane::SimDataplane;
use crate::adapters::entropy::SimEntropy;
use crate::adapters::gateway::{
    LegScript, PublicLegScript, SimConnectHold, SimGatewayClientMtls, SimGatewayConnectDataplane,
    SimGatewayIdentityActionTarget, SimGatewayIdentityLifecycleControl, SimGatewayRelay,
};
use crate::adapters::kek::SimKek;
use crate::adapters::observation_store::SimObservationStore;
use crate::adapters::read_ports::SimServiceVipView;
use crate::harness::{InvariantResult, InvariantStatus};

const GENERATION_LIFECYCLE_NAME: &str = "gateway-application-generation-lifecycle-is-safe";
const SHUTDOWN_CONVERGES_NAME: &str = "gateway-application-shutdown-converges";
const HOST: &str = "gateway-node";
const HOSTNAME: &str = "api.example.com";
const SERVICE_PORT: u16 = 8080;
/// Loopback address slots per case kind: 250 x 250.
const SLOTS: u64 = 62_500;
const PUBLIC_PORT: u16 = 443;
const ROUTE_ID: &str = "api-route";
/// Real-time bound of every wait on an owner task.
const WAIT_STEPS: u32 = 600;
const WAIT_STEP: Duration = Duration::from_millis(5);
/// The retained connection's keep-alive idle span on the injected clock: the
/// script delivers the peer's close_notify this long after the first response.
const RETAIN_SPAN: Duration = Duration::from_secs(2);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const CLOCK_STEP: Duration = Duration::from_millis(100);

/// A throwaway self-signed leaf for `api.example.com` valid 2020..2100, minted
/// once with rcgen; it protects nothing.
const FIXTURE_CHAIN_PEM: &str = "-----BEGIN CERTIFICATE-----\n\
MIIBZDCCAQqgAwIBAgIUM2Y6Hz6d6yaNpC4bACIgCj/dXlYwCgYIKoZIzj0EAwIw\n\
ITEfMB0GA1UEAwwWcmNnZW4gc2VsZiBzaWduZWQgY2VydDAgFw0yMDAxMDEwMDAw\n\
MDBaGA8yMTAwMDEwMTAwMDAwMFowITEfMB0GA1UEAwwWcmNnZW4gc2VsZiBzaWdu\n\
ZWQgY2VydDBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABJBFy+iaVB84WRanC8Fn\n\
RG+C7qi5xuBj8eyys0YZMVdKHPZx8EGLKXaRvkDnIbjiGLdVWdboh9vB3sGgEM+0\n\
TnmjHjAcMBoGA1UdEQQTMBGCD2FwaS5leGFtcGxlLmNvbTAKBggqhkjOPQQDAgNI\n\
ADBFAiBXmZx6J0SBkDFvX7jVKfsPvpQoashfJZHt5/f2K5nx1wIhAJJwZfwqWdm8\n\
1z6qjMKo2GRLMfWBBmQDPAqYiFId9E1M\n\
-----END CERTIFICATE-----\n";
const FIXTURE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgY83Ae6Wzj7SwGAWV\n\
EszPknPMHiPHQiG3seFUvfPklcGhRANCAASQRcvomlQfOFkWpwvBZ0Rvgu6oucbg\n\
Y/HssrNGGTFXShz2cfBBiyl2kb5A5yG44hi3VVnW6Ifbwd7BoBDPtE55\n\
-----END PRIVATE KEY-----\n";

// ---------------------------------------------------------------------------
// Pinned case shapes
// ---------------------------------------------------------------------------

/// One owner-driving input of a case.
pub(crate) enum GatewayApplicationSimInput {
    ApplyRoute(Route),
    WithdrawRoute(RouteId),
    RetainCurrentPublicConnection,
    ReleaseRetainedPublicConnection,
    RestartOwner,
    BeginShutdown,
}

/// One driven-port fault of a case.
pub(crate) enum GatewayApplicationSimFault {
    /// Holds the staged gate's completion for one frontend, parked at an
    /// awaited operation the gate owns (DG-9, R-65): inside the connect
    /// dataplane probe for the first candidate of a boot (a wait the owner
    /// cannot cancel), at the identity wait
    /// (`GatewayIdentityLifecycleControl::ensure_current`) for a later one. A
    /// driven-port fault needing no backend and no `Selected` receipt.
    HoldConnectProbe(ServiceFrontend),
    ReleaseConnectProbe(ServiceFrontend),
}

/// An ordered owner-driving case.
///
/// `fault_slots[i]` is the index of the input before which `faults[i]` is
/// delivered; `inputs.len()` means after the last input. The pinned two-field
/// shape cannot express the interleaving of faults and inputs, so this
/// crate-private field carries it.
pub(crate) struct GatewayApplicationInvariantCase {
    pub(crate) inputs: Vec<GatewayApplicationSimInput>,
    pub(crate) faults: Vec<GatewayApplicationSimFault>,
    pub(crate) fault_slots: Vec<usize>,
}

impl GatewayApplicationInvariantCase {
    /// Four generations of one Route ID. The first (`late`) is held inside the
    /// connect dataplane probe, a wait the owner cannot cancel, and superseded
    /// by the second before the hold is released: its late completion is
    /// delivered to its gate, which discards it, and the second becomes
    /// Current. A connection is retained on that Current; the third
    /// generation's gate is held at the identity wait and superseded by the
    /// fourth (the owner cancels the third's wait, so nothing is delivered for
    /// it); withdrawal; release of the retained connection; one restart. The
    /// seed selects among the legal tails.
    pub(crate) fn generation_lifecycle(seed: u64, frontend: ServiceFrontend) -> Self {
        let route_id = route_id();
        let mut inputs = vec![
            GatewayApplicationSimInput::ApplyRoute(route("/v0")),
            GatewayApplicationSimInput::ApplyRoute(route("/v1")),
            GatewayApplicationSimInput::RetainCurrentPublicConnection,
            GatewayApplicationSimInput::ApplyRoute(route("/v2")),
            GatewayApplicationSimInput::ApplyRoute(route("/v3")),
        ];
        // The second hold is released after the fourth apply, before the tail.
        let release_slot = inputs.len();
        let tail = pick_tail(seed);
        match tail {
            Tail::WithdrawReleaseRestart => {
                inputs.push(GatewayApplicationSimInput::WithdrawRoute(route_id));
                inputs.push(GatewayApplicationSimInput::ReleaseRetainedPublicConnection);
                inputs.push(GatewayApplicationSimInput::RestartOwner);
            }
            Tail::ReleaseWithdrawRestart => {
                inputs.push(GatewayApplicationSimInput::ReleaseRetainedPublicConnection);
                inputs.push(GatewayApplicationSimInput::WithdrawRoute(route_id));
                inputs.push(GatewayApplicationSimInput::RestartOwner);
            }
            Tail::ReleaseRestartWithdraw => {
                inputs.push(GatewayApplicationSimInput::ReleaseRetainedPublicConnection);
                inputs.push(GatewayApplicationSimInput::RestartOwner);
                inputs.push(GatewayApplicationSimInput::WithdrawRoute(route_id));
            }
        }
        Self {
            inputs,
            faults: vec![
                GatewayApplicationSimFault::HoldConnectProbe(frontend),
                GatewayApplicationSimFault::ReleaseConnectProbe(frontend),
                GatewayApplicationSimFault::HoldConnectProbe(frontend),
                GatewayApplicationSimFault::ReleaseConnectProbe(frontend),
            ],
            // Hold before the first apply (index 0), release after the second
            // (before the retained connection); hold before the third apply
            // (index 3), release after the fourth.
            fault_slots: vec![0, 2, 3, release_slot],
        }
    }

    /// One Current retained connection and one Staged generation whose probe is
    /// held; the stale-probe release, the connection release and the shutdown
    /// begin are permuted by the seed.
    pub(crate) fn shutdown(seed: u64, frontend: ServiceFrontend) -> Self {
        let mut inputs = vec![
            GatewayApplicationSimInput::ApplyRoute(route("/v1")),
            GatewayApplicationSimInput::RetainCurrentPublicConnection,
            GatewayApplicationSimInput::ApplyRoute(route("/v2")),
        ];
        let base = inputs.len();
        // Items to permute: 0 = release probe (a fault), 1 = release connection,
        // 2 = begin shutdown.
        let mut order = [0_u8, 1, 2];
        order.shuffle(&mut StdRng::seed_from_u64(seed));
        let mut faults = vec![GatewayApplicationSimFault::HoldConnectProbe(frontend)];
        let mut fault_slots = vec![2];
        for item in order {
            match item {
                0 => {
                    faults.push(GatewayApplicationSimFault::ReleaseConnectProbe(frontend));
                    fault_slots.push(inputs.len());
                }
                1 => inputs.push(GatewayApplicationSimInput::ReleaseRetainedPublicConnection),
                _ => inputs.push(GatewayApplicationSimInput::BeginShutdown),
            }
        }
        debug_assert!(inputs.len() >= base);
        Self { inputs, faults, fault_slots }
    }
}

#[derive(Clone, Copy)]
enum Tail {
    WithdrawReleaseRestart,
    ReleaseWithdrawRestart,
    ReleaseRestartWithdraw,
}

fn pick_tail(seed: u64) -> Tail {
    use rand::Rng as _;
    match StdRng::seed_from_u64(seed).gen_range(0..3_u8) {
        0 => Tail::WithdrawReleaseRestart,
        1 => Tail::ReleaseWithdrawRestart,
        _ => Tail::ReleaseRestartWithdraw,
    }
}

// ---------------------------------------------------------------------------
// Route and Service builders
// ---------------------------------------------------------------------------

fn route_id() -> RouteId {
    RouteId::new(ROUTE_ID).unwrap_or_else(|_| unreachable!("fixed Route ID is valid"))
}

fn route(path: &str) -> Route {
    Route::from_submit(PublicRouteInput {
        id: ROUTE_ID.to_owned(),
        hostname: HOSTNAME.to_owned(),
        path: path.to_owned(),
        path_match: "segment_prefix".to_owned(),
        certified_key: "api-origin".to_owned(),
        target: ServiceListenerReferenceInput {
            service: "api".to_owned(),
            port: SERVICE_PORT,
            protocol: "tcp".to_owned(),
        },
    })
    .unwrap_or_else(|error| unreachable!("fixed Route input is valid: {error}"))
}

// ---------------------------------------------------------------------------
// The rig: one production composition over Sim ports
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Addresses {
    bind: Ipv4Addr,
    vip: Ipv4Addr,
}

impl Addresses {
    /// The address pair of `slot` for a case of `kind`.
    fn at(kind: u8, slot: u64) -> Self {
        let third = u8::try_from(slot / 250).unwrap_or(0);
        let fourth = u8::try_from(slot % 250).unwrap_or(0) + 1;
        Self {
            bind: Ipv4Addr::new(127, kind, third, fourth),
            vip: Ipv4Addr::new(127, kind + 2, third, fourth),
        }
    }
}

/// Claim a distinct address pair for one case of `kind` by binding the Service
/// frontend listener at the first free slot.
///
/// The addresses are process-private state: nothing in the trace renders them
/// (the status shows only `bound`). The search starts from a slot mixed from
/// the seed and the process id, so concurrent processes that run the same or
/// neighbouring seeds land apart without touching the seed-determined trace.
/// The bind is the claim: `AddrInUse` moves to the next slot, any other error
/// is returned.
fn claim_addresses(kind: u8, seed: u64) -> Result<(Addresses, std::net::TcpListener), String> {
    let mixed = (seed ^ (u64::from(std::process::id()) << 32)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let start = (mixed >> 16) % SLOTS;
    for step in 0..SLOTS {
        let addresses = Addresses::at(kind, (start + step) % SLOTS);
        match std::net::TcpListener::bind((addresses.vip, SERVICE_PORT)) {
            Ok(listener) => return Ok((addresses, listener)),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {}
            Err(error) => {
                return Err(format!(
                    "Service frontend listener {}:{SERVICE_PORT}: {error}",
                    addresses.vip
                ));
            }
        }
    }
    Err("no free loopback address pair for the case".to_owned())
}

/// The durable stores and Sim ports one composition is built from; they
/// outlive a restart.
struct Ports {
    chain_path: std::path::PathBuf,
    key_path: std::path::PathBuf,
    addresses: Addresses,
    clock: Arc<SimClock>,
    intent: Arc<LocalIntentStore>,
    observations: Arc<SimObservationStore>,
    vip_view: Arc<SimServiceVipView>,
    connect: Arc<SimGatewayConnectDataplane>,
    hold: SimConnectHold,
    relay: Arc<SimGatewayRelay>,
    identity: Arc<SimGatewayIdentityActionTarget>,
    backend_identity: SpiffeId,
    seed: u64,
}

struct Rig {
    _dir: TempDir,
    ports: Ports,
    _frontend_listener: std::net::TcpListener,
    routes: GatewayRouteCommands,
    status: GatewayStatusReads,
    handle: Option<GatewayHandle>,
    retained: Option<TcpStream>,
    shutdown: Option<tokio::task::JoinHandle<GatewayShutdownReport>>,
}

/// Poll `ready` every `WAIT_STEP` of real time, at most `WAIT_STEPS` times.
async fn poll_until(mut ready: impl FnMut() -> bool) -> bool {
    for _ in 0..WAIT_STEPS {
        if ready() {
            return true;
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    false
}

fn node_id() -> Result<NodeId, String> {
    NodeId::new(HOST).map_err(|error| format!("node ID: {error}"))
}

impl Rig {
    async fn start(seed: u64, kind: u8) -> Result<Self, String> {
        let (addresses, listener) = claim_addresses(kind, seed)?;
        let dir = tempfile::tempdir().map_err(|error| format!("tempdir: {error}"))?;
        let chain_path = dir.path().join("api-origin-chain.pem");
        let key_path = dir.path().join("api-origin-key.pem");
        write_private(&chain_path, FIXTURE_CHAIN_PEM, 0o644)?;
        write_private(&key_path, FIXTURE_KEY_PEM, 0o600)?;
        let clock = Arc::new(SimClock::new());
        let intent = Arc::new(
            LocalIntentStore::open(dir.path().join("intent.redb"))
                .map_err(|error| format!("intent store: {error}"))?,
        );
        let node = node_id()?;
        let observations = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
        let vip_view = persist_api_service(&intent, addresses.vip).await?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("nonblocking frontend listener: {error}"))?;
        let frontend = ServiceFrontend::new(
            ServiceVip::new(addresses.vip.into()).map_err(|error| format!("VIP: {error}"))?,
            NonZeroU16::new(SERVICE_PORT).ok_or("service port is nonzero")?,
            Proto::Tcp,
        )
        .map_err(|error| format!("frontend: {error}"))?;

        let workload = WorkloadId::new("api").map_err(|error| format!("workload ID: {error}"))?;
        let alloc = AllocationId::new("a1b2c3d4").map_err(|error| format!("alloc ID: {error}"))?;
        let backend_identity = SpiffeId::for_allocation(&workload, &alloc);
        let dataplane = Arc::new(SimDataplane::new());
        dataplane
            .update_service(
                frontend,
                vec![Backend {
                    alloc: backend_identity.clone(),
                    addr: std::net::SocketAddr::from((addresses.vip, SERVICE_PORT)),
                    weight: 1,
                    healthy: true,
                }],
            )
            .await
            .map_err(|error| format!("program the Service: {error}"))?;
        let connect = Arc::new(SimGatewayConnectDataplane::new(dataplane));
        let hold = connect.connect_hold();
        let relay = Arc::new(SimGatewayRelay::new(clock.clone()));

        let identity = fresh_identity(&clock, seed)?;
        let ports = Ports {
            chain_path,
            key_path,
            addresses,
            clock,
            intent,
            observations,
            vip_view,
            connect,
            hold,
            relay,
            identity,
            backend_identity,
            seed,
        };
        let (handle, routes, status) = compose(&ports).await?;
        Ok(Self {
            _dir: dir,
            ports,
            _frontend_listener: listener,
            routes,
            status,
            handle: Some(handle),
            retained: None,
            shutdown: None,
        })
    }

    async fn status(&self) -> Result<Option<GatewayApplicationStatusRowV1>, String> {
        self.status
            .application_status()
            .await
            .map_err(|error| format!("application status read: {error}"))
    }

    async fn wait_status(
        &self,
        what: &str,
        predicate: impl Fn(&GatewayApplicationStatusRowV1) -> bool,
    ) -> Result<GatewayApplicationStatusRowV1, String> {
        let mut last = None;
        for _ in 0..WAIT_STEPS {
            if let Some(row) = self.status().await? {
                if predicate(&row) {
                    return Ok(row);
                }
                last = Some(row);
            }
            tokio::time::sleep(WAIT_STEP).await;
        }
        Err(format!("timed out waiting for {what}; last status {}", render_row(last.as_ref())))
    }

    /// Wait until the connect path holds no intent and no receipt: every probe
    /// has completed and its cleanup has run.
    async fn wait_connect_idle(&self) -> Result<(), String> {
        if poll_until(|| self.live_counts() == (Some(0), Some(0))).await {
            return Ok(());
        }
        Err(format!("the connect path never went idle; counts {:?}", self.live_counts()))
    }

    /// Wait until exactly `gates` staged gates are parked at the identity wait
    /// and the connect path holds no intent and no receipt: a parked gate has
    /// not reached the connector probe, so it has opened nothing. Both reads
    /// are of settled state: each parked gate's task is blocked on the hold
    /// until released, so nothing further moves until the next action.
    async fn wait_gates_parked(&self, gates: u32) -> Result<(), String> {
        let hold = &self.ports.hold;
        let settled = (Some(0), Some(0));
        let parked = usize::try_from(gates).unwrap_or(usize::MAX);
        if poll_until(|| hold.held_entries() == parked && self.live_counts() == settled).await {
            return Ok(());
        }
        Err(format!(
            "the gate hold did not settle: expected {gates} parked with counts \
             {settled:?}, found {} parked with counts {:?}",
            hold.held_entries(),
            self.live_counts()
        ))
    }

    fn live_counts(&self) -> (Option<u32>, Option<u32>) {
        (
            self.ports.connect.live_intent_count().ok(),
            self.ports
                .connect
                .live_receipts()
                .ok()
                .and_then(|receipts| u32::try_from(receipts.len()).ok()),
        )
    }

    /// Open one real client connection to the public listener, scripted as a
    /// keep-alive connection that serves one response and idles until
    /// `RETAIN_SPAN` of injected time passes.
    async fn retain_connection(&mut self) -> Result<(), String> {
        let first = self.ports.relay.leg_transcripts().len();
        self.ports.relay.script_public_connection(PublicLegScript {
            sni: PublicHostname::new(HOSTNAME).map_err(|error| format!("SNI: {error}"))?,
            inbound: LegScript::new()
                .octets(b"GET /v1/retained HTTP/1.1\r\nHost: api.example.com\r\n\r\n".to_vec())
                .quiet(RETAIN_SPAN)
                .close_notify(),
        });
        self.ports.relay.script_upstream_leg(
            LegScript::new().octets(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\none".to_vec()),
        );
        let client = TcpStream::connect(SocketAddrV4::new(self.ports.addresses.bind, PUBLIC_PORT))
            .map_err(|error| format!("connect to the public listener: {error}"))?;
        self.retained = Some(client);
        let relay = &self.ports.relay;
        let served = poll_until(|| {
            relay
                .leg_transcripts()
                .iter()
                .skip(first)
                .any(|leg| leg.written.windows(12).any(|window| window == b"HTTP/1.1 200"))
        })
        .await;
        if served {
            return Ok(());
        }
        Err("the retained connection never served its first response".to_owned())
    }

    /// Release the retained connection: its idle span elapses on the injected
    /// clock, the peer's close_notify arrives and the connection closes.
    async fn release_retained(&mut self) -> Result<(), String> {
        self.ports.clock.tick(RETAIN_SPAN);
        if poll_until(|| self.ports.relay.live_leg_count() == 0).await {
            self.retained = None;
            return Ok(());
        }
        Err("the retained connection never closed after its idle span".to_owned())
    }

    fn begin_shutdown(&mut self) -> Result<(), String> {
        let handle = self.handle.take().ok_or("the gateway is not running")?;
        self.shutdown = Some(tokio::spawn(handle.shutdown(SHUTDOWN_GRACE)));
        Ok(())
    }

    async fn finish_shutdown(&mut self) -> Result<GatewayShutdownReport, String> {
        let task = self.shutdown.take().ok_or("shutdown was never begun")?;
        for _ in 0..(WAIT_STEPS * 4) {
            if task.is_finished() {
                return task.await.map_err(|error| format!("shutdown task: {error}"));
            }
            self.ports.clock.tick(CLOCK_STEP);
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        task.abort();
        Err("shutdown did not finish within the grace".to_owned())
    }

    /// Restart: stop the running owner cleanly, then compose again over the
    /// unchanged durable stores.
    async fn restart(&mut self) -> Result<(), String> {
        self.begin_shutdown()?;
        let report = self.finish_shutdown().await?;
        if !report.is_clean() {
            return Err(format!("restart's shutdown was not clean: {report:?}"));
        }
        // The gateway identity is volatile: the clean stop cleared it, and the
        // node's SVID issuance supplies a fresh one for the restarted owner.
        self.ports.identity = fresh_identity(&self.ports.clock, self.ports.seed)?;
        let (handle, routes, status) = compose(&self.ports).await?;
        self.routes = routes;
        self.status = status;
        self.handle = Some(handle);
        Ok(())
    }
}

/// A gateway identity slot holding a freshly issued, audited SVID at the first
/// epoch.
fn fresh_identity(
    clock: &Arc<SimClock>,
    seed: u64,
) -> Result<Arc<SimGatewayIdentityActionTarget>, String> {
    let node = node_id()?;
    let identity = Arc::new(SimGatewayIdentityActionTarget::new(GatewayIdentityDesired {
        epoch: GatewayIdentityEpoch::first(),
        spiffe_id: Some(SpiffeId::for_gateway(&node)),
    }));
    let ca = SimCa::new(Arc::new(SimEntropy::new(seed)));
    let now = clock.unix_now();
    let material = ca
        .issue_svid(&SvidRequest::new(
            SpiffeId::for_gateway(&node),
            UnixInstant::from_unix_duration(now),
            UnixInstant::from_unix_duration(now + Duration::from_secs(86_400)),
        ))
        .map_err(|error| format!("gateway SVID: {error}"))?;
    identity.hold_audited(GatewayIdentityEpoch::first(), material);
    Ok(identity)
}

/// Compose the production owner over the durable stores and Sim ports.
async fn compose(
    ports: &Ports,
) -> Result<(GatewayHandle, GatewayRouteCommands, GatewayStatusReads), String> {
    let config = GatewayConfig::new(
        ports.addresses.bind,
        ManualCertifiedKeyConfig::new(
            PublicCertifiedKeyId::new("api-origin")
                .map_err(|error| format!("certified-key ID: {error}"))?,
            ports.chain_path.clone(),
            ports.key_path.clone(),
        ),
    );
    let kek_id = KekId::new("overdrive-public-certified-key")
        .map_err(|error| format!("custody KEK ID: {error}"))?;
    let kek = Arc::new(SimKek::new().with(&kek_id, [0x5a; KEK_LEN]));
    let intent_port: Arc<dyn IntentStore> = ports.intent.clone();
    let observation_port: Arc<dyn ObservationStore> = ports.observations.clone();
    let clock_port: Arc<dyn Clock> = ports.clock.clone();
    let node = node_id()?;
    let builder = UnboundGatewayBuilder::new(
        node.clone(),
        config,
        intent_port,
        observation_port,
        ports.vip_view.clone(),
        Arc::new(PublicCertifiedKeyAeadCodec::new(kek)),
        clock_port.clone(),
    )
    .await
    .map_err(|error| format!("gateway builder: {error}"))?;
    let (builder, seal) = builder.into_builder();
    let client = SimGatewayClientMtls::new(seal, ports.relay.clone(), clock_port.clone());
    client.set_peer(ports.backend_identity.clone());
    let evaluation = Evaluation {
        reconciler: ReconcilerName::new("gateway-svid-lifecycle")
            .map_err(|error| format!("reconciler name: {error}"))?,
        target: TargetResource::new(&format!("node/{}", node.as_str()))
            .map_err(|error| format!("target: {error}"))?,
    };
    let lifecycle = SimGatewayIdentityLifecycleControl::new(
        ports.identity.clone(),
        Arc::new(parking_lot::Mutex::new(EvaluationBroker::new())),
        evaluation,
        clock_port,
    )
    .with_hold(ports.hold.clone());
    let connect: Arc<dyn GatewayConnectDataplane> = ports.connect.clone();
    let dependencies = GatewayRuntimeDependencies::new(
        GatewayLimits::first_slice(),
        connect,
        Arc::new(client),
        Arc::new(lifecycle),
        ports.relay.clone(),
    );
    Ok(builder.start(dependencies).await.into_parts())
}

fn write_private(path: &std::path::Path, contents: &str, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::write(path, contents).map_err(|error| format!("write {}: {error}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|error| format!("chmod {}: {error}", path.display()))
}

async fn persist_api_service(
    intent: &LocalIntentStore,
    vip: Ipv4Addr,
) -> Result<Arc<SimServiceVipView>, String> {
    let service = ServiceV1::from_submit(ServiceSpecInput {
        id: "api".to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 10, memory_bytes: 16 * 1024 * 1024 },
        driver: DriverInput::Vm(VmInput {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: SERVICE_PORT, protocol: "tcp".to_owned() }],
        startup_probes: Vec::new(),
        readiness_probes: Vec::new(),
        liveness_probes: Vec::new(),
    })
    .map_err(|error| format!("api Service input: {error}"))?;
    let workload_id = service.id.clone();
    let service_intent = WorkloadIntent::Service(service);
    let digest = service_intent.spec_digest().map_err(|error| format!("digest: {error}"))?;
    let archived =
        service_intent.archive_for_store().map_err(|error| format!("archive: {error}"))?;
    intent
        .put(IntentKey::for_workload(&workload_id).as_bytes(), archived.as_ref())
        .await
        .map_err(|error| format!("persist api Service: {error}"))?;
    intent
        .put(
            IntentKey::for_workload_kind(&workload_id).as_bytes(),
            &[WorkloadKind::Service.discriminator_byte()],
        )
        .await
        .map_err(|error| format!("persist api Service kind: {error}"))?;
    Ok(Arc::new(SimServiceVipView::new(BTreeMap::<ContentHash, ServiceVip>::from([(
        digest,
        ServiceVip::new(vip.into()).map_err(|error| format!("VIP: {error}"))?,
    )]))))
}

// ---------------------------------------------------------------------------
// Trace and rendering
// ---------------------------------------------------------------------------

fn short(generation: &dyn std::fmt::Debug) -> String {
    let rendered = format!("{generation:?}");
    rendered.chars().filter(char::is_ascii_hexdigit).take(8).collect()
}

fn render_row(row: Option<&GatewayApplicationStatusRowV1>) -> String {
    let Some(row) = row else {
        return "no-row".to_owned();
    };
    let one = |generation: &overdrive_core::public_ingress::GatewayApplicationGenerationStatus| {
        format!(
            "{}:{}",
            short(&generation.application_generation),
            short(&generation.route_generation)
        )
    };
    let listener = match row.listener {
        GatewayListenerStatus::Unbound => "unbound".to_owned(),
        GatewayListenerStatus::Bound { .. } => "bound".to_owned(),
    };
    format!(
        "listener={listener} staged={} current={} draining=[{}] unavailable={:?} connect={:?}",
        row.staged.as_ref().map_or_else(|| "-".to_owned(), one),
        row.current.as_ref().map_or_else(|| "-".to_owned(), one),
        row.draining.iter().map(one).collect::<Vec<_>>().join(","),
        row.unavailable,
        row.connect_path,
    )
}

/// FNV-1a over the rendered trace.
fn digest(lines: &[String]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in lines.iter().flat_map(|line| line.bytes().chain(std::iter::once(b'\n'))) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn verdict(name: &str, seed: u64, outcome: Result<Vec<String>, String>) -> InvariantResult {
    match outcome {
        Ok(trace) => InvariantResult {
            name: name.to_owned(),
            status: InvariantStatus::Pass,
            tick: u64::try_from(trace.len()).unwrap_or(u64::MAX),
            host: HOST.to_owned(),
            cause: Some(format!(
                "ordered-trace len={} digest={:#018x}",
                trace.len(),
                digest(&trace)
            )),
        },
        Err(cause) => InvariantResult {
            name: name.to_owned(),
            status: InvariantStatus::Fail,
            tick: 0,
            host: HOST.to_owned(),
            cause: Some(format!("seed={seed}: {cause}")),
        },
    }
}

/// What the oracle has learned about the three generations by name.
#[derive(Default)]
struct Generations {
    /// The superseded generation whose gate parks inside the connect probe
    /// (lifecycle case only).
    late: Option<RouteGeneration>,
    first: Option<RouteGeneration>,
    second: Option<RouteGeneration>,
    third: Option<RouteGeneration>,
}

struct Observer {
    trace: Vec<String>,
    generations: Generations,
    /// Shutdown is running concurrently with the case: a snapshot taken now
    /// depends on how far the shutdown task has progressed, so the trace
    /// records the label only and the universal checks still run.
    shutdown_in_progress: bool,
    /// The connect probe hold is armed for the frontend.
    probe_held: bool,
    /// Gates parked at the identity wait since the hold was armed, proven parked
    /// (quiescent live counts) before the next action. The owner cancels a
    /// superseded gate's wait, so at most the live staged gate stays parked.
    held_gates: u32,
}

impl Observer {
    const fn new() -> Self {
        Self {
            trace: Vec::new(),
            generations: Generations { late: None, first: None, second: None, third: None },
            shutdown_in_progress: false,
            probe_held: false,
            held_gates: 0,
        }
    }

    /// Record one observation and apply the universal checks.
    async fn observe(
        &mut self,
        rig: &Rig,
        label: &str,
    ) -> Result<Option<GatewayApplicationStatusRowV1>, String> {
        let row = rig.status().await?;
        let (intents, receipts) = rig.live_counts();
        let line = format!(
            "{label}: {} intents={intents:?} receipts={receipts:?}",
            render_row(row.as_ref())
        );
        if self.shutdown_in_progress {
            self.trace.push(format!("{label}: shutdown in progress"));
        } else {
            self.trace.push(line.clone());
        }
        if let Some(row) = &row {
            if let (Some(staged), Some(current)) = (&row.staged, &row.current)
                && staged.application_generation == current.application_generation
            {
                return Err(format!("Staged equals Current ({line})"));
            }
            // The second generation is superseded once the third exists; until
            // then its probe is held, so it cannot be Current either.
            let barred = self
                .generations
                .second
                .filter(|_| self.probe_held || self.generations.third.is_some());
            if let (Some(second), Some(current)) = (barred, &row.current)
                && current.route_generation == second
            {
                return Err(format!("the superseded second generation became Current ({line})"));
            }
            if let Some(late) = self.generations.late
                && row
                    .staged
                    .iter()
                    .chain(row.current.iter())
                    .chain(row.draining.iter())
                    .any(|generation| generation.route_generation == late)
                && self.generations.first.is_some()
            {
                return Err(format!(
                    "the generation superseded while its connect probe was held reappeared ({line})"
                ));
            }
        }
        Ok(row)
    }
}

// ---------------------------------------------------------------------------
// Case interpreter
// ---------------------------------------------------------------------------

struct Outcome {
    trace: Vec<String>,
    report: Option<GatewayShutdownReport>,
}

/// Deliver the faults due before input `index`.
fn due_faults<'a>(
    case: &'a GatewayApplicationInvariantCase,
    index: usize,
    delivered: &mut [bool],
) -> Vec<&'a GatewayApplicationSimFault> {
    let mut due = Vec::new();
    for (position, slot) in case.fault_slots.iter().enumerate() {
        if *slot == index && !delivered[position] {
            delivered[position] = true;
            due.push(&case.faults[position]);
        }
    }
    due
}

async fn run_case(
    seed: u64,
    kind: u8,
    build_case: fn(u64, ServiceFrontend) -> GatewayApplicationInvariantCase,
    check_lifecycle: bool,
) -> Result<Outcome, String> {
    let mut rig = Rig::start(seed, kind).await?;
    let case = build_case(seed, frontend_for(rig.ports.addresses)?);
    let mut observer = Observer::new();
    let mut delivered = vec![false; case.faults.len()];
    let mut applies = 0_u32;
    let mut withdrawn = false;
    let mut report = None;

    observer.observe(&rig, "composed").await?;
    for index in 0..=case.inputs.len() {
        for fault in due_faults(&case, index, &mut delivered) {
            match fault {
                GatewayApplicationSimFault::HoldConnectProbe(frontend) => {
                    // No gate may be in flight when the hold is armed.
                    rig.wait_gates_parked(0).await?;
                    rig.ports.hold.hold(*frontend);
                    observer.probe_held = true;
                    observer.trace.push("fault: connect probe held".to_owned());
                }
                GatewayApplicationSimFault::ReleaseConnectProbe(frontend) => {
                    // The hold provably fired: the held gate is parked before the
                    // release is delivered. Once shutdown is running it owns the held
                    // gate's teardown (it cancels the wait), so the proof is the one
                    // recorded when the gate parked.
                    if !observer.shutdown_in_progress {
                        rig.wait_gates_parked(observer.held_gates).await?;
                    }
                    rig.ports.hold.release(*frontend);
                    observer.trace.push("fault: connect probe released".to_owned());
                    rig.wait_connect_idle().await?;
                    observer.probe_held = false;
                    observer.held_gates = 0;
                    if check_lifecycle && observer.generations.third.is_none() {
                        // The first generation's gate was superseded while parked
                        // inside the connect probe, which the owner cannot cancel:
                        // the release delivers its late completion to the gate
                        // itself (both parked gates return), which discards it. The
                        // replacement becomes Current and the stale generation never
                        // appears.
                        rig.wait_gates_parked(0).await?;
                        let first = observer.generations.first;
                        let row = rig
                            .wait_status("the replacement to become Current", |row| {
                                row.current.as_ref().map(|c| c.route_generation) == first
                                    && row.staged.is_none()
                            })
                            .await?;
                        let late = observer.generations.late;
                        if row.draining.iter().any(|d| Some(d.route_generation) == late) {
                            return Err(format!(
                                "the stale completion left its generation Draining ({})",
                                render_row(Some(&row))
                            ));
                        }
                        observer.observe(&rig, "stale connect probe completion released").await?;
                    } else if check_lifecycle {
                        // The successor's gate cancelled the held identity wait of the
                        // generation it superseded, so no completion is delivered for
                        // it; the successor must become Current and the superseded
                        // generation never does.
                        let third = observer.generations.third;
                        let row = rig
                            .wait_status("the successor to become Current", |row| {
                                row.current.as_ref().map(|c| c.route_generation) == third
                                    && row.staged.is_none()
                            })
                            .await?;
                        let first = observer.generations.first;
                        let retained_open = rig.retained.is_some();
                        if retained_open
                            && !row.draining.iter().any(|d| Some(d.route_generation) == first)
                        {
                            return Err(format!(
                                "the displaced first generation is not Draining while its \
                                 retained connection is open ({})",
                                render_row(Some(&row))
                            ));
                        }
                        observer.observe(&rig, "identity hold released").await?;
                    }
                }
            }
        }
        let Some(input) = case.inputs.get(index) else {
            break;
        };
        match input {
            GatewayApplicationSimInput::ApplyRoute(route) => {
                applies += 1;
                rig.routes
                    .routes()
                    .apply(route.clone())
                    .await
                    .map_err(|error| format!("apply Route {applies}: {error}"))?;
                // A hold armed before the first apply parks that gate inside the
                // connect probe: the generation is Staged, never Current, until
                // the hold is released.
                let first_gate_held = applies == 1 && observer.probe_held;
                let ordinal = applies - u32::from(observer.generations.late.is_some());
                if first_gate_held {
                    let row = rig
                        .wait_status("the held first generation to be Staged", |row| {
                            row.staged.is_some() && row.current.is_none()
                        })
                        .await?;
                    observer.generations.late = row.staged.as_ref().map(|s| s.route_generation);
                } else {
                    match ordinal {
                        1 if observer.probe_held => {
                            let late = observer.generations.late;
                            let row = rig
                                .wait_status(
                                    "the replacement to supersede the held Staged",
                                    |row| {
                                        row.staged.is_some()
                                            && row.staged.as_ref().map(|s| s.route_generation)
                                                != late
                                    },
                                )
                                .await?;
                            observer.generations.first =
                                row.staged.as_ref().map(|s| s.route_generation);
                            if row.current.is_some() {
                                return Err(format!(
                                    "Current appeared while every gate was held ({})",
                                    render_row(Some(&row))
                                ));
                            }
                        }
                        1 => {
                            let row = rig
                                .wait_status("the first generation to become Current", |row| {
                                    row.current.is_some() && row.unavailable.is_none()
                                })
                                .await?;
                            observer.generations.first =
                                row.current.as_ref().map(|c| c.route_generation);
                        }
                        2 => {
                            let first = observer.generations.first;
                            let row = rig
                                .wait_status("the second generation to be Staged", |row| {
                                    row.staged.as_ref().map(|s| s.route_generation) != first
                                        && row.staged.is_some()
                                })
                                .await?;
                            observer.generations.second =
                                row.staged.as_ref().map(|s| s.route_generation);
                            if row.current.as_ref().map(|c| c.route_generation) != first {
                                return Err(format!(
                                    "Current changed before the second generation was promoted ({})",
                                    render_row(Some(&row))
                                ));
                            }
                        }
                        _ => {
                            let second = observer.generations.second;
                            let row = rig
                                .wait_status("the third generation to be Staged", |row| {
                                    row.staged.is_some()
                                        && row.staged.as_ref().map(|s| s.route_generation) != second
                                })
                                .await?;
                            observer.generations.third =
                                row.staged.as_ref().map(|s| s.route_generation);
                        }
                    }
                }
                if observer.probe_held {
                    // The gate of a generation held at the identity wait parks there
                    // (the superseded generation's wait is cancelled, so exactly one
                    // gate is parked). A gate held inside the connect probe is not
                    // cancelled: the superseded first generation's gate stays parked
                    // beside its replacement's, so two are. The observation below is
                    // taken only once they have parked.
                    observer.held_gates =
                        if applies == 2 && observer.generations.late.is_some() { 2 } else { 1 };
                    rig.wait_gates_parked(observer.held_gates).await?;
                }
                observer.observe(&rig, &format!("route {applies} applied")).await?;
            }
            GatewayApplicationSimInput::RetainCurrentPublicConnection => {
                rig.retain_connection().await?;
                observer.observe(&rig, "connection retained").await?;
            }
            GatewayApplicationSimInput::WithdrawRoute(id) => {
                rig.routes
                    .routes()
                    .withdraw(id.clone())
                    .await
                    .map_err(|error| format!("withdraw Route: {error}"))?;
                let row = rig
                    .wait_status("withdrawal to clear Staged and Current", |row| {
                        row.staged.is_none() && row.current.is_none()
                    })
                    .await?;
                withdrawn = true;
                if check_lifecycle && rig.retained.is_some() && row.draining.is_empty() {
                    return Err(format!(
                        "a generation retired before its retained connection closed ({})",
                        render_row(Some(&row))
                    ));
                }
                observer.observe(&rig, "route withdrawn").await?;
            }
            GatewayApplicationSimInput::ReleaseRetainedPublicConnection => {
                rig.release_retained().await?;
                if check_lifecycle {
                    let route_present = !withdrawn;
                    let third = observer.generations.third;
                    rig.wait_status("the displaced generation to retire", |row| {
                        row.draining.is_empty()
                            && (!route_present
                                || row.current.as_ref().map(|c| c.route_generation) == third)
                    })
                    .await?;
                }
                observer.observe(&rig, "retained connection released").await?;
            }
            GatewayApplicationSimInput::RestartOwner => {
                rig.restart().await?;
                if withdrawn {
                    let row = rig
                        .wait_status("the restarted owner to find no Route", |row| {
                            row.staged.is_none() && row.current.is_none() && row.draining.is_empty()
                        })
                        .await?;
                    observer.observe(&rig, "restarted without a Route").await?;
                    drop(row);
                } else {
                    let third = observer.generations.third;
                    let row = rig
                        .wait_status("the restart to relist the durable Route", |row| {
                            row.current.as_ref().map(|c| c.route_generation) == third
                        })
                        .await?;
                    observer.observe(&rig, "restarted with the durable Route").await?;
                    drop(row);
                }
                // Restart relists durable intent: a withdrawn or superseded
                // generation is never resurrected.
                if let Some(row) = rig.status().await? {
                    let late = observer.generations.late;
                    let first = observer.generations.first;
                    let second = observer.generations.second;
                    let present = [row.staged.as_ref(), row.current.as_ref()]
                        .into_iter()
                        .flatten()
                        .map(|g| Some(g.route_generation));
                    for generation in present {
                        if generation == late || generation == first || generation == second {
                            return Err(format!(
                                "restart resurrected a superseded generation ({})",
                                render_row(Some(&row))
                            ));
                        }
                    }
                }
            }
            GatewayApplicationSimInput::BeginShutdown => {
                rig.begin_shutdown()?;
                observer.shutdown_in_progress = true;
                // Shutdown has stopped accepts and closed the application (its
                // first externally visible step) and then waits on the injected
                // clock for the drain, which only this case advances.
                rig.wait_status("shutdown to unbind and clear Staged and Current", |row| {
                    matches!(row.listener, GatewayListenerStatus::Unbound)
                        && row.staged.is_none()
                        && row.current.is_none()
                })
                .await?;
                observer.trace.push("shutdown begun".to_owned());
            }
        }
    }
    if rig.shutdown.is_some() {
        let finished = rig.finish_shutdown().await?;
        observer.shutdown_in_progress = false;
        let row = observer.observe(&rig, "after shutdown").await?;
        observer.trace.push(format!(
            "report: clean={} forced={} identity_empty={} residual={:?}",
            finished.is_clean(),
            finished.forced_connections,
            finished.identity_empty,
            finished.residual_connect
        ));
        if let Some(row) = row
            && (row.staged.is_some() || row.current.is_some())
        {
            return Err(format!(
                "a Staged or Current application remained after shutdown ({})",
                render_row(Some(&row))
            ));
        }
        report = Some(finished);
    } else if let Some(handle) = rig.handle.take() {
        // A lifecycle case ends with a clean stop so no task or lease outlives it.
        rig.shutdown = Some(tokio::spawn(handle.shutdown(SHUTDOWN_GRACE)));
        let finished = rig.finish_shutdown().await?;
        if !finished.is_clean() {
            return Err(format!("the closing shutdown was not clean: {finished:?}"));
        }
    }
    // Drain the retained socket's peer side so the client end is closed.
    if let Some(mut client) = rig.retained.take() {
        let _ = client.set_nonblocking(true);
        let mut sink = [0_u8; 16];
        let _ = client.read(&mut sink);
    }
    Ok(Outcome { trace: observer.trace, report })
}

// ---------------------------------------------------------------------------
// Evaluators
// ---------------------------------------------------------------------------

fn frontend_for(addresses: Addresses) -> Result<ServiceFrontend, String> {
    ServiceFrontend::new(
        ServiceVip::new(addresses.vip.into()).map_err(|error| format!("VIP: {error}"))?,
        NonZeroU16::new(SERVICE_PORT).ok_or("service port is nonzero")?,
        Proto::Tcp,
    )
    .map_err(|error| format!("frontend: {error}"))
}

/// Generation lifecycle safety while the owner is live: staged precedes
/// promotion, the displaced Current drains while its retained connection is
/// open, a stale completion never promotes or retires the successor, withdrawal
/// converges, and restart relists durable intent without resurrecting a
/// withdrawn or superseded generation.
pub(crate) async fn evaluate_generation_lifecycle_is_safe(seed: u64) -> InvariantResult {
    let outcome = async {
        run_case(seed, 120, GatewayApplicationInvariantCase::generation_lifecycle, true).await
    }
    .await;
    verdict(GENERATION_LIFECYCLE_NAME, seed, outcome.map(|outcome| outcome.trace))
}

/// Shutdown convergence after ownership is revoked: the listener unbinds, no
/// Staged or Current application remains, retained leases are joined, the
/// identity slot is empty, the ledger is empty and the dataplane holds no
/// intent or receipt.
pub(crate) async fn evaluate_shutdown_converges(seed: u64) -> InvariantResult {
    let outcome = async {
        let outcome = run_case(seed, 121, GatewayApplicationInvariantCase::shutdown, false).await?;
        let report = outcome.report.as_ref().ok_or("the case never began shutdown")?;
        if !report.is_clean() {
            return Err(format!("the shutdown report is not clean: {report:?}"));
        }
        if !report.identity_empty {
            return Err("the gateway identity slot was not empty after shutdown".to_owned());
        }
        // Production reports `None` exactly when the ledger is empty and the
        // dataplane holds no intent or receipt.
        if let Some(residual) = &report.residual_connect {
            return Err(format!("residual connect state after shutdown: {residual:?}"));
        }
        Ok(outcome.trace)
    }
    .await;
    verdict(SHUTDOWN_CONVERGES_NAME, seed, outcome)
}
