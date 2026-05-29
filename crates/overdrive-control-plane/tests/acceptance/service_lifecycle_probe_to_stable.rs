//! GAP-7 integration witness — the production `ProbeRunner` supervised
//! tick loop produces a `ProbeResultRow` whose `ProbeStatus` flows
//! through to a `Stable` verdict from the
//! `ServiceLifecycleReconciler` when the rest of the alloc state is
//! consistent with Running + Pass.
//!
//! This acceptance test closes the structural cascade GAP-1 + GAP-6
//! + GAP-7 jointly defend:
//!
//!  * GAP-1 — `hydrate_actual` projects `latest_startup_probe` from
//!    the LWW `ProbeResultRow` (already landed; see commit `d6ef5aa9`).
//!  * GAP-6 — `WorkloadIntent::Service` persists probe descriptors
//!    (already landed; see commit `8afebdde`).
//!  * GAP-7 — `ProbeRunner::start_alloc` actually spawns per-descriptor
//!    tick tasks (this commit).
//!
//! Without GAP-7, no `ProbeResultRow` is ever written in production,
//! so the chain
//!   probe runs → row written → hydrate projects Pass → reconciler
//!   emits `Stable`
//! is structurally dead at the second arrow. This AT pins the property
//! that, with GAP-7 closed, the chain is restored end-to-end.
//!
//! ## Scope and shape
//!
//! The AT bypasses the full reconciler-runtime + `AppState` composition
//! root to keep the witness focused on the GAP-7 → reconciler bridge.
//! It exercises:
//!
//!  1. `ProbeRunner::start_alloc(&alloc, vec![descriptor])` — the
//!     supervised tick task spawn.
//!  2. `SimClock::tick(interval)` — the deterministic time advance.
//!  3. `SimObservationStore::list_probe_results_for_alloc(&alloc)` —
//!     the LWW projection the production `hydrate_actual` consults.
//!  4. A synthetic `ServiceAllocFact` built from the projected row —
//!     mirrors what the production `hydrate_actual_for_test` emits
//!     when joined with the Running `AllocStatusRow` + the persisted
//!     `WorkloadIntent::Service`.
//!  5. `ServiceLifecycleReconciler::reconcile(&desired, &actual,
//!     &view, &tick)` — the pure-sync verdict.
//!
//! The integration property is "step 3's output is a faithful input
//! to step 4's fact construction." Step 4's projection mirrors
//! `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_hydrate.rs`'s
//! shape (which already pins the production hydrate_actual against
//! the same LWW projection) — the two ATs cross-witness via the
//! `latest_startup_probe` slot.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::observation::{ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::service_lifecycle::{
    ServiceAllocFact, ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_core::traits::prober::ProbeOutcome;
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};
use overdrive_worker::probe_runner::ProbeRunner;

fn alloc_id(s: &str) -> AllocationId {
    AllocationId::new(s).expect("alloc id parses")
}

fn descriptor_tcp_1s(host: &str, port: u16) -> ProbeDescriptor {
    ProbeDescriptor {
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: host.to_owned(), port },
        timeout_seconds: 5,
        interval_seconds: 1,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

async fn yield_for_task_poll() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// Build a `ServiceAllocFact` whose shape matches what the production
/// `hydrate_actual` arm emits when it joins a Running `AllocStatusRow`
/// with the LWW `ProbeResultRow` projection AND the persisted
/// `WorkloadIntent::Service` probe-descriptor metadata.
///
/// The slot mapping is:
///   - `alloc_id` ← `ProbeResultRow.alloc_id`
///   - `state` ← `AllocStatusRow.state` (assumed `Running` for the
///     Stable arm)
///   - `started_at` ← `AllocStatusRow.started_at`
///   - `exit_code` ← `None` for Running
///   - `latest_startup_probe` ← `ProbeResultRow.status` projected by
///     the LWW resolver (one row per `(alloc_id, probe_idx=0)`)
///   - `max_attempts` / `startup_deadline` ← `ProbeDescriptor` fields
///     (persisted on the Service intent post-GAP-6)
///   - `mechanic_summary` / `inferred` ← derived from `ProbeDescriptor`
///   - `startup_probes_empty` ← `false` (descriptor is present)
fn fact_from_row_and_intent(
    row: &overdrive_core::observation::ProbeResultRow,
    started_at_unix_ms: u64,
    descriptor: &ProbeDescriptor,
) -> ServiceAllocFact {
    let mechanic_summary = match &descriptor.mechanic {
        ProbeMechanic::Tcp { host, port } => format!("tcp {host}:{port}"),
        ProbeMechanic::Http { host, port, path } => {
            format!("http {}:{port}{path}", host.as_deref().unwrap_or(""))
        }
        ProbeMechanic::Exec { command } => format!("exec {command:?}"),
    };
    ServiceAllocFact {
        alloc_id: row.alloc_id.clone(),
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_millis(
            started_at_unix_ms,
        ))),
        exit_code: None,
        latest_startup_probe: Some(row.status.clone()),
        max_attempts: descriptor.max_attempts,
        startup_deadline: Duration::from_secs(
            u64::from(descriptor.max_attempts) * u64::from(descriptor.interval_seconds),
        ),
        mechanic_summary,
        inferred: descriptor.inferred,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
            .expect("valid spiffe"),
        backend_addr: std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 8080)),
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        restart_count: 0,
        restart_spec: overdrive_core::traits::driver::AllocationSpec {
            alloc: row.alloc_id.clone(),
            identity: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/x")
                .expect("valid spiffe"),
            command: "/bin/svc".to_string(),
            args: vec![],
            resources: overdrive_core::traits::driver::Resources {
                cpu_milli: 100,
                memory_bytes: 64 * 1024 * 1024,
            },
            probe_descriptors: vec![],
        },
    }
}

/// Tick context with synthetic wall-clock derived from the
/// `SimClock`'s unix_now() at the moment of the reconcile call.
fn tick_at_unix_ms(now_unix_ms: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_millis(now_unix_ms)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

/// End-to-end GAP-7 integration witness — the production
/// `ProbeRunner::start_alloc` supervised tick loop writes a
/// `ProbeStatus::Pass` row that, projected into a `ServiceAllocFact`,
/// drives the `ServiceLifecycleReconciler` to emit
/// `Action::FinalizeFailed { terminal: Some(Stable { .. }) }`.
///
/// The structural witness is the round-trip: ProbeRunner writes the
/// row, then the reconciler's Stable arm consumes it. Pre-patch the
/// row was never written (GAP-7); pre-patch the Service intent did
/// not persist descriptors (GAP-6); pre-patch the hydrate projection
/// did not consult the probe row (GAP-1). All three gaps must be
/// closed for this AT to GREEN.
#[tokio::test]
async fn given_probe_runner_writes_pass_row_when_service_lifecycle_reconciles_then_emits_stable() {
    // -----------------------------------------------------------------
    // Setup — production ProbeRunner with Sim adapters.
    // -----------------------------------------------------------------
    let tcp = Arc::new(SimTcpProber::new());
    tcp.enqueue_outcome(ProbeOutcome::Pass);
    let http = Arc::new(SimHttpProber::new());
    let exec = Arc::new(SimExecProber::new());
    let clock = Arc::new(SimClock::default());
    let obs = Arc::new(SimObservationStore::single_peer(
        NodeId::new("probe-to-stable-test").expect("valid NodeId"),
        0,
    ));

    let runner = ProbeRunner::new(
        tcp,
        http,
        exec,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
    );

    let alloc = alloc_id("alloc-probe-to-stable");
    let descriptor = descriptor_tcp_1s("127.0.0.1", 8080);

    // Capture the alloc's started-at timestamp BEFORE the supervisor
    // ticks — the Stable verdict computes settled_in_ms as
    // `tick.now_unix - started_at`, so the test's tick_at_unix_ms
    // must dominate started_at_unix_ms.
    let started_at_unix_ms =
        u64::try_from(clock.unix_now().as_millis()).expect("unix_now fits in u64");

    // -----------------------------------------------------------------
    // ACT 1 — start the supervised tick loop, advance the clock past
    // one interval, wait for the row to land in the obs store.
    // -----------------------------------------------------------------
    let _token = runner.start_alloc(&alloc, vec![descriptor.clone()]);
    yield_for_task_poll().await;
    clock.tick(Duration::from_secs(1));

    let mut row_opt = None;
    for _ in 0..64 {
        let rows =
            obs.list_probe_results_for_alloc(&alloc).await.expect("list_probe_results_for_alloc");
        if let Some(row) = rows.into_iter().next() {
            row_opt = Some(row);
            break;
        }
        tokio::task::yield_now().await;
    }
    let row = row_opt.expect(
        "ProbeRunner::start_alloc must produce a ProbeResultRow after one tick — \
         pre-patch GAP-7 left this side of the chain dead",
    );
    assert_eq!(
        row.status,
        ProbeStatus::Pass,
        "row.status carries the Sim adapter's Pass outcome verbatim",
    );

    // Cancel the supervisor — we have the witness row; further
    // ticks would just LWW-overwrite with the same status (queue
    // was drained; next tick would pull SimTcpProber's empty-queue
    // default Pass). Keep the test deterministic.
    runner.stop_alloc(&alloc);

    // -----------------------------------------------------------------
    // ACT 2 — project the row into a ServiceAllocFact (mirroring what
    // the production hydrate_actual emits) and drive the reconciler.
    // -----------------------------------------------------------------
    let fact = fact_from_row_and_intent(&row, started_at_unix_ms, &descriptor);
    let actual = {
        let mut allocs = BTreeMap::new();
        allocs.insert(fact.alloc_id.clone(), fact.clone());
        ServiceLifecycleState { allocs, service_dataplane: None }
    };
    let desired = actual.clone();
    let view = ServiceLifecycleView::default();

    // Reconcile tick — pin the wall-clock at started_at + 500ms so
    // `settled_in_ms` lands deterministically at 500.
    let tick = tick_at_unix_ms(started_at_unix_ms + 500);

    let reconciler = ServiceLifecycleReconciler::new();
    let (actions, next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);

    // -----------------------------------------------------------------
    // ASSERT — the reconciler emits exactly one Stable action.
    // -----------------------------------------------------------------
    assert_eq!(
        actions.len(),
        1,
        "exactly one Stable action expected after GAP-1 + GAP-6 + GAP-7 close jointly; \
         got: {actions:?}",
    );
    match &actions[0] {
        Action::FinalizeFailed {
            alloc_id: emitted,
            terminal: Some(TerminalCondition::Stable { settled_in_ms, witness }),
        } => {
            assert_eq!(emitted, &fact.alloc_id);
            assert_eq!(*settled_in_ms, 500, "settled_in_ms = tick.now_unix - started_at");
            assert_eq!(witness.probe_idx, 0);
            assert_eq!(witness.role, "startup");
            assert_eq!(witness.mechanic_summary, "tcp 127.0.0.1:8080");
            assert!(!witness.inferred, "operator-declared descriptor → witness.inferred = false");
        }
        other => panic!("expected Stable action, got {other:?}"),
    }
    assert!(
        next_view.stable_announced.contains(&fact.alloc_id),
        "next-View must include alloc in stable_announced (dedup guard)",
    );
}
