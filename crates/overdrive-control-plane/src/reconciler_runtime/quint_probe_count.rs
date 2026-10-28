//! Quint conformance driver — `probe-count` (netns-density-295:
//! PROBE-RUN-COUNT with D-1, ADR-0191; PROBE-NOSAMPLE, ADR-0187).
//!
//! Replays `specs/quint/probe-count/probe_count.qnt`, conformance main
//! `probe_streak_conf` (the configuration of `probe_streak`, recording
//! `conf_step`; D-UP-1), against the production owners the protocol spans,
//! composed by the reconciler runtime (`probe_count_fixture`): the probe
//! runner's probe tasks (`ProbeRunner::start_alloc`, the producer) and
//! `ServiceLifecycle` driven through `run_convergence_tick` (the consumer),
//! with `SimObservationStore`, `SimClock`, `SimGuestStreamDial` and
//! `SimDriver` at the driven ports. The spec is referenced in place.
//!
//! One model key, two production keys (feature-delta § *Formal protocol
//! model* → "Probe-run counting", conformance): the model's key carries both
//! verdicts, so the driver maps it onto the liveness probe 0 and the
//! readiness probe 0 of one VM allocation, scripted with the same outcome at
//! every run; both keys therefore write identical `run_seq` / status /
//! `same_status_runs` sequences, and the readiness row must equal the
//! liveness row's three values at every step.
//!
//! Action → production call or fault (feature-delta § "Probe-run counting",
//! action map):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` (initial state) | compose the world: the Service intent (readiness + liveness, startup opt-out), the allocation Running, its probe tasks started; one look announces Stable before any run |
//! | `ProbeRun(Pass \| Fail, _)` | `SimGuestStreamDial` scripted for both probe dials (`Pass` → `Ok`, `Fail` → `Refused { ECONNREFUSED }`), then `SimClock::tick` by the probe interval so each task runs one `probe_tick` |
//! | `ProbeRun(NoSample, node_capacity \| quiesced)` | both probe dials refused `Capacity` / `Quiesced`, then the tick |
//! | `ProbeRun(WriteFail, _)` | `SimObservationStore::inject_write_failure` once per task, then the tick |
//! | `ConsumerLook(stopOk)` | `SimDriver` scripted (`stopOk`: `stop` completes; otherwise `stop` returns an error other than `NotFound`), then `run_convergence_tick(service-lifecycle)` — hydrate → reconcile → view write-through → dispatch, awaited |
//! | `ClockBack` | `SimClock::step_wall_clock_back` by two probe intervals |
//! | `Idle` | the driver stutters: no production call; the runtime is driven to quiescence so any in-flight probe task finishes, and the comparison checks that nothing changed |
//!
//! `ProbeRun(NoSample, adapter_error)` does not occur in the spec's traces
//! (`probe_count.qnt` draws the no-sample `reason` only from
//! `node_capacity` and `quiesced`).
//!
//! Projection (`State::from_driver`), per the pinned table:
//! `rseq` / `rst` / `rsame` ← `ObservationStore::list_probe_results_for_alloc`
//! (the `(Liveness, ProbeIdx(0))` row's `run_seq.get()`, status,
//! `same_status_runs.get()`; no row → 0 / "" / 0) — `SimObservationStore` is
//! the named arbiter of the stored row, paired with the real-adapter
//! R5-PROBE-MERGE (`overdrive-store-local`); `A` ←
//! `ReconcilerRuntime::loaded_service_lifecycle_views_for_test`, the target's
//! `ServiceLifecycleView::liveness_last_acted_run[(alloc, ProbeIdx(0))]`
//! (absent → 0); `running` ← `ObservationStore::alloc_status_rows()`, false
//! iff the allocation's row carries `Stopped { by: LivenessProbe }` (same
//! arbiter). `rat`, `clock`, `pseq`, `pst`, `psame`, `runs`, `hist`, `g`,
//! `terms`, `tseqs`, `fs`, `M` are model-only and not compared; the terminate
//! count's production evidence is R5-PROBE-COUNT's stop-call oracle.
//!
//! Step decoding (D-UP-1, spike QT1): `Config::nondet =
//! &["probe_streak_conf::probe_count::conf_step"]` (one element, the
//! qualified ITF name); the state projection matches the qualified variable
//! names `probe_streak_conf::probe_count::<var>`.
//!
//! The owner of `A` is held directly: the driver keeps the composition's
//! `ReconcilerRuntime` (the production runtime whose in-memory
//! `ServiceLifecycleView` the pinned accessor reads), replaced at every
//! `init` by the new world's runtime. The world fixture itself (the Sim
//! adapters and the started probe tasks) is stepping support; the stored
//! rows are `SimObservationStore`'s, the named arbiter.
//!
//! Every named run that encodes a critical scenario is pinned with
//! `#[quint_test(max_samples = 1)]` (a named run is deterministic: one
//! sample replays it); `#[quint_run]` replays randomized traces (seed printed
//! on failure).

#![allow(
    clippy::single_option_map,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    non_snake_case,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose; the spec's nondet pick is named `stopOk`"
)]

use std::sync::Arc;

use itf::Value;
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::ReconcilerName;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tokio::runtime::Runtime;

use super::ReconcilerRuntime;
use super::probe_count_fixture::{PROBE_INTERVAL, ProbeSet, ProbeWorld, RunOutcome, settle};

/// The conformance main this driver replays (D-UP-1).
const MAIN: &str = "probe_streak_conf";
/// The module that declares the compared variables.
const MODULE: &str = "probe_count";

/// Why a driver step cannot proceed.
#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("the driver has no composed world yet (no `init`)")]
    NoWorld,
    #[error("`ProbeRun` outcome `{0}` is not one of Pass | Fail | NoSample | WriteFail")]
    UnknownOutcome(String),
    #[error(
        "`ProbeRun(NoSample, {0:?})`: the spec draws no-sample reasons only from node_capacity | quiesced"
    )]
    UnknownNoSampleReason(String),
    #[error("a look's convergence tick failed: {0}")]
    Look(String),
    #[error(
        "the readiness row diverged from the liveness row (both keys are scripted alike): \
         liveness {liveness:?}, readiness {readiness:?}"
    )]
    KeysDiverged { liveness: Option<(u64, String, u64)>, readiness: Option<(u64, String, u64)> },
}

struct ProbeCountDriver {
    runtime: Runtime,
    /// The production reconciler runtime of the current world: the owner of
    /// the `ServiceLifecycleView` that `A` projects (the pinned accessor
    /// `loaded_service_lifecycle_views_for_test`). Before the first `init`
    /// it is an empty runtime over its own view store, never projected.
    reconcilers: Arc<ReconcilerRuntime>,
    /// The view store directory of the pre-`init` runtime.
    _views: tempfile::TempDir,
    world: Option<ProbeWorld>,
}

impl ProbeCountDriver {
    fn new() -> Self {
        let views = tempfile::TempDir::new().expect("tempdir");
        let reconcilers = Arc::new(
            ReconcilerRuntime::new_with_redb_view_store_for_test(views.path())
                .expect("reconciler runtime"),
        );
        Self {
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime"),
            reconcilers,
            _views: views,
            world: None,
        }
    }

    fn world(&mut self) -> std::result::Result<&mut ProbeWorld, DriverError> {
        self.world.as_mut().ok_or(DriverError::NoWorld)
    }

    fn init(&mut self) {
        // A fresh composition per trace. The previous trace's probe tasks are
        // cancelled cooperatively through the runner before its world drops.
        if let Some(previous) = self.world.take() {
            previous.runner.stop_alloc(&previous.alloc);
            self.runtime.block_on(settle());
        }
        let world = self.runtime.block_on(ProbeWorld::compose(ProbeSet::ReadinessAndLiveness));
        self.reconcilers = Arc::clone(&world.state.runtime);
        self.world = Some(world);
    }

    /// `ProbeRun(out, reason)`: `reason` is drawn on every run and read only
    /// for `NoSample`.
    fn probe_run(&mut self, out: &str, reason: &str) -> Result {
        let outcome = match out {
            "Pass" => RunOutcome::Pass,
            "Fail" => RunOutcome::Fail,
            "WriteFail" => RunOutcome::WriteFail,
            "NoSample" => match reason {
                "node_capacity" => RunOutcome::NoSampleCapacity,
                "quiesced" => RunOutcome::NoSampleQuiesced,
                other => {
                    return Err(DriverError::UnknownNoSampleReason(other.to_owned()).into());
                }
            },
            other => return Err(DriverError::UnknownOutcome(other.to_owned()).into()),
        };
        let runtime = &self.runtime;
        let world = self.world.as_mut().ok_or(DriverError::NoWorld)?;
        runtime.block_on(world.run(outcome));
        Ok(())
    }

    fn consumer_look(&mut self, stop_ok: bool) -> Result {
        let runtime = &self.runtime;
        let world = self.world.as_mut().ok_or(DriverError::NoWorld)?;
        runtime
            .block_on(world.look(stop_ok))
            .map_err(|error| DriverError::Look(error.to_string()))?;
        Ok(())
    }

    fn clock_back(&mut self) -> Result {
        let world = self.world()?;
        world.clock.step_wall_clock_back(PROBE_INTERVAL * 2);
        Ok(())
    }

    fn idle(&mut self) -> Result {
        let _world = self.world()?;
        self.runtime.block_on(settle());
        Ok(())
    }
}

impl Driver for ProbeCountDriver {
    type State = ProbeCountState;

    fn config() -> Config {
        Config { state: &[], nondet: &["probe_streak_conf::probe_count::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            // The initial state's recorded step is `init` (`probe_count_conf::ConfStep`).
            init => self.init(),
            ProbeRun(out: String, reason: String) => self.probe_run(&out, &reason)?,
            ConsumerLook(stopOk: bool) => self.consumer_look(stopOk)?,
            ClockBack => self.clock_back()?,
            Idle => self.idle()?,
        })
    }
}

/// The compared `probe_count` variables.
#[derive(Debug, PartialEq, Eq)]
struct ProbeCountState {
    rseq: i64,
    rst: String,
    rsame: i64,
    A: i64,
    running: bool,
}

/// `(run_seq, status, same_status_runs)` in the spec's vocabulary.
fn row_values(row: Option<&ProbeResultRow>) -> Option<(u64, String, u64)> {
    row.map(|row| {
        let status = match row.status {
            ProbeStatus::Pass => "Pass",
            ProbeStatus::Fail { .. } => "Fail",
        };
        (row.run_seq.get(), status.to_owned(), u64::from(row.same_status_runs.get()))
    })
}

fn as_i64(value: u64) -> i64 {
    i64::try_from(value).expect("model-bounded counter fits i64")
}

impl State<ProbeCountDriver> for ProbeCountState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &ProbeCountDriver) -> Result<Self> {
        let world = driver.world.as_ref().ok_or(DriverError::NoWorld)?;
        // conformance-lint: sim-arbiter-ok SimObservationStore is the named arbiter of the stored probe row (feature-delta "Probe-run counting" projection), paired with R5-PROBE-MERGE on LocalObservationStore
        let rows = driver.runtime.block_on(world.obs.list_probe_results_for_alloc(&world.alloc))?;
        let of = |role: ProbeRole| {
            rows.iter().find(|row| row.role == role && row.probe_idx == ProbeIdx::new(0))
        };
        let liveness = row_values(of(ProbeRole::Liveness));
        let readiness = row_values(of(ProbeRole::Readiness));
        if liveness != readiness {
            return Err(DriverError::KeysDiverged { liveness, readiness }.into());
        }
        // conformance-lint: sim-arbiter-ok SimObservationStore is the named arbiter of the allocation row the shim's stop dispatch wrote (feature-delta "Probe-run counting" projection, `running`), paired with R5-PROBE-MERGE on LocalObservationStore
        let alloc_rows = driver.runtime.block_on(world.obs.alloc_status_rows())?;
        let running =
            alloc_rows.iter().find(|row| row.alloc_id == world.alloc).is_some_and(|row| {
                !matches!(
                    row.terminal,
                    Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe })
                )
            });
        // The pinned owner accessor for `A`: the runtime's in-memory
        // `ServiceLifecycleView` of the target.
        let acted = driver
            .reconcilers
            .loaded_service_lifecycle_views_for_test(&ReconcilerName::new("service-lifecycle")?)
            .and_then(|views| views.get(&world.target).cloned())
            .and_then(|view| {
                view.liveness_last_acted_run.get(&(world.alloc.clone(), ProbeIdx::new(0))).copied()
            })
            .map_or(0, |seq| as_i64(seq.get()));
        let (rseq, rst, rsame) = liveness.unwrap_or((0, String::new(), 0));
        Ok(Self { rseq: as_i64(rseq), rst, rsame: as_i64(rsame), A: acted, running })
    }
}

/// The spec side: the qualified variables `probe_streak_conf::probe_count::*`
/// (D-UP-1, spike QT1 caveat 1: the trace qualifies every variable with the
/// main and its declaring module; the projection matches the full name).
impl ProbeCountState {
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
        let int = |name: &str| -> std::result::Result<i64, ItfError> {
            let value = var(name)?;
            match value {
                Value::Number(n) => Ok(*n),
                Value::BigInt(big) => i64::try_from(big.get().clone()).map_err(|_| {
                    ItfError::Shape { expected: "a 64-bit int", found: value.clone() }
                }),
                other => Err(ItfError::Shape { expected: "an int", found: other.clone() }),
            }
        };
        let text = |name: &str| -> std::result::Result<String, ItfError> {
            match var(name)? {
                Value::String(s) => Ok(s.clone()),
                other => Err(ItfError::Shape { expected: "a string", found: other.clone() }),
            }
        };
        let boolean = |name: &str| -> std::result::Result<bool, ItfError> {
            match var(name)? {
                Value::Bool(b) => Ok(*b),
                other => Err(ItfError::Shape { expected: "a bool", found: other.clone() }),
            }
        };
        Ok(Self {
            rseq: int("rseq")?,
            rst: text("rst")?,
            rsame: int("rsame")?,
            A: int("A")?,
            running: boolean("running")?,
        })
    }
}

impl<'de> Deserialize<'de> for ProbeCountState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

/// Why a trace value does not have the shape the driver compares.
#[derive(Debug, thiserror::Error)]
enum ItfError {
    #[error("the trace has no `{0}` variable")]
    MissingVar(String),
    #[error("expected {expected} in the trace, found {found:?}")]
    Shape { expected: &'static str, found: Value },
}

// ---------------------------------------------------------------------
// Named runs — one per critical scenario of the `Formal protocol model`
// section, then randomized traces.
// ---------------------------------------------------------------------

/// Three `Fail` runs, a look after each: terminate at the threshold; the
/// stop succeeds.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/probe-count/probe_count.qnt",
    main = "probe_streak_conf",
    test = "HappyPathThreeFailsTerminateAtThreshold",
    max_samples = 1
)]
fn probe_count_happy_path_three_fails_terminate_at_threshold() -> impl Driver {
    ProbeCountDriver::new()
}

/// Three `Fail` runs, then one look: terminate on the first look.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/probe-count/probe_count.qnt",
    main = "probe_streak_conf",
    test = "CoalescedFailsTerminateOnFirstLook",
    max_samples = 1
)]
fn probe_count_coalesced_fails_terminate_on_first_look() -> impl Driver {
    ProbeCountDriver::new()
}

/// `NoSample` / `WriteFail` runs between `Fail` runs change nothing; the run
/// number is reused.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/probe-count/probe_count.qnt",
    main = "probe_streak_conf",
    test = "MissingSampleChangesNothing",
    max_samples = 1
)]
fn probe_count_missing_sample_changes_nothing() -> impl Driver {
    ProbeCountDriver::new()
}

/// A re-read of the acted row after a failed stop does not re-terminate.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/probe-count/probe_count.qnt",
    main = "probe_streak_conf",
    test = "ReReadAfterFailedStopDoesNotReTerminate",
    max_samples = 1
)]
fn probe_count_re_read_after_failed_stop_does_not_re_terminate() -> impl Driver {
    ProbeCountDriver::new()
}

/// A failed stop is retried on the next crossed row; nothing after a stop
/// that succeeded.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/probe-count/probe_count.qnt",
    main = "probe_streak_conf",
    test = "FailedStopRetriedOnNextRow",
    max_samples = 1
)]
fn probe_count_failed_stop_retried_on_next_row() -> impl Driver {
    ProbeCountDriver::new()
}

/// After a failed stop, a `Pass` and a new crossed streak terminate again.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/probe-count/probe_count.qnt",
    main = "probe_streak_conf",
    test = "NewStreakAfterPassTerminatesAgain",
    max_samples = 1
)]
fn probe_count_new_streak_after_pass_terminates_again() -> impl Driver {
    ProbeCountDriver::new()
}

/// The status changes between two looks: no early terminate.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/probe-count/probe_count.qnt",
    main = "probe_streak_conf",
    test = "FlipBetweenLooksNoEarlyTerminate",
    max_samples = 1
)]
fn probe_count_flip_between_looks_no_early_terminate() -> impl Driver {
    ProbeCountDriver::new()
}

/// Readiness reaches its threshold, then is lost on a `Fail`.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/probe-count/probe_count.qnt",
    main = "probe_streak_conf",
    test = "ReadinessReachesThreshold",
    max_samples = 1
)]
fn probe_count_readiness_reaches_threshold() -> impl Driver {
    ProbeCountDriver::new()
}

/// A backward clock step loses no row (merge by `run_seq`).
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/probe-count/probe_count.qnt",
    main = "probe_streak_conf",
    test = "ClockStepBackDoesNotLoseRow",
    max_samples = 1
)]
fn probe_count_clock_step_back_does_not_lose_row() -> impl Driver {
    ProbeCountDriver::new()
}

/// Randomized `probe_streak_conf` traces against the production owners.
/// CONTRACT_SHAPE: bounded-change.
#[quint_run(spec = "../../specs/quint/probe-count/probe_count.qnt", main = "probe_streak_conf")]
fn probe_count_conformance_probe_streak_conf() -> impl Driver {
    ProbeCountDriver::new()
}
