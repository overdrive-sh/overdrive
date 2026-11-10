//! Quint conformance driver — `liveness_gate` (service-allocation-readiness-and-ending
//! D9, ADR-0261; B1, ADR-0265; S-9 for this module only).
//!
//! Replays `specs/quint/service-allocation-readiness-and-ending/liveness_gate.qnt`
//! (referenced in place), conformance main `lg_conf`, against the production
//! owners the protocol spans, composed by `liveness_gate_fixture`: the
//! production `VmDriver` holding the real `ProbeRunner` (the owner of the
//! liveness gate), the production `ServiceLifecycle` reconciler through the
//! reconciler runtime and the action shim, with `SimObservationStore`,
//! `SimClock`, `SimGuestStreamDial` and `SimVmm` at the driven ports.
//!
//! Placement (testing.md "Placement and lane"): in the production owner's
//! crate `src/`, behind `#[cfg(all(test, feature = "integration-tests"))]`.
//! The protocol spans `overdrive-worker` (`ProbeRunner`, `VmDriver`),
//! `overdrive-reconcilers` (`ServiceLifecycle`) and the control plane's action
//! shim; `overdrive-worker` does not depend on `overdrive-reconcilers`, so the
//! one crate that composes all three owners, `overdrive-control-plane`, is the
//! home, beside `quint_probe_count` over the same pair of owners.
//!
//! Action -> production call (feature-delta "Pinned conformance projection",
//! "Actions map to", D9):
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | compose the world: Service intent (opt-out startup, liveness `failure_threshold` = the model's `LT`), nothing running |
//! | `OnRunning` | the node starts the allocation: `Running` row, `Driver::on_alloc_running` |
//! | `StableCommit(w)` | `w == "failed"`: the Stable row write is scripted to fail once on `SimObservationStore`; then one `ServiceLifecycle` convergence tick (hydrate, reconcile, view write-through, shim dispatch) with the driver registry emptied so the shim fires no hook (the hook is the `OnStable` step) |
//! | `OnStable` | `Driver::on_alloc_stable` |
//! | `LivenessRow(res)` | the liveness probe dial is scripted `res` and the `SimClock` advances one probe interval so the runner's liveness task writes one row |
//! | `LivenessStop` | one `ServiceLifecycle` tick (hook-less as `StableCommit`); the returned liveness `StopAllocation` is dispatched by the shim and the row must end `Stopped { by: LivenessProbe }` |
//! | `Crash` | the guest crashes: the `Failed` row the exit observer writes (an environment fault) |
//! | `OnTerminal` | `Driver::on_alloc_terminal` |
//! | `Idle` | the runtime is driven to quiescence |
//!
//! Projection (`State::from_driver`): `role` <- `ProbeRunner::is_role_live(alloc,
//! Liveness)`; `sup` <- `ProbeRunner::active_alloc_count()`; `committed` <-
//! `ObservationStore::alloc_status_row(alloc)` (`terminal` is `Stable`), compared
//! only while the row is `Running` (the model's `rw`): the model's `committed`
//! is monotone across a `Crash`, whereas the pinned accessor reads the current
//! row, which the exit observer's `Failed` row replaces. `swr`, `ret`, `hook`,
//! `ended`, `streak`, `since`, `rows`, `started`, `stopBeforeStable` are
//! model-only or inputs (the streak is an input, its effect is the liveness
//! stop the `LivenessStop` handler requires).
//!
//! Every named run of `lg_conf` is pinned with `#[quint_test]`; `#[quint_run]`
//! replays randomized traces. Lane: `integration-tests`, `cargo xtask metal run --`.

#![allow(
    clippy::single_option_map,
    clippy::unnecessary_wraps,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    non_snake_case,
    reason = "a conformance driver asserts its own preconditions; protocol names are prose"
)]

use std::sync::Arc;

use itf::Value;
use overdrive_core::id::AllocationId;
use overdrive_core::observation::ProbeRole;
use overdrive_core::traits::driver::Driver as _;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use overdrive_worker::probe_runner::ProbeRunner;
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tokio::runtime::Runtime;

use super::liveness_gate_fixture::LivenessWorld;
use super::probe_count_fixture::{ProbeSet, RunOutcome, settle};

const MAIN: &str = "lg_conf";
const MODULE: &str = "liveness_gate";
/// The model's `LT`: the liveness `failure_threshold`.
const LT: u32 = 2;

#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("unknown outcome `{0}`")]
    UnknownOutcome(String),
    #[error("`{action}`: production did not do what the model enables: {why}")]
    NotEnabled { action: &'static str, why: String },
    #[error("a convergence tick failed: {0}")]
    Look(String),
    #[error("the trace has no `{0}` variable")]
    MissingVar(String),
    #[error("expected {expected} in the trace, found {found:?}")]
    Shape { expected: &'static str, found: Value },
}

struct LivenessGateDriver {
    runtime: Runtime,
    world: LivenessWorld,
    /// The production owners the projection reads, held directly (the same instances as the
    /// world's): the allocation's probe runner and the observation port the shim and the exit
    /// observer write to.
    runner: Arc<ProbeRunner>,
    obs: Arc<dyn ObservationStore>,
    alloc: AllocationId,
}

impl LivenessGateDriver {
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let world =
            runtime.block_on(LivenessWorld::compose_unstarted(ProbeSet::ReadinessAndLiveness, LT));
        Self {
            runner: Arc::clone(&world.runner),
            obs: Arc::clone(&world.state.obs),
            alloc: world.alloc.clone(),
            runtime,
            world,
        }
    }

    fn world(&mut self) -> (&Runtime, &mut LivenessWorld) {
        (&self.runtime, &mut self.world)
    }

    fn init(&mut self) {
        self.runner.stop_alloc(&self.alloc);
        self.runtime.block_on(settle());
        let world = self
            .runtime
            .block_on(LivenessWorld::compose_unstarted(ProbeSet::ReadinessAndLiveness, LT));
        self.runner = Arc::clone(&world.runner);
        self.obs = Arc::clone(&world.state.obs);
        self.alloc = world.alloc.clone();
        self.world = world;
    }

    fn on_running(&mut self) -> Result {
        let (rt, world) = self.world();
        rt.block_on(world.start_allocation());
        Ok(())
    }

    fn stable_commit(&mut self, w: &str) -> Result {
        let failed = match w {
            "landed" => false,
            "failed" => true,
            other => return Err(DriverError::UnknownOutcome(other.to_owned()).into()),
        };
        let (rt, world) = self.world();
        if failed {
            world.fail_next_write();
        }
        let result = rt.block_on(world.look_without_hooks());
        match (failed, result) {
            (true, Err(_)) | (false, Ok(())) => Ok(()),
            (true, Ok(())) => Err(DriverError::NotEnabled {
                action: "StableCommit(failed)",
                why: "the scripted Stable write failure did not reach the shim's write".to_owned(),
            }
            .into()),
            (false, Err(error)) => Err(DriverError::Look(error.to_string()).into()),
        }
    }

    fn on_stable(&mut self) -> Result {
        let (rt, world) = self.world();
        // `on_alloc_stable` spawns the liveness tasks: it runs inside the runtime
        // context, as the shim's hook call does.
        let entered = rt.enter();
        let alloc = world.alloc.clone();
        world.driver.on_alloc_stable(&alloc);
        // The spawned liveness tasks register their first sleep before the clock
        // is next advanced (tick-then-sleep): quiesce, as `start_allocation` does.
        drop(entered);
        rt.block_on(settle());
        Ok(())
    }

    fn liveness_row(&mut self, res: &str) -> Result {
        let outcome = match res {
            "pass" => RunOutcome::Pass,
            "fail" => RunOutcome::Fail,
            other => return Err(DriverError::UnknownOutcome(other.to_owned()).into()),
        };
        let (rt, world) = self.world();
        rt.block_on(world.period(&outcome, &[]));
        Ok(())
    }

    fn liveness_stop(&mut self) -> Result {
        let (rt, world) = self.world();
        rt.block_on(world.look_without_hooks()).map_err(|e| DriverError::Look(e.to_string()))?;
        let row = rt.block_on(world.alloc_row());
        let stopped = row.is_some_and(|row| {
            matches!(
                row.terminal,
                Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe })
            )
        });
        if stopped {
            Ok(())
        } else {
            Err(DriverError::NotEnabled {
                action: "LivenessStop",
                why: "ServiceLifecycle emitted no liveness stop for a crossed streak".to_owned(),
            }
            .into())
        }
    }

    fn crash(&mut self) -> Result {
        let (rt, world) = self.world();
        rt.block_on(world.crash());
        Ok(())
    }

    fn on_terminal(&mut self) -> Result {
        let (rt, world) = self.world();
        let _entered = rt.enter();
        let alloc = world.alloc.clone();
        world.driver.on_alloc_terminal(&alloc);
        Ok(())
    }

    fn idle(&mut self) -> Result {
        let (rt, _) = self.world();
        rt.block_on(settle());
        Ok(())
    }
}

impl Driver for LivenessGateDriver {
    type State = LivenessGateState;

    fn config() -> Config {
        Config { state: &[], nondet: &["lg_conf::liveness_gate::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init(),
            OnRunning => self.on_running()?,
            StableCommit(w: String) => self.stable_commit(&w)?,
            OnStable => self.on_stable()?,
            LivenessRow(res: String) => self.liveness_row(&res)?,
            LivenessStop => self.liveness_stop()?,
            Crash => self.crash()?,
            OnTerminal => self.on_terminal()?,
            Idle => self.idle()?,
        })
    }
}

/// The compared `liveness_gate` variables.
#[derive(Debug, PartialEq, Eq)]
struct LivenessGateState {
    /// `lv.role == "live"`.
    role_live: bool,
    /// `lv.sup`.
    supervised: bool,
    /// `lv.committed`, compared only while the row is `Running` (`lv.rw`).
    committed_while_running: Option<bool>,
}

impl State<LivenessGateDriver> for LivenessGateState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &LivenessGateDriver) -> Result<Self> {
        let role_live = driver.runner.is_role_live(&driver.alloc, ProbeRole::Liveness);
        let supervised = driver.runner.active_alloc_count() == 1;
        let row = driver.runtime.block_on(driver.obs.alloc_status_row(&driver.alloc))?;
        // DG-3 (feature-delta § Pinned conformance projection, `committed` row): `committed` is compared only while the row is `Running`; after `Crash` it is model-only.
        let committed_while_running = row
            .filter(|row| row.state == AllocState::Running)
            .map(|row| matches!(row.terminal, Some(TerminalCondition::Stable { .. })));
        Ok(Self { role_live, supervised, committed_while_running })
    }
}

impl LivenessGateState {
    fn from_value(state: &Value) -> std::result::Result<Self, DriverError> {
        let Value::Record(record) = state else {
            return Err(DriverError::Shape {
                expected: "a record of variables",
                found: state.clone(),
            });
        };
        let qualified = format!("{MAIN}::{MODULE}::lv");
        let lv = record.get(&qualified).ok_or(DriverError::MissingVar(qualified))?;
        let Value::Record(lv) = lv else {
            return Err(DriverError::Shape { expected: "the `lv` record", found: lv.clone() });
        };
        let field = |name: &str| -> std::result::Result<&Value, DriverError> {
            lv.get(name).ok_or_else(|| DriverError::MissingVar(format!("lv.{name}")))
        };
        let boolean = |name: &str| -> std::result::Result<bool, DriverError> {
            match field(name)? {
                Value::Bool(b) => Ok(*b),
                other => Err(DriverError::Shape { expected: "a bool", found: other.clone() }),
            }
        };
        let role_live = match field("role")? {
            Value::String(s) => s == "live",
            other => return Err(DriverError::Shape { expected: "a string", found: other.clone() }),
        };
        let rw = boolean("rw")?;
        let committed = boolean("committed")?;
        Ok(Self {
            role_live,
            supervised: boolean("sup")?,
            committed_while_running: rw.then_some(committed),
        })
    }
}

impl<'de> Deserialize<'de> for LivenessGateState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------
// Named runs of `lg_conf` (README "Named runs"), then randomized traces.
// ---------------------------------------------------------------------

/// D9: the liveness probe would have failed from the start; no liveness role
/// and no stop before Stable.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/liveness_gate.qnt",
    main = "lg_conf",
    test = "liveness_failing_during_startup_is_not_stopped",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn liveness_gate_failing_during_startup_is_not_stopped() -> impl Driver {
    LivenessGateDriver::new()
}

/// D9: liveness starts at Stable and stops the instance after the threshold.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/liveness_gate.qnt",
    main = "lg_conf",
    test = "liveness_starts_at_stable_and_stops_on_failure",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn liveness_gate_starts_at_stable_and_stops_on_failure() -> impl Driver {
    LivenessGateDriver::new()
}

/// D9: the supervisor ended before the Stable hook fired: no liveness.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/liveness_gate.qnt",
    main = "lg_conf",
    test = "stable_after_end_starts_no_liveness",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn liveness_gate_stable_after_end_starts_no_liveness() -> impl Driver {
    LivenessGateDriver::new()
}

/// B1: a lost Stable write is announced again; once it lands the hook starts
/// liveness.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/liveness_gate.qnt",
    main = "lg_conf",
    test = "lost_stable_write_is_reannounced",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn liveness_gate_lost_stable_write_is_reannounced() -> impl Driver {
    LivenessGateDriver::new()
}

/// D9: an opt-out Service's first Running is its Stable; liveness starts then.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/liveness_gate.qnt",
    main = "lg_conf",
    test = "opt_out_service_starts_liveness_at_first_running_stable",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn liveness_gate_opt_out_service_starts_liveness_at_first_running_stable() -> impl Driver {
    LivenessGateDriver::new()
}

/// Randomized `lg_conf` traces against the production owners.
#[quint_run(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/liveness_gate.qnt",
    main = "lg_conf"
)]
/// CONTRACT_SHAPE: bounded-change.
fn liveness_gate_conformance_lg_conf() -> impl Driver {
    LivenessGateDriver::new()
}
