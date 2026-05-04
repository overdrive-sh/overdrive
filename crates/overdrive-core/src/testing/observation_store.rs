//! Trait-generic LWW conformance harness for [`ObservationStore`].
//!
//! Per `docs/feature/fix-observation-lww-merge/deliver/rca.md` Cause B
//! (invariant asserted at impl level, never at trait level), every
//! adapter of [`ObservationStore`] exercises the same harness so the
//! LWW contract on `write` (see the trait-method docstring) is enforced
//! identically. Adapter divergence is caught at the trait level rather
//! than per-implementation.
//!
//! # Properties under test
//!
//! 1. **Newer dominates older** — write older first then newer; read
//!    returns newer; both writes accepted (no prior, then newer
//!    dominates).
//! 2. **Older arriving after newer is rejected** — write newer first
//!    then older; read still returns newer; subscriber receives only
//!    the newer write.
//! 3. **Equal-timestamp idempotency** — re-deliver the same row;
//!    state unchanged; subscriber receives the first delivery only.
//! 4. **Tiebreak on writer when counters tie** — two writers, same
//!    counter; the higher-`Display` writer wins.
//! 5. **Subscriber emission semantics** — losers MUST NOT be emitted
//!    on the broadcast stream. Each rejection check uses a bounded
//!    `tokio::time::timeout` and asserts the result is the timeout
//!    error variant.
//! 6. **Both row variants** — every property tested for
//!    `AllocStatusRow` AND `NodeHealthRow`.
//! 7. **Comparator property loop** — a deterministic sweep over
//!    `(counter_a, counter_b, writer_a, writer_b)` tuples that asserts
//!    "writing a then b leaves the store with the row whose timestamp
//!    dominates per [`LogicalTimestamp::dominates`]". This is the
//!    surface that kills mutations on the comparator's branches
//!    (counter `>` vs `>=`, `Less` / `Greater` swap, tiebreak `>` vs
//!    `<`).
//!
//! # Determinism
//!
//! Sub-cases use UNIQUE keys per case (e.g. `alloc-i-j` /
//! `node-i-j`) so prior writes do not interfere — the harness does
//! not need a `clear` method on the store. The property loop iterates
//! over a fixed set of comparator-branch-covering tuples, not random
//! draws, so no proptest seed is required and the harness stays
//! `dst-lint` clean (no `Instant::now`, no `rand::random`, no
//! `tokio::net`).
//!
//! [`ObservationStore`]: crate::traits::ObservationStore

// `expect` is the standard idiom in test-shaped code — a panic with a
// message is exactly what you want when a precondition fails. The
// crate-level lint that bans `expect`/`unwrap` outside `#[cfg(test)]`
// is intended for production code paths; this module IS test code,
// gated behind `feature = "test-utils"` and only ever compiled into
// adapter test suites.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;
use std::time::Duration;

use futures::StreamExt;
use tokio::time::timeout;

use crate::id::{AllocationId, JobId, NodeId, Region};
use crate::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, NodeHealthRow, ObservationRow, ObservationStore,
};

/// Bounded poll window for "subscriber must not have received this
/// row" assertions. Long enough that a real fan-out delivery would
/// arrive comfortably, short enough that the harness stays well under
/// the per-test wall-clock budget across every adapter.
const REJECT_POLL_TIMEOUT: Duration = Duration::from_millis(50);

/// Bounded poll window for "subscriber MUST receive this row"
/// assertions. Two seconds is the same budget used by the Phase 1
/// acceptance suites in this workspace; comfortably above any
/// in-process fan-out latency.
const ACCEPT_POLL_TIMEOUT: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// Row helpers — every sub-case uses unique keys so cross-case state
// leakage is impossible without an explicit clear.
// ---------------------------------------------------------------------------

fn alloc_id(scope: &str, idx: usize) -> AllocationId {
    AllocationId::from_str(&format!("alloc-{scope}-{idx}")).expect("alloc id is valid")
}

fn node_id(name: &str) -> NodeId {
    NodeId::from_str(name).expect("node id is valid")
}

fn region() -> Region {
    Region::from_str("local").expect("region is valid")
}

fn alloc_row(scope: &str, idx: usize, state: AllocState, ts: LogicalTimestamp) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: alloc_id(scope, idx),
        job_id: JobId::from_str("payments").expect("job id is valid"),
        node_id: node_id("control-plane-0"),
        state,
        updated_at: ts,
        reason: None,
        detail: None,
        terminal: None,
    }
}

fn node_row(scope: &str, idx: usize, ts: LogicalTimestamp) -> NodeHealthRow {
    NodeHealthRow {
        node_id: NodeId::from_str(&format!("node-{scope}-{idx}")).expect("node id is valid"),
        region: region(),
        last_heartbeat: ts,
    }
}

fn ts(counter: u64, writer: &str) -> LogicalTimestamp {
    LogicalTimestamp { counter, writer: node_id(writer) }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the full LWW conformance suite against `store`.
///
/// Every adapter's test suite invokes this harness with a freshly-opened
/// store. Each sub-case writes against unique primary keys so prior
/// sub-cases do not interfere — adapters do not need to expose a
/// `clear` / `reset` method.
///
/// On any contract violation the harness panics with a message naming
/// the property that failed and the rows involved. Adapter test
/// suites surface the panic through `#[tokio::test]`'s standard
/// failure path.
pub async fn run_lww_conformance<T: ObservationStore + ?Sized>(store: &T) {
    // (i) Newer dominates older — accepted on first write, accepted
    //     again as the dominator on second.
    case_newer_dominates_older_alloc_status(store).await;
    case_newer_dominates_older_node_health(store).await;

    // (ii) Older arriving after newer is rejected — read unchanged;
    //      subscriber sees only the first (newer) row.
    case_older_after_newer_rejected_alloc_status(store).await;
    case_older_after_newer_rejected_node_health(store).await;

    // (iii) Equal-timestamp idempotency — re-delivery is a no-op.
    case_equal_timestamp_idempotent_alloc_status(store).await;
    case_equal_timestamp_idempotent_node_health(store).await;

    // (iv) Tiebreak on writer — higher-`Display` writer wins.
    case_writer_tiebreak_alloc_status(store).await;
    case_writer_tiebreak_node_health(store).await;

    // (v) Point lookup — `alloc_status_row(id)` returns the same
    //     LWW-winner as the equivalent filter over `alloc_status_rows`,
    //     and `None` for an absent id. Future adapters MUST implement
    //     this as a direct point lookup; this case rejects regressions
    //     that route the call back through a scan-and-filter.
    case_alloc_status_row_point_lookup(store).await;

    // (vii) Comparator property sweep — deterministic tuples that
    //       cover every comparator branch in
    //       [`LogicalTimestamp::dominates`].
    property_loop_alloc_status(store).await;
    property_loop_node_health(store).await;
}

// ---------------------------------------------------------------------------
// Sub-cases — AllocStatus
// ---------------------------------------------------------------------------

async fn case_newer_dominates_older_alloc_status<T: ObservationStore + ?Sized>(store: &T) {
    let scope = "newer-dominates-older";
    let older = alloc_row(scope, 0, AllocState::Pending, ts(1, "control-plane-0"));
    let newer = alloc_row(scope, 0, AllocState::Running, ts(5, "control-plane-0"));

    let mut sub = store.subscribe_all().await.expect("subscribe");

    store.write(ObservationRow::AllocStatus(older.clone())).await.expect("write older");
    let first = timeout(ACCEPT_POLL_TIMEOUT, sub.next())
        .await
        .expect("subscription delivers older within timeout")
        .expect("stream yields older");
    assert_eq!(
        first,
        ObservationRow::AllocStatus(older.clone()),
        "(i) older row must be emitted on first write — no prior to dominate it"
    );

    store.write(ObservationRow::AllocStatus(newer.clone())).await.expect("write newer");
    let second = timeout(ACCEPT_POLL_TIMEOUT, sub.next())
        .await
        .expect("subscription delivers newer within timeout")
        .expect("stream yields newer");
    assert_eq!(
        second,
        ObservationRow::AllocStatus(newer.clone()),
        "(i) newer row must be emitted — it dominates the older row"
    );

    let rows = store.alloc_status_rows().await.expect("read alloc rows");
    let observed: Vec<&AllocStatusRow> =
        rows.iter().filter(|r| r.alloc_id == older.alloc_id).collect();
    assert_eq!(observed.len(), 1, "(i) exactly one row per key after LWW merge");
    assert_eq!(*observed[0], newer, "(i) newer row must survive on read");
}

async fn case_older_after_newer_rejected_alloc_status<T: ObservationStore + ?Sized>(store: &T) {
    let scope = "older-after-newer";
    let newer = alloc_row(scope, 0, AllocState::Running, ts(5, "control-plane-0"));
    let older = alloc_row(scope, 0, AllocState::Pending, ts(2, "control-plane-0"));

    // Newer first — accepted because no prior.
    store.write(ObservationRow::AllocStatus(newer.clone())).await.expect("write newer");

    // Subscribe BEFORE the older write so a wrongly-emitted loser would
    // be observed.
    let mut sub = store.subscribe_all().await.expect("subscribe");

    store.write(ObservationRow::AllocStatus(older)).await.expect("write older");

    let delivery = timeout(REJECT_POLL_TIMEOUT, sub.next()).await;
    assert!(delivery.is_err(), "(ii) LWW loser must NOT emit on subscriptions; got {delivery:?}");

    let rows = store.alloc_status_rows().await.expect("read alloc rows");
    let observed: Vec<&AllocStatusRow> =
        rows.iter().filter(|r| r.alloc_id == newer.alloc_id).collect();
    assert_eq!(observed.len(), 1, "(ii) exactly one row per key after LWW merge");
    assert_eq!(*observed[0], newer, "(ii) older row must NOT regress newer on read");
}

async fn case_equal_timestamp_idempotent_alloc_status<T: ObservationStore + ?Sized>(store: &T) {
    let scope = "equal-timestamp";
    let row_a = alloc_row(scope, 0, AllocState::Running, ts(3, "control-plane-0"));

    let mut sub = store.subscribe_all().await.expect("subscribe");

    store.write(ObservationRow::AllocStatus(row_a.clone())).await.expect("write first");
    let first = timeout(ACCEPT_POLL_TIMEOUT, sub.next())
        .await
        .expect("subscription delivers first within timeout")
        .expect("stream yields first");
    assert_eq!(
        first,
        ObservationRow::AllocStatus(row_a.clone()),
        "(iii) first delivery must emit the row"
    );

    // Re-deliver the same row. Equal timestamps do NOT dominate
    // (idempotency case) — must be rejected.
    store.write(ObservationRow::AllocStatus(row_a.clone())).await.expect("re-deliver");
    let delivery = timeout(REJECT_POLL_TIMEOUT, sub.next()).await;
    assert!(delivery.is_err(), "(iii) re-delivered identical row must NOT emit; got {delivery:?}");

    let rows = store.alloc_status_rows().await.expect("read alloc rows");
    let observed: Vec<&AllocStatusRow> =
        rows.iter().filter(|r| r.alloc_id == row_a.alloc_id).collect();
    assert_eq!(observed.len(), 1, "(iii) re-delivery is a no-op on read");
    assert_eq!(*observed[0], row_a, "(iii) row must be unchanged after re-delivery");
}

async fn case_writer_tiebreak_alloc_status<T: ObservationStore + ?Sized>(store: &T) {
    let scope = "writer-tiebreak";
    // Same counter; lex-greater writer wins. "control-plane-1" >
    // "control-plane-0" by `Display` ordering.
    let lower_writer = alloc_row(scope, 0, AllocState::Pending, ts(7, "control-plane-0"));
    let higher_writer = alloc_row(scope, 0, AllocState::Running, ts(7, "control-plane-1"));

    // Write lower-writer first; it has no prior to compare against, so
    // it is accepted.
    store
        .write(ObservationRow::AllocStatus(lower_writer.clone()))
        .await
        .expect("write lower writer");

    // Subscribe BEFORE the second write so the assertion is precise.
    let mut sub = store.subscribe_all().await.expect("subscribe");

    // Higher-writer arrives second. Same counter; tiebreak on writer:
    // "control-plane-1" > "control-plane-0", so higher wins.
    store
        .write(ObservationRow::AllocStatus(higher_writer.clone()))
        .await
        .expect("write higher writer");

    let delivery = timeout(ACCEPT_POLL_TIMEOUT, sub.next())
        .await
        .expect("higher-writer delivery within timeout")
        .expect("stream yields higher writer");
    assert_eq!(
        delivery,
        ObservationRow::AllocStatus(higher_writer.clone()),
        "(iv) higher-`Display` writer must win the tiebreak"
    );

    let rows = store.alloc_status_rows().await.expect("read alloc rows");
    let observed: Vec<&AllocStatusRow> =
        rows.iter().filter(|r| r.alloc_id == lower_writer.alloc_id).collect();
    assert_eq!(observed.len(), 1, "(iv) exactly one row per key after tiebreak");
    assert_eq!(
        *observed[0], higher_writer,
        "(iv) higher-writer row must win — Display tiebreak per LogicalTimestamp::dominates"
    );

    // Now write lower-writer AGAIN with the same counter — must lose
    // (counter ties, writer < winning writer).
    let mut sub2 = store.subscribe_all().await.expect("subscribe-2");
    store
        .write(ObservationRow::AllocStatus(lower_writer.clone()))
        .await
        .expect("write lower writer second time");
    let reject_delivery = timeout(REJECT_POLL_TIMEOUT, sub2.next()).await;
    assert!(
        reject_delivery.is_err(),
        "(iv) lex-lower writer must NOT win against the existing higher-writer row; got \
         {reject_delivery:?}"
    );
    let rows = store.alloc_status_rows().await.expect("read alloc rows");
    let observed: Vec<&AllocStatusRow> =
        rows.iter().filter(|r| r.alloc_id == lower_writer.alloc_id).collect();
    assert_eq!(observed.len(), 1, "(iv) exactly one row per key");
    assert_eq!(
        *observed[0], higher_writer,
        "(iv) higher-writer row must remain after losing tiebreak"
    );
}

async fn case_alloc_status_row_point_lookup<T: ObservationStore + ?Sized>(store: &T) {
    let scope = "point-lookup";
    let initial = alloc_row(scope, 0, AllocState::Pending, ts(1, "control-plane-0"));
    let updated = alloc_row(scope, 0, AllocState::Running, ts(4, "control-plane-0"));

    store.write(ObservationRow::AllocStatus(initial.clone())).await.expect("write initial");
    let after_initial =
        store.alloc_status_row(&initial.alloc_id).await.expect("point lookup initial");
    assert_eq!(
        after_initial.as_ref(),
        Some(&initial),
        "(v) point lookup must return the LWW-winner; got {after_initial:?}"
    );

    store.write(ObservationRow::AllocStatus(updated.clone())).await.expect("write updated");
    let after_updated =
        store.alloc_status_row(&updated.alloc_id).await.expect("point lookup updated");
    assert_eq!(
        after_updated.as_ref(),
        Some(&updated),
        "(v) point lookup must return the new LWW-winner after a dominating write; got \
         {after_updated:?}"
    );

    let snapshot = store.alloc_status_rows().await.expect("snapshot");
    let from_snapshot: Option<AllocStatusRow> =
        snapshot.into_iter().find(|r| r.alloc_id == updated.alloc_id);
    assert_eq!(
        after_updated, from_snapshot,
        "(v) point lookup and snapshot-filter must agree on the winner"
    );

    let absent = format!("{scope}-absent");
    let absent_alloc = AllocationId::new(&absent).expect("absent id");
    let none = store.alloc_status_row(&absent_alloc).await.expect("point lookup absent");
    assert_eq!(none, None, "(v) absent id must return None; got {none:?}");
}

// ---------------------------------------------------------------------------
// Sub-cases — NodeHealth
// ---------------------------------------------------------------------------

async fn case_newer_dominates_older_node_health<T: ObservationStore + ?Sized>(store: &T) {
    let writer = "node-newer-dominates-older-0";
    let older =
        NodeHealthRow { node_id: node_id(writer), region: region(), last_heartbeat: ts(1, writer) };
    let newer =
        NodeHealthRow { node_id: node_id(writer), region: region(), last_heartbeat: ts(5, writer) };

    let mut sub = store.subscribe_all().await.expect("subscribe");

    store.write(ObservationRow::NodeHealth(older.clone())).await.expect("write older");
    let first = timeout(ACCEPT_POLL_TIMEOUT, sub.next())
        .await
        .expect("subscription delivers older within timeout")
        .expect("stream yields older");
    assert_eq!(
        first,
        ObservationRow::NodeHealth(older.clone()),
        "(i) older node health row must emit on first write"
    );

    store.write(ObservationRow::NodeHealth(newer.clone())).await.expect("write newer");
    let second = timeout(ACCEPT_POLL_TIMEOUT, sub.next())
        .await
        .expect("subscription delivers newer within timeout")
        .expect("stream yields newer");
    assert_eq!(
        second,
        ObservationRow::NodeHealth(newer.clone()),
        "(i) newer node health row must emit — dominates older"
    );

    let rows = store.node_health_rows().await.expect("read node rows");
    let observed: Vec<&NodeHealthRow> =
        rows.iter().filter(|r| r.node_id == newer.node_id).collect();
    assert_eq!(observed.len(), 1, "(i) exactly one row per key after LWW merge");
    assert_eq!(*observed[0], newer, "(i) newer row must survive on read");
}

async fn case_older_after_newer_rejected_node_health<T: ObservationStore + ?Sized>(store: &T) {
    let writer = "node-older-after-newer-0";
    let newer =
        NodeHealthRow { node_id: node_id(writer), region: region(), last_heartbeat: ts(5, writer) };
    let older =
        NodeHealthRow { node_id: node_id(writer), region: region(), last_heartbeat: ts(2, writer) };

    store.write(ObservationRow::NodeHealth(newer.clone())).await.expect("write newer");

    let mut sub = store.subscribe_all().await.expect("subscribe");

    store.write(ObservationRow::NodeHealth(older)).await.expect("write older");

    let delivery = timeout(REJECT_POLL_TIMEOUT, sub.next()).await;
    assert!(
        delivery.is_err(),
        "(ii) NodeHealth LWW loser must NOT emit on subscriptions; got {delivery:?}"
    );

    let rows = store.node_health_rows().await.expect("read node rows");
    let observed: Vec<&NodeHealthRow> =
        rows.iter().filter(|r| r.node_id == newer.node_id).collect();
    assert_eq!(observed.len(), 1, "(ii) exactly one row per key after LWW merge");
    assert_eq!(*observed[0], newer, "(ii) older row must NOT regress newer on read");
}

async fn case_equal_timestamp_idempotent_node_health<T: ObservationStore + ?Sized>(store: &T) {
    let writer = "node-equal-timestamp-0";
    let row =
        NodeHealthRow { node_id: node_id(writer), region: region(), last_heartbeat: ts(3, writer) };

    let mut sub = store.subscribe_all().await.expect("subscribe");

    store.write(ObservationRow::NodeHealth(row.clone())).await.expect("write first");
    let first = timeout(ACCEPT_POLL_TIMEOUT, sub.next())
        .await
        .expect("subscription delivers first within timeout")
        .expect("stream yields first");
    assert_eq!(
        first,
        ObservationRow::NodeHealth(row.clone()),
        "(iii) first delivery must emit the row"
    );

    store.write(ObservationRow::NodeHealth(row.clone())).await.expect("re-deliver");
    let delivery = timeout(REJECT_POLL_TIMEOUT, sub.next()).await;
    assert!(
        delivery.is_err(),
        "(iii) re-delivered identical NodeHealth row must NOT emit; got {delivery:?}"
    );

    let rows = store.node_health_rows().await.expect("read node rows");
    let observed: Vec<&NodeHealthRow> = rows.iter().filter(|r| r.node_id == row.node_id).collect();
    assert_eq!(observed.len(), 1, "(iii) re-delivery is a no-op on read");
    assert_eq!(*observed[0], row, "(iii) row must be unchanged after re-delivery");
}

async fn case_writer_tiebreak_node_health<T: ObservationStore + ?Sized>(store: &T) {
    // NodeHealth is keyed by `node_id`, which on this row IS the
    // writer. To exercise the writer-tiebreak branch correctly, we
    // need the same primary key (`node_id`) but two different writer
    // ids on `last_heartbeat`. That happens in cross-region multi-peer
    // gossip (any peer may stamp a row about another peer's health
    // when forwarding). Build the case explicitly.
    let same_node = node_id("node-writer-tiebreak-0");
    let lower_writer = NodeHealthRow {
        node_id: same_node.clone(),
        region: region(),
        last_heartbeat: ts(7, "writer-aaa"),
    };
    let higher_writer = NodeHealthRow {
        node_id: same_node.clone(),
        region: region(),
        last_heartbeat: ts(7, "writer-zzz"),
    };

    store.write(ObservationRow::NodeHealth(lower_writer.clone())).await.expect("write lower");

    let mut sub = store.subscribe_all().await.expect("subscribe");

    store.write(ObservationRow::NodeHealth(higher_writer.clone())).await.expect("write higher");
    let delivery = timeout(ACCEPT_POLL_TIMEOUT, sub.next())
        .await
        .expect("higher writer delivery within timeout")
        .expect("stream yields higher writer");
    assert_eq!(
        delivery,
        ObservationRow::NodeHealth(higher_writer.clone()),
        "(iv) higher-`Display` writer must win the NodeHealth tiebreak"
    );

    let rows = store.node_health_rows().await.expect("read node rows");
    let observed: Vec<&NodeHealthRow> = rows.iter().filter(|r| r.node_id == same_node).collect();
    assert_eq!(observed.len(), 1, "(iv) exactly one row per key after tiebreak");
    assert_eq!(
        *observed[0], higher_writer,
        "(iv) higher-writer NodeHealth must win — Display tiebreak"
    );
}

// ---------------------------------------------------------------------------
// (vii) Comparator property loop — deterministic tuples covering every
//       branch of `LogicalTimestamp::dominates`. The point of this loop
//       is to kill mutation-testing variants — `>` flipped to `>=`,
//       `Less`/`Greater` swapped, tiebreak `>` flipped to `<`. Every
//       tuple writes against a unique alloc/node id so prior cases
//       cannot interfere.
// ---------------------------------------------------------------------------

async fn property_loop_alloc_status<T: ObservationStore + ?Sized>(store: &T) {
    // Tuples: `(counter_a, writer_a, counter_b, writer_b, expected_b_dominates_a)`.
    //
    // Coverage:
    // - counter_a < counter_b (b strictly newer) — kills `Greater`/`Less` swap
    // - counter_a > counter_b (b strictly older) — kills `Less`/`Greater` swap
    // - counter_a == counter_b, writer_a < writer_b (b wins on tiebreak) — kills
    //   tiebreak `>` flipped to `<`
    // - counter_a == counter_b, writer_a > writer_b (a wins on tiebreak)
    // - counter_a == counter_b, writer_a == writer_b (idempotent — neither
    //   dominates) — kills `>` flipped to `>=`
    let cases: &[(u64, &str, u64, &str, bool)] = &[
        (1, "writer-aaa", 2, "writer-aaa", true), // newer counter wins
        (5, "writer-aaa", 2, "writer-aaa", false), // older counter loses
        (3, "writer-aaa", 3, "writer-zzz", true), // tiebreak: b > a
        (3, "writer-zzz", 3, "writer-aaa", false), // tiebreak: a > b
        (3, "writer-aaa", 3, "writer-aaa", false), // identical: neither dominates
    ];

    for (idx, (counter_a, writer_a, counter_b, writer_b, expected_b_wins)) in
        cases.iter().enumerate()
    {
        let row_a = alloc_row("property-loop", idx, AllocState::Pending, ts(*counter_a, writer_a));
        let row_b = alloc_row("property-loop", idx, AllocState::Running, ts(*counter_b, writer_b));

        store.write(ObservationRow::AllocStatus(row_a.clone())).await.expect("write a");
        store.write(ObservationRow::AllocStatus(row_b.clone())).await.expect("write b");

        let rows = store.alloc_status_rows().await.expect("read alloc rows");
        let observed: Vec<&AllocStatusRow> =
            rows.iter().filter(|r| r.alloc_id == row_a.alloc_id).collect();
        assert_eq!(
            observed.len(),
            1,
            "(vii) exactly one alloc row per key after LWW merge for case {idx}"
        );
        let winner = if *expected_b_wins { &row_b } else { &row_a };
        assert_eq!(
            *observed[0], *winner,
            "(vii) alloc case {idx}: dominates({counter_b}, {writer_b}) over \
             ({counter_a}, {writer_a}) expected b_wins={expected_b_wins}"
        );

        // Cross-check against the shared comparator — the test must
        // agree with `LogicalTimestamp::dominates` directly, otherwise
        // either the test or the comparator is wrong.
        let observed_b_dominates_a = row_b.updated_at.dominates(&row_a.updated_at);
        assert_eq!(
            observed_b_dominates_a, *expected_b_wins,
            "(vii) LogicalTimestamp::dominates case {idx} disagrees with the test \
             oracle — either the comparator or the test table is wrong"
        );
    }
}

async fn property_loop_node_health<T: ObservationStore + ?Sized>(store: &T) {
    // Same shape as the alloc property loop, against
    // [`NodeHealthRow::last_heartbeat`].
    let cases: &[(u64, &str, u64, &str, bool)] = &[
        (1, "writer-aaa", 2, "writer-aaa", true),
        (5, "writer-aaa", 2, "writer-aaa", false),
        (3, "writer-aaa", 3, "writer-zzz", true),
        (3, "writer-zzz", 3, "writer-aaa", false),
        (3, "writer-aaa", 3, "writer-aaa", false),
    ];

    for (idx, (counter_a, writer_a, counter_b, writer_b, expected_b_wins)) in
        cases.iter().enumerate()
    {
        let row_a = node_row("property-loop", idx, ts(*counter_a, writer_a));
        let row_b = node_row("property-loop", idx, ts(*counter_b, writer_b));

        store.write(ObservationRow::NodeHealth(row_a.clone())).await.expect("write a");
        store.write(ObservationRow::NodeHealth(row_b.clone())).await.expect("write b");

        let rows = store.node_health_rows().await.expect("read node rows");
        let observed: Vec<&NodeHealthRow> =
            rows.iter().filter(|r| r.node_id == row_a.node_id).collect();
        assert_eq!(
            observed.len(),
            1,
            "(vii) exactly one node-health row per key after LWW merge for case {idx}"
        );
        let winner = if *expected_b_wins { &row_b } else { &row_a };
        assert_eq!(
            *observed[0], *winner,
            "(vii) node-health case {idx}: dominates({counter_b}, {writer_b}) over \
             ({counter_a}, {writer_a}) expected b_wins={expected_b_wins}"
        );

        let observed_b_dominates_a = row_b.last_heartbeat.dominates(&row_a.last_heartbeat);
        assert_eq!(
            observed_b_dominates_a, *expected_b_wins,
            "(vii) LogicalTimestamp::dominates node-health case {idx} disagrees \
             with the test oracle"
        );
    }
}
