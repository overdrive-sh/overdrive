//! S-ND295-29B — the composed server recovers every shared-network component
//! through its required ports (GH #295, correctness-recovery proof §3.3,
//! correctness gap 2).
//!
//! Moved from `overdrive-sim/tests/shared_network_supervisor_recovery_proof.rs`:
//! the contract's owner is the control-plane supervisor, and the proof boots
//! the production composition `run_server_with_obs_and_driver`, so it is the
//! in-process lane of the owning crate (test-scenarios.md § *Recovery proof
//! tests — landing decisions*). After D-295-R16 every boot runs the real
//! `HostMtlsEnforcement` kTLS probe, so the lane is Lima root with
//! `integration-tests`; the whole control-plane integration binary is
//! `host-kernel-shared`.
//!
//! # Driving port and injected ports
//!
//! `run_server_with_obs_and_driver(ServerConfig::new(kek, mtls_intercept,
//! guest_dns), obs, driver, vm_host_state, shared_guest_network,
//! guest_network_exec, vm_cgroups)` (feature-delta FD 9533-9559) with a
//! `SimDriver`, a test-local owner delegating to `SimSharedGuestNetworkOwner`,
//! `SimGuestDnsFactory` as the required DNS port, a test-local stateful
//! intercept delegating `bind_transparent` to an inner `SimMtlsIntercept` as the
//! required intercept port, and `vm_cgroups = CgroupManager::new(<root>,
//! Arc::new(fs.clone()))` over one `SimCgroupFs` (FD 4074-4107). No test writes
//! a real `cgroup.kill`.
//!
//! # Faults — only through the driven ports
//!
//! | Component | Stimulus | Repair blocker lifted by `unblock` |
//! |---|---|---|
//! | `Bridge`, `TcxLink`, `EndpointMap`, `CounterMap`, `BpffsPin`, `BridgeGuard` | the sim owner's standing audit slot for that component | the slot disarmed |
//! | `IpRules` | the intercept reports its owned table absent | `converge_shared` refusal lifted |
//! | `IpSets` | the intercept reports an unexpected managed member | `converge_allocation_elements` refusal lifted |
//! | `LegF`, `LegC` | `SimAcceptScript::ListenerLost { errno: EINVAL }` at the leg address the worker converged | the exact-port rebind refusal lifted |
//! | `Dns` | the live `SimGuestDns` serve ends (`Return`) | the factory's standing probe refusal lifted |
//!
//! Plus the owner's scripted quiescence outcome and damage set (C6), and two
//! test-local owner-adapter faults: an audit held in flight (C7b) and an audit
//! that panics (C8). Every recovery transition, quiescence, kill, request, and
//! reopen is authored by production; no gate transition, row, kill, or request
//! is fabricated.
//!
//! # Observation (test-scenarios.md § *In-process observation*)
//!
//! The test keeps `wiring.gate()` and `wiring.supervisor()` before moving the
//! wiring into the boot, advances the injected `SimClock` in steps, and after
//! each step polls `gate.claim_release()` and `handle.shutdown_requested()`
//! exactly once with `now_or_never`, dropping each future; `recovery_progress()`
//! is read directly. Operational events are captured append-only.
//!
//! # Oracle
//!
//! ADR-0124's accepted cadence contract — the 1 s audit period, the 250 ms
//! attempt period, the 5 s recovery deadline, 20 attempts — written as that
//! contract and measured on the injected clock. The private `SHARED_NETWORK_*`
//! constants are unreachable from `tests/` and are never named here (FD
//! 4109-4145). A hung-audit detection is bounded only by the 5 s horizon.
//!
//! Every cell prints its seed, cell, and verdict append-only; a body fails when
//! any cell is RED or UNREACHED (a clause whose precondition production never
//! produced). Reproduce: `OVERDRIVE_SUPERVISOR_PROOF_SEEDS=<seed>[,<seed>...]`.

#![allow(
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::large_futures,
    clippy::cast_possible_truncation,
    clippy::unchecked_time_subtraction,
    clippy::significant_drop_tightening,
    clippy::struct_excessive_bools,
    reason = "proof test: seed-bearing diagnostics, exact CONTRACT_SHAPE lines, and long narrative \
              episodes; every Duration subtraction is `later_snapshot - earlier_snapshot` of the \
              monotonically advancing simulated elapsed counter; the intercept model's flags are \
              independent injected faults, not a state machine"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine;
use futures::FutureExt;
use overdrive_control_plane::api::{SubmitWorkloadRequest, SubmitWorkloadResponse};
use overdrive_control_plane::dns_responder::GuestDnsFactory;
use overdrive_control_plane::guest_network::{
    GuestNetworkOperation, GuestNetworkPlan, GuestNetworkProvisioner, Result as GuestNetworkResult,
    SharedGuestNetworkAudit, SharedGuestNetworkAuditError, SharedGuestNetworkOwner, TapActivation,
    TapQuiescence,
};
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server_with_obs_and_driver};
use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput, VmInput};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::guest_network::{
    GuestNetworkExecGate, GuestNetworkExecSupervisor, GuestNetworkExecWiring, ServeShutdownRequest,
    SharedGuestNetworkComponent, SharedGuestNetworkFailStop, SharedGuestNetworkFailStopCause,
    SharedGuestNetworkRecovery,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::{SimQuiesceOutcome, SimSharedGuestNetworkOwner};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_sim::adapters::{
    SimAcceptScript, SimCgroupFs, SimEntry, SimGuestDnsFactory, SimGuestDnsServeExit, SimKek,
    SimMtlsIntercept,
};
use overdrive_worker::cgroup_manager::CgroupManager;
use overdrive_worker::mtls_intercept::{
    InterceptElementKey, InterceptElementOperation, InterceptError, InterceptPostcondition,
    InterceptSet, NetlinkError, Result as InterceptResult,
};
use overdrive_worker::mtls_intercept_port::{
    InterceptGuard, InterceptMembers, InterceptState, MtlsIntercept,
};
use parking_lot::Mutex;
use tokio::sync::Semaphore;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

// ---------------------------------------------------------------------------
// ADR-0124's accepted cadence contract. These are the contract values, not the
// private `SHARED_NETWORK_*` constants of the crate root (FD 4109-4145).
// ---------------------------------------------------------------------------

/// ADR-0124: the one-second full-audit period.
const AUDIT_PERIOD: Duration = Duration::from_secs(1);
/// ADR-0124: one recovery attempt every 250 ms.
const ATTEMPT_PERIOD: Duration = Duration::from_millis(250);
/// ADR-0124: the bounded recovery window before the typed fail-stop request.
const RECOVERY_DEADLINE: Duration = Duration::from_secs(5);
/// ADR-0124: completed attempts at the recovery deadline.
const DEADLINE_ATTEMPTS: u32 = 20;
/// Injected-clock step between observations.
const STEP: Duration = Duration::from_millis(10);
/// The simulated cgroupfs root handed to the kill capability's manager.
const CGROUP_ROOT: &str = "/sys/fs/cgroup";
/// A guest-shaped address no allocation holds: the unexpected intercept member.
const FOREIGN_MEMBER: Ipv4Addr = Ipv4Addr::new(100, 95, 255, 254);

/// The eleven node-level components in D8's fixed order (FD 4190-4194). The
/// `Supervisor` component is C8's, reached only through task loss.
const COMPONENTS: [SharedGuestNetworkComponent; 11] = [
    SharedGuestNetworkComponent::Bridge,
    SharedGuestNetworkComponent::TcxLink,
    SharedGuestNetworkComponent::EndpointMap,
    SharedGuestNetworkComponent::CounterMap,
    SharedGuestNetworkComponent::BpffsPin,
    SharedGuestNetworkComponent::BridgeGuard,
    SharedGuestNetworkComponent::IpRules,
    SharedGuestNetworkComponent::IpSets,
    SharedGuestNetworkComponent::LegF,
    SharedGuestNetworkComponent::LegC,
    SharedGuestNetworkComponent::Dns,
];

/// Which production owner repairs a component (the FD 3961-3974 matrix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepairOwner {
    SharedGuestNetworkOwner,
    MtlsWorker,
    DnsOwner,
}

const fn repair_owner(component: SharedGuestNetworkComponent) -> RepairOwner {
    match component {
        SharedGuestNetworkComponent::IpRules
        | SharedGuestNetworkComponent::IpSets
        | SharedGuestNetworkComponent::LegF
        | SharedGuestNetworkComponent::LegC => RepairOwner::MtlsWorker,
        SharedGuestNetworkComponent::Dns => RepairOwner::DnsOwner,
        _ => RepairOwner::SharedGuestNetworkOwner,
    }
}

/// Kernel-path components quiesce managed TAPs; a pure listener or DNS loss
/// never does (FD 4238-4249).
const fn is_kernel_path(component: SharedGuestNetworkComponent) -> bool {
    !matches!(
        component,
        SharedGuestNetworkComponent::LegF
            | SharedGuestNetworkComponent::LegC
            | SharedGuestNetworkComponent::Dns
            | SharedGuestNetworkComponent::Supervisor
    )
}

fn kernel_path_components() -> Vec<SharedGuestNetworkComponent> {
    COMPONENTS.into_iter().filter(|component| is_kernel_path(*component)).collect()
}

/// The detection cause a stimulus produces (FD 4223-4226).
const fn expected_cause(component: SharedGuestNetworkComponent) -> &'static str {
    match component {
        SharedGuestNetworkComponent::LegF
        | SharedGuestNetworkComponent::LegC
        | SharedGuestNetworkComponent::Dns => "task_exit",
        _ => "audit_mismatch",
    }
}

/// Compare a component label as the event renders it (`Debug` or a snake /
/// kebab label) with the component.
fn same_label(rendered: &str, component: SharedGuestNetworkComponent) -> bool {
    let normalize = |text: &str| {
        text.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase()
    };
    normalize(rendered) == normalize(&format!("{component:?}"))
}

// ---------------------------------------------------------------------------
// Seeds.
// ---------------------------------------------------------------------------

const DEFAULT_SEEDS: [u64; 2] = [0x2953_3000_0000_0001, 0x2953_3000_5eed_0002];

fn seeds() -> Vec<u64> {
    std::env::var("OVERDRIVE_SUPERVISOR_PROOF_SEEDS").map_or_else(
        |_| DEFAULT_SEEDS.to_vec(),
        |raw| {
            raw.split(',')
                .map(|part| {
                    let part = part.trim();
                    part.strip_prefix("0x")
                        .map_or_else(|| part.parse::<u64>(), |hex| u64::from_str_radix(hex, 16))
                        .unwrap_or_else(|error| panic!("invalid proof seed {part:?}: {error}"))
                })
                .collect()
        },
    )
}

/// SplitMix64 over `seed ^ cell`: a deterministic per-cell stream, so one seed
/// reproduces every cell bit-for-bit.
struct CellRng(u64);

impl CellRng {
    const fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `low..=high`.
    const fn inclusive(&mut self, low: u32, high: u32) -> u32 {
        low + (self.next_u64() % (high - low + 1) as u64) as u32
    }

    const fn coin(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

const fn cell_rng(seed: u64, cell: usize) -> CellRng {
    CellRng(seed ^ (cell as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// A seeded offset, a whole number of steps inside one audit period.
fn seeded_phase(rng: &mut CellRng) -> Duration {
    STEP * rng.inclusive(0, 99)
}

// ---------------------------------------------------------------------------
// Append-only operational-event capture (guest_network.shared_owner_*).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct OwnerEvent {
    name: String,
    fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldVisitor(BTreeMap<String, String>);

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
}

struct OwnerEventCapture {
    store: Arc<Mutex<Vec<OwnerEvent>>>,
}

impl<S> Layer<S> for OwnerEventCapture
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let name = event.metadata().name();
        if !name.starts_with("guest_network.shared_owner") {
            return;
        }
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.store.lock().push(OwnerEvent { name: name.to_owned(), fields: visitor.0 });
    }
}

fn event_store() -> &'static Arc<Mutex<Vec<OwnerEvent>>> {
    static STORE: OnceLock<Arc<Mutex<Vec<OwnerEvent>>>> = OnceLock::new();
    STORE.get_or_init(|| {
        let store = Arc::new(Mutex::new(Vec::new()));
        let capture =
            OwnerEventCapture { store: Arc::clone(&store) }.with_filter(LevelFilter::INFO);
        tracing::subscriber::set_global_default(Registry::default().with(capture))
            .expect("proof process installs exactly one global capture subscriber");
        store
    })
}

fn events_since(cursor: usize) -> Vec<OwnerEvent> {
    event_store().lock()[cursor..].to_vec()
}

fn events_cursor() -> usize {
    event_store().lock().len()
}

fn named<'a>(events: &'a [OwnerEvent], name: &str) -> Vec<&'a OwnerEvent> {
    events.iter().filter(|event| event.name == name).collect()
}

/// `guest_network.shared_owner_vm_killed` events naming `alloc`.
fn vm_killed_for<'a>(events: &'a [OwnerEvent], alloc: &AllocationId) -> Vec<&'a OwnerEvent> {
    named(events, "guest_network.shared_owner_vm_killed")
        .into_iter()
        .filter(|event| event.fields.get("alloc").is_some_and(|a| a.contains(alloc.as_str())))
        .collect()
}

// ---------------------------------------------------------------------------
// Test-local owner-port double: the accepted Sim owner plus two adapter faults.
// ---------------------------------------------------------------------------

/// The one owner instance, delegating every port call to the accepted
/// `SimSharedGuestNetworkOwner`. Two driven-port faults are layered on top,
/// neither of which authors a supervisor consequence:
///
/// - when the latch is armed, `audit_shared` awaits one permit before
///   delegating, modelling one owner read-back still in flight;
/// - when the panic slot is armed, `audit_shared` panics, modelling an owner
///   adapter defect that unwinds the task polling it.
#[derive(Default)]
struct ProofOwner {
    sim: SimSharedGuestNetworkOwner,
    audit_latch: Mutex<Option<Arc<Semaphore>>>,
    audits_in_flight: AtomicUsize,
    audit_panic: AtomicBool,
    audit_panics: AtomicUsize,
}

impl ProofOwner {
    fn arm_audit_latch(&self) -> Arc<Semaphore> {
        let latch = Arc::new(Semaphore::new(0));
        *self.audit_latch.lock() = Some(Arc::clone(&latch));
        latch
    }

    fn disarm_audit_latch(&self) {
        *self.audit_latch.lock() = None;
    }

    fn audits_in_flight(&self) -> usize {
        self.audits_in_flight.load(Ordering::SeqCst)
    }

    fn arm_audit_panic(&self) {
        self.audit_panic.store(true, Ordering::SeqCst);
    }

    fn audit_panics(&self) -> usize {
        self.audit_panics.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl GuestNetworkProvisioner for ProofOwner {
    async fn provision(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.sim.provision(plan).await
    }
    async fn activate(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<TapActivation> {
        self.sim.activate(plan).await
    }
    async fn teardown(&self, plan: &GuestNetworkPlan) -> GuestNetworkResult<()> {
        self.sim.teardown(plan).await
    }
}

#[async_trait::async_trait]
impl SharedGuestNetworkOwner for ProofOwner {
    async fn probe_startup(&self) -> GuestNetworkResult<()> {
        self.sim.probe_startup().await
    }
    async fn sweep_stale(&self) -> GuestNetworkResult<()> {
        self.sim.sweep_stale().await
    }
    async fn converge_shared(&self) -> GuestNetworkResult<()> {
        self.sim.converge_shared().await
    }
    #[allow(clippy::panic, reason = "a scripted owner-adapter panic is this double's fault")]
    async fn audit_shared(
        &self,
    ) -> std::result::Result<SharedGuestNetworkAudit, SharedGuestNetworkAuditError> {
        if self.audit_panic.swap(false, Ordering::SeqCst) {
            self.audit_panics.fetch_add(1, Ordering::SeqCst);
            panic!("proof owner-port fault: shared-owner audit adapter panicked");
        }
        let latch = self.audit_latch.lock().clone();
        if let Some(latch) = latch {
            self.audits_in_flight.fetch_add(1, Ordering::SeqCst);
            latch.acquire().await.expect("proof audit latch is never closed").forget();
            self.audits_in_flight.fetch_sub(1, Ordering::SeqCst);
        }
        self.sim.audit_shared().await
    }
    async fn quiesce_managed_taps(&self) -> GuestNetworkResult<TapQuiescence> {
        self.sim.quiesce_managed_taps().await
    }
    async fn restore_quiesced_taps(&self) -> GuestNetworkResult<()> {
        self.sim.restore_quiesced_taps().await
    }
}

// ---------------------------------------------------------------------------
// Test-local stateful intercept (the required `ServerConfig.mtls_intercept`).
// ---------------------------------------------------------------------------

/// One recorded call into the intercept port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InterceptCall {
    Bind(SocketAddrV4),
    ConvergeShared,
    ObserveShared,
    ObserveState,
    ConvergeMembers,
    RemoveMembers,
    InstallOutbound,
    InstallInbound,
}

impl InterceptCall {
    /// A call that repairs intercept state (as opposed to observing it).
    const fn is_repair(self) -> bool {
        matches!(self, Self::Bind(_) | Self::ConvergeShared | Self::ConvergeMembers)
    }
}

/// The injected intercept state: losses and repair refusals layered over the
/// inner `SimMtlsIntercept`, which owns binding and the member model.
#[derive(Debug, Default)]
struct InterceptModel {
    /// The owned table reads back absent.
    program_lost: bool,
    /// `converge_shared` refuses.
    converge_blocked: bool,
    /// An unexpected managed member reads back.
    foreign_member: Option<Ipv4Addr>,
    /// `converge_allocation_elements` refuses.
    members_blocked: bool,
    /// `bind_transparent` refuses with `EADDRINUSE`.
    binds_blocked: bool,
    /// The `(leg_f, leg_c)` targets of the last successful `converge_shared`.
    legs: Option<(SocketAddrV4, SocketAddrV4)>,
    /// Every call, in call order.
    calls: Vec<InterceptCall>,
}

/// Test-local `MtlsIntercept` delegating `bind_transparent` (and every effect
/// it does not fault) to an inner `SimMtlsIntercept`.
struct ProofIntercept {
    sim: SimMtlsIntercept,
    model: Mutex<InterceptModel>,
}

impl ProofIntercept {
    fn new() -> Self {
        Self { sim: SimMtlsIntercept::new(), model: Mutex::new(InterceptModel::default()) }
    }

    fn record(&self, call: InterceptCall) {
        self.model.lock().calls.push(call);
    }

    fn calls_len(&self) -> usize {
        self.model.lock().calls.len()
    }

    fn calls_since(&self, cursor: usize) -> Vec<InterceptCall> {
        self.model.lock().calls[cursor..].to_vec()
    }

    fn leg(&self, component: SharedGuestNetworkComponent) -> Option<SocketAddrV4> {
        let legs = self.model.lock().legs?;
        match component {
            SharedGuestNetworkComponent::LegF => Some(legs.0),
            SharedGuestNetworkComponent::LegC => Some(legs.1),
            _ => None,
        }
    }

    fn program_present(&self) -> bool {
        !self.model.lock().program_lost
    }

    fn foreign_member_present(&self) -> bool {
        self.model.lock().foreign_member.is_some()
    }

    fn update(&self, change: impl FnOnce(&mut InterceptModel)) {
        change(&mut self.model.lock());
    }

    fn refusal(op: &'static str) -> InterceptError {
        InterceptError::NftRuleInstallFailed {
            op,
            source: NetlinkError::nft(op, std::io::Error::from_raw_os_error(libc::EBUSY)),
        }
    }
}

impl MtlsIntercept for ProofIntercept {
    fn bind_transparent(&self, addr: SocketAddrV4) -> InterceptResult<std::net::TcpListener> {
        self.record(InterceptCall::Bind(addr));
        if self.model.lock().binds_blocked {
            return Err(InterceptError::TransparentListener {
                addr,
                source: std::io::Error::from_raw_os_error(libc::EADDRINUSE),
            });
        }
        self.sim.bind_transparent(addr)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.record(InterceptCall::ConvergeShared);
        if self.model.lock().converge_blocked {
            return Err(Self::refusal("shared-replace"));
        }
        let guard = self.sim.converge_shared(prior, leg_f, leg_c)?;
        self.update(|model| {
            model.legs = Some((leg_f, leg_c));
            model.program_lost = false;
        });
        Ok(guard)
    }

    fn observe_shared(&self) -> InterceptResult<Option<InterceptPostcondition>> {
        self.record(InterceptCall::ObserveShared);
        if self.model.lock().program_lost {
            return Ok(None);
        }
        self.sim.observe_shared()
    }

    fn install_outbound(
        &self,
        source_addr: Ipv4Addr,
        agent_leg_f_port: u16,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.record(InterceptCall::InstallOutbound);
        self.sim.install_outbound(source_addr, agent_leg_f_port)
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        agent_leg_c_port: u16,
    ) -> InterceptResult<Box<dyn InterceptGuard>> {
        self.record(InterceptCall::InstallInbound);
        self.sim.install_inbound(virt, agent_leg_c_port)
    }

    fn observe_shared_state(&self) -> InterceptResult<Option<InterceptState>> {
        self.record(InterceptCall::ObserveState);
        let (program_lost, foreign) = {
            let model = self.model.lock();
            (model.program_lost, model.foreign_member)
        };
        if program_lost {
            return Ok(None);
        }
        let mut state = self.sim.observe_shared_state()?;
        if let (Some(state), Some(foreign)) = (state.as_mut(), foreign) {
            state.members.managed_guest_ips.insert(foreign);
        }
        Ok(state)
    }

    fn converge_allocation_elements(
        &self,
        expected: &InterceptMembers,
    ) -> InterceptResult<Option<InterceptState>> {
        self.record(InterceptCall::ConvergeMembers);
        let (program_lost, blocked, foreign) = {
            let model = self.model.lock();
            (model.program_lost, model.members_blocked, model.foreign_member)
        };
        if program_lost {
            return Ok(None);
        }
        if blocked {
            return Err(InterceptError::NftElementUpdateFailed {
                set: InterceptSet::ManagedGuestIps,
                operation: InterceptElementOperation::Delete,
                key: InterceptElementKey::Address(foreign.unwrap_or(FOREIGN_MEMBER)),
                source: NetlinkError::nft(
                    "shared-element-converge",
                    std::io::Error::from_raw_os_error(libc::EBUSY),
                ),
            });
        }
        self.update(|model| model.foreign_member = None);
        self.sim.converge_allocation_elements(expected)
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> InterceptResult<InterceptState> {
        self.record(InterceptCall::RemoveMembers);
        self.sim.remove_allocation_elements(source_addr, destinations)
    }
}

// ---------------------------------------------------------------------------
// One production node composed through the real composition root.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    Open,
    Closed,
    FailStopped,
}

type CgroupSnapshot = BTreeMap<PathBuf, (SimEntry, Vec<u8>)>;

struct Node {
    seed: u64,
    handle: Option<ServerHandle>,
    request: Option<(Duration, ServeShutdownRequest)>,
    /// The `SimCgroupFs` snapshot taken when the request was first received.
    request_snapshot: Option<CgroupSnapshot>,
    clock: Arc<SimClock>,
    owner: Arc<ProofOwner>,
    intercept: Arc<ProofIntercept>,
    dns: Arc<SimGuestDnsFactory>,
    cgroups: SimCgroupFs,
    gate: Arc<GuestNetworkExecGate>,
    exec: Arc<GuestNetworkExecSupervisor>,
    obs: Arc<SimObservationStore>,
    config_dir: PathBuf,
    elapsed: Duration,
    boot_calls: usize,
    _dir: tempfile::TempDir,
}

async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

/// Real-I/O settle for HTTPS/redb phases only (workload deploy). Wall-clock
/// here is integration-host scheduling, never a simulated measurement.
async fn settle_io() {
    tokio::time::sleep(Duration::from_millis(5)).await;
    settle().await;
}

impl Node {
    async fn boot(seed: u64) -> Self {
        let _ = event_store();
        let dir = tempfile::tempdir().expect("proof tempdir");
        let data_dir = dir.path().join("data");
        let config_dir = dir.path().join("operator");
        std::fs::create_dir_all(&data_dir).expect("proof data dir");
        std::fs::create_dir_all(&config_dir).expect("proof operator dir");
        let clock = Arc::new(SimClock::new());
        let clock_port: Arc<dyn Clock> = clock.clone();
        let intercept = Arc::new(ProofIntercept::new());
        let intercept_port: Arc<dyn MtlsIntercept> = intercept.clone();
        let dns = Arc::new(SimGuestDnsFactory::default());
        let dns_port: Arc<dyn GuestDnsFactory> = dns.clone();
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("loopback bind"),
            data_dir,
            operator_config_dir: config_dir.clone(),
            clock: Arc::clone(&clock_port),
            dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
            dataplane_override: Some(Arc::new(SimDataplane::new())),
            ..ServerConfig::new(Arc::new(SimKek::for_boot()), intercept_port, dns_port)
        };
        let owner = Arc::new(ProofOwner::default());
        let owner_port: Arc<dyn SharedGuestNetworkOwner> = owner.clone();
        let obs = Arc::new(SimObservationStore::single_peer(
            NodeId::new("nd295-proof-3-3").expect("node id"),
            seed,
        ));
        let obs_port: Arc<dyn ObservationStore> = obs.clone();
        let driver: Arc<dyn Driver> =
            Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock_port)));
        let cgroups = SimCgroupFs::new();
        let vm_cgroups = CgroupManager::new(PathBuf::from(CGROUP_ROOT), Arc::new(cgroups.clone()));
        let wiring = GuestNetworkExecWiring::new(Arc::clone(&clock_port));
        let gate = wiring.gate();
        let exec = wiring.supervisor();
        let handle = run_server_with_obs_and_driver(
            config,
            obs_port,
            driver,
            Arc::new(SimVmHostState::new()),
            owner_port,
            wiring,
            vm_cgroups,
        )
        .await
        .unwrap_or_else(|error| panic!("seed={seed:#x}: production boot failed: {error}"));
        settle().await;
        let boot_calls = owner.sim.calls().len();
        eprintln!(
            "seed={seed:#x} diagnostic=boot owner_calls={:?} intercept_calls={:?} dns_responders={}",
            owner.sim.calls(),
            intercept.calls_since(0),
            dns.responders().len()
        );
        Self {
            seed,
            handle: Some(handle),
            request: None,
            request_snapshot: None,
            clock,
            owner,
            intercept,
            dns,
            cgroups,
            gate,
            exec,
            obs,
            config_dir,
            elapsed: Duration::ZERO,
            boot_calls,
            _dir: dir,
        }
    }

    /// The required protection and DNS ports were composed at boot (R16): the
    /// worker's shared owner converged its program through the intercept port,
    /// and the DNS task owner built exactly its boot responder.
    fn composition_gap(&self) -> Option<String> {
        let legs = self.intercept.model.lock().legs;
        let responders = self.dns.responders().len();
        (legs.is_none() || responders != 1).then(|| {
            format!(
                "precondition R16: required ports not composed at boot (intercept legs {legs:?}, \
                 DNS responders built {responders})"
            )
        })
    }

    /// One non-blocking release attempt at the VM driver's claim boundary.
    fn admission(&self) -> Admission {
        match self.gate.claim_release().now_or_never() {
            Some(Some(claim)) => {
                drop(claim);
                Admission::Open
            }
            Some(None) => Admission::FailStopped,
            None => Admission::Closed,
        }
    }

    fn progress(&self) -> Option<SharedGuestNetworkRecovery> {
        self.exec.recovery_progress()
    }

    /// Owner-port calls issued after boot completed.
    fn runtime_calls(&self) -> Vec<GuestNetworkOperation> {
        self.owner.sim.calls()[self.boot_calls..].to_vec()
    }

    fn call_count(&self) -> usize {
        self.owner.sim.calls().len() - self.boot_calls
    }

    /// Observe the typed shutdown request without consuming the handle. Once a
    /// request is observed it is retained, with the cgroup snapshot taken at
    /// receipt, and never polled for again.
    fn poll_request(&mut self) -> Option<&ServeShutdownRequest> {
        if self.request.is_none() {
            let handle = self.handle.as_mut().expect("live server handle");
            if let Some(request) = handle.shutdown_requested().now_or_never() {
                self.request_snapshot = Some(self.cgroups.snapshot());
                self.request = Some((self.elapsed, request));
            }
        }
        self.request.as_ref().map(|(_, request)| request)
    }

    async fn step(&mut self) {
        self.clock.tick(STEP);
        self.elapsed += STEP;
        settle().await;
        let _ = self.poll_request();
    }

    async fn advance(&mut self, duration: Duration) {
        let target = self.elapsed + duration;
        while self.elapsed < target {
            self.step().await;
        }
    }

    /// Inject `component`'s loss together with the blocker that keeps its
    /// owner's repair failing. `Err` names a precondition production never
    /// produced.
    fn inject(&self, component: SharedGuestNetworkComponent) -> Result<(), String> {
        match component {
            SharedGuestNetworkComponent::IpRules => {
                self.intercept.update(|model| {
                    model.program_lost = true;
                    model.converge_blocked = true;
                });
                Ok(())
            }
            SharedGuestNetworkComponent::IpSets => {
                self.intercept.update(|model| {
                    model.foreign_member = Some(FOREIGN_MEMBER);
                    model.members_blocked = true;
                });
                Ok(())
            }
            SharedGuestNetworkComponent::LegF | SharedGuestNetworkComponent::LegC => {
                let Some(leg) = self.intercept.leg(component) else {
                    return Err(format!("no converged {component:?} address recorded"));
                };
                self.intercept.update(|model| model.binds_blocked = true);
                if self
                    .intercept
                    .sim
                    .script_accept(leg, SimAcceptScript::ListenerLost { errno: libc::EINVAL })
                {
                    Ok(())
                } else {
                    Err(format!("no live sim listener at the recorded {component:?} address {leg}"))
                }
            }
            SharedGuestNetworkComponent::Dns => {
                let Some(live) = self.dns.responders().last().cloned() else {
                    return Err("no DNS responder was built at boot".to_owned());
                };
                self.dns.script_probe_failure(true);
                live.end_serve(SimGuestDnsServeExit::Return);
                Ok(())
            }
            _ => {
                self.owner.sim.script_component_audit_failure(component, true);
                Ok(())
            }
        }
    }

    /// Lift the blocker so the owner's next repair of `component` succeeds.
    fn unblock(&self, component: SharedGuestNetworkComponent) {
        match component {
            SharedGuestNetworkComponent::IpRules => {
                self.intercept.update(|model| model.converge_blocked = false);
            }
            SharedGuestNetworkComponent::IpSets => {
                self.intercept.update(|model| model.members_blocked = false);
            }
            SharedGuestNetworkComponent::LegF | SharedGuestNetworkComponent::LegC => {
                self.intercept.update(|model| model.binds_blocked = false);
            }
            SharedGuestNetworkComponent::Dns => self.dns.script_probe_failure(false),
            _ => self.owner.sim.script_component_audit_failure(component, false),
        }
    }

    /// Make `component` healthy without any owner repair.
    fn heal(&self, component: SharedGuestNetworkComponent) {
        self.unblock(component);
        self.intercept.update(|model| match component {
            SharedGuestNetworkComponent::IpRules => model.program_lost = false,
            SharedGuestNetworkComponent::IpSets => model.foreign_member = None,
            _ => {}
        });
    }

    /// Create the workloads slice and each allocation's scope directory, with
    /// every ancestor, so a kill write under them lands in the snapshot.
    async fn create_scopes(&self, allocs: &BTreeSet<AllocationId>) {
        let root = Path::new(CGROUP_ROOT);
        self.cgroups
            .create_dir(&CgroupPath::workloads_slice().resolve(root))
            .await
            .expect("sim workloads slice");
        for alloc in allocs {
            self.cgroups
                .create_dir(&CgroupPath::for_alloc(alloc).resolve(root))
                .await
                .expect("sim allocation scope");
        }
    }

    async fn shutdown(mut self) {
        let seed = self.seed;
        if let Some(handle) = self.handle.take() {
            let clock = Arc::clone(&self.clock);
            let mut shutdown = std::pin::pin!(handle.shutdown(Duration::from_secs(2)));
            let result = loop {
                tokio::select! {
                    biased;
                    result = &mut shutdown => break result,
                    () = tokio::time::sleep(Duration::from_millis(1)) => clock.tick(STEP),
                }
            };
            if let Err(error) = result {
                eprintln!("seed={seed:#x} diagnostic=node_shutdown_error error={error}");
            }
        }
    }
}

fn kill_path(scope: &CgroupPath) -> PathBuf {
    scope.resolve(Path::new(CGROUP_ROOT)).join("cgroup.kill")
}

fn killed(snapshot: &CgroupSnapshot, scope: &CgroupPath) -> bool {
    snapshot.get(&kill_path(scope)) == Some(&(SimEntry::File, b"1\n".to_vec()))
}

fn touched(snapshot: &CgroupSnapshot, scope: &CgroupPath) -> bool {
    snapshot.contains_key(&kill_path(scope))
}

fn count(calls: &[GuestNetworkOperation], operation: GuestNetworkOperation) -> usize {
    calls.iter().filter(|call| **call == operation).count()
}

// ---------------------------------------------------------------------------
// Verdict bookkeeping.
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Verdict {
    Green,
    Red(String),
    Unreached(String),
}

struct Report {
    clause: &'static str,
    cells: Vec<(u64, String, Verdict)>,
}

impl Report {
    fn new(clause: &'static str) -> Self {
        eprintln!("proof=3.3 clause={clause} seeds={:x?}", seeds());
        Self { clause, cells: Vec::new() }
    }

    fn record(&mut self, seed: u64, cell: impl Into<String>, verdict: Verdict) {
        let cell = cell.into();
        let (label, detail) = match &verdict {
            Verdict::Green => ("GREEN", String::new()),
            Verdict::Red(detail) => ("RED", detail.clone()),
            Verdict::Unreached(detail) => ("UNREACHED", detail.clone()),
        };
        eprintln!(
            "proof=3.3 clause={} seed={seed:#x} cell={cell} verdict={label} {detail}",
            self.clause
        );
        self.cells.push((seed, cell, verdict));
    }

    fn finish(self) {
        let total = self.cells.len();
        let red: Vec<_> = self
            .cells
            .iter()
            .filter(|(_, _, verdict)| matches!(verdict, Verdict::Red(_)))
            .collect();
        let unreached: Vec<_> = self
            .cells
            .iter()
            .filter(|(_, _, verdict)| matches!(verdict, Verdict::Unreached(_)))
            .collect();
        eprintln!(
            "proof=3.3 clause={} summary total={total} green={} red={} unreached={}",
            self.clause,
            total - red.len() - unreached.len(),
            red.len(),
            unreached.len()
        );
        if let Some((seed, cell, verdict)) = red.first().or_else(|| unreached.first()) {
            panic!(
                "clause {} violated in {} of {total} cells (red={}, unreached={}); first: seed={seed:#x} cell={cell} {verdict:?}",
                self.clause,
                red.len() + unreached.len(),
                red.len(),
                unreached.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Shared episode: arm one component at a seeded phase and await detection.
// ---------------------------------------------------------------------------

struct Detected {
    at: Duration,
    latency: Duration,
    progress: SharedGuestNetworkRecovery,
    admission: Admission,
    calls_at_arm: usize,
    intercept_calls_at_arm: usize,
    responders_at_arm: usize,
    events_at_arm: usize,
}

enum ArmError {
    /// The stimulus could not be applied: a precondition production never
    /// produced.
    Unarmable(String),
    /// No recovery was observed within the detection horizon.
    Missed(String),
}

impl ArmError {
    fn into_unreached(self) -> Verdict {
        match self {
            Self::Unarmable(reason) => Verdict::Unreached(format!("precondition arm: {reason}")),
            Self::Missed(reason) => Verdict::Unreached(format!("precondition detection: {reason}")),
        }
    }
}

async fn arm_and_detect(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    phase: Duration,
) -> Result<Detected, ArmError> {
    if let Some(gap) = node.composition_gap() {
        return Err(ArmError::Unarmable(gap));
    }
    node.advance(phase).await;
    let armed_at = node.elapsed;
    let calls_at_arm = node.call_count();
    let intercept_calls_at_arm = node.intercept.calls_len();
    let responders_at_arm = node.dns.responders().len();
    let events_at_arm = events_cursor();
    node.inject(component).map_err(ArmError::Unarmable)?;
    while node.elapsed - armed_at < AUDIT_PERIOD + STEP {
        node.step().await;
        if let Some(progress) = node.progress() {
            return Ok(Detected {
                at: node.elapsed,
                latency: node.elapsed - armed_at,
                progress,
                admission: node.admission(),
                calls_at_arm,
                intercept_calls_at_arm,
                responders_at_arm,
                events_at_arm,
            });
        }
    }
    let calls_since_arm = node.runtime_calls()[calls_at_arm..].to_vec();
    Err(ArmError::Missed(format!(
        "no recovery within one audit period (+1 step) of the {component:?} fault: admission={:?} \
         audit_shared_calls_since_arm={} owner_calls_since_arm={calls_since_arm:?} \
         intercept_calls_since_arm={:?}",
        node.admission(),
        count(&calls_since_arm, GuestNetworkOperation::BridgeObserve),
        node.intercept.calls_since(intercept_calls_at_arm)
    )))
}

/// Advance until `progress.attempts` reaches `attempts`, returning the time it
/// was first observed, or `None` if the next attempt did not complete within
/// one attempt period (+1 step).
async fn await_attempt(
    node: &mut Node,
    attempts: u32,
) -> Option<(Duration, SharedGuestNetworkRecovery)> {
    let start = node.elapsed;
    while node.elapsed - start < ATTEMPT_PERIOD + STEP {
        node.step().await;
        match node.progress() {
            Some(progress) if progress.attempts >= attempts => {
                return Some((node.elapsed, progress));
            }
            None => return None,
            Some(_) => {}
        }
    }
    None
}

fn within_one_step(observed: Duration, expected: Duration) -> bool {
    observed.abs_diff(expected) <= STEP
}

/// Step until the recovery started at `detected_at` reopens, or its 5 s window
/// ends. Returns the reopen time.
async fn await_reopen(node: &mut Node, detected_at: Duration) -> Option<Duration> {
    while node.elapsed < detected_at + RECOVERY_DEADLINE + STEP && node.request.is_none() {
        node.step().await;
        if node.progress().is_none() {
            return Some(node.elapsed);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// C0 — healthy control.
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C0 — control: a healthy node composed with its required protection and DNS
/// ports never enters recovery, never quiesces or repairs, requests no
/// shutdown, and keeps admission open.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn healthy_owner_keeps_admission_open_without_recovery_effects() {
    let mut report = Report::new("C0-healthy-preservation");
    for seed in seeds() {
        let mut node = Node::boot(seed).await;
        if let Some(gap) = node.composition_gap() {
            report.record(seed, "healthy-10s", Verdict::Unreached(gap));
            node.shutdown().await;
            continue;
        }
        let events_at_boot = events_cursor();
        let intercept_at_boot = node.intercept.calls_len();
        let mut violation = (node.admission() != Admission::Open)
            .then(|| format!("admission after boot is {:?}", node.admission()));
        for _ in 0..1_000 {
            node.step().await;
            if violation.is_none() {
                if let Some(progress) = node.progress() {
                    violation =
                        Some(format!("spurious recovery at {:?}: {progress:?}", node.elapsed));
                } else if node.admission() != Admission::Open {
                    violation =
                        Some(format!("admission {:?} at {:?}", node.admission(), node.elapsed));
                } else if node.request.is_some() {
                    violation = Some(format!("spurious shutdown request {:?}", node.request));
                }
            }
        }
        let calls = node.runtime_calls();
        if violation.is_none() {
            for forbidden in
                [GuestNetworkOperation::TapSetDown, GuestNetworkOperation::BridgeConverge]
            {
                if count(&calls, forbidden) != 0 {
                    violation = Some(format!("healthy owner received {forbidden:?}: {calls:?}"));
                }
            }
        }
        let intercept_repairs: Vec<_> = node
            .intercept
            .calls_since(intercept_at_boot)
            .into_iter()
            .filter(|call| call.is_repair())
            .collect();
        if violation.is_none() && !intercept_repairs.is_empty() {
            violation = Some(format!("healthy intercept was repaired: {intercept_repairs:?}"));
        }
        if violation.is_none() && node.dns.responders().len() != 1 {
            violation = Some(format!(
                "healthy DNS was replaced: {} responders",
                node.dns.responders().len()
            ));
        }
        let unhealthy =
            named(&events_since(events_at_boot), "guest_network.shared_owner_unhealthy").len();
        if violation.is_none() && unhealthy != 0 {
            violation = Some(format!("{unhealthy} unhealthy events on a healthy node"));
        }
        report.record(seed, "healthy-10s", violation.map_or(Verdict::Green, Verdict::Red));
        node.shutdown().await;
    }
    report.finish();
}

// ---------------------------------------------------------------------------
// C1 — the one-second full audit reads every owner.
// ---------------------------------------------------------------------------

fn gaps(times: &[Duration], window: Duration) -> Vec<Duration> {
    let mut gaps = Vec::with_capacity(times.len() + 1);
    let mut previous = Duration::ZERO;
    for at in times {
        gaps.push(*at - previous);
        previous = *at;
    }
    gaps.push(window - previous);
    gaps
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C1 — the full audit reads the shared owner (`audit_shared`) exactly once per
/// 1 s audit period and the intercept owner's state (`observe_shared_state`)
/// at least once per period.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn healthy_node_audits_the_shared_owner_every_second() {
    let mut report = Report::new("C1-one-second-audit-cadence");
    for seed in seeds() {
        let mut node = Node::boot(seed).await;
        if let Some(gap) = node.composition_gap() {
            report.record(seed, "healthy-10s", Verdict::Unreached(gap));
            node.shutdown().await;
            continue;
        }
        let intercept_at_boot = node.intercept.calls_len();
        let mut owner_audits = Vec::new();
        let mut intercept_audits = Vec::new();
        let (mut owner_seen, mut intercept_seen) = (0, 0);
        for _ in 0..1_000 {
            node.step().await;
            let owner_now = count(&node.runtime_calls(), GuestNetworkOperation::BridgeObserve);
            for _ in owner_seen..owner_now {
                owner_audits.push(node.elapsed);
            }
            owner_seen = owner_now;
            let intercept_now = node
                .intercept
                .calls_since(intercept_at_boot)
                .into_iter()
                .filter(|call| *call == InterceptCall::ObserveState)
                .count();
            for _ in intercept_seen..intercept_now {
                intercept_audits.push(node.elapsed);
            }
            intercept_seen = intercept_now;
        }
        let window = node.elapsed;
        let owner_gaps = gaps(&owner_audits, window);
        let intercept_gaps = gaps(&intercept_audits, window);
        let owner_max = owner_gaps.iter().copied().max().unwrap_or(window);
        let owner_min = owner_audits.windows(2).map(|pair| pair[1] - pair[0]).min();
        let intercept_max = intercept_gaps.iter().copied().max().unwrap_or(window);
        let verdict = if owner_max > AUDIT_PERIOD + STEP {
            Verdict::Red(format!(
                "audit_shared calls in {window:?} = {}; largest unaudited interval {owner_max:?} > \
                 {AUDIT_PERIOD:?} (audit times {owner_audits:?})",
                owner_audits.len()
            ))
        } else if owner_min.is_some_and(|min| min + STEP < AUDIT_PERIOD) {
            Verdict::Red(format!(
                "more than one audit_shared per period: smallest interval {owner_min:?} \
                 (audit times {owner_audits:?})"
            ))
        } else if intercept_max > AUDIT_PERIOD + STEP {
            Verdict::Red(format!(
                "observe_shared_state calls in {window:?} = {}; largest unaudited interval \
                 {intercept_max:?} > {AUDIT_PERIOD:?} (times {intercept_audits:?})",
                intercept_audits.len()
            ))
        } else {
            Verdict::Green
        };
        report.record(seed, "healthy-10s", verdict);
        node.shutdown().await;
    }
    report.finish();
}

// ---------------------------------------------------------------------------
// C2 — every component is detected within one audit and closes admission.
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C2 — each of the eleven node-level components' loss is detected within one
/// audit period, closes new guest-command release, records
/// `Recovering { component, 0 }`, and emits exactly one
/// `guest_network.shared_owner_unhealthy { component, cause }` whose cause is
/// `audit_mismatch` for an audited loss and `task_exit` for a listener or DNS
/// task end; a shared-owner audit that never answers is detected within 5 s of
/// injected time as `Bridge` with cause `audit_timeout`.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn every_component_loss_is_detected_within_one_audit_and_closes_admission() {
    let mut report = Report::new("C2-detection-and-admission-closure");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, index);
            let phase = seeded_phase(&mut rng);
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}@phase={}ms", phase.as_millis());
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(ArmError::Unarmable(reason)) => Verdict::Unreached(reason),
                Err(ArmError::Missed(reason)) => Verdict::Red(reason),
                Ok(detected) => {
                    let events = events_since(detected.events_at_arm);
                    let unhealthy = named(&events, "guest_network.shared_owner_unhealthy");
                    if detected.progress.component != component || detected.progress.attempts != 0 {
                        Verdict::Red(format!("detection snapshot {:?}", detected.progress))
                    } else if detected.latency > AUDIT_PERIOD + STEP {
                        Verdict::Red(format!("detection latency {:?}", detected.latency))
                    } else if detected.admission != Admission::Closed {
                        Verdict::Red(format!("admission {:?} while Recovering", detected.admission))
                    } else if unhealthy.len() != 1
                        || !unhealthy[0]
                            .fields
                            .get("component")
                            .is_some_and(|rendered| same_label(rendered, component))
                    {
                        Verdict::Red(format!("unhealthy events {unhealthy:?}"))
                    } else if unhealthy[0].fields.get("cause").map(String::as_str)
                        != Some(expected_cause(component))
                    {
                        Verdict::Red(format!(
                            "unhealthy cause {:?}; expected {}",
                            unhealthy[0].fields.get("cause"),
                            expected_cause(component)
                        ))
                    } else {
                        Verdict::Green
                    }
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
        // The hung-audit cell: the shared owner's periodic audit never answers.
        let mut rng = cell_rng(seed, 50);
        let phase = seeded_phase(&mut rng);
        let mut node = Node::boot(seed).await;
        let cell = format!("hung-audit@phase={}ms", phase.as_millis());
        let verdict = hung_audit_detection(&mut node, phase).await;
        report.record(seed, cell, verdict);
        node.shutdown().await;
    }
    report.finish();
}

/// A shared-owner audit that never answers fails that owner's first
/// component, `Bridge`, with cause `audit_timeout`. The call bound is private
/// and E18-derived, so detection is bounded only by the 5 s horizon (FD
/// 4123-4145, 4200-4202).
async fn hung_audit_detection(node: &mut Node, phase: Duration) -> Verdict {
    if let Some(gap) = node.composition_gap() {
        return Verdict::Unreached(gap);
    }
    node.advance(phase).await;
    let events_at_arm = events_cursor();
    let latch = node.owner.arm_audit_latch();
    let armed_at = node.elapsed;
    let mut detected = None;
    while node.elapsed - armed_at <= RECOVERY_DEADLINE {
        node.step().await;
        if let Some(progress) = node.progress() {
            detected = Some((node.elapsed - armed_at, progress, node.admission()));
            break;
        }
    }
    let hung = node.owner.audits_in_flight();
    node.owner.disarm_audit_latch();
    latch.add_permits(64);
    let Some((latency, progress, admission)) = detected else {
        return if hung == 0 {
            Verdict::Unreached("precondition: no owner audit was held in flight".to_owned())
        } else {
            Verdict::Red(format!(
                "a hung shared-owner audit was not detected within {RECOVERY_DEADLINE:?}"
            ))
        };
    };
    let events = events_since(events_at_arm);
    let unhealthy = named(&events, "guest_network.shared_owner_unhealthy");
    if progress.component != SharedGuestNetworkComponent::Bridge || progress.attempts != 0 {
        Verdict::Red(format!("hung-audit detection snapshot {progress:?} after {latency:?}"))
    } else if admission != Admission::Closed {
        Verdict::Red(format!("admission {admission:?} while Recovering"))
    } else if unhealthy.len() != 1
        || unhealthy[0].fields.get("cause").map(String::as_str) != Some("audit_timeout")
    {
        Verdict::Red(format!("unhealthy events for a hung audit {unhealthy:?}"))
    } else {
        Verdict::Green
    }
}

// ---------------------------------------------------------------------------
// C3 — component-specific TAP quiescence.
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C3 — a kernel-path loss quiesces managed TAPs exactly once, before the first
/// repair attempt; a pure listener or DNS loss never quiesces TAPs.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn kernel_path_loss_quiesces_once_before_repair_and_listener_or_dns_loss_never_does() {
    let mut report = Report::new("C3-component-quiescence-rules");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let kernel_path = is_kernel_path(component);
            let mut rng = cell_rng(seed, index);
            let phase = seeded_phase(&mut rng);
            let failed_attempts = rng.inclusive(0, 3);
            let mut node = Node::boot(seed).await;
            let cell = format!(
                "{component:?}/kernel_path={kernel_path}/failed_attempts={failed_attempts}"
            );
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(error) => error.into_unreached(),
                Ok(detected) => {
                    let mut unreached = None;
                    for attempt in 1..=failed_attempts {
                        if await_attempt(&mut node, attempt).await.is_none() {
                            unreached = Some(format!("attempt {attempt} never completed"));
                            break;
                        }
                    }
                    node.unblock(component);
                    if unreached.is_none() {
                        node.advance(ATTEMPT_PERIOD + STEP).await;
                    }
                    let episode = node.runtime_calls()[detected.calls_at_arm..].to_vec();
                    let quiesces = count(&episode, GuestNetworkOperation::TapSetDown);
                    let first_quiesce =
                        episode.iter().position(|call| *call == GuestNetworkOperation::TapSetDown);
                    let detection_audit = episode
                        .iter()
                        .position(|call| *call == GuestNetworkOperation::BridgeObserve);
                    let first_repair = episode
                        .iter()
                        .enumerate()
                        .skip(detection_audit.map_or(0, |index| index + 1))
                        .find(|(_, call)| {
                            matches!(
                                call,
                                GuestNetworkOperation::BridgeConverge
                                    | GuestNetworkOperation::BridgeObserve
                            )
                        })
                        .map(|(index, _)| index);
                    match (kernel_path, unreached) {
                        (_, Some(reason)) => Verdict::Unreached(reason),
                        (true, None) => {
                            if quiesces != 1 {
                                Verdict::Red(format!("TapSetDown x{quiesces}: {episode:?}"))
                            } else if first_repair.is_some_and(|repair| {
                                first_quiesce.is_some_and(|quiesce| quiesce > repair)
                            }) {
                                Verdict::Red(format!("repair preceded quiesce: {episode:?}"))
                            } else {
                                Verdict::Green
                            }
                        }
                        (false, None) => {
                            if quiesces == 0 {
                                Verdict::Green
                            } else {
                                Verdict::Red(format!(
                                    "pure listener/DNS loss quiesced TAPs x{quiesces}: {episode:?}"
                                ))
                            }
                        }
                    }
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

// ---------------------------------------------------------------------------
// C4 — exact-owner repair on the attempt cadence; partial repair; one reopen.
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C4 — recovery repairs the failed component through its owning component
/// every 250 ms; each attempt counts only after convergence plus a full audit;
/// a partial repair records the first remaining component and never reopens;
/// a kernel-path recovery restores the quiesced TAPs once after a clean audit;
/// complete repair reopens admission exactly once.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn repair_runs_through_the_owning_component_on_the_attempt_cadence_and_reopens_once() {
    let mut report = Report::new("C4-exact-owner-repair-cadence-reopen");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, index);
            let phase = seeded_phase(&mut rng);
            let failed_attempts = rng.inclusive(0, 12);
            // Partial repair only within the shared-owner class, so the exact
            // owner of the first remaining component is unchanged.
            let partial = (repair_owner(component) == RepairOwner::SharedGuestNetworkOwner
                && rng.coin())
            .then(|| {
                let peers: Vec<_> = COMPONENTS
                    .into_iter()
                    .filter(|peer| {
                        *peer != component
                            && repair_owner(*peer) == RepairOwner::SharedGuestNetworkOwner
                    })
                    .collect();
                let pick = rng.inclusive(0, peers.len() as u32 - 1) as usize;
                (peers[pick], rng.inclusive(1, 4))
            });
            let mut node = Node::boot(seed).await;
            let cell = format!(
                "{component:?}/owner={:?}/failed_attempts={failed_attempts}/partial={partial:?}",
                repair_owner(component)
            );
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(error) => error.into_unreached(),
                Ok(detected) => {
                    recovery_episode(&mut node, component, &detected, failed_attempts, partial)
                        .await
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

/// The repair evidence of the owning component, beyond the owner journal.
fn owner_repair_evidence(
    node: &Node,
    component: SharedGuestNetworkComponent,
    detected: &Detected,
) -> Result<(), String> {
    let intercept_calls = node.intercept.calls_since(detected.intercept_calls_at_arm);
    let intercept_repairs: Vec<_> =
        intercept_calls.iter().copied().filter(|call| call.is_repair()).collect();
    let responders_built = node.dns.responders().len() - detected.responders_at_arm;
    match component {
        SharedGuestNetworkComponent::IpRules => {
            if !intercept_calls.contains(&InterceptCall::ConvergeShared) {
                return Err(format!(
                    "IpRules repaired without converge_shared: {intercept_calls:?}"
                ));
            }
            if !node.intercept.program_present() {
                return Err("the owned program reads back absent after reopen".to_owned());
            }
        }
        SharedGuestNetworkComponent::IpSets => {
            if !intercept_calls.contains(&InterceptCall::ConvergeMembers) {
                return Err(format!(
                    "IpSets repaired without converge_allocation_elements: {intercept_calls:?}"
                ));
            }
            if node.intercept.foreign_member_present() {
                return Err("the unexpected member reads back after reopen".to_owned());
            }
        }
        SharedGuestNetworkComponent::LegF | SharedGuestNetworkComponent::LegC => {
            let leg = node.intercept.leg(component).ok_or("no recorded leg address")?;
            if !intercept_calls.contains(&InterceptCall::Bind(leg)) {
                return Err(format!(
                    "{component:?} not rebound at its recorded address {leg}: {intercept_calls:?}"
                ));
            }
            if !node.intercept.sim.live_listeners().contains(&leg) {
                return Err(format!(
                    "no live listener at the recorded {component:?} address {leg} after reopen \
                     (live {:?})",
                    node.intercept.sim.live_listeners()
                ));
            }
        }
        SharedGuestNetworkComponent::Dns => {
            if responders_built == 0 {
                return Err("DNS reopened without a freshly built responder".to_owned());
            }
            if !intercept_repairs.is_empty() {
                return Err(format!("DNS recovery repaired the intercept: {intercept_repairs:?}"));
            }
        }
        _ => {
            if !intercept_repairs.is_empty() {
                return Err(format!(
                    "shared-owner recovery repaired the intercept: {intercept_repairs:?}"
                ));
            }
            if responders_built != 0 {
                return Err(format!(
                    "shared-owner recovery built {responders_built} DNS responders"
                ));
            }
        }
    }
    Ok(())
}

async fn recovery_episode(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    detected: &Detected,
    failed_attempts: u32,
    partial: Option<(SharedGuestNetworkComponent, u32)>,
) -> Verdict {
    let calls_at_detection = node.call_count();
    // Phase 1: `failed_attempts` failed attempts against the original component.
    for attempt in 1..=failed_attempts {
        let Some((at, progress)) = await_attempt(node, attempt).await else {
            return Verdict::Red(format!("attempt {attempt} did not complete within one period"));
        };
        let expected_elapsed = ATTEMPT_PERIOD * attempt;
        if progress
            != (SharedGuestNetworkRecovery {
                component,
                attempts: attempt,
                elapsed: progress.elapsed,
            })
            || !within_one_step(progress.elapsed, expected_elapsed)
            || !within_one_step(at - detected.at, expected_elapsed)
        {
            return Verdict::Red(format!(
                "attempt {attempt} snapshot {progress:?} at {:?} after detection",
                at - detected.at
            ));
        }
        if node.admission() != Admission::Closed {
            return Verdict::Red(format!(
                "admission {:?} during attempt {attempt}",
                node.admission()
            ));
        }
    }
    // Phase 2 (optional): the original component's repair succeeds but a
    // second remains.
    let mut total_failed = failed_attempts;
    node.unblock(component);
    if let Some((second, partial_attempts)) = partial {
        node.owner.sim.script_component_audit_failure(second, true);
        for offset in 1..=partial_attempts {
            let attempt = failed_attempts + offset;
            let Some((_, progress)) = await_attempt(node, attempt).await else {
                return Verdict::Red(format!("partial attempt {attempt} did not complete"));
            };
            if progress.component != second || progress.attempts != attempt {
                return Verdict::Red(format!(
                    "partial repair snapshot {progress:?}; expected first remaining {second:?}"
                ));
            }
            if node.admission() != Admission::Closed {
                return Verdict::Red(format!(
                    "partial repair reopened admission at attempt {attempt}"
                ));
            }
        }
        total_failed += partial_attempts;
        node.owner.sim.script_component_audit_failure(second, false);
    }
    // Phase 3: the next attempt converges and passes the full audit.
    let reopen_deadline = node.elapsed + ATTEMPT_PERIOD + STEP;
    let mut reopened_at = None;
    while node.elapsed < reopen_deadline {
        node.step().await;
        if node.progress().is_none() {
            reopened_at = Some(node.elapsed);
            break;
        }
    }
    let Some(reopened_at) = reopened_at else {
        return Verdict::Red(format!(
            "no reopen one attempt after repair; progress {:?}",
            node.progress()
        ));
    };
    let expected_reopen = ATTEMPT_PERIOD * (total_failed + 1);
    if !within_one_step(reopened_at - detected.at, expected_reopen) {
        return Verdict::Red(format!(
            "reopened {:?} after detection; expected {expected_reopen:?}",
            reopened_at - detected.at
        ));
    }
    if node.admission() != Admission::Open {
        return Verdict::Red(format!("admission {:?} after complete repair", node.admission()));
    }
    let recovery_calls = node.runtime_calls()[calls_at_detection..].to_vec();
    let completed = total_failed + 1;
    let converges = count(&recovery_calls, GuestNetworkOperation::BridgeConverge) as u32;
    let audits = count(&recovery_calls, GuestNetworkOperation::BridgeObserve) as u32;
    let restores = count(&recovery_calls, GuestNetworkOperation::TapSetUp);
    match repair_owner(component) {
        RepairOwner::SharedGuestNetworkOwner => {
            let pairs = recovery_calls
                .iter()
                .filter(|call| {
                    matches!(
                        call,
                        GuestNetworkOperation::BridgeConverge
                            | GuestNetworkOperation::BridgeObserve
                    )
                })
                .copied()
                .collect::<Vec<_>>();
            let expected_pairs =
                [GuestNetworkOperation::BridgeConverge, GuestNetworkOperation::BridgeObserve]
                    .repeat(completed as usize);
            if converges != completed || pairs != expected_pairs {
                return Verdict::Red(format!(
                    "attempts are not shared-owner converge-then-full-audit pairs: {recovery_calls:?}"
                ));
            }
        }
        RepairOwner::MtlsWorker | RepairOwner::DnsOwner => {
            if converges != 0 {
                return Verdict::Red(format!(
                    "the shared owner was converged while repairing {component:?}: {recovery_calls:?}"
                ));
            }
            if audits != completed {
                return Verdict::Red(format!("full audit x{audits} for {completed} attempts"));
            }
        }
    }
    let expected_restores = usize::from(is_kernel_path(component));
    if restores != expected_restores {
        return Verdict::Red(format!(
            "TapSetUp x{restores} during recovery; expected {expected_restores} (restore after the \
             clean audit only when TAPs were quiesced): {recovery_calls:?}"
        ));
    }
    if let Err(reason) = owner_repair_evidence(node, component, detected) {
        return Verdict::Red(reason);
    }
    let events = events_since(detected.events_at_arm);
    let retries = named(&events, "guest_network.shared_owner_retry");
    let recovered = named(&events, "guest_network.shared_owner_recovered");
    if named(&events, "guest_network.shared_owner_unhealthy").len() != 1
        || retries.len() != total_failed as usize
        || recovered.len() != 1
    {
        return Verdict::Red(format!(
            "operational events: unhealthy/retry/recovered = {}/{}/{} for {total_failed} failed attempts",
            named(&events, "guest_network.shared_owner_unhealthy").len(),
            retries.len(),
            recovered.len()
        ));
    }
    for (index, retry) in retries.iter().enumerate() {
        let attempt = index as u32 + 1;
        if retry.fields.get("attempt") != Some(&attempt.to_string()) {
            return Verdict::Red(format!("retry event {index} {retry:?}"));
        }
    }
    // Post-recovery preservation: admission stays open and no further quiesce.
    let calls_after_reopen = node.call_count();
    node.advance(Duration::from_secs(2)).await;
    let after = node.runtime_calls()[calls_after_reopen..].to_vec();
    if node.admission() != Admission::Open || node.progress().is_some() {
        return Verdict::Red(format!(
            "post-recovery admission {:?} progress {:?}",
            node.admission(),
            node.progress()
        ));
    }
    if count(&after, GuestNetworkOperation::TapSetDown) != 0 {
        return Verdict::Red(format!("post-recovery quiesce: {after:?}"));
    }
    Verdict::Green
}

// ---------------------------------------------------------------------------
// C5 — bounded window ends in one typed fail-stop request.
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C5 — an unrepaired component fail-stops at exactly five seconds / twenty
/// completed attempts with one typed request, and admission refuses from then.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn unrepaired_loss_fail_stops_with_one_typed_request_at_the_deadline() {
    let mut report = Report::new("C5-deadline-typed-fail-stop");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, index);
            let phase = seeded_phase(&mut rng);
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}");
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(error) => error.into_unreached(),
                Ok(detected) => deadline_episode(&mut node, component, &detected).await,
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

async fn deadline_episode(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    detected: &Detected,
) -> Verdict {
    let deadline = detected.at + RECOVERY_DEADLINE;
    while node.elapsed + STEP < deadline {
        node.step().await;
        if let Some((at, request)) = node.request.clone() {
            return Verdict::Red(format!(
                "request {request:?} before the deadline, {:?} after detection",
                at - detected.at
            ));
        }
        if node.admission() != Admission::Closed {
            return Verdict::Red(format!(
                "admission {:?} {:?} after detection",
                node.admission(),
                node.elapsed - detected.at
            ));
        }
    }
    node.advance(STEP * 3).await;
    let Some((at, request)) = node.request.clone() else {
        return Verdict::Red(format!(
            "no typed request {:?} after detection; progress {:?} admission {:?}",
            node.elapsed - detected.at,
            node.progress(),
            node.admission()
        ));
    };
    let expected = ServeShutdownRequest::SharedGuestNetwork(SharedGuestNetworkFailStop {
        component,
        cause: SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded,
        attempts: DEADLINE_ATTEMPTS,
        elapsed: RECOVERY_DEADLINE,
    });
    if request != expected || !within_one_step(at - detected.at, RECOVERY_DEADLINE) {
        return Verdict::Red(format!(
            "request {request:?} at {:?}; expected {expected:?} at {RECOVERY_DEADLINE:?}",
            at - detected.at
        ));
    }
    if node.admission() != Admission::FailStopped {
        return Verdict::Red(format!("admission {:?} after fail-stop", node.admission()));
    }
    Verdict::Green
}

// ---------------------------------------------------------------------------
// C6 — kill scope: unconfirmed, undetermined, and damaged-while-open.
// ---------------------------------------------------------------------------

fn operator_client(config_dir: &Path) -> reqwest::Client {
    let contents = std::fs::read_to_string(config_dir.join(".overdrive/config"))
        .expect("operator trust config written at boot");
    let config: toml::Value = toml::from_str(&contents).expect("operator trust config parses");
    let encoded = config["contexts"]
        .as_array()
        .expect("contexts array")
        .iter()
        .find(|entry| entry["name"].as_str() == Some("local"))
        .expect("local context")["ca"]
        .as_str()
        .expect("ca field");
    let pem = base64::engine::general_purpose::STANDARD.decode(encoded).expect("ca base64");
    reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(&pem).expect("ca pem"))
        .https_only(true)
        .use_rustls_tls()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("operator client")
}

async fn submit_vm_job(client: &reqwest::Client, base: &str, id: &str) {
    let body = SubmitWorkloadRequest {
        spec: SubmitSpecInput::Job(JobSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/sleep".to_owned(),
                args: vec!["3600".to_owned()],
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
        }),
    };
    let response = client
        .post(format!("{base}/v1/workloads"))
        .json(&body)
        .send()
        .await
        .expect("submit reaches the public API");
    assert!(response.status().is_success(), "submit {id}: {response:?}");
    let _: SubmitWorkloadResponse = response.json().await.expect("submit response");
}

async fn running_allocations(obs: &SimObservationStore) -> BTreeSet<AllocationId> {
    obs.alloc_status_rows()
        .await
        .expect("sim observation rows")
        .into_iter()
        .filter(|row| row.state == AllocState::Running)
        .map(|row| row.alloc_id)
        .collect()
}

/// Deploy `count` VM jobs through the public HTTPS API and return the running
/// allocation ids once every job is Running.
async fn deploy_running_vms(node: &mut Node, count: usize) -> BTreeSet<AllocationId> {
    let bound = node
        .handle
        .as_ref()
        .expect("live handle")
        .local_addr()
        .await
        .expect("server bound address");
    let client = operator_client(&node.config_dir);
    let base = format!("https://localhost:{}", bound.port());
    for index in 0..count {
        submit_vm_job(&client, &base, &format!("nd295-proof-vm-{index}")).await;
    }
    for _ in 0..200 {
        node.clock.tick(Duration::from_millis(100));
        node.elapsed += Duration::from_millis(100);
        settle_io().await;
        let running = running_allocations(&node.obs).await;
        if running.len() >= count {
            return running;
        }
    }
    panic!("seed={:#x}: {count} VM jobs did not reach Running through the public API", node.seed);
}

/// Deploy two or three VMs and pick the seeded allocation A; create the slice
/// and every scope directory in the one `SimCgroupFs`.
async fn deploy_and_pick(
    node: &mut Node,
    rng: &mut CellRng,
) -> (AllocationId, BTreeSet<AllocationId>) {
    let vm_count = rng.inclusive(2, 3) as usize;
    let running = deploy_running_vms(node, vm_count).await;
    node.create_scopes(&running).await;
    let pick = rng.inclusive(0, running.len() as u32 - 1) as usize;
    let affected = running.iter().nth(pick).cloned().expect("a running allocation");
    let others = running.into_iter().filter(|alloc| *alloc != affected).collect();
    (affected, others)
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C6a — when a kernel-path loss's quiescence reports exactly allocation A's TAP
/// unconfirmed, only A is killed (`1\n` at A's scope `cgroup.kill`, nothing at
/// any other scope or at the workloads slice), one
/// `guest_network.shared_owner_vm_killed { alloc: A, cause:
/// "quiescence_unconfirmed" }` is emitted, and recovery continues and reopens
/// with no shutdown request.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn unconfirmed_quiescence_kills_only_the_affected_vm_and_recovery_reopens() {
    let mut report = Report::new("C6a-unconfirmed-quiescence-kills-only-that-vm");
    for seed in seeds() {
        for (index, component) in kernel_path_components().into_iter().enumerate() {
            let mut rng = cell_rng(seed, 100 + index);
            let phase = seeded_phase(&mut rng);
            let failed_attempts = rng.inclusive(0, 2);
            let mut node = Node::boot(seed).await;
            let verdict = if let Some(gap) = node.composition_gap() {
                (format!("{component:?}"), Verdict::Unreached(gap))
            } else {
                let (affected, others) = deploy_and_pick(&mut node, &mut rng).await;
                node.owner.sim.script_quiesce_outcome(SimQuiesceOutcome::Unconfirmed(
                    BTreeSet::from([affected.clone()]),
                ));
                let cell = format!(
                    "{component:?}/affected={affected}/others={}/failed_attempts={failed_attempts}",
                    others.len()
                );
                let verdict = match arm_and_detect(&mut node, component, phase).await {
                    Err(error) => error.into_unreached(),
                    Ok(detected) => {
                        unconfirmed_episode(
                            &mut node,
                            component,
                            &detected,
                            failed_attempts,
                            &affected,
                            &others,
                        )
                        .await
                    }
                };
                (cell, verdict)
            };
            report.record(seed, verdict.0, verdict.1);
            node.shutdown().await;
        }
    }
    report.finish();
}

async fn unconfirmed_episode(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    detected: &Detected,
    failed_attempts: u32,
    affected: &AllocationId,
    others: &BTreeSet<AllocationId>,
) -> Verdict {
    for attempt in 1..=failed_attempts {
        if await_attempt(node, attempt).await.is_none() {
            return Verdict::Red(format!(
                "attempt {attempt} did not complete after the per-VM kill; progress {:?} request \
                 {:?}",
                node.progress(),
                node.request
            ));
        }
    }
    node.unblock(component);
    let Some(reopened_at) = await_reopen(node, detected.at).await else {
        return Verdict::Red(format!(
            "no reopen within the 5 s window after the per-VM kill; progress {:?} request {:?}",
            node.progress(),
            node.request
        ));
    };
    let snapshot = node.cgroups.snapshot();
    let events = events_since(detected.events_at_arm);
    let kills = vm_killed_for(&events, affected);
    if !killed(&snapshot, &CgroupPath::for_alloc(affected)) {
        return Verdict::Red(format!("no `1\\n` at {affected}'s scope cgroup.kill: {snapshot:?}"));
    }
    if let Some(other) =
        others.iter().find(|other| touched(&snapshot, &CgroupPath::for_alloc(other)))
    {
        return Verdict::Red(format!("unaffected allocation {other} was killed: {snapshot:?}"));
    }
    if touched(&snapshot, &CgroupPath::workloads_slice()) {
        return Verdict::Red(format!("the whole workloads slice was killed: {snapshot:?}"));
    }
    if kills.len() != 1
        || kills[0].fields.get("cause").map(String::as_str) != Some("quiescence_unconfirmed")
    {
        return Verdict::Red(format!("vm_killed events for {affected}: {kills:?}"));
    }
    if let Some(other) = others.iter().find(|other| !vm_killed_for(&events, other).is_empty()) {
        return Verdict::Red(format!("vm_killed event for unaffected {other}"));
    }
    if node.request.is_some() || node.admission() != Admission::Open {
        return Verdict::Red(format!(
            "after reopen at {:?}: request {:?} admission {:?}",
            reopened_at - detected.at,
            node.request,
            node.admission()
        ));
    }
    Verdict::Green
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C6b — when a kernel-path loss's quiescence fails or does not answer (the
/// failing set is undetermined), the node sends exactly one typed request
/// `TapQuiescenceUndetermined` within the 5 s window, and when that request is
/// received the snapshot holds `1\n` at the workloads slice's `cgroup.kill`.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn undetermined_quiescence_fails_the_node_with_one_typed_request() {
    let mut report = Report::new("C6b-undetermined-quiescence-slice-kill-and-fail-stop");
    for seed in seeds() {
        for (index, component) in kernel_path_components().into_iter().enumerate() {
            let mut rng = cell_rng(seed, 150 + index);
            let phase = seeded_phase(&mut rng);
            let (label, outcome) = if (index + usize::from(rng.coin())) % 2 == 0 {
                ("Fail", SimQuiesceOutcome::Fail)
            } else {
                ("Hang", SimQuiesceOutcome::Hang)
            };
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}/quiesce={label}");
            let verdict = if let Some(gap) = node.composition_gap() {
                Verdict::Unreached(gap)
            } else {
                node.create_scopes(&BTreeSet::new()).await;
                node.owner.sim.script_quiesce_outcome(outcome);
                match arm_and_detect(&mut node, component, phase).await {
                    Err(error) => error.into_unreached(),
                    Ok(detected) => undetermined_episode(&mut node, component, &detected).await,
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

async fn undetermined_episode(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    detected: &Detected,
) -> Verdict {
    while node.elapsed < detected.at + RECOVERY_DEADLINE + STEP * 3 && node.request.is_none() {
        node.step().await;
    }
    let Some((at, request)) = node.request.clone() else {
        return Verdict::Red(format!(
            "no typed request within the 5 s window after detection; progress {:?} admission {:?}",
            node.progress(),
            node.admission()
        ));
    };
    let ServeShutdownRequest::SharedGuestNetwork(fail_stop) = &request;
    if fail_stop.cause != SharedGuestNetworkFailStopCause::TapQuiescenceUndetermined
        || fail_stop.component != component
        || fail_stop.attempts != 0
        || fail_stop.elapsed > RECOVERY_DEADLINE
        || at - detected.at > RECOVERY_DEADLINE + STEP
    {
        return Verdict::Red(format!(
            "request {request:?} {:?} after detection; expected {component:?} / \
             TapQuiescenceUndetermined / 0 attempts within {RECOVERY_DEADLINE:?}",
            at - detected.at
        ));
    }
    let snapshot = node.request_snapshot.clone().unwrap_or_default();
    if !killed(&snapshot, &CgroupPath::workloads_slice()) {
        return Verdict::Red(format!(
            "no `1\\n` at the workloads slice cgroup.kill when the request was received: {snapshot:?}"
        ));
    }
    if node.admission() != Admission::FailStopped {
        return Verdict::Red(format!("admission {:?} after fail-stop", node.admission()));
    }
    Verdict::Green
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C6c — per-allocation damage reported by a clean audit while admission is
/// open kills exactly the damaged VM (`1\n` at A's scope `cgroup.kill` only),
/// emits one `guest_network.shared_owner_vm_killed { alloc: A, cause:
/// "attachment_damaged" }`, keeps admission open with no recovery, and never
/// names A again.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn audit_damage_while_open_kills_only_that_vm_and_keeps_admission_open() {
    let mut report = Report::new("C6c-damage-while-open-kills-only-that-vm");
    for seed in seeds() {
        let mut rng = cell_rng(seed, 180);
        let mut node = Node::boot(seed).await;
        let (cell, verdict) = if let Some(gap) = node.composition_gap() {
            ("damage-while-open".to_owned(), Verdict::Unreached(gap))
        } else {
            let (affected, others) = deploy_and_pick(&mut node, &mut rng).await;
            let cell = format!("damage-while-open/affected={affected}/others={}", others.len());
            (cell, damage_episode(&mut node, &affected, &others).await)
        };
        report.record(seed, cell, verdict);
        node.shutdown().await;
    }
    report.finish();
}

async fn damage_episode(
    node: &mut Node,
    affected: &AllocationId,
    others: &BTreeSet<AllocationId>,
) -> Verdict {
    let events_at_arm = events_cursor();
    let armed_at = node.elapsed;
    node.owner.sim.script_audit_damage(BTreeSet::from([affected.clone()]));
    let mut killed_at = None;
    while node.elapsed - armed_at < AUDIT_PERIOD + STEP * 2 {
        node.step().await;
        if node.admission() != Admission::Open || node.progress().is_some() {
            return Verdict::Red(format!(
                "damage while open changed admission: {:?} progress {:?} at {:?}",
                node.admission(),
                node.progress(),
                node.elapsed - armed_at
            ));
        }
        if !vm_killed_for(&events_since(events_at_arm), affected).is_empty() {
            killed_at = Some(node.elapsed);
            break;
        }
    }
    let Some(killed_at) = killed_at else {
        return Verdict::Red(format!(
            "{affected} was not killed within one audit period of the reported damage; events \
             {:?}",
            events_since(events_at_arm)
        ));
    };
    node.advance(Duration::from_secs(3)).await;
    let snapshot = node.cgroups.snapshot();
    let events = events_since(events_at_arm);
    let kills = vm_killed_for(&events, affected);
    if kills.len() != 1
        || kills[0].fields.get("cause").map(String::as_str) != Some("attachment_damaged")
    {
        return Verdict::Red(format!(
            "vm_killed events for {affected} (a killed VM is never named again): {kills:?}"
        ));
    }
    if !killed(&snapshot, &CgroupPath::for_alloc(affected)) {
        return Verdict::Red(format!("no `1\\n` at {affected}'s scope cgroup.kill: {snapshot:?}"));
    }
    if let Some(other) =
        others.iter().find(|other| touched(&snapshot, &CgroupPath::for_alloc(other)))
    {
        return Verdict::Red(format!("undamaged allocation {other} was killed: {snapshot:?}"));
    }
    if touched(&snapshot, &CgroupPath::workloads_slice()) {
        return Verdict::Red(format!("the whole workloads slice was killed: {snapshot:?}"));
    }
    if node.admission() != Admission::Open || node.progress().is_some() || node.request.is_some() {
        return Verdict::Red(format!(
            "{:?} after the kill: admission {:?} progress {:?} request {:?}",
            node.elapsed - killed_at,
            node.admission(),
            node.progress(),
            node.request
        ));
    }
    Verdict::Green
}

// ---------------------------------------------------------------------------
// C7 — late success cannot reopen after fail-stop.
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C7a — after the typed fail-stop, a component that becomes healthy again
/// cannot reopen admission, re-enter recovery, or receive further owner
/// effects.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn healed_owner_after_fail_stop_cannot_reopen_admission() {
    let mut report = Report::new("C7a-post-fail-stop-heal-cannot-reopen");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, 200 + index);
            let phase = seeded_phase(&mut rng);
            let linger = Duration::from_secs(u64::from(rng.inclusive(1, 10)));
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}/linger={linger:?}");
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(error) => error.into_unreached(),
                Ok(detected) => {
                    node.advance(RECOVERY_DEADLINE + STEP * 3).await;
                    if let Some((at, _)) = node.request.clone() {
                        let events_at_heal = events_cursor();
                        let calls_at_heal = node.call_count();
                        let intercept_at_heal = node.intercept.calls_len();
                        let responders_at_heal = node.dns.responders().len();
                        node.heal(component);
                        let mut violation = None;
                        let until = node.elapsed + linger;
                        while node.elapsed < until {
                            node.step().await;
                            if violation.is_none()
                                && (node.admission() != Admission::FailStopped
                                    || node.progress().is_some())
                            {
                                violation = Some(format!(
                                    "admission {:?} progress {:?} {:?} after healing",
                                    node.admission(),
                                    node.progress(),
                                    node.elapsed - at
                                ));
                            }
                        }
                        let after = node.runtime_calls()[calls_at_heal..].to_vec();
                        let recovered = named(
                            &events_since(events_at_heal),
                            "guest_network.shared_owner_recovered",
                        )
                        .len();
                        let effects = after
                            .iter()
                            .filter(|call| {
                                matches!(
                                    call,
                                    GuestNetworkOperation::BridgeObserve
                                        | GuestNetworkOperation::BridgeConverge
                                        | GuestNetworkOperation::TapSetDown
                                        | GuestNetworkOperation::TapSetUp
                                )
                            })
                            .count();
                        let intercept_effects = node.intercept.calls_since(intercept_at_heal);
                        let responders_built = node.dns.responders().len() - responders_at_heal;
                        violation
                            .or_else(|| {
                                (recovered != 0).then(|| {
                                    format!("{recovered} recovered events after fail-stop")
                                })
                            })
                            .or_else(|| {
                                (effects != 0)
                                    .then(|| format!("owner effects after fail-stop: {after:?}"))
                            })
                            .or_else(|| {
                                (!intercept_effects.is_empty()).then(|| {
                                    format!(
                                        "intercept effects after fail-stop: {intercept_effects:?}"
                                    )
                                })
                            })
                            .or_else(|| {
                                (responders_built != 0).then(|| {
                                    format!(
                                        "{responders_built} DNS responders built after fail-stop"
                                    )
                                })
                            })
                            .map_or(Verdict::Green, Verdict::Red)
                    } else {
                        Verdict::Unreached(format!(
                            "precondition fail-stop: no typed request {:?} after detection",
                            node.elapsed - detected.at
                        ))
                    }
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C7b — an owner read-back still in flight at the five-second deadline cannot
/// extend the window, counts as an incomplete attempt, and its later success
/// cannot reopen admission.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn in_flight_success_after_the_deadline_cannot_reopen_admission() {
    let mut report = Report::new("C7b-in-flight-late-success-cannot-reopen");
    for seed in seeds() {
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, 300 + index);
            let phase = seeded_phase(&mut rng);
            let hung_attempt = rng.inclusive(16, DEADLINE_ATTEMPTS);
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}/hung_attempt={hung_attempt}");
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(error) => error.into_unreached(),
                Ok(detected) => {
                    in_flight_episode(&mut node, component, &detected, hung_attempt).await
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

async fn in_flight_episode(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    detected: &Detected,
    hung_attempt: u32,
) -> Verdict {
    for attempt in 1..hung_attempt {
        if await_attempt(node, attempt).await.is_none() {
            return Verdict::Unreached(format!(
                "precondition cadence: attempt {attempt} never completed"
            ));
        }
    }
    let latch = node.owner.arm_audit_latch();
    let deadline = detected.at + RECOVERY_DEADLINE;
    while node.elapsed < deadline + STEP * 3 {
        node.step().await;
    }
    let hung = node.owner.audits_in_flight();
    let request = node.request.clone();
    // Heal the component and let the in-flight read-back finish successfully.
    node.heal(component);
    node.owner.disarm_audit_latch();
    latch.add_permits(64);
    let events_at_release = events_cursor();
    node.advance(Duration::from_secs(2)).await;
    if hung == 0 {
        return Verdict::Unreached(format!(
            "precondition in-flight audit: attempt {hung_attempt} issued no owner audit"
        ));
    }
    let Some((at, ServeShutdownRequest::SharedGuestNetwork(fail_stop))) = request else {
        return Verdict::Red(format!(
            "a hung owner read-back extended the window: no typed request by {:?} after detection",
            deadline + STEP * 3 - detected.at
        ));
    };
    if !within_one_step(at - detected.at, RECOVERY_DEADLINE)
        || fail_stop.component != component
        || fail_stop.cause != SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded
        || fail_stop.attempts != hung_attempt - 1
        || fail_stop.elapsed != RECOVERY_DEADLINE
    {
        return Verdict::Red(format!(
            "fail-stop {fail_stop:?} at {:?}; expected {component:?}/RecoveryDeadlineExceeded/{} attempts at {RECOVERY_DEADLINE:?}",
            at - detected.at,
            hung_attempt - 1
        ));
    }
    let recovered =
        named(&events_since(events_at_release), "guest_network.shared_owner_recovered").len();
    if node.admission() != Admission::FailStopped || node.progress().is_some() || recovered != 0 {
        return Verdict::Red(format!(
            "late in-flight success reopened: admission {:?} progress {:?} recovered_events {recovered}",
            node.admission(),
            node.progress()
        ));
    }
    Verdict::Green
}

// ---------------------------------------------------------------------------
// C8 — supervisor task loss is observed immediately and fail-stops.
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-29B — The composed server recovers every component through its required ports
/// CONTRACT_SHAPE: bounded-change.
///
/// C8 — a supervisor task loss (the panic class, induced by the owner adapter
/// panicking under the supervisor's own poll) is observed by `ServerHandle`
/// immediately, writes FailStop before the typed request returns, and carries
/// the snapshot the accepted contract fixes: no prior recovery reports
/// `Supervisor`/0/zero; in-progress recovery reports the latest remaining
/// component, completed attempts, and injected-clock elapsed.
#[tokio::test(flavor = "current_thread")]
#[ignore = "pending DELIVER step 09-01 (S-ND295-29B)"]
async fn supervisor_task_loss_is_observed_immediately_and_fail_stops_with_the_latest_snapshot() {
    let mut report = Report::new("C8-supervisor-task-loss-fail-stop");
    for seed in seeds() {
        // (a) No prior recovery: the panic fires inside the periodic audit.
        {
            let mut rng = cell_rng(seed, 400);
            let phase = seeded_phase(&mut rng);
            let mut node = Node::boot(seed).await;
            let cell = format!("panic-in-periodic-audit@phase={}ms", phase.as_millis());
            let verdict = if let Some(gap) = node.composition_gap() {
                Verdict::Unreached(gap)
            } else {
                node.advance(phase).await;
                node.owner.arm_audit_panic();
                let armed_at = node.elapsed;
                let mut panicked_at = None;
                while node.elapsed - armed_at < AUDIT_PERIOD + STEP && node.request.is_none() {
                    node.step().await;
                    if panicked_at.is_none() && node.owner.audit_panics() > 0 {
                        panicked_at = Some(node.elapsed);
                    }
                }
                match (panicked_at, node.request.clone()) {
                    (None, _) => Verdict::Unreached(format!(
                        "precondition: the supervisor invoked no owner audit within one period of arming (owner calls since boot {:?})",
                        node.runtime_calls()
                    )),
                    (Some(at), None) => Verdict::Red(format!(
                        "supervisor task loss at {:?} after arming produced no typed request",
                        at - armed_at
                    )),
                    (Some(at), Some((requested_at, request))) => {
                        let expected =
                            ServeShutdownRequest::SharedGuestNetwork(SharedGuestNetworkFailStop {
                                component: SharedGuestNetworkComponent::Supervisor,
                                cause: SharedGuestNetworkFailStopCause::SupervisorPanicked,
                                attempts: 0,
                                elapsed: Duration::ZERO,
                            });
                        if request != expected {
                            Verdict::Red(format!("request {request:?}; expected {expected:?}"))
                        } else if requested_at > at {
                            Verdict::Red(format!(
                                "task loss at {at:?} observed late at {requested_at:?}"
                            ))
                        } else if node.admission() != Admission::FailStopped {
                            Verdict::Red(format!(
                                "admission {:?} after fail-stop",
                                node.admission()
                            ))
                        } else {
                            Verdict::Green
                        }
                    }
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
        // (b) In-progress recovery for every snapshot component: the panic
        // fires inside the full audit of attempt `failed + 1`.
        for (index, component) in COMPONENTS.into_iter().enumerate() {
            let mut rng = cell_rng(seed, 500 + index);
            let phase = seeded_phase(&mut rng);
            let failed = rng.inclusive(1, 6);
            let mut node = Node::boot(seed).await;
            let cell = format!("{component:?}/panic-after-{failed}-failed-attempts");
            let verdict = match arm_and_detect(&mut node, component, phase).await {
                Err(error) => error.into_unreached(),
                Ok(detected) => {
                    in_progress_panic_episode(&mut node, component, &detected, failed).await
                }
            };
            report.record(seed, cell, verdict);
            node.shutdown().await;
        }
    }
    report.finish();
}

async fn in_progress_panic_episode(
    node: &mut Node,
    component: SharedGuestNetworkComponent,
    detected: &Detected,
    failed: u32,
) -> Verdict {
    for attempt in 1..=failed {
        if await_attempt(node, attempt).await.is_none() {
            return Verdict::Unreached(format!(
                "precondition cadence: attempt {attempt} never completed"
            ));
        }
    }
    node.owner.arm_audit_panic();
    let armed_at = node.elapsed;
    let mut panicked_at = None;
    while node.elapsed - armed_at < ATTEMPT_PERIOD + STEP && node.request.is_none() {
        node.step().await;
        if panicked_at.is_none() && node.owner.audit_panics() > 0 {
            panicked_at = Some(node.elapsed);
        }
    }
    match (panicked_at, node.request.clone()) {
        (None, _) => Verdict::Unreached(format!(
            "precondition: attempt {} issued no full owner audit",
            failed + 1
        )),
        (Some(at), None) => Verdict::Red(format!(
            "supervisor task loss {:?} after detection produced no typed request",
            at - detected.at
        )),
        (Some(at), Some((requested_at, ServeShutdownRequest::SharedGuestNetwork(fail_stop)))) => {
            let expected_elapsed = ATTEMPT_PERIOD * (failed + 1);
            if fail_stop.component != component
                || fail_stop.cause != SharedGuestNetworkFailStopCause::SupervisorPanicked
                || fail_stop.attempts != failed
                || !within_one_step(fail_stop.elapsed, expected_elapsed)
            {
                Verdict::Red(format!(
                    "request {fail_stop:?}; expected {component:?}/SupervisorPanicked/{failed} attempts/{expected_elapsed:?}"
                ))
            } else if requested_at > at {
                Verdict::Red(format!(
                    "task loss at {:?} observed late at {:?}",
                    at - detected.at,
                    requested_at - detected.at
                ))
            } else if node.admission() != Admission::FailStopped {
                Verdict::Red(format!("admission {:?} after fail-stop", node.admission()))
            } else {
                Verdict::Green
            }
        }
    }
}
