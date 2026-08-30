//! T-C + T-G + T-H (ADR-0078 § D6) — the action shim writes the
//! crash-observability facts, at the real call sites, against a real
//! observation store.
//!
//! WHY-NEW-FILE: crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs
//!   CLOSEST-EXISTING: crates/overdrive-control-plane/tests/acceptance/finalize_failed_forward_carries_workload_addr.rs
//!   EXTENSION-COST: that file's module doc, filename and mutation
//!     rationale all scope it to ONE branch — the `workload_addr`
//!     forward-carry in the `FinalizeFailed` arm — and it drives only
//!     `Action::FinalizeFailed`. Three of the four scenarios here drive
//!     `Action::RestartAllocation` against a terminal prior, which that
//!     file's whole `finalize_and_read_successor` harness (an
//!     `InertDriver` whose `start` deliberately errors) structurally
//!     cannot express.
//!   PARALLEL-RATIONALE: different action under test
//!     (`RestartAllocation`, which requires a driver that actually
//!     STARTS), a different driven-port shape (a driver double whose
//!     accept/reject behaviour is the variable under test), and a
//!     different assertion surface (`last_terminated` / `restart_count`
//!     rather than `workload_addr`).
//!
//! # PORT-TO-PORT litmus
//!
//! Drives the production driving port `action_shim::dispatch` and
//! asserts at the driven-port boundary (the `AllocStatusRow` written to
//! the `SimObservationStore`) — never on internal state. This is the
//! falsifiable form of the § D2 per-site disposition table: a site that
//! passes the wrong `prior`, or a builder that stops computing the facts
//! from `(prior, state)`, turns these RED.
//!
//! Default lane: sim adapters for every port, `mtls_worker: None`, no
//! root, no netns. The Tier-3 sibling that drives the REAL exit observer
//! through two crash cycles is
//! `tests/integration/workload_lifecycle/crash_observability_two_cycles.rs`
//! (T-F).

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use overdrive_control_plane::action_shim::{
    ShimError, WorkloadNetworkProvisioner, dispatch, dispatch_with_network_provisioner,
};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_control_plane::veth_provisioner::{
    NetSlotAllocator, VethProvisionError, VmTapPlan, WorkloadNetnsPlan,
};
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationStoreError,
};
use overdrive_core::transition_reason::{TerminalCondition, TransitionReason};
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

#[cfg(feature = "integration-tests")]
use overdrive_core::traits::IdentityRead;
#[cfg(feature = "integration-tests")]
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
#[cfg(feature = "integration-tests")]
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
#[cfg(feature = "integration-tests")]
use overdrive_sim::adapters::{
    SimIdentityRead, SimMtlsEnforcement, SimMtlsIntercept, SimMtlsResolve,
};
#[cfg(feature = "integration-tests")]
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;

/// Driver double whose `start` outcome is the variable under test:
/// `Accept` models a driver that brings the workload back up (T-C),
/// `Reject` models the `StartRejected` shape (T-G).
#[derive(Clone, Copy, PartialEq, Eq)]
enum StartOutcome {
    Accept,
    Reject,
}

struct ScriptedDriver {
    outcome: StartOutcome,
    driver_type: DriverType,
    terminal_calls: Arc<AtomicUsize>,
    stop_calls: Arc<AtomicUsize>,
    stop_failures_remaining: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Driver for ScriptedDriver {
    fn r#type(&self) -> DriverType {
        self.driver_type
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        match self.outcome {
            StartOutcome::Accept => {
                Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: Some(4242) })
            }
            StartOutcome::Reject => Err(DriverError::StartRejected {
                failure: overdrive_core::traits::driver::DriverStartFailure {
                    class: overdrive_core::traits::driver::DriverStartClass::Unclassified {
                        driver: DriverType::Exec,
                    },
                    detail: "scripted rejection: no capacity".to_owned(),
                },
            }),
        }
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
        self.stop_calls.fetch_add(1, Ordering::SeqCst);
        if self
            .stop_failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| remaining.checked_sub(1))
            .is_ok()
        {
            return Err(DriverError::Io(std::io::Error::other("injected driver stop I/O")));
        }
        Ok(())
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        Err(DriverError::NotFound { alloc: handle.alloc.clone() })
    }

    async fn resize(
        &self,
        _handle: &AllocationHandle,
        _resources: Resources,
    ) -> Result<(), DriverError> {
        Ok(())
    }

    fn on_alloc_terminal(&self, _alloc_id: &AllocationId) {
        self.terminal_calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct CountingNetworkProvisioner {
    attempts: AtomicUsize,
    teardowns: AtomicUsize,
    failures_remaining: AtomicUsize,
}

impl CountingNetworkProvisioner {
    const fn fail_first_teardown() -> Self {
        Self {
            attempts: AtomicUsize::new(0),
            teardowns: AtomicUsize::new(0),
            failures_remaining: AtomicUsize::new(1),
        }
    }
}

impl WorkloadNetworkProvisioner for CountingNetworkProvisioner {
    fn provision(
        &self,
        _workload: &WorkloadNetnsPlan,
        _vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        Ok(())
    }

    fn teardown(&self, _workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if self
            .failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| remaining.checked_sub(1))
            .is_ok()
        {
            return Err(VethProvisionError::SysctlSetFailed {
                key: "net.ipv4.ip_forward".to_owned(),
                value: "1".to_owned(),
                path: "/injected/finalization/teardown".to_owned(),
                source: std::io::Error::other("injected first teardown failure"),
            });
        }
        self.teardowns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn alloc_id() -> AllocationId {
    AllocationId::new("alloc-crashobs-0").expect("valid alloc id")
}

fn workload_id() -> WorkloadId {
    WorkloadId::new("crashobs").expect("valid workload id")
}

fn node_id() -> NodeId {
    NodeId::new("node-001").expect("valid node id")
}

fn spec() -> AllocationSpec {
    AllocationSpec {
        alloc: alloc_id(),
        identity: SpiffeId::new("spiffe://overdrive.local/workload/crashobs/alloc/0")
            .expect("valid spiffe id"),
        driver: overdrive_core::traits::driver::DriverPayload::Exec(
            overdrive_core::traits::driver::ExecPayload {
                command: "/bin/true".to_owned(),
                args: Vec::new(),
            },
        ),
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

/// Seed a terminal `Failed` row at `counter` carrying the crash facts a
/// SIGKILL produces, plus whatever crash history the scenario needs the
/// successor write to forward or replace.
fn seeded_failed_row(
    counter: u64,
    restart_count: u32,
    last_terminated: Option<overdrive_core::traits::observation_store::LastTerminated>,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: alloc_id(),
        workload_id: workload_id(),
        node_id: node_id(),
        state: AllocState::Failed,
        updated_at: LogicalTimestamp { counter, writer: node_id() },
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(137),
            signal: Some(9),
            stderr_tail: Some("Segmentation fault".to_owned()),
        }),
        detail: Some("killed by SIGKILL".to_owned()),
        terminal: None,
        stderr_tail: Some("Segmentation fault".to_owned()),
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated,
        restart_count,
    }
}

/// Dispatch `action` against a store seeded with `seed`, and return the
/// LWW-winner row afterwards.
async fn dispatch_against_seed(seed: AllocStatusRow, action: Action) -> AllocStatusRow {
    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    obs.write(ObservationRow::AllocStatus(Box::new(seed))).await.expect("seed prior row");

    let start_outcome = if matches!(action, Action::RestartAllocation { .. }) {
        StartOutcome::Accept
    } else {
        StartOutcome::Reject
    };
    dispatch_with_driver(obs.as_ref(), store, action, start_outcome).await;

    obs.alloc_status_row(&alloc_id())
        .await
        .expect("read alloc row")
        .expect("a successor row must exist after dispatch")
}

async fn dispatch_with_driver(
    obs: &dyn ObservationStore,
    store: Arc<dyn IntentStore>,
    action: Action,
    outcome: StartOutcome,
) {
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
    let driver: Arc<dyn Driver> = Arc::new(ScriptedDriver {
        outcome,
        driver_type: DriverType::Exec,
        terminal_calls: Arc::new(AtomicUsize::new(0)),
        stop_calls: Arc::new(AtomicUsize::new(0)),
        stop_failures_remaining: Arc::new(AtomicUsize::new(0)),
    });
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let writer_node = NodeId::new("writer-1").expect("NodeId");
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));
    let net_slot_allocator = NetSlotAllocator::new();
    let test_broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());

    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    dispatch(
        vec![action],
        drivers.as_ref(),
        &alloc_drivers,
        obs,
        dataplane.as_ref(),
        &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        )),
        &overdrive_sim::adapters::clock::SimClock::new(),
        &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        &lifecycle_tx,
        &tick,
        &writer_node,
        Arc::clone(&allocator),
        &test_broker,
        None,
        None,
        &net_slot_allocator,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await
    .expect("dispatch must succeed");
}

/// Dispatch a `RestartAllocation` against `seed`, with the driver's
/// `start` outcome as the variable.
async fn restart_against(seed: AllocStatusRow, outcome: StartOutcome) -> AllocStatusRow {
    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));

    obs.write(ObservationRow::AllocStatus(Box::new(seed))).await.expect("seed prior row");

    dispatch_with_driver(
        obs.as_ref(),
        store,
        Action::RestartAllocation {
            alloc_id: alloc_id(),
            spec: spec(),
            kind: WorkloadKind::Service,
            // `None` is the crash-loop restart pathway — the restart cause
            // is implicit in the prior alloc's terminal, which is exactly
            // the shape under test.
        },
        outcome,
    )
    .await;

    obs.alloc_status_row(&alloc_id())
        .await
        .expect("read alloc row")
        .expect("a successor row must exist after dispatch")
}

// ---------------------------------------------------------------------------
// T-C — the shim writes the facts on a successful restart
// ---------------------------------------------------------------------------

/// T-C: seed a `Failed` row at counter `K` carrying
/// `WorkloadCrashedImmediately`; dispatch `Action::RestartAllocation`;
/// the stored row is `Running`, `restart_count == 1`, and
/// `last_terminated` snapshots the seeded row.
///
/// This is § D2 site 5 — the crash-observability site — exercised
/// end-to-end through the real dispatcher.
#[tokio::test]
async fn restart_allocation_snapshots_the_crash_and_counts_the_restart() {
    let seed = seeded_failed_row(7, 0, None);
    let row = restart_against(seed.clone(), StartOutcome::Accept).await;

    assert_eq!(row.state, AllocState::Running, "a successful restart lands Running");
    assert_eq!(row.restart_count, 1, "the observed restart is counted");

    let lt = row.last_terminated.as_ref().expect("the recovered row must carry last_terminated");
    assert_eq!(lt.state, AllocState::Failed, "the snapshot describes the terminal it superseded");
    assert_eq!(lt.reason, seed.reason, "the typed cause-class rides the snapshot verbatim");
    assert_eq!(lt.detail, seed.detail, "the verbatim driver text rides the snapshot");
    assert_eq!(lt.stderr_tail, seed.stderr_tail, "the workload's dying words ride the snapshot");
    assert_eq!(lt.started_at, seed.started_at, "the dead generation's start wall-clock rides it");
    assert_eq!(
        lt.terminated_at, seed.updated_at,
        "the snapshot identifies exactly WHICH durable observation it summarises",
    );

    assert!(
        row.updated_at.dominates(&lt.terminated_at),
        "the recovered row must strictly dominate the terminal it snapshots",
    );
}

// ---------------------------------------------------------------------------
// T-G — a driver-REJECTED restart forwards; it neither counts nor overwrites
// ---------------------------------------------------------------------------

/// T-G: seed a `Failed` row already carrying crash history; dispatch
/// `RestartAllocation` against a driver returning `StartRejected`. The
/// successor row stays `Failed`, `restart_count` is UNCHANGED, and
/// `last_terminated` is the FORWARDED prior value — not a snapshot of
/// the rejected row.
///
/// Pins § D1's `terminal → terminal` edge case at a real call site.
/// Nothing restarted, so nothing is counted; the prior crash's facts are
/// lost to the accepted depth-1 limit, and the *attempt* is counted
/// separately by the reconciler's own budget.
#[tokio::test]
async fn driver_rejected_restart_forwards_and_does_not_count() {
    let earlier = overdrive_core::traits::observation_store::LastTerminated {
        state: AllocState::Terminated,
        reason: Some(TransitionReason::Stopped {
            by: overdrive_core::transition_reason::StoppedBy::Process,
        }),
        detail: Some("an earlier, unrelated terminal".to_owned()),
        terminal: Some(TerminalCondition::Completed { exit_code: 0 }),
        stderr_tail: None,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_600_000_000))),
        terminated_at: LogicalTimestamp { counter: 3, writer: node_id() },
    };
    let seed = seeded_failed_row(7, 2, Some(earlier.clone()));

    let row = restart_against(seed, StartOutcome::Reject).await;

    assert_eq!(row.state, AllocState::Failed, "a rejected restart lands Failed");
    assert_eq!(row.restart_count, 2, "nothing restarted — the count is UNCHANGED");
    assert_eq!(
        row.last_terminated,
        Some(earlier),
        "the prior's snapshot is FORWARDED, not overwritten with the rejected row",
    );
}

// ---------------------------------------------------------------------------
// T-H — `FinalizeFailed` does not self-duplicate
// ---------------------------------------------------------------------------

/// T-H: seed a `Failed` row carrying a `reason`; dispatch
/// `FinalizeFailed { terminal: Some(BackoffExhausted { .. }) }`. The
/// written row's `last_terminated` is the FORWARDED prior value (`None`
/// on a first failure), NOT a snapshot of the row's own `reason` /
/// `detail` / `stderr_tail`.
///
/// This is the falsifiable form of § D2 site 3. `FinalizeFailed` is not a
/// *new* terminal — it is the same terminal restamped with a terminal
/// claim — so snapshotting there would put five facts on one row twice.
#[tokio::test]
async fn finalize_failed_does_not_snapshot_its_own_row() {
    let seed = seeded_failed_row(7, 0, None);

    let row = dispatch_against_seed(
        seed.clone(),
        Action::FinalizeFailed {
            alloc_id: alloc_id(),
            terminal: Some(TerminalCondition::BackoffExhausted { attempts: 3 }),
        },
    )
    .await;

    assert_eq!(row.state, AllocState::Failed, "a genuine terminal claim lands Failed");
    assert_eq!(
        row.last_terminated, None,
        "FinalizeFailed FORWARDS the prior's (absent) snapshot — it must NOT self-describe, \
         which would put reason/detail/stderr_tail/started_at/terminal on the row twice",
    );
    assert_eq!(row.restart_count, 0, "restamping a terminal is not a restart");
    // The row's OWN fields still carry the crash facts — that is where a
    // terminal row's facts live (§ D1's "last_terminated never describes
    // the row that carries it").
    assert_eq!(row.reason, seed.reason, "the row's own reason is forward-carried as before");
    assert_eq!(row.stderr_tail, seed.stderr_tail, "and so is its own stderr_tail");
}

/// The companion to T-H: a `FinalizeFailed` against a prior that ALREADY
/// carries a snapshot forwards that snapshot verbatim rather than
/// dropping it. Kills a mutant that replaces the forward-carry with a
/// literal `None` — which `finalize_failed_does_not_snapshot_its_own_row`
/// alone cannot catch, because there the expected value IS `None`.
#[tokio::test]
async fn finalize_failed_forwards_an_existing_snapshot() {
    let earlier = overdrive_core::traits::observation_store::LastTerminated {
        state: AllocState::Failed,
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(1),
            signal: None,
            stderr_tail: None,
        }),
        detail: Some("the crash this alloc previously survived".to_owned()),
        terminal: None,
        stderr_tail: None,
        started_at: None,
        terminated_at: LogicalTimestamp { counter: 4, writer: node_id() },
    };
    let seed = seeded_failed_row(7, 5, Some(earlier.clone()));

    let row = dispatch_against_seed(
        seed,
        Action::FinalizeFailed {
            alloc_id: alloc_id(),
            terminal: Some(TerminalCondition::BackoffExhausted { attempts: 5 }),
        },
    )
    .await;

    assert_eq!(
        row.last_terminated,
        Some(earlier),
        "the terminal row must keep describing the crash the alloc previously survived",
    );
    assert_eq!(row.restart_count, 5, "and the monotone counter rides through the terminal");
}

/// CONTRACT_SHAPE: bounded-change (one pre-READY terminal write, no restart side effect).
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn unreported_pre_ready_vmm_exit_finalizes_once_without_restart_or_view_change() {
    let mut seed = seeded_failed_row(7, 4, None);
    seed.kind = WorkloadKind::Job;
    seed.reason =
        Some(TransitionReason::VmGuestExitUnreported { vmm_exit_code: None, vmm_signal: Some(9) });

    let before = seed.clone();
    let row = dispatch_against_seed(
        seed,
        Action::FinalizeFailed {
            alloc_id: alloc_id(),
            terminal: Some(TerminalCondition::Failed { exit_code: None }),
        },
    )
    .await;

    assert_eq!(row.state, AllocState::Failed);
    assert_eq!(row.terminal, Some(TerminalCondition::Failed { exit_code: None }));
    assert_eq!(row.restart_count, 4, "finalization must not count a restart");
    assert!(matches!(
        row.reason,
        Some(TransitionReason::VmGuestExitUnreported { vmm_exit_code: None, vmm_signal: Some(9) })
    ));
    assert_eq!(row.alloc_id, before.alloc_id);
    assert_eq!(row.workload_id, before.workload_id);
    assert_eq!(row.node_id, before.node_id);
    assert_eq!(row.detail, before.detail);
    assert_eq!(row.stderr_tail, before.stderr_tail);
    assert_eq!(row.kind, before.kind);
    assert_eq!(row.listeners, before.listeners);
    assert_eq!(row.started_at, before.started_at);
    assert_eq!(row.workload_addr, before.workload_addr);
    assert_eq!(row.last_terminated, before.last_terminated);
    assert_eq!(row.updated_at.counter, before.updated_at.counter + 1);
    assert_eq!(row.updated_at.writer, before.updated_at.writer);
}

/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::too_many_lines,
    reason = "one action-boundary scenario keeps the complete duplicate-effect complement visible"
)]
#[tokio::test]
async fn same_job_finalization_is_terminal_and_count_preserving() {
    let mut seed = seeded_failed_row(11, 4, None);
    seed.kind = WorkloadKind::Job;
    seed.reason = Some(TransitionReason::WorkloadCrashedImmediately {
        exit_code: Some(78),
        signal: None,
        stderr_tail: None,
    });
    let action = Action::FinalizeFailed {
        alloc_id: alloc_id(),
        terminal: Some(TerminalCondition::Failed { exit_code: Some(78) }),
    };

    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn IntentStore> = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    obs.write(ObservationRow::AllocStatus(Box::new(seed.clone())))
        .await
        .expect("seed pre-final row");

    let selected_terminal_calls = Arc::new(AtomicUsize::new(0));
    let fallback_terminal_calls = Arc::new(AtomicUsize::new(0));
    let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
    registry.insert(Arc::new(ScriptedDriver {
        outcome: StartOutcome::Accept,
        driver_type: DriverType::Exec,
        terminal_calls: Arc::clone(&selected_terminal_calls),
        stop_calls: Arc::new(AtomicUsize::new(0)),
        stop_failures_remaining: Arc::new(AtomicUsize::new(0)),
    }));
    registry.insert(Arc::new(ScriptedDriver {
        outcome: StartOutcome::Accept,
        driver_type: DriverType::Vm,
        terminal_calls: Arc::clone(&fallback_terminal_calls),
        stop_calls: Arc::new(AtomicUsize::new(0)),
        stop_failures_remaining: Arc::new(AtomicUsize::new(0)),
    }));
    let effect_root = tmp.path().join("terminal-effects");
    let mut alloc_drivers =
        overdrive_control_plane::action_shim::AllocDriverIndex::persistent(effect_root.clone());
    alloc_drivers.lock().insert(alloc_id(), DriverType::Exec);
    let (lifecycle_tx, mut lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));
    let net_slots = NetSlotAllocator::new();
    net_slots.assign(alloc_id()).expect("pre-final allocation owns one network slot");
    let network = CountingNetworkProvisioner::fail_first_teardown();
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    let dataplane = overdrive_sim::adapters::dataplane::SimDataplane::new();
    let ca = overdrive_sim::adapters::ca::SimCa::new(Arc::new(
        overdrive_sim::adapters::entropy::SimEntropy::new(0),
    ));
    let clock = overdrive_sim::adapters::clock::SimClock::new();
    let identity = overdrive_control_plane::identity_mgr::IdentityMgr::new(None);
    let writer = NodeId::new("writer-1").expect("writer node");
    let host = overdrive_sim::adapters::vm_host_state::SimVmHostState::new();
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    #[cfg(feature = "integration-tests")]
    let mtls_worker = {
        let identity_read: Arc<dyn IdentityRead> =
            Arc::new(SimIdentityRead::new(std::collections::BTreeMap::new(), None));
        let enforcement: Arc<dyn MtlsEnforcement> =
            Arc::new(SimMtlsEnforcement::new(identity_read, MtlsLimits::default()));
        let resolve: Arc<dyn MtlsResolve> = Arc::new(SimMtlsResolve::new(
            std::collections::BTreeMap::new(),
            MtlsResolution::NonMesh,
        ));
        Arc::new(MtlsInterceptWorker::new(
            enforcement,
            resolve,
            Arc::new(overdrive_sim::adapters::clock::SimClock::new()),
            Arc::new(SimMtlsIntercept::new()),
        ))
    };
    #[cfg(feature = "integration-tests")]
    mtls_worker.start_alloc(&spec()).expect("record one live intercept before finalization");

    let attempt_count = if cfg!(feature = "integration-tests") { 5 } else { 4 };
    for attempt in 0..attempt_count {
        if attempt == 1 {
            obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
                "injected terminal-fence write failure",
            )));
        }
        let result = dispatch_with_network_provisioner(
            vec![action.clone()],
            &registry,
            &alloc_drivers,
            obs.as_ref(),
            &dataplane,
            &ca,
            &clock,
            &identity,
            &lifecycle_tx,
            &tick,
            &writer,
            Arc::clone(&allocator),
            &broker,
            None,
            {
                #[cfg(feature = "integration-tests")]
                {
                    Some(&mtls_worker)
                }
                #[cfg(not(feature = "integration-tests"))]
                {
                    None
                }
            },
            &net_slots,
            &network,
            &host,
        )
        .await;
        if attempt == 0 {
            assert!(matches!(result, Err(ShimError::WorkloadNetnsProvision(_))));
            let after_fault = obs
                .alloc_status_row(&alloc_id())
                .await
                .expect("read row after fault")
                .expect("pre-final row survives fault");
            assert_eq!(
                after_fault, seed,
                "a failed cleanup cannot publish the terminal completion fence"
            );
            assert!(matches!(
                lifecycle_rx.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ));
            assert_eq!(
                selected_terminal_calls.load(Ordering::SeqCst),
                0,
                "network cleanup must succeed before the process-local terminal hook"
            );
            assert_eq!(fallback_terminal_calls.load(Ordering::SeqCst), 0);
            assert_eq!(network.attempts.load(Ordering::SeqCst), 1);
            assert_eq!(network.teardowns.load(Ordering::SeqCst), 0);
            assert_eq!(net_slots.snapshot().len(), 1, "failed teardown retains ownership");
            #[cfg(feature = "integration-tests")]
            assert_eq!(mtls_worker.stop_alloc_calls_for_test(), 1);
            // Replacement immediately before the hook has no completion
            // marker, so the reconstructed exact driver owner must still run
            // it on the converging attempt.
            alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::persistent(
                effect_root.clone(),
            );
            alloc_drivers.lock().insert(alloc_id(), DriverType::Exec);
        } else if attempt == 1 {
            assert!(matches!(result, Err(ShimError::Observation(_))));
            let after_write_fault = obs
                .alloc_status_row(&alloc_id())
                .await
                .expect("read row after terminal-fence write fault")
                .expect("pre-final row survives terminal-fence write fault");
            assert_eq!(after_write_fault, seed, "a failed fence write remains retryable");
            assert!(matches!(
                lifecycle_rx.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ));
            assert_eq!(
                selected_terminal_calls.load(Ordering::SeqCst),
                1,
                "cleanup success reaches the process-local terminal hook before the durable fence"
            );
            assert_eq!(fallback_terminal_calls.load(Ordering::SeqCst), 0);
            assert_eq!(network.attempts.load(Ordering::SeqCst), 2);
            assert_eq!(network.teardowns.load(Ordering::SeqCst), 1);
            assert!(net_slots.snapshot().is_empty(), "completed cleanup stays complete");
            #[cfg(feature = "integration-tests")]
            assert_eq!(mtls_worker.stop_alloc_calls_for_test(), 1);
            // Replace the process-local owner after cleanup + hook succeeded
            // but before the durable terminal fence. The fresh owner must not
            // broadcast the hook to every composed driver on replay.
            alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::persistent(
                effect_root.clone(),
            );
            // A replacement may reconstruct the surviving process owner. The
            // durable effect marker, not absence of a process-local route,
            // suppresses the already-completed terminal hook.
            alloc_drivers.lock().insert(alloc_id(), DriverType::Exec);
            // The next attempt commits the terminal row and prepared original
            // event context, then fails immediately before event delivery.
            #[cfg(feature = "integration-tests")]
            alloc_drivers.inject_lifecycle_claim_failure_for_test();
        } else {
            #[cfg(feature = "integration-tests")]
            if attempt == 2 {
                assert!(matches!(result, Err(ShimError::TerminalEffectJournal(_))));
                let after_event_cut = obs
                    .alloc_status_row(&alloc_id())
                    .await
                    .expect("read row after pre-event cut")
                    .expect("terminal row commits before event projection");
                assert_eq!(after_event_cut.state, AllocState::Failed);
                assert!(matches!(
                    lifecycle_rx.try_recv(),
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty)
                ));
                // Replacement at row-before-event loads the durable original
                // `from`/source context rather than fabricating Failed→Failed.
                alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::persistent(
                    effect_root.clone(),
                );
                continue;
            }
            result.expect("fence retry and completed replay both succeed");
            let delivered_attempt = if cfg!(feature = "integration-tests") { 3 } else { 2 };
            if attempt == delivered_attempt {
                // Recreate the owner after the row + lifecycle outbox commit;
                // exact terminal replay must not publish the same event again.
                alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::persistent(
                    effect_root.clone(),
                );
            }
        }
    }

    let twice = obs
        .alloc_status_row(&alloc_id())
        .await
        .expect("read terminal row")
        .expect("terminal row exists");
    let first_event = lifecycle_rx.recv().await.expect("one terminal lifecycle event");
    assert_eq!(first_event.alloc_id, alloc_id());
    assert_eq!(
        first_event.from,
        AllocStateWire::Failed,
        "row-before-event replacement must retain the original transition source state"
    );
    assert!(
        matches!(lifecycle_rx.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)),
        "exact replay must emit no second lifecycle event"
    );
    assert_eq!(twice.state, AllocState::Failed);
    assert_eq!(twice.terminal, Some(TerminalCondition::Failed { exit_code: Some(78) }));
    assert_eq!(twice.updated_at.counter, 12, "exact replay authors no second durable transition");
    assert_eq!(twice.restart_count, 4, "finalization never counts a restart");
    assert_eq!(
        selected_terminal_calls.load(Ordering::SeqCst),
        1,
        "replacement after the hook boundary must not repeat the old process's terminal hook"
    );
    assert_eq!(
        fallback_terminal_calls.load(Ordering::SeqCst),
        0,
        "index removal after the first transition must not turn replay into all-driver fallback"
    );
    assert_eq!(network.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(network.teardowns.load(Ordering::SeqCst), 1);
    assert!(net_slots.snapshot().is_empty(), "the one teardown releases the one slot");
    #[cfg(feature = "integration-tests")]
    assert_eq!(
        mtls_worker.stop_alloc_calls_for_test(),
        1,
        "the action boundary, not the worker's internal idempotency, fences duplicate mTLS stop"
    );
}

/// CONTRACT_SHAPE: bounded-change (retained cleanup owner is unchanged).
#[tokio::test]
async fn finalize_failed_cannot_bypass_retained_failed_start_cleanup() {
    let mut seed = seeded_failed_row(17, 0, None);
    seed.state = AllocState::Pending;
    seed.reason = Some(TransitionReason::DriverInternalError {
        detail: "start cleanup retained by the VM driver".to_owned(),
    });
    seed.terminal = None;

    let row = dispatch_against_seed(
        seed.clone(),
        Action::FinalizeFailed {
            alloc_id: alloc_id(),
            terminal: Some(TerminalCondition::Failed { exit_code: None }),
        },
    )
    .await;

    assert_eq!(
        row, seed,
        "FinalizeFailed preserves the exact Pending cleanup-ownership token for 02-05's retry path"
    );
}

// ---------------------------------------------------------------------------
// § D2 site 6 — StopAllocation forwards
// ---------------------------------------------------------------------------

/// § D2 site 6: `Running → Terminated` is a non-terminal prior, so
/// `StopAllocation` forwards both fields. A Terminated row carrying a
/// prior generation's `last_terminated` keeps that history visible on the
/// durable surface an operator polls after the stop.
#[tokio::test]
async fn stop_allocation_forwards_the_crash_history() {
    let earlier = overdrive_core::traits::observation_store::LastTerminated {
        state: AllocState::Failed,
        reason: Some(TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(137),
            signal: Some(9),
            stderr_tail: None,
        }),
        detail: None,
        terminal: None,
        stderr_tail: None,
        started_at: None,
        terminated_at: LogicalTimestamp { counter: 2, writer: node_id() },
    };
    let mut seed = seeded_failed_row(7, 1, Some(earlier.clone()));
    seed.state = AllocState::Running;

    let row = dispatch_against_seed(
        seed,
        Action::StopAllocation {
            alloc_id: alloc_id(),
            terminal: Some(TerminalCondition::Stopped {
                by: overdrive_core::transition_reason::StoppedBy::Operator,
            }),
        },
    )
    .await;

    assert_eq!(row.state, AllocState::Terminated, "the stop lands Terminated");
    assert_eq!(row.restart_count, 1, "the monotone counter survives an operator stop");
    assert_eq!(
        row.last_terminated,
        Some(earlier),
        "a stopped alloc still shows the crash it previously survived",
    );
}

/// CONTRACT_SHAPE: bounded-change (fallible stop cleanup remains retryable until terminal-last).
#[allow(
    clippy::too_many_lines,
    reason = "one action-boundary scenario keeps driver, network, hook, event, and replacement cuts visible"
)]
#[tokio::test]
async fn stop_allocation_does_not_publish_terminal_before_network_cleanup_succeeds() {
    let mut seed = seeded_failed_row(23, 2, None);
    seed.state = AllocState::Running;
    seed.terminal = None;
    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn IntentStore> = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    obs.write(ObservationRow::AllocStatus(Box::new(seed.clone()))).await.expect("seed Running row");
    let terminal_calls = Arc::new(AtomicUsize::new(0));
    let stop_calls = Arc::new(AtomicUsize::new(0));
    let stop_failures_remaining = Arc::new(AtomicUsize::new(1));
    let mut drivers = overdrive_core::traits::driver::DriverRegistry::new();
    drivers.insert(Arc::new(ScriptedDriver {
        outcome: StartOutcome::Accept,
        driver_type: DriverType::Exec,
        terminal_calls: Arc::clone(&terminal_calls),
        stop_calls: Arc::clone(&stop_calls),
        stop_failures_remaining: Arc::clone(&stop_failures_remaining),
    }));
    let effect_root = tmp.path().join("terminal-effects");
    let mut alloc_drivers =
        overdrive_control_plane::action_shim::AllocDriverIndex::persistent(effect_root.clone());
    alloc_drivers.lock().insert(alloc_id(), DriverType::Exec);
    let net_slots = NetSlotAllocator::new();
    net_slots.assign(alloc_id()).expect("running alloc owns its slot");
    let network = CountingNetworkProvisioner::fail_first_teardown();
    let (lifecycle_tx, mut lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&store),
    )));
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_100)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    let action = Action::StopAllocation {
        alloc_id: alloc_id(),
        terminal: Some(TerminalCondition::Stopped {
            by: overdrive_core::transition_reason::StoppedBy::Operator,
        }),
    };
    let attempt_count = if cfg!(feature = "integration-tests") { 5 } else { 4 };
    for attempt in 0..attempt_count {
        let outcome = dispatch_with_network_provisioner(
            vec![action.clone()],
            &drivers,
            &alloc_drivers,
            obs.as_ref(),
            &overdrive_sim::adapters::dataplane::SimDataplane::new(),
            &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
                overdrive_sim::adapters::entropy::SimEntropy::new(0),
            )),
            &overdrive_sim::adapters::clock::SimClock::new(),
            &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
            &lifecycle_tx,
            &tick,
            &NodeId::new("writer-1").expect("writer"),
            Arc::clone(&allocator),
            &broker,
            None,
            None,
            &net_slots,
            &network,
            &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
        )
        .await;

        if attempt == 0 {
            assert!(matches!(outcome, Err(ShimError::Driver(DriverError::Io(_)))));
            assert_eq!(network.attempts.load(Ordering::SeqCst), 0);
        } else if attempt == 1 {
            assert!(matches!(outcome, Err(ShimError::WorkloadNetnsProvision(_))));
            assert_eq!(network.attempts.load(Ordering::SeqCst), 1);
        } else {
            #[cfg(feature = "integration-tests")]
            if attempt == 2 {
                assert!(matches!(outcome, Err(ShimError::TerminalEffectJournal(_))));
                assert_eq!(
                    obs.alloc_status_row(&alloc_id())
                        .await
                        .expect("read pre-event row")
                        .expect("terminal row exists")
                        .state,
                    AllocState::Terminated
                );
            } else {
                outcome.expect("cleanup convergence and exact completed replay succeed");
            }
            #[cfg(not(feature = "integration-tests"))]
            outcome.expect("cleanup convergence and exact completed replay succeed");
        }

        if attempt < 2 {
            assert_eq!(
                obs.alloc_status_row(&alloc_id()).await.expect("read row").expect("row remains"),
                seed,
                "failed cleanup must leave the non-terminal durable owner for retry"
            );
            assert!(matches!(
                lifecycle_rx.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ));
            assert_eq!(terminal_calls.load(Ordering::SeqCst), 0);
            assert_eq!(net_slots.snapshot().len(), 1);
            // Process replacement at either failed boundary reconstructs the
            // live driver route and retries from the still-nonterminal row.
            alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::persistent(
                effect_root.clone(),
            );
            alloc_drivers.lock().insert(alloc_id(), DriverType::Exec);
            if attempt == 1 {
                #[cfg(feature = "integration-tests")]
                alloc_drivers.inject_lifecycle_claim_failure_for_test();
            }
        } else if attempt == 2 || (cfg!(feature = "integration-tests") && attempt == 3) {
            alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::persistent(
                effect_root.clone(),
            );
        }
    }

    let terminal_row =
        obs.alloc_status_row(&alloc_id()).await.expect("read row").expect("terminal row exists");
    assert_eq!(terminal_row.state, AllocState::Terminated);
    assert_eq!(stop_calls.load(Ordering::SeqCst), 3, "completed replay does not stop again");
    assert_eq!(network.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(network.teardowns.load(Ordering::SeqCst), 1);
    assert_eq!(terminal_calls.load(Ordering::SeqCst), 1);
    assert!(net_slots.snapshot().is_empty());
    let event = lifecycle_rx.recv().await.expect("one terminal event");
    assert_eq!(
        event.from,
        AllocStateWire::Running,
        "Stop row-before-event replacement retains the original Running source"
    );
    assert!(matches!(
        lifecycle_rx.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}

// ---------------------------------------------------------------------------
// § D2 — the structured `alloc.restart.observed` event
// ---------------------------------------------------------------------------
//
// ADR-0078 § D2 mandates a structured event at the single increment site so a
// crash-and-recover is *alertable*, not merely pollable. Its emission is gated
// on `facts.restart_count > superseded.restart_count` inside
// `build_alloc_status_row` — a comparison with NO effect on the written row, so
// nothing else in the suite can falsify it. Without these two tests the gate's
// `>` mutants (`==`, `<`, `>=`) all survive and the "alertable" claim is
// folklore.
//
// Capture mechanism: thread-local `tracing::subscriber::set_default`, the same
// harness `sim_observation_lww_reject_logging.rs` uses. Thread-local is
// sufficient because `build_alloc_status_row` emits synchronously on the
// caller's thread inside `dispatch`, and every test here is
// `flavor = "current_thread"`.

use std::sync::Mutex;
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

/// Records every event the subscriber sees as `"<name> | f=v …"`.
#[derive(Clone, Default)]
struct CapturedEvents {
    inner: Arc<Mutex<Vec<String>>>,
}

impl CapturedEvents {
    fn restart_observed(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("captured events mutex")
            .iter()
            .filter(|e| e.contains("alloc.restart.observed"))
            .cloned()
            .collect()
    }
}

struct V<'a>(&'a mut String);

impl tracing::field::Visit for V<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        let _ = write!(self.0, " {}={:?}", field.name(), value);
    }
}

impl<S> Layer<S> for CapturedEvents
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut buf = String::new();
        buf.push_str(event.metadata().name());
        event.record(&mut V(&mut buf));
        self.inner.lock().expect("captured events mutex").push(buf);
    }
}

/// The event FIRES on the write that observes a restart landing, and carries
/// the alloc, the workload, the new count, and the state it recovered from.
#[tokio::test(flavor = "current_thread")]
async fn restart_landing_emits_the_structured_alloc_restart_observed_event() {
    let captured = CapturedEvents::default();
    let _guard = set_default(Registry::default().with(captured.clone()));

    let seed = seeded_failed_row(7, 4, None);
    let row = restart_against(seed, StartOutcome::Accept).await;
    assert_eq!(row.restart_count, 5, "precondition: the restart was counted");

    let events = captured.restart_observed();
    assert_eq!(
        events.len(),
        1,
        "exactly one alloc.restart.observed event per observed restart; got {events:?}",
    );
    let event = &events[0];
    assert!(event.contains("restart_count=5"), "the event carries the NEW count; got {event:?}");
    assert!(event.contains("alloc-crashobs-0"), "and the alloc; got {event:?}");
    assert!(event.contains("crashobs"), "and the workload; got {event:?}");
    assert!(
        event.contains("prior_state=failed"),
        "and the terminal state it recovered from; got {event:?}",
    );
}

/// The event does NOT fire when no restart landed — a driver-rejected restart
/// against the same terminal prior. An alert that fires on a non-event is
/// noise, not signal.
///
/// Together with the test above this pins the `>` gate exactly: `>=` and `==`
/// both fire here (the counter is unchanged), and `<` fails to fire above.
#[tokio::test(flavor = "current_thread")]
async fn a_rejected_restart_emits_no_alloc_restart_observed_event() {
    let captured = CapturedEvents::default();
    let _guard = set_default(Registry::default().with(captured.clone()));

    let seed = seeded_failed_row(7, 4, None);
    let row = restart_against(seed, StartOutcome::Reject).await;
    assert_eq!(row.restart_count, 4, "precondition: nothing restarted, nothing counted");

    assert!(
        captured.restart_observed().is_empty(),
        "no restart landed — the event must stay silent; got {:?}",
        captured.restart_observed(),
    );
}

/// A forward-carry write (a `StopAllocation` against a `Running` prior that
/// already carries a non-zero count) emits nothing either. Kills the `>=`
/// mutant on a path where the counter is carried rather than incremented.
#[tokio::test(flavor = "current_thread")]
async fn a_forward_carry_write_emits_no_alloc_restart_observed_event() {
    let captured = CapturedEvents::default();
    let _guard = set_default(Registry::default().with(captured.clone()));

    let mut seed = seeded_failed_row(7, 4, None);
    seed.state = AllocState::Running;
    let row = dispatch_against_seed(
        seed,
        Action::StopAllocation { alloc_id: alloc_id(), terminal: None },
    )
    .await;
    assert_eq!(row.restart_count, 4, "precondition: the count was forwarded, not incremented");

    assert!(
        captured.restart_observed().is_empty(),
        "a forward-carry is not a restart — the event must stay silent; got {:?}",
        captured.restart_observed(),
    );
}
