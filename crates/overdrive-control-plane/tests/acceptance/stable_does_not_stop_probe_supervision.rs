//! ADR-0080 § D4 / § D7 items 4 + 5 — `Stable` retires ONLY the
//! startup role; a genuine terminal still tears the whole supervisor
//! down.
//!
//! # The defect this pins
//!
//! `TerminalCondition::Stable` is NON-TERMINAL (ADR-0055 :1, :255-258,
//! :267-276): the alloc stays `Running` and keeps serving, and
//! readiness / liveness are continuous post-Stable per `ProbeRole`'s
//! own contract (`probe_result_row.rs` — "startup: bounded by
//! `startup_deadline`; readiness/liveness: continuous post-Stable").
//!
//! The action shim nevertheless routed BOTH a Stable and a genuine
//! terminal through `Driver::on_alloc_terminal`, whose production
//! implementation calls `ProbeRunner::stop_alloc` — removing the
//! supervisor and cancelling EVERY task under it. Readiness and
//! liveness supervision therefore ended at the exact moment ADR-0055
//! says it begins, which is one of the two independent mechanisms that
//! made `Backend.healthy` a constant function of intent rather than a
//! function of observation.
//!
//! Mechanism 2 fires for EVERY Service, not only the ones with an
//! empty startup array, so both Stable emission branches
//! (`service_lifecycle.rs:568` empty-startup opt-out and `:593`
//! startup-probe-Pass) are exercised below — they reach the same shim
//! hook.
//!
//! # PORT-TO-PORT litmus
//!
//! Enters through TWO production driving ports composed together —
//! `compose_production_driver` (the sole composition site `run_server`
//! calls) and `action_shim::dispatch` (the runtime's action driving
//! port) — and asserts on the `ProbeRunner`'s own inspection surface
//! (`active_alloc_count` / `is_role_live`). No test-only wiring stands
//! in for a production call site: the shim decides which hook to fire,
//! the production `ExecDriver` decides what that hook does, and the
//! production `ProbeRunner` owns the supervisor being asserted on.
//!
//! `is_role_live` reports TOKEN liveness. That is a valid proxy for
//! task liveness only because `supervised_probe_loop` has exactly one
//! exit — its `child_token.cancelled()` arm — so a live token implies a
//! live task. `SimClock` parks each probe task on its interval
//! deadline (the steady state of a probe between ticks) and never
//! auto-advances, so no task can exit for any other reason during the
//! assertions below.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::action_shim::dispatch;
use overdrive_control_plane::compose_production_driver;
use overdrive_control_plane::veth_provisioner::NetSlotAllocator;
use overdrive_core::SpiffeId;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{AllocationSpec, Driver, Resources};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_core::transition_reason::{ProbeWitness, TerminalCondition, TransitionReason};
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::SimCgroupFs;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::probe_runner::ProbeRunner;
use tempfile::TempDir;

/// A TCP descriptor at the given per-role array position (ADR-0080
/// § D1 — `idx` is per-role, parser-assigned).
fn tcp_descriptor(role: ProbeRole, idx: u32, port: u16) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(idx),
        role,
        mechanic: ProbeMechanic::Tcp { host: "127.0.0.1".to_owned(), port },
        timeout_seconds: 1,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: if role == ProbeRole::Liveness { Some(3) } else { None },
        success_threshold: if role == ProbeRole::Readiness { Some(1) } else { None },
        inferred: false,
    }
}

fn spec_with(alloc: &AllocationId, probe_descriptors: Vec<ProbeDescriptor>) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::from_str("spiffe://overdrive.local/test/svc").expect("valid SpiffeId"),
        driver: overdrive_core::traits::driver::DriverPayload::Exec(
            overdrive_core::traits::driver::ExecPayload {
                command: "/bin/true".to_owned(),
                args: vec![],
            },
        ),
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors,
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    }
}

/// The `Stable` witness the `ServiceLifecycleReconciler` emits from the
/// **empty-startup opt-out** branch (`service_lifecycle.rs:568`).
fn stable_terminal_empty_startup() -> TerminalCondition {
    TerminalCondition::Stable {
        settled_in_ms: 0,
        witness: ProbeWitness {
            probe_idx: 0,
            role: "startup".to_owned(),
            mechanic_summary: "none (opted out)".to_owned(),
            inferred: false,
        },
    }
}

/// The `Stable` witness the reconciler emits from the
/// **startup-probe-Pass** branch (`service_lifecycle.rs:593`). Both
/// branches reach the same shim hook, so both are exercised.
fn stable_terminal_startup_pass() -> TerminalCondition {
    TerminalCondition::Stable {
        settled_in_ms: 114,
        witness: ProbeWitness {
            probe_idx: 0,
            role: "startup".to_owned(),
            mechanic_summary: "tcp 127.0.0.1:8080".to_owned(),
            inferred: true,
        },
    }
}

/// Everything the shim needs, composed the way `run_server` composes
/// it. Returns the production driver, its wired `ProbeRunner`, and the
/// observation store the shim writes through.
struct Harness {
    driver: Arc<dyn Driver>,
    // Single-entry registry over `driver` (ADR-0083 §D1, GH #42) +
    // its per-boot routing index — constructed once in `harness()` and
    // reused across every `dispatch_one` call so the index persists
    // across a Start-then-terminal sequence within one test.
    drivers: Arc<overdrive_core::traits::driver::DriverRegistry>,
    alloc_drivers: overdrive_control_plane::action_shim::AllocDriverIndex,
    runner: Arc<ProbeRunner>,
    obs: Arc<dyn ObservationStore>,
    intent: Arc<dyn IntentStore>,
    _tmp: TempDir,
}

async fn harness() -> Harness {
    let tmp = TempDir::new().expect("tempdir");
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("stable-probe-test").expect("valid NodeId"),
        0,
    ));
    let intent: Arc<dyn IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent"));

    let (driver, runner) = compose_production_driver(
        // Empty prober queues → every attempt Passes. The probe tasks
        // park on `SimClock::sleep` between ticks and never reach an
        // adapter during these assertions.
        Arc::new(SimTcpProber::new()),
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        PathBuf::from("/tmp/overdrive-test-stable-probe"),
        // `SimClock::sleep` PARKS on a deadline and only resolves when
        // the harness calls `tick()`, which this test never does. Each
        // spawned probe task therefore sits in its `select!` with the
        // cancellation arm live — the steady state the observable
        // assumes.
        Arc::new(SimClock::new()),
        Arc::new(SimCgroupFs::new()),
        Arc::clone(&obs),
    )
    .await
    .expect("Earned-Trust gate passes with default Sim probers");

    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    Harness { driver, drivers, alloc_drivers, runner, obs, intent, _tmp: tmp }
}

/// Seed the `Running` prior row the `FinalizeFailed` arm reads.
/// `counter: 0` so the shim's write (counter `tick.tick + 1`) strictly
/// dominates under LWW.
async fn seed_running_row(obs: &dyn ObservationStore, alloc: &AllocationId, node: &NodeId) {
    let row = AllocStatusRow {
        alloc_id: alloc.clone(),
        workload_id: WorkloadId::new("stable-probe-svc").expect("valid workload id"),
        node_id: node.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 0, writer: node.clone() },
        reason: Some(TransitionReason::Started),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };
    obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("seed Running row");
}

/// Drive ONE action through the production `action_shim::dispatch`
/// against the harness's composed driver.
async fn dispatch_one(h: &Harness, action: Action) {
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
    let (lifecycle_tx, _lifecycle_rx) = tokio::sync::broadcast::channel(16);
    let writer_node = NodeId::new("writer-1").expect("NodeId");
    let allocator = Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(
        VipRange::default(),
        Arc::clone(&h.intent),
    )));
    let net_slot_allocator = NetSlotAllocator::new();
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());

    let now = Instant::now();
    let tick = TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    };

    dispatch(
        vec![action],
        h.drivers.as_ref(),
        &h.alloc_drivers,
        h.obs.as_ref(),
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
        &broker,
        None,
        // No mTLS worker — the genuine-terminal teardown seam is a
        // clean no-op, keeping this default-lane (no netns, no root).
        None,
        &net_slot_allocator,
    )
    .await
    .expect("dispatch must succeed");
}

/// ADR-0080 § D7 item 4, **startup-probe-Pass branch**
/// (`service_lifecycle.rs:593`).
///
/// A Service declaring one startup AND one readiness probe reaches
/// `Stable` via the startup-Pass branch. After the shim dispatches that
/// Stable:
///
/// * the supervisor is STILL registered (`active_alloc_count == 1`),
/// * the startup role is retired (startup probing is bounded by the
///   startup window and is complete at Stable), and
/// * the readiness role is STILL LIVE — readiness is continuous
///   post-Stable, and it is the observation `Backend.healthy` is a
///   function of.
///
/// Pre-fix this asserted `active_alloc_count == 0` and BOTH roles dead:
/// the shim called `on_alloc_terminal` unconditionally, so
/// `stop_alloc` removed the supervisor 114 ms into the alloc's life —
/// well inside the 2 s default readiness interval.
#[tokio::test]
async fn stable_from_startup_pass_retires_startup_only_and_keeps_readiness_supervised() {
    let h = harness().await;
    let alloc = AllocationId::new("stable-pass-alloc").expect("valid alloc id");
    let node = NodeId::new("node-001").expect("valid node id");

    h.driver.on_alloc_running(&spec_with(
        &alloc,
        vec![
            tcp_descriptor(ProbeRole::Startup, 0, 8080),
            tcp_descriptor(ProbeRole::Readiness, 0, 8080),
        ],
    ));
    assert_eq!(h.runner.active_alloc_count(), 1, "on_alloc_running registers the supervisor");
    assert!(h.runner.is_role_live(&alloc, ProbeRole::Startup), "startup supervised pre-Stable");
    assert!(h.runner.is_role_live(&alloc, ProbeRole::Readiness), "readiness supervised pre-Stable");

    seed_running_row(h.obs.as_ref(), &alloc, &node).await;
    dispatch_one(
        &h,
        Action::FinalizeFailed {
            alloc_id: alloc.clone(),
            terminal: Some(stable_terminal_startup_pass()),
        },
    )
    .await;

    assert_eq!(
        h.runner.active_alloc_count(),
        1,
        "Stable is NON-TERMINAL (ADR-0055): the supervisor must survive. A count of 0 means \
         the shim routed Stable through on_alloc_terminal -> stop_alloc, cancelling every \
         role and ending readiness/liveness supervision at the moment it should begin",
    );
    assert!(
        !h.runner.is_role_live(&alloc, ProbeRole::Startup),
        "startup probing is bounded by the startup window and is complete at Stable; leaving \
         it running would tick forever post-Stable (supervised_probe_loop is unbounded) and \
         pollute `latest` with post-Stable startup rows",
    );
    assert!(
        h.runner.is_role_live(&alloc, ProbeRole::Readiness),
        "readiness is CONTINUOUS post-Stable per ProbeRole's contract — this is the \
         supervision `Backend.healthy` is a function of",
    );
}

/// ADR-0080 § D7 item 4, **empty-startup opt-out branch**
/// (`service_lifecycle.rs:568`).
///
/// Per § D7 item 4 the fixture declares readiness only for this branch,
/// since D4 creates per-role tokens on first use and an undeclared role
/// has none. The assertion is that readiness survives the Stable.
///
/// This branch is where the executed population diff was taken: with
/// `startup_probes: vec![]` the alloc settled Stable at
/// `settled_in_ms: 114` with `all probe rows for alloc: Ok([])` and
/// `active_alloc_count = Some(0)`.
#[tokio::test]
async fn stable_from_empty_startup_optout_keeps_readiness_supervised() {
    let h = harness().await;
    let alloc = AllocationId::new("stable-optout-alloc").expect("valid alloc id");
    let node = NodeId::new("node-001").expect("valid node id");

    h.driver
        .on_alloc_running(&spec_with(&alloc, vec![tcp_descriptor(ProbeRole::Readiness, 0, 8080)]));
    assert_eq!(h.runner.active_alloc_count(), 1, "on_alloc_running registers the supervisor");

    seed_running_row(h.obs.as_ref(), &alloc, &node).await;
    dispatch_one(
        &h,
        Action::FinalizeFailed {
            alloc_id: alloc.clone(),
            terminal: Some(stable_terminal_empty_startup()),
        },
    )
    .await;

    assert_eq!(
        h.runner.active_alloc_count(),
        1,
        "the empty-startup Stable branch reaches the same shim hook and must also leave the \
         supervisor alive",
    );
    assert!(
        h.runner.is_role_live(&alloc, ProbeRole::Readiness),
        "readiness supervision survives the empty-startup opt-out Stable",
    );
    assert!(
        !h.runner.is_role_live(&alloc, ProbeRole::Startup),
        "a role that never spawned a task is never live — an undeclared role has no token",
    );
}

/// ADR-0080 § D7 item 5 — **terminal still stops**, guarding against
/// D4 over-reaching.
///
/// A genuine terminal (`BackoffExhausted` / `Completed` / `Failed` —
/// i.e. `terminal: None` or any non-`Stable` condition) is a dead
/// alloc, not a live backend. The whole supervisor goes, across every
/// role.
#[tokio::test]
async fn genuine_terminal_still_stops_the_whole_supervisor() {
    let h = harness().await;
    let alloc = AllocationId::new("genuine-terminal-alloc").expect("valid alloc id");
    let node = NodeId::new("node-001").expect("valid node id");

    h.driver.on_alloc_running(&spec_with(
        &alloc,
        vec![
            tcp_descriptor(ProbeRole::Startup, 0, 8080),
            tcp_descriptor(ProbeRole::Readiness, 0, 8080),
            tcp_descriptor(ProbeRole::Liveness, 0, 8080),
        ],
    ));
    assert_eq!(h.runner.active_alloc_count(), 1, "supervisor registered");

    seed_running_row(h.obs.as_ref(), &alloc, &node).await;
    dispatch_one(&h, Action::FinalizeFailed { alloc_id: alloc.clone(), terminal: None }).await;

    assert_eq!(
        h.runner.active_alloc_count(),
        0,
        "a genuine terminal tears the whole supervisor down — D4 must not over-reach into \
         keeping dead allocs supervised",
    );
    for role in [ProbeRole::Startup, ProbeRole::Readiness, ProbeRole::Liveness] {
        assert!(
            !h.runner.is_role_live(&alloc, role),
            "{role:?} must not survive a genuine terminal",
        );
    }
}

/// ADR-0080 § D7 item 5 — the `StopAllocation` arm
/// (`action_shim/mod.rs:1748`) is UNCHANGED by D4: a stop is a genuine
/// terminal, so it keeps calling `on_alloc_terminal`.
#[tokio::test]
async fn stop_allocation_still_stops_the_whole_supervisor() {
    let h = harness().await;
    let alloc = AllocationId::new("stop-alloc").expect("valid alloc id");
    let node = NodeId::new("node-001").expect("valid node id");

    h.driver
        .on_alloc_running(&spec_with(&alloc, vec![tcp_descriptor(ProbeRole::Readiness, 0, 8080)]));
    assert_eq!(h.runner.active_alloc_count(), 1, "supervisor registered");

    seed_running_row(h.obs.as_ref(), &alloc, &node).await;
    dispatch_one(&h, Action::StopAllocation { alloc_id: alloc.clone(), terminal: None }).await;

    assert_eq!(
        h.runner.active_alloc_count(),
        0,
        "StopAllocation is a genuine terminal — the supervisor goes",
    );
    assert!(!h.runner.is_role_live(&alloc, ProbeRole::Readiness));
}
