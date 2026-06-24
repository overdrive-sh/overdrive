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
//! 8. **Issued-certificate append-only** — the `issued_certificates`
//!    audit table is keyed by serial with no `updated_at`. A second
//!    write at an already-present serial MUST be rejected: the prior
//!    audit row is never overwritten, and the duplicate is never
//!    re-broadcast (`write` returns no second fan-out). Defends the
//!    contract on [`ObservationStore::issued_certificate_rows`] against
//!    a blind upsert.
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
use futures::future::join_all;
use tokio::time::timeout;

use crate::UnixInstant;
use crate::ca::issued_certificate_row::IssuedCertificateRow;
use crate::id::{AllocationId, CertSerial, IssuanceOrdinal, NodeId, Region, SpiffeId, WorkloadId};
use crate::traits::observation_store::{
    AllocState, AllocStatusRow, LagAwareSubscription, LogicalTimestamp, NodeHealthRow,
    ObservationRow, ObservationStore, SubscriptionEvent,
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

/// Await exactly one delivered [`ObservationRow`] off a lag-surfacing
/// subscription, within [`ACCEPT_POLL_TIMEOUT`].
///
/// The conformance harness writes one row at a time and drains it
/// immediately, so the 1024-deep broadcast window cannot overrun — a
/// [`SubscriptionEvent::Lagged`] here is a structural impossibility that
/// signals a real lag bug. The helper therefore **panics loudly** on
/// `Lagged` rather than skipping it: surfacing the loss is the whole point
/// of the migration off the (deleted) lossy `subscribe_all`. A timeout (no
/// delivery) is a genuine "row never emitted" conformance failure.
async fn expect_emitted_row(sub: &mut LagAwareSubscription, ctx: &str) -> ObservationRow {
    match timeout(ACCEPT_POLL_TIMEOUT, sub.next()).await {
        Ok(Some(SubscriptionEvent::Row(row))) => row,
        Ok(Some(SubscriptionEvent::Lagged { missed })) => panic!(
            "{ctx}: LWW conformance harness observed broadcast Lagged({missed}) — the harness \
             writes/drains one row at a time and cannot lag; the broadcast window was overrun"
        ),
        Ok(None) => panic!("{ctx}: subscription stream ended before the expected row was emitted"),
        Err(elapsed) => {
            panic!("{ctx}: subscription did not deliver the expected row within timeout: {elapsed}")
        }
    }
}

/// Assert NO row is emitted on a lag-surfacing subscription within
/// [`REJECT_POLL_TIMEOUT`] — the LWW-loser / append-only-duplicate
/// suppression check.
///
/// The pass condition is a TIMEOUT (the loser was correctly never
/// broadcast). A delivered [`SubscriptionEvent::Row`] is the conformance
/// failure this defends against (a rejected row wrongly fanned out). A
/// [`SubscriptionEvent::Lagged`] is — as in [`expect_emitted_row`] — a
/// structural impossibility for this harness; it **panics loudly** so a
/// real lag cannot masquerade as the (correct) "loser suppressed" timeout.
async fn assert_no_emission(sub: &mut LagAwareSubscription, ctx: &str) {
    match timeout(REJECT_POLL_TIMEOUT, sub.next()).await {
        // Timeout — the loser was never broadcast. This is the pass.
        Err(_) => {}
        Ok(Some(SubscriptionEvent::Row(row))) => {
            panic!("{ctx}: LWW loser / duplicate must NOT emit on subscriptions; got {row:?}")
        }
        Ok(Some(SubscriptionEvent::Lagged { missed })) => panic!(
            "{ctx}: LWW conformance harness observed broadcast Lagged({missed}) — the harness \
             writes/drains one row at a time and cannot lag; a lag must not be mistaken for \
             loser-suppression"
        ),
        Ok(None) => {
            panic!("{ctx}: subscription stream ended unexpectedly during a rejection check")
        }
    }
}

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
    // Subsidiary GAP-1 fix: trait-conformance harness rows model
    // generic LWW shapes. `None` on Pending (no Running observation
    // yet); `Some(_)` on Running-or-later states to match the
    // production invariant. Value is fixed for test determinism.
    let started_at = match state {
        AllocState::Pending => None,
        _ => Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
    };
    AllocStatusRow {
        alloc_id: alloc_id(scope, idx),
        workload_id: WorkloadId::from_str("payments").expect("job id is valid"),
        node_id: node_id("control-plane-0"),
        state,
        updated_at: ts,
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: crate::aggregate::WorkloadKind::Service,
        listeners: Vec::new(),
        started_at,
        // Generic LWW-conformance harness rows are host-netns shapes —
        // no Path-A netns provision, so no canonical workload address.
        // `None`, symmetric with `AllocationSpec.netns`/`host_veth`
        // being absent (canonical-workload-address-inbound-tproxy,
        // GH #241 / AllocStatusRowV2 additive field).
        workload_addr: None,
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

/// Build an `issued_certificates` audit row at a given serial. `spiffe`
/// varies the row *body* so two rows sharing one serial are still
/// distinguishable — the append-only case relies on a body difference to
/// prove the prior row was (not) overwritten. Timestamps are fixed for
/// determinism; they are audit inputs, not an LWW comparison key.
fn issued_cert_row(serial: &CertSerial, spiffe: &str) -> IssuedCertificateRow {
    let at = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    IssuedCertificateRow {
        serial: serial.clone(),
        spiffe_id: SpiffeId::from_str(spiffe).expect("spiffe id is valid"),
        issuer_serial: CertSerial::new("00").expect("issuer serial parses"),
        not_before: at,
        not_after: at,
        node_id: node_id("control-plane-0"),
        issued_at: at,
        // The append-only contract case distinguishes rows by `spiffe` body
        // difference, not ordinal; a fixed ordinal suffices here.
        issuance_ordinal: IssuanceOrdinal::new(0),
    }
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

    // (viii) Append-only contract for `issued_certificates` — a
    //        duplicate serial never overwrites the prior audit row and
    //        is never re-broadcast. Unlike the LWW cases there is no
    //        `updated_at`; the serial key itself is the immutability
    //        boundary. See
    //        `docs/feature/fix-issued-cert-append-only/deliver/rca.md`.
    case_issued_certificate_append_only(store).await;
}

// ---------------------------------------------------------------------------
// Sub-cases — AllocStatus
// ---------------------------------------------------------------------------

async fn case_newer_dominates_older_alloc_status<T: ObservationStore + ?Sized>(store: &T) {
    let scope = "newer-dominates-older";
    let older = alloc_row(scope, 0, AllocState::Pending, ts(1, "control-plane-0"));
    let newer = alloc_row(scope, 0, AllocState::Running, ts(5, "control-plane-0"));

    let mut sub = store.subscribe_all_events().await.expect("subscribe");

    store.write(ObservationRow::AllocStatus(Box::new(older.clone()))).await.expect("write older");
    let first = expect_emitted_row(&mut sub, "(i) alloc older delivery").await;
    assert_eq!(
        first,
        ObservationRow::AllocStatus(Box::new(older.clone())),
        "(i) older row must be emitted on first write — no prior to dominate it"
    );

    store.write(ObservationRow::AllocStatus(Box::new(newer.clone()))).await.expect("write newer");
    let second = expect_emitted_row(&mut sub, "(i) alloc newer delivery").await;
    assert_eq!(
        second,
        ObservationRow::AllocStatus(Box::new(newer.clone())),
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
    store.write(ObservationRow::AllocStatus(Box::new(newer.clone()))).await.expect("write newer");

    // Subscribe BEFORE the older write so a wrongly-emitted loser would
    // be observed.
    let mut sub = store.subscribe_all_events().await.expect("subscribe");

    store.write(ObservationRow::AllocStatus(Box::new(older))).await.expect("write older");

    assert_no_emission(&mut sub, "(ii) alloc LWW loser").await;

    let rows = store.alloc_status_rows().await.expect("read alloc rows");
    let observed: Vec<&AllocStatusRow> =
        rows.iter().filter(|r| r.alloc_id == newer.alloc_id).collect();
    assert_eq!(observed.len(), 1, "(ii) exactly one row per key after LWW merge");
    assert_eq!(*observed[0], newer, "(ii) older row must NOT regress newer on read");
}

async fn case_equal_timestamp_idempotent_alloc_status<T: ObservationStore + ?Sized>(store: &T) {
    let scope = "equal-timestamp";
    let row_a = alloc_row(scope, 0, AllocState::Running, ts(3, "control-plane-0"));

    let mut sub = store.subscribe_all_events().await.expect("subscribe");

    store.write(ObservationRow::AllocStatus(Box::new(row_a.clone()))).await.expect("write first");
    let first = expect_emitted_row(&mut sub, "(iii) alloc first delivery").await;
    assert_eq!(
        first,
        ObservationRow::AllocStatus(Box::new(row_a.clone())),
        "(iii) first delivery must emit the row"
    );

    // Re-deliver the same row. Equal timestamps do NOT dominate
    // (idempotency case) — must be rejected.
    store.write(ObservationRow::AllocStatus(Box::new(row_a.clone()))).await.expect("re-deliver");
    assert_no_emission(&mut sub, "(iii) alloc re-delivered identical row").await;

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
        .write(ObservationRow::AllocStatus(Box::new(lower_writer.clone())))
        .await
        .expect("write lower writer");

    // Subscribe BEFORE the second write so the assertion is precise.
    let mut sub = store.subscribe_all_events().await.expect("subscribe");

    // Higher-writer arrives second. Same counter; tiebreak on writer:
    // "control-plane-1" > "control-plane-0", so higher wins.
    store
        .write(ObservationRow::AllocStatus(Box::new(higher_writer.clone())))
        .await
        .expect("write higher writer");

    let delivery = expect_emitted_row(&mut sub, "(iv) alloc higher-writer delivery").await;
    assert_eq!(
        delivery,
        ObservationRow::AllocStatus(Box::new(higher_writer.clone())),
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
    let mut sub2 = store.subscribe_all_events().await.expect("subscribe-2");
    store
        .write(ObservationRow::AllocStatus(Box::new(lower_writer.clone())))
        .await
        .expect("write lower writer second time");
    assert_no_emission(&mut sub2, "(iv) alloc lex-lower writer must lose tiebreak").await;
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

    store
        .write(ObservationRow::AllocStatus(Box::new(initial.clone())))
        .await
        .expect("write initial");
    let after_initial =
        store.alloc_status_row(&initial.alloc_id).await.expect("point lookup initial");
    assert_eq!(
        after_initial.as_ref(),
        Some(&initial),
        "(v) point lookup must return the LWW-winner; got {after_initial:?}"
    );

    store
        .write(ObservationRow::AllocStatus(Box::new(updated.clone())))
        .await
        .expect("write updated");
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

    let mut sub = store.subscribe_all_events().await.expect("subscribe");

    store.write(ObservationRow::NodeHealth(older.clone())).await.expect("write older");
    let first = expect_emitted_row(&mut sub, "(i) node older delivery").await;
    assert_eq!(
        first,
        ObservationRow::NodeHealth(older.clone()),
        "(i) older node health row must emit on first write"
    );

    store.write(ObservationRow::NodeHealth(newer.clone())).await.expect("write newer");
    let second = expect_emitted_row(&mut sub, "(i) node newer delivery").await;
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

    let mut sub = store.subscribe_all_events().await.expect("subscribe");

    store.write(ObservationRow::NodeHealth(older)).await.expect("write older");

    assert_no_emission(&mut sub, "(ii) node LWW loser").await;

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

    let mut sub = store.subscribe_all_events().await.expect("subscribe");

    store.write(ObservationRow::NodeHealth(row.clone())).await.expect("write first");
    let first = expect_emitted_row(&mut sub, "(iii) node first delivery").await;
    assert_eq!(
        first,
        ObservationRow::NodeHealth(row.clone()),
        "(iii) first delivery must emit the row"
    );

    store.write(ObservationRow::NodeHealth(row.clone())).await.expect("re-deliver");
    assert_no_emission(&mut sub, "(iii) node re-delivered identical row").await;

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

    let mut sub = store.subscribe_all_events().await.expect("subscribe");

    store.write(ObservationRow::NodeHealth(higher_writer.clone())).await.expect("write higher");
    let delivery = expect_emitted_row(&mut sub, "(iv) node higher-writer delivery").await;
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

        store.write(ObservationRow::AllocStatus(Box::new(row_a.clone()))).await.expect("write a");
        store.write(ObservationRow::AllocStatus(Box::new(row_b.clone()))).await.expect("write b");

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

// ---------------------------------------------------------------------------
// Sub-case — IssuedCertificate append-only
// ---------------------------------------------------------------------------

/// (viii) The `issued_certificates` audit table is append-only, keyed by
/// serial with no `updated_at` to compare. A second write at an
/// already-present serial MUST be a no-op: the prior audit row is never
/// overwritten, and — mirroring the LWW-reject path — the duplicate is
/// never re-broadcast.
///
/// RED against a blind `table.insert` (redb upsert) / `BTreeMap::insert`:
/// the overwrite replaces the stored body (fails the "still the first
/// body" assertion) and the unconditional `true` return drives a second
/// fan-out (fails the "no re-broadcast" assertion). GREEN once both
/// adapters guard on prior-key presence and return `false` on collision.
async fn case_issued_certificate_append_only<T: ObservationStore + ?Sized>(store: &T) {
    let serial = CertSerial::new("0badc0de").expect("serial parses");
    let first = issued_cert_row(&serial, "spiffe://overdrive.local/wl/first");
    let second = issued_cert_row(&serial, "spiffe://overdrive.local/wl/second");
    assert_ne!(first, second, "(viii) test rows must differ in body to detect overwrite");

    let mut sub = store.subscribe_all_events().await.expect("subscribe");

    // First issuance at a fresh serial is accepted and fans out unchanged.
    store
        .write(ObservationRow::IssuedCertificate(first.clone()))
        .await
        .expect("write first issuance");
    let received = expect_emitted_row(&mut sub, "(viii) first issuance delivery").await;
    assert_eq!(
        received,
        ObservationRow::IssuedCertificate(first.clone()),
        "(viii) first issuance must fan out unchanged"
    );

    // Second write at the SAME serial is a duplicate — append-only rejects it.
    store
        .write(ObservationRow::IssuedCertificate(second.clone()))
        .await
        .expect("write duplicate serial");

    // (a) No second fan-out: a duplicate serial must NOT be re-broadcast.
    assert_no_emission(&mut sub, "(viii) duplicate serial must not re-broadcast").await;

    // (b) The stored audit row is still the FIRST body, never overwritten.
    let rows = store.issued_certificate_rows().await.expect("(viii) read issued certificate rows");
    let stored: Vec<_> = rows.into_iter().filter(|r| r.serial == serial).collect();
    assert_eq!(stored.len(), 1, "(viii) append-only: exactly one row per serial");
    assert_eq!(
        stored[0], first,
        "(viii) append-only: the original audit row must never be overwritten by a duplicate serial"
    );
}

// ---------------------------------------------------------------------------
// Issuance-ordinal allocation conformance (ADR-0063 D6 rev 8)
//
// The sibling of `run_lww_conformance` for the additive
// `ObservationStore::next_issuance_ordinal` port method. Both adapters
// (`SimObservationStore`, `LocalObservationStore`) are driven through one
// allocation sequence and asserted to observe the § 3.1 trait contract
// identically (`development.md` § "The DST equivalence test is the
// structural guard"). The TOCTOU this method closes is documented in
// `docs/feature/fix-issuance-ordinal-toctou/deliver/rca.md`: the former
// `issued_certificate_rows().len()` derivation let two concurrent
// issuances stamp DUPLICATE ordinals; an atomically-allocated durable
// counter makes that collision unrepresentable.
// ---------------------------------------------------------------------------

/// Number of concurrent allocations driven in the monotonic-and-unique
/// sub-case. Large enough that an interleaving slip would surface a
/// duplicate (the pre-fix `len()` derivation collided at any N >= 2);
/// small enough to stay well under the per-test wall-clock budget.
const ORDINAL_CONCURRENCY: usize = 64;

/// Run the issuance-ordinal allocation conformance suite against `store`.
///
/// Drives BOTH adapters through the same allocation sequence and asserts
/// the § 3.1 observable invariants of
/// [`ObservationStore::next_issuance_ordinal`]:
///
/// 1. **Monotonic-and-unique under concurrency** (architecture § 4.3 #1) —
///    `ORDINAL_CONCURRENCY` concurrent `next_issuance_ordinal()` calls
///    against one store yield that many DISTINCT ordinals which, once
///    sorted, are strictly increasing with NO gaps for this contiguous
///    run (every value in the half-open range is hit exactly once).
/// 2. **Independent of the audit table** (architecture § 4.3 #2) —
///    allocate, write an `issued_certificate` row, allocate again; the
///    second ordinal is strictly greater than the first AND is NOT equal
///    to `issued_certificate_rows().len()` (proving the ordinal is not a
///    function of the row count — the exact `len()`-derivation defect this
///    fix removes).
///
/// The host-only durable-across-reopen sub-case (architecture § 4.3 #3) is
/// NOT driven here — it requires dropping and reopening a real redb file,
/// which only the host adapter can do. That sub-case lives in the host
/// adapter's `integration-tests`-gated suite (see
/// `overdrive-store-local/tests/integration/issuance_ordinal_durable_reopen.rs`).
///
/// On any contract violation the harness panics with a message naming the
/// property that failed and the ordinals involved.
pub async fn run_issuance_ordinal_conformance<T: ObservationStore + ?Sized>(store: &T) {
    case_monotonic_and_unique_under_concurrency(store).await;
    case_independent_of_audit_table(store).await;
}

/// (1) Monotonic-and-unique under concurrency. Fires
/// `ORDINAL_CONCURRENCY` allocations concurrently (`join_all`) against the
/// SAME store and asserts the returned ordinals are all distinct and form
/// a strictly-increasing contiguous run once sorted. The pre-fix
/// `len()`-derived ordinal collided here: two concurrent reads of the
/// same `len()` stamped the same value.
async fn case_monotonic_and_unique_under_concurrency<T: ObservationStore + ?Sized>(store: &T) {
    let allocations = join_all((0..ORDINAL_CONCURRENCY).map(|_| store.next_issuance_ordinal()));
    let mut ordinals: Vec<u64> = allocations
        .await
        .into_iter()
        .map(|r| r.expect("(1) ordinal allocation must succeed").as_u64())
        .collect();

    assert_eq!(
        ordinals.len(),
        ORDINAL_CONCURRENCY,
        "(1) every concurrent allocation must return a value"
    );

    ordinals.sort_unstable();

    // No two concurrent callers ever receive the same value.
    let mut deduped = ordinals.clone();
    deduped.dedup();
    assert_eq!(
        deduped.len(),
        ordinals.len(),
        "(1) concurrent allocations must be UNIQUE — duplicate ordinals would re-open the \
         TOCTOU this method closes; got {ordinals:?}"
    );

    // Strictly increasing AND contiguous for this fresh run: a fresh store
    // starts at 0, so the first ORDINAL_CONCURRENCY allocations are exactly
    // 0..ORDINAL_CONCURRENCY in some order.
    for (expected, observed) in ordinals.iter().enumerate() {
        assert_eq!(
            *observed, expected as u64,
            "(1) sorted ordinals must be the contiguous strictly-increasing run \
             0..{ORDINAL_CONCURRENCY}; got {ordinals:?}"
        );
    }
}

/// (2) Independent of the audit table. Allocates, writes an
/// `issued_certificate` row (advancing the audit table's `len()` WITHOUT
/// advancing the counter), then allocates again. The second ordinal must
/// be strictly greater than the first and MUST NOT equal the audit
/// table's row count — pinning "not derived from `len()`" and the gap
/// semantics in one assertion.
async fn case_independent_of_audit_table<T: ObservationStore + ?Sized>(store: &T) {
    let first = store.next_issuance_ordinal().await.expect("(2) first allocation").as_u64();

    // Write an audit row at a serial unique to this sub-case. This bumps
    // `issued_certificate_rows().len()` but does NOT touch the counter.
    let serial = CertSerial::new("0ad17a51").expect("(2) serial parses");
    let row = issued_cert_row(&serial, "spiffe://overdrive.local/wl/ordinal-independence");
    store
        .write(ObservationRow::IssuedCertificate(row))
        .await
        .expect("(2) audit row write must succeed");

    let second = store.next_issuance_ordinal().await.expect("(2) second allocation").as_u64();

    assert!(
        second > first,
        "(2) the counter is strictly monotonic across an interleaved audit-row write; \
         first={first}, second={second}"
    );

    let row_count =
        store.issued_certificate_rows().await.expect("(2) read audit rows").len() as u64;
    assert_ne!(
        second, row_count,
        "(2) the ordinal must NOT be a function of the audit table's row count — equality \
         would mean the `len()` derivation is back; second={second}, row_count={row_count}"
    );
}
