//! GH #295 — S-ND295-13D through the ordinary production composition
//! (G-295-1 rows 2 and 4; DISTILL review H6).
//!
//! The fresh-process branch of the shared mTLS owner converges the dynamic
//! member sets to the empty set before it reads the program (boot step 6.2,
//! DELIVER 08-02). These bodies boot `run_server_with_obs_and_driver` with the
//! required ports; the `MtlsIntercept` port is a test-local recording double
//! over `SimMtlsIntercept` whose model holds a killed prior process's program
//! and members. The double scripts only the port result of the boot clear
//! (`converge_allocation_elements(&InterceptMembers::default())`), and the
//! production worker, composition root, and supervisor author every refusal,
//! event, and gate state the oracles read:
//!
//! - the clear returns `Err(e)`: the boot refuses with
//!   `MtlsSharedOwnerError::BootMemberClear { source: e }`;
//! - the clear returns `Ok(Some(state))` with members left:
//!   `BootMemberClear { source: InterceptError::MembersRemain { observed } }`;
//! - the clear returns `Err(e)` and its batch lands in the double's model only
//!   after the boot returned (a late commit): nothing publishes.
//!
//! In every case `health.startup.refused` is recorded, the EXEC gate stays
//! BootClosed, no guest attachment is made, and no listener is bound, no
//! program read, and no program converged after the clear.

#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::error::{ControlPlaneError, MtlsBootError};
use overdrive_control_plane::guest_network::{GuestNetworkOperation, SharedGuestNetworkOwner};
use overdrive_control_plane::{ServerConfig, run_server_with_obs_and_driver};
use overdrive_core::guest_network::{GuestNetworkExecSupervisor, GuestNetworkExecWiring};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::mtls_intercept::SimMtlsIntercept;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_worker::mtls_intercept::{
    InterceptElementKey, InterceptElementOperation, InterceptError, InterceptPostcondition,
    InterceptSet, NetlinkError,
};
use overdrive_worker::mtls_intercept_port::{
    InterceptGuard, InterceptListener, InterceptMembers, InterceptState, MtlsIntercept,
};
use overdrive_worker::mtls_intercept_worker::MtlsSharedOwnerError;
use parking_lot::Mutex;
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

// ---------------------------------------------------------------------------
// Event capture.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct EventRow {
    name: String,
    fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldVisitor {
    fields: BTreeMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.insert(field.name().to_owned(), value.to_owned());
    }
}

#[derive(Clone, Default)]
struct EventCollector {
    inner: Arc<Mutex<Vec<EventRow>>>,
}

impl EventCollector {
    fn snapshot(&self) -> Vec<EventRow> {
        self.inner.lock().clone()
    }
}

impl<S: Subscriber> Layer<S> for EventCollector {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.inner
            .lock()
            .push(EventRow { name: event.metadata().name().to_owned(), fields: visitor.fields });
    }
}

// ---------------------------------------------------------------------------
// The recording `MtlsIntercept` double.
// ---------------------------------------------------------------------------

/// The listener type `MtlsIntercept::bind_transparent` returns. The DELIVER
/// step that carries B-7 (05-01 at the latest) changes it to
/// `Arc<dyn InterceptListener>`; the delegation below is unchanged by that
/// step.
type BoundListener = Arc<dyn InterceptListener>;

/// The killed prior process's listener targets, outside the default ephemeral
/// range so no listener of this process can share them.
const PRIOR_LEG_F: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 61_001);
const PRIOR_LEG_C: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 61_002);

/// The guest address whose members the killed prior process left behind.
const STALE_GUEST: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 9);

/// The declared Service port of the stale guest's inbound member.
const STALE_SERVICE_PORT: u16 = 8080;

/// The `op` of the boot-clear rejection the double scripts.
const BOOT_CLEAR_REJECTION_OP: &str = "nd295-boot-member-clear";

/// The dynamic members the killed prior process left behind.
fn stale_members() -> InterceptMembers {
    InterceptMembers {
        managed_guest_ips: [STALE_GUEST].into(),
        outbound_sources: [STALE_GUEST].into(),
        inbound_destinations: [SocketAddrV4::new(STALE_GUEST, STALE_SERVICE_PORT)].into(),
    }
}

/// The error a rejected boot-clear batch reports: deleting the stale guest's
/// managed-guest member was refused with `EBUSY`, and nothing changed.
fn boot_clear_rejection() -> InterceptError {
    InterceptError::NftElementUpdateFailed {
        set: InterceptSet::ManagedGuestIps,
        operation: InterceptElementOperation::Delete,
        key: InterceptElementKey::Address(STALE_GUEST),
        source: NetlinkError::nft(
            BOOT_CLEAR_REJECTION_OP,
            std::io::Error::from_raw_os_error(libc::EBUSY),
        ),
    }
}

/// Whether `error` is exactly [`boot_clear_rejection`], matched by variant and
/// every field of its cause.
fn is_boot_clear_rejection(error: &InterceptError) -> bool {
    matches!(
        error,
        InterceptError::NftElementUpdateFailed {
            set: InterceptSet::ManagedGuestIps,
            operation: InterceptElementOperation::Delete,
            key: InterceptElementKey::Address(key),
            source: NetlinkError::Nft { op, source: io, .. },
        } if *key == STALE_GUEST
            && *op == BOOT_CLEAR_REJECTION_OP
            && io.raw_os_error() == Some(libc::EBUSY)
    )
}

/// How the double answers the boot clear,
/// `converge_allocation_elements(&InterceptMembers::default())`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootClearScript {
    /// The batch is rejected: `Err(boot_clear_rejection())`, the model unchanged.
    Reject,
    /// The call returns `Ok(Some(state))` whose read-back still holds the
    /// stale members.
    LeaveMembers,
    /// The batch is rejected as in `Reject`, and the double holds it; it lands
    /// in the double's model only when [`RecordingBootIntercept::land_late_commit`]
    /// releases it, after the boot returned.
    RejectThenCommitLater,
}

/// One call the production composition made on the `MtlsIntercept` port.
#[derive(Debug, Clone, PartialEq, Eq)]
enum InterceptCall {
    BindTransparent,
    ConvergeShared,
    ObserveShared,
    InstallOutbound,
    InstallInbound,
    ObserveSharedState,
    /// `converge_allocation_elements` with the members it was asked for.
    ConvergeAllocationElements(InterceptMembers),
    RemoveAllocationElements,
}

/// A `MtlsIntercept` double whose model, a `SimMtlsIntercept`, holds the
/// program and members a killed prior process left behind. It records every
/// port call in order and scripts only the boot clear's result; every other
/// call delegates to the model.
struct RecordingBootIntercept {
    model: SimMtlsIntercept,
    /// The prior process's node guard and element guards, held for the
    /// double's lifetime: a killed process never runs a guard's `Drop`, so its
    /// program and members persist in the model (DISTILL gap B-8: a node
    /// guard's `Drop` would withdraw the record and, with no member yet,
    /// remove the program).
    _prior_guards: Vec<Box<dyn InterceptGuard>>,
    script: BootClearScript,
    journal: Mutex<Vec<InterceptCall>>,
    /// The rejected batch a `RejectThenCommitLater` clear holds.
    held_commit: Mutex<Option<InterceptMembers>>,
}

impl RecordingBootIntercept {
    fn left_by_a_prior_process(script: BootClearScript) -> Self {
        let model = SimMtlsIntercept::new();
        let program = model
            .converge_shared(None, PRIOR_LEG_F, PRIOR_LEG_C)
            .expect("the prior process converged its program");
        let outbound = model
            .install_outbound(STALE_GUEST, PRIOR_LEG_F.port())
            .expect("the prior process admitted its guest's outbound source");
        let inbound = model
            .install_inbound(SocketAddrV4::new(STALE_GUEST, STALE_SERVICE_PORT), PRIOR_LEG_C.port())
            .expect("the prior process registered its guest's inbound destination");
        let intercept = Self {
            model,
            _prior_guards: vec![program, outbound, inbound],
            script,
            journal: Mutex::new(Vec::new()),
            held_commit: Mutex::new(None),
        };
        assert_eq!(
            intercept.model_members(),
            stale_members(),
            "precondition: the model holds the prior process's members"
        );
        intercept
    }

    fn record(&self, call: InterceptCall) {
        self.journal.lock().push(call);
    }

    fn journal(&self) -> Vec<InterceptCall> {
        self.journal.lock().clone()
    }

    /// The members the model holds now.
    fn model_members(&self) -> InterceptMembers {
        self.model
            .observe_shared_state()
            .expect("the model's state is observable")
            .expect("the prior process's program is in the model")
            .members
    }

    /// Land the held batch of a `RejectThenCommitLater` clear in the model.
    /// Returns whether a batch was held.
    fn land_late_commit(&self) -> bool {
        let Some(expected) = self.held_commit.lock().take() else {
            return false;
        };
        self.model
            .converge_allocation_elements(&expected)
            .expect("the late batch lands in the model")
            .expect("the model holds a program");
        true
    }
}

impl MtlsIntercept for RecordingBootIntercept {
    fn bind_transparent(
        &self,
        addr: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<BoundListener> {
        self.record(InterceptCall::BindTransparent);
        self.model.bind_transparent(addr)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.record(InterceptCall::ConvergeShared);
        self.model.converge_shared(prior, leg_f, leg_c)
    }

    fn observe_shared(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptPostcondition>> {
        self.record(InterceptCall::ObserveShared);
        self.model.observe_shared()
    }

    fn install_outbound(
        &self,
        source_addr: Ipv4Addr,
        agent_leg_f_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.record(InterceptCall::InstallOutbound);
        self.model.install_outbound(source_addr, agent_leg_f_port)
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        agent_leg_c_port: u16,
    ) -> overdrive_worker::mtls_intercept::Result<Box<dyn InterceptGuard>> {
        self.record(InterceptCall::InstallInbound);
        self.model.install_inbound(virt, agent_leg_c_port)
    }

    fn observe_shared_state(
        &self,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptState>> {
        self.record(InterceptCall::ObserveSharedState);
        self.model.observe_shared_state()
    }

    fn converge_allocation_elements(
        &self,
        expected: &InterceptMembers,
    ) -> overdrive_worker::mtls_intercept::Result<Option<InterceptState>> {
        self.record(InterceptCall::ConvergeAllocationElements(expected.clone()));
        if *expected != InterceptMembers::default() {
            return self.model.converge_allocation_elements(expected);
        }
        match self.script {
            BootClearScript::Reject => Err(boot_clear_rejection()),
            BootClearScript::LeaveMembers => self.model.observe_shared_state(),
            BootClearScript::RejectThenCommitLater => {
                *self.held_commit.lock() = Some(expected.clone());
                Err(boot_clear_rejection())
            }
        }
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> overdrive_worker::mtls_intercept::Result<InterceptState> {
        self.record(InterceptCall::RemoveAllocationElements);
        self.model.remove_allocation_elements(source_addr, destinations)
    }
}

// ---------------------------------------------------------------------------
// The refused composed boot.
// ---------------------------------------------------------------------------

/// What one refused production boot leaves for the oracles.
struct RefusedBoot {
    error: ControlPlaneError,
    supervisor: Arc<GuestNetworkExecSupervisor>,
    owner: Arc<SimSharedGuestNetworkOwner>,
    _tmp: TempDir,
}

fn is_root() -> bool {
    // SAFETY: `geteuid` has no memory-safety preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// Boot `run_server_with_obs_and_driver` with the required ports, `intercept`
/// as the protection port, the sim DNS factory, the sim owner, a `SimDriver`,
/// and `vm_cgroups` over `SimCgroupFs`, keeping the EXEC supervisor capability
/// before the wiring moves in. A published server fails the body.
async fn refused_boot(intercept: &Arc<RecordingBootIntercept>) -> RefusedBoot {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create config dir");
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("loopback bind"),
        data_dir,
        operator_config_dir,
        dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
        dataplane_override: Some(Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new())),
        ..ServerConfig::new(
            Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
            Arc::clone(intercept) as Arc<dyn MtlsIntercept>,
            Arc::new(overdrive_sim::adapters::SimGuestDnsFactory::default()),
        )
    };
    let wiring = GuestNetworkExecWiring::new(Arc::clone(&config.clock));
    let supervisor = wiring.supervisor();
    let owner = Arc::new(SimSharedGuestNetworkOwner::default());
    let owner_port: Arc<dyn SharedGuestNetworkOwner> = owner.clone();
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("nd295-boot-member-clear").expect("node id"),
        0,
    ));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));

    let result = run_server_with_obs_and_driver(
        config,
        obs,
        driver,
        Arc::new(overdrive_sim::adapters::vm_host_state::SimVmHostState::new()),
        owner_port,
        wiring,
        overdrive_worker::cgroup_manager::CgroupManager::new(
            std::path::PathBuf::from("/sys/fs/cgroup"),
            Arc::new(overdrive_sim::adapters::SimCgroupFs::new()),
        ),
    )
    .await;
    let error = match result {
        Err(error) => error,
        Ok(handle) => {
            handle
                .shutdown(Duration::from_secs(10))
                .await
                .expect("an unexpectedly published server still shuts down");
            panic!(
                "a fresh-process boot whose member clear fails must refuse; it published a \
                 server (journal {:?})",
                intercept.journal()
            );
        }
    };
    RefusedBoot { error, supervisor, owner, _tmp: tmp }
}

/// The `InterceptError` a refused boot's `BootMemberClear` carries, through
/// the typed chain the composition root returns.
fn boot_member_clear_source(error: &ControlPlaneError) -> &InterceptError {
    match error {
        ControlPlaneError::MtlsBoot(MtlsBootError::SharedOwner {
            source: MtlsSharedOwnerError::BootMemberClear { source },
        }) => source,
        other => panic!(
            "the refusal is MtlsBoot(SharedOwner(BootMemberClear {{ source }})); got {other:?}"
        ),
    }
}

/// The oracles every refused boot shares: `health.startup.refused` recorded,
/// EXEC BootClosed, no guest attachment, the clear made, and no listener
/// bound, program read, or program converged after it.
fn assert_refused_without_publication(
    boot: &RefusedBoot,
    intercept: &RecordingBootIntercept,
    events: &[EventRow],
) {
    assert!(
        events.iter().any(|event| event.name == "health.startup.refused"),
        "the refusal records health.startup.refused; events {:?}",
        events.iter().map(|event| (&event.name, &event.fields)).collect::<Vec<_>>()
    );
    assert!(boot.supervisor.is_boot_closed(), "the EXEC gate stays BootClosed after the refusal");
    let owner_calls = boot.owner.calls();
    assert!(
        !owner_calls.iter().any(|call| {
            matches!(call, GuestNetworkOperation::TapCreate | GuestNetworkOperation::TapSetUp)
        }),
        "a refused boot makes no guest attachment: {owner_calls:?}"
    );
    let journal = intercept.journal();
    let clear = journal
        .iter()
        .position(|call| {
            matches!(
                call,
                InterceptCall::ConvergeAllocationElements(expected)
                    if *expected == InterceptMembers::default()
            )
        })
        .unwrap_or_else(|| panic!("the boot ran the member clear; journal {journal:?}"));
    let after_clear = &journal[clear + 1..];
    assert!(
        !after_clear.iter().any(|call| matches!(
            call,
            InterceptCall::BindTransparent
                | InterceptCall::ObserveShared
                | InterceptCall::ConvergeShared
        )),
        "nothing is bound, read, or converged after the failed clear; journal {journal:?}"
    );
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-13D — The fresh intercept owner refuses to start when stale members cannot be cleared.
/// CONTRACT_SHAPE: bounded-change.
///
/// G-295-1 row 2, composed: the boot clear's batch is rejected. The boot
/// refuses with `BootMemberClear` whose source is the clear's own error, and
/// the rejected clear left the prior process's members unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rejected_boot_member_clear_refuses_the_composed_boot_with_its_own_cause() {
    assert!(
        is_root(),
        "S-ND295-13D's composed bodies boot the production composition, whose always-composed \
         mTLS worker probes enforcement on the real kernel; they run as root"
    );
    let collector = EventCollector::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(collector.clone()));
    let intercept =
        Arc::new(RecordingBootIntercept::left_by_a_prior_process(BootClearScript::Reject));

    let boot = refused_boot(&intercept).await;

    let source = boot_member_clear_source(&boot.error);
    assert!(
        is_boot_clear_rejection(source),
        "BootMemberClear carries the clear's own rejection; got {source:?}"
    );
    assert_refused_without_publication(&boot, &intercept, &collector.snapshot());
    assert_eq!(intercept.model_members(), stale_members(), "the rejected clear changed no member");
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-13D — The fresh intercept owner refuses to start when stale members cannot be cleared.
/// CONTRACT_SHAPE: bounded-change.
///
/// G-295-1 row 2, composed: the boot clear returns `Ok(Some(state))` whose
/// read-back still holds members. The boot refuses with `BootMemberClear`
/// whose source is `InterceptError::MembersRemain` naming exactly those
/// members.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_boot_member_clear_that_leaves_members_refuses_with_the_members_it_observed() {
    assert!(
        is_root(),
        "S-ND295-13D's composed bodies boot the production composition, whose always-composed \
         mTLS worker probes enforcement on the real kernel; they run as root"
    );
    let collector = EventCollector::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(collector.clone()));
    let intercept =
        Arc::new(RecordingBootIntercept::left_by_a_prior_process(BootClearScript::LeaveMembers));

    let boot = refused_boot(&intercept).await;

    match boot_member_clear_source(&boot.error) {
        InterceptError::MembersRemain { observed } => assert_eq!(
            observed,
            &stale_members(),
            "MembersRemain names exactly the members the clear's read-back held"
        ),
        other => panic!("BootMemberClear's source is MembersRemain; got {other:?}"),
    }
    assert_refused_without_publication(&boot, &intercept, &collector.snapshot());
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-13D — The fresh intercept owner refuses to start when stale members cannot be cleared.
/// CONTRACT_SHAPE: bounded-change.
///
/// G-295-1 row 4, composed: the boot clear is rejected, and its batch lands in
/// the double's model only after `run_server_with_obs_and_driver` has
/// returned. The late success resurrects nothing: the gate stays BootClosed
/// and no further intercept call is made.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_boot_member_clear_that_commits_after_the_refusal_publishes_nothing() {
    assert!(
        is_root(),
        "S-ND295-13D's composed bodies boot the production composition, whose always-composed \
         mTLS worker probes enforcement on the real kernel; they run as root"
    );
    let collector = EventCollector::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(collector.clone()));
    let intercept = Arc::new(RecordingBootIntercept::left_by_a_prior_process(
        BootClearScript::RejectThenCommitLater,
    ));

    let boot = refused_boot(&intercept).await;

    let source = boot_member_clear_source(&boot.error);
    assert!(
        is_boot_clear_rejection(source),
        "BootMemberClear carries the clear's own rejection; got {source:?}"
    );
    assert_refused_without_publication(&boot, &intercept, &collector.snapshot());
    let journal_at_refusal = intercept.journal();

    // The rejected batch lands now, after the boot returned.
    assert!(intercept.land_late_commit(), "the rejected clear's batch was held for a late commit");
    assert_eq!(
        intercept.model_members(),
        InterceptMembers::default(),
        "the late commit landed in the double's model"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(boot.supervisor.is_boot_closed(), "the late commit leaves the EXEC gate BootClosed");
    assert_eq!(
        intercept.journal(),
        journal_at_refusal,
        "nothing the refused boot left behind calls the intercept after the late commit"
    );
    let owner_calls = boot.owner.calls();
    assert!(
        !owner_calls.iter().any(|call| {
            matches!(call, GuestNetworkOperation::TapCreate | GuestNetworkOperation::TapSetUp)
        }),
        "the late commit admits no guest attachment: {owner_calls:?}"
    );
}
