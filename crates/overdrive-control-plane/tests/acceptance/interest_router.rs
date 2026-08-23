//! Piece B interest-router behaviour (ADR-0081 §5, GH #266) — DST default
//! lane, driving `spawn_interest_router` (the LIVE router) directly against a
//! `SimObservationStore` with a hand-built interest table and a standalone
//! `EvaluationBroker` the test owns and inspects.
//!
//! Scenarios: S-266-08 (interested wakes on change), S-266-09 (empty
//! interests never woken), S-266-12 (derive `workload/<W>` through the
//! router), S-266-14 (Lagged → relist), S-266-15 (List-then-Watch +
//! boot-window), S-266-16 (non-accepted write wakes nobody), S-266-22 (no
//! fan-out storm — write-flood coalesces).
//!
//! The walking skeleton S-266-01 lives in
//! `tests/integration/interest_router_run_server.rs` — it boots the real
//! `run_server_with_obs_and_driver` and asserts the production entry spawns
//! the router (vertical-slice rule: no test installs the one production call
//! site the feature omitted).
//!
//! Two subscription sources are used, both driving the SAME `spawn_interest_router`
//! code path: (a) the REAL `obs.subscribe_all_events()` (store fan-out) for
//! the accepted-write scenarios, and (b) a channel-controlled subscription for
//! the scenarios that must deterministically inject a `Lagged` gap (S-266-14)
//! or an exact watch-delivery count without the store's LWW gating (S-266-22).
//! Forcing a real broadcast lag (>1024 buffered rows) is impractical/flaky, so
//! the controlled subscription is the standard way to exercise the `Lagged`
//! relist handler — the broker and the router are real; only the row-delivery
//! source is a test-owned channel.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::{InterestRouterBroker, spawn_interest_router};
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::eval_broker::{Evaluation, EvaluationBroker};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::ReconcilerName;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LagAwareSubscription, LogicalTimestamp, ObservationRow,
    ObservationRowKind, ObservationStore, SubscriptionEvent,
};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn fresh_store() -> Arc<dyn ObservationStore> {
    Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0))
}

fn fresh_broker() -> Arc<parking_lot::Mutex<EvaluationBroker>> {
    Arc::new(parking_lot::Mutex::new(EvaluationBroker::new()))
}

/// A `Running` `alloc_status` row for `(alloc, workload)` at LWW `counter`.
/// The `local` writer matches the store's node so `dominates` tie-breaks
/// deterministically on the counter alone.
fn alloc_row(alloc: &str, workload: &str, counter: u64) -> AllocStatusRow {
    let node = NodeId::new("local").expect("node id");
    AllocStatusRow {
        alloc_id: AllocationId::new(alloc).expect("alloc id"),
        workload_id: WorkloadId::new(workload).expect("workload id"),
        node_id: node.clone(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter, writer: node },
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
    }
}

async fn write_alloc(obs: &Arc<dyn ObservationStore>, row: AllocStatusRow) {
    obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write alloc_status");
}

fn interest_table(
    kind: ObservationRowKind,
    names: &[&str],
) -> BTreeMap<ObservationRowKind, Vec<ReconcilerName>> {
    let mut table = BTreeMap::new();
    table.insert(
        kind,
        names.iter().map(|n| ReconcilerName::new(n).expect("reconciler name")).collect(),
    );
    table
}

fn handle_for(
    broker: &Arc<parking_lot::Mutex<EvaluationBroker>>,
) -> InterestRouterBroker {
    InterestRouterBroker::from_shared_broker(Arc::clone(broker))
}

/// Subscribe (FIRST) then spawn the router over the real store fan-out.
async fn start_router_real(
    obs: &Arc<dyn ObservationStore>,
    table: BTreeMap<ObservationRowKind, Vec<ReconcilerName>>,
    broker: &Arc<parking_lot::Mutex<EvaluationBroker>>,
) -> (tokio::task::JoinHandle<()>, CancellationToken) {
    let subscription = obs.subscribe_all_events().await.expect("subscribe");
    let shutdown = CancellationToken::new();
    let task = spawn_interest_router(
        Arc::clone(obs),
        subscription,
        table,
        handle_for(broker),
        shutdown.clone(),
    );
    (task, shutdown)
}

/// Poll `cond` up to ~1s (200 × 5ms), yielding to the router task each pass.
async fn eventually<F: FnMut() -> bool>(mut cond: F) -> bool {
    for _ in 0..200 {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    cond()
}

/// Assert `cond` holds across a settle window (the `assert_always` analogue).
async fn holds_for<F: FnMut() -> bool>(mut cond: F, passes: u32) -> bool {
    for _ in 0..passes {
        if !cond() {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    true
}

fn drain(broker: &Arc<parking_lot::Mutex<EvaluationBroker>>) -> Vec<Evaluation> {
    broker.lock().drain_pending()
}

fn has_key(broker: &Arc<parking_lot::Mutex<EvaluationBroker>>, reconciler: &str, target: &str) -> bool {
    broker
        .lock()
        .drain_pending()
        .iter()
        .any(|e| e.reconciler.as_str() == reconciler && e.target.as_str() == target)
}

// ---------------------------------------------------------------------------
// S-266-08 — an interested reconciler wakes when its observed rows change.
// proptest-equivalent: iterate several workload ids and interested-set sizes.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn interested_reconciler_wakes_on_accepted_alloc_status_change() {
    for (idx, (workload, names)) in
        [("w1", &["r-a"][..]), ("payments", &["r-a", "r-b"][..]), ("svc0", &["r-a", "r-b", "r-c"][..])]
            .into_iter()
            .enumerate()
    {
        let obs = fresh_store();
        let broker = fresh_broker();
        let table = interest_table(ObservationRowKind::AllocStatus, names);
        let (task, shutdown) = start_router_real(&obs, table, &broker).await;

        write_alloc(&obs, alloc_row(&format!("a{idx}"), workload, 1)).await;

        let target = format!("workload/{workload}");
        let want = u64::try_from(names.len()).expect("names count fits u64");
        let woke = eventually(|| broker.lock().counters().queued >= want).await;
        assert!(
            woke,
            "router must submit for every interested reconciler on an accepted change ({workload})",
        );
        let pending = drain(&broker);
        let mut got: Vec<String> = pending
            .iter()
            .inspect(|e| assert_eq!(e.target.as_str(), target, "target derived per row"))
            .map(|e| e.reconciler.as_str().to_owned())
            .collect();
        got.sort();
        let mut want: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
        want.sort();
        assert_eq!(got, want, "exactly the interested reconcilers wake for {workload}");

        shutdown.cancel();
        let _ = task.await;
    }
}

// ---------------------------------------------------------------------------
// S-266-09 — a host-state reconciler (empty interests) is never event-woken.
// The router only submits names present in the interest table; a reconciler
// with the default `&[]` is never added, hence never submitted (SD-6).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn host_state_reconciler_with_empty_interests_is_never_event_woken() {
    let obs = fresh_store();
    let broker = fresh_broker();
    // `r-a` is interested; `host-h` (a host-backed reconciler with default
    // empty interests) is NOT in the table — it can never be submitted.
    let table = interest_table(ObservationRowKind::AllocStatus, &["r-a"]);
    let (task, shutdown) = start_router_real(&obs, table, &broker).await;

    write_alloc(&obs, alloc_row("a1", "w1", 1)).await;

    // The interested reconciler wakes …
    assert!(
        eventually(|| broker.lock().counters().queued >= 1).await,
        "the interested reconciler must wake",
    );
    // … and across a settle window, NO submit ever names the host-backed
    // reconciler (empty interests ⟺ never event-woken, SD-6). `host-h` is not
    // in the interest table, so the router can never submit it.
    let mut host_seen = false;
    for _ in 0..30 {
        for eval in drain(&broker) {
            if eval.reconciler.as_str() == "host-h" {
                host_seen = true;
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        !host_seen,
        "a host-backed (empty-interests) reconciler must NEVER be submitted by the router",
    );

    shutdown.cancel();
    let _ = task.await;
}

// ---------------------------------------------------------------------------
// S-266-12 — the router derives `workload/<W>` INLINE from the row and submits
// `(R, workload/W)`. Mutation surface #3 (inline target derivation). proptest-
// equivalent over several workload ids.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn router_derives_workload_scoped_target_inline_from_alloc_status_row() {
    for (idx, workload) in ["w1", "payments", "svc-42", "a"].into_iter().enumerate() {
        let obs = fresh_store();
        let broker = fresh_broker();
        let table = interest_table(ObservationRowKind::AllocStatus, &["r-a"]);
        let (task, shutdown) = start_router_real(&obs, table, &broker).await;

        write_alloc(&obs, alloc_row(&format!("a{idx}"), workload, 1)).await;

        let expected = format!("workload/{workload}");
        let woke = eventually(|| broker.lock().counters().queued >= 1).await;
        assert!(woke, "router must submit for {workload}");
        let pending = drain(&broker);
        assert_eq!(pending.len(), 1, "coalesced to one pending for a single change");
        assert_eq!(pending[0].reconciler.as_str(), "r-a");
        assert_eq!(
            pending[0].target.as_str(),
            expected,
            "target MUST be workload/<row.workload_id> derived inline from the row",
        );

        shutdown.cancel();
        let _ = task.await;
    }
}

// ---------------------------------------------------------------------------
// S-266-14 — after a `Lagged` gap the router relists `alloc_status_rows()` and
// re-submits per derived target — no permanently-missed row. Uses a
// channel-controlled subscription to inject the gap deterministically; the
// row that is caught ONLY via the relist proves the relist is load-bearing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lagged_triggers_relist_and_wakes_every_snapshot_target() {
    let obs = fresh_store();
    let broker = fresh_broker();
    let table = interest_table(ObservationRowKind::AllocStatus, &["r-a"]);

    // A channel-controlled subscription — the router's WATCH sees ONLY what we
    // push; store writes reach the router only via LIST / relist.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SubscriptionEvent>();
    let sub: LagAwareSubscription = Box::new(Box::pin(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|ev| (ev, rx))
    })));

    // Pre-existing row `w1` — the initial LIST must wake it.
    write_alloc(&obs, alloc_row("a1", "w1", 1)).await;

    let shutdown = CancellationToken::new();
    let task = spawn_interest_router(
        Arc::clone(&obs),
        sub,
        table,
        handle_for(&broker),
        shutdown.clone(),
    );

    // Initial LIST wakes w1; drain it so the post-Lagged submits are unambiguous.
    assert!(
        eventually(|| broker.lock().counters().queued >= 1).await,
        "initial LIST must wake the pre-existing row",
    );
    assert!(has_key(&broker, "r-a", "workload/w1"), "LIST woke workload/w1");

    // A NEW row `w2` lands AFTER the initial list — NOT delivered on the
    // controlled watch, so it is reachable ONLY via a relist.
    write_alloc(&obs, alloc_row("a2", "w2", 1)).await;

    // Inject the gap: the router must relist and catch w2 (and re-wake w1).
    tx.send(SubscriptionEvent::Lagged { missed: 3 }).expect("send Lagged");

    let relisted_w2 = eventually(|| {
        broker
            .lock()
            .drain_pending()
            .iter()
            .any(|e| e.reconciler.as_str() == "r-a" && e.target.as_str() == "workload/w2")
    })
    .await;
    assert!(
        relisted_w2,
        "on Lagged the router MUST relist the snapshot and wake every interested target — \
         workload/w2 (added after the initial list, never delivered on the watch) was missed",
    );

    drop(tx);
    shutdown.cancel();
    let _ = task.await;
}

// ---------------------------------------------------------------------------
// S-266-15 — List-then-Watch: a pre-existing row (written BEFORE subscribe)
// wakes via the LIST; a boot-window row (written AFTER subscribe, before the
// list runs) is not missed (subscribe-first). Both targets end up submitted.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_then_watch_wakes_pre_existing_rows_and_misses_no_boot_window_write() {
    let obs = fresh_store();
    let broker = fresh_broker();
    let table = interest_table(ObservationRowKind::AllocStatus, &["r-a"]);

    // (1) Pre-existing row — written BEFORE the subscription opens.
    write_alloc(&obs, alloc_row("a1", "w1", 1)).await;

    // (2) Subscribe FIRST (closes the boot-window gap) …
    let subscription = obs.subscribe_all_events().await.expect("subscribe");

    // (3) … then a boot-window write lands AFTER subscribe, before spawn.
    write_alloc(&obs, alloc_row("a2", "w2", 1)).await;

    let shutdown = CancellationToken::new();
    let task = spawn_interest_router(
        Arc::clone(&obs),
        subscription,
        table,
        handle_for(&broker),
        shutdown.clone(),
    );

    // Both targets are woken: w1 via LIST, w2 via LIST-or-WATCH (never missed
    // because the subscription opened first). They are two distinct broker
    // keys, so they are pending together — drain once and assert both.
    let both = eventually(|| broker.lock().counters().queued >= 2).await;
    assert!(both, "both the pre-existing and boot-window targets must be woken");
    let pending = drain(&broker);
    assert!(
        pending.iter().any(|e| e.target.as_str() == "workload/w1"),
        "the pre-existing row (written before subscribe) must wake via the LIST",
    );
    assert!(
        pending.iter().any(|e| e.target.as_str() == "workload/w2"),
        "the boot-window write (after subscribe) must NOT be missed (subscribe-first ordering)",
    );

    shutdown.cancel();
    let _ = task.await;
}

// ---------------------------------------------------------------------------
// S-266-16 — a non-accepted (LWW-loser) write is never delivered as a `Row`,
// so the router submits nothing for it.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_accepted_lww_loser_write_wakes_nobody() {
    let obs = fresh_store();
    let broker = fresh_broker();
    let table = interest_table(ObservationRowKind::AllocStatus, &["r-a"]);
    let (task, shutdown) = start_router_real(&obs, table, &broker).await;

    // A genuine accepted change wakes the reconciler (counter 5 wins).
    write_alloc(&obs, alloc_row("a1", "w1", 5)).await;
    assert!(
        eventually(|| broker.lock().counters().queued >= 1).await,
        "the accepted write must wake the reconciler",
    );
    let _ = drain(&broker); // clear the accepted-change submit; broker now empty
    assert_eq!(broker.lock().counters().queued, 0, "broker empty after draining the winner");

    // A losing write for the SAME alloc (counter 3 < 5) is rejected by LWW,
    // never broadcast as a `Row`, so the router submits nothing.
    write_alloc(&obs, alloc_row("a1", "w1", 3)).await;

    let stayed_empty = holds_for(|| broker.lock().counters().queued == 0, 30).await;
    assert!(
        stayed_empty,
        "a non-accepted (LWW-loser) write must wake nobody — the router must submit nothing \
         for a write the watcher never delivered as a Row",
    );

    shutdown.cancel();
    let _ = task.await;
}

// ---------------------------------------------------------------------------
// S-266-22 — no fan-out storm: N accepted `alloc_status` writes for the same
// W drive N router submits at `(R, workload/W)`; the broker collapses to ≤1
// pending per drain and `cancelled` increases by exactly N-1. Exercised
// through the LIVE router. A channel-controlled subscription delivers exactly
// N Row events (empty store → LIST contributes nothing) so the submit count is
// deterministic.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn write_flood_coalesces_to_one_pending_eval_per_interested_target() {
    let obs = fresh_store(); // empty — LIST submits nothing
    let broker = fresh_broker();
    let table = interest_table(ObservationRowKind::AllocStatus, &["r-a"]);

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SubscriptionEvent>();
    let sub: LagAwareSubscription = Box::new(Box::pin(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|ev| (ev, rx))
    })));

    let shutdown = CancellationToken::new();
    let task = spawn_interest_router(
        Arc::clone(&obs),
        sub,
        table,
        handle_for(&broker),
        shutdown.clone(),
    );

    // N accepted writes for the SAME workload W arrive on the watch (same
    // broker key `(r-a, workload/w1)`), before the broker drains.
    let n: u64 = 16;
    for c in 1..=n {
        tx.send(SubscriptionEvent::Row(ObservationRow::AllocStatus(Box::new(alloc_row(
            "a1", "w1", c,
        )))))
        .expect("send Row");
    }

    // The broker LWW-collapses to ≤1 pending at the key; `cancelled` reaches
    // exactly N-1. `queued` is NEVER > 1 (same-key submits can only supersede).
    let settled = eventually(|| {
        let c = broker.lock().counters();
        assert!(
            c.queued <= 1,
            "fan-out must NEVER exceed 1 pending at (r-a, workload/w1); got queued={}",
            c.queued,
        );
        c.cancelled == n - 1
    })
    .await;
    assert!(settled, "the write-flood must coalesce: cancelled must reach exactly N-1");

    let c = broker.lock().counters();
    assert_eq!(c.queued, 1, "exactly one pending eval at (r-a, workload/w1)");
    assert_eq!(c.cancelled, n - 1, "cancelled increases by exactly N-1 for N same-key submits");

    drop(tx);
    shutdown.cancel();
    let _ = task.await;
}
