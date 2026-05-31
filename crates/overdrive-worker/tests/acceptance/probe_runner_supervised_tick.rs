//! GAP-7 closure — `ProbeRunner::start_alloc` spawns per-descriptor
//! supervised tick tasks that drive `probe_once_and_record` and write
//! `ProbeResultRow`s on every `clock.sleep(interval)` resolve, until
//! the per-alloc supervisor's cancellation token fires.
//!
//! Pre-patch (commit `004032b8` and earlier) `start_alloc` registered
//! a supervisor and discarded the `probe_descriptors` parameter as
//! `_probe_descriptors` — net effect: no `ProbeResultRow` was ever
//! written in production, so the `ServiceLifecycleReconciler` never
//! observed `ProbeStatus::Pass` and the Stable verdict could not fire.
//! See `.context/01-03-structural-gap-audit.md` GAP-7.
//!
//! This acceptance test pins the post-patch loop body:
//!
//! - **AT-01** — `start_alloc` spawns a per-descriptor tick task.
//!   Advancing the injected `SimClock` past one `interval` produces
//!   exactly one `ProbeResultRow` write at the
//!   `(alloc_id, probe_idx=0)` primary key. The structural property
//!   is "spawning a tick task is observable as a row write."
//!
//! - **AT-02** — `stop_alloc` cooperatively shuts the spawned task
//!   down. Subsequent clock advances past several intervals MUST NOT
//!   produce additional rows. The structural property is "the child
//!   token derived from the supervisor's root token actually drains
//!   the supervised loop."
//!
//! Port-to-port shape: the AT enters through `ProbeRunner::start_alloc`
//! (the driving port that `ExecDriver::on_alloc_running` dispatches to
//! in production) and asserts on the observable state at the driven
//! port boundary — `ObservationStore::list_probe_results_for_alloc`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::observation::{ProbeRole, ProbeStatus};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::ProbeOutcome;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};
use overdrive_worker::probe_runner::ProbeRunner;

fn alloc_id(s: &str) -> AllocationId {
    AllocationId::new(s).expect("alloc id parses")
}

fn node_id_for_obs_store() -> NodeId {
    NodeId::new("supervised-tick-test").expect("node id parses")
}

/// Descriptor with a 1-second tick interval — short enough for tests
/// to advance the SimClock past one interval without unbounded looping,
/// long enough that the post-tick `child_token.cancelled()` poll
/// (which fires in the same select iteration) does not race the next
/// `clock.sleep` registration in the typical case.
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

/// Yield to the tokio scheduler enough times that any spawned task
/// poll on `clock.sleep(...)` registers its waker against the
/// `SimClock` timer registry BEFORE the next `clock.tick(...)` fires.
/// Without this the test races between `tokio::spawn` returning and
/// the spawned future actually being polled the first time.
async fn yield_for_task_poll() {
    // Three yields covers the `tokio::spawn` → first poll → `clock.sleep`
    // registration latency across both current-thread and
    // multi-threaded tokio runtimes. The cost is three scheduler
    // ticks; the alternative (a wall-clock sleep) would un-determine
    // the test.
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// Poll `list_probe_results_for_alloc` against the obs store until
/// it observes `at_least` rows OR `max_yields` scheduler yields
/// elapse. Returns the observed rows. Deterministic — no wall-clock
/// sleep, just scheduler yields that drain the spawned task's
/// `write_probe_result` future.
async fn wait_for_rows(
    obs: &SimObservationStore,
    alloc: &AllocationId,
    at_least: usize,
    max_yields: usize,
) -> Vec<overdrive_core::observation::ProbeResultRow> {
    for _ in 0..max_yields {
        let rows = obs
            .list_probe_results_for_alloc(alloc)
            .await
            .expect("list_probe_results_for_alloc succeeds");
        if rows.len() >= at_least {
            return rows;
        }
        tokio::task::yield_now().await;
    }
    obs.list_probe_results_for_alloc(alloc)
        .await
        .expect("list_probe_results_for_alloc succeeds (final read after yield budget)")
}

/// AT-01 — `start_alloc(alloc_id, vec![descriptor])` spawns a
/// per-descriptor tick task whose `clock.sleep(interval) →
/// probe_tick → observation_store.write_probe_result` round-trip
/// produces exactly one `ProbeResultRow` at `(alloc_id, probe_idx=0)`
/// after the SimClock advances past one `interval`.
///
/// Pre-patch the loop body did not exist — `start_alloc` discarded
/// the descriptor vector. Under that shape the assertion
/// `rows.len() == 1` fails (rows.len() == 0): no row was ever
/// written.
#[tokio::test]
async fn given_start_alloc_with_one_tcp_descriptor_when_clock_ticks_interval_then_writes_probe_result_row()
 {
    let tcp = Arc::new(SimTcpProber::new());
    tcp.enqueue_outcome(ProbeOutcome::Pass);
    let http = Arc::new(SimHttpProber::new());
    let exec = Arc::new(SimExecProber::new());
    let clock = Arc::new(SimClock::default());
    let obs = Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0));

    let runner = ProbeRunner::new(
        tcp,
        http,
        exec,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
    );

    let alloc = alloc_id("alloc-supervised-tick-1");
    let descriptor = descriptor_tcp_1s("127.0.0.1", 9999);

    // BEFORE: zero rows, zero supervisors.
    let before = obs
        .list_probe_results_for_alloc(&alloc)
        .await
        .expect("list_probe_results_for_alloc before start");
    assert!(before.is_empty(), "no rows before start_alloc");
    assert_eq!(runner.active_alloc_count(), 0, "no supervisor before start_alloc");

    // ACT: start the alloc with the descriptor. This registers a
    // per-alloc supervisor AND spawns one tick task per descriptor.
    let _token = runner.start_alloc(&alloc, vec![descriptor]);
    assert_eq!(
        runner.active_alloc_count(),
        1,
        "start_alloc must register the supervisor (covers register_alloc → BTreeMap insert)"
    );

    // Give the spawned task a chance to poll once and park on
    // `clock.sleep(1s)`. Without this yield, the next `clock.tick`
    // races the spawned task's first poll.
    yield_for_task_poll().await;

    // Advance the SimClock past one descriptor interval (1s). The
    // spawned task's `clock.sleep(1s)` future wakes, the tick body
    // runs, the row lands in the obs store.
    clock.tick(Duration::from_secs(1));

    // Wait for the row to appear — the chain is
    // tick → SleepUntil::Ready → next poll → probe_tick → write.
    let rows = wait_for_rows(&obs, &alloc, 1, 64).await;

    assert_eq!(
        rows.len(),
        1,
        "exactly one ProbeResultRow expected after first tick (post-patch loop body); \
         pre-patch start_alloc discarded the descriptor and rows.len() was 0"
    );
    let row = &rows[0];
    assert_eq!(row.alloc_id, alloc);
    assert_eq!(row.probe_idx.0, 0);
    assert_eq!(row.role, ProbeRole::Startup);
    assert_eq!(row.status, ProbeStatus::Pass);
    assert!(!row.inferred, "operator-declared descriptor → row.inferred = false");

    // Cleanup: cancel the supervisor so the spawned task exits
    // before the test scope drops. Without this the task remains
    // parked on the next clock.sleep(1s) until the runtime is torn
    // down — harmless but noisy.
    runner.stop_alloc(&alloc);
}

/// AT-02 — `stop_alloc` cooperatively drains the supervised tick
/// loop. After `stop_alloc`, subsequent `clock.tick(interval)`
/// invocations MUST NOT produce additional `ProbeResultRow`s.
///
/// The structural property is that the child token derived from
/// the supervisor's root token (per `AllocSupervisor::spawn_probe_task`)
/// actually wakes the `select! { ..., child_token.cancelled() }`
/// arm and causes the loop body to `return`. Pre-patch the loop
/// body did not exist; this AT is meaningless under that shape (no
/// loop to drain). Post-patch it pins the cooperative-shutdown
/// guarantee that downstream operators rely on.
#[tokio::test]
async fn given_started_alloc_when_stop_alloc_then_no_further_probe_result_rows() {
    let tcp = Arc::new(SimTcpProber::new());
    // Enqueue enough outcomes that the spawned loop never starves;
    // the assertion is "no rows after stop," NOT "exhausts the
    // queue." A starving queue would surface as `ProbeOutcome::Pass`
    // (SimTcpProber's empty-queue default), which still produces
    // a row — so the assertion remains load-bearing either way.
    for _ in 0..16 {
        tcp.enqueue_outcome(ProbeOutcome::Pass);
    }
    let http = Arc::new(SimHttpProber::new());
    let exec = Arc::new(SimExecProber::new());
    let clock = Arc::new(SimClock::default());
    let obs = Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0));

    let runner = ProbeRunner::new(
        tcp,
        http,
        exec,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
    );

    let alloc = alloc_id("alloc-supervised-tick-2");
    let descriptor = descriptor_tcp_1s("127.0.0.1", 9999);

    let _token = runner.start_alloc(&alloc, vec![descriptor]);
    yield_for_task_poll().await;

    // Fire one tick so we have a known-non-zero baseline before
    // cancellation. If the row never lands, the test would falsely
    // pass (zero rows after stop is trivially satisfied by zero
    // rows EVER); the baseline pins the loop is actually ticking.
    clock.tick(Duration::from_secs(1));
    let rows_before_stop = wait_for_rows(&obs, &alloc, 1, 64).await;
    assert_eq!(
        rows_before_stop.len(),
        1,
        "baseline: one tick fires before stop — pre-patch start_alloc would yield 0 here too"
    );

    // ACT: cancel the supervisor. The spawned task observes the
    // child token cancellation on its next `select!` round and
    // returns from `supervised_probe_loop`.
    runner.stop_alloc(&alloc);
    assert_eq!(runner.active_alloc_count(), 0, "supervisor removed");

    // Give the spawned task a chance to observe cancellation.
    yield_for_task_poll().await;

    // Advance the clock past several intervals — under the LWW
    // contract (`last_observed_at_unix_ms` strictly dominates), even
    // a re-fire would land at most one row per probe_idx, which the
    // first row would already saturate. The structural property is
    // sharper: NO write_probe_result calls happen after stop_alloc,
    // because the loop body returned.
    for _ in 0..5 {
        clock.tick(Duration::from_secs(1));
        yield_for_task_poll().await;
    }

    // STATE-DELTA: ProbeResultRow count is unchanged from the
    // pre-stop baseline. The LWW semantics keep the row count at 1
    // per probe_idx regardless, but the row's
    // `last_observed_at_unix_ms` would shift if the loop had not
    // drained. Pin the timestamp to assert the loop is genuinely
    // stopped, not just LWW-silenced.
    let rows_after_stop = obs
        .list_probe_results_for_alloc(&alloc)
        .await
        .expect("list_probe_results_for_alloc after stop");
    assert_eq!(
        rows_after_stop.len(),
        1,
        "row count unchanged after stop (LWW gives at most one per probe_idx anyway)"
    );
    assert_eq!(
        rows_after_stop[0].last_observed_at_unix_ms, rows_before_stop[0].last_observed_at_unix_ms,
        "last_observed_at_unix_ms unchanged — the loop did not re-fire after stop_alloc; \
         a non-cancelling stop_alloc would shift this timestamp on every post-stop clock.tick"
    );
}

/// AT-03 — Regression: `start_alloc` called twice for the same
/// `alloc_id` MUST NOT spawn a second set of probe tasks.
///
/// Pre-fix `start_alloc` re-entered the for-loop unconditionally
/// on re-call — two tasks per descriptor, writing at double cadence.
/// The observable: after one clock tick, `SimTcpProber::probe_call_count()`
/// must be exactly 1 (one task fired), not 2 (duplicate tasks).
#[tokio::test]
async fn given_start_alloc_called_twice_then_no_duplicate_probe_tasks() {
    let tcp = Arc::new(SimTcpProber::new());
    let http = Arc::new(SimHttpProber::new());
    let exec = Arc::new(SimExecProber::new());
    let clock = Arc::new(SimClock::default());
    let obs = Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0));

    let runner = ProbeRunner::new(
        Arc::clone(&tcp) as Arc<dyn overdrive_core::traits::prober::TcpProber>,
        http,
        exec,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
    );

    let alloc = alloc_id("alloc-no-dup");
    let descriptor = descriptor_tcp_1s("127.0.0.1", 9999);

    // First start — spawns 1 task.
    let token1 = runner.start_alloc(&alloc, vec![descriptor.clone()]);

    // Second start — MUST be a no-op (same alloc_id).
    let token2 = runner.start_alloc(&alloc, vec![descriptor]);

    assert_eq!(
        runner.active_alloc_count(),
        1,
        "second start_alloc must not create a second supervisor"
    );

    yield_for_task_poll().await;

    // Tick past one interval — exactly 1 task should fire.
    clock.tick(Duration::from_secs(1));

    // Wait for the single expected row.
    let _rows = wait_for_rows(&obs, &alloc, 1, 64).await;

    // The structural invariant: only 1 probe invocation, not 2.
    assert_eq!(
        tcp.probe_call_count(),
        1,
        "duplicate start_alloc must not spawn duplicate probe tasks; \
         expected 1 probe invocation but got {}",
        tcp.probe_call_count()
    );

    // Both calls returned the same root token.
    drop(token1);
    drop(token2);
    runner.stop_alloc(&alloc);
}
