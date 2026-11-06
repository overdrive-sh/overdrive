//! The startup stop gate under the guest-listeners mechanic (D14 item 3, P1;
//! S-54).
//!
//! Driven through hydration (`hydrate_actual_for_test` over a persisted
//! Service whose startup descriptor has the mechanic under test, so the
//! reconciler's own `spec_facts_for_service` supplies the gate's attempts
//! threshold) and `ServiceLifecycleReconciler::reconcile`. Oracle: whether the
//! returned actions carry the startup-probe-failed ending.
//!
//! Expected, per the design: for `Tcp` and `Http` the stop requires the latest
//! startup `Fail` row's streak `>= max_attempts` (as before), the deadline
//! elapsed and no `Pass`; for `GuestListeners` it holds at streak 1 once the
//! deadline has elapsed with no `Pass`, never before the deadline.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::unused_async,
    reason = "test fixtures assert their own preconditions"
)]

use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::AppState;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, hydrate_actual_for_test, hydrate_desired_for_test,
};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, IntentKey, ResourcesInput, Service, WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq, ProbeStatus, SameStatusRuns,
};
use overdrive_core::reconcilers::{Reconciler, TargetResource, TickContext};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore, TransitionSource,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::service_lifecycle::{
    ServiceLifecycleReconciler, ServiceLifecycleState, ServiceLifecycleView,
};
use overdrive_reconcilers::{AnyReconciler, AnyState};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use proptest::prelude::*;
use tempfile::TempDir;

const MAX_ATTEMPTS: u32 = 3;
const INTERVAL_SECS: u32 = 20;
/// `interval x max_attempts`.
const DEADLINE_SECS: u64 = 60;
const STARTED_SECS: u64 = 1_700_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mechanic {
    Tcp,
    Http,
    GuestListeners,
}

impl Mechanic {
    fn descriptor(self) -> ProbeDescriptor {
        let mechanic = match self {
            Self::Tcp => ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: 8080 },
            Self::Http => {
                ProbeMechanic::Http { path: "/healthz".to_owned(), port: 8080, host: None }
            }
            Self::GuestListeners => ProbeMechanic::GuestListeners,
        };
        ProbeDescriptor {
            idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            mechanic,
            timeout_seconds: 5,
            interval_seconds: INTERVAL_SECS,
            max_attempts: MAX_ATTEMPTS,
            failure_threshold: None,
            success_threshold: None,
            inferred: false,
        }
    }

    /// The latest startup row's streak at which the gate holds.
    const fn attempts_required(self) -> u32 {
        match self {
            Self::Tcp | Self::Http => MAX_ATTEMPTS,
            Self::GuestListeners => 1,
        }
    }
}

/// The latest startup row.
#[derive(Debug, Clone, Copy)]
enum Latest {
    Pass,
    /// A `Fail` row whose `same_status_runs` is this streak.
    Fail(u32),
}

fn node(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid NodeId")
}

async fn build_app_state(tmp: &TempDir, obs: Arc<dyn ObservationStore>) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        node("writer-1"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
        crate::acceptance::serve_ports::worker(),
        crate::acceptance::serve_ports::owner(),
        crate::acceptance::serve_ports::exec_gate(),
        crate::acceptance::serve_ports::pool(),
    )
}

fn service_with(wid: &WorkloadId, descriptor: ProbeDescriptor) -> Service {
    let input = ServiceSpecInput {
        id: wid.as_str().to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Vm(overdrive_core::aggregate::VmInput {
            command: "/bin/serve".to_owned(),
            args: vec![],
            kernel: "/kernel".to_owned(),
            rootfs: "/rootfs".to_owned(),
        }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    };
    let mut svc = Service::from_submit(input).expect("valid service spec");
    // The mechanic under test is placed directly: `GuestListeners` cannot be
    // written in TOML (D14 item 1), so the submit surface cannot carry it.
    svc.startup_probes = vec![descriptor];
    svc
}

/// Whether the reconciler ends the allocation for a failed startup, given the
/// service's mechanic, the latest startup row and the elapsed time since start.
async fn stop_emitted(mechanic: Mechanic, latest: Latest, elapsed_secs: u64) -> bool {
    let tmp = TempDir::new().expect("tmpdir");
    let obs =
        Arc::new(SimObservationStore::single_peer(node("local"), 0)) as Arc<dyn ObservationStore>;
    let state = build_app_state(&tmp, Arc::clone(&obs)).await;

    let wid = WorkloadId::new("startup-gate").expect("workload id");
    let svc = service_with(&wid, mechanic.descriptor());
    let archived = WorkloadIntent::Service(svc).archive_for_store().expect("rkyv archive");
    state
        .store
        .put(IntentKey::for_workload(&wid).as_bytes(), archived.as_ref())
        .await
        .expect("put");
    state
        .store
        .put(
            IntentKey::for_workload_kind(&wid).as_bytes(),
            &[WorkloadKind::Service.discriminator_byte()],
        )
        .await
        .expect("put kind");

    let aid = AllocationId::new("startup-gate-0").expect("alloc id");
    let started = UnixInstant::from_unix_duration(Duration::from_secs(STARTED_SECS));
    state
        .obs
        .write_alloc_lifecycle(
            AllocStatusRow {
                alloc_id: aid.clone(),
                workload_id: wid.clone(),
                node_id: node("local"),
                state: AllocState::Running,
                updated_at: LogicalTimestamp { counter: 1, writer: node("writer-1") },
                reason: None,
                detail: None,
                terminal: None,
                stderr_tail: None,
                kind: WorkloadKind::Service,
                listeners: Vec::new(),
                started_at: Some(started),
                // A Running Service row carries its workload address (ADR-0276).
                workload_addr: Some(std::net::Ipv4Addr::from(
                    u32::from(overdrive_core::guest_transport::GUEST_NODE_PREFIX.network()) + 2,
                )),
                last_terminated: None,
                restart_count: 0,
            },
            TransitionSource::Reconciler,
        )
        .await
        .expect("write alloc row");

    let (status, streak) = match latest {
        Latest::Pass => (ProbeStatus::Pass, 1),
        Latest::Fail(streak) => {
            (ProbeStatus::Fail { last_fail_reason: "not ready".to_owned() }, streak)
        }
    };
    obs.write_probe_result(ProbeResultRow {
        alloc_id: aid,
        probe_idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        status,
        last_observed_at_unix_ms: (STARTED_SECS + elapsed_secs) * 1000,
        inferred: false,
        run_seq: ProbeRunSeq::new(u64::from(streak)).expect("non-zero"),
        same_status_runs: SameStatusRuns::new(streak).expect("non-zero"),
    })
    .await
    .expect("write probe row");

    let target = TargetResource::new(&format!("workload/{wid}")).expect("target");
    let reconciler = AnyReconciler::ServiceLifecycle(ServiceLifecycleReconciler::new());
    let desired = hydrate_desired_for_test(&reconciler, &target, &state).await.expect("desired");
    let actual = hydrate_actual_for_test(&reconciler, &target, &state).await.expect("actual");
    let (AnyState::ServiceLifecycle(desired), AnyState::ServiceLifecycle(actual)) =
        (desired, actual)
    else {
        panic!("expected ServiceLifecycle states");
    };
    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(STARTED_SECS + elapsed_secs)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };
    let (actions, _) = ServiceLifecycleReconciler::new().reconcile(
        &desired as &ServiceLifecycleState,
        &actual,
        &ServiceLifecycleView::default(),
        &tick,
    );
    actions.iter().any(|a| format!("{a:?}").contains("StartupProbeFailed"))
}

const fn expected_stop(mechanic: Mechanic, latest: Latest, elapsed_secs: u64) -> bool {
    match latest {
        Latest::Pass => false,
        Latest::Fail(streak) => {
            streak >= mechanic.attempts_required() && elapsed_secs >= DEADLINE_SECS
        }
    }
}

fn arb_mechanic() -> impl Strategy<Value = Mechanic> {
    prop_oneof![Just(Mechanic::Tcp), Just(Mechanic::Http), Just(Mechanic::GuestListeners)]
}

fn arb_latest() -> impl Strategy<Value = Latest> {
    prop_oneof![
        Just(Latest::Pass),
        Just(Latest::Fail(1)),
        Just(Latest::Fail(2)),
        Just(Latest::Fail(MAX_ATTEMPTS)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// S-54 (D14 item 3, P1): mechanic x latest startup row x elapsed. Tcp and
    /// Http stop only at streak >= max_attempts; GuestListeners stops at streak
    /// 1 once the deadline has elapsed, never before it; a `Pass` never stops.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER: D14 P1 startup_attempts_required stop gate for GuestListeners"]
    fn the_startup_stop_gate_takes_its_attempts_threshold_from_the_mechanic(
        mechanic in arb_mechanic(),
        latest in arb_latest(),
        before_deadline in any::<bool>(),
    ) {
        let elapsed = if before_deadline { DEADLINE_SECS - 1 } else { DEADLINE_SECS };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let stopped = runtime.block_on(stop_emitted(mechanic, latest, elapsed));
        prop_assert_eq!(
            stopped,
            expected_stop(mechanic, latest, elapsed),
            "{:?} {:?} elapsed {}s", mechanic, latest, elapsed
        );
    }
}

/// S-54 golden cases, today's Tcp/Http behaviour (streak 2 of 3 does not stop;
/// streak 3 at the deadline does) and the new GuestListeners behaviour (streak 1
/// at the deadline stops). The Tcp/Http cases hold on HEAD.
#[tokio::test]
async fn dial_mechanics_stop_only_at_max_attempts() {
    for mechanic in [Mechanic::Tcp, Mechanic::Http] {
        assert!(
            !stop_emitted(mechanic, Latest::Fail(1), DEADLINE_SECS).await,
            "{mechanic:?} streak 1"
        );
        assert!(
            !stop_emitted(mechanic, Latest::Fail(2), DEADLINE_SECS).await,
            "{mechanic:?} streak 2"
        );
        assert!(
            !stop_emitted(mechanic, Latest::Fail(MAX_ATTEMPTS), DEADLINE_SECS - 1).await,
            "{mechanic:?} before the deadline"
        );
        assert!(
            stop_emitted(mechanic, Latest::Fail(MAX_ATTEMPTS), DEADLINE_SECS).await,
            "{mechanic:?} at max_attempts and the deadline"
        );
    }
}

#[tokio::test]
#[ignore = "pending DELIVER: D14 P1 startup_attempts_required stop gate for GuestListeners"]
async fn guest_listeners_stop_at_streak_one_once_the_deadline_elapsed() {
    assert!(!stop_emitted(Mechanic::GuestListeners, Latest::Fail(1), DEADLINE_SECS - 1).await);
    assert!(stop_emitted(Mechanic::GuestListeners, Latest::Fail(1), DEADLINE_SECS).await);
    assert!(!stop_emitted(Mechanic::GuestListeners, Latest::Pass, DEADLINE_SECS).await);
}
