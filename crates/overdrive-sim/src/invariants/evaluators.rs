//! Per-invariant evaluator functions for the Phase 1 default catalogue.
//!
//! Each function takes the minimum state it needs to decide whether its
//! invariant holds, and returns an [`InvariantResult`]. The harness
//! ([`super::super::harness::Harness`]) composes these evaluators over
//! the live [`Host`]s it owns; individual evaluators are also unit-
//! testable without booting the full harness — see
//! `crates/overdrive-sim/tests/invariant_evaluators.rs`.
//!
//! # Why evaluators are per-function rather than per-trait
//!
//! A single `trait InvariantEvaluator` would have to carry the union of
//! every evaluator's inputs (an intent store, an observation cluster,
//! an entropy seed, ...). Free functions keep each evaluator's surface
//! narrow to the state it actually reads. The harness dispatches on the
//! [`super::Invariant`] enum and calls the matching function directly —
//! no dynamic dispatch, no erased interface, every caller's input list
//! checked at compile time.
//!
//! # Phase 1 scope
//!
//! The `SingleLeader` evaluator operates against a stubbed leader
//! topology per US-06 Technical Note 3: a simple state machine in the
//! harness designates one host as the leader for each epoch. The
//! Phase 2 step that adds `RaftStore` retires this stub and replaces it
//! with a read against the real Raft leader term. The stub is documented
//! inline in the evaluator body.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::time::Duration;

use overdrive_core::id::NodeId;
use overdrive_core::reconciler::{AnyReconciler, AnyReconcilerView, State, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::intent_store::IntentStore;

use crate::adapters::clock::SimClock;
use crate::adapters::entropy::SimEntropy;
use crate::adapters::observation_store::{
    SimObservationCluster, SimObservationStore, check_lww_convergence,
};
use crate::harness::{InvariantResult, InvariantStatus};

/// Default reporting tick emitted by Phase 1 evaluators. Later phases
/// replace this with the violating tick on failure.
const REPORT_TICK: u64 = 1_000;

/// Default reporting host used when an evaluator produces a
/// cluster-wide verdict (no single host is responsible).
const CLUSTER_HOST: &str = "cluster";

/// Invariant name — owner of the kebab-case mapping lives in
/// [`super::Invariant`]. Helper so every evaluator returns a result
/// pinned to the canonical string.
fn result(
    name: &str,
    status: InvariantStatus,
    host: &str,
    cause: Option<String>,
) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status,
        tick: REPORT_TICK,
        host: host.to_owned(),
        cause,
    }
}

// ---------------------------------------------------------------------------
// SingleLeader
// ---------------------------------------------------------------------------

/// Evaluate `SingleLeader` against a stubbed 3-host topology.
///
/// The stub: `hosts` is the full participant list; `leader` is the one
/// host the stub has elected for the current epoch (or `None` if the
/// stub has not yet converged). The invariant holds iff exactly one
/// host is a leader, and that host is in the participant list.
///
/// Phase 2 replaces this with a read against the real Raft leader
/// term — the stub is exercised only by the in-harness `SingleLeader`
/// evaluation until then (US-06 Technical Note 3).
#[must_use]
pub fn evaluate_single_leader_from_topology(
    hosts: &[NodeId],
    leader: Option<&NodeId>,
) -> InvariantResult {
    let name = "single-leader";
    match leader {
        None => result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some("no host claims leader — stub failed to converge".to_owned()),
        ),
        Some(l) if hosts.contains(l) => result(name, InvariantStatus::Pass, &l.to_string(), None),
        Some(l) => result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!("leader {l} is not in the participant set")),
        ),
    }
}

/// Evaluate `SingleLeader` against an explicit leader set — the shape the
/// Phase 1 unit test uses to plant a "two hosts claim leader" failure.
///
/// Exactly one entry passes; zero or more-than-one fails.
#[must_use]
pub fn evaluate_single_leader_from_leader_set(leaders: &[NodeId]) -> InvariantResult {
    let name = "single-leader";
    match leaders.len() {
        1 => result(name, InvariantStatus::Pass, &leaders[0].to_string(), None),
        0 => result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some("zero hosts claim leader".to_owned()),
        ),
        n => result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!("{n} hosts claim leader simultaneously")),
        ),
    }
}

// ---------------------------------------------------------------------------
// IntentNeverCrossesIntoObservation
// ---------------------------------------------------------------------------

/// Observation-class key prefixes. A key in the `IntentStore` whose
/// bytes start with any of these is a §4 guardrail violation — the
/// write went into the wrong store.
const OBSERVATION_KEY_PREFIXES: &[&[u8]] =
    &[b"alloc_status/", b"node_health/", b"service_backends/"];

/// Evaluate `IntentNeverCrossesIntoObservation` for a single-host pair.
///
/// Inspects the intent store for any observation-class key prefix.
/// The observation side of the check is structural, not runtime — the
/// type system closes the other direction (see
/// `crates/overdrive-core/src/traits/observation_store.rs` for the
/// compile-fail shape that rejects intent-class rows on `write`). For
/// the runtime invariant we scan the intent keyspace and report any
/// observation-prefix match.
pub async fn evaluate_intent_crossing(
    intent: &impl IntentStore,
    _observation: &SimObservationStore,
) -> InvariantResult {
    let name = "intent-never-crosses-into-observation";
    // Snapshot once, then scan every banned prefix against the same
    // view. Scanning inside the loop would pay for N redb roundtrips
    // and — worse — race against concurrent writers under Phase 2
    // multi-writer scenarios, meaning a prefix that only showed up
    // after the first snapshot could slip past the second scan and
    // back before the third. A single atomic snapshot closes the
    // window.
    let snap = match intent.export_snapshot().await {
        Ok(s) => s,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("intent snapshot failed: {err}")),
            );
        }
    };
    for prefix in OBSERVATION_KEY_PREFIXES {
        // `get` on a prefix key directly is not enough — we need to
        // check whether *any* key in intent starts with one of the
        // banned prefixes. `watch` + a short drain would work but is
        // racy for a one-shot probe. Instead, we probe for the exact
        // prefix; a production writer of observation-class data into
        // intent would almost certainly write the prefix verbatim
        // (the failure shape the test exercises) plus some alloc id
        // suffix.
        for (k, _v) in &snap.entries {
            if k.starts_with(prefix) {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!(
                        "intent store holds observation-prefix key: {:?}",
                        String::from_utf8_lossy(k),
                    )),
                );
            }
        }
    }
    result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
}

// ---------------------------------------------------------------------------
// SnapshotRoundtripBitIdentical
// ---------------------------------------------------------------------------

/// Evaluate the snapshot roundtrip invariant against `intent`.
///
/// Drives the step 03-02 logic from within the harness: export,
/// bootstrap a second `LocalIntentStore` from the frame, re-export, and
/// compare bytes.
pub async fn evaluate_snapshot_roundtrip(intent: &impl IntentStore) -> InvariantResult {
    let name = "snapshot-roundtrip-bit-identical";

    let first = match intent.export_snapshot().await {
        Ok(s) => s,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("first export failed: {err}")),
            );
        }
    };

    // Bootstrap a fresh LocalIntentStore from the frame.
    let tmp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("tempdir for roundtrip failed: {err}")),
            );
        }
    };
    let path = tmp.path().join("roundtrip.redb");
    let second_store = match overdrive_store_local::LocalIntentStore::open(&path) {
        Ok(s) => s,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("second open failed: {err}")),
            );
        }
    };
    if let Err(err) = second_store.bootstrap_from(first.clone()).await {
        return result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!("bootstrap_from failed: {err}")),
        );
    }
    let second = match second_store.export_snapshot().await {
        Ok(s) => s,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("second export failed: {err}")),
            );
        }
    };

    if first.bytes() == second.bytes() {
        result(name, InvariantStatus::Pass, "host-0", None)
    } else {
        result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!(
                "roundtrip bytes differ: first={} second={}",
                first.bytes().len(),
                second.bytes().len(),
            )),
        )
    }
}

// ---------------------------------------------------------------------------
// SimObservationLwwConverges
// ---------------------------------------------------------------------------

/// Evaluate the LWW-convergence invariant against `cluster`.
///
/// Drives the step 04-03 `check_lww_convergence` helper from within the
/// harness. The invariant holds when every peer that has observed an
/// alloc holds the same row for it as every other peer that has
/// observed it.
pub async fn evaluate_sim_observation_lww(cluster: &SimObservationCluster) -> InvariantResult {
    use std::str::FromStr;

    use overdrive_core::id::{AllocationId, JobId};
    use overdrive_core::traits::observation_store::{
        AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    };

    let name = "sim-observation-lww-converges";

    // Drive two concurrent writes from different peers to the same
    // allocation. Without this, `check_lww_convergence` asserts
    // trivially on an empty cluster — there is nothing for LWW to
    // resolve. The WS-3 canary bug only manifests when LWW actually
    // has to pick between competing timestamps.
    // `cluster.peers()` is a `HashMap::iter()` under the hood — order
    // varies per-run via Rust's default `RandomState` hasher. Sort
    // explicitly so writer[0] vs writer[1] is pinned and K3 bit-for-bit
    // reproducibility holds across invocations of the same seed.
    let mut peers: Vec<NodeId> = cluster.peers().map(|(id, _)| id.clone()).collect();
    peers.sort();
    if peers.len() >= 2 {
        let alloc_id = match AllocationId::from_str("a1b2c3") {
            Ok(a) => a,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    CLUSTER_HOST,
                    Some(format!("could not construct alloc id: {err}")),
                );
            }
        };
        let job_id = match JobId::from_str("payments") {
            Ok(j) => j,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    CLUSTER_HOST,
                    Some(format!("could not construct job id: {err}")),
                );
            }
        };

        // Two writers, two different logical timestamps, same alloc.
        // Counter 1 < counter 2 so LWW has a definitive winner.
        for (i, writer) in peers.iter().take(2).enumerate() {
            let counter = (i as u64) + 1;
            let state = if i == 0 { AllocState::Pending } else { AllocState::Running };
            let row = AllocStatusRow {
                alloc_id: alloc_id.clone(),
                job_id: job_id.clone(),
                node_id: writer.clone(),
                state,
                updated_at: LogicalTimestamp { counter, writer: writer.clone() },
            };
            let peer = cluster.peer(writer);
            if let Err(err) = peer.write(ObservationRow::AllocStatus(row)).await {
                return result(
                    name,
                    InvariantStatus::Fail,
                    &writer.to_string(),
                    Some(format!("peer write failed: {err}")),
                );
            }
        }
    }

    // Drain the gossip window after the writes so every peer has seen
    // every row. Two advances past the gossip-delay ceiling so FIFOs
    // fully drain even under the cluster's default delay.
    cluster.advance(Duration::from_millis(500)).await;
    cluster.advance(Duration::from_millis(500)).await;

    let report = check_lww_convergence(cluster);
    if report.is_converged() {
        result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
    } else {
        // Report the first peer with a disagreement so the WS-3 failure
        // block names a concrete host. The report is deterministic
        // under the BTreeMap ordering in ConvergenceReport so "first"
        // is stable across runs.
        let host = report
            .peer_views()
            .keys()
            .next()
            .map_or_else(|| CLUSTER_HOST.to_owned(), ToString::to_string);
        result(
            name,
            InvariantStatus::Fail,
            &host,
            Some("peers disagree on an alloc_status row after gossip drain".to_owned()),
        )
    }
}

// ---------------------------------------------------------------------------
// ReplayEquivalentEmptyWorkflow
// ---------------------------------------------------------------------------

/// Evaluate the empty-workflow replay invariant.
///
/// Phase 1's "workflow" is a trivial deterministic transcript — the
/// seed itself, hashed via the same `SimEntropy` instance twice. The
/// invariant holds when the two hashes match. This proves the replay-
/// check machinery; the full workflow runtime is Phase 2+.
#[must_use]
pub fn evaluate_replay_equivalent_empty_workflow(seed: u64) -> InvariantResult {
    let name = "replay-equivalent-empty-workflow";

    // Two SimEntropy instances seeded identically are the Phase 1
    // stand-in for a "run the workflow twice" transcript. Phase 2
    // replaces this with an actual workflow journal replay.
    let first = capture_transcript(seed);
    let second = capture_transcript(seed);

    if first == second {
        result(name, InvariantStatus::Pass, "host-0", None)
    } else {
        result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some("empty-workflow transcript differs across replay".to_owned()),
        )
    }
}

/// Produce a deterministic transcript from a seed. The length is fixed
/// so that a mutation that returns an empty Vec is caught by the two
/// above tests (empty == empty would be trivially equal).
fn capture_transcript(seed: u64) -> Vec<u64> {
    let entropy = SimEntropy::new(seed);
    (0..16).map(|_| entropy.u64()).collect()
}

// ---------------------------------------------------------------------------
// EntropyDeterminismUnderReseed
// ---------------------------------------------------------------------------

/// Evaluate the entropy determinism invariant for a single seed —
/// two `SimEntropy` instances seeded with `seed` produce identical
/// draw sequences.
#[must_use]
pub fn evaluate_entropy_determinism(seed: u64) -> InvariantResult {
    let a = SimEntropy::new(seed);
    let b = SimEntropy::new(seed);
    evaluate_entropy_determinism_against(&a, &b)
}

/// Evaluate the entropy determinism invariant against two instances.
///
/// Used by the planted-failure unit test that passes differently-seeded
/// entropies and asserts the evaluator catches the disagreement.
#[must_use]
pub fn evaluate_entropy_determinism_against(a: &SimEntropy, b: &SimEntropy) -> InvariantResult {
    /// Number of draws compared across the two entropy instances.
    /// Larger than a handful so a mutation that returns a constant
    /// first draw cannot hide a full-stream divergence.
    const DRAWS: usize = 1_024;

    let name = "entropy-determinism-under-reseed";
    for i in 0..DRAWS {
        let x = a.u64();
        let y = b.u64();
        if x != y {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("SimEntropy diverges at draw {i}: {x:#x} vs {y:#x}")),
            );
        }
    }
    result(name, InvariantStatus::Pass, "host-0", None)
}

// ---------------------------------------------------------------------------
// AtLeastOneReconcilerRegistered (step 04-05)
// ---------------------------------------------------------------------------

/// Evaluate `AtLeastOneReconcilerRegistered` — the registry is never
/// empty after boot.
///
/// Per whitepaper §18 and ADR-0013 §2, a control-plane boot with zero
/// registered reconcilers is a silent-failure shape: the cluster sees
/// no convergence pressure and the operator sees no error. Phase 1
/// registers `noop-heartbeat` as proof-of-life; this invariant catches
/// any future regression that skips registration.
///
/// The harness passes the count of registered reconcilers it composed;
/// the evaluator asserts the count is non-zero. `count` rather than a
/// trait-object dependency on `overdrive-control-plane` keeps the sim
/// crate a leaf adapter.
#[must_use]
pub fn evaluate_at_least_one_reconciler_registered(registered_count: usize) -> InvariantResult {
    let name = "at-least-one-reconciler-registered";
    if registered_count >= 1 {
        result(name, InvariantStatus::Pass, "host-0", None)
    } else {
        result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some("reconciler registry is empty after boot".to_owned()),
        )
    }
}

// ---------------------------------------------------------------------------
// DuplicateEvaluationsCollapse (step 04-05)
// ---------------------------------------------------------------------------

/// Observable broker counters the `DuplicateEvaluationsCollapse`
/// evaluator inspects.
///
/// Mirrors the shape of
/// `overdrive_control_plane::eval_broker::BrokerCounters` but is
/// redefined locally so the sim crate does not take a cyclic dependency
/// on `overdrive-control-plane` (which already depends on
/// `overdrive-sim` via `observation_wiring`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokerCountersSnapshot {
    /// Number of evaluations currently pending dispatch.
    pub queued: u64,
    /// Cumulative count of superseded evaluations.
    pub cancelled: u64,
    /// Cumulative count of dispatched evaluations.
    pub dispatched: u64,
}

/// Evaluate `DuplicateEvaluationsCollapse`.
///
/// N (≥3) concurrent evaluations at the same
/// `(ReconcilerName, TargetResource)` key collapse to exactly one
/// dispatched invocation and `N - 1` cancellations, per ADR-0013 §8
/// storm-proofing.
///
/// The harness is responsible for driving the submit-N-at-same-key +
/// drain sequence; the evaluator inspects the resulting counter
/// snapshot. Passing requires `dispatched == 1`, `cancelled == n - 1`,
/// and `queued == 0` (drain completed).
#[must_use]
pub fn evaluate_duplicate_evaluations_collapse(
    n_submitted: u64,
    counters: BrokerCountersSnapshot,
) -> InvariantResult {
    let name = "duplicate-evaluations-collapse";

    if n_submitted < 3 {
        return result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!(
                "harness submitted {n_submitted} evaluations — invariant requires at least 3",
            )),
        );
    }

    let expected_cancelled = n_submitted - 1;
    if counters.dispatched == 1 && counters.cancelled == expected_cancelled && counters.queued == 0
    {
        result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
    } else {
        result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!(
                "expected dispatched=1 cancelled={expected_cancelled} queued=0 after {n_submitted} same-key submits; got {counters:?}",
            )),
        )
    }
}

// ---------------------------------------------------------------------------
// ReconcilerIsPure (step 04-05)
// ---------------------------------------------------------------------------

/// Evaluate `ReconcilerIsPure` — twin invocation of `reconciler.reconcile`
/// with identical `(desired, actual, view, tick)` inputs produces
/// bit-identical `(Vec<Action>, NextView)` tuples.
///
/// This is the runtime witness of the ADR-0013 §2 purity contract. A
/// reconciler that smuggles non-determinism (wall-clock read,
/// `rand::thread_rng`, internal `RefCell` counter, ...) fails here.
/// Phase 1 runs this against the `noop-heartbeat` reconciler, which
/// always returns `vec![Action::Noop]` — a deterministic baseline that
/// proves the machinery is live. The optional `canary-bug` gate in
/// this crate exposes a deliberately non-deterministic reconciler to
/// prove this evaluator actually catches divergences.
///
/// # Time injection
///
/// The `TickContext` passed to `reconcile` pulls its `now` from the
/// caller-supplied `SimClock`, NOT from `std::time::Instant::now()`.
/// Under DST, `SimClock::now()` is seed-deterministic — two harness
/// runs at the same seed see the same `now`, and the twin invocation
/// within a single run sees one `now` shared across both calls (the
/// same `TickContext` reference is passed to each). This preserves
/// the ADR-0013 §2c "time is input state, injected once per tick"
/// contract even at the sim-layer callsite.
#[must_use]
pub fn evaluate_reconciler_is_pure(
    reconciler: &AnyReconciler,
    clock: &SimClock,
) -> InvariantResult {
    /// Monotonic tick counter — the evaluator runs once per harness
    /// pass (not inside a real reconcile loop), so a fixed zero is
    /// the right shape. The field exists to give reconcilers a
    /// deterministic tie-breaker that does not depend on wall-clock
    /// granularity; the harness's single-shot nature means there is
    /// no per-call progression to model.
    const TICK: u64 = 0;
    /// Per-evaluation reconcile budget. No injected production
    /// budget yet (§14 right-sizing will provide one); a 1-second
    /// literal matches the 04-07 test-side `TickContext`
    /// construction.
    const BUDGET: Duration = Duration::from_secs(1);

    let name = "reconciler-is-pure";

    // Twin invocation with identical inputs per ADR-0013 §2 / §2c. ONE
    // `TickContext` is constructed and passed to BOTH calls so time is
    // a shared input, not a per-call side channel. The full §18 purity
    // semantics (pre-hydrated view + next-view tuple return) are
    // exercised here — `(actions, next_view)` are asserted as paired
    // but separate bit-identical comparisons so a mutation that drops
    // either side is caught.
    let desired = State;
    let actual = State;
    let view = AnyReconcilerView::Unit;
    let now = clock.now();
    let tick = TickContext { now, tick: TICK, deadline: now + BUDGET };

    let (actions_a, next_view_a) = reconciler.reconcile(&desired, &actual, &view, &tick);
    let (actions_b, next_view_b) = reconciler.reconcile(&desired, &actual, &view, &tick);

    if actions_a == actions_b && next_view_a == next_view_b {
        result(name, InvariantStatus::Pass, "host-0", None)
    } else {
        result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!(
                "reconciler {} diverged under twin invocation: \
                 first=(actions={actions_a:?}, next_view={next_view_a:?}) \
                 second=(actions={actions_b:?}, next_view={next_view_b:?})",
                reconciler.name(),
            )),
        )
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    //! Library-level witnesses for each evaluator. Paired with the
    //! integration tests under `crates/overdrive-sim/tests/invariant_evaluators.rs`
    //! — the integration tests prove the full contract, these tests
    //! kill the low-hanging mutations in this file.

    use super::*;
    use std::str::FromStr;

    fn n(s: &str) -> NodeId {
        NodeId::from_str(s).expect("valid node id")
    }

    #[test]
    fn topology_with_single_leader_passes_and_names_that_host() {
        let hosts = vec![n("host-0"), n("host-1"), n("host-2")];
        let r = evaluate_single_leader_from_topology(&hosts, Some(&n("host-1")));
        assert_eq!(r.status, InvariantStatus::Pass);
        assert_eq!(r.host, "host-1");
    }

    #[test]
    fn topology_with_leader_outside_participants_fails() {
        let hosts = vec![n("host-0"), n("host-1")];
        let r = evaluate_single_leader_from_topology(&hosts, Some(&n("intruder")));
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("not in the participant set")));
    }

    #[test]
    fn leader_set_with_exactly_one_entry_passes() {
        let leaders = vec![n("host-0")];
        let r = evaluate_single_leader_from_leader_set(&leaders);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    #[test]
    fn empty_leader_set_fails() {
        let leaders: Vec<NodeId> = Vec::new();
        let r = evaluate_single_leader_from_leader_set(&leaders);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    #[test]
    fn two_leaders_fail_with_count_in_cause() {
        let leaders = vec![n("host-0"), n("host-1")];
        let r = evaluate_single_leader_from_leader_set(&leaders);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains('2')));
    }

    #[test]
    fn entropy_determinism_reports_pass_on_equal_seeds() {
        assert_eq!(evaluate_entropy_determinism(7).status, InvariantStatus::Pass);
    }

    #[test]
    fn entropy_determinism_reports_fail_on_divergent_streams() {
        let a = SimEntropy::new(1);
        let b = SimEntropy::new(2);
        assert_eq!(evaluate_entropy_determinism_against(&a, &b).status, InvariantStatus::Fail);
    }

    #[test]
    fn empty_workflow_transcript_is_non_empty_and_deterministic() {
        let t1 = capture_transcript(42);
        let t2 = capture_transcript(42);
        assert_eq!(t1, t2);
        assert_eq!(t1.len(), 16, "transcript length is pinned so an empty-Vec mutation fails");
    }

    #[test]
    fn empty_workflow_transcript_differs_across_seeds() {
        assert_ne!(capture_transcript(1), capture_transcript(2));
    }

    #[test]
    fn replay_equivalent_empty_workflow_passes_on_deterministic_seed() {
        let r = evaluate_replay_equivalent_empty_workflow(42);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    // -----------------------------------------------------------------
    // Step 04-05 — reconciler-primitive invariant witnesses
    // -----------------------------------------------------------------

    #[test]
    fn at_least_one_reconciler_passes_on_nonzero_count() {
        assert_eq!(evaluate_at_least_one_reconciler_registered(1).status, InvariantStatus::Pass,);
        assert_eq!(evaluate_at_least_one_reconciler_registered(42).status, InvariantStatus::Pass,);
    }

    #[test]
    fn at_least_one_reconciler_fails_on_empty_registry() {
        let r = evaluate_at_least_one_reconciler_registered(0);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("empty")));
    }

    #[test]
    fn duplicate_evaluations_collapse_passes_on_clean_3_way_collapse() {
        let counters = BrokerCountersSnapshot { queued: 0, cancelled: 2, dispatched: 1 };
        assert_eq!(
            evaluate_duplicate_evaluations_collapse(3, counters).status,
            InvariantStatus::Pass,
        );
    }

    #[test]
    fn duplicate_evaluations_collapse_fails_when_dispatched_not_one() {
        // dispatched == 2 means the second submit didn't supersede the
        // first — key-collapse is broken.
        let counters = BrokerCountersSnapshot { queued: 0, cancelled: 1, dispatched: 2 };
        let r = evaluate_duplicate_evaluations_collapse(3, counters);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    #[test]
    fn duplicate_evaluations_collapse_fails_when_cancelled_count_wrong() {
        // N=3 should yield cancelled=2; cancelled=0 means nothing was
        // actually superseded.
        let counters = BrokerCountersSnapshot { queued: 0, cancelled: 0, dispatched: 1 };
        let r = evaluate_duplicate_evaluations_collapse(3, counters);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    #[test]
    fn duplicate_evaluations_collapse_fails_when_queued_not_drained() {
        // queued > 0 means the drain half of the sequence never ran.
        let counters = BrokerCountersSnapshot { queued: 1, cancelled: 2, dispatched: 0 };
        let r = evaluate_duplicate_evaluations_collapse(3, counters);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    #[test]
    fn duplicate_evaluations_collapse_fails_when_n_below_three() {
        // Invariant requires at least 3 submitted to be meaningful.
        let counters = BrokerCountersSnapshot { queued: 0, cancelled: 1, dispatched: 1 };
        let r = evaluate_duplicate_evaluations_collapse(2, counters);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("at least 3")));
    }

    #[test]
    fn reconciler_is_pure_passes_for_deterministic_reconciler() {
        use overdrive_core::reconciler::{AnyReconciler, NoopHeartbeat};

        // The deterministic witness is the real `NoopHeartbeat` —
        // wrapping it in `AnyReconciler::NoopHeartbeat` exercises the
        // exact enum-dispatch path the evaluator runs in production.
        let r = AnyReconciler::NoopHeartbeat(NoopHeartbeat::canonical());
        let clock = SimClock::new();
        assert_eq!(evaluate_reconciler_is_pure(&r, &clock).status, InvariantStatus::Pass);
    }

    /// The non-deterministic witness requires the `canary-bug` feature.
    ///
    /// Under the `AnyReconciler` enum-dispatch model (04-07), inhabiting
    /// the enum is restricted to first-party variants — a one-off
    /// `Flappy` struct from a test module cannot be dispatched through
    /// `AnyReconciler` without modifying the enum. The `canary-bug`
    /// feature exists precisely to ship a flip-on-call variant in-tree:
    /// `HarnessNoopHeartbeat` under `#[cfg(feature = "canary-bug")]`
    /// alternates one/two `Noop`s per call, which the twin-invocation
    /// check must flag.
    ///
    /// The default unit lane does NOT exercise this branch (the feature
    /// is off). End-to-end coverage that this evaluator actually flags
    /// a real divergence is provided by
    /// `xtask/tests/acceptance/dst_canary_red_run.rs` and the sim
    /// acceptance suite run under `--features
    /// overdrive-sim/canary-bug`, both of which drive a full harness
    /// run with the canary variant in the registry.
    #[cfg(feature = "canary-bug")]
    #[test]
    fn reconciler_is_pure_fails_for_non_deterministic_reconciler() {
        use overdrive_core::reconciler::{AnyReconciler, HarnessNoopHeartbeat};

        let r = AnyReconciler::HarnessNoopHeartbeat(HarnessNoopHeartbeat::canonical());
        let clock = SimClock::new();
        let result = evaluate_reconciler_is_pure(&r, &clock);
        assert_eq!(result.status, InvariantStatus::Fail);
        assert!(result.cause.as_ref().is_some_and(|c| c.contains("diverged")));
    }
}
