//! Conformance driver — `progress_deadline` (D10, ADR-0262, ADR-0264), conformance
//! instance `pd_conf` of
//! `specs/quint/service-allocation-readiness-and-ending/progress_deadline.qnt`.
//!
//! # The production owner under test
//!
//! The Service submit stream handler: every stream is opened through the production
//! `submit_workload` handler on the NDJSON lane (`Inserted` for the first stream,
//! `Unchanged` for a redeploy of the same bytes), which computes the stream's
//! progress deadline and hands it to `build_service_stream`. The driver reads each
//! stream's response body frame by frame, polling a stream only at the step that
//! concerns it, so a stream whose timer has elapsed fires exactly at the model's
//! `Fire(s)` step and never earlier.
//!
//! # Action to production call or environment
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | a fresh node; no workload |
//! | `Deploy(sd)` | the workload's intent: a Service whose startup probe has interval `sd x 10 s` and one attempt (startup deadline `sd x 10 s`); its first attempt is written `Running` |
//! | `Open(s)` | `submit_workload` over the NDJSON lane; the stream is polled to its `Accepted` line, its subscription and its wait |
//! | `Advance(t)` | the injected `SimClock` is advanced to model tick `t` (10 s per tick); no stream is polled |
//! | `AttemptEnd(obs)` | the shim's commit then publish for an attempt ending (`startup_stop`: `Terminated`, `Stopped { StartupProbe }`; `crash`: `Failed`, no terminal), then the successor attempt `Running`; no stream is polled (the event waits in each subscription) |
//! | `StableHappens` / `GiveUp` / `OperatorStop` | the shim's commit then publish of the terminal on the current attempt; every open stream is polled |
//! | `Fire(s)` | stream `s` is polled: its own timer arm yields the timeout line |
//! | `Idle` | the model's stutter: no stream has yielded more than one terminal line |
//!
//! # Projection (`State::from_driver`)
//!
//! Per stream slot: `st` (unopened, open, or closed once a terminal line was yielded)
//! and `kind` / `closes` from the terminal lines the production stream yielded
//! (`Failed { Timeout }` → `timeout`, `Stable` → `stable`, any other `Failed` →
//! `giveup`, `Stopped { Operator }` → `operator_stop`); `dl` from the value returned
//! by the production `progress_deadline` for the deployed startup probes (10 s per
//! tick; 0 for an unopened slot); `last` from the `last_attempt` field of the
//! timeout line (`none`, `startup_stop`, `crash`), compared only for a stream that
//! closed as a timeout (`last_attempt` is carried only by that line). The model's
//! `armed` (the tick at open), `verdict` and the world (`w`) are model-only: the
//! stream writes no row, the world is the driver's own input, and `conf_step` is the
//! recorder.

use std::collections::BTreeMap;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, header};
use axum::routing::post;
use futures::FutureExt as _;
use http_body_util::BodyExt as _;
use itf::Value;
use overdrive_core::TransitionReason;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{DriverInput, IntentKey, ResourcesInput, WorkloadIntent};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::id::WorkloadId;
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::traits::intent_store::IntentStore as _;
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::transition_reason::{ServiceFailureReason, StoppedBy, TerminalCondition};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tokio::runtime::Runtime;
use tower::ServiceExt as _;

use super::{Composition, alloc_row, commit, compose, publish};
use crate::api::SubmitWorkloadRequest;
use crate::guest_network::quint_itf::{self, ItfError, Qualified};
use crate::handlers::submit_workload;
use crate::streaming::{ServiceSubmitEvent, progress_deadline};

const MAIN: &str = "pd_conf";
const MODULE: &str = "progress_deadline";
const WORKLOAD: &str = "svc";
/// The slots of the conformance instance (`SLOTS = Set(1, 2)`).
const SLOTS: [i64; 2] = [1, 2];
/// Seconds per model tick.
const TICK_SECONDS: u64 = 10;

/// One opened stream: the response body, polled on demand, and the lines read.
struct Slot {
    body: Body,
    lines: Vec<String>,
    ended: bool,
}

struct ProgressDeadlineDriver {
    rt: Runtime,
    /// The composed node, held directly: the projection reads the deployed intent the
    /// production handler persisted through it.
    node: Composition,
    /// The deployed intent's startup probes (what `progress_deadline` is derived from).
    probes: Vec<ProbeDescriptor>,
    slots: BTreeMap<i64, Slot>,
    model_tick: i64,
    attempt: u32,
    counter: u64,
}

impl ProgressDeadlineDriver {
    fn new() -> Self {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let node = rt.block_on(compose(2910));
        Self {
            rt,
            node,
            probes: Vec::new(),
            slots: BTreeMap::new(),
            model_tick: 0,
            attempt: 0,
            counter: 0,
        }
    }

    fn node(&self) -> &Composition {
        &self.node
    }

    fn init(&mut self) {
        eprintln!("quint-conformance progress_deadline seed=2910");
        self.node = self.rt.block_on(compose(2910));
        self.probes.clear();
        self.slots.clear();
        self.model_tick = 0;
        self.attempt = 0;
        self.counter = 0;
    }

    fn spec(&self) -> ServiceSpecInput {
        ServiceSpecInput {
            id: WORKLOAD.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
                command: "/bin/sleep".into(),
                args: vec!["3600".into()],
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
            listeners: vec![ListenerInput { port: 18081, protocol: "tcp".into() }],
            startup_probes: self.probes.clone(),
            readiness_probes: vec![],
            liveness_probes: vec![],
        }
    }

    /// The startup probes of the deployed Service as the production handler persisted
    /// them (the intent the stream's progress deadline is derived from).
    fn deployed_startup_probes(&self) -> Result<Vec<ProbeDescriptor>> {
        let key = IntentKey::for_workload(&WorkloadId::new(WORKLOAD)?);
        let bytes = self
            .rt
            .block_on(self.node.state.store.get(key.as_bytes()))?
            .ok_or_else(|| anyhow::anyhow!("no workload intent persisted: no stream was opened"))?;
        let intent = WorkloadIntent::from_store_bytes(
            &bytes,
            &self.node.state.intent_redb_path,
            Some(key.as_str()),
        )?;
        match intent {
            WorkloadIntent::Service(service) => Ok(service.startup_probes),
            other => anyhow::bail!("the persisted intent is not a Service: {other:?}"),
        }
    }

    fn next_counter(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    fn deploy(&mut self, sd: i64) -> Result {
        let interval = u32::try_from(sd)? * u32::try_from(TICK_SECONDS)?;
        self.probes = vec![ProbeDescriptor {
            idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".into(), port: 18999 },
            timeout_seconds: 1,
            interval_seconds: interval,
            max_attempts: 1,
            failure_threshold: None,
            success_threshold: None,
        }];
        let counter = self.next_counter();
        let running = alloc_row(
            WORKLOAD,
            self.attempt,
            counter,
            AllocState::Running,
            TransitionReason::Started,
            None,
        );
        self.rt.block_on(commit(self.node(), &running));
        Ok(())
    }

    fn open(&mut self, s: i64) -> Result {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/v1/workloads")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/x-ndjson")
            .body(Body::from(serde_json::to_vec(&SubmitWorkloadRequest {
                spec: SubmitSpecInput::Service(self.spec()),
            })?))?;
        let router = Router::new()
            .route("/v1/workloads", post(submit_workload))
            .with_state(self.node().state.clone());
        let response = self.rt.block_on(router.oneshot(request))?;
        let mut slot = Slot { body: response.into_body(), lines: Vec::new(), ended: false };
        self.rt.block_on(async { poll_slot(&mut slot) })?;
        self.slots.insert(s, slot);
        Ok(())
    }

    fn advance(&mut self, t: i64) {
        let delta = u64::try_from(t - self.model_tick).expect("the model advances forward");
        self.node().clock.tick(std::time::Duration::from_secs(delta * TICK_SECONDS));
        self.model_tick = t;
    }

    /// The shim's commit then publish for an attempt ending, then the successor.
    fn attempt_end(&mut self, obs: &str) -> Result {
        let (state, reason, terminal) = match obs {
            "startup_stop" => (
                AllocState::Terminated,
                TransitionReason::Stopped { by: StoppedBy::StartupProbe },
                Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
            ),
            "crash" => (
                AllocState::Failed,
                TransitionReason::WorkloadCrashedImmediately {
                    exit_code: Some(1),
                    signal: None,
                    stderr_tail: None,
                },
                None,
            ),
            other => anyhow::bail!("unmodelled attempt-ending observation `{other}`"),
        };
        let counter = self.next_counter();
        let ended = alloc_row(WORKLOAD, self.attempt, counter, state, reason, terminal);
        self.rt.block_on(commit(self.node(), &ended));
        publish(self.node(), &ended);
        self.attempt += 1;
        let counter = self.next_counter();
        let successor = alloc_row(
            WORKLOAD,
            self.attempt,
            counter,
            AllocState::Running,
            TransitionReason::Started,
            None,
        );
        self.rt.block_on(commit(self.node(), &successor));
        Ok(())
    }

    /// The shim's commit then publish of a terminal on the current attempt; every open
    /// stream then consumes its event.
    fn terminal(&mut self, state: AllocState, terminal: TerminalCondition) -> Result {
        let counter = self.next_counter();
        let row = alloc_row(
            WORKLOAD,
            self.attempt,
            counter,
            state,
            TransitionReason::Started,
            Some(terminal),
        );
        self.rt.block_on(commit(self.node(), &row));
        publish(self.node(), &row);
        self.poll_all_open()
    }

    fn poll_all_open(&mut self) -> Result {
        for slot in self.slots.values_mut() {
            if slot.lines.len() < 2 && !slot.ended {
                self.rt.block_on(async { poll_slot(slot) })?;
            }
        }
        Ok(())
    }

    fn fire(&mut self, s: i64) -> Result {
        let slot =
            self.slots.get_mut(&s).ok_or_else(|| anyhow::anyhow!("stream {s} is not open"))?;
        self.rt.block_on(async { poll_slot(slot) })?;
        match terminal_event(slot)? {
            Some(ServiceSubmitEvent::Failed {
                reason: ServiceFailureReason::Timeout { .. },
                ..
            }) => Ok(()),
            other => {
                anyhow::bail!("stream {s}: its own timer did not yield a timeout; got {other:?}")
            }
        }
    }

    fn idle(&self) -> Result {
        for (s, slot) in &self.slots {
            anyhow::ensure!(
                slot.lines.len() <= 2,
                "stream {s} yielded more than one terminal line (StreamEndsOnceHonestly)",
            );
        }
        Ok(())
    }

    fn step_on(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init(),
            Deploy(sd: i64) => self.deploy(sd)?,
            Open(s: i64) => self.open(s)?,
            Advance(t: i64) => self.advance(t),
            AttemptEnd(obs: String) => self.attempt_end(&obs)?,
            StableHappens => self.terminal(
                AllocState::Running,
                TerminalCondition::Stable {
                    settled_in_ms: 700,
                    witness: overdrive_core::transition_reason::ProbeWitness {
                        probe_idx: 0,
                        role: "startup".to_owned(),
                        mechanic_summary: "tcp 127.0.0.1:18999".to_owned(),
                        inferred: false,
                    },
                },
            )?,
            GiveUp => self.terminal(
                AllocState::Failed,
                TerminalCondition::BackoffExhausted { attempts: 5 },
            )?,
            OperatorStop => self.terminal(
                AllocState::Terminated,
                TerminalCondition::Stopped { by: StoppedBy::Operator },
            )?,
            Fire(s: i64) => self.fire(s)?,
            Idle => self.idle()?,
        })
    }
}

/// Poll the stream's response body until it is pending or ended, keeping each line.
fn poll_slot(slot: &mut Slot) -> Result {
    loop {
        if slot.ended {
            return Ok(());
        }
        match slot.body.frame().now_or_never() {
            None => return Ok(()),
            Some(None) => {
                slot.ended = true;
                return Ok(());
            }
            Some(Some(Err(error))) => anyhow::bail!("the stream body failed: {error}"),
            Some(Some(Ok(frame))) => {
                if let Ok(data) = frame.into_data() {
                    for line in String::from_utf8(data.to_vec())?.lines() {
                        if !line.is_empty() {
                            slot.lines.push(line.to_owned());
                        }
                    }
                }
            }
        }
    }
}

/// The terminal line a stream yielded (the line after `Accepted`), if any.
fn terminal_event(slot: &Slot) -> Result<Option<ServiceSubmitEvent>> {
    slot.lines.get(1).map(|line| Ok(serde_json::from_str(line)?)).transpose()
}

impl Driver for ProgressDeadlineDriver {
    type State = ProgressDeadlineState;

    fn config() -> Config {
        Config { state: &[], nondet: &["pd_conf::progress_deadline::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        self.step_on(step)
    }
}

// ----- the compared state ------------------------------------------------------

/// One stream slot's compared fields.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamView {
    st: String,
    kind: String,
    dl: i64,
    last: String,
    closes: i64,
}

#[derive(Debug, PartialEq)]
struct ProgressDeadlineState {
    sm: BTreeMap<i64, StreamView>,
}

impl ProgressDeadlineState {
    fn from_value(value: &Value) -> std::result::Result<Self, ItfError> {
        let vars = Qualified::of(value, MAIN, MODULE)?;
        let sm = quint_itf::int_map(vars.var("sm")?, |entry| {
            let kind = quint_itf::string(quint_itf::field(entry, "kind")?)?;
            // `last` is compared only for a stream that closed as a timeout: only the
            // synthesised timeout line carries `last_attempt`.
            let last = quint_itf::string(quint_itf::field(entry, "last")?)?;
            Ok(StreamView {
                st: quint_itf::string(quint_itf::field(entry, "st")?)?,
                last: if kind == "timeout" { last } else { String::new() },
                kind,
                dl: quint_itf::int(quint_itf::field(entry, "dl")?)?,
                closes: quint_itf::int(quint_itf::field(entry, "closes")?)?,
            })
        })?;
        Ok(Self { sm })
    }
}

impl<'de> Deserialize<'de> for ProgressDeadlineState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<ProgressDeadlineDriver> for ProgressDeadlineState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &ProgressDeadlineDriver) -> Result<Self> {
        let mut sm = BTreeMap::new();
        for s in SLOTS {
            let view = match driver.slots.get(&s) {
                None => StreamView {
                    st: "unopened".to_owned(),
                    kind: String::new(),
                    dl: 0,
                    last: String::new(),
                    closes: 0,
                },
                Some(slot) => {
                    let probes = driver.deployed_startup_probes()?;
                    let dl = i64::try_from(
                        progress_deadline(&probes, crate::DEFAULT_STREAMING_CAP).as_secs()
                            / TICK_SECONDS,
                    )?;
                    match terminal_event(slot)? {
                        None => StreamView {
                            st: "open".to_owned(),
                            kind: String::new(),
                            dl,
                            last: String::new(),
                            closes: 0,
                        },
                        Some(event) => {
                            let (kind, last) = classify(&event)?;
                            StreamView {
                                st: "closed".to_owned(),
                                kind: kind.to_owned(),
                                dl,
                                last: last.to_owned(),
                                closes: i64::try_from(slot.lines.len() - 1)?,
                            }
                        }
                    }
                }
            };
            sm.insert(s, view);
        }
        Ok(Self { sm })
    }
}

/// The model's closing kind of a terminal line, and the `last_attempt` class of a
/// timeout line (`""` for every other line).
fn classify(event: &ServiceSubmitEvent) -> Result<(&'static str, &'static str)> {
    match event {
        ServiceSubmitEvent::Stable { .. } => Ok(("stable", "")),
        ServiceSubmitEvent::Stopped { by: StoppedBy::Operator, .. } => Ok(("operator_stop", "")),
        ServiceSubmitEvent::Failed {
            reason: ServiceFailureReason::Timeout { .. },
            last_attempt,
            ..
        } => {
            let last = match last_attempt {
                None => "none",
                Some(attempt) => match (&attempt.terminal, &attempt.reason) {
                    (Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }), _) => {
                        "startup_stop"
                    }
                    (None, TransitionReason::WorkloadCrashedImmediately { .. }) => "crash",
                    other => anyhow::bail!("an attempt class the model does not have: {other:?}"),
                },
            };
            Ok(("timeout", last))
        }
        ServiceSubmitEvent::Failed { .. } => Ok(("giveup", "")),
        other => anyhow::bail!("a terminal line the model does not have: {other:?}"),
    }
}

// ----- the runs ----------------------------------------------------------------
//
// One `#[quint_test]` per named run of `pd_conf` (README § "Named runs"), plus
// `#[quint_run]` randomized traces (seed printed on failure). Each is ignored until
// D10 lands: the projection reads the production `progress_deadline`, whose body is
// the scaffold.

/// Named run `slow_start_within_startup_deadline_reaches_stable_not_timeout` (D10).
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/progress_deadline.qnt",
    main = "pd_conf",
    test = "slow_start_within_startup_deadline_reaches_stable_not_timeout",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_progress_deadline_slow_start_reaches_stable_not_timeout() -> impl Driver {
    ProgressDeadlineDriver::new()
}

/// Named run `never_ready_times_out_with_last_attempt_cause` (D10).
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/progress_deadline.qnt",
    main = "pd_conf",
    test = "never_ready_times_out_with_last_attempt_cause",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_progress_deadline_never_ready_times_out_with_last_attempt_cause() -> impl Driver {
    ProgressDeadlineDriver::new()
}

/// Named run `giveup_inside_deadline_wins_over_timeout` (D10).
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/progress_deadline.qnt",
    main = "pd_conf",
    test = "giveup_inside_deadline_wins_over_timeout",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_progress_deadline_giveup_inside_deadline_wins_over_timeout() -> impl Driver {
    ProgressDeadlineDriver::new()
}

/// Named run `redeploy_unchanged_opens_new_clock` (D10).
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/progress_deadline.qnt",
    main = "pd_conf",
    test = "redeploy_unchanged_opens_new_clock",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_progress_deadline_redeploy_unchanged_opens_new_clock() -> impl Driver {
    ProgressDeadlineDriver::new()
}

/// Randomized traces of `pd_conf` (seed printed on failure).
#[quint_run(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/progress_deadline.qnt",
    main = "pd_conf"
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_progress_deadline_randomized() -> impl Driver {
    ProgressDeadlineDriver::new()
}
