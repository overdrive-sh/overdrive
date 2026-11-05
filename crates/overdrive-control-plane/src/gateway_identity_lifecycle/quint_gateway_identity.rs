//! Quint conformance driver QC-1 — `gateway_identity` (public-ingress-gateway,
//! ADR-0225).
//!
//! Replays `specs/quint/public-ingress-gateway/gateway_identity.qnt`,
//! conformance main `identity_conf` (the configuration of `gateway_identity`
//! with `REC = true`, recording `conf_step`; D-UP-1), against the production
//! owners the protocol spans: the dataplane `GatewayIdentitySlot`, the
//! control-plane `ControlPlaneGatewayIdentityLifecycle`, the production
//! `GatewaySvidLifecycle` reconciler (its `hydrate_*` and `reconcile` produce
//! every `Action` the driver dispatches; the driver never constructs one), the
//! `EvaluationBroker`, and the action-shim executor
//! `action_shim::gateway_svid::dispatch_issue`. A Sim CA, `SimObservationStore`
//! and `SimClock` stand at their driven ports. The spec is referenced in place.
//!
//! Disable is a synchronous clear (R-4): there is no drop action, and the spec
//! has no `settle_ok` / `settle_timeout` step. The stutter `idle` is absent from
//! the conformance main (ruling 7).
//!
//! Action -> owner driving port (feature-delta, "Conformance action maps for the
//! three older mains", `identity_conf`):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | rebuild the world: a new slot, broker and lifecycle control, an empty observation store |
//! | `submit_evaluation` | `GatewayIdentityLifecycleControl::ensure_current`, polled once so the retained `Evaluation` is submitted; the driver then drains the broker, runs the production reconciler's `hydrate_*` and `reconcile` at the `SimClock` tick and queues the `IssueGatewaySvid` it emits and asserts that no pending epoch is beyond the slot's desired epoch (the model's `issues` is model-only and a `Step` carries no model state, so equality with it cannot be asserted in a handler) |
//! | `issue_complete(epoch)` | `action_shim::gateway_svid::dispatch_issue` with the queued action of that epoch, which is then dropped; the step fails when no Issue of that epoch is pending (R-31) |
//! | `disable` | `GatewayIdentityLifecycleControl::disable_after_drain` (infallible, a synchronous clear) |
//! | `advance_time` | `SimClock::tick` by one model time unit |
//! | `audit_fail` | a failing `SimObservationStore` write for the next issued-certificate row (queued in the store at this step) |
//! | `restart` | drop the owner composition and rebuild it (`GatewayIdentitySlot::new`, a new `EvaluationBroker`, a new lifecycle control) over the same `SimObservationStore` and CA, discarding queued actions and the pending `ensure_current` future |
//!
//! Projection (`State::from_driver`), per the pinned table: `desired_epoch` and
//! `desired_enabled` <- `GatewayIdentitySlot::desired()`; `held` <-
//! `GatewayIdentitySlot::current()`; `audited` <-
//! `ObservationStore::issued_certificate_rows` filtered to the gateway SPIFFE
//! id (the owner's own output surface, read through the port trait). The model
//! variables `now`, `next_serial`, `audit_armed`, `issues`, `g` and
//! `conf_step` are model-only and are not projected.
//!
//! The CA draws serials from seeded entropy, so a certificate serial has no
//! ordinal of its own. The model numbers serials by issuance attempt (a failed
//! audit consumes a serial that never reaches the audit rows). The driver
//! derives that number from the owner's own output: the audit row's
//! `issuance_ordinal` (allocated per attempt by
//! `ObservationStore::next_issuance_ordinal` before the audit write, so a
//! failed attempt consumes one) plus one, since the store counts from zero and
//! the model from one. No driver counter participates in the comparison.
//!
//! The retry `View` is not threaded between evaluations: the model has no
//! backoff, so each evaluation reconciles over `GatewaySvidLifecycleView::default()`.
//!
//! Step decoding (D-UP-1): `Config::nondet =
//! &["identity_conf::gateway_identity::conf_step"]`; the state projection
//! matches `identity_conf::gateway_identity::<var>`.
//!
//! The model's `Gen.na` (expiry, in model time units) is compared with the
//! slot's `not_after`. The issuer sets `not_after = issued_at - SKEW_TOLERANCE +
//! WORKLOAD_SVID_TTL` (`issue_and_audit`), the model's `TTL` is four time units
//! of `WORKLOAD_SVID_TTL / 4`, and the `SimClock` advances only through
//! `advance_time`, so `na = (not_after + SKEW_TOLERANCE - clock_epoch) /
//! MODEL_TIME_UNIT`, where `clock_epoch` is the `SimClock`'s wall clock at
//! construction (model `now = 0`). A `not_after` that is not a whole number of
//! model units from that epoch fails the step rather than being rounded.
//!
//! Every named run bound to this main is pinned with
//! `#[quint_test(max_samples = 1)]`; `#[quint_run]` replays randomized traces
//! (seed printed on failure). The tests run against the landed identity
//! owners.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::cast_possible_wrap,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use itf::Value;
use overdrive_core::ca::{SKEW_TOLERANCE, WORKLOAD_SVID_TTL};
use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};
use overdrive_core::id::{CertSerial, NodeId, SpiffeId};
use overdrive_core::reconcilers::{Action, HydrationContext, TickContext};
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::DriverRegistry;
use overdrive_core::traits::observation_store::{
    ObservationStore, ObservationStoreError, ObservationWrite,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::workflow::{SignalKey, SignalValue};
use overdrive_dataplane::mtls::gateway_identity::GatewayIdentitySlot;
use overdrive_gateway::ports::GatewayIdentityLifecycleControl;
use overdrive_reconcilers::{
    AnyReconciler, AnyReconcilerView, GatewaySvidLifecycle, GatewaySvidLifecycleView,
};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::read_ports::{
    SimHeldSvidView, SimListenerFacts, SimServiceVipView, SimWorkflowLiveSet,
};
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_store_local::LocalIntentStore;
use parking_lot::Mutex;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::runtime::Runtime;

use super::{ControlPlaneGatewayIdentityLifecycle, gateway_svid_lifecycle_registration};
use crate::action_shim::ShimError;
use crate::action_shim::gateway_svid::dispatch_issue;

/// The conformance main this driver replays.
const MAIN: &str = "identity_conf";
/// The module that declares the compared variables.
const MODULE: &str = "gateway_identity";
/// The model's `TTL` is four time units; one unit is a quarter of the SVID TTL.
const MODEL_TIME_UNIT: Duration = Duration::from_secs(WORKLOAD_SVID_TTL.as_secs() / 4);
/// Seed of the CA's entropy and of the observation store.
const SEED: u64 = 0x1D_E471_7C0F;

/// The node the composition runs on.
fn node() -> NodeId {
    NodeId::from_str("gw-node").expect("fixture node id")
}

/// Everything one process lifetime owns; `restart` replaces it, `init` replaces
/// it together with the durable stores.
struct Owners {
    /// The production slot: the owner of the compared state.
    slot: Arc<GatewayIdentitySlot>,
    /// The production lifecycle control over the slot, the broker and the clock.
    lifecycle: ControlPlaneGatewayIdentityLifecycle,
    /// The broker the control submits its retained evaluation to.
    broker: Arc<Mutex<EvaluationBroker>>,
    /// The retained evaluation (reconciler name and `node/<id>` target).
    evaluation: Evaluation,
}

struct IdentityConfDriver {
    owners: Owners,
    /// Queued `IssueGatewaySvid` actions by model epoch (the model's `issues`).
    queued: BTreeMap<i64, Action>,
    /// Durable fixtures: the CA and the observation store survive `restart`.
    ca: SimCa,
    observation: Arc<dyn ObservationStore>,
    faults: Arc<SimObservationStore>,
    clock: Arc<SimClock>,
    /// The hydration context's intent store, kept alive for the whole run.
    intent: LocalIntentStore,
    intent_path: std::path::PathBuf,
    _intent_dir: TempDir,
    /// Whether the next issuance's audit write was made to fail.
    audit_armed: bool,
    /// The `SimClock`'s wall clock at construction: the model's `now = 0`.
    clock_epoch: Duration,
    runtime: Runtime,
}

impl IdentityConfDriver {
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let faults = Arc::new(SimObservationStore::single_peer(node(), SEED));
        let observation: Arc<dyn ObservationStore> = faults.clone();
        let clock = Arc::new(SimClock::new());
        let ca = SimCa::new(Arc::new(SimEntropy::new(SEED)));
        let intent_dir = TempDir::new().expect("intent store directory");
        let intent_path = intent_dir.path().join("intent.redb");
        let intent = LocalIntentStore::open(&intent_path).expect("LocalIntentStore::open");
        let owners = Self::build_owners(&ca, &clock);
        let clock_epoch = clock.unix_now();
        Self {
            owners,
            queued: BTreeMap::new(),
            ca,
            observation,
            faults,
            clock,
            intent,
            intent_path,
            _intent_dir: intent_dir,
            audit_armed: false,
            clock_epoch,
            runtime,
        }
    }

    /// A fresh slot, broker and lifecycle control: the volatile owners.
    fn build_owners(ca: &SimCa, clock: &Arc<SimClock>) -> Owners {
        let bundle = ca.trust_bundle().expect("the Sim CA composes a trust bundle");
        let slot = Arc::new(GatewayIdentitySlot::new(node(), bundle));
        let broker = Arc::new(Mutex::new(EvaluationBroker::new()));
        let (_reconciler, evaluation) = gateway_svid_lifecycle_registration(&node());
        let lifecycle = ControlPlaneGatewayIdentityLifecycle::new(
            Arc::new(slot.lifecycle_target()),
            Arc::clone(&broker),
            evaluation.clone(),
            Arc::clone(clock) as Arc<dyn Clock>,
        );
        Owners { slot, lifecycle, broker, evaluation }
    }

    fn init(&mut self) {
        let fresh = Self::new();
        *self = fresh;
    }

    fn submit_evaluation(&mut self) -> Result {
        // Poll `ensure_current` once: the first poll submits the retained
        // evaluation; the future is then dropped without awaiting completion.
        let deadline = Instant::now() + Duration::from_secs(3600);
        self.runtime.block_on(async {
            let future = self.owners.lifecycle.ensure_current(deadline);
            futures::pin_mut!(future);
            let _ = futures::poll!(&mut future);
        });

        let now_unix = UnixInstant::from_clock(self.clock.as_ref());
        let admitted =
            self.owners.broker.lock().drain_pending(8, &BTreeSet::new(), Instant::now(), now_unix);
        for (evaluation, _queued_for) in admitted {
            self.reconcile(&evaluation, now_unix)?;
        }
        self.assert_pending_issues_follow_the_slot()
    }

    /// The model's `issues` is the set of epochs of in-flight, not-yet-completed
    /// `IssueGatewaySvid` actions (R-31: model-only, no accessor). The distinct
    /// epochs the driver holds are the production reconciler's output, so the
    /// handler asserts what the model derives for a reconcile from the slot's
    /// own `desired()`: an Issue is only ever emitted while the slot is
    /// enabled, and it carries the slot's desired epoch (or an earlier epoch
    /// that is still pending, which the model keeps in `issues` until its
    /// `issue_complete`).
    fn assert_pending_issues_follow_the_slot(&self) -> Result {
        let desired = self.owners.slot.desired();
        let desired_epoch = i64::try_from(desired.epoch.get().get())?;
        if let Some(newest) = self.queued.keys().next_back()
            && *newest > desired_epoch
        {
            bail!(
                "a pending Issue carries epoch {newest}, beyond the slot's desired epoch {desired_epoch}"
            );
        }
        Ok(())
    }

    /// Run the production reconciler's `hydrate_*` and `reconcile` for one
    /// admitted evaluation and queue the issue action it emits.
    fn reconcile(&mut self, evaluation: &Evaluation, now_unix: UnixInstant) -> Result {
        let reconciler = AnyReconciler::GatewaySvidLifecycle(GatewaySvidLifecycle::canonical());
        let drivers = DriverRegistry::new();
        let vm_host = SimVmHostState::new();
        let listener_facts = SimListenerFacts::new(BTreeMap::new());
        let service_vip_view = SimServiceVipView::new(BTreeMap::new());
        let workflow_live_set = SimWorkflowLiveSet::new(BTreeSet::new());
        let held_svid_view = SimHeldSvidView::new(BTreeMap::new());
        let guest_attachments = overdrive_sim::adapters::SimGuestAttachmentView::new(
            overdrive_core::guest_network::GuestAttachmentOccupancy { held: 0, retiring: 0 },
            BTreeMap::new(),
        );
        let node_id = node();
        let ctx = HydrationContext {
            intent_store: &self.intent,
            observation_store: self.observation.as_ref(),
            drivers: &drivers,
            vm_host_state: &vm_host,
            listener_facts: &listener_facts,
            service_vip_view: &service_vip_view,
            workflow_live_set: &workflow_live_set,
            held_svid_view: &held_svid_view,
            guest_attachments: &guest_attachments,
            gateway_identity: self.owners.slot.as_ref(),
            node_id: &node_id,
            host_ipv4: std::net::Ipv4Addr::LOCALHOST,
            intent_redb_path: &self.intent_path,
        };
        let (desired, actual) = self.runtime.block_on(async {
            let desired = reconciler.hydrate_desired(&ctx, &evaluation.target).await?;
            let actual = reconciler.hydrate_actual(&ctx, &evaluation.target).await?;
            Ok::<_, overdrive_core::reconcilers::HydrateError>((desired, actual))
        })?;
        let view = AnyReconcilerView::GatewaySvidLifecycle(GatewaySvidLifecycleView::default());
        let tick = TickContext {
            now: Instant::now(),
            now_unix,
            tick: 1,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);
        for action in actions {
            if let Action::IssueGatewaySvid { epoch, .. } = &action {
                let model_epoch = i64::try_from(epoch.get().get())?;
                self.queued.insert(model_epoch, action);
            }
        }
        Ok(())
    }

    fn issue_complete(&mut self, epoch: i64) -> Result {
        let action = self
            .queued
            .remove(&epoch)
            .ok_or_else(|| anyhow!("no IssueGatewaySvid of epoch {epoch} is queued"))?;
        let before = self.gateway_serials()?;
        let target = self.owners.slot.action_target();
        let outcome = self.runtime.block_on(dispatch_issue(
            &action,
            &self.ca,
            self.observation.as_ref(),
            self.clock.as_ref(),
            &target,
        ));
        let armed = std::mem::take(&mut self.audit_armed);
        let after = self.gateway_serials()?;
        match (armed, outcome) {
            (true, Err(ShimError::GatewaySvidIssue(_))) => {
                if after != before {
                    bail!("a failed audit write must leave no audit row");
                }
            }
            (true, other) => {
                bail!("an armed audit failure must surface GatewaySvidIssue: {other:?}")
            }
            (false, Ok(_)) => {
                let fresh: Vec<&CertSerial> = after.difference(&before).collect();
                let [_] = fresh.as_slice() else {
                    bail!(
                        "a successful issue leaves exactly one new audit row, saw {}",
                        fresh.len()
                    );
                };
            }
            (false, Err(error)) => {
                bail!("dispatch_issue failed without an injected fault: {error}")
            }
        }
        Ok(())
    }

    fn disable(&self) {
        self.owners.lifecycle.disable_after_drain();
    }

    fn advance_time(&self) {
        self.clock.tick(MODEL_TIME_UNIT);
    }

    /// The next issuance's audit write fails: the failure is queued in the
    /// Sim store, a driven port. The store offers no way to withdraw a queued
    /// failure, so `restart` consumes it by performing the write that pops it
    /// (the model's `restart` clears `audit_armed`).
    fn audit_fail(&mut self) {
        self.faults.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
            "injected gateway audit failure",
        )));
        self.audit_armed = true;
    }

    /// The volatile owners die with the process; the CA and the observation
    /// store are durable.
    fn restart(&mut self) -> Result {
        self.owners = Self::build_owners(&self.ca, &self.clock);
        self.queued.clear();
        if std::mem::take(&mut self.audit_armed) {
            self.consume_queued_audit_failure()?;
        }
        Ok(())
    }

    /// Pop the queued write failure through the store's own write port, with a
    /// row that is not a gateway audit row, so the next issuance is not failed
    /// by a fault the model has already cleared.
    fn consume_queued_audit_failure(&self) -> Result {
        let write = ObservationWrite::Signal {
            key: SignalKey::new("conf-drain")?,
            value: SignalValue::new("drain"),
        };
        match self.runtime.block_on(self.observation.write(write)) {
            Err(_) => Ok(()),
            Ok(()) => bail!("the queued audit failure was not consumed by the draining write"),
        }
    }

    /// The serials of the audit rows that name the gateway subject.
    fn gateway_serials(&self) -> Result<BTreeSet<CertSerial>> {
        let gateway = SpiffeId::for_gateway(&node());
        let rows = self
            .runtime
            .block_on(self.observation.issued_certificate_rows())
            .map_err(|error| anyhow!("issued_certificate_rows failed: {error}"))?;
        Ok(rows.into_iter().filter(|row| row.spiffe_id == gateway).map(|row| row.serial).collect())
    }

    /// The model's `na` of a held generation: its `not_after` in model time
    /// units since the clock epoch (see the header for the derivation).
    fn model_expiry(&self, not_after: UnixInstant) -> Result<i64> {
        let from_epoch = (not_after.as_unix_duration() + SKEW_TOLERANCE)
            .checked_sub(self.clock_epoch)
            .ok_or_else(|| anyhow!("not_after {not_after:?} precedes the clock epoch"))?;
        let unit = MODEL_TIME_UNIT.as_nanos();
        if from_epoch.as_nanos() % unit != 0 {
            bail!(
                "not_after {not_after:?} is not a whole number of model time units from the epoch"
            );
        }
        Ok(i64::try_from(from_epoch.as_nanos() / unit)?)
    }

    /// The model's attempt ordinal of an audited serial, from its audit row.
    fn ordinal_of(&self, serial: &CertSerial) -> Result<i64> {
        let gateway = SpiffeId::for_gateway(&node());
        let rows = self
            .runtime
            .block_on(self.observation.issued_certificate_rows())
            .map_err(|error| anyhow!("issued_certificate_rows failed: {error}"))?;
        let row = rows
            .into_iter()
            .find(|row| row.spiffe_id == gateway && &row.serial == serial)
            .ok_or_else(|| anyhow!("serial {serial} has no gateway audit row"))?;
        Ok(i64::try_from(row.issuance_ordinal.as_u64())? + 1)
    }
}

impl Driver for IdentityConfDriver {
    type State = IdentityState;

    fn config() -> Config {
        Config { state: &[], nondet: &["identity_conf::gateway_identity::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init(),
            submit_evaluation => self.submit_evaluation()?,
            issue_complete(epoch: i64) => self.issue_complete(epoch)?,
            disable => self.disable(),
            advance_time => self.advance_time(),
            audit_fail => self.audit_fail(),
            restart => self.restart()?,
        })
    }
}

/// The compared variables: the model's `desired_epoch`, `desired_enabled`,
/// `held` (epoch, serial ordinal, expiry in model units) and `audited` (serial ordinals).
#[derive(Debug, PartialEq, Eq)]
struct IdentityState {
    desired_epoch: i64,
    desired_enabled: bool,
    held: BTreeSet<(i64, i64, i64)>,
    audited: BTreeSet<i64>,
}

impl State<IdentityConfDriver> for IdentityState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &IdentityConfDriver) -> Result<Self> {
        let desired = driver.owners.slot.desired();
        let held = match driver.owners.slot.current() {
            Some(facts) => BTreeSet::from([(
                i64::try_from(facts.epoch.get().get())?,
                driver.ordinal_of(&facts.serial)?,
                driver.model_expiry(facts.not_after)?,
            )]),
            None => BTreeSet::new(),
        };
        let audited = driver
            .gateway_serials()?
            .iter()
            .map(|serial| driver.ordinal_of(serial))
            .collect::<Result<BTreeSet<i64>>>()?;
        Ok(Self {
            desired_epoch: i64::try_from(desired.epoch.get().get())?,
            desired_enabled: desired.spiffe_id.is_some(),
            held,
            audited,
        })
    }
}

/// Why a trace value does not have the shape the driver compares.
#[derive(Debug, thiserror::Error)]
enum ItfError {
    #[error("the trace has no `{0}` variable")]
    MissingVar(String),
    #[error("the trace's record has no `{0}` field")]
    MissingField(String),
    #[error("expected {expected} in the trace, found {found:?}")]
    Shape { expected: &'static str, found: Value },
}

fn int(value: &Value) -> std::result::Result<i64, ItfError> {
    match value {
        Value::Number(n) => Ok(*n),
        Value::BigInt(big) => i64::try_from(big.get().clone())
            .map_err(|_| ItfError::Shape { expected: "a 64-bit int", found: value.clone() }),
        other => Err(ItfError::Shape { expected: "an int", found: other.clone() }),
    }
}

fn set_items(value: &Value) -> std::result::Result<Vec<&Value>, ItfError> {
    match value {
        Value::Set(set) => Ok(set.iter().collect()),
        other => Err(ItfError::Shape { expected: "a set", found: other.clone() }),
    }
}

impl IdentityState {
    /// The spec side: the `identity_conf::gateway_identity::*` variables.
    fn from_value(state: &Value) -> std::result::Result<Self, ItfError> {
        let Value::Record(record) = state else {
            return Err(ItfError::Shape {
                expected: "a record of variables",
                found: state.clone(),
            });
        };
        let var = |name: &str| -> std::result::Result<&Value, ItfError> {
            let qualified = format!("{MAIN}::{MODULE}::{name}");
            record.get(&qualified).ok_or(ItfError::MissingVar(qualified))
        };
        let desired_enabled = match var("desired_enabled")? {
            Value::Bool(flag) => *flag,
            other => return Err(ItfError::Shape { expected: "a bool", found: other.clone() }),
        };
        let held = set_items(var("held")?)?
            .into_iter()
            .map(|generation| {
                let Value::Record(generation) = generation else {
                    return Err(ItfError::Shape {
                        expected: "a generation record",
                        found: generation.clone(),
                    });
                };
                let field = |name: &str| {
                    generation.get(name).ok_or_else(|| ItfError::MissingField(name.to_owned()))
                };
                Ok((int(field("epoch")?)?, int(field("serial")?)?, int(field("na")?)?))
            })
            .collect::<std::result::Result<BTreeSet<_>, ItfError>>()?;
        let audited = set_items(var("audited")?)?
            .into_iter()
            .map(int)
            .collect::<std::result::Result<BTreeSet<_>, ItfError>>()?;
        Ok(Self { desired_epoch: int(var("desired_epoch")?)?, desired_enabled, held, audited })
    }
}

impl<'de> Deserialize<'de> for IdentityState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------
// Named runs bound to `identity_conf`, then randomized traces.
// ---------------------------------------------------------------------

/// Enabled and empty: one reconcile, one issue, the audited generation is held.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/gateway_identity.qnt",
    main = "identity_conf",
    test = "issue_hold",
    max_samples = 1
)]
fn gateway_identity_issue_hold() -> impl Driver {
    IdentityConfDriver::new()
}

/// Near expiry: a fresh audited generation replaces the old one atomically.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/gateway_identity.qnt",
    main = "identity_conf",
    test = "reissue_near_expiry",
    max_samples = 1
)]
fn gateway_identity_reissue_near_expiry() -> impl Driver {
    IdentityConfDriver::new()
}

/// An Issue is in flight when the node disables: the disable clears at once and
/// the stale-epoch completion is a no-op.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/gateway_identity.qnt",
    main = "identity_conf",
    test = "disable_races_inflight_issue",
    max_samples = 1
)]
fn gateway_identity_disable_races_inflight_issue() -> impl Driver {
    IdentityConfDriver::new()
}

/// An injected audit write failure holds nothing; the retry audits and holds.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/gateway_identity.qnt",
    main = "identity_conf",
    test = "audit_failure_no_hold",
    max_samples = 1
)]
fn gateway_identity_audit_failure_no_hold() -> impl Driver {
    IdentityConfDriver::new()
}

/// A restart begins empty, keeps the durable audit rows and reissues with a new
/// serial; a disable then clears.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/public-ingress-gateway/gateway_identity.qnt",
    main = "identity_conf",
    test = "restart_reissue",
    max_samples = 1
)]
fn gateway_identity_restart_reissue() -> impl Driver {
    IdentityConfDriver::new()
}

/// Randomized traces of the conformance main (seed printed on failure).
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(
    spec = "../../specs/quint/public-ingress-gateway/gateway_identity.qnt",
    main = "identity_conf"
)]
fn gateway_identity_identity_conf_randomized_traces() -> impl Driver {
    IdentityConfDriver::new()
}
