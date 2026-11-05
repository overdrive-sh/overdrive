//! SVID issuance failure occurrence, workload half and the shared failure
//! values (R-46, R-50, R-51; ADR-0275; O-SIF-W1, W4).
//!
//! A failed workload SVID issuance is an operator-visible occurrence: the
//! `IssueSvid` executor records it through the atomic
//! `ObservationStore::record_workload_svid_issue_failure` after
//! `issue_and_audit` returned `Err`, and `GET /v1/allocs` reads it back into the
//! allocation row (`svid_issue_failures`, `last_svid_issue_failure`).
//!
//! # Driving ports
//!
//! `run_convergence_tick` for the production `svid-lifecycle` reconciler (the
//! reconciler emits `IssueSvid` for a Running allocation, including the
//! `rotate-svid` reissue of a held leaf nearing expiry) and the `alloc_status`
//! handler of `GET /v1/allocs`. Faults enter only at driven ports: a CA that
//! cannot sign (`ToggleCa`), and a delegating observation store whose faults
//! are scoped to the `issued_certificates` audit write, the failure-record
//! operation or the failure-row read (`IssueFaultStore`). Causes are driven
//! through the reconciler, never seeded as rows.
//!
//! The first two tests are pure and active; the rest carry complete bodies and
//! are ignored until the executor records and the handler stamps.

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    reason = "CONTRACT_SHAPE is required scenario metadata; fixture preconditions panic with their cause"
)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::extract::{Query, State};
use overdrive_control_plane::api::{
    AllocStatusResponse, AllocStatusRowBody, SvidIssueFailureBody, SvidIssueFailureCauseBody,
};
use overdrive_control_plane::ca_issuance::CaIssuanceError;
use overdrive_control_plane::handlers::{AllocStatusQuery, alloc_status};
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{
    ConvergenceError, ReconcilerRuntime, run_convergence_tick,
};
use overdrive_control_plane::{AppState, noop_heartbeat, svid_lifecycle};
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{
    DriverInput, IntentKey, Job, JobSpecInput, ResourcesInput, VmInput, WorkloadIntent,
    WorkloadKind,
};
use overdrive_core::ca::issue_failure::{SvidIssueFailure, SvidIssueFailureCause};
use overdrive_core::eval_broker::{Evaluation, EvaluationEligibility};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Reconciler, ReconcilerName, TargetResource};
use overdrive_core::traits::ca::{
    Ca, CaError, IntermediateHandle, RootCaHandle, SvidMaterial, SvidRequest, TrustBundle,
};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, ObservationStoreError,
    TransitionSource,
};
use overdrive_reconcilers::svid_lifecycle::{NEAR_EXPIRY_THRESHOLD_SECS, SvidLifecycle};
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

use crate::acceptance::issue_fault_store::{ISSUE_FAULT_MARKER, IssueFaultStore};

const NODE_NAME: &str = "host-0";
const WORKLOAD_NAME: &str = "payments";

fn nid(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn wid(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}

// ---------------------------------------------------------------------------
// Pure, active: the failure class and the wire body
// ---------------------------------------------------------------------------

/// The failure class of an issuance error is its variant class only: a signing
/// failure is `Ca`, an audit-write failure is `Audit`, and the class carries no
/// source text.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_failure_class_of_an_issuance_error_is_its_variant_class() {
    let signing = CaIssuanceError::Ca { source: CaError::signing_failed("signing backend gone") };
    let audit = CaIssuanceError::Audit {
        source: ObservationStoreError::Io(std::io::Error::other("disk full")),
    };
    assert_eq!(signing.failure_cause(), SvidIssueFailureCause::Ca);
    assert_eq!(audit.failure_cause(), SvidIssueFailureCause::Audit);
}

/// The wire body projects the failure as the lowercase cause label and the
/// instant, and carries nothing else.
///
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_issue_failure_wire_body_is_the_cause_label_and_the_instant_only() {
    for (cause, label) in
        [(SvidIssueFailureCause::Ca, "ca"), (SvidIssueFailureCause::Audit, "audit")]
    {
        let failure = SvidIssueFailure {
            cause,
            at: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)),
        };
        let body = SvidIssueFailureBody::from(failure);
        assert_eq!(body.at, failure.at);

        let json = serde_json::to_value(&body).expect("serialize the body");
        let object = json.as_object().expect("the body is an object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["at", "cause"], "the body carries the cause and the instant only");
        assert_eq!(object["cause"], label);
    }
}

// ---------------------------------------------------------------------------
// Composition: the production reconciler, a faulty CA and a faulty store
// ---------------------------------------------------------------------------

/// A CA whose leaf signing fails while `fail_signing` is set; everything else
/// delegates to a real `SimCa`. A driven-port double, not production surface.
struct ToggleCa {
    delegate: SimCa,
    fail_signing: Arc<AtomicBool>,
}

impl Ca for ToggleCa {
    fn root(&self) -> Result<RootCaHandle, CaError> {
        self.delegate.root()
    }

    fn issue_intermediate(&self, node: &NodeId) -> Result<IntermediateHandle, CaError> {
        self.delegate.issue_intermediate(node)
    }

    fn issue_svid(&self, req: &SvidRequest) -> Result<SvidMaterial, CaError> {
        if self.fail_signing.load(Ordering::SeqCst) {
            return Err(CaError::signing_failed("injected signing failure"));
        }
        self.delegate.issue_svid(req)
    }

    fn trust_bundle(&self) -> Result<TrustBundle, CaError> {
        self.delegate.trust_bundle()
    }
}

struct World {
    _tmp: TempDir,
    state: AppState,
    clock: Arc<SimClock>,
    faults: Arc<IssueFaultStore>,
    signing_fails: Arc<AtomicBool>,
    alloc: AllocationId,
    tick_n: u64,
}

impl World {
    /// A node with one Running allocation of the Job `payments`, composed over
    /// the production `svid-lifecycle` reconciler.
    async fn running_job() -> Self {
        let tmp = TempDir::new().expect("tmpdir");
        let clock = Arc::new(SimClock::new());
        let sim: Arc<dyn ObservationStore> =
            Arc::new(SimObservationStore::single_peer(nid(NODE_NAME), 0x51F));
        let faults = IssueFaultStore::over(sim);
        let signing_fails = Arc::new(AtomicBool::new(false));

        let mut runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
        runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
        runtime.register(svid_lifecycle()).await.expect("register svid-lifecycle");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
        let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
        let allocator = overdrive_control_plane::test_default_allocator(
            Arc::clone(&store) as Arc<dyn IntentStore>
        );
        let ca: Arc<dyn Ca> = Arc::new(ToggleCa {
            delegate: SimCa::new(Arc::new(SimEntropy::new(0x5F1))),
            fail_signing: Arc::clone(&signing_fails),
        });
        let obs: Arc<dyn ObservationStore> = faults.clone();
        let state = AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            driver,
            clock.clone(),
            Arc::new(SimDataplane::new()),
            ca,
            Arc::new(IdentityMgr::new(None)),
            nid(NODE_NAME),
            allocator,
            overdrive_control_plane::test_empty_listener_facts(),
            std::net::Ipv4Addr::LOCALHOST,
            crate::acceptance::serve_ports::worker(),
            crate::acceptance::serve_ports::owner(),
            crate::acceptance::serve_ports::exec_gate(),
            crate::acceptance::serve_ports::pool(),
        );

        let job = Job::from_submit(JobSpecInput {
            id: WORKLOAD_NAME.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 500, memory_bytes: 134_217_728 },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/true".to_owned(),
                args: vec![],
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
        })
        .expect("Job::from_submit");
        let archived =
            WorkloadIntent::Job(job.clone()).archive_for_store().expect("archive the Job");
        state
            .store
            .put(IntentKey::for_workload(&job.id).as_bytes(), archived.as_ref())
            .await
            .expect("persist the Job");
        state
            .store
            .put(
                IntentKey::for_workload_kind(&job.id).as_bytes(),
                &[WorkloadKind::Job.discriminator_byte()],
            )
            .await
            .expect("persist the Job kind");

        let alloc = aid("payments-0");
        state
            .obs
            .write_alloc_lifecycle(
                AllocStatusRow {
                    alloc_id: alloc.clone(),
                    workload_id: wid(WORKLOAD_NAME),
                    node_id: nid(NODE_NAME),
                    state: AllocState::Running,
                    updated_at: LogicalTimestamp { counter: 1, writer: nid(NODE_NAME) },
                    reason: None,
                    detail: None,
                    terminal: None,
                    stderr_tail: None,
                    kind: WorkloadKind::Job,
                    listeners: Vec::new(),
                    started_at: None,
                    workload_addr: None,
                    last_terminated: None,
                    restart_count: 0,
                },
                TransitionSource::Reconciler,
            )
            .await
            .expect("write the Running allocation");

        Self { _tmp: tmp, state, clock, faults, signing_fails, alloc, tick_n: 0 }
    }

    /// Advance the injected clock past any backoff window, let the production
    /// `svid-lifecycle` reconciler tick once for the workload, and return the
    /// tick's outcome.
    async fn reconcile_after(&mut self, advance: Duration) -> Result<(), ConvergenceError> {
        self.clock.tick(advance);
        let name = ReconcilerName::new(<SvidLifecycle as Reconciler>::NAME).expect("name");
        let target =
            TargetResource::new(&format!("workload/{WORKLOAD_NAME}")).expect("valid target");
        let now = std::time::Instant::now();
        self.state.runtime.broker().submit(
            Evaluation { reconciler: name.clone(), target },
            now,
            EvaluationEligibility::Immediate,
        );
        let pending = self.state.runtime.broker().drain_pending(
            usize::MAX,
            &BTreeSet::new(),
            now,
            UnixInstant::from_clock(&*self.state.clock),
        );
        let mut outcome = Ok(());
        for (eval, _) in pending {
            if eval.reconciler != name {
                continue;
            }
            self.tick_n += 1;
            outcome = run_convergence_tick(
                &self.state,
                &eval.reconciler,
                &eval.target,
                now,
                self.tick_n,
                now + Duration::from_millis(100),
            )
            .await;
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
        }
        outcome
    }

    /// `GET /v1/allocs?job=payments`: the operator's snapshot.
    async fn snapshot(
        &self,
    ) -> Result<AllocStatusResponse, overdrive_control_plane::error::ControlPlaneError> {
        alloc_status(
            State(self.state.clone()),
            Query(AllocStatusQuery { job: Some(WORKLOAD_NAME.to_owned()) }),
        )
        .await
        .map(|json| json.0)
    }

    async fn row(&self) -> AllocStatusRowBody {
        self.snapshot()
            .await
            .expect("the allocations read succeeds")
            .rows
            .into_iter()
            .find(|row| row.alloc_id == self.alloc.to_string())
            .expect("the allocation row")
    }

    fn now(&self) -> UnixInstant {
        UnixInstant::from_clock(&*self.clock)
    }
}

/// An error and its `source` chain as one string.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut next = error.source();
    while let Some(cause) = next {
        text.push_str(" <- ");
        text.push_str(&cause.to_string());
        next = cause.source();
    }
    text
}

const BEYOND_ANY_BACKOFF: Duration = Duration::from_secs(600);

/// O-SIF-W1: repeated failed issuances for one allocation, driven by the
/// production reconciler (a first-issue `Ca` failure, a first-issue `Audit`
/// failure, then after a success a `rotate-svid` reissue that fails), give a
/// monotone count and the latest cause and instant through the store and
/// through `GET /v1/allocs`; a success never clears them; the failures leave no
/// audit row and no held material.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn repeated_failed_issuances_give_a_monotone_count_and_the_latest_cause_on_the_allocations_surface()
 {
    let mut world = World::running_job().await;
    let fresh = world.row().await;
    assert_eq!(fresh.svid_issue_failures, 0);
    assert_eq!(fresh.last_svid_issue_failure, None);

    // First issue, the CA cannot sign.
    world.signing_fails.store(true, Ordering::SeqCst);
    let first = world.reconcile_after(BEYOND_ANY_BACKOFF).await;
    assert!(matches!(first, Err(ConvergenceError::Shim(_))), "got {first:?}");
    let first_instant = world.now();
    let after_first = world.row().await;
    assert_eq!(after_first.svid_issue_failures, 1);
    assert_eq!(
        after_first.last_svid_issue_failure,
        Some(SvidIssueFailureBody { cause: SvidIssueFailureCauseBody::Ca, at: first_instant }),
    );

    // First issue again, the audit write fails (the CA signs).
    world.signing_fails.store(false, Ordering::SeqCst);
    world.faults.fail_issued_certificate_writes.store(true, Ordering::SeqCst);
    let second = world.reconcile_after(BEYOND_ANY_BACKOFF).await;
    assert!(matches!(second, Err(ConvergenceError::Shim(_))), "got {second:?}");
    let second_instant = world.now();
    let after_second = world.row().await;
    assert_eq!(after_second.svid_issue_failures, 2, "the count rose by exactly one");
    assert_eq!(
        after_second.last_svid_issue_failure,
        Some(SvidIssueFailureBody { cause: SvidIssueFailureCauseBody::Audit, at: second_instant }),
    );
    assert!(second_instant > first_instant);
    assert!(
        world.state.obs.issued_certificate_rows().await.expect("audit rows").is_empty(),
        "failed issuances leave no audit row",
    );
    assert!(world.state.identity.held_snapshot().is_empty(), "failed issuances hold nothing");

    // Both faults lifted: the issuance succeeds and the occurrence is retained.
    world.faults.fail_issued_certificate_writes.store(false, Ordering::SeqCst);
    world.reconcile_after(BEYOND_ANY_BACKOFF).await.expect("the issuance succeeds");
    assert_eq!(world.state.identity.held_snapshot().len(), 1, "the SVID is held");
    let after_success = world.row().await;
    assert_eq!(after_success.svid_issue_failures, 2, "a success never clears the count");
    assert_eq!(after_success.last_svid_issue_failure, after_second.last_svid_issue_failure);

    // The held leaf nears expiry and the `rotate-svid` reissue fails.
    world.signing_fails.store(true, Ordering::SeqCst);
    let rotate = world.reconcile_after(Duration::from_secs(NEAR_EXPIRY_THRESHOLD_SECS + 100)).await;
    assert!(matches!(rotate, Err(ConvergenceError::Shim(_))), "got {rotate:?}");
    let rotate_instant = world.now();
    let after_rotate = world.row().await;
    assert_eq!(after_rotate.svid_issue_failures, 3, "a reissue failure counts like a first issue");
    assert_eq!(
        after_rotate.last_svid_issue_failure,
        Some(SvidIssueFailureBody { cause: SvidIssueFailureCauseBody::Ca, at: rotate_instant }),
    );
    assert_eq!(
        world.state.identity.held_snapshot().len(),
        1,
        "the previous leaf stays held: a failed reissue unwinds nothing",
    );

    // The store agrees with the surface, and the surface carries no error text.
    let stored = world
        .state
        .obs
        .workload_svid_issue_status_row(&world.alloc)
        .await
        .expect("point read")
        .expect("a recorded row");
    assert_eq!(stored.issue_failures, 3);
    let rendered = serde_json::to_string(&world.snapshot().await.expect("snapshot")).expect("json");
    for forbidden in [ISSUE_FAULT_MARKER, "injected signing failure", "audit row"] {
        assert!(!rendered.contains(forbidden), "the surface leaked {forbidden:?}: {rendered}");
    }
}

/// O-SIF-W1 (unrecorded arm): a failing record write is surfaced by the tick
/// (the error it returns differs from the one returned when the same issuance
/// failure is recorded, because it also carries the recording failure) and not
/// counted, nothing is held, and the next failed attempt records (the count is
/// then one).
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_failing_record_write_is_surfaced_and_not_counted_and_the_next_failure_records() {
    let mut world = World::running_job().await;
    world.signing_fails.store(true, Ordering::SeqCst);

    world.faults.fail_failure_record.store(true, Ordering::SeqCst);
    let unrecorded = world.reconcile_after(BEYOND_ANY_BACKOFF).await;
    let unrecorded_error = unrecorded.expect_err("the failure is surfaced, not absorbed");
    assert!(matches!(unrecorded_error, ConvergenceError::Shim(_)), "got {unrecorded_error:?}");
    assert_eq!(world.row().await.svid_issue_failures, 0, "an unrecorded failure is not counted");
    assert!(world.state.identity.held_snapshot().is_empty(), "nothing is held");

    world.faults.fail_failure_record.store(false, Ordering::SeqCst);
    let recorded = world.reconcile_after(BEYOND_ANY_BACKOFF).await;
    let recorded_error = recorded.expect_err("the issuance still fails");
    assert!(matches!(recorded_error, ConvergenceError::Shim(_)), "got {recorded_error:?}");
    assert!(
        error_chain(&unrecorded_error).contains(ISSUE_FAULT_MARKER),
        "the unrecorded failure also carries the recording failure: {}",
        error_chain(&unrecorded_error),
    );
    assert!(
        !error_chain(&recorded_error).contains(ISSUE_FAULT_MARKER),
        "a recorded failure carries only the issuance failure: {}",
        error_chain(&recorded_error),
    );
    let row = world.row().await;
    assert_eq!(row.svid_issue_failures, 1, "the next failed attempt records");
    assert_eq!(
        row.last_svid_issue_failure,
        Some(SvidIssueFailureBody { cause: SvidIssueFailureCauseBody::Ca, at: world.now() }),
    );
}

/// O-SIF-W4: an observation read error on the failure row is the allocations
/// handler's error, never a defaulted zero; once the read recovers the recorded
/// count is served.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_failure_row_read_error_is_the_handlers_error_and_never_a_zero() {
    let mut world = World::running_job().await;
    world.signing_fails.store(true, Ordering::SeqCst);
    world.reconcile_after(BEYOND_ANY_BACKOFF).await.expect_err("the issuance fails");
    assert_eq!(world.row().await.svid_issue_failures, 1);

    world.faults.fail_issue_status_reads.store(true, Ordering::SeqCst);
    let failed = world.snapshot().await;
    assert!(failed.is_err(), "a failed read must not read as zero failures: {failed:?}");

    world.faults.fail_issue_status_reads.store(false, Ordering::SeqCst);
    assert_eq!(world.row().await.svid_issue_failures, 1, "the next healthy read serves the count");
}
