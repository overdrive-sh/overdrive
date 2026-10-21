//! GH #295 — runtime recovery under counted quiescence (QHOLD, ADR-0169),
//! the bounded recovery and fail-stop of ADR-0124, the `Forwarder`
//! component recovery (EVMOD-CTL) and the failed boot write of the shared
//! mTLS table (ADR-0184). Scenarios P6-S1..S6, C2, R5-32 (QHOLD, the leaked
//! hold, the foreign hold, D-UP-4's leak at the issuing ledger), R5-58 (the
//! supervisor's audit detection and attempt counting, seeded), R5-61 (the
//! `Forwarder` recovery order, seeded half), R5-30 / ADR-0184 and the
//! supervisor's `ForeignHold` → `ForeignQuiescenceHold` mapping
//! (FHOLD-CHECK). The Quint `quiescence` conformance driver is
//! `crate::quint_quiescence`.
//!
//! Every body drives the production composition through the composed node
//! rig (`crate::shared_network_test_ports::composed`): boot through
//! `run_server_with_obs_and_drivers`, the production guest-flow owner over
//! `SimGuestFlowIo` (its forwarder calls on `SimFlowForwarder`), the
//! production runtime shared-network supervisor, and `SimMtlsIntercept` at
//! the shared table. Faults enter only at driven ports (a component's accept
//! task exits at the owner seam; the forwarder context fails `Io`; a
//! repair's listener rebind fails; boot step 8's table transaction fails; the
//! process is dropped) or through the owner's own driving port for the two
//! defect stimuli (`quiesce()` with its hold dropped, D-UP-3; `restore` of a
//! second ledger's hold, D-UP-4). No body seeds a hold, a latch, an episode
//! or a table state. The FHOLD-CHECK cell composes the same production
//! supervisor over the test-local `TestSharedOwner` at the owner port — the
//! existing driven port, not a new seam.
//!
//! # Contract / invariant, owner path, oracle (DISTILL § composed behaviour)
//!
//! | Scenario | Contract | Fault stimulus | Oracle | Tier |
//! |---|---|---|---|---|
//! | P6-S1 | one recovery quiesces (first hold latches, aborts flows), repairs, restores; reopen at count zero; aborted flows are not restored | `FlowListeners` accept task exits | O3 `quiescence_holds`, `admission`, held flows, CID phase and session; O6 episodes; S table; `dial` → `Quiesced` | seeded sim |
//! | P6-S2 / R5-32 | two overlapping recoveries: the first restore leaves forwarding quiesced (`StillQuiesced`, no holder list); reopen only when the last hold is restored; no flow opens and a dial answers `Quiesced` while a hold remains | both listener tasks exit | as P6-S1, plus the refused guest flow | seeded sim |
//! | R5-32 (leak) | a hold dropped unredeemed keeps forwarding quiesced and the supervisor fail-stops `LeakedQuiescenceHold`; the restart converges | the composed owner's `quiesce()`, its hold dropped (D-UP-3) | O3 `leaked_quiescence_holds`, `quiescence_holds`, `admission`; the shutdown request's cause | seeded sim (+ Quint `DroppedHoldKeepsForwardingQuiescedThenFailStops`) |
//! | R5-32 (foreign) / D-UP-4 | a restore of another ledger's hold returns `ForeignHold` and changes nothing at the owner; the hold, dropped unredeemed, counts leaked at its issuing ledger (+1) and nowhere else | a second `QuiescenceLedger`'s hold restored at the owner | the `restore` result; the second ledger's `leaked()`; O3 counts and admission | seeded sim (the model-only D-UP-4 claim's discharge) |
//! | FHOLD-CHECK | the supervisor maps `restore` → `Err(ForeignHold)` to the fail-stop `ForeignQuiescenceHold` at once | a test-local owner at the owner port answering `ForeignHold` | the shutdown request's cause | sim unit cell (+ Quint `quiescence-foreign-hold-ends-in-fail-stop`) |
//! | R5-58 | `AuditDetectsComponentExitWithinOneTick`, `RepairErrIsOneFailedAttempt`, `EpisodeEndsByRestoreOrFailStop` (≤ 3 components, ≤ 10 s of sim time; seed printed, triggering sequence retained) | random-instant task exits; repairs scripted `Err` k times; a failed post-repair audit | O6 episodes, O3 holds and leaked count, the shutdown request | seeded sim |
//! | R5-61 (seeded half) | the `Forwarder` recovery aborts every flow, closes the old context and only then opens and probes a new one; `forwarder_generation` advances by one | the forwarder context fails `Io` | the seam's issued order (`AbortAll` → `CloseForwarder` → `OpenForwarder`), O3 held flows at the reopen, O3 `forwarder_generation` | seeded sim (+ Tier-3 far-end resets, + Quint `ForwarderRecoveryAbortsThenReopens`) |
//! | G-V1 (boot step 1) | the forwarder context is opened and probed at `probe_startup`, before any listener bind or the gateway route; a failed open or probe refuses boot with its typed cause and `health.startup.refused` reason `guest_network.probe`, before any boot effect | the boot's `open_forwarder` scripted `AbiMismatch` / `DeviceAbsent` | the typed `GuestNetworkBoot(Forwarder)` refusal and reason; the seam's issued order; O3 `forwarder_generation` = 1 after a good boot | seeded sim (+ Tier-3 R5-61 module probe) |
//! | R5-11 (seeded half) | every boot — fresh and after a crash — converges the gateway route once in `converge_shared` and serves; no runtime recovery writes it | the process is dropped, then rebooted; a listener recovery | the seam's `GatewayRoute` effects per boot and per recovery | seeded sim (+ Tier-3 R5-11 keep / replace / add) |
//! | P6-S3 / C2 / R5-30 | a failed boot write of the shared table refuses boot before forwarding opens; the table is left as it was; the next boot converges | step 8's transaction fails (fresh node and after a crash) | the typed `MtlsBoot(SharedOwner)` refusal, `health.startup.refused` reason `mtls.shared_owner`, S table | seeded sim (+ R5-30 Tier-3 pair) |
//! | P6-S4 | activation waits for quiescence to end (`QuiescenceLatched` changes nothing), then activates | a recovery holds while an allocation is provisioned | `activate` results; O3 CID phase | seeded sim |
//! | P6-S5 | a spent recovery budget fail-stops `serve`; forwarding never reopens before; the table stays as booted; the restart converges | every repair of `FlowListeners` fails | the `ServeShutdownRequest::SharedGuestNetwork` request; O3 admission; S table | seeded sim (+ V-8 native, DELIVER 10-02) |
//! | P6-S6 | a crash during a recovery ends its process-scoped hold; the restart converges with forwarding open and the table kept | the process is dropped mid-recovery | O3 after the restart; S table | seeded sim |

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::cast_possible_truncation,
    reason = "acceptance bodies fail at once on a broken precondition, naming it"
)]

use std::collections::BTreeMap;
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::guest_network::{
    ServeShutdownRequest, SharedGuestNetworkComponent, SharedGuestNetworkFailStopCause,
};
use overdrive_core::guest_transport::dial::{DialPurpose, GuestDialError, GuestStreamDial};
use overdrive_core::guest_transport::forwarder::{ForwarderError, ForwarderGeneration};
use overdrive_sim::adapters::{SimInterceptFault, SimSharedTable};
use parking_lot::Mutex;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::error::{ControlPlaneError, MtlsBootError};
use crate::guest_network::flow_io::sim::{SimEffectKind, SimFailure, SimForwarderFailure};
use crate::guest_network::{
    CidPhase, ForwardingActivation, GuestAdmission, GuestNetworkError, GuestNetworkProvisioner,
    GuestOwnerObservation, QuiescenceRestoreError, SharedGuestNetworkOwner as _,
};
use crate::shared_network_test_ports::TestSharedOwner;
use crate::shared_network_test_ports::composed::{
    C1, C2, ComposedNode, LISTEN_PORT, NodeStop, TestOwnerProcess,
};
use crate::{
    RecoveryEpisode, RecoveryPhase, SHARED_NETWORK_AUDIT_PERIOD, SHARED_NETWORK_RECOVERY_DEADLINE,
    SHARED_NETWORK_RETRY_PERIOD,
};

// ---------------------------------------------------------------------------
// Observation helpers (port-exposed universe only)
// ---------------------------------------------------------------------------

/// The port-exposed quiescence universe of one node at a consistency point.
#[derive(Debug, Clone, PartialEq, Eq)]
struct QuiescenceUniverse {
    holds: u32,
    leaked: u32,
    admission: GuestAdmission,
    flows: usize,
    episodes: BTreeMap<SharedGuestNetworkComponent, RecoveryEpisode>,
    table: SimSharedTable,
}

async fn universe(node: &ComposedNode) -> QuiescenceUniverse {
    let observed: GuestOwnerObservation = node.owner_observation().await;
    QuiescenceUniverse {
        holds: observed.quiescence_holds,
        leaked: observed.leaked_quiescence_holds,
        admission: observed.admission,
        flows: flows_of(&observed),
        episodes: node.recovery_observation(),
        table: node.table(),
    }
}

fn flows_of(observed: &GuestOwnerObservation) -> usize {
    observed
        .cids
        .values()
        .map(|cid| cid.guest_flows.len() + cid.dials.len() + cid.datagrams.len())
        .sum()
}

const fn held(failed_attempts: u32) -> RecoveryEpisode {
    RecoveryEpisode { phase: RecoveryPhase::Held, failed_attempts }
}

/// Open, nothing held, nothing leaked, the table complete.
fn open_universe(flows: usize) -> QuiescenceUniverse {
    QuiescenceUniverse {
        holds: 0,
        leaked: 0,
        admission: GuestAdmission::Open,
        flows,
        episodes: BTreeMap::new(),
        table: SimSharedTable::Complete,
    }
}

/// The Active allocation's (CID phase, control-session holder), the facts
/// quiescence must leave unchanged.
async fn cid_facts(node: &ComposedNode) -> BTreeMap<String, (CidPhase, bool)> {
    node.owner_observation()
        .await
        .cids
        .iter()
        .map(|(cid, observed)| (format!("{cid:?}"), (observed.phase, observed.session.is_some())))
        .collect()
}

/// A dial of the rig's Active allocation through the owner's driving port.
async fn dial(node: &ComposedNode) -> Result<(), GuestDialError> {
    let plan = node.active_plan().expect("an Active allocation");
    let target = SocketAddrV4::new(plan.assignment().workload_addr(), LISTEN_PORT);
    node.process().owner.dial(target, DialPurpose::Relay).await.map(drop)
}

/// One audit tick: the detecting audit runs and is released.
async fn audit_tick(node: &ComposedNode) {
    node.advance(SHARED_NETWORK_AUDIT_PERIOD).await;
    node.release_audits().await;
}

/// One retry tick, then `component`'s parked repair returns `ok`.
async fn repair(node: &ComposedNode, component: SharedGuestNetworkComponent, ok: bool) {
    if node.held_repair(component).is_none() {
        node.advance(SHARED_NETWORK_RETRY_PERIOD).await;
    }
    node.decide_repair(component, ok).await;
}

// ---------------------------------------------------------------------------
// Boot-refusal event capture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RefusalEvent {
    reason: Option<String>,
}

#[derive(Default)]
struct Fields(BTreeMap<String, String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

#[derive(Clone, Default)]
struct RefusalCollector(Arc<Mutex<Vec<RefusalEvent>>>);

impl<S: Subscriber> Layer<S> for RefusalCollector {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if event.metadata().name() != "health.startup.refused" {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.0.lock().push(RefusalEvent { reason: fields.0.get("reason").cloned() });
    }
}

impl RefusalCollector {
    fn reasons(&self) -> Vec<Option<String>> {
        self.0.lock().iter().map(|event| event.reason.clone()).collect()
    }
}

fn assert_step8_refusal(result: Result<(), ControlPlaneError>) {
    match result {
        Err(ControlPlaneError::MtlsBoot(MtlsBootError::SharedOwner { .. })) => {}
        other => {
            panic!("boot step 8's failed write refuses with MtlsBoot(SharedOwner), got {other:?}")
        }
    }
}

const fn failed_table_write() -> SimInterceptFault {
    SimInterceptFault::NftRuleInstall { op: "shared-table-transaction", errno: libc::EBUSY }
}

// ---------------------------------------------------------------------------
// P6-S1 — one recovery quiesces, repairs, restores
// ---------------------------------------------------------------------------

/// P6-S1 — A listener recovery quiesces forwarding under its own hold,
/// repairs, and reopens only when its hold is restored.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a booted node with an Active allocation whose control session is
/// live and one paired guest-opened flow, forwarding open (no hold).
/// When the flow listeners' accept task exits and the supervisor's audit tick
/// passes, then the recovery holds one hold: admission is `Quiesced`, the
/// flow is aborted, a dial answers `Quiesced`, and the CID stays Active with
/// its session (the CID table and control sessions are outside the quiesce
/// universe). When the repair returns `Ok` and the post-repair audit is
/// clean, then the hold is restored at count zero: admission `Open`, no
/// episode, no leaked hold — and the aborted flow is not resurrected, while
/// a new guest flow pairs.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn a_listener_recovery_quiesces_under_its_hold_and_reopens_at_count_zero() {
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");
    node.open_guest_flow().await;
    assert_eq!(universe(&node).await, open_universe(1), "precondition: open, one paired flow");
    let cids_before = cid_facts(&node).await;

    node.damage(C1).await;
    audit_tick(&node).await;

    assert_eq!(
        universe(&node).await,
        QuiescenceUniverse {
            holds: 1,
            leaked: 0,
            admission: GuestAdmission::Quiesced,
            flows: 0,
            episodes: BTreeMap::from([(C1, held(0))]),
            table: SimSharedTable::Complete,
        },
        "the first hold latches admission and aborts every flow"
    );
    assert_eq!(cid_facts(&node).await, cids_before, "the CID table and its session are unchanged");
    assert_eq!(dial(&node).await, Err(GuestDialError::Quiesced), "a dial is refused Quiesced");
    assert_eq!(universe(&node).await.holds, 1, "a refused dial changes no quiescence state");

    repair(&node, C1, true).await;
    assert_eq!(
        node.recovery_observation(),
        BTreeMap::from([(
            C1,
            RecoveryEpisode { phase: RecoveryPhase::Repaired, failed_attempts: 0 }
        )]),
        "the repair returned; the post-repair audit is pending under the hold"
    );
    assert_eq!(
        node.owner_observation().await.admission,
        GuestAdmission::Quiesced,
        "nothing reopens before the clean audit"
    );

    node.release_audits().await;
    assert_eq!(
        universe(&node).await,
        open_universe(0),
        "restore at zero reopens; flows stay aborted"
    );
    assert_eq!(cid_facts(&node).await, cids_before, "the CID kept serving through the recovery");

    node.open_guest_flow().await;
    assert_eq!(universe(&node).await.flows, 1, "a new guest flow pairs after the reopen");
}

// ---------------------------------------------------------------------------
// P6-S2 — two overlapping recoveries
// ---------------------------------------------------------------------------

/// P6-S2 / R5-32 — Two overlapping recoveries keep forwarding quiesced until
/// the last hold is restored (the first restore is `StillQuiesced`, no
/// holder list).
/// CONTRACT_SHAPE: bounded-change.
///
/// Given both the flow listeners' and the control listener's accept tasks
/// exited and both recoveries quiesced (two holds, admission `Quiesced`).
/// When the flow-listener recovery repairs and restores first, then one hold
/// remains: admission stays `Quiesced`, a dial still answers `Quiesced`, and a
/// guest flow delivered now is not admitted. When the control-listener
/// recovery repairs and restores, then the count reaches zero and admission
/// reopens; a guest flow then pairs.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn overlapping_recoveries_reopen_only_when_the_last_hold_is_restored() {
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");
    node.open_guest_flow().await;

    node.damage(C1).await;
    node.damage(C2).await;
    audit_tick(&node).await;
    audit_tick(&node).await;
    assert_eq!(
        universe(&node).await,
        QuiescenceUniverse {
            holds: 2,
            leaked: 0,
            admission: GuestAdmission::Quiesced,
            flows: 0,
            episodes: BTreeMap::from([(C1, held(0)), (C2, held(0))]),
            table: SimSharedTable::Complete,
        },
        "each recovery holds its own hold"
    );

    repair(&node, C1, true).await;
    node.release_audits().await;
    assert_eq!(
        universe(&node).await,
        QuiescenceUniverse {
            holds: 1,
            leaked: 0,
            admission: GuestAdmission::Quiesced,
            flows: 0,
            episodes: BTreeMap::from([(C2, held(0))]),
            table: SimSharedTable::Complete,
        },
        "the first restore leaves the count above zero and reopens nothing"
    );
    assert_eq!(dial(&node).await, Err(GuestDialError::Quiesced));
    node.open_guest_flow().await;
    assert_eq!(universe(&node).await.flows, 0, "no flow is admitted while a hold remains");

    repair(&node, C2, true).await;
    node.release_audits().await;
    assert_eq!(universe(&node).await, open_universe(0), "the last restore reopens at zero");
    node.open_guest_flow().await;
    assert_eq!(universe(&node).await.flows, 1, "a guest flow pairs once forwarding is open");
}

// ---------------------------------------------------------------------------
// P6-S3 / C2 — a failed boot write of the shared table refuses boot
// ---------------------------------------------------------------------------

/// P6-S3 — A failed boot write of the shared mTLS table refuses boot and
/// leaves the table as it was; the next boot converges (ADR-0184).
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a node that has never booted (no shared table).
/// When boot step 8's single table transaction fails, then `run_server`
/// returns `MtlsBoot(SharedOwner)` with `health.startup.refused` reason
/// `mtls.shared_owner`, no process runs, and the table is still absent (no
/// partial table). When it fails again, the same. When the next boot's write
/// succeeds, then the node runs with the table complete, forwarding open and
/// no hold.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER step 12"]
async fn a_failed_boot_table_write_refuses_boot_and_the_retry_converges() {
    let collector = RefusalCollector::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(collector.clone()));
    let mut node = ComposedNode::fresh();
    assert_eq!(node.table(), SimSharedTable::Absent, "precondition: no table");

    for attempt in 1..=2 {
        node.intercept.fail_next_table_transaction(failed_table_write());
        assert_step8_refusal(node.boot().await);
        assert!(node.process.is_none(), "attempt {attempt}: no serve process runs");
        assert_eq!(node.last_stop, Some(NodeStop::Refused));
        assert_eq!(
            node.table(),
            SimSharedTable::Absent,
            "attempt {attempt}: the table is as it was"
        );
    }
    assert_eq!(
        collector.reasons(),
        vec![Some("mtls.shared_owner".to_owned()); 2],
        "each refusal is one health.startup.refused naming boot step 8"
    );

    node.boot().await.expect("the retry's write succeeds");
    assert_eq!(universe(&node).await, open_universe(0), "serve runs only with the table written");
}

/// C2 — The shared table is only ever absent or complete across boots,
/// crashes and failed writes; a failed write over a complete table leaves it
/// complete and still refuses boot.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a running node (table complete). When the process crashes, then the
/// table stays complete (the kernel keeps it). When the next boot's step-8
/// write fails, then boot is refused and the table is still complete. When
/// the following boot's write succeeds, then the node runs with forwarding
/// open.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER step 12"]
async fn the_shared_table_is_absent_or_complete_across_crash_and_failed_write() {
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");
    assert_eq!(node.table(), SimSharedTable::Complete);

    node.crash().await;
    assert_eq!(node.table(), SimSharedTable::Complete, "a crash leaves the kernel's table");

    node.intercept.fail_next_table_transaction(failed_table_write());
    assert_step8_refusal(node.boot().await);
    assert!(node.process.is_none(), "a refused boot runs no process");
    assert_eq!(
        node.table(),
        SimSharedTable::Complete,
        "the failed write leaves the table as it was"
    );

    node.boot().await.expect("the retry converges");
    assert_eq!(universe(&node).await, open_universe(0));
}

// ---------------------------------------------------------------------------
// P6-S4 — activation waits for quiescence to end
// ---------------------------------------------------------------------------

/// P6-S4 — Activation of a provisioned allocation waits while a recovery
/// holds quiescence and activates once the hold is restored.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a recovery holding quiescence and a newly provisioned allocation
/// whose control session is live (provisioning and the session open are not
/// quiescence-gated). When it is activated, then the result is
/// `QuiescenceLatched` and its CID stays Provisioned. When the recovery
/// restores, then a retried activation returns `Activated`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn activation_waits_for_quiescence_to_end_then_activates() {
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");
    node.damage(C1).await;
    audit_tick(&node).await;
    assert_eq!(universe(&node).await.holds, 1, "precondition: one recovery holds");

    let plan = node
        .pool
        .assign(
            overdrive_core::id::AllocationId::new("nd295-p6-s4").expect("allocation id"),
            &[overdrive_core::net::ListenPort::new(LISTEN_PORT).expect("listen port")],
            node.process().owner.flow_ports().expect("the booted owner reports its bound ports"),
        )
        .expect("a lease");
    node.provision_with_session(&plan).await;
    let owner = Arc::clone(&node.process().owner);

    assert_eq!(
        owner.activate(&plan).await.expect("activation returns a waiting result"),
        ForwardingActivation::QuiescenceLatched,
    );
    let phase =
        node.owner_observation().await.cids.get(&plan.assignment().cid()).map(|cid| cid.phase);
    assert_eq!(phase, Some(CidPhase::Provisioned), "a latched activation changes nothing");

    repair(&node, C1, true).await;
    node.release_audits().await;
    assert_eq!(universe(&node).await.holds, 0);
    assert_eq!(
        owner.activate(&plan).await.expect("the retry is answered"),
        ForwardingActivation::Activated,
        "after the restore the activation no longer waits"
    );
}

// ---------------------------------------------------------------------------
// P6-S5 — a spent budget fail-stops serve
// ---------------------------------------------------------------------------

/// P6-S5 — A recovery whose repair fails for the whole bounded window
/// fail-stops `serve`; forwarding never reopens before; the shared table
/// stays as booted; the restart converges.
/// CONTRACT_SHAPE: unbounded-preservation.
///
/// Given a damaged flow-listener component whose every repair fails.
/// When the recovery bound passes, then the supervisor requests
/// `ServeShutdownRequest::SharedGuestNetwork` with cause
/// `RecoveryDeadlineExceeded`; at every retry tick before it admission was
/// `Quiesced`; the table is complete throughout. When the node restarts,
/// then forwarding is open with no hold.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn a_spent_recovery_budget_fail_stops_and_the_restart_converges() {
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");
    node.damage(C1).await;
    audit_tick(&node).await;

    let mut request = None;
    let ticks =
        SHARED_NETWORK_RECOVERY_DEADLINE.as_millis() / SHARED_NETWORK_RETRY_PERIOD.as_millis();
    for tick in 0..=ticks {
        if let Some(made) = node.fail_stop_requested() {
            request = Some(made);
            break;
        }
        assert_eq!(
            node.owner_observation().await.admission,
            GuestAdmission::Quiesced,
            "retry tick {tick}: forwarding never reopens before the fail-stop"
        );
        assert_eq!(node.table(), SimSharedTable::Complete, "retry tick {tick}: the table stays");
        if node.held_repair(C1).is_none() {
            node.advance(SHARED_NETWORK_RETRY_PERIOD).await;
        }
        if node.held_repair(C1).is_some() {
            node.decide_repair(C1, false).await;
        }
    }
    let request =
        request.or_else(|| node.fail_stop_requested()).expect("the bound fail-stops serve");
    match request {
        ServeShutdownRequest::SharedGuestNetwork(fail_stop) => {
            assert_eq!(
                fail_stop.cause,
                SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded,
                "the typed fail-stop names the spent bound"
            );
            assert_eq!(fail_stop.component, C1, "the fail-stop names the unrecovered component");
        }
    }
    node.end_after_fail_stop().await;
    assert_eq!(node.table(), SimSharedTable::Complete, "the fail-stop leaves the table as booted");

    node.boot().await.expect("the restart converges");
    assert_eq!(universe(&node).await, open_universe(0));
}

// ---------------------------------------------------------------------------
// P6-S6 — a crash during a recovery
// ---------------------------------------------------------------------------

/// P6-S6 — A crash during a recovery ends its process-scoped hold; the
/// restart converges with forwarding open and the table kept.
/// CONTRACT_SHAPE: unbounded-preservation.
///
/// Given a recovery holding quiescence with one flow aborted.
/// When the `serve` process crashes, then the table stays complete. When the
/// node restarts, then the new owner holds no hold, admission is open, no
/// flow of the dead process is held, and a guest flow pairs.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn a_crash_during_a_recovery_ends_its_hold_and_the_restart_converges() {
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");
    node.open_guest_flow().await;
    node.damage(C1).await;
    audit_tick(&node).await;
    assert_eq!(universe(&node).await.holds, 1, "precondition: the recovery holds");

    node.crash().await;
    assert_eq!(node.table(), SimSharedTable::Complete, "the kernel keeps the table");

    node.boot().await.expect("the restart converges");
    assert_eq!(
        universe(&node).await,
        open_universe(0),
        "process-scoped holds ended with the process"
    );
    node.open_guest_flow().await;
    assert_eq!(universe(&node).await.flows, 1);
}

// ---------------------------------------------------------------------------
// R5-32 — a leaked hold keeps forwarding quiesced, then fail-stops
// ---------------------------------------------------------------------------

/// R5-32 (D-UP-3) — A hold dropped without `restore` keeps forwarding
/// quiesced, the leaked count rises, and the supervisor fail-stops `serve`
/// with `LeakedQuiescenceHold` at its next audit tick; the restart converges
/// with forwarding open.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a booted node with one paired guest flow and a listener recovery
/// holding quiescence (one hold). When the composed owner's `quiesce()` hands
/// out a hold that is dropped unredeemed (the `LeakHold` stimulus), then the
/// count is 2, the leaked count 1, admission `Quiesced`. When the listener
/// recovery repairs and restores, then one hold remains (the leaked one),
/// admission stays `Quiesced` and no episode remains. At the next audit tick
/// the supervisor requests `ServeShutdownRequest::SharedGuestNetwork` with
/// cause `LeakedQuiescenceHold`. After the restart, forwarding is open with no
/// hold, no leak, and a guest flow pairs.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn a_leaked_hold_keeps_forwarding_quiesced_until_the_supervisor_fail_stops() {
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");
    node.open_guest_flow().await;
    node.damage(C1).await;
    audit_tick(&node).await;

    node.leak_hold().await;
    assert_eq!(
        universe(&node).await,
        QuiescenceUniverse {
            holds: 2,
            leaked: 1,
            admission: GuestAdmission::Quiesced,
            flows: 0,
            episodes: BTreeMap::from([(C1, held(0))]),
            table: SimSharedTable::Complete,
        },
        "a dropped hold keeps its count and is counted leaked; it is no episode"
    );

    repair(&node, C1, true).await;
    node.release_audits().await;
    let after_restore = universe(&node).await;
    assert_eq!(
        (after_restore.holds, after_restore.leaked, after_restore.admission),
        (1, 1, GuestAdmission::Quiesced),
        "the recovery's restore leaves the leaked hold: forwarding stays quiesced"
    );
    assert!(after_restore.episodes.is_empty(), "the listener recovery ended by its restore");
    assert_eq!(dial(&node).await, Err(GuestDialError::Quiesced));

    audit_tick(&node).await;
    match node.fail_stop_requested().expect("a leaked hold fail-stops at the audit tick") {
        ServeShutdownRequest::SharedGuestNetwork(fail_stop) => assert_eq!(
            fail_stop.cause,
            SharedGuestNetworkFailStopCause::LeakedQuiescenceHold,
            "the typed fail-stop names the leaked hold"
        ),
    }
    node.end_after_fail_stop().await;

    node.boot().await.expect("the restart converges");
    assert_eq!(
        universe(&node).await,
        open_universe(0),
        "a new process starts with no hold or leak"
    );
    node.open_guest_flow().await;
    assert_eq!(universe(&node).await.flows, 1, "forwarding opens after the restart");
}

// ---------------------------------------------------------------------------
// R5-32 / D-UP-4 — a foreign hold changes nothing and leaks at its ledger
// ---------------------------------------------------------------------------

/// R5-32 / D-UP-4 (PD-1, PD-2) — A `restore` of a hold another
/// `QuiescenceLedger` issued returns `ForeignHold` and changes nothing at the
/// owner; the hold, dropped unredeemed, is counted leaked at its issuing
/// ledger only.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a booted node, first with forwarding open (no hold), then with one
/// listener recovery holding quiescence. When a hold issued by a second
/// ledger is restored at the composed owner, then the answer is
/// `Err(ForeignHold)` each time; the owner's hold count, admission, leaked
/// count and episodes are as before; the second ledger's `leaked()` rises by
/// exactly one per foreign hold. (Model-only claim of `quiescence.qnt`
/// discharged here: no variable of the model represents the second ledger.)
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn a_foreign_hold_changes_nothing_at_the_owner_and_leaks_at_its_issuing_ledger() {
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");

    let open = universe(&node).await;
    let (restored, issuing) = node.restore_foreign_hold().await;
    assert_eq!(restored, Err(QuiescenceRestoreError::ForeignHold), "a foreign hold is refused");
    assert_eq!(universe(&node).await, open, "nothing at the owner changes while open");
    assert_eq!(issuing.leaked(), 1, "the dropped foreign hold is leaked at its issuing ledger");

    node.damage(C1).await;
    audit_tick(&node).await;
    let quiesced = universe(&node).await;
    assert_eq!((quiesced.holds, quiesced.leaked), (1, 0), "precondition: one recovery holds");

    let (restored, issuing) = node.restore_foreign_hold().await;
    assert_eq!(restored, Err(QuiescenceRestoreError::ForeignHold));
    assert_eq!(
        universe(&node).await,
        quiesced,
        "the count, the latch, the leaked count and the recovery's episode are unchanged"
    );
    assert_eq!(issuing.leaked(), 1, "exactly one leak, at the second ledger");
    assert_eq!(
        node.owner_observation().await.leaked_quiescence_holds,
        0,
        "the owner's ledger counts no leak for a hold it did not issue"
    );
}

// ---------------------------------------------------------------------------
// FHOLD-CHECK — the supervisor's ForeignHold mapping
// ---------------------------------------------------------------------------

/// FHOLD-CHECK (user ruling 2026-10-15; PD-1) — When the supervisor's
/// `restore` after a clean repair returns `Err(ForeignHold)`, the supervisor
/// fail-stops `serve` at once with cause `ForeignQuiescenceHold`.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given the production composition over a test-local owner at the owner
/// port whose `restore` answers `ForeignHold` (the composition defect, no
/// seam added) and a `FlowListeners` audit failure scripted once. When the
/// supervisor's audit tick detects it, quiesces, repairs, audits clean and
/// restores, then the next shutdown request is
/// `ServeShutdownRequest::SharedGuestNetwork` with cause
/// `ForeignQuiescenceHold`, naming the component, and the owner's hold count
/// is unchanged by the refused restore (forwarding stays quiesced until the
/// process ends).
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn a_restore_answered_foreign_hold_fail_stops_with_foreign_quiescence_hold() {
    let owner = Arc::new(TestSharedOwner::new());
    owner.script_foreign_restores(true);
    owner.script_audit_failures([C1]);
    let mut process = TestOwnerProcess::boot(Arc::clone(&owner)).await;

    let mut request = None;
    for _ in 0..8 {
        process.advance(SHARED_NETWORK_AUDIT_PERIOD).await;
        if let Some(made) = process.fail_stop_requested() {
            request = Some(made);
            break;
        }
    }
    let quiesced = owner.journal().iter().any(|entry| {
        matches!(entry.call, crate::shared_network_test_ports::TestOwnerCall::Quiesce { .. })
    });
    assert!(quiesced, "precondition: the recovery quiesced");
    match request.expect("the refused restore fail-stops serve") {
        ServeShutdownRequest::SharedGuestNetwork(fail_stop) => {
            assert_eq!(
                fail_stop.cause,
                SharedGuestNetworkFailStopCause::ForeignQuiescenceHold,
                "ForeignHold maps to the ForeignQuiescenceHold fail-stop"
            );
            assert_eq!(fail_stop.component, C1);
        }
    }
    assert_eq!(owner.quiescence_holds(), 1, "the refused restore changed no count");
    assert_eq!(owner.leaked_quiescence_holds(), 0, "the refused hold is kept, not leaked");
}

// ---------------------------------------------------------------------------
// R5-58 — audit detection and attempt counting (seeded)
// ---------------------------------------------------------------------------

/// A small deterministic generator for the seeded cells (the seed is
/// printed; a failing seed reproduces with `OVERDRIVE_R5_58_SEED`).
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

fn seeds() -> Vec<u64> {
    std::env::var("OVERDRIVE_R5_58_SEED").ok().and_then(|seed| seed.parse().ok()).map_or_else(
        || (1..=8).map(|n: u64| n.wrapping_mul(0x9E37_79B9_7F4A_7C15)).collect(),
        |seed| vec![seed],
    )
}

/// The components a run damages (at most 3, R5-58's bound).
const R5_58_COMPONENTS: [SharedGuestNetworkComponent; 3] =
    [C1, C2, SharedGuestNetworkComponent::Forwarder];

async fn damage_component(node: &ComposedNode, component: SharedGuestNetworkComponent) {
    if component == SharedGuestNetworkComponent::Forwarder {
        node.damage_forwarder().await;
    } else {
        node.damage(component).await;
    }
}

/// R5-58 `AuditDetectsComponentExitWithinOneTick` — every component task
/// exit, at a random instant of an audit period, is reported by the audit
/// within one `SimClock` audit period of the exit: its recovery's episode
/// exists, `Held`, under one hold of its own.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
#[allow(clippy::print_stderr, reason = "the seed is printed so a failure reproduces")]
async fn the_audit_detects_every_component_exit_within_one_audit_period() {
    for seed in seeds() {
        eprintln!("R5-58 AuditDetectsComponentExitWithinOneTick seed={seed}");
        let mut rng = Seeded(seed | 1);
        let mut node = ComposedNode::fresh();
        node.boot().await.expect("the node boots");
        let damaged = 1 + rng.below(3) as usize;
        let mut sequence = Vec::new();
        for component in R5_58_COMPONENTS.iter().copied().take(damaged) {
            let offset =
                Duration::from_millis(rng.below(SHARED_NETWORK_AUDIT_PERIOD.as_millis() as u64));
            node.advance(offset).await;
            damage_component(&node, component).await;
            sequence.push((offset, component));
            node.advance(SHARED_NETWORK_AUDIT_PERIOD).await;
            node.release_audits().await;
            assert!(
                node.recovery_observation()
                    .get(&component)
                    .is_some_and(|episode| episode.phase == RecoveryPhase::Held),
                "seed={seed} sequence={sequence:?}: {component:?} is detected within one audit period"
            );
        }
        let observed = node.owner_observation().await;
        assert_eq!(
            observed.quiescence_holds as usize,
            node.recovery_observation().len(),
            "seed={seed} sequence={sequence:?}: one hold per recovery (CountCoversRecoveries)"
        );
        assert_eq!(observed.leaked_quiescence_holds, 0, "seed={seed}: no hold leaked");
    }
}

/// R5-58 `RepairErrIsOneFailedAttempt` and `EpisodeEndsByRestoreOrFailStop`
/// — each scripted repair `Err` and each failed post-repair audit raises the
/// recovery's `failed_attempts` by exactly one while it keeps its hold; with
/// `k` failures below the budget the episode ends by `restore`, at the budget
/// by the shutdown request; no hold is ever dropped.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
#[allow(clippy::print_stderr, reason = "the seed is printed so a failure reproduces")]
async fn each_failed_attempt_counts_once_and_every_episode_ends_by_restore_or_fail_stop() {
    for seed in seeds() {
        eprintln!("R5-58 RepairErrIsOneFailedAttempt seed={seed}");
        let mut rng = Seeded(seed | 1);
        let mut node = ComposedNode::fresh();
        node.boot().await.expect("the node boots");
        let component = if rng.below(2) == 0 { C1 } else { C2 };
        node.damage(component).await;
        audit_tick(&node).await;
        let k = 1 + rng.below(3) as u32;
        let redo = rng.below(2) == 0;
        let mut sequence = vec![format!("damage {component:?}"), format!("k={k} redo={redo}")];
        for attempt in 1..=k {
            repair(&node, component, false).await;
            sequence.push(format!("repair Err #{attempt}"));
            let episode = node.recovery_observation()[&component];
            assert_eq!(
                (episode.phase, episode.failed_attempts),
                (RecoveryPhase::Held, attempt),
                "seed={seed} sequence={sequence:?}: one repair Err is one failed attempt"
            );
            assert_eq!(
                node.owner_observation().await.quiescence_holds,
                1,
                "seed={seed} sequence={sequence:?}: the recovery keeps its hold"
            );
        }
        if redo {
            repair(&node, component, true).await;
            node.damage(component).await;
            node.release_audits().await;
            sequence.push("post-repair audit fails".to_owned());
            assert_eq!(
                node.recovery_observation()[&component].failed_attempts,
                k + 1,
                "seed={seed} sequence={sequence:?}: a failed post-repair audit is one failed attempt"
            );
        }
        repair(&node, component, true).await;
        node.release_audits().await;
        sequence.push("repair Ok, clean audit".to_owned());
        assert_eq!(
            universe(&node).await,
            open_universe(0),
            "seed={seed} sequence={sequence:?}: below the budget the episode ends by restore"
        );
        assert!(node.fail_stop_requested().is_none(), "seed={seed}: no fail-stop below the budget");
    }
    // At the budget: every repair fails until the bound; the episode ends by
    // the shutdown request still holding its hold (never by a drop).
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");
    node.damage(C1).await;
    audit_tick(&node).await;
    let bound = Duration::from_secs(10);
    let mut spent = Duration::ZERO;
    let request = loop {
        if let Some(made) = node.fail_stop_requested() {
            break made;
        }
        assert!(spent <= bound, "R5-58 bound: the fail-stop arrives within 10 s of sim time");
        assert_eq!(node.owner_observation().await.leaked_quiescence_holds, 0, "no hold is dropped");
        repair(&node, C1, false).await;
        spent += SHARED_NETWORK_RETRY_PERIOD;
    };
    match request {
        ServeShutdownRequest::SharedGuestNetwork(fail_stop) => {
            assert_eq!(fail_stop.cause, SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded);
        }
    }
}

// ---------------------------------------------------------------------------
// R5-61 — the `Forwarder` component recovery order (seeded half)
// ---------------------------------------------------------------------------

/// The position of the first `kind` at or after `from` in `kinds`.
fn position_of(kinds: &[SimEffectKind], from: usize, kind: SimEffectKind) -> Option<usize> {
    kinds.iter().skip(from).position(|issued| *issued == kind).map(|at| at + from)
}

/// R5-61 (EVMOD-CTL) — The `Forwarder` component recovery through the
/// production supervisor aborts every flow, closes the old context, and only
/// then opens and probes a new one; `forwarder_generation` advances by one
/// and forwarding reopens under the new context.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a booted node (generation `G`) with one paired guest flow. When the
/// forwarder context fails `Io` and the audit tick passes, then the recovery
/// holds quiescence, and the seam's issued order after the fault is
/// `AbortAll`, then `CloseForwarder`, then `OpenForwarder`. While the reopen
/// is parked the owner holds no flow and still reports generation `G`. When
/// the reopen succeeds and the post-repair audit is clean, then forwarding is
/// open with generation `G + 1`, no flow of the old context is held, and a
/// new guest flow pairs.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn the_forwarder_recovery_aborts_every_flow_closes_the_old_context_then_reopens() {
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");
    node.open_guest_flow().await;
    let before = node.owner_observation().await.forwarder_generation;
    assert_eq!(universe(&node).await.flows, 1, "precondition: one paired flow");
    let from = node.issued_kinds().len();

    node.damage_forwarder().await;
    audit_tick(&node).await;
    let reopen = node.held_forwarder_reopen().expect("the repair parks at its reopen");

    let kinds = node.issued_kinds();
    let aborted = position_of(&kinds, from, SimEffectKind::AbortAll).expect("every flow aborted");
    let closed =
        position_of(&kinds, aborted, SimEffectKind::CloseForwarder).expect("old context closed");
    position_of(&kinds, closed, SimEffectKind::OpenForwarder)
        .expect("the new context is opened only after the old one is closed");
    let parked = node.owner_observation().await;
    assert_eq!(flows_of(&parked), 0, "no flow is held when the new context opens");
    assert_eq!(
        parked.forwarder_generation, before,
        "the generation advances only at a successful open"
    );
    assert_eq!(
        node.recovery_observation().get(&SharedGuestNetworkComponent::Forwarder).map(|e| e.phase),
        Some(RecoveryPhase::Held),
        "the recovery holds its hold across the reopen"
    );

    node.release(reopen).await;
    node.release_audits().await;
    let after = node.owner_observation().await;
    assert_eq!(after.forwarder_generation, before.next().expect("a next generation"));
    assert_eq!(after.admission, GuestAdmission::Open, "forwarding reopens at count zero");
    assert_eq!(flows_of(&after), 0, "no pair of the old context remains");
    node.open_guest_flow().await;
    assert_eq!(universe(&node).await.flows, 1, "a new flow pairs under the new context");
}

// ---------------------------------------------------------------------------
// G-V1 — the forwarder is opened and probed at boot step 1
// ---------------------------------------------------------------------------

/// The scripted forwarder failures of UI-COV-1, each with the typed cause the
/// refusal must carry.
/// One scripted forwarder failure with its typed-cause predicate.
type ForwarderFailureCase = (&'static str, SimFailure, fn(&ForwarderError) -> bool);

fn forwarder_failures() -> Vec<ForwarderFailureCase> {
    vec![
        ("AbiMismatch", SimFailure::Forwarder(SimForwarderFailure::AbiMismatch), |e| {
            matches!(e, ForwarderError::AbiMismatch { expected: 1, found: 2 })
        }),
        ("DeviceAbsent", SimFailure::Forwarder(SimForwarderFailure::DeviceAbsent), |e| {
            matches!(e, ForwarderError::DeviceAbsent { .. })
        }),
    ]
}

/// G-V1 (EVMOD-CTL, EVMOD-LIFE, D-8) — A forwarder whose phase-1 probe
/// fails at boot step 1 refuses boot with its typed cause before any boot
/// effect.
/// CONTRACT_SHAPE: unbounded-preservation.
///
/// Given a fresh node whose phase-1 forwarder probe is scripted to fail with
/// `AbiMismatch` or `DeviceAbsent`. When `serve` boots, then
/// `run_server` returns `GuestNetworkBoot(Forwarder { source })` naming that
/// cause, `health.startup.refused` carries reason `guest_network.probe`, no
/// process runs, and the refused boot issued no listener bind and no gateway
/// route. A following boot with a good forwarder serves at generation 1.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn a_forwarder_that_fails_its_boot_probe_refuses_boot_before_any_effect() {
    for (name, failure, is_cause) in forwarder_failures() {
        let collector = RefusalCollector::default();
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(collector.clone()),
        );
        let mut node = ComposedNode::fresh();
        node.fail_next_forwarder_open(failure);
        match node.boot().await {
            Err(ControlPlaneError::GuestNetworkBoot(GuestNetworkError::Forwarder { source }))
                if is_cause(&source) => {}
            other => {
                panic!("{name}: the boot refuses with the forwarder's typed cause, got {other:?}")
            }
        }
        assert!(node.process.is_none(), "{name}: no serve process runs");
        assert_eq!(
            collector.reasons(),
            vec![Some("guest_network.probe".to_owned())],
            "{name}: one health.startup.refused naming the boot probe"
        );
        let issued = node.refused_boot_issued_kinds();
        assert!(
            !issued.iter().any(|kind| matches!(
                kind,
                SimEffectKind::BindListener | SimEffectKind::GatewayRoute
            )),
            "{name}: no listener bind or gateway route before the refusal: {issued:?}"
        );

        node.boot().await.expect("a boot with a good forwarder serves");
        assert_eq!(
            node.owner_observation().await.forwarder_generation,
            ForwarderGeneration::FIRST,
            "{name}: one serving context, opened at boot step 9 (the probe's is not counted)"
        );
    }
}

/// G-V1 — A boot opens and probes the forwarder before it binds a listener
/// or converges the gateway route.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
async fn a_boot_opens_and_probes_the_forwarder_before_any_listener_or_route() {
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");
    let kinds = node.issued_kinds();
    let opened =
        position_of(&kinds, 0, SimEffectKind::OpenForwarder).expect("the forwarder opened");
    for later in [SimEffectKind::BindListener, SimEffectKind::GatewayRoute] {
        let at =
            position_of(&kinds, 0, later).unwrap_or_else(|| panic!("{later:?} issued at boot"));
        assert!(opened < at, "the forwarder opens before {later:?}: {kinds:?}");
    }
    assert_eq!(
        kinds.iter().filter(|kind| **kind == SimEffectKind::OpenForwarder).count(),
        1,
        "one context per boot"
    );
}

// ---------------------------------------------------------------------------
// R5-11 (seeded half) — the gateway route is converged at boot only
// ---------------------------------------------------------------------------

/// R5-11 (GW-LOCAL, H-4) — Every boot, fresh or after a crash, converges the
/// gateway route once and serves; a runtime recovery never writes it.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a fresh node. When it boots, then exactly one `GatewayRoute`
/// effect was issued. When a listener recovery runs to its restore, then no
/// further `GatewayRoute` effect is issued. When the process is dropped
/// (SIGKILL: the constant rules and the route stay) and the node boots
/// again, then the new process issues exactly one `GatewayRoute` effect,
/// the table is complete and forwarding is open.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "pending DELIVER step 13"]
async fn the_gateway_route_is_converged_once_per_boot_and_never_at_runtime() {
    let routes = |kinds: &[SimEffectKind]| {
        kinds.iter().filter(|kind| **kind == SimEffectKind::GatewayRoute).count()
    };
    let mut node = ComposedNode::fresh();
    node.boot().await.expect("the node boots");
    assert_eq!(routes(&node.issued_kinds()), 1, "the boot converges the gateway route once");

    node.damage(C1).await;
    audit_tick(&node).await;
    repair(&node, C1, true).await;
    node.release_audits().await;
    assert_eq!(universe(&node).await, open_universe(0), "the recovery restored");
    assert_eq!(routes(&node.issued_kinds()), 1, "no runtime recovery writes the gateway route");

    node.crash().await;
    node.boot().await.expect("the boot after SIGKILL converges and serves");
    assert_eq!(routes(&node.issued_kinds()), 1, "the new process converges the route once");
    assert_eq!(universe(&node).await, open_universe(0), "it serves with forwarding open");
}
