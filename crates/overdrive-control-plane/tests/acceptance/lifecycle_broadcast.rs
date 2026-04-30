//! Acceptance — Slice 02 step 02-01.
//!
//! `S-CP-04` + `S-CP-05` — `LifecycleEvent` broadcast wiring + cause-class
//! classifier. Both scenarios drive through the action shim's
//! `dispatch` — the action shim IS the driving port for the
//! row-write-and-broadcast contract.
//!
//! Per ADR-0032 §4 Amendment 2026-04-30 (cause-class refactor) and
//! design/architecture.md §10 (single writer of `AllocStatusRow` is the
//! action shim, broadcast emit is one more side effect of the same
//! layer).
//!
//! # S-CP-04 — broadcast emits exactly N events for N transitions
//!
//! Property test, 1024 cases, N ∈ [1, 32]: dispatch N successful
//! `StartAllocation` actions through the shim; assert that exactly N
//! `LifecycleEvent` values land on the broadcast channel in submit
//! order. The test subscribes to the channel BEFORE dispatch.
//!
//! # S-CP-05 — classifier prefix-match table
//!
//! Five branches over `DriverError::StartRejected.reason` text:
//!
//! | `reason_text`                                                        | variant                                      |
//! |---|---|
//! | `spawn /no/such: No such file or directory (os error 2)`             | `ExecBinaryNotFound { path: "/no/such" }`    |
//! | `spawn /usr/local/bin/payments: Permission denied (os error 13)`     | `ExecPermissionDenied { path: "..." }`       |
//! | `spawn /tmp/garbage: Exec format error (os error 8)`                 | `ExecBinaryInvalid { path, kind }`           |
//! | `cgroup setup failed: place_pid: ...`                                | `CgroupSetupFailed { kind, source }`         |
//! | `(unclassified driver text)`                                         | `DriverInternalError { detail }`             |
//!
//! Each branch:
//!   1. Constructs a sim driver returning `StartRejected` with the
//!      tabled reason text.
//!   2. Dispatches a single `Action::StartAllocation`.
//!   3. Asserts the written `AllocStatusRow.reason` matches the typed
//!      cause-class variant; `AllocStatusRow.detail` carries the
//!      verbatim text (audit trail per architecture.md).
//!   4. Asserts the broadcast `LifecycleEvent.reason` is the same
//!      typed variant (byte-equal to the row's reason).
//!   5. Asserts the row's `state` is `Failed` (NOT `Terminated`) —
//!      driver-start failure is now the dedicated terminal-failure
//!      lifecycle bucket per ADR-0032 §5.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use proptest::prelude::*;

use overdrive_control_plane::action_shim::{LifecycleEvent, dispatch};
use overdrive_core::SpiffeId;
use overdrive_core::TransitionReason;
use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::reconciler::{Action, TickContext};
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, Resources,
};
use overdrive_core::traits::observation_store::{AllocState, ObservationRow, ObservationStore};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------

/// Sim driver that always succeeds on `start`. Used by S-CP-04 to
/// drive the success-path broadcast emission.
struct AlwaysOkDriver;

#[async_trait]
impl Driver for AlwaysOkDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        Ok(AllocationHandle { alloc: spec.alloc.clone(), pid: None })
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
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
}

/// Sim driver that returns `DriverError::StartRejected { driver, reason }`
/// on every `start` call. Configured via constructor.
struct FailingDriver {
    reason_text: String,
}

impl FailingDriver {
    fn new(reason_text: impl Into<String>) -> Self {
        Self { reason_text: reason_text.into() }
    }
}

#[async_trait]
impl Driver for FailingDriver {
    fn r#type(&self) -> DriverType {
        DriverType::Exec
    }

    async fn start(&self, _spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        Err(DriverError::StartRejected {
            driver: DriverType::Exec,
            reason: self.reason_text.clone(),
        })
    }

    async fn stop(&self, _handle: &AllocationHandle) -> Result<(), DriverError> {
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
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_spec(alloc_id: &AllocationId, job_id: &JobId) -> AllocationSpec {
    let identity = SpiffeId::new(&format!(
        "spiffe://overdrive.local/job/{}/alloc/{}",
        job_id.as_str(),
        alloc_id.as_str(),
    ))
    .expect("spiffe id");
    AllocationSpec {
        alloc: alloc_id.clone(),
        identity,
        command: "/bin/true".to_owned(),
        args: vec![],
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
    }
}

fn fresh_node() -> NodeId {
    NodeId::new("local").expect("node id")
}

fn make_tick(tick_n: u64) -> TickContext {
    let now = Instant::now();
    TickContext { now, tick: tick_n, deadline: now + Duration::from_secs(1) }
}

// ---------------------------------------------------------------------------
// S-CP-04 — N transitions emit exactly N broadcast events, in order
// ---------------------------------------------------------------------------

/// Drain at most `max` events from `rx` non-blockingly. Returns the
/// drained vec. Used by S-CP-04 to assert that exactly the expected
/// number of events were broadcast.
fn drain_events(rx: &mut broadcast::Receiver<LifecycleEvent>, max: usize) -> Vec<LifecycleEvent> {
    let mut events = Vec::with_capacity(max);
    for _ in 0..max {
        match rx.try_recv() {
            Ok(event) => events.push(event),
            Err(_) => break,
        }
    }
    events
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1024,
        ..ProptestConfig::default()
    })]

    /// S-CP-04: For any N ∈ [1, 32] successful StartAllocation actions
    /// dispatched through the action shim, exactly N `LifecycleEvent`s
    /// land on the broadcast channel, in the order the actions were
    /// dispatched.
    #[test]
    fn s_cp_04_broadcast_emits_exactly_n_events_in_order(n in 1usize..=32) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        runtime.block_on(async move {
            // Broadcast channel — capacity comfortably above N=32 so
            // no slow-receiver lag in this test.
            let (tx, mut rx) = broadcast::channel::<LifecycleEvent>(256);

            let driver: Arc<dyn Driver> = Arc::new(AlwaysOkDriver);
            let obs: Arc<dyn ObservationStore> =
                Arc::new(SimObservationStore::single_peer(fresh_node(), 0));
            let job_id = JobId::new("payments").expect("job id");
            let node_id = fresh_node();

            // Build N successful StartAllocation actions, each with a
            // distinct alloc id so the obs store sees N distinct rows.
            let mut actions: Vec<Action> = Vec::with_capacity(n);
            let mut expected_alloc_ids: Vec<AllocationId> = Vec::with_capacity(n);
            for i in 0..n {
                let alloc_id = AllocationId::new(&format!("alloc-{i}"))
                    .expect("alloc id");
                expected_alloc_ids.push(alloc_id.clone());
                let spec = build_spec(&alloc_id, &job_id);
                actions.push(Action::StartAllocation {
                    alloc_id,
                    job_id: job_id.clone(),
                    node_id: node_id.clone(),
                    spec,
                });
            }

            let tick = make_tick(0);

            // Dispatch — the shim writes N rows AND broadcasts N events.
            dispatch(actions, driver.as_ref(), obs.as_ref(), &tx, &tick)
                .await
                .expect("dispatch must succeed");

            // Assert exactly N events arrived on the channel, in submit
            // order (broadcast preserves send order to all subscribers).
            let events = drain_events(&mut rx, n + 1);
            prop_assert_eq!(events.len(), n,
                "expected exactly N={} events, got {}", n, events.len());

            for (i, event) in events.iter().enumerate() {
                prop_assert_eq!(&event.alloc_id, &expected_alloc_ids[i],
                    "event {} alloc_id mismatch", i);
                prop_assert_eq!(&event.job_id, &job_id);
                prop_assert!(matches!(event.reason, TransitionReason::Started));
            }

            Ok::<(), TestCaseError>(())
        })?;
    }
}

// ---------------------------------------------------------------------------
// S-CP-05 — classifier prefix-match table (5 branches)
// ---------------------------------------------------------------------------

/// Run a single classifier scenario. Sets up the action shim with a
/// `FailingDriver` returning `reason_text`, dispatches one
/// `StartAllocation`, then asserts:
///   - the written `AllocStatusRow.reason` matches `expected_reason`
///   - the written row's `detail` carries `reason_text` verbatim
///   - the written row's `state` is `Failed` (not `Terminated`)
///   - the broadcast event's `reason` matches `expected_reason`
async fn run_classifier_scenario(reason_text: &str, expected_reason: TransitionReason) {
    let (tx, mut rx) = broadcast::channel::<LifecycleEvent>(16);

    let driver: Arc<dyn Driver> = Arc::new(FailingDriver::new(reason_text));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(fresh_node(), 0));

    let alloc_id = AllocationId::new("alloc-fail").expect("alloc id");
    let job_id = JobId::new("payments").expect("job id");
    let node_id = fresh_node();
    let spec = build_spec(&alloc_id, &job_id);
    let action = Action::StartAllocation {
        alloc_id: alloc_id.clone(),
        job_id: job_id.clone(),
        node_id: node_id.clone(),
        spec,
    };

    let tick = make_tick(0);

    dispatch(vec![action], driver.as_ref(), obs.as_ref(), &tx, &tick)
        .await
        .expect("dispatch must succeed even on driver failure (failure is recorded)");

    // Assert the row.
    let rows = obs.alloc_status_rows().await.expect("read rows");
    assert_eq!(rows.len(), 1, "exactly one row written");
    let row = &rows[0];
    assert_eq!(
        row.state,
        AllocState::Failed,
        "StartRejected must write state=Failed (not Terminated) per ADR-0032 §5"
    );
    assert_eq!(
        row.reason,
        Some(expected_reason.clone()),
        "row.reason must be the classified cause-class variant"
    );
    assert_eq!(
        row.detail.as_deref(),
        Some(reason_text),
        "row.detail must carry verbatim driver text for audit"
    );

    // Assert the broadcast event.
    let event = rx.try_recv().expect("broadcast event must arrive");
    assert_eq!(event.alloc_id, alloc_id);
    assert_eq!(
        event.reason, expected_reason,
        "event.reason must match the row's classified reason"
    );
    // Ensure no extra events.
    assert!(rx.try_recv().is_err(), "exactly one broadcast event per row write");
}

#[tokio::test]
async fn s_cp_05_classifier_enoent_to_exec_binary_not_found() {
    run_classifier_scenario(
        "spawn /no/such: No such file or directory (os error 2)",
        TransitionReason::ExecBinaryNotFound { path: "/no/such".to_owned() },
    )
    .await;
}

#[tokio::test]
async fn s_cp_05_classifier_eacces_to_exec_permission_denied() {
    run_classifier_scenario(
        "spawn /usr/local/bin/payments: Permission denied (os error 13)",
        TransitionReason::ExecPermissionDenied { path: "/usr/local/bin/payments".to_owned() },
    )
    .await;
}

#[tokio::test]
async fn s_cp_05_classifier_enoexec_to_exec_binary_invalid() {
    run_classifier_scenario(
        "spawn /tmp/garbage: Exec format error (os error 8)",
        TransitionReason::ExecBinaryInvalid {
            path: "/tmp/garbage".to_owned(),
            kind: "exec_format_error".to_owned(),
        },
    )
    .await;
}

#[tokio::test]
async fn s_cp_05_classifier_cgroup_failure_to_cgroup_setup_failed() {
    run_classifier_scenario(
        "cgroup setup failed: place_pid: write cgroup.procs: Permission denied",
        TransitionReason::CgroupSetupFailed {
            kind: "place_pid".to_owned(),
            source: "write cgroup.procs: Permission denied".to_owned(),
        },
    )
    .await;
}

#[tokio::test]
async fn s_cp_05_classifier_unclassified_falls_through_to_driver_internal_error() {
    let raw = "totally unclassifiable driver text from a future driver";
    run_classifier_scenario(raw, TransitionReason::DriverInternalError { detail: raw.to_owned() })
        .await;
}

// ---------------------------------------------------------------------------
// Sanity — a Stop action also broadcasts a LifecycleEvent (architectural
// guarantee per architecture.md §10: every obs.write is paired with a
// bus.send).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stop_action_also_broadcasts_lifecycle_event() {
    let (tx, mut rx) = broadcast::channel::<LifecycleEvent>(16);

    let driver: Arc<dyn Driver> = Arc::new(AlwaysOkDriver);
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(fresh_node(), 0));

    // Seed a prior alloc row so `find_prior_alloc_row` finds it on the
    // Stop arm.
    let alloc_id = AllocationId::new("alloc-stop").expect("alloc id");
    let job_id = JobId::new("payments").expect("job id");
    let node_id = fresh_node();
    let prior_row = overdrive_core::traits::observation_store::AllocStatusRow {
        alloc_id: alloc_id.clone(),
        job_id: job_id.clone(),
        node_id: node_id.clone(),
        state: AllocState::Running,
        updated_at: overdrive_core::traits::observation_store::LogicalTimestamp {
            counter: 1,
            writer: node_id.clone(),
        },
        reason: Some(TransitionReason::Started),
        detail: None,
    };
    obs.write(ObservationRow::AllocStatus(prior_row)).await.expect("seed prior row");

    // Dispatch a Stop action — should write Terminated row AND emit broadcast.
    let action = Action::StopAllocation { alloc_id: alloc_id.clone() };
    let tick = make_tick(1);
    dispatch(vec![action], driver.as_ref(), obs.as_ref(), &tx, &tick)
        .await
        .expect("dispatch must succeed");

    let event = rx.try_recv().expect("broadcast event must arrive");
    assert_eq!(event.alloc_id, alloc_id);
    assert!(matches!(event.reason, TransitionReason::Stopped { .. }));
}

// ---------------------------------------------------------------------------
// Sanity — recording driver test fixture for ergonomic Mutex usage in
// the always-ok branch. (Anchors the unused-symbol lint.)
// ---------------------------------------------------------------------------
#[allow(dead_code)]
fn _suppress_unused_mutex_import() {
    let _: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
}
