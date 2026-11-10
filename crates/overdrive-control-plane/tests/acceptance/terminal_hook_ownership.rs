//! Terminal-hook ownership (feature `service-allocation-readiness-and-ending`,
//! GH #318 and #334): D1, D2, D3 and D4, gates G-1 and G-2.
//!
//! The composition under test is the production one, assembled in-process with
//! `Sim*` adapters at the driven ports only: the production exit observer
//! (`exit_observer::spawn_with_runtime`) consuming `ExitEvent`s from a
//! `Driver`'s receiver, the production action shim (the VM dispatch seam)
//! dispatching `StartAllocation`, `RestartAllocation`, `StopAllocation` and
//! `FinalizeFailed`, a real `ProbeRunner` over `SimClock`, `SimTcpProber` and
//! `SimObservationStore`, and a `SimDriver`-backed VM-typed driver whose probe
//! hooks forward to that runner exactly as `VmDriver`'s do (a three-line
//! override: `start_alloc` / `stop_alloc` / `stop_role`).
//!
//! The only hand-written double is [`HookDriver`], a `Driver` at the observer's
//! and the shim's driven port. It records the order of the lifecycle hooks and
//! gate calls, and it decides when an exit becomes available (the watcher's
//! part). It authors no row, no decision and no supervisor: every supervisor
//! entry, row and event under assertion is written by production code.
//!
//! Observable universe (port-exposed names only): `ProbeRunner::active_alloc_count()`,
//! `ProbeRunner::is_role_live(alloc, role)`, `ObservationStore::alloc_status_row(alloc)`
//! (state and terminal), `ObservationStore::all_service_backends_rows()`, the lifecycle bus,
//! and the recorded call order of the driver's hooks. Each test compares a full
//! before/after snapshot of that universe against the expected delta.
//!
//! Seeds: the exit-timing tests run a fixed range of seeds; each seed fixes how many
//! scheduler yields the watcher gives the observer right after an exit becomes
//! available. The failing seed is printed in every assertion message.
//!
//! The matching Quint conformance driver is
//! `src/worker/quint_hook_ownership.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::action_shim::LifecycleEvent;
use overdrive_control_plane::worker::exit_observer;
use overdrive_core::SpiffeId;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverType, ExitEvent, ExitKind, Resources, VmPayload,
};
use overdrive_core::traits::observation_store::{
    AllocState, ObservationStore, ObservationStoreError,
};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_worker::probe_runner::ProbeRunner;
use parking_lot::Mutex;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

/// How many seeds each exit-timing scenario runs.
const SEEDS: u64 = 12;

const ROLES: [(ProbeRole, &str); 3] = [
    (ProbeRole::Startup, "startup"),
    (ProbeRole::Readiness, "readiness"),
    (ProbeRole::Liveness, "liveness"),
];

// ----- the recorded driver -------------------------------------------------------------

use crate::acceptance::hooked_driver::{Call, HookedDriver};
use crate::acceptance::vm_dispatch::{VmDispatchFixture, VmDispatchPorts};

fn crash_event(alloc: &AllocationId) -> ExitEvent {
    ExitEvent {
        alloc: alloc.clone(),
        kind: ExitKind::Crashed { exit_code: Some(1), signal: None },
        intentional_stop: false,
        stderr_tail: None,
        oom: None,
    }
}

// ----- the fixture ---------------------------------------------------------------------

struct Fixture {
    obs: Arc<SimObservationStore>,
    clock: Arc<SimClock>,
    runner: Arc<ProbeRunner>,
    driver: Arc<HookedDriver>,
    vm: VmDispatchFixture,
    events: Arc<broadcast::Sender<LifecycleEvent>>,
    ticks: Mutex<u64>,
}

fn node() -> NodeId {
    NodeId::new("node-001").expect("valid node id")
}

fn alloc(name: &str) -> AllocationId {
    AllocationId::new(name).expect("valid alloc id")
}

fn tcp_descriptor(role: ProbeRole, idx: u32) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(idx),
        role,
        mechanic: ProbeMechanic::Tcp { host: "127.0.0.1".to_owned(), port: 8080 },
        timeout_seconds: 1,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: if role == ProbeRole::Liveness { Some(3) } else { None },
        success_threshold: if role == ProbeRole::Readiness { Some(1) } else { None },
        inferred: false,
    }
}

/// A Service allocation declaring a startup, a readiness and a liveness probe.
fn probed_spec(alloc: &AllocationId) -> AllocationSpec {
    spec(
        alloc,
        vec![
            tcp_descriptor(ProbeRole::Startup, 0),
            tcp_descriptor(ProbeRole::Readiness, 0),
            tcp_descriptor(ProbeRole::Liveness, 0),
        ],
    )
}

fn spec(alloc: &AllocationId, probe_descriptors: Vec<ProbeDescriptor>) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::from_str("spiffe://overdrive.local/workload/hooks/alloc/0")
            .expect("valid spiffe id"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/sbin/init".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/kernel"),
            rootfs: PathBuf::from("/rootfs"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors,
        transport: None,
        service_ports: Vec::new(),
        declared_listeners: Vec::new(),
    }
}

impl Fixture {
    /// The production observer, the production shim and a real runner over `Sim*` ports.
    async fn new() -> Self {
        let obs =
            Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
        let clock = Arc::new(SimClock::new());
        let runner = Arc::new(ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(SimGuestStreamDial::new()),
            Arc::new(overdrive_worker::GuestListenerBoard::new()),
        ));
        let sim =
            Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock) as Arc<dyn Clock>));
        let driver = Arc::new(HookedDriver::new(sim, Arc::clone(&runner), Arc::clone(&clock)));
        let mut ports = VmDispatchPorts::sim(Arc::clone(&driver) as Arc<dyn Driver>);
        ports.obs = Arc::clone(&obs) as Arc<dyn ObservationStore>;
        ports.clock = Arc::clone(&clock) as Arc<dyn Clock>;
        ports.node_id = node();
        let vm = VmDispatchFixture::boot(ports).await;
        let events = Arc::clone(&vm.state.lifecycle_events);
        Self { obs, clock, runner, driver, vm, events, ticks: Mutex::new(0) }
    }

    /// Spawns the production exit observer over the fixture's driver.
    fn spawn_observer(&self) -> CancellationToken {
        let token = CancellationToken::new();
        let _handle = exit_observer::spawn_with_runtime(
            Arc::clone(&self.obs) as Arc<dyn ObservationStore>,
            Arc::clone(&self.driver) as Arc<dyn Driver>,
            Arc::clone(&self.events),
            Arc::clone(&self.clock) as Arc<dyn Clock>,
            None,
            token.clone(),
        );
        token
    }

    /// One action through the production VM dispatch seam.
    async fn dispatch(
        &self,
        action: Action,
    ) -> Result<(), overdrive_control_plane::action_shim::ShimError> {
        let tick_no = {
            let mut ticks = self.ticks.lock();
            *ticks += 1;
            *ticks
        };
        let now = Instant::now();
        let tick = TickContext {
            now,
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000 + tick_no)),
            tick: tick_no,
            deadline: now + Duration::from_secs(1),
        };
        self.vm.dispatch(vec![action], &tick).await
    }

    /// `StartAllocation` of a probed Service allocation through the shim.
    async fn start(&self, id: &AllocationId) {
        self.start_with(id, probed_spec(id)).await;
    }

    async fn start_with(&self, id: &AllocationId, spec: AllocationSpec) {
        self.dispatch(Action::StartAllocation {
            alloc_id: id.clone(),
            workload_id: overdrive_core::id::WorkloadId::new("hooks").expect("workload id"),
            node_id: node(),
            spec,
            kind: WorkloadKind::Service,
        })
        .await
        .expect("StartAllocation dispatches");
    }

    /// The guest of `id` dies: the exit is available to the observer now.
    async fn crash(&self, id: &AllocationId) {
        self.driver.deliver_exit(crash_event(id)).await;
    }

    /// Polls (every millisecond, bounded) until `cond` holds. Owners read time only through `SimClock`.
    async fn settle<F, Fut>(&self, what: &str, mut cond: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cond().await {
            assert!(Instant::now() < deadline, "did not settle within the bound: {what}");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    async fn row_state(&self, id: &AllocationId) -> Option<AllocState> {
        self.obs.alloc_status_row(id).await.expect("read alloc row").map(|r| r.state)
    }

    async fn wait_for_terminal_row(&self, id: &AllocationId) {
        let id = id.clone();
        self.settle("the exit observer wrote the terminal row", || {
            let id = id.clone();
            async move {
                matches!(
                    self.row_state(&id).await,
                    Some(AllocState::Failed | AllocState::Terminated)
                )
            }
        })
        .await;
    }

    /// The observable universe over `allocs`.
    async fn observe(&self, allocs: &[&AllocationId]) -> Observed {
        let mut roles_live = BTreeMap::new();
        let mut rows = BTreeMap::new();
        for id in allocs {
            for (role, name) in ROLES {
                roles_live.insert((id.to_string(), name), self.runner.is_role_live(id, role));
            }
            let row = self.obs.alloc_status_row(id).await.expect("read alloc row");
            rows.insert(id.to_string(), row.map(|r| (r.state, r.terminal)));
        }
        Observed {
            supervised: self.runner.active_alloc_count(),
            roles_live,
            rows,
            backend_rows: self
                .obs
                .all_service_backends_rows()
                .await
                .expect("read backend rows")
                .len(),
        }
    }
}

/// A snapshot of the port-exposed universe the hook can move.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Observed {
    /// `ProbeRunner::active_alloc_count()`.
    supervised: usize,
    /// `ProbeRunner::is_role_live(alloc, role)` per allocation and role.
    roles_live: BTreeMap<(String, &'static str), bool>,
    /// `ObservationStore::alloc_status_row(alloc)`: state and terminal.
    rows: BTreeMap<String, Option<(AllocState, Option<TerminalCondition>)>>,
    /// The number of `service_backends` rows.
    backend_rows: usize,
}

impl Observed {
    /// The expected snapshot when `id`'s supervision ends: its roles are not
    /// live and the supervised count falls by one.
    fn without_supervision_of(mut self, id: &AllocationId) -> Self {
        self.supervised -= 1;
        for (_, name) in ROLES {
            self.roles_live.insert((id.to_string(), name), false);
        }
        self
    }

    fn with_row(mut self, id: &AllocationId, state: AllocState) -> Self {
        let terminal = self.rows.get(&id.to_string()).cloned().flatten().and_then(|(_, t)| t);
        self.rows.insert(id.to_string(), Some((state, terminal)));
        self
    }
}

const fn splitmix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// How many scheduler yields the schedule of `seed` gives the other task.
fn yields_of(seed: u64) -> u32 {
    u32::try_from(splitmix(seed) % 6).expect("a small number")
}

// ----- D1, G-1: the exit observer ends the supervision of an exited allocation ---------

/// S-1 (D1, G-1; GH #334): a `Running` Service allocation with a registered supervisor
/// crashes and its restart budget is not due. The observer handles the `ExitEvent`;
/// the supervisor is gone and every role is dead, and the row is `Failed`. No
/// reconciler runs and the `SimClock` is never advanced, so no restart backoff elapses.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_crashed_allocation_stops_probing_as_soon_as_the_observer_consumes_the_exit() {
    for seed in 0..SEEDS {
        let f = Fixture::new().await;
        let _observer = f.spawn_observer();
        let a = alloc("alloc-crash-0");
        f.start(&a).await;
        for _ in 0..yields_of(seed) {
            tokio::task::yield_now().await;
        }
        let before = f.observe(&[&a]).await;
        assert_eq!(before.supervised, 1, "seed {seed}: the shim registered the supervisor");
        assert!(
            before.roles_live[&(a.to_string(), "liveness")],
            "seed {seed}: liveness supervised"
        );

        f.crash(&a).await;
        f.wait_for_terminal_row(&a).await;

        let after = f.observe(&[&a]).await;
        let expected = before.without_supervision_of(&a).with_row(&a, AllocState::Failed);
        assert_eq!(
            after, expected,
            "seed {seed}: the supervisor ends within the observer's handling of the exit, and \
             the row is Failed, before any restart backoff (the clock was never advanced)",
        );
        assert_eq!(
            f.driver.terminal_calls(),
            vec![a.clone()],
            "seed {seed}: one hook call, from the observer"
        );
    }
}

/// S-2 (D1, D4; GH #318): the shim restart path. A `Failed` predecessor with a fresh
/// successor id; `RestartAllocation` is dispatched through the shim. The predecessor is
/// unsupervised, the successor alone is supervised, and the only terminal hook call in the
/// trace is the observer's for the predecessor: the shim made none for the successor and
/// none of its own for the predecessor.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_restart_leaves_only_the_successor_supervised_without_a_second_hook_call() {
    for seed in 0..SEEDS {
        let f = Fixture::new().await;
        let _observer = f.spawn_observer();
        let pred = alloc("alloc-restart-0");
        let succ = alloc("alloc-restart-1");
        f.start(&pred).await;
        f.crash(&pred).await;
        f.wait_for_terminal_row(&pred).await;
        assert_eq!(
            f.driver.terminal_calls(),
            vec![pred.clone()],
            "seed {seed}: the observer fired the hook"
        );
        for _ in 0..yields_of(seed) {
            tokio::task::yield_now().await;
        }

        let before = f.observe(&[&pred, &succ]).await;
        f.dispatch(Action::RestartAllocation {
            alloc_id: pred.clone(),
            spec: probed_spec(&succ),
            kind: WorkloadKind::Service,
        })
        .await
        .expect("RestartAllocation dispatches");

        let after = f.observe(&[&pred, &succ]).await;
        assert_eq!(after.supervised, 1, "seed {seed}: the successor only is supervised");
        for (_, name) in ROLES {
            assert!(
                !after.roles_live[&(pred.to_string(), name)],
                "seed {seed}: predecessor {name} not live"
            );
        }
        assert!(
            after.roles_live[&(succ.to_string(), "liveness")],
            "seed {seed}: successor liveness live"
        );
        assert_eq!(
            after.rows[&pred.to_string()],
            before.rows[&pred.to_string()],
            "seed {seed}: the predecessor's attempt row stays observable and unchanged",
        );
        assert_eq!(
            f.driver.terminal_calls(),
            vec![pred.clone()],
            "seed {seed}: no terminal hook call came from the shim, for the predecessor or the successor",
        );
    }
}

/// S-3 (D1 outcome independence), retries exhausted (`RetryOutcome::Failed`): the observer's
/// row write fails on every attempt of its bounded retry. The supervisor is gone all the
/// same, the row stays `Running`, and the degraded `LifecycleEvent` is still sent.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_crash_ends_probe_supervision_even_when_every_row_write_attempt_fails() {
    for seed in 0..SEEDS {
        let f = Fixture::new().await;
        let mut bus = f.events.subscribe();
        let _observer = f.spawn_observer();
        let a = alloc("alloc-exhaust-0");
        f.start(&a).await;
        let before = f.observe(&[&a]).await;

        // One initial attempt and three retries, all retryable.
        for _ in 0..4 {
            f.obs.inject_write_failure(ObservationStoreError::Io(io::Error::from(
                io::ErrorKind::Interrupted,
            )));
        }
        f.crash(&a).await;

        // The observer backs off on the injected clock between attempts: advance it until the
        // degraded event arrives.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut degraded = None;
        while degraded.is_none() {
            assert!(Instant::now() < deadline, "seed {seed}: the degraded event never arrived");
            f.clock.tick(Duration::from_millis(250));
            tokio::time::sleep(Duration::from_millis(1)).await;
            while let Ok(event) = bus.try_recv() {
                if matches!(event.reason, TransitionReason::DriverInternalError { .. }) {
                    degraded = Some(event);
                }
            }
        }
        let degraded = degraded.expect("the degraded lifecycle event");
        assert_eq!(degraded.alloc_id, a, "seed {seed}: the degraded event names the allocation");

        let after = f.observe(&[&a]).await;
        assert_eq!(
            after,
            before.without_supervision_of(&a),
            "seed {seed}: supervision ended although no row was written; the row stays Running",
        );
        assert_eq!(f.driver.terminal_calls(), vec![a.clone()], "seed {seed}: the hook ran once");
    }
}

/// S-3 (D1 outcome independence), `NoWrite`: the exit names an allocation that has no prior
/// row (so the observer writes nothing). Its supervisor is still gone.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_crash_with_no_prior_row_still_ends_probe_supervision() {
    let f = Fixture::new().await;
    let _observer = f.spawn_observer();
    let a = alloc("alloc-norow-0");
    // Supervision registered through the driver hook; the shim never wrote a row.
    f.driver.on_alloc_running(&probed_spec(&a));
    let before = f.observe(&[&a]).await;
    assert_eq!(before.supervised, 1);
    assert_eq!(before.rows[&a.to_string()], None, "no row exists for the allocation");

    f.crash(&a).await;
    f.settle("the observer fired the hook", || {
        let calls = f.driver.terminal_calls();
        async move { !calls.is_empty() }
    })
    .await;

    assert_eq!(
        f.observe(&[&a]).await,
        before.without_supervision_of(&a),
        "supervision ended and still no row was written",
    );
}

// ----- D3, G-2: the shim registers supervision before it releases the gate -------------

/// S-4 (D3, G-2): the recorded call order of the shim's `StartAllocation` and of the
/// `RestartAllocation` successor is `on_alloc_running` before `release_for_exit_emission`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_start_registers_probe_supervision_before_it_releases_the_exit_gate() {
    let f = Fixture::new().await;
    let _observer = f.spawn_observer();
    let first = alloc("alloc-order-0");
    let successor = alloc("alloc-order-1");

    f.start(&first).await;
    let running =
        f.driver.index_of(&Call::Running(first.clone())).expect("on_alloc_running called");
    let release = f.driver.index_of(&Call::Release(first.clone())).expect("the gate was released");
    assert!(
        running < release,
        "StartAllocation: on_alloc_running ({running}) precedes the gate release ({release})"
    );

    f.crash(&first).await;
    f.wait_for_terminal_row(&first).await;
    f.dispatch(Action::RestartAllocation {
        alloc_id: first.clone(),
        spec: probed_spec(&successor),
        kind: WorkloadKind::Service,
    })
    .await
    .expect("RestartAllocation dispatches");
    let running =
        f.driver.index_of(&Call::Running(successor.clone())).expect("successor registered");
    let release =
        f.driver.index_of(&Call::Release(successor.clone())).expect("successor gate released");
    assert!(
        running < release,
        "RestartAllocation successor: on_alloc_running ({running}) precedes the gate release ({release})",
    );
}

/// S-4 (D3, `NoRegisterAfterEnd`), seeded: the guest is already dead when the gate releases, so
/// the `ExitEvent` is available the instant the gate send happens; the watcher then gives the
/// observer a seed-chosen number of scheduler turns before the shim's next step. Whatever the
/// schedule, after the observer consumed the event no supervisor remains for the id.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn an_exit_available_the_instant_the_gate_releases_leaves_no_supervisor() {
    for seed in 0..SEEDS {
        let f = Fixture::new().await;
        let _observer = f.spawn_observer();
        let a = alloc("alloc-instant-0");
        f.driver.exit_at_release(&a, yields_of(seed));

        f.start(&a).await;
        f.wait_for_terminal_row(&a).await;

        let after = f.observe(&[&a]).await;
        assert_eq!(
            after.supervised,
            0,
            "seed {seed}: no supervisor remains for an id whose exit was consumed (NoRegisterAfterEnd); \
             calls: {:?}",
            f.driver.calls(),
        );
        for (_, name) in ROLES {
            assert!(!after.roles_live[&(a.to_string(), name)], "seed {seed}: {name} not live");
        }
        assert_eq!(
            after.rows[&a.to_string()].as_ref().map(|r| r.0),
            Some(AllocState::Failed),
            "seed {seed}"
        );
    }
}

// ----- D4: the retained shim hooks -----------------------------------------------------

/// S-6 (D4), operator stop: `StopAllocation` of a `Running` allocation ends its supervision
/// exactly once; the row is the operator's `Terminated` as before. A stop is not an exit: no
/// `ExitEvent` is sent, so the observer fires nothing (the emitting watcher's behaviour is the
/// VM driver's, covered by its own suite).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn an_operator_stop_ends_probe_supervision_exactly_once() {
    let f = Fixture::new().await;
    let _observer = f.spawn_observer();
    let a = alloc("alloc-stop-0");
    f.start(&a).await;
    let before = f.observe(&[&a]).await;
    let operator = Some(TerminalCondition::Stopped { by: StoppedBy::Operator });

    f.dispatch(Action::StopAllocation { alloc_id: a.clone(), terminal: operator.clone() })
        .await
        .expect("StopAllocation dispatches");

    let mut expected = before.without_supervision_of(&a);
    expected.rows.insert(a.to_string(), Some((AllocState::Terminated, operator)));
    assert_eq!(f.observe(&[&a]).await, expected, "supervision gone, row Terminated as today");
    assert_eq!(f.driver.terminal_calls(), vec![a], "the stop fired the hook exactly once");
}

/// S-6 (D4), genuine-terminal finalize: `FinalizeFailed` of a live allocation ends its
/// supervision exactly once. It runs against a VM that may still be alive, so no `ExitEvent`
/// exists to do it.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn finalizing_a_live_allocation_ends_probe_supervision_exactly_once() {
    let f = Fixture::new().await;
    let _observer = f.spawn_observer();
    let a = alloc("alloc-finalize-0");
    f.start(&a).await;
    let before = f.observe(&[&a]).await;
    let give_up = Some(TerminalCondition::BackoffExhausted { attempts: 5 });

    f.dispatch(Action::FinalizeFailed { alloc_id: a.clone(), terminal: give_up.clone() })
        .await
        .expect("FinalizeFailed dispatches");

    let after = f.observe(&[&a]).await;
    let mut expected = before.without_supervision_of(&a);
    expected.rows.insert(a.to_string(), Some((AllocState::Failed, give_up)));
    assert_eq!(after, expected, "supervision gone, row carries the give-up terminal as today");
    assert_eq!(f.driver.terminal_calls(), vec![a], "the finalize fired the hook exactly once");
}

// ----- D2: idempotence ------------------------------------------------------------------

/// S-7 (D2): the hook on an unknown or never-registered id, and repeated calls for one id, change
/// nothing beyond the first effect: the supervised count, the sibling's and the successor's
/// supervision, and every row are untouched.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn the_terminal_hook_is_idempotent_and_a_no_op_for_unknown_ids() {
    let f = Fixture::new().await;
    let sibling = alloc("alloc-idem-1");
    let ended = alloc("alloc-idem-0");
    let unknown = alloc("alloc-idem-9");
    f.start(&sibling).await;
    f.start(&ended).await;
    let before = f.observe(&[&sibling, &ended, &unknown]).await;
    assert_eq!(before.supervised, 2);

    f.driver.on_alloc_terminal(&unknown);
    assert_eq!(
        f.observe(&[&sibling, &ended, &unknown]).await,
        before,
        "an unknown id changes nothing"
    );

    f.driver.on_alloc_terminal(&ended);
    let after_first = f.observe(&[&sibling, &ended, &unknown]).await;
    assert_eq!(
        after_first,
        before.clone().without_supervision_of(&ended),
        "the first call ends one supervisor"
    );

    f.driver.on_alloc_terminal(&ended);
    f.driver.on_alloc_terminal(&ended);
    f.driver.on_alloc_terminal(&unknown);
    assert_eq!(
        f.observe(&[&sibling, &ended, &unknown]).await,
        after_first,
        "repeated calls change nothing; the sibling stays supervised",
    );
}

/// S-7 (D2, D1, D4): double fire. The observer fires the hook for the crash; a later
/// `FinalizeFailed` (the give-up) fires it again for the same id. The second call changes
/// nothing, and the sibling's supervisor is untouched.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_crash_then_a_finalize_fire_the_hook_twice_with_one_effect() {
    let f = Fixture::new().await;
    let _observer = f.spawn_observer();
    let a = alloc("alloc-double-0");
    let sibling = alloc("alloc-double-1");
    f.start(&a).await;
    f.start(&sibling).await;

    f.crash(&a).await;
    f.wait_for_terminal_row(&a).await;
    let after_observer = f.observe(&[&a, &sibling]).await;
    assert_eq!(after_observer.supervised, 1, "the observer's hook ended the crashed id only");
    assert!(after_observer.roles_live[&(sibling.to_string(), "liveness")]);

    let give_up = Some(TerminalCondition::BackoffExhausted { attempts: 5 });
    f.dispatch(Action::FinalizeFailed { alloc_id: a.clone(), terminal: give_up.clone() })
        .await
        .expect("FinalizeFailed dispatches");

    let mut expected = after_observer;
    expected.rows.insert(a.to_string(), Some((AllocState::Failed, give_up)));
    assert_eq!(
        f.observe(&[&a, &sibling]).await,
        expected,
        "the second fire changed only the row's terminal"
    );
    assert_eq!(f.driver.terminal_calls(), vec![a.clone(), a], "two fires, one effect");
}

// ----- unaffected states and the default path ------------------------------------------

/// S-8 (D1): a crash of one of two allocations of a Service leaves the sibling alone: its probe
/// roles stay live (so its readiness rows, from which `healthy` is derived, keep being
/// produced), its row is unchanged, and no `service_backends` row is authored by the hook.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_crash_ends_only_the_crashed_allocations_supervision() {
    let f = Fixture::new().await;
    let _observer = f.spawn_observer();
    let crashed = alloc("alloc-sibling-0");
    let sibling = alloc("alloc-sibling-1");
    f.start(&crashed).await;
    f.start(&sibling).await;
    let before = f.observe(&[&crashed, &sibling]).await;

    f.crash(&crashed).await;
    f.wait_for_terminal_row(&crashed).await;

    let after = f.observe(&[&crashed, &sibling]).await;
    assert_eq!(
        after,
        before.without_supervision_of(&crashed).with_row(&crashed, AllocState::Failed),
        "only the crashed allocation's supervision and row moved",
    );
}

/// S-8 control (unaffected state, passes without D1): whatever the observer does for the crashed
/// allocation, the sibling's supervision, its row and the backend rows are exactly as before.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_crash_leaves_the_siblings_supervision_and_row_untouched() {
    let f = Fixture::new().await;
    let _observer = f.spawn_observer();
    let crashed = alloc("alloc-control-0");
    let sibling = alloc("alloc-control-1");
    f.start(&crashed).await;
    f.start(&sibling).await;
    let before = f.observe(&[&sibling]).await;

    f.crash(&crashed).await;
    f.wait_for_terminal_row(&crashed).await;

    let after = f.observe(&[&sibling]).await;
    assert_eq!(after.roles_live, before.roles_live, "the sibling's roles are unchanged");
    assert_eq!(after.rows, before.rows, "the sibling's row is unchanged");
    assert_eq!(after.backend_rows, before.backend_rows, "no backend row was authored");
}

/// S-8 (disabled path): a driver with no probe runner takes the default path. The observer's
/// hook call is the trait's default no-op, the row is written as before, and nothing panics.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_driver_with_no_probe_runner_takes_the_default_path() {
    let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    let clock = Arc::new(SimClock::new());
    let plain =
        Arc::new(SimDriver::with_clock(DriverType::Vm, Arc::clone(&clock) as Arc<dyn Clock>));
    let events = Arc::new(broadcast::channel(16).0);
    let _handle = exit_observer::spawn_with_runtime(
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
        Arc::clone(&plain) as Arc<dyn Driver>,
        events,
        Arc::clone(&clock) as Arc<dyn Clock>,
        None,
        CancellationToken::new(),
    );
    let a = alloc("alloc-plain-0");
    let handle = plain.start(&probed_spec(&a)).await.expect("start");
    plain.release_for_exit_emission(&handle).await;
    let running = overdrive_core::traits::observation_store::AllocStatusRow {
        alloc_id: a.clone(),
        workload_id: overdrive_core::id::WorkloadId::new("hooks").expect("workload id"),
        node_id: node(),
        state: AllocState::Running,
        updated_at: overdrive_core::traits::observation_store::LogicalTimestamp {
            counter: 0,
            writer: node(),
        },
        reason: Some(TransitionReason::Started),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: None,
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };
    obs.write_alloc_lifecycle(
        running,
        overdrive_core::traits::observation_store::TransitionSource::Reconciler,
    )
    .await
    .expect("seed Running row");

    plain.inject_exit_after(
        &a,
        Duration::ZERO,
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let row = obs.alloc_status_row(&a).await.expect("read alloc row");
        if row.as_ref().is_some_and(|r| r.state == AllocState::Failed) {
            break;
        }
        assert!(Instant::now() < deadline, "the observer wrote the Failed row on the default path");
        clock.tick(Duration::from_millis(1));
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}
