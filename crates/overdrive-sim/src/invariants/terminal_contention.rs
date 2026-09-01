//! Seeded terminal-contention invariant for the real stop/exit composition.
//!
//! The schedule is the production-reachable race described by
//! `guest-stack-transparent-mtls-intercept` BTR-1:
//!
//! 1. a VM allocation is Running and supervised by [`SimDriver`];
//! 2. [`Action::StopAllocation`] reaches its first terminal compound write
//!    after `Driver::stop` has made the real exit event intentional;
//! 3. the harness parks that proposal at the [`ObservationStore`] port and
//!    advances [`SimClock`], letting the real `exit_observer` accept the
//!    equal-timestamp driver occurrence first;
//! 4. the parked stop proposal is released and loses the real LWW comparison;
//! 5. production dispatch fresh-reads, rebases once, and accepts the
//!    authoritative operator terminal claim.
//!
//! The decorator controls only the ordering at an existing driven port. It
//! neither authors the competing row nor changes the store verdict: both
//! accepted writes are produced by production code, and the rejected write is
//! rejected by [`SimObservationStore`]'s real LWW comparator. The evidence
//! checker pins current-row convergence, occurrence/event semantics, the
//! two-proposal bound, route removal, and supervision release. Its negative
//! control removes the observed LWW loss from a healthy trace; if the checker
//! still passes, the invariant has lost its teeth.

#![allow(
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_control_plane::action_shim::{AllocDriverIndex, LifecycleEvent, dispatch};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::veth_provisioner::NetSlotAllocator;
use overdrive_control_plane::worker::exit_observer;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::id::{
    AllocationId, CorrelationKey, IssuanceOrdinal, NodeId, ServiceId, SpiffeId, WorkloadId,
};
use overdrive_core::observation::ProbeResultRow;
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverRegistry, DriverType, ExitKind, Resources,
    VmPayload,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocLifecycleOccurrenceRow, AllocState, AllocStatusRow, LagAwareSubscription,
    LogicalTimestamp, NodeHealthRow, ObservationStore, ObservationStoreError, ObservationWrite,
    ReconcileConflictRow, ServiceBackendRow, ServiceHydrationResultRow, TransitionSource,
};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};
use overdrive_store_local::LocalIntentStore;
use parking_lot::Mutex;
use tempfile::TempDir;
use tokio::sync::{Semaphore, oneshot};

use crate::adapters::ca::SimCa;
use crate::adapters::clock::SimClock;
use crate::adapters::dataplane::SimDataplane;
use crate::adapters::driver::SimDriver;
use crate::adapters::entropy::SimEntropy;
use crate::adapters::observation_store::SimObservationStore;
use crate::adapters::vm_host_state::SimVmHostState;
use crate::harness::{InvariantResult, InvariantStatus};

const NAME: &str = "terminal-contention-converges";
const HOST: &str = "host-0";
const COOPERATIVE_BUDGET: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteVerdict {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WriteAttempt {
    source: TransitionSource,
    at: LogicalTimestamp,
    terminal: Option<TerminalCondition>,
    verdict: WriteVerdict,
}

#[derive(Debug, Clone)]
struct Evidence {
    seed: u64,
    base_counter: u64,
    current: AllocStatusRow,
    occurrences: Vec<AllocLifecycleOccurrenceRow>,
    events: Vec<LifecycleEvent>,
    attempts: Vec<WriteAttempt>,
    route_removed: bool,
    supervision_released: bool,
    observer_drained: bool,
}

/// Observation-store scheduling adapter for the one load-bearing partition.
///
/// It parks only the first reconciler terminal proposal. Driver writes and
/// every store decision still go directly through `SimObservationStore`.
struct ContentionStore {
    inner: Arc<SimObservationStore>,
    target: AllocationId,
    stop_proposals: AtomicUsize,
    attempts: Mutex<Vec<WriteAttempt>>,
    first_stop_entered: Mutex<Option<oneshot::Sender<()>>>,
    driver_accepted: Mutex<Option<oneshot::Sender<()>>>,
    resume_first_stop: Semaphore,
}

impl ContentionStore {
    fn new(
        inner: Arc<SimObservationStore>,
        target: AllocationId,
    ) -> (Self, oneshot::Receiver<()>, oneshot::Receiver<()>) {
        let (first_stop_entered, first_stop_entered_rx) = oneshot::channel();
        let (driver_accepted, driver_accepted_rx) = oneshot::channel();
        (
            Self {
                inner,
                target,
                stop_proposals: AtomicUsize::new(0),
                attempts: Mutex::new(Vec::new()),
                first_stop_entered: Mutex::new(Some(first_stop_entered)),
                driver_accepted: Mutex::new(Some(driver_accepted)),
                resume_first_stop: Semaphore::new(0),
            },
            first_stop_entered_rx,
            driver_accepted_rx,
        )
    }

    fn release_first_stop(&self) {
        self.resume_first_stop.add_permits(1);
    }

    fn attempts(&self) -> Vec<WriteAttempt> {
        self.attempts.lock().clone()
    }
}

#[async_trait]
impl ObservationStore for ContentionStore {
    async fn write(&self, row: ObservationWrite) -> Result<(), ObservationStoreError> {
        self.inner.write(row).await
    }

    async fn write_alloc_lifecycle(
        &self,
        current: AllocStatusRow,
        source: TransitionSource,
    ) -> Result<Option<AllocLifecycleOccurrenceRow>, ObservationStoreError> {
        let is_stop_proposal = current.alloc_id == self.target
            && source == TransitionSource::Reconciler
            && current.terminal.is_some();
        if is_stop_proposal {
            let proposal = self.stop_proposals.fetch_add(1, Ordering::SeqCst);
            if proposal == 0 {
                let entered = self.first_stop_entered.lock().take();
                if let Some(entered) = entered {
                    let _ = entered.send(());
                }
                self.resume_first_stop
                    .acquire()
                    .await
                    .map_err(|_| {
                        ObservationStoreError::Io(std::io::Error::other(
                            "terminal-contention semaphore closed",
                        ))
                    })?
                    .forget();
            }
        }

        let occurrence = self.inner.write_alloc_lifecycle(current.clone(), source).await?;
        let verdict =
            if occurrence.is_some() { WriteVerdict::Accepted } else { WriteVerdict::Rejected };
        self.attempts.lock().push(WriteAttempt {
            source,
            at: current.updated_at,
            terminal: current.terminal,
            verdict,
        });

        if source == TransitionSource::Driver(DriverType::Vm)
            && verdict == WriteVerdict::Accepted
            && let Some(accepted) = self.driver_accepted.lock().take()
        {
            let _ = accepted.send(());
        }

        Ok(occurrence)
    }

    async fn alloc_lifecycle_occurrences(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Vec<AllocLifecycleOccurrenceRow>, ObservationStoreError> {
        self.inner.alloc_lifecycle_occurrences(alloc_id).await
    }

    async fn subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError> {
        self.inner.subscribe_all_events().await
    }

    async fn alloc_status_rows(&self) -> Result<Vec<AllocStatusRow>, ObservationStoreError> {
        self.inner.alloc_status_rows().await
    }

    async fn alloc_status_row(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Option<AllocStatusRow>, ObservationStoreError> {
        self.inner.alloc_status_row(alloc_id).await
    }

    async fn node_health_rows(&self) -> Result<Vec<NodeHealthRow>, ObservationStoreError> {
        self.inner.node_health_rows().await
    }

    async fn issued_certificate_rows(
        &self,
    ) -> Result<Vec<IssuedCertificateRow>, ObservationStoreError> {
        self.inner.issued_certificate_rows().await
    }

    async fn next_issuance_ordinal(&self) -> Result<IssuanceOrdinal, ObservationStoreError> {
        self.inner.next_issuance_ordinal().await
    }

    async fn service_hydration_results_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceHydrationResultRow>, ObservationStoreError> {
        self.inner.service_hydration_results_rows(service_id).await
    }

    async fn service_backends_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.service_backends_rows(service_id).await
    }

    async fn all_service_backends_rows(
        &self,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.all_service_backends_rows().await
    }

    async fn reconcile_conflict_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ReconcileConflictRow>, ObservationStoreError> {
        self.inner.reconcile_conflict_rows(service_id).await
    }

    async fn write_probe_result(&self, row: ProbeResultRow) -> Result<(), ObservationStoreError> {
        self.inner.write_probe_result(row).await
    }

    async fn list_probe_results_for_alloc(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Vec<ProbeResultRow>, ObservationStoreError> {
        self.inner.list_probe_results_for_alloc(alloc_id).await
    }

    async fn workflow_terminal_rows(
        &self,
    ) -> Result<Vec<(CorrelationKey, WorkflowStatus)>, ObservationStoreError> {
        self.inner.workflow_terminal_rows().await
    }

    async fn workflow_signal(
        &self,
        key: &SignalKey,
    ) -> Result<Option<SignalValue>, ObservationStoreError> {
        self.inner.workflow_signal(key).await
    }
}

fn fixture_ids(seed: u64) -> Result<(AllocationId, WorkloadId, NodeId), String> {
    let suffix = seed % 1_000_000;
    Ok((
        AllocationId::new(&format!("alloc-terminal-contention-{suffix}"))
            .map_err(|error| format!("allocation id: {error:?}"))?,
        WorkloadId::new(&format!("terminal-contention-{suffix}"))
            .map_err(|error| format!("workload id: {error:?}"))?,
        NodeId::new(&format!("node-terminal-contention-{suffix}"))
            .map_err(|error| format!("node id: {error:?}"))?,
    ))
}

fn vm_spec(alloc: &AllocationId, workload: &WorkloadId) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::for_allocation(workload, alloc),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/sim/kernel"),
            rootfs: PathBuf::from("/sim/rootfs"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
        guest_tap: None,
        guest_mac: None,
        guest_gateway: None,
        guest_prefix_len: None,
        guest_dns: None,
    }
}

fn running_row(
    alloc: AllocationId,
    workload: WorkloadId,
    node: NodeId,
    counter: u64,
    started_at: UnixInstant,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: alloc,
        workload_id: workload,
        node_id: node.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter, writer: node },
        reason: Some(TransitionReason::Started),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: Some(started_at),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

async fn await_stop_partition(
    signal: &mut oneshot::Receiver<()>,
    dispatch: &mut (
             impl std::future::Future<
        Output = Result<(), overdrive_control_plane::action_shim::ShimError>,
    > + Unpin
         ),
) -> Result<(), String> {
    for _ in 0..COOPERATIVE_BUDGET {
        tokio::select! {
            biased;
            result = &mut *signal => return result.map_err(|error| format!("stop partition signal closed: {error}")),
            result = &mut *dispatch => return Err(format!("StopAllocation completed before its first terminal proposal was partitioned: {result:?}")),
            () = tokio::task::yield_now() => {}
        }
    }
    Err("StopAllocation did not reach its first terminal proposal within the cooperative budget"
        .to_owned())
}

async fn await_signal(signal: &mut oneshot::Receiver<()>, label: &str) -> Result<(), String> {
    for _ in 0..COOPERATIVE_BUDGET {
        match signal.try_recv() {
            Ok(()) => return Ok(()),
            Err(oneshot::error::TryRecvError::Closed) => {
                return Err(format!("{label} signal closed before firing"));
            }
            Err(oneshot::error::TryRecvError::Empty) => tokio::task::yield_now().await,
        }
    }
    Err(format!("{label} did not occur within the cooperative budget"))
}

async fn await_observer_drain(observer: &mut tokio::task::JoinHandle<()>) -> Result<bool, String> {
    for _ in 0..COOPERATIVE_BUDGET {
        if observer.is_finished() {
            return observer
                .await
                .map(|()| true)
                .map_err(|error| format!("exit observer join failed: {error}"));
        }
        tokio::task::yield_now().await;
    }
    observer.abort();
    let _ = observer.await;
    Err("exit observer did not drain after the driver channel closed".to_owned())
}

async fn drive(seed: u64) -> Result<Evidence, String> {
    let (alloc, workload, node) = fixture_ids(seed)?;
    let base_counter = 16 + seed % 1_024;
    let emit_delay = Duration::from_nanos(1 + seed % 31);
    let clock = Arc::new(SimClock::new());
    let inner = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
    inner
        .write_alloc_lifecycle(
            running_row(
                alloc.clone(),
                workload.clone(),
                node.clone(),
                base_counter,
                UnixInstant::from_unix_duration(clock.unix_now()),
            ),
            TransitionSource::Reconciler,
        )
        .await
        .map_err(|error| format!("seed Running occurrence: {error}"))?;

    let driver = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
    let spec = vm_spec(&alloc, &workload);
    driver
        .start(&spec)
        .await
        .map_err(|error| format!("seed supervised SimDriver allocation: {error}"))?;
    driver.inject_exit_after(
        &alloc,
        emit_delay,
        ExitKind::Crashed { exit_code: Some(137), signal: None },
    );

    let (contention, mut first_stop_entered, mut driver_accepted) =
        ContentionStore::new(Arc::clone(&inner), alloc.clone());
    let contention = Arc::new(contention);
    let observation_port: Arc<dyn ObservationStore> = contention.clone();
    let driver_port: Arc<dyn Driver> = driver.clone();
    let mut drivers = DriverRegistry::new();
    drivers.insert(driver_port.clone());
    let alloc_drivers = AllocDriverIndex::default();
    alloc_drivers.lock().insert(alloc.clone(), DriverType::Vm);

    let (events_tx, mut events_rx) = tokio::sync::broadcast::channel(8);
    let mut observer = exit_observer::spawn(
        observation_port,
        driver_port.clone(),
        Arc::new(events_tx.clone()),
        clock.clone(),
    );

    let tempdir = TempDir::new().map_err(|error| format!("tempdir: {error}"))?;
    let intent: Arc<dyn IntentStore> = Arc::new(
        LocalIntentStore::open(tempdir.path().join("intent.redb"))
            .map_err(|error| format!("open local intent store: {error}"))?,
    );
    let allocator = overdrive_control_plane::test_default_allocator(intent);
    let dataplane = SimDataplane::new();
    let ca = SimCa::new(Arc::new(SimEntropy::new(seed)));
    let identity = IdentityMgr::new(None);
    let broker = Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    let net_slots = NetSlotAllocator::new();
    let host = SimVmHostState::new();
    let now = clock.now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(clock.unix_now()),
        tick: base_counter - 1,
        deadline: now + Duration::from_secs(1),
    };
    let writer_node = NodeId::new("writer-terminal-contention")
        .map_err(|error| format!("writer node: {error:?}"))?;
    let requested_terminal = TerminalCondition::Stopped { by: StoppedBy::Operator };

    {
        let dispatch = dispatch(
            vec![Action::StopAllocation {
                alloc_id: alloc.clone(),
                terminal: Some(requested_terminal.clone()),
            }],
            &drivers,
            &alloc_drivers,
            contention.as_ref(),
            &dataplane,
            &ca,
            clock.as_ref(),
            &identity,
            &events_tx,
            &tick,
            &writer_node,
            allocator,
            &broker,
            None,
            None,
            &net_slots,
            &host,
        );
        tokio::pin!(dispatch);

        await_stop_partition(&mut first_stop_entered, &mut dispatch).await?;
        clock.tick(emit_delay);
        await_signal(&mut driver_accepted, "real exit-observer accepted write").await?;
        contention.release_first_stop();
        dispatch.await.map_err(|error| format!("StopAllocation dispatch failed: {error}"))?;
    }

    let current = inner
        .alloc_status_row(&alloc)
        .await
        .map_err(|error| format!("read current row: {error}"))?
        .ok_or_else(|| "current allocation row disappeared".to_owned())?;
    let occurrences = inner
        .alloc_lifecycle_occurrences(&alloc)
        .await
        .map_err(|error| format!("read occurrences: {error}"))?;
    let attempts = contention.attempts();
    let route_removed = !alloc_drivers.lock().contains_key(&alloc);
    let supervision_released = driver.live_allocations() == Some(Vec::new());

    drop(drivers);
    drop(driver_port);
    drop(driver);
    let observer_drained = await_observer_drain(&mut observer).await?;

    let mut events = Vec::new();
    while let Ok(event) = events_rx.try_recv() {
        events.push(event);
    }

    Ok(Evidence {
        seed,
        base_counter,
        current,
        occurrences,
        events,
        attempts,
        route_removed,
        supervision_released,
        observer_drained,
    })
}

/// CONTRACT_SHAPE: pure-function.
fn check_evidence(evidence: &Evidence) -> Result<(), String> {
    let expected_terminal = Some(TerminalCondition::Stopped { by: StoppedBy::Operator });
    if evidence.current.state != AllocState::Terminated
        || evidence.current.terminal != expected_terminal
        || evidence.current.updated_at.counter != evidence.base_counter + 2
    {
        return Err(format!(
            "seed {}: authoritative terminal did not converge after rebase: current={:?}, expected counter={} and operator terminal",
            evidence.seed,
            evidence.current,
            evidence.base_counter + 2
        ));
    }

    let expected_attempts = [
        WriteAttempt {
            source: TransitionSource::Driver(DriverType::Vm),
            at: LogicalTimestamp {
                counter: evidence.base_counter + 1,
                writer: evidence.current.node_id.clone(),
            },
            terminal: None,
            verdict: WriteVerdict::Accepted,
        },
        WriteAttempt {
            source: TransitionSource::Reconciler,
            at: LogicalTimestamp {
                counter: evidence.base_counter + 1,
                writer: evidence.current.node_id.clone(),
            },
            terminal: expected_terminal.clone(),
            verdict: WriteVerdict::Rejected,
        },
        WriteAttempt {
            source: TransitionSource::Reconciler,
            at: LogicalTimestamp {
                counter: evidence.base_counter + 2,
                writer: evidence.current.node_id.clone(),
            },
            terminal: expected_terminal.clone(),
            verdict: WriteVerdict::Accepted,
        },
    ];
    if evidence.attempts != expected_attempts {
        return Err(format!(
            "seed {}: expected real exit accept -> equal-timestamp Stop LWW loss -> one rebased Stop accept; attempts={:?}",
            evidence.seed, evidence.attempts
        ));
    }

    if evidence.occurrences.len() != 3 {
        return Err(format!(
            "seed {}: rejected proposal must append no occurrence; expected Running + driver terminal + rebased operator terminal, got {:?}",
            evidence.seed, evidence.occurrences
        ));
    }
    let driver_occurrence = &evidence.occurrences[1];
    let operator_occurrence = &evidence.occurrences[2];
    if driver_occurrence.source != TransitionSource::Driver(DriverType::Vm)
        || driver_occurrence.at.counter != evidence.base_counter + 1
        || driver_occurrence.terminal.is_some()
        || operator_occurrence.source != TransitionSource::Reconciler
        || operator_occurrence.at.counter != evidence.base_counter + 2
        || operator_occurrence.terminal != expected_terminal
    {
        return Err(format!(
            "seed {}: occurrence semantics diverged: {:?}",
            evidence.seed, evidence.occurrences
        ));
    }

    let driver_events = evidence
        .events
        .iter()
        .filter(|event| {
            event.source == TransitionSource::Driver(DriverType::Vm)
                && event.terminal.is_none()
                && matches!(event.reason, TransitionReason::Stopped { by: StoppedBy::Operator })
        })
        .count();
    let operator_events = evidence
        .events
        .iter()
        .filter(|event| {
            event.source == TransitionSource::Reconciler
                && event.terminal == expected_terminal
                && matches!(event.reason, TransitionReason::Stopped { by: StoppedBy::Reconciler })
        })
        .count();
    if evidence.events.len() != 2 || driver_events != 1 || operator_events != 1 {
        return Err(format!(
            "seed {}: exactly the two accepted terminal occurrences must broadcast (rejected LWW loser emits none); events={:?}",
            evidence.seed, evidence.events
        ));
    }

    if !evidence.route_removed || !evidence.supervision_released || !evidence.observer_drained {
        return Err(format!(
            "seed {}: terminal convergence left cleanup ownership behind: route_removed={}, supervision_released={}, observer_drained={}",
            evidence.seed,
            evidence.route_removed,
            evidence.supervision_released,
            evidence.observer_drained
        ));
    }

    Ok(())
}

/// Negative control: erase the actual equal-timestamp LWW loss from the
/// captured driven-port trace. A checker without rebase teeth would pass it.
fn hide_lww_loss(evidence: &Evidence) -> Evidence {
    let mut defective = evidence.clone();
    defective.attempts.retain(|attempt| {
        !(attempt.source == TransitionSource::Reconciler
            && attempt.verdict == WriteVerdict::Rejected)
    });
    defective
}

/// Evaluate the seeded production stop/exit contention invariant.
pub async fn evaluate(seed: u64) -> InvariantResult {
    let evidence = match drive(seed).await {
        Ok(evidence) => evidence,
        Err(cause) => return fail(cause),
    };
    if let Err(cause) = check_evidence(&evidence) {
        return fail(cause);
    }
    if check_evidence(&hide_lww_loss(&evidence)).is_ok() {
        return fail(format!(
            "seed {seed}: negative control hid the observed LWW loss but the invariant still passed"
        ));
    }
    pass()
}

fn pass() -> InvariantResult {
    InvariantResult {
        name: NAME.to_owned(),
        status: InvariantStatus::Pass,
        tick: 3,
        host: HOST.to_owned(),
        cause: None,
    }
}

fn fail(cause: String) -> InvariantResult {
    InvariantResult {
        name: NAME.to_owned(),
        status: InvariantStatus::Fail,
        tick: 3,
        host: HOST.to_owned(),
        cause: Some(cause),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "current_thread")]
    async fn fixed_seed_drives_real_lww_loss_and_one_rebase() {
        let result = evaluate(424_242).await;
        assert_eq!(result.status, InvariantStatus::Pass, "{:?}", result.cause);
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[tokio::test(flavor = "current_thread")]
    async fn invariant_has_teeth_when_lww_loss_is_hidden() {
        let evidence = match drive(424_242).await {
            Ok(evidence) => evidence,
            Err(cause) => panic!("fixed seed did not drive contention: {cause}"),
        };
        if let Err(cause) = check_evidence(&evidence) {
            panic!("healthy production trace violated the invariant: {cause}");
        }
        let Err(cause) = check_evidence(&hide_lww_loss(&evidence)) else {
            panic!("removing the actual LWW loser must turn the invariant red");
        };
        assert!(cause.contains("LWW loss"), "unexpected teeth failure: {cause}");
    }
}
