//! Conformance driver — `stream_snapshot` (D11, ADR-0263), conformance instance
//! `ss_conf` of
//! `specs/quint/service-allocation-readiness-and-ending/stream_snapshot.qnt`.
//!
//! # The production owner under test
//!
//! `build_service_stream`, the Service submit stream. The driver holds the stream and
//! polls it by hand (`now_or_never`), so the stream's own steps happen when
//! the model's `Subscribe`, `Read`, `Deliver` and `Relist` steps do; the environment
//! (the shim's row commit and event publish, a lagging bus, a fresh-id restart) acts
//! on the `SimObservationStore` and the lifecycle bus at scripted points relative to
//! the stream's polls.
//!
//! # Action to production call or environment
//!
//! | Action | Driver |
//! |---|---|
//! | `init` | a fresh node; the stream is built (nothing polled) |
//! | `Subscribe` | the stream yields `Accepted`, then its next poll creates the real bus receiver and waits at the test-local observation-port gate before scanning rows |
//! | `Read(ok)` | the observation-port gate releases; the stream reads the recorded verdict and ends with it, waits, or (for `ok = false`, the `SimObservationStore`'s injected `alloc_status_rows` fault) ends with a stream error |
//! | `Commit(a, t)` | the shim's row commit for attempt `a` (`stable`: `Running` + `Stable`; `startup_stop` / `liveness_stop`: `Terminated` + the probe stop; `giveup`: `Failed` + `BackoffExhausted`; `operator_stop`: `Terminated` + `Stopped { Operator }`) |
//! | `Publish(a)` | the shim's event for the committed write on the lifecycle bus |
//! | `Deliver(a)` | the stream is polled: it drains its ready FIFO from the oldest event `a` until a verdict or Pending, as the model's one Deliver step does |
//! | `Lag` | the bus is overflowed with other workloads' events, so the subscription's undelivered events are dropped and its next receive is `Lagged` |
//! | `Relist(ok)` | the stream is polled: its `Lagged` receive repeats the read (`ok = false`: the injected fault) |
//! | `Restart` | the shim's commit of the fresh-id successor attempt `Running` (the predecessor's event was published first, SS-6) |
//! | `Idle` | the model's stutter: the stream has yielded at most one closing outcome |
//!
//! # Projection (`State::from_driver`) — DG-4
//!
//! The compared state is the closing `AllocStatusRow`, read through the existing
//! `ObservationStore::alloc_status_row` on the composed node's store (a production-typed,
//! non-`Option` driver field). `kind` and `closeAtt` come from that row: `Running` +
//! `Stable` → `stable`, `Failed` + `BackoffExhausted` → `giveup`, `Terminated` +
//! `Stopped { Operator }` → `operator_stop`; `closeAtt` is the attempt in the row's
//! allocation id. The line `build_service_stream` yields is an input and an oracle, not a
//! projected variable: it names which allocation's row closed the stream, and `from_driver`
//! fails when the yielded kind disagrees with the row's. A stream error closes without a
//! row (`kind == error`, `closeAtt == -1`); the driver accepts only the injected
//! typed `ObservationRead` cause for this projection. `closes` is `1` once a closing outcome is
//! known; a second closing outcome is rejected at the poll (the stream must end after its
//! one verdict). Model-only: the world (`w`), the channel and lag flags (the subscription's
//! internals), `armed` (only observable as D10's later timeout), `src` and `closeTerm` (a
//! snapshot line equals the live line byte for byte, S-41), `readCur`, `lags`, `readFail`
//! (carried by `kind == error`), and `conf_step`.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use async_trait::async_trait;
use axum::body::Bytes;
use futures::{FutureExt as _, Stream, StreamExt as _};
use itf::Value;
use overdrive_core::TransitionReason;
use overdrive_core::ca::issue_failure::SvidIssueFailure;
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::id::WorkloadId;
use overdrive_core::id::{AllocationId, CorrelationKey, IssuanceOrdinal, NodeId, ServiceId};
use overdrive_core::observation::ProbeResultRow;
use overdrive_core::public_ingress::{
    GatewayApplicationStatusRowV1, PublicCertifiedKeyId, PublicCertifiedKeyStatusRowV1,
};
use overdrive_core::traits::observation_store::WorkloadSvidIssueStatusRowV1;
use overdrive_core::traits::observation_store::{
    AllocLifecycleOccurrenceRow, AllocStatusRow, LagAwareSubscription, NodeHealthRow,
    ObservationWrite, ReconcileConflictRow, ServiceBackendRow, ServiceHydrationResultRow,
    TransitionSource,
};
use overdrive_core::traits::observation_store::{
    AllocState, ObservationStore, ObservationStoreError,
};
use overdrive_core::transition_reason::{ProbeWitness, StoppedBy, TerminalCondition};
use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;
use tokio::runtime::Runtime;
use tokio::sync::Semaphore;

use super::{Composition, alloc_row, commit, compose, publish};
use crate::api::IdempotencyOutcome;
use crate::guest_network::quint_itf::{self, ItfError, Qualified};
use crate::streaming::{
    ServiceStreamError, ServiceSubmitEvent, build_service_accepted, build_service_stream,
};

const MAIN: &str = "ss_conf";
const MODULE: &str = "stream_snapshot";
const WORKLOAD: &str = "ss";

type Lines = Pin<Box<dyn Stream<Item = std::result::Result<Bytes, ServiceStreamError>> + Send>>;

/// What one poll of the stream produced.
enum Polled {
    Pending,
    Ended,
    Line(ServiceSubmitEvent),
    Error,
}

// The existing ObservationStore port supplies the test's poll boundary. Holding
// its first snapshot read leaves build_service_stream suspended only after the
// production bus receiver exists. All other calls delegate unchanged.
type StoreResult<T, E> = std::result::Result<T, E>;

struct SnapshotReadGate {
    inner: Arc<overdrive_sim::adapters::observation_store::SimObservationStore>,
    // 0 = no gate, 1 = before snapshot, 2 = after snapshot.
    pause_next: AtomicU8,
    entered: AtomicBool,
    release: Semaphore,
}

impl SnapshotReadGate {
    fn new(inner: Arc<overdrive_sim::adapters::observation_store::SimObservationStore>) -> Self {
        Self {
            inner,
            pause_next: AtomicU8::new(0),
            entered: AtomicBool::new(false),
            release: Semaphore::new(0),
        }
    }

    fn hold_next_read(&self) {
        self.entered.store(false, Ordering::SeqCst);
        self.pause_next.store(1, Ordering::SeqCst);
    }

    fn hold_after_snapshot(&self) {
        self.entered.store(false, Ordering::SeqCst);
        self.pause_next.store(2, Ordering::SeqCst);
    }

    fn entered(&self) -> bool {
        self.entered.load(Ordering::SeqCst)
    }

    fn release(&self) {
        self.release.add_permits(1);
    }
}

#[async_trait]
impl ObservationStore for SnapshotReadGate {
    async fn public_certified_key_status_row(
        &self,
        id: &PublicCertifiedKeyId,
    ) -> StoreResult<Option<PublicCertifiedKeyStatusRowV1>, ObservationStoreError> {
        self.inner.public_certified_key_status_row(id).await
    }

    async fn gateway_application_status_row(
        &self,
        node_id: &NodeId,
    ) -> StoreResult<Option<GatewayApplicationStatusRowV1>, ObservationStoreError> {
        self.inner.gateway_application_status_row(node_id).await
    }

    async fn record_workload_svid_issue_failure(
        &self,
        alloc_id: &AllocationId,
        node_id: &NodeId,
        failure: SvidIssueFailure,
    ) -> StoreResult<(), ObservationStoreError> {
        self.inner.record_workload_svid_issue_failure(alloc_id, node_id, failure).await
    }

    async fn workload_svid_issue_status_row(
        &self,
        alloc_id: &AllocationId,
    ) -> StoreResult<Option<WorkloadSvidIssueStatusRowV1>, ObservationStoreError> {
        self.inner.workload_svid_issue_status_row(alloc_id).await
    }

    async fn write(&self, row: ObservationWrite) -> StoreResult<(), ObservationStoreError> {
        self.inner.write(row).await
    }

    async fn write_alloc_lifecycle(
        &self,
        current: AllocStatusRow,
        source: TransitionSource,
    ) -> StoreResult<Option<AllocLifecycleOccurrenceRow>, ObservationStoreError> {
        self.inner.write_alloc_lifecycle(current, source).await
    }

    async fn alloc_lifecycle_occurrences(
        &self,
        alloc_id: &AllocationId,
    ) -> StoreResult<Vec<AllocLifecycleOccurrenceRow>, ObservationStoreError> {
        self.inner.alloc_lifecycle_occurrences(alloc_id).await
    }

    async fn subscribe_all_events(
        &self,
    ) -> StoreResult<LagAwareSubscription, ObservationStoreError> {
        self.inner.subscribe_all_events().await
    }

    async fn alloc_status_rows(&self) -> StoreResult<Vec<AllocStatusRow>, ObservationStoreError> {
        let pause = self.pause_next.swap(0, Ordering::SeqCst);
        if pause == 1 {
            self.entered.store(true, Ordering::SeqCst);
            self.release.acquire().await.expect("read gate stays open").forget();
        }
        let snapshot = self.inner.alloc_status_rows().await;
        if pause == 2 {
            self.entered.store(true, Ordering::SeqCst);
            self.release.acquire().await.expect("read gate stays open").forget();
        }
        snapshot
    }

    async fn alloc_status_row(
        &self,
        alloc_id: &AllocationId,
    ) -> StoreResult<Option<AllocStatusRow>, ObservationStoreError> {
        self.inner.alloc_status_row(alloc_id).await
    }

    async fn node_health_rows(&self) -> StoreResult<Vec<NodeHealthRow>, ObservationStoreError> {
        self.inner.node_health_rows().await
    }

    async fn issued_certificate_rows(
        &self,
    ) -> StoreResult<Vec<IssuedCertificateRow>, ObservationStoreError> {
        self.inner.issued_certificate_rows().await
    }

    async fn next_issuance_ordinal(&self) -> StoreResult<IssuanceOrdinal, ObservationStoreError> {
        self.inner.next_issuance_ordinal().await
    }

    async fn service_hydration_results_rows(
        &self,
        service_id: &ServiceId,
    ) -> StoreResult<Vec<ServiceHydrationResultRow>, ObservationStoreError> {
        self.inner.service_hydration_results_rows(service_id).await
    }

    async fn service_backends_rows(
        &self,
        service_id: &ServiceId,
    ) -> StoreResult<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.service_backends_rows(service_id).await
    }

    async fn all_service_backends_rows(
        &self,
    ) -> StoreResult<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.all_service_backends_rows().await
    }

    async fn reconcile_conflict_rows(
        &self,
        service_id: &ServiceId,
    ) -> StoreResult<Vec<ReconcileConflictRow>, ObservationStoreError> {
        self.inner.reconcile_conflict_rows(service_id).await
    }

    async fn write_probe_result(
        &self,
        row: ProbeResultRow,
    ) -> StoreResult<(), ObservationStoreError> {
        self.inner.write_probe_result(row).await
    }

    async fn list_probe_results_for_alloc(
        &self,
        alloc_id: &AllocationId,
    ) -> StoreResult<Vec<ProbeResultRow>, ObservationStoreError> {
        self.inner.list_probe_results_for_alloc(alloc_id).await
    }

    async fn workflow_terminal_rows(
        &self,
    ) -> StoreResult<Vec<(CorrelationKey, WorkflowStatus)>, ObservationStoreError> {
        self.inner.workflow_terminal_rows().await
    }

    async fn workflow_signal(
        &self,
        key: &SignalKey,
    ) -> StoreResult<Option<SignalValue>, ObservationStoreError> {
        self.inner.workflow_signal(key).await
    }
}

struct StreamSnapshotDriver {
    rt: Runtime,
    /// The composed node, held directly: the projection reads the closing row through
    /// the node's observation store (DG-4).
    node: Composition,
    stream: Option<Lines>,
    /// Every closing outcome the stream yielded, in order.
    yielded: Vec<Polled>,
    /// Events published and not yet delivered to the stream, oldest first (the order
    /// the bus will deliver them in).
    undelivered: std::collections::VecDeque<u32>,
    /// The stream's first read is held at the existing observation port after
    /// production has subscribed, so the model's Subscribe is a real boundary.
    read_gate: Arc<SnapshotReadGate>,
    /// Whether the stream has subscribed (the held poll after `Accepted`).
    subscribed: bool,
    /// The model's `lagged` flag: set by `Lag`, cleared by the `Relist` poll.
    lagged: bool,
    cur: u32,
    counter: u64,
}

impl StreamSnapshotDriver {
    fn new() -> Self {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        let mut node = rt.block_on(compose(2911));
        let read_gate = Arc::new(SnapshotReadGate::new(Arc::clone(&node.obs)));
        node.state.obs = Arc::clone(&read_gate) as Arc<dyn ObservationStore>;
        Self {
            rt,
            node,
            stream: None,
            yielded: Vec::new(),
            undelivered: std::collections::VecDeque::new(),
            read_gate,
            subscribed: false,
            lagged: false,
            cur: 0,
            counter: 0,
        }
    }

    fn node(&self) -> &Composition {
        &self.node
    }

    fn init(&mut self) {
        eprintln!("quint-conformance stream_snapshot seed=2911");
        self.node = self.rt.block_on(compose(2911));
        self.read_gate = Arc::new(SnapshotReadGate::new(Arc::clone(&self.node.obs)));
        self.node.state.obs = Arc::clone(&self.read_gate) as Arc<dyn ObservationStore>;
        let accepted = build_service_accepted(
            "0".repeat(64),
            format!("workloads/{WORKLOAD}"),
            IdempotencyOutcome::Unchanged,
        );
        let stream: Lines = Box::pin(build_service_stream(
            self.node.state.clone(),
            WorkloadId::new(WORKLOAD).expect("workload id"),
            accepted,
            std::time::Duration::from_secs(3600),
        ));
        self.stream = Some(stream);
        self.yielded.clear();
        self.undelivered.clear();
        self.subscribed = false;
        self.lagged = false;
        self.cur = 0;
        self.counter = 0;
    }

    /// Drive a logical stream step through ready work. Tokio may return a
    /// cooperative Pending while draining a full broadcast buffer, so one
    /// external model step can require more than one physical poll.
    fn poll(&mut self) -> Result {
        let stream = self.stream.as_mut().expect("init built the stream");
        // The fixture can leave at most one full broadcast buffer plus eight
        // noise events ready. A stable Pending after this bound is a true wait.
        for _ in 0..=(crate::DEFAULT_LIFECYCLE_BROADCAST_CAPACITY + 8) {
            let produced = self.rt.block_on(async {
                loop {
                    let polled = match stream.next().now_or_never() {
                        None => Polled::Pending,
                        Some(None) => Polled::Ended,
                        Some(Some(Ok(bytes))) => Polled::Line(serde_json::from_slice(&bytes)?),
                        Some(Some(Err(ServiceStreamError::ObservationRead {
                            source: ObservationStoreError::Unreachable { peer },
                        }))) if peer == "peer-b" => Polled::Error,
                        Some(Some(Err(error))) => {
                            anyhow::bail!(
                                "unexpected stream error instead of typed read cause: {error:?}"
                            )
                        }
                    };
                    match polled {
                        // The `Accepted` line is not a closing outcome: keep polling.
                        Polled::Line(ServiceSubmitEvent::Accepted { .. }) => {}
                        outcome => return Ok::<_, anyhow::Error>(outcome),
                    }
                }
            })?;
            match produced {
                Polled::Pending => {}
                Polled::Ended => return Ok(()),
                outcome => {
                    anyhow::ensure!(
                        self.yielded.is_empty(),
                        "the stream yielded a second closing outcome after its verdict",
                    );
                    self.yielded.push(outcome);
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Poll past `Accepted` into the real `bus.subscribe`, then hold the
    /// observation read. The next environment action is inside the actual
    /// subscribe-before-read window.
    fn subscribe(&mut self) -> Result {
        self.subscribe_at_snapshot(false)
    }

    fn subscribe_at_snapshot(&mut self, after_snapshot: bool) -> Result {
        let stream = self.stream.as_mut().expect("init built the stream");
        let first = self
            .rt
            .block_on(async { stream.next().now_or_never() })
            .flatten()
            .ok_or_else(|| anyhow::anyhow!("the stream yields Accepted on its first poll"))??;
        let line: ServiceSubmitEvent = serde_json::from_slice(&first)?;
        anyhow::ensure!(
            matches!(line, ServiceSubmitEvent::Accepted { .. }),
            "the first line is Accepted; got {line:?}"
        );
        if after_snapshot {
            self.read_gate.hold_after_snapshot();
        } else {
            self.read_gate.hold_next_read();
        }
        anyhow::ensure!(
            self.rt.block_on(async { stream.next().now_or_never() }).is_none(),
            "the stream waits at its first recorded-verdict read"
        );
        anyhow::ensure!(
            self.read_gate.entered(),
            "the stream reached the observation read after subscribing"
        );
        self.subscribed = true;
        Ok(())
    }

    fn read(&mut self, ok: bool) -> Result {
        if !ok {
            self.fail_next_read();
        }
        self.read_gate.release();
        self.poll()?;
        self.undelivered.clear();
        Ok(())
    }

    fn fail_next_read(&self) {
        self.node().obs.inject_alloc_status_rows_failure(ObservationStoreError::Unreachable {
            peer: "peer-b".to_owned(),
        });
    }

    fn next_counter(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    fn commit(&mut self, a: i64, terminal: &str) -> Result {
        let (state, reason, terminal) = match terminal {
            "stable" => (
                AllocState::Running,
                TransitionReason::Started,
                TerminalCondition::Stable {
                    settled_in_ms: 700,
                    witness: ProbeWitness {
                        probe_idx: 0,
                        role: "startup".to_owned(),
                        mechanic_summary: "tcp 127.0.0.1:8080".to_owned(),
                    },
                },
            ),
            "startup_stop" => (
                AllocState::Terminated,
                TransitionReason::Stopped { by: StoppedBy::StartupProbe },
                TerminalCondition::Stopped { by: StoppedBy::StartupProbe },
            ),
            "liveness_stop" => (
                AllocState::Terminated,
                TransitionReason::Stopped { by: StoppedBy::LivenessProbe },
                TerminalCondition::Stopped { by: StoppedBy::LivenessProbe },
            ),
            "giveup" => (
                AllocState::Failed,
                TransitionReason::Started,
                TerminalCondition::BackoffExhausted { attempts: 5 },
            ),
            "operator_stop" => (
                AllocState::Terminated,
                TransitionReason::Stopped { by: StoppedBy::Operator },
                TerminalCondition::Stopped { by: StoppedBy::Operator },
            ),
            other => anyhow::bail!("unmodelled terminal `{other}`"),
        };
        let counter = self.next_counter();
        let row = alloc_row(WORKLOAD, u32::try_from(a)?, counter, state, reason, Some(terminal));
        self.rt.block_on(commit(self.node(), &row));
        Ok(())
    }

    fn publish(&mut self, a: i64) -> Result {
        let attempt = u32::try_from(a)?;
        let alloc = overdrive_core::id::AllocationId::new(&format!("alloc-{WORKLOAD}-{attempt}"))?;
        let row = self
            .rt
            .block_on(self.node().obs.alloc_status_row(&alloc))?
            .ok_or_else(|| anyhow::anyhow!("an event is published only for a committed row"))?;
        publish(self.node(), &row);
        if self.subscribed {
            // Retained behind the lag when lagged, as the model's `Publish` appends.
            self.undelivered.push_back(attempt);
        }
        Ok(())
    }

    fn deliver(&mut self, a: i64) -> Result {
        let attempt = u32::try_from(a)?;
        // The model's channel is an ordered list; `Deliver` is disabled while lagged and
        // starts at the oldest event (DG-5). The bus delivers in publication order.
        anyhow::ensure!(!self.lagged, "ModelAbstraction: Deliver is disabled while lagged");
        anyhow::ensure!(
            self.undelivered.front() == Some(&attempt),
            "ModelAbstraction: the model delivers attempt {attempt} but the bus's oldest \
             undelivered event is {:?}",
            self.undelivered.front(),
        );
        self.poll()?;
        self.undelivered.clear();
        Ok(())
    }

    fn lag(&mut self) {
        // Other workloads' events overflow the subscription's buffer: everything
        // undelivered is dropped and the next receive is `Lagged`.
        let noise = alloc_row("noise", 0, 1, AllocState::Running, TransitionReason::Started, None);
        for _ in 0..(crate::DEFAULT_LIFECYCLE_BROADCAST_CAPACITY + 8) {
            publish(self.node(), &noise);
        }
        self.undelivered.clear();
        self.lagged = true;
    }

    fn relist(&mut self, ok: bool) -> Result {
        if !ok {
            self.fail_next_read();
        }
        self.lagged = false;
        self.poll()?;
        self.undelivered.clear();
        Ok(())
    }

    fn restart(&mut self) {
        self.cur += 1;
        let counter = self.next_counter();
        let successor = alloc_row(
            WORKLOAD,
            self.cur,
            counter,
            AllocState::Running,
            TransitionReason::Started,
            None,
        );
        self.rt.block_on(commit(self.node(), &successor));
    }

    fn idle(&self) -> Result {
        anyhow::ensure!(
            self.yielded.len() <= 1,
            "the stream yielded more than one closing outcome without being driven",
        );
        Ok(())
    }

    /// The closing `AllocStatusRow` the stream's verdict is about: the owner-side
    /// observation the projection compares (DG-4).
    fn closing_row(
        &self,
        alloc_id: &str,
    ) -> Result<overdrive_core::traits::observation_store::AllocStatusRow> {
        let alloc = overdrive_core::id::AllocationId::new(alloc_id)?;
        self.rt
            .block_on(self.node.state.obs.alloc_status_row(&alloc))?
            .ok_or_else(|| anyhow::anyhow!("the stream closed on `{alloc_id}`, which has no row"))
    }

    fn step_on(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init(),
            Subscribe => self.subscribe()?,
            Read(ok: bool) => self.read(ok)?,
            Commit(a: i64, t: String) => self.commit(a, &t)?,
            Publish(a: i64) => self.publish(a)?,
            Deliver(a: i64) => self.deliver(a)?,
            Lag => self.lag(),
            Relist(ok: bool) => self.relist(ok)?,
            Restart => self.restart(),
            Idle => self.idle()?,
        })
    }
}

impl Driver for StreamSnapshotDriver {
    type State = StreamSnapshotState;

    fn config() -> Config {
        Config { state: &[], nondet: &["ss_conf::stream_snapshot::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        self.step_on(step)
    }
}

// ----- the compared state ------------------------------------------------------

#[derive(Debug, PartialEq)]
struct StreamSnapshotState {
    st: String,
    kind: String,
    close_att: i64,
    closes: i64,
}

impl StreamSnapshotState {
    fn from_value(value: &Value) -> std::result::Result<Self, ItfError> {
        let vars = Qualified::of(value, MAIN, MODULE)?;
        let s = vars.var("s")?;
        Ok(Self {
            st: quint_itf::string(quint_itf::field(s, "st")?)?,
            kind: quint_itf::string(quint_itf::field(s, "kind")?)?,
            close_att: quint_itf::int(quint_itf::field(s, "closeAtt")?)?,
            closes: quint_itf::int(quint_itf::field(s, "closes")?)?,
        })
    }
}

impl<'de> Deserialize<'de> for StreamSnapshotState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<StreamSnapshotDriver> for StreamSnapshotState {
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &StreamSnapshotDriver) -> Result<Self> {
        let Some(outcome) = driver.yielded.first() else {
            return Ok(Self {
                st: "open".to_owned(),
                kind: String::new(),
                close_att: -1,
                closes: 0,
            });
        };
        // The yielded line is the oracle: it names the allocation whose row closed the
        // stream and the kind the stream claims.
        let (claimed, alloc_id) = match outcome {
            Polled::Line(ServiceSubmitEvent::Stable { alloc_id, .. }) => ("stable", alloc_id),
            Polled::Line(ServiceSubmitEvent::Failed { alloc_id: Some(alloc_id), .. }) => {
                ("giveup", alloc_id)
            }
            Polled::Line(ServiceSubmitEvent::Stopped { alloc_id, by: StoppedBy::Operator }) => {
                ("operator_stop", alloc_id)
            }
            // A failed read closes the stream without a row.
            Polled::Error => {
                return Ok(Self {
                    st: "closed".to_owned(),
                    kind: "error".to_owned(),
                    close_att: -1,
                    closes: 1,
                });
            }
            Polled::Line(other) => {
                anyhow::bail!("a closing line the model does not have: {other:?}")
            }
            Polled::Pending | Polled::Ended => anyhow::bail!("not a closing outcome"),
        };
        // The compared state: the closing row's `state` and `terminal`.
        let row = driver.closing_row(alloc_id)?;
        let kind = match (row.state, row.terminal.as_ref()) {
            (AllocState::Running, Some(TerminalCondition::Stable { .. })) => "stable",
            (AllocState::Failed, Some(TerminalCondition::BackoffExhausted { .. })) => "giveup",
            (
                AllocState::Terminated,
                Some(TerminalCondition::Stopped { by: StoppedBy::Operator }),
            ) => "operator_stop",
            (state, terminal) => anyhow::bail!(
                "the closing row `{alloc_id}` is {state:?} with terminal {terminal:?}, which \
                 closes no stream"
            ),
        };
        anyhow::ensure!(
            kind == claimed,
            "the stream yielded `{claimed}` but the closing row says `{kind}`"
        );
        Ok(Self {
            st: "closed".to_owned(),
            kind: kind.to_owned(),
            close_att: attempt_of(row.alloc_id.as_str())?,
            closes: 1,
        })
    }
}

/// The attempt a closing line names: the numeric suffix of its allocation id.
fn attempt_of(alloc_id: &str) -> Result<i64> {
    let suffix = alloc_id
        .rsplit('-')
        .next()
        .ok_or_else(|| anyhow::anyhow!("allocation id `{alloc_id}` has no attempt suffix"))?;
    Ok(suffix.parse()?)
}

// ----- the runs ----------------------------------------------------------------
//
// One `#[quint_test]` per named run of `ss_conf` (README § "Named runs"), plus
// `#[quint_run]` randomized traces (seed printed on failure).

/// D11 S-40 counterfactual: the first snapshot has already returned no verdict,
/// then a terminal is committed and published before the stream resumes. Only
/// subscribe-before-read can deliver that event after the empty snapshot.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_terminal_published_after_the_empty_snapshot_reaches_the_subscribed_stream() {
    let mut driver = StreamSnapshotDriver::new();
    driver.init();
    driver.subscribe_at_snapshot(true).expect("the stream subscribes and scans empty rows");

    driver.commit(0, "stable").expect("the terminal commits after the empty snapshot");
    driver.publish(0).expect("the terminal publishes before the snapshot returns");
    driver.read_gate.release();
    driver.poll().expect("the live event closes the stream");

    assert!(matches!(
        driver.yielded.as_slice(),
        [Polled::Line(ServiceSubmitEvent::Stable { alloc_id, .. })] if alloc_id == "alloc-ss-0"
    ));
    assert!(matches!(
        driver.rt.block_on(async { driver.stream.as_mut().unwrap().next().now_or_never() }),
        Some(None)
    ));
}

/// Named run `redeploy_unchanged_of_stable_ends_stable` (D11).
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/stream_snapshot.qnt",
    main = "ss_conf",
    test = "redeploy_unchanged_of_stable_ends_stable",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_stream_snapshot_redeploy_unchanged_of_stable_ends_stable() -> impl Driver {
    StreamSnapshotDriver::new()
}

/// Named run `terminal_between_subscribe_and_read_is_not_lost` (D11).
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/stream_snapshot.qnt",
    main = "ss_conf",
    test = "terminal_between_subscribe_and_read_is_not_lost",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_stream_snapshot_terminal_between_subscribe_and_read_is_not_lost() -> impl Driver {
    StreamSnapshotDriver::new()
}

/// Named run `restartable_stop_recorded_is_not_a_verdict` (D11, D7 item 5).
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/stream_snapshot.qnt",
    main = "ss_conf",
    test = "restartable_stop_recorded_is_not_a_verdict",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_stream_snapshot_restartable_stop_recorded_is_not_a_verdict() -> impl Driver {
    StreamSnapshotDriver::new()
}

/// Named run `superseded_attempt_terminal_is_not_reported` (D11).
/// Regression witness at HEAD: every compared state is `open`, so it proves only that the
/// driver replays the run. It does NOT prove a superseded attempt's terminal is withheld;
/// `lagged_relist_recovers_terminal` and the stream acceptance tests do.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/stream_snapshot.qnt",
    main = "ss_conf",
    test = "superseded_attempt_terminal_is_not_reported",
    max_samples = 1
)]
fn quint_stream_snapshot_superseded_attempt_terminal_is_not_reported() -> impl Driver {
    StreamSnapshotDriver::new()
}

/// Named run `lagged_relist_recovers_terminal` (D11).
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/stream_snapshot.qnt",
    main = "ss_conf",
    test = "lagged_relist_recovers_terminal",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_stream_snapshot_lagged_relist_recovers_terminal() -> impl Driver {
    StreamSnapshotDriver::new()
}

/// Named run `publish_after_relist_reaches_channel` (D11): an event published after a
/// relist that found no verdict reaches the channel and is delivered, oldest first.
/// Every compared state in the run is `open`, so it already holds at HEAD (not ignored,
/// like `superseded_attempt_terminal_is_not_reported`). Regression witness only: it does
/// NOT prove the relist delivers a later event in order; `lagged_relist_recovers_terminal`
/// exercises the relist.
/// CONTRACT_SHAPE: bounded-change.
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/stream_snapshot.qnt",
    main = "ss_conf",
    test = "publish_after_relist_reaches_channel",
    max_samples = 1
)]
fn quint_stream_snapshot_publish_after_relist_reaches_channel() -> impl Driver {
    StreamSnapshotDriver::new()
}

/// Named run `failed_read_ends_with_error` (D11, G-8 failure projection).
#[quint_test(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/stream_snapshot.qnt",
    main = "ss_conf",
    test = "failed_read_ends_with_error",
    max_samples = 1
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_stream_snapshot_failed_read_ends_with_error() -> impl Driver {
    StreamSnapshotDriver::new()
}

/// Randomized traces of `ss_conf` (seed printed on failure).
#[quint_run(
    spec = "../../specs/quint/service-allocation-readiness-and-ending/stream_snapshot.qnt",
    main = "ss_conf"
)]
/// CONTRACT_SHAPE: bounded-change.
fn quint_stream_snapshot_randomized() -> impl Driver {
    StreamSnapshotDriver::new()
}
