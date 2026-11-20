//! Node-local persistent volume (GH #348, ADR-0317, ADR-0102): the launch path's boundary
//! scenarios 2 and 4, beyond the Quint run, observed through allocation rows and the restart
//! authority's view.
//!
//! The serialisation of a workload's evaluations (boundary 4) lives in the convergence loop
//! (ADR-0102: one workload lane, ownership ends after the awaited shim dispatch), not in
//! `VmDriver`. This module composes the same production `serve` composition as the conformance
//! driver (`quint_node_volume::boot_serve`: operator handlers, `WorkloadLifecycle`, the action
//! shim, `VmDriver` over `SimVmm`, the exit observer, `SimVolumeBacking` through
//! `SimVolumePoolProbe::composed`) and runs the workload's evaluations one after another, as
//! the lane runs them.
//!
//! A stop recorded during a launch does not wait behind it: the operator handler raises the
//! lane's supersession signal (ADR-0334) and the launch ends with `StartSuperseded`. Each
//! evaluation here is a spawned single tick; the order of the stop's evaluation after the
//! launch's is the test's, as in the driver. ADR-0102's own tests cover the broker's
//! active-target exclusivity.
//!
//! The second half of this module drives the production convergence loop itself
//! (`spawn_convergence_loop`, crate-private) over sim driven ports and a `SimDriver` start held
//! by `park_next_start`: the loop's shutdown arm raises every lane (boundary 4a (h)).
//!
//! Evidence tier: in-memory acceptance (layer 2). Pending: the production spec parser refuses
//! `[vm.volume]` until DELIVER's spec-surface step (prerequisite 2); each test fails there
//! today, and the steps after it are compile-checked only until the sim (prerequisite 5) and
//! the launch path (prerequisite 7) land.
//!
//! CONTRACT_SHAPE: bounded-change.

#![allow(
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::large_futures,
    clippy::future_not_send,
    clippy::panic,
    reason = "scenarios assert their own preconditions"
)]

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State as AxumState};
use axum::http::HeaderMap;
use overdrive_core::aggregate::WorkloadSpecInput;
use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
use overdrive_core::testing::volume::volume_size_gib;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::traits::volume_backing::{ClaimState, VolumeBacking};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_core::volume::VolumeUnavailableCause;
use overdrive_testing::placement;
use tokio::task::JoinHandle;

use crate::api::SubmitWorkloadRequest;
use crate::handlers::{stop_workload, submit_workload};
use crate::quint_node_volume::{
    DECLARATION, DECLARED_GIB, Node, PAST_BACKOFF, POLL_STEP, PassThroughProvisioner, Serve,
    boot_serve, declaration_wire, reconciler_name, target, workload_id,
};
use crate::reconciler_runtime::{
    ConvergenceError, run_convergence_tick_with_guest_network_provisioner_for_test,
};

placement!(host_pure);

/// The wait the declaration's refusal ends: every test starts by declaring the workload.
const PARSER_REFUSAL: &str =
    "a [vm.volume] workload must be accepted by the production spec parser";

struct Rig {
    node: Node,
    serve: Serve,
    ticks: Vec<JoinHandle<Result<(), ConvergenceError>>>,
}

impl Rig {
    fn new() -> Self {
        let node = Node::new();
        let (serve, _reconcilers) = boot_serve(&node, 1);
        Self { node, serve, ticks: Vec::new() }
    }

    fn pump(&self, rounds: u32) {
        for _ in 0..rounds {
            self.serve.rt.block_on(async { tokio::time::sleep(Duration::from_millis(1)).await });
        }
    }

    fn settle(&self, what: &str, cond: impl Fn(&Self) -> bool) {
        for _ in 0..10_000 {
            if cond(self) {
                return;
            }
            self.pump(1);
        }
        panic!("did not settle: {what}");
    }

    fn rows(&self) -> Vec<AllocStatusRow> {
        self.serve
            .rt
            .block_on(self.node.obs.alloc_status_rows())
            .expect("the observation store reads its rows")
    }

    fn states(&self) -> Vec<AllocState> {
        self.rows().into_iter().map(|row| row.state).collect()
    }

    /// The operator declares the workload through the production handler.
    fn declare(&self) {
        WorkloadSpecInput::from_toml_str(DECLARATION).expect(PARSER_REFUSAL);
        let request: SubmitWorkloadRequest =
            serde_json::from_value(declaration_wire(&self.node.root().display().to_string()))
                .expect("the wire spec");
        self.serve
            .rt
            .block_on(submit_workload(
                AxumState(self.serve.state.clone()),
                HeaderMap::new(),
                Json(request),
            ))
            .expect("submit_workload accepts the declaration");
    }

    fn stop(&self) {
        let _stopped = self
            .serve
            .rt
            .block_on(stop_workload(
                AxumState(self.serve.state.clone()),
                Path(workload_id().to_string()),
            ))
            .expect("stop_workload records the stop");
    }

    /// A predecessor VMM died with its in-flight I/O still draining: the volume exists and its
    /// kernel claim is held.
    fn hold_volume_claim(&self) {
        self.serve
            .rt
            .block_on(self.node.backing.ensure(&workload_id(), volume_size_gib(DECLARED_GIB)))
            .expect("the volume exists before the claim is held");
        self.node.backing.hold_claim(&workload_id());
    }

    fn claim(&self) -> ClaimState {
        self.serve
            .rt
            .block_on(self.node.backing.claim_state(&workload_id()))
            .expect("the volume is active")
    }

    /// One production convergence tick of the workload, spawned: its dispatch may park inside
    /// `VmDriver::start`. Returns the tick's index.
    fn spawn_tick(&mut self) -> usize {
        let clock = Arc::clone(&self.node.clock);
        clock.tick(PAST_BACKOFF);
        self.serve.tick_n += 1;
        let tick_n = self.serve.tick_n;
        let state = self.serve.state.clone();
        let name: ReconcilerName = reconciler_name();
        let target: TargetResource = target();
        self.ticks.push(self.serve.rt.spawn(async move {
            let now = clock.now();
            run_convergence_tick_with_guest_network_provisioner_for_test(
                &state,
                &name,
                &target,
                now,
                tick_n,
                now + Duration::from_secs(1),
                &PassThroughProvisioner,
            )
            .await
        }));
        self.ticks.len() - 1
    }

    fn finished(&self, tick: usize) -> bool {
        self.ticks[tick].is_finished()
    }

    /// Lets simulated time pass in `POLL_STEP`s until the tick ends (the claim bound is 30 s).
    fn pass_until_finished(&self, tick: usize, what: &str) {
        for _ in 0..8 {
            if self.finished(tick) {
                return;
            }
            self.node.clock.tick(POLL_STEP);
            self.pump(50);
        }
        assert!(self.finished(tick), "{what}");
    }

    fn reap(&mut self, tick: usize) {
        let handle = self.ticks.remove(tick);
        self.serve
            .rt
            .block_on(handle)
            .expect("the tick did not panic")
            .expect("the convergence tick returned Ok");
    }

    fn held_failure_row(&self) -> Option<AllocStatusRow> {
        self.failure_row_with(&VolumeUnavailableCause::Held)
    }

    fn failure_row_with(&self, cause: &VolumeUnavailableCause) -> Option<AllocStatusRow> {
        self.rows().into_iter().find(|row| {
            row.state == AllocState::Failed
                && matches!(
                    &row.reason,
                    Some(TransitionReason::VmVolumeUnavailable { cause: row_cause })
                        if row_cause == cause
                )
        })
    }
}

/// Boundary 4 (late success, ADR-0334): a stop recorded while a launch waits on a held claim
/// interrupts it. The launch ends at once with `StartSuperseded` (no simulated time passes, so
/// it does not wait for its 30 s claim bound): no row, no VMM, nothing charged to the restart
/// budget, the predecessor's claim untouched, and the volume kept. The stop's own evaluation,
/// which runs after it, finds nothing to stop and never reattaches the stale launch: its result
/// is never used.
///
/// Pending: the production spec parser refuses `[vm.volume]` until DELIVER's spec-surface step,
/// so the body stops at `declare`; the assertions after it are the contract the launch path must
/// meet once the parser and the lane table land.
#[test]
#[should_panic(expected = "a [vm.volume] workload must be accepted by the production spec parser")]
fn a_stop_interrupts_the_start_and_its_result_is_never_used() {
    let mut rig = Rig::new();
    rig.declare();
    rig.hold_volume_claim();

    let launch = rig.spawn_tick();
    rig.pump(50);
    assert!(
        !rig.finished(launch),
        "a launch on a held claim must keep waiting inside its evaluation"
    );
    let counts_before =
        rig.serve.state.runtime.view_for_workload_lifecycle(&target()).restart_counts;

    rig.stop();

    // No clock step: the launch ends because the stop interrupted it, not at its claim bound.
    rig.pump(200);
    assert!(
        rig.finished(launch),
        "a stop recorded during a launch's claim wait must end the launch at once with StartSuperseded"
    );
    rig.reap(launch);
    assert!(
        rig.rows().is_empty(),
        "a superseded launch writes no row, not even a Failed Held row; rows: {:?}",
        rig.rows()
    );
    assert!(rig.held_failure_row().is_none(), "the claim wait's bound was never reached");
    assert_eq!(
        rig.serve.state.runtime.view_for_workload_lifecycle(&target()).restart_counts,
        counts_before,
        "a superseded launch changes no restart-budget input"
    );
    assert!(rig.node.attached.lock().is_empty(), "no VMM is attached by the interrupted launch");
    assert_eq!(rig.claim(), ClaimState::Held, "the predecessor's claim is untouched");

    // The stop's evaluation finds the workload stopped: nothing starts, the result of the
    // interrupted launch is never used.
    let stop = rig.spawn_tick();
    rig.settle("the stop's evaluation returns", |rig| rig.finished(stop));
    rig.reap(stop);
    assert!(
        !rig.states().contains(&AllocState::Running) && rig.rows().is_empty(),
        "the stale launch must never run after the stop: rows {:?}",
        rig.rows()
    );
    assert!(rig.node.attached.lock().is_empty(), "the stale launch's result is never reattached");
}

/// Boundary 2 at row level: a claim held past the 30 s bound ends the launch as a `Failed` row
/// whose reason is `VmVolumeUnavailable { Held }`, nothing is attached, and the failure is
/// charged to the restart budget: the restart authority's view carries the charge once the
/// next evaluation issues the replacement.
#[test]
#[should_panic(expected = "a [vm.volume] workload must be accepted by the production spec parser")]
fn a_launch_held_past_its_bound_writes_the_typed_failed_row_and_charges_the_restart_budget() {
    let mut rig = Rig::new();
    rig.declare();
    rig.hold_volume_claim();

    let launch = rig.spawn_tick();
    rig.pass_until_finished(launch, "the launch must end at its claim bound");
    rig.reap(launch);

    let row = rig.held_failure_row().unwrap_or_else(|| {
        panic!("the held claim must fail the launch typed; rows: {:?}", rig.rows())
    });
    assert_eq!(row.state, AllocState::Failed);
    assert!(rig.node.attached.lock().is_empty(), "no VMM is attached by a failed launch");

    // The next evaluation issues the replacement and charges the failed attempt to it.
    let next = rig.spawn_tick();
    let target = target();
    rig.settle("the restart authority charges the held-claim failure", |rig| {
        rig.serve
            .state
            .runtime
            .view_for_workload_lifecycle(&target)
            .restart_counts
            .values()
            .any(|charged| *charged >= 1)
    });
    let _ = next;
}

/// Boundary 4a (h): node shutdown through the production convergence loop. The loop is driven
/// over sim driven ports (`SimClock` that never advances, `SimDriver`, the test shared-network
/// owner) with a `SimDriver` start held by `park_next_start`. Shutdown is the loop's own
/// `CancellationToken`, the one `ServerHandle::shutdown`, `FailedBootTasks::shutdown` and the
/// `run_server` early return all cancel.
///
/// Evidence tier: in-memory acceptance (layer 2). CONTRACT_SHAPE: bounded-change.
mod loop_shutdown {
    use overdrive_testing::placement;

    placement!(host_pure);

    use std::net::Ipv4Addr;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Json;
    use axum::extract::State as AxumState;
    use axum::http::HeaderMap;
    use overdrive_core::eval_broker::{Evaluation, EvaluationEligibility};
    use overdrive_core::id::NodeId;
    use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
    use overdrive_core::traits::clock::Clock;
    use overdrive_core::traits::driver::{Driver, DriverType};
    use overdrive_core::traits::intent_store::IntentStore;
    use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
    use overdrive_sim::adapters::ca::SimCa;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::dataplane::SimDataplane;
    use overdrive_sim::adapters::driver::{SimDriver, StartPark};
    use overdrive_sim::adapters::entropy::SimEntropy;
    use overdrive_sim::adapters::observation_store::SimObservationStore;
    use overdrive_store_local::LocalIntentStore;
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;

    use crate::api::SubmitWorkloadRequest;
    use crate::handlers::submit_workload;
    use crate::reconciler_runtime::ReconcilerRuntime;
    use crate::{AppState, spawn_convergence_loop};

    const CADENCE: Duration = Duration::from_millis(100);
    const POLLS: u32 = 1_000;

    struct LoopRig {
        state: AppState,
        clock: Arc<SimClock>,
        sim: Arc<SimDriver>,
        shutdown: CancellationToken,
        _wiring: overdrive_core::guest_network::GuestNetworkExecWiring,
        _tmp: tempfile::TempDir,
    }

    async fn pump(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    async fn settled(cond: impl Fn() -> bool) -> bool {
        for _ in 0..POLLS {
            if cond() {
                return true;
            }
            pump(1).await;
        }
        cond()
    }

    impl LoopRig {
        async fn new() -> Self {
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let mut runtime =
                ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
            runtime.register(crate::noop_heartbeat()).await.expect("register noop-heartbeat");
            runtime
                .register(crate::workload_lifecycle())
                .await
                .expect("register workload-lifecycle");
            let node = NodeId::new("shutdown-node").expect("node id");
            let store_path = tmp.path().join("intent.redb");
            let store = Arc::new(LocalIntentStore::open(&store_path).expect("open intent store"));
            let obs: Arc<dyn ObservationStore> =
                Arc::new(SimObservationStore::single_peer(node.clone(), 0));
            let clock = Arc::new(SimClock::new());
            let sim = Arc::new(SimDriver::with_clock(DriverType::Vm, clock.clone()));
            let allocator =
                crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
            let mtls_worker = crate::shared_network_test_ports::mtls_worker();
            mtls_worker.start_shared_owner().await.expect("the shared mTLS owner starts");
            let guest_owner = crate::shared_network_test_ports::shared_guest_network_owner();
            crate::guest_network::SharedGuestNetworkOwner::converge_shared(guest_owner.as_ref())
                .await
                .expect("the shared owner converges");
            let wiring = overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::clone(
                &clock,
            )
                as Arc<dyn Clock>);
            assert!(wiring.supervisor().open_after_boot());
            let state = AppState::new(
                store,
                store_path,
                obs,
                Arc::new(runtime),
                Arc::clone(&sim) as Arc<dyn Driver>,
                Arc::clone(&clock) as Arc<dyn Clock>,
                Arc::new(SimDataplane::new()),
                Arc::new(SimCa::new(Arc::new(SimEntropy::new(0)))),
                Arc::new(crate::identity_mgr::IdentityMgr::new(None)),
                node,
                allocator,
                crate::test_empty_listener_facts(),
                Ipv4Addr::LOCALHOST,
                mtls_worker,
                guest_owner,
                wiring.gate(),
                crate::shared_network_test_ports::guest_pool(),
            );
            Self {
                state,
                clock,
                sim,
                shutdown: CancellationToken::new(),
                _wiring: wiring,
                _tmp: tmp,
            }
        }

        fn spawn_loop(&self) -> JoinHandle<()> {
            spawn_convergence_loop(
                self.state.clone(),
                Arc::clone(&self.clock) as Arc<dyn Clock>,
                CADENCE,
                self.shutdown.clone(),
            )
        }

        /// The operator declares a Service through the real handler; it enqueues the workload's
        /// evaluation.
        async fn declare(&self, id: &str) {
            let wire = serde_json::json!({
                "spec": {
                    "kind": "service", "id": id, "replicas": 1,
                    "resources": {"cpu_milli": 100, "memory_bytes": 134_217_728u64},
                    "vm": {"command": "/sbin/init", "args": [], "kernel": "/kernel", "rootfs": "/rootfs"},
                    "listeners": [{"port": 8080, "protocol": "tcp"}],
                    "startup_probes": [], "readiness_probes": [], "liveness_probes": []
                }
            });
            let request: SubmitWorkloadRequest =
                serde_json::from_value(wire).expect("the wire spec");
            submit_workload(AxumState(self.state.clone()), HeaderMap::new(), Json(request))
                .await
                .expect("submit_workload accepts the declaration");
        }

        async fn rows(&self) -> Vec<AllocStatusRow> {
            self.state.obs.alloc_status_rows().await.expect("the observation store reads its rows")
        }
    }

    async fn entered(park: &StartPark) {
        tokio::time::timeout(Duration::from_secs(10), park.entered())
            .await
            .expect("setup: the launch reaches the driver's start");
    }

    async fn ends(task: &JoinHandle<()>) -> bool {
        settled(|| task.is_finished()).await
    }

    /// PENDING (boundary 4a (h), normal): node shutdown ends a launch that is waiting inside
    /// `Driver::start`. The launch returns `StartSuperseded`, no row is written, the loop drains and
    /// exits, and the simulated clock never advances (the launch's own bound is never reached).
    #[tokio::test(flavor = "current_thread")]
    #[should_panic(
        expected = "shutdown must raise the signal of the lane a waiting launch is dispatching under"
    )]
    async fn node_shutdown_ends_a_waiting_launch_with_no_row_and_the_loop_exits() {
        let rig = LoopRig::new().await;
        let park = rig.sim.park_next_start(true);
        let task = rig.spawn_loop();
        rig.declare("loop-x").await;
        entered(&park).await;
        let clock_before = rig.clock.now();

        rig.shutdown.cancel();

        assert!(
            settled(|| park.observed_supersession()).await,
            "shutdown must raise the signal of the lane a waiting launch is dispatching under"
        );
        assert!(ends(&task).await, "the loop drains its admitted evaluation and exits");
        assert!(rig.rows().await.is_empty(), "a launch abandoned by shutdown writes no row");
        assert_eq!(
            rig.clock.now(),
            clock_before,
            "no simulated time passed: no launch bound was reached"
        );
    }

    /// GUARD (boundary 4a (h), edge): shutdown leaves a Running allocation and a non-launch
    /// evaluation alone: the allocation stays Running and supervised, the other evaluation runs to
    /// its result, and the loop exits.
    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_leaves_a_running_allocation_and_a_non_launch_evaluation_alone() {
        let rig = LoopRig::new().await;
        let task = rig.spawn_loop();
        rig.declare("loop-x").await;
        assert!(
            settled_rows(&rig, |rows| rows.iter().any(|row| row.state == AllocState::Running))
                .await,
            "setup: the unparked launch reaches Running"
        );
        let before = rig.rows().await;

        let dispatched_before = rig.state.runtime.broker().counters().dispatched;
        rig.state.runtime.submit_evaluation(
            Evaluation {
                reconciler: ReconcilerName::new("noop-heartbeat").expect("name"),
                target: TargetResource::new("node/shutdown-a").expect("target"),
            },
            rig.clock.now(),
            EvaluationEligibility::Immediate,
        );
        assert!(
            settled(|| rig.state.runtime.broker().counters().dispatched > dispatched_before).await,
            "setup: the non-launch evaluation is admitted and runs"
        );
        rig.shutdown.cancel();

        assert!(ends(&task).await, "the loop drains and exits");
        let after = rig.rows().await;
        assert_eq!(
            after.iter().map(|row| (&row.alloc_id, row.state)).collect::<Vec<_>>(),
            before.iter().map(|row| (&row.alloc_id, row.state)).collect::<Vec<_>>(),
            "a Running allocation is unaffected by shutdown"
        );
        assert_eq!(rig.sim.live_count(), 1, "the Running allocation is still supervised");
    }

    async fn settled_rows(rig: &LoopRig, cond: impl Fn(&[AllocStatusRow]) -> bool) -> bool {
        for _ in 0..POLLS {
            if cond(&rig.rows().await) {
                return true;
            }
            pump(1).await;
        }
        false
    }
}
