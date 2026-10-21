//! Quint conformance driver — `quiescence` (QHOLD, ADR-0169; ADR-0184;
//! ADR-0124; D-UP-3, D-UP-4), conformance main `quiesce_conf` (QPROJ-1,
//! D-UP-1).
//!
//! Replays `specs/quint/guest-flow-owner/quiescence.qnt` in place against the
//! production composition: one node booted the way `run_server` boots it —
//! phase 1 (`probe_guest_flow_startup`) over the process's seam, then
//! `boot_server` with `GuestNetworkSource::Composed`, whose step 5 composes
//! the production guest-flow owner and its mTLS worker
//! (`compose_guest_network` over `SimGuestFlowIo`, which holds the
//! `SimFlowForwarder`; D-8, T-8) — the production runtime shared-network
//! supervisor, `SimMtlsIntercept` at the shared mTLS table and a `SimClock`
//! ([`crate::shared_network_test_ports::composed`]). Each boot is handed a
//! fresh `BootAdapters` bundle (NO-OVERRIDE-SEAMS: no `ServerConfig` adapter
//! field exists); the node's stores are reopened from its data directory at
//! every boot (STORES-IN), so a `RestartWith` reads back durable state the
//! way a new process does. The driver plays the
//! environment the model leaves to it — component damage, guest flows, the
//! process crash, boot step 8's outcome, the clock, and the two defect
//! stimuli of D-UP-3 / D-UP-4 — and re-implements no rule of the owner's
//! counted hold, the supervisor's recovery or the boot order.
//!
//! `quiesce_conf` records `conf_step` (D-UP-1): the driver reads each step's
//! action and picks from the single-element path
//! `"quiesce_conf::quiescence::conf_step"` ([`Driver::config`]), and its state
//! projection matches the trace's qualified variable names
//! (`quiesce_conf::quiescence::<var>`, spike QT1).
//!
//! # Action → production call or fault (§ *Conformance observation surface and action map*, `quiescence`)
//!
//! | Action | Driver |
//! |---|---|
//! | `InitWith(fl)` | a fresh node booted through the composed boot (phase 1, then `boot_server` with `GuestNetworkSource::Composed`; boot step 8 writes the table); with `fl = 1` one guest-opened flow is admitted, read, attached and paired on an Active allocation |
//! | `InitDown` | a fresh node, never booted: no process, no table |
//! | `Damage(c)` | seam: component `c`'s accept task exits (`fail_listener`) |
//! | `OpenFlow` | seam: a guest-opened flow is admitted, read, attached, paired |
//! | `Crash` | drop the composition (`ServerHandle::kill_for_test`); the kernel keeps the shared table |
//! | `RestartWith(writeOk)` | recompose through the in-crate composed rig: phase 1 over a fresh `SimGuestFlowIo`, then `boot_server` with `GuestNetworkSource::Composed` over that seam and a fresh `BootAdapters` (the same `boot_server` `run_server` calls; not `run_server`, which takes only `GuestFlowAdapter::host()`, so `Damage` / `RepFail` could not be scripted at the seam); boot step 8's table transaction on the node's `SimMtlsIntercept` succeeds, or is scripted to fail (`fail_next_table_transaction`) and `boot_server` returns the typed refusal `MtlsBoot(SharedOwner)` of ADR-0184 |
//! | `RecQuiesce(c)` | `SimClock` to the supervisor's audit tick (`SHARED_NETWORK_AUDIT_PERIOD`); the detecting audit is released; the supervisor's recovery of `c` calls `quiesce()` |
//! | `RecRepair(c)` / `RepFail(c)` | `SimClock` to the retry tick; `c`'s repair (`converge_shared`, parked at its `bind_listener` on the held socket) returns `Ok` (still listening: accept re-spawned, nothing closed or bound) / a scripted `Err` (the held socket found not listening, a failed attempt) |
//! | `RecRedo(c)` | the post-repair audit is released over the damage the trace's preceding `Damage(c)` re-applied |
//! | `RecRestore(c)` | the clean post-repair audit is released; the supervisor calls `restore(hold)` |
//! | `LeakHold` | the driver calls the composed owner's `quiesce()` and drops the returned hold unredeemed (D-UP-3; [`ComposedNode::leak_hold`]) |
//! | `ForeignRestore` | the driver issues a hold from a second `QuiescenceLedger::new()` (PD-2) and calls the composed owner's `restore(hold)`; it requires `Err(ForeignHold)` (D-UP-4, PD-1; [`ComposedNode::restore_foreign_hold`]) |
//! | `FailStop(c)` | `SimClock` to the audit tick (a leaked hold, cause `LeakedQuiescenceHold`) or past `SHARED_NETWORK_RECOVERY_DEADLINE` with the repair still failing (cause `RecoveryDeadlineExceeded`); the driver receives `ServeShutdownRequest::SharedGuestNetwork` and the process ends |
//!
//! **`FailStop` after `ForeignRestore`.** The supervisor's
//! `ForeignQuiescenceHold` fail-stop is not driven: the foreign `restore` is
//! the driver's call, not the supervisor's, and no production path presents
//! a foreign hold to the supervisor (FHOLD-CHECK, user ruling 2026-10-15).
//! That fail-stop is checked by the Quint check
//! `quiescence-foreign-hold-ends-in-fail-stop` and by the supervisor's cell
//! in `shared_network_recovery_acceptance`
//! (`a_restore_answered_foreign_hold_fail_stops_with_foreign_quiescence_hold`).
//! A trace whose `FailStop` is enabled only by the driver's `foreign` flag
//! therefore fails with a typed error naming it (no empty handler); the
//! named run `ForeignHoldRestoreChangesNothingThenFailStops` is not pinned.
//!
//! # Projection (`State::from_driver`)
//!
//! While a `serve` process runs (`up`): `cnt` ← O3
//! `flow_observation().quiescence_holds`; `latch` ← O3 `admission`; `leaked`
//! ← O3 `leaked_quiescence_holds` (the owner's ledger's `leaked()`);
//! `flowsOpen` ← O3 the flows the owner holds (`guest_flows` + `dials` +
//! `datagrams` over every CID); `rec(c)` / `att(c)` ← O6
//! `recovery_observation()` (`Held` → `held`, `Repaired` → `repaired`,
//! absent → `idle`; `failed_attempts`); `foreign` ← C (the driver's own
//! `ForeignRestore` returned `Err(ForeignHold)`; false again once the
//! composition is dropped); `table` ← S
//! `SimMtlsIntercept::shared_table_state()` (`Complete` ⇔ true; the QPROJ-7
//! arbiter, Tier-3 pair R5-30).
//!
//! While no process runs: `stop` ← C (how the driver's own last call ended:
//! `crash` after it dropped the composition, `failstop` after it received the
//! supervisor's request, `refused` after the boot returned the typed refusal)
//! and `table` ← S. The model's `cnt`, `latch`, `rec`, `att`, `flowsOpen`,
//! `leaked`, `foreign` of a stopped node describe a process that does not
//! exist (the model resets them); there is no owner or supervisor to read
//! them from, so they are not compared while `up` is false.
//!
//! `dmg`, `repLeft` are stimulus; `g` is ghost; `conf_step` is the step
//! record — none is compared.
//!
//! Lane: `integration-tests` (it spawns the `quint` CLI through
//! quint-connect and boots `boot_server` over real redb stores in a
//! temporary directory), `cargo xtask lima run --`.

#![allow(
    clippy::match_same_arms,
    clippy::needless_pass_by_ref_mut,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::unnecessary_wraps,
    non_snake_case,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose; \
              `switch!` binds each nondeterministic pick under the spec's own name (`writeOk`); \
              handler signatures follow the action map"
)]

use std::collections::BTreeMap;
use std::time::Duration;

use itf::Value;
use overdrive_core::guest_network::{
    ServeShutdownRequest, SharedGuestNetworkComponent, SharedGuestNetworkFailStopCause,
};
use overdrive_sim::adapters::{SimInterceptFault, SimSharedTable};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;

use crate::error::{ControlPlaneError, MtlsBootError};
use crate::guest_network::quint_itf::{self, ItfError, Qualified};
use crate::guest_network::{GuestAdmission, QuiescenceRestoreError};
use crate::shared_network_test_ports::composed::{C1, C2, ComposedNode, NodeStop};
use crate::{RecoveryPhase, SHARED_NETWORK_AUDIT_PERIOD, SHARED_NETWORK_RECOVERY_DEADLINE};

/// The conformance main (D-UP-1: the one `quiescence` main recording
/// `conf_step`) and the module declaring its variables; the projection reads
/// `quiesce_conf::quiescence::<var>` (spike QT1).
const MAIN: &str = "quiesce_conf";
const MODULE: &str = "quiescence";

/// The retry tick of a recovery attempt (ADR-0124, D-295-R13).
const RETRY: Duration = crate::SHARED_NETWORK_RETRY_PERIOD;

/// Bound on the retry ticks `FailStop` advances through before it expects the
/// supervisor's request: the recovery deadline over the retry period, plus
/// one.
const FAIL_STOP_TICKS: u32 = 21;

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{action}` is not enabled in the production state: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error(
        "`FailStop` enabled only by the driver's own `ForeignRestore`: the supervisor's \
         `ForeignQuiescenceHold` fail-stop is not drivable (no production path presents a foreign \
         hold to the supervisor; FHOLD-CHECK, checked by `quiescence-foreign-hold-ends-in-fail-stop` \
         and the supervisor cell)"
    )]
    ForeignFailStopNotDrivable,
    #[error("component `{0}` is not one of the instance's components C1, C2")]
    UnknownComponent(String),
    #[error("boot step 8 was scripted to {expected} but the boot {observed}")]
    BootDiverged { expected: &'static str, observed: String },
}

/// The production component the model's component `c` names (QPROJ-1
/// identity relabelling: a bijection fixed here, never a state source).
fn component(c: &str) -> std::result::Result<SharedGuestNetworkComponent, DriverError> {
    match c {
        "C1" => Ok(C1),
        "C2" => Ok(C2),
        other => Err(DriverError::UnknownComponent(other.to_owned())),
    }
}

/// The model's name for a production component; `None` has no pre-image.
fn model_name(component: SharedGuestNetworkComponent) -> Option<&'static str> {
    if component == C1 {
        Some("C1")
    } else if component == C2 {
        Some("C2")
    } else {
        None
    }
}

struct QuiescenceDriver {
    rt: tokio::runtime::Runtime,
    /// The production composition under test (the composed node, whose live
    /// process holds the owner and the supervisor), held directly from
    /// construction (S-7, the "Owner field" ruling); `InitWith` / `InitDown`
    /// replace it with a fresh node.
    node: ComposedNode,
    /// C: this process's `ForeignRestore` returned `Err(ForeignHold)`.
    foreign: bool,
}

impl QuiescenceDriver {
    fn fresh() -> Self {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("current-thread runtime with paused time");
        Self { rt, node: ComposedNode::fresh(), foreign: false }
    }

    fn init_with(&mut self, flows: i64) -> Result {
        let mut node = ComposedNode::fresh();
        self.rt.block_on(async {
            node.boot().await.map_err(|refusal| DriverError::BootDiverged {
                expected: "succeed",
                observed: format!("refused: {refusal}"),
            })?;
            if flows == 1 {
                node.open_guest_flow().await;
            }
            Ok::<(), DriverError>(())
        })?;
        self.node = node;
        self.foreign = false;
        Ok(())
    }

    fn init_down(&mut self) {
        self.node = ComposedNode::fresh();
        self.foreign = false;
    }

    fn damage(&mut self, c: &str) -> Result {
        let target = component(c)?;
        let node = &self.node;
        self.rt.block_on(node.damage(target));
        Ok(())
    }

    fn open_flow(&mut self) -> Result {
        let rt = &self.rt;
        let node = &mut self.node;
        rt.block_on(node.open_guest_flow());
        Ok(())
    }

    fn crash(&mut self) -> Result {
        let rt = &self.rt;
        let node = &mut self.node;
        rt.block_on(node.crash());
        self.foreign = false;
        Ok(())
    }

    fn restart_with(&mut self, write_ok: bool) -> Result {
        let rt = &self.rt;
        let node = &mut self.node;
        if !write_ok {
            node.intercept.fail_next_table_transaction(SimInterceptFault::NftRuleInstall {
                op: "shared-table-transaction",
                errno: libc::EBUSY,
            });
        }
        let booted = rt.block_on(node.boot());
        self.foreign = false;
        match (write_ok, booted) {
            (true, Ok(())) => Ok(()),
            (false, Err(ControlPlaneError::MtlsBoot(MtlsBootError::SharedOwner { .. }))) => Ok(()),
            (true, Err(refusal)) => Err(DriverError::BootDiverged {
                expected: "succeed",
                observed: format!("refused: {refusal}"),
            }
            .into()),
            (false, Err(other)) => Err(DriverError::BootDiverged {
                expected: "refuse with MtlsBoot(SharedOwner)",
                observed: format!("refused otherwise: {other}"),
            }
            .into()),
            (false, Ok(())) => Err(DriverError::BootDiverged {
                expected: "refuse with MtlsBoot(SharedOwner)",
                observed: "succeeded".to_owned(),
            }
            .into()),
        }
    }

    fn rec_quiesce(&mut self, c: &str) -> Result {
        let target = component(c)?;
        let rt = &self.rt;
        let node = &mut self.node;
        rt.block_on(async {
            node.advance(SHARED_NETWORK_AUDIT_PERIOD).await;
            node.release_audits().await;
        });
        if !node.recovery_observation().contains_key(&target) {
            return Err(DriverError::NotEnabled {
                action: "RecQuiesce",
                why: format!("no recovery of {target:?} began at the audit tick"),
            }
            .into());
        }
        Ok(())
    }

    fn rec_repair(&mut self, c: &str, ok: bool) -> Result {
        let target = component(c)?;
        let rt = &self.rt;
        let node = &mut self.node;
        rt.block_on(async {
            if node.held_repair(target).is_none() {
                node.advance(RETRY).await;
            }
            if node.held_repair(target).is_none() {
                return Err(DriverError::NotEnabled {
                    action: if ok { "RecRepair" } else { "RepFail" },
                    why: format!("{target:?}'s repair is not parked after the retry tick"),
                });
            }
            node.decide_repair(target, ok).await;
            Ok(())
        })?;
        Ok(())
    }

    /// The post-repair audit runs over the damage the trace's preceding
    /// `Damage(c)` re-applied; the recovery counts the attempt and holds on.
    fn rec_redo(&mut self, c: &str) -> Result {
        component(c)?;
        let node = &self.node;
        self.rt.block_on(node.release_audits());
        Ok(())
    }

    fn rec_restore(&mut self, c: &str) -> Result {
        component(c)?;
        let node = &self.node;
        self.rt.block_on(node.release_audits());
        Ok(())
    }

    /// D-UP-3: the composed owner's `quiesce()`, the hold dropped.
    fn leak_hold(&mut self) -> Result {
        let node = &self.node;
        self.rt.block_on(node.leak_hold());
        Ok(())
    }

    /// D-UP-4 (PD-1): a second ledger's hold restored at the composed owner.
    fn foreign_restore(&mut self) -> Result {
        let node = &self.node;
        let (restored, _issuing_ledger) = self.rt.block_on(node.restore_foreign_hold());
        if restored != Err(QuiescenceRestoreError::ForeignHold) {
            return Err(DriverError::NotEnabled {
                action: "ForeignRestore",
                why: format!("the owner answered a foreign hold with {restored:?}"),
            }
            .into());
        }
        self.foreign = true;
        Ok(())
    }

    fn fail_stop(&mut self, c: &str) -> Result {
        let target = component(c)?;
        let foreign = self.foreign;
        let rt = &self.rt;
        let node = &mut self.node;
        let leaked = rt.block_on(node.owner_observation()).leaked_quiescence_holds;
        let budget_spent = node.recovery_observation().get(&target).is_some_and(|episode| {
            episode.phase == RecoveryPhase::Held && episode.failed_attempts >= 2
        });
        if leaked == 0 && !budget_spent && foreign {
            return Err(DriverError::ForeignFailStopNotDrivable.into());
        }
        let request = rt.block_on(async {
            // A leaked hold: the next audit tick fail-stops (D-UP-3).
            node.advance(SHARED_NETWORK_AUDIT_PERIOD).await;
            node.release_audits().await;
            if let Some(request) = node.fail_stop_requested() {
                return Some(request);
            }
            // A spent budget: the production bound (ADR-0124) is reached with
            // the repair still failing.
            node.advance(SHARED_NETWORK_RECOVERY_DEADLINE).await;
            for _ in 0..FAIL_STOP_TICKS {
                if let Some(request) = node.fail_stop_requested() {
                    return Some(request);
                }
                if node.held_repair(target).is_some() {
                    node.decide_repair(target, false).await;
                }
                node.release_audits().await;
                node.advance(RETRY).await;
            }
            node.fail_stop_requested()
        });
        let Some(ServeShutdownRequest::SharedGuestNetwork(fail_stop)) = request else {
            return Err(DriverError::NotEnabled {
                action: "FailStop",
                why: format!(
                    "no shared-network fail-stop request past {target:?}'s bound: {request:?}"
                ),
            }
            .into());
        };
        let expected = if leaked > 0 {
            SharedGuestNetworkFailStopCause::LeakedQuiescenceHold
        } else {
            SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded
        };
        if fail_stop.cause != expected {
            return Err(DriverError::NotEnabled {
                action: "FailStop",
                why: format!("the fail-stop names {:?}, expected {expected:?}", fail_stop.cause),
            }
            .into());
        }
        rt.block_on(node.end_after_fail_stop());
        self.foreign = false;
        Ok(())
    }
}

impl Driver for QuiescenceDriver {
    type State = QuiescenceState;

    fn config() -> Config {
        Config { state: &[], nondet: &["quiesce_conf::quiescence::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            InitWith(fl: i64) => self.init_with(fl)?,
            InitDown => self.init_down(),
            Damage(c: String) => self.damage(&c)?,
            OpenFlow => self.open_flow()?,
            Crash => self.crash()?,
            RestartWith(writeOk: bool) => self.restart_with(writeOk)?,
            RecQuiesce(c: String) => self.rec_quiesce(&c)?,
            RecRepair(c: String) => self.rec_repair(&c, true)?,
            RepFail(c: String) => self.rec_repair(&c, false)?,
            RecRedo(c: String) => self.rec_redo(&c)?,
            RecRestore(c: String) => self.rec_restore(&c)?,
            LeakHold => self.leak_hold()?,
            ForeignRestore => self.foreign_restore()?,
            FailStop(c: String) => self.fail_stop(&c)?,
        })
    }
}

// ----- the compared state ------------------------------------------------------

/// The compared `quiescence` variables (module doc § Projection).
#[derive(Debug, Clone, PartialEq, Eq)]
enum QuiescenceState {
    /// A `serve` process runs.
    Up {
        cnt: i64,
        latch: bool,
        leaked: i64,
        foreign: bool,
        flows_open: i64,
        rec: BTreeMap<String, String>,
        att: BTreeMap<String, i64>,
        table: bool,
    },
    /// No process runs.
    Down { stop: String, table: bool },
}

/// Why a value has no image in the other vocabulary.
#[derive(Debug, thiserror::Error)]
enum ProjectionError {
    #[error("ITF: {0}")]
    Itf(#[from] ItfError),
    #[error("{0} has no pre-image in the model (the bijection is fixed by the driver)")]
    NoPreImage(String),
}

impl QuiescenceState {
    /// Decodes the compared variables by their qualified names
    /// `quiesce_conf::quiescence::<var>` (D-UP-1, spike QT1); a state of any
    /// other main fails with the missing qualified variable.
    fn from_value(value: &Value) -> std::result::Result<Self, ProjectionError> {
        let vars = Qualified::of(value, MAIN, MODULE)?;
        let table = quint_itf::boolean(vars.var("table")?)?;
        if !quint_itf::boolean(vars.var("up")?)? {
            return Ok(Self::Down { stop: quint_itf::string(vars.var("stop")?)?, table });
        }
        Ok(Self::Up {
            cnt: quint_itf::int(vars.var("cnt")?)?,
            latch: quint_itf::boolean(vars.var("latch")?)?,
            leaked: quint_itf::int(vars.var("leaked")?)?,
            foreign: quint_itf::boolean(vars.var("foreign")?)?,
            flows_open: quint_itf::int(vars.var("flowsOpen")?)?,
            rec: quint_itf::str_map(vars.var("rec")?, quint_itf::string)?,
            att: quint_itf::str_map(vars.var("att")?, quint_itf::int)?,
            table,
        })
    }
}

impl<'de> Deserialize<'de> for QuiescenceState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<QuiescenceDriver> for QuiescenceState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &QuiescenceDriver) -> Result<Self> {
        let node = &driver.node;
        // conformance-lint: sim-arbiter-ok QPROJ-7 shared mTLS table arbiter (boot step 8, one nft transaction leaves Absent or Complete), Tier-3 pair R5-30
        let table = node.table() == SimSharedTable::Complete;
        if node.process.is_none() {
            let stop = match node.last_stop {
                Some(NodeStop::Crash) => "crash",
                Some(NodeStop::FailStop) => "failstop",
                Some(NodeStop::Refused) => "refused",
                None => unreachable!("a node with no process records how its last process ended"),
            };
            return Ok(Self::Down { stop: stop.to_owned(), table });
        }
        let observed = driver.rt.block_on(node.owner_observation());
        let flows_open: usize = observed
            .cids
            .values()
            .map(|cid| cid.guest_flows.len() + cid.dials.len() + cid.datagrams.len())
            .sum();
        let mut rec: BTreeMap<String, String> =
            [("C1", "idle"), ("C2", "idle")].map(|(c, r)| (c.to_owned(), r.to_owned())).into();
        let mut att: BTreeMap<String, i64> =
            [("C1", 0), ("C2", 0)].map(|(c, a)| (c.to_owned(), a)).into();
        for (component, episode) in node.recovery_observation() {
            let name = model_name(component)
                .ok_or_else(|| ProjectionError::NoPreImage(format!("recovery of {component:?}")))?;
            let phase = match episode.phase {
                RecoveryPhase::Held => "held",
                RecoveryPhase::Repaired => "repaired",
            };
            rec.insert(name.to_owned(), phase.to_owned());
            att.insert(name.to_owned(), i64::from(episode.failed_attempts));
        }
        Ok(Self::Up {
            cnt: i64::from(observed.quiescence_holds),
            latch: observed.admission == GuestAdmission::Quiesced,
            leaked: i64::from(observed.leaked_quiescence_holds),
            foreign: driver.foreign,
            flows_open: i64::try_from(flows_open)?,
            rec,
            att,
            table,
        })
    }
}

// ----- the runs ----------------------------------------------------------------
//
// Each `#[quint_test]` pins one named run of the spec's critical scenarios on
// the conformance main `quiesce_conf` with `max_samples = 1` (§ *Named runs
// pinned by each driver*, `quiescence`): every run except
// `ForeignHoldRestoreChangesNothingThenFailStops` (its last steps are the
// supervisor's undrivable `ForeignQuiescenceHold` fail-stop; FHOLD-CHECK).
// `#[quint_run]` replays randomized traces of `quiesce_conf` with the seed
// printed on failure.

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/quiescence.qnt",
    main = "quiesce_conf",
    test = "SingleRecoveryQuiescesRepairsRestores",
    max_samples = 1
)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
fn quiescence_single_recovery_quiesces_repairs_restores() -> impl Driver {
    QuiescenceDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/quiescence.qnt",
    main = "quiesce_conf",
    test = "TwoOverlappingRecoveriesKeepForwardingQuiescedUntilTheLastRestore",
    max_samples = 1
)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
fn quiescence_two_overlapping_recoveries_keep_forwarding_quiesced() -> impl Driver {
    QuiescenceDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/quiescence.qnt",
    main = "quiesce_conf",
    test = "NoFlowOpensWhileCountedHoldRemains",
    max_samples = 1
)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
fn quiescence_no_flow_opens_while_counted_hold_remains() -> impl Driver {
    QuiescenceDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/quiescence.qnt",
    main = "quiesce_conf",
    test = "TwoOverlappingRecoveriesReopenOnlyAtZero",
    max_samples = 1
)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
fn quiescence_two_overlapping_recoveries_reopen_only_at_zero() -> impl Driver {
    QuiescenceDriver::fresh()
}

/// D-UP-3: the `LeakHold` stimulus is the composed owner's `quiesce()` and
/// the drop of its hold (no new seam).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/quiescence.qnt",
    main = "quiesce_conf",
    test = "DroppedHoldKeepsForwardingQuiescedThenFailStops",
    max_samples = 1
)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
fn quiescence_dropped_hold_keeps_forwarding_quiesced_then_fail_stops() -> impl Driver {
    QuiescenceDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/quiescence.qnt",
    main = "quiesce_conf",
    test = "RepairFailureCountsAttemptThenRepairs",
    max_samples = 1
)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
fn quiescence_repair_failure_counts_attempt_then_repairs() -> impl Driver {
    QuiescenceDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/quiescence.qnt",
    main = "quiesce_conf",
    test = "PostRepairAuditFailureCountsAttempt",
    max_samples = 1
)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
fn quiescence_post_repair_audit_failure_counts_attempt() -> impl Driver {
    QuiescenceDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/quiescence.qnt",
    main = "quiesce_conf",
    test = "SpentBudgetFailStopsAndRestartConverges",
    max_samples = 1
)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
fn quiescence_spent_budget_fail_stops_and_restart_converges() -> impl Driver {
    QuiescenceDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/quiescence.qnt",
    main = "quiesce_conf",
    test = "CrashDuringRecoveryThenRestartRecovers",
    max_samples = 1
)]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
fn quiescence_crash_during_recovery_then_restart_recovers() -> impl Driver {
    QuiescenceDriver::fresh()
}

/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/quiescence.qnt",
    main = "quiesce_conf",
    test = "FailedBootTableWriteRefusesBootThenRetrySucceeds",
    max_samples = 1
)]
#[ignore = "pending DELIVER step 12"]
fn quiescence_failed_boot_table_write_refuses_boot_then_retry_succeeds() -> impl Driver {
    QuiescenceDriver::fresh()
}

/// Randomized traces of the conformance main (seed printed on failure).
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/quiescence.qnt", main = "quiesce_conf")]
#[ignore = "pending DELIVER attachment-cut step (12-01 rework)"]
fn quiescence_quiesce_conf_randomized() -> impl Driver {
    QuiescenceDriver::fresh()
}
