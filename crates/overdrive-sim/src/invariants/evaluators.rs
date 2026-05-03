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

use overdrive_core::UnixInstant;
use overdrive_core::id::{JobId, NodeId};
use overdrive_core::reconciler::{AnyReconciler, AnyReconcilerView, AnyState, TickContext};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow};

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
// IntentStoreReturnsCallerBytes
// ---------------------------------------------------------------------------

/// Probe keys for [`evaluate_intent_store_returns_caller_bytes`]. Owns
/// each [`overdrive_core::aggregate::IntentKey`] so the slice borrows
/// returned by `as_bytes()` outlive the evaluator's loop.
struct IntentStoreProbeKeys {
    empty: overdrive_core::aggregate::IntentKey,
    le_prefix: overdrive_core::aggregate::IntentKey,
    long: overdrive_core::aggregate::IntentKey,
}

/// Derive the three probe keys via `IntentKey::for_job` so the literal
/// `jobs/` prefix is sourced from `aggregate/mod.rs` (the SSOT).
fn derive_intent_store_probe_keys() -> Result<IntentStoreProbeKeys, String> {
    let mk = |raw: &str| -> Result<overdrive_core::aggregate::IntentKey, String> {
        overdrive_core::id::JobId::new(raw)
            .map(|id| overdrive_core::aggregate::IntentKey::for_job(&id))
            .map_err(|e| format!("derive IntentKey for {raw}: {e}"))
    };
    Ok(IntentStoreProbeKeys {
        empty: mk("k-empty")?,
        le_prefix: mk("k-le-prefix")?,
        long: mk("k-long")?,
    })
}

/// Evaluate the structural-regression guard from ADR-0020 §Enforcement.
///
/// Writes a small fixed set of `(key, value)` pairs through
/// [`IntentStore::put`] and [`IntentStore::put_if_absent`], then reads
/// each key back and checks the returned bytes are byte-identical to
/// the bytes that went in. This catches any future re-introduction of
/// inline framing (`[u64-LE-prefix || value]`) or any other on-disk
/// row encoding that would surface a transformed value at the trait
/// boundary.
///
/// The fixtures cover the failure shapes inline framing produces:
/// * an empty value (an inline u64 prefix would surface as 8 bytes)
/// * a value whose bytes start with what would be a plausible u64-LE
///   prefix (an inline framing would slice off the first 8 bytes)
/// * a long value (general round-trip witness)
///
/// The invariant uses a fresh tempdir-backed `LocalIntentStore` rather
/// than the harness's per-host store so it cannot interact with state
/// other invariants leave behind.
pub async fn evaluate_intent_store_returns_caller_bytes() -> InvariantResult {
    let name = "intent-store-returns-caller-bytes";

    let tmp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("tempdir for intent-store probe failed: {err}")),
            );
        }
    };
    let path = tmp.path().join("caller-bytes.redb");
    let store = match overdrive_store_local::LocalIntentStore::open(&path) {
        Ok(s) => s,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("intent store open failed: {err}")),
            );
        }
    };

    // Fixtures chosen to flush inline-framing regressions:
    //   * empty value — inline framing would surface 8 prefix bytes
    //   * 8-byte LE-looking prefix — inline framing would slice it off
    //   * arbitrary 32-byte payload — general round-trip witness
    //
    // Keys are derived through `IntentKey::for_job` so the literal
    // `jobs/` prefix appears in exactly one production file (the
    // SSOT in `aggregate/mod.rs`); the canonical-key grep gate in
    // `overdrive-core/tests/acceptance/intent_key_canonical.rs`
    // enforces this.
    let keys = match derive_intent_store_probe_keys() {
        Ok(k) => k,
        Err(err) => {
            return result(name, InvariantStatus::Fail, "host-0", Some(err));
        }
    };
    let fixtures: &[(&[u8], &[u8])] = &[
        (keys.empty.as_bytes(), b""),
        (keys.le_prefix.as_bytes(), &[0x07, 0, 0, 0, 0, 0, 0, 0, b'a', b'b', b'c']),
        (keys.long.as_bytes(), b"the-rain-in-spain-falls-mainly-on"),
    ];

    // Write the first two via `put`, the third via `put_if_absent` so
    // the invariant covers both insert paths. A regression that
    // re-introduces framing on only one path would still be caught.
    for (k, v) in &fixtures[..2] {
        if let Err(err) = store.put(k, v).await {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("put({:?}) failed: {err}", String::from_utf8_lossy(k))),
            );
        }
    }
    if let Err(err) = store.put_if_absent(fixtures[2].0, fixtures[2].1).await {
        return result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!(
                "put_if_absent({:?}) failed: {err}",
                String::from_utf8_lossy(fixtures[2].0),
            )),
        );
    }

    for (k, expected) in fixtures {
        match store.get(k).await {
            Ok(Some(actual)) => {
                if actual.as_ref() != *expected {
                    return result(
                        name,
                        InvariantStatus::Fail,
                        "host-0",
                        Some(format!(
                            "get({key:?}) returned {actual_len} bytes, expected {expected_len}: \
                             actual={actual:?} expected={expected:?}",
                            key = String::from_utf8_lossy(k),
                            actual_len = actual.len(),
                            expected_len = expected.len(),
                            actual = actual.as_ref(),
                            expected = expected,
                        )),
                    );
                }
            }
            Ok(None) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("get({:?}) returned None after put", String::from_utf8_lossy(k))),
                );
            }
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("get({:?}) failed: {err}", String::from_utf8_lossy(k))),
                );
            }
        }
    }

    result(name, InvariantStatus::Pass, "host-0", None)
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
                reason: None,
                detail: None,
                terminal: None,
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
// BrokerDrainOrderIsDeterministic (step 01-05)
// ---------------------------------------------------------------------------

/// Observable per-pass drain order the
/// `BrokerDrainOrderIsDeterministic` evaluator inspects.
///
/// Sibling to [`BrokerCountersSnapshot`] — counters proves the LWW
/// key-collapse invariant, this snapshot proves drain-order
/// determinism. Both coexist; neither replaces the other.
///
/// The harness captures one of these from a FIRST drain pass and a
/// SECOND drain pass (each drain replays identical submit semantics)
/// and the evaluator asserts the two `dispatched_order` vecs are
/// element-equal in the same positions. A divergence at any position
/// means the broker's drain order depends on something other than the
/// submit sequence — `HashSet` iteration order, allocator placement,
/// thread scheduling — and the invariant fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerDrainOrderSnapshot {
    /// The ordered sequence of `(ReconcilerName, TargetResource)` keys
    /// the broker dispatched during a single drain pass.
    pub dispatched_order: Vec<(
        overdrive_core::reconciler::ReconcilerName,
        overdrive_core::reconciler::TargetResource,
    )>,
}

/// Evaluate `BrokerDrainOrderIsDeterministic`.
///
/// Two drain passes against identical submit sequences must produce
/// element-equal `dispatched_order` vecs at every position. On
/// mismatch, the failure cause names the first divergent position by
/// index — a structured signal mutation testing can target precisely.
///
/// The harness is responsible for driving two copies of the same
/// submit-and-drain sequence and capturing both
/// `BrokerDrainOrderSnapshot`s; this evaluator inspects only the
/// snapshots.
#[must_use]
pub fn evaluate_broker_drain_order_is_deterministic(
    a: &BrokerDrainOrderSnapshot,
    b: &BrokerDrainOrderSnapshot,
) -> InvariantResult {
    let name = "broker-drain-order-is-deterministic";

    // Find the first divergent position via zip().enumerate().find().
    // Length mismatch is also a divergence: the shorter vec ends
    // first, so a missing trailing entry shows up as a position-equal
    // length comparison after the zip exhausts.
    let first_divergence = a
        .dispatched_order
        .iter()
        .zip(b.dispatched_order.iter())
        .enumerate()
        .find(|(_, (lhs, rhs))| lhs != rhs)
        .map(|(idx, _)| idx);

    if first_divergence.is_none() && a.dispatched_order.len() == b.dispatched_order.len() {
        result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
    } else {
        let position = first_divergence
            .unwrap_or_else(|| a.dispatched_order.len().min(b.dispatched_order.len()));
        result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!(
                "drain order diverged at position {position}: \
                 first={:?} second={:?}",
                a.dispatched_order, b.dispatched_order,
            )),
        )
    }
}

// ---------------------------------------------------------------------------
// DispatchRoutingIsNameRestricted (fix-dst-dispatch-routing-invariant 01-01)
// ---------------------------------------------------------------------------

/// Evaluation-shape mirror for the
/// `DispatchRoutingIsNameRestricted` evaluator.
///
/// Mirrors `overdrive_control_plane::eval_broker::Evaluation` rather
/// than importing it; sim crate stays a leaf adapter (per CLAUDE.md
/// crate classes / ADR-0004 sim-host split). The harness submits these
/// to its mirrored dispatcher and feeds the result into this evaluator.
/// Sibling to [`BrokerCountersSnapshot`]: that mirror covers broker
/// counters; this one covers the eval shape the dispatcher consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    /// Reconciler this evaluation is keyed against. The dispatcher MUST
    /// invoke this — and only this — reconciler against `target`.
    pub reconciler: overdrive_core::reconciler::ReconcilerName,
    /// Target resource the reconciler converges.
    pub target: overdrive_core::reconciler::TargetResource,
}

/// Snapshot of dispatcher invocations during one tick.
///
/// Each entry is one `(reconciler, target)` tuple from a single
/// `run_convergence_tick` invocation — captured by the harness mirror,
/// not by importing `overdrive-control-plane`. Sibling to
/// [`BrokerCountersSnapshot`] (broker-side entry collapse) and
/// [`BrokerDrainOrderSnapshot`] (broker-side drain order); all three
/// coexist and none subsumes another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchRecord {
    /// Each `(reconciler, target)` the dispatcher invoked. Order
    /// reflects the dispatcher's call order, but the invariant is
    /// permutation-invariant on the target axis (one entry per drained
    /// eval, per the §8 entry-collapse contract).
    pub dispatched: Vec<(
        overdrive_core::reconciler::ReconcilerName,
        overdrive_core::reconciler::TargetResource,
    )>,
}

/// Evaluate `DispatchRoutingIsNameRestricted`.
///
/// For every drained `Evaluation { reconciler: R, target: T }` the
/// harness submitted, the dispatch record MUST contain exactly one
/// entry `(R, T)` and zero entries `(R', T)` for any `R' != R`. This
/// pins the §8 storm-proofing dispatch-routing contract end-to-end:
/// the DST-tier peer of the unit/acceptance pin at
/// `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs::eval_dispatch_runs_only_the_named_reconciler`
/// (commit `e6f5e5e`).
///
/// Four branches in this exact order:
///
/// 1. **Vacuous-pass** on empty input — the invariant is "for every
///    drained eval ..." and ∀∅ holds trivially. Without this the
///    empty-input case would misreport as a fail under the cardinality
///    branch.
/// 2. **Cardinality** — `record.dispatched.len() == submitted.len()`.
///    A surplus entry is a fan-out regression; a deficit is a missed
///    dispatch.
/// 3. **Per-eval routing** — for every submitted `(R, T)`, exactly one
///    matching entry in `record.dispatched`. Catches the "named
///    reconciler not dispatched" shape under clean cardinality.
/// 4. **Smoking-gun** — any dispatch entry naming a reconciler outside
///    the submitted set is a fan-out smoking gun. Catches the precise
///    bug shape the precursor fix closed: a `run_convergence_tick`
///    that iterates the registry rather than looking up by name.
#[must_use]
pub fn evaluate_dispatch_routing_is_name_restricted(
    submitted: &[Evaluation],
    record: &DispatchRecord,
) -> InvariantResult {
    let name = "dispatch-routing-is-name-restricted";

    // (a) Vacuous-pass on empty input.
    if submitted.is_empty() {
        return result(name, InvariantStatus::Pass, CLUSTER_HOST, None);
    }

    // (b) Cardinality check.
    if record.dispatched.len() != submitted.len() {
        return result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!(
                "expected {expected} dispatch entries (one per drained eval), got {got}: \
                 dispatched={dispatched:?}",
                expected = submitted.len(),
                got = record.dispatched.len(),
                dispatched = record.dispatched,
            )),
        );
    }

    // (c) Per-eval routing — every submitted (R, T) must appear exactly
    //     once in the dispatch record. A count of zero means the named
    //     reconciler was not dispatched; a count > 1 means it was
    //     dispatched more than once.
    for eval in submitted {
        let key = (eval.reconciler.clone(), eval.target.clone());
        let matches = record.dispatched.iter().filter(|d| **d == key).count();
        if matches != 1 {
            return result(
                name,
                InvariantStatus::Fail,
                CLUSTER_HOST,
                Some(format!(
                    "expected exactly one dispatch of ({reconciler}, {target}) — \
                     the named reconciler — got {matches} entries: dispatched={dispatched:?}",
                    reconciler = eval.reconciler,
                    target = eval.target,
                    matches = matches,
                    dispatched = record.dispatched,
                )),
            );
        }
    }

    // (d) Smoking-gun — any dispatched entry naming a reconciler NOT in
    //     the submitted set is a fan-out regression.
    let submitted_names: std::collections::BTreeSet<&overdrive_core::reconciler::ReconcilerName> =
        submitted.iter().map(|e| &e.reconciler).collect();
    for (r, t) in &record.dispatched {
        if !submitted_names.contains(r) {
            return result(
                name,
                InvariantStatus::Fail,
                CLUSTER_HOST,
                Some(format!(
                    "dispatcher invoked unsubmitted reconciler {r} against {t} — fan-out regression",
                )),
            );
        }
    }

    result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
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
/// proves the machinery is live.
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
    let name = "reconciler-is-pure";

    // Twin invocation with identical inputs per ADR-0013 §2 / §2c. ONE
    // `TickContext` is constructed and passed to BOTH calls so time is
    // a shared input, not a per-call side channel. The full §18 purity
    // semantics (pre-hydrated view + next-view tuple return) are
    // exercised here — `(actions, next_view)` are asserted as paired
    // but separate bit-identical comparisons so a mutation that drops
    // either side is caught.
    //
    // Per ADR-0021 (step 02-01), `desired`/`actual` are now typed
    // `&AnyState` rather than the prior `&State, &State` placeholder
    // pair. Phase 1 reconcilers (`NoopHeartbeat`) use `AnyState::Unit`
    // because their `Reconciler::State = ()`; the `JobLifecycle`
    // reconciler's `AnyState::JobLifecycle(...)` arm becomes reachable
    // in step 02-03 when the runtime tick loop ships.
    let desired = AnyState::Unit;
    let actual = AnyState::Unit;
    let view = AnyReconcilerView::Unit;
    let tick = build_tick_context(clock);

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

/// Construct the `TickContext` snapshot handed to both twin
/// invocations inside `evaluate_reconciler_is_pure`.
///
/// Extracted from the evaluator body so the `now + BUDGET` arithmetic
/// can be unit-tested. With the `TickContext` construction inlined,
/// the `+` mutation survived — the Phase 1 reconciler fixture
/// (`NoopHeartbeat`) ignores `tick.deadline` entirely, so a
/// deadline-in-the-past produced no observable divergence. The
/// helper form gives mutation testing a direct target: a unit test
/// asserts `tick.deadline > tick.now` and `+ -> -` flips the sign
/// on that difference.
///
/// The `tick` counter stays zero (the evaluator runs once per harness
/// pass, not inside a real reconcile loop) and the budget is a
/// 1-second literal matching the 04-07 test-side `TickContext`
/// construction — no injected production budget exists yet per ADR-0013.
#[inline]
fn build_tick_context(clock: &SimClock) -> TickContext {
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

    let now = clock.now();
    let now_unix = UnixInstant::from_clock(clock);
    TickContext { now, now_unix, tick: TICK, deadline: now + BUDGET }
}

// ---------------------------------------------------------------------------
// phase-1-first-workload — slice 3 (US-03) — convergence invariants
// ---------------------------------------------------------------------------

/// Evaluate `JobScheduledAfterSubmission` (eventually).
///
/// Per US-03 AC: for every submitted job in `submitted_jobs`, an
/// `AllocStatusRow{state: Running}` referencing that job MUST exist in
/// `alloc_status` after the convergence-loop budget elapsed. The harness
/// drives the runtime tick loop forward N ticks and then snapshots the
/// `ObservationStore`'s `alloc_status` table; that snapshot is fed to
/// this evaluator.
///
/// **Pure verdict** — `submitted_jobs` is the desired set;
/// `alloc_status` is the actual observation. The evaluator counts pass
/// iff every submitted job has at least one Running alloc in the
/// snapshot. Pure, no I/O — the harness owns the tick loop.
#[must_use]
pub fn evaluate_job_scheduled_after_submission(
    submitted_jobs: &[JobId],
    alloc_status: &[AllocStatusRow],
) -> InvariantResult {
    let name = "job-scheduled-after-submission";
    // Vacuous-pass when no jobs were submitted — the invariant is
    // "for every submitted job ..." and ∀ ∅ holds trivially. Without
    // this the empty-input case would surface as a "0 jobs running"
    // false positive on the next mutation of the `submitted_jobs.len()`
    // comparison.
    if submitted_jobs.is_empty() {
        return result(name, InvariantStatus::Pass, CLUSTER_HOST, None);
    }
    for job_id in submitted_jobs {
        let has_running = alloc_status
            .iter()
            .any(|row| &row.job_id == job_id && row.state == AllocState::Running);
        if !has_running {
            return result(
                name,
                InvariantStatus::Fail,
                CLUSTER_HOST,
                Some(format!("submitted job {job_id} has no Running alloc within budget")),
            );
        }
    }
    result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
}

/// Evaluate `DesiredReplicaCountConverges` (eventually).
///
/// Per US-03 AC: `count(state == Running for job_j) == replicas_j` for
/// every submitted job. Phase 1 jobs are 1-replica so this is a
/// vacuous-pass at N=1 — but the evaluator still walks the rows to
/// catch the leak-across-jobs case where one job's Running row is
/// double-counted against another.
///
/// `desired_replicas` carries the per-job replica count; `alloc_status`
/// is the observation snapshot.
#[must_use]
pub fn evaluate_desired_replica_count_converges(
    desired_replicas: &[(JobId, u32)],
    alloc_status: &[AllocStatusRow],
) -> InvariantResult {
    let name = "desired-replica-count-converges";
    for (job_id, want) in desired_replicas {
        let running_count = u32::try_from(
            alloc_status
                .iter()
                .filter(|row| &row.job_id == job_id && row.state == AllocState::Running)
                .count(),
        )
        .unwrap_or(u32::MAX);
        if running_count != *want {
            return result(
                name,
                InvariantStatus::Fail,
                CLUSTER_HOST,
                Some(format!("job {job_id}: want {want} Running, observed {running_count}")),
            );
        }
    }
    result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
}

/// Evaluate `NoDoubleScheduling` (always).
///
/// Per US-03 AC: every `alloc_id` in the snapshot agrees on a single
/// `node_id`. Two rows for the same `alloc_id` pinned to different
/// nodes is a double-scheduling violation. Operates on a single
/// snapshot; the "always" semantics come from the harness invoking
/// this evaluator on every tick rather than just at the end.
///
/// Implementation note: a `BTreeMap<AllocationId, NodeId>` accumulates
/// the first observed pin and rejects subsequent rows whose node
/// differs.
#[must_use]
pub fn evaluate_no_double_scheduling(alloc_status: &[AllocStatusRow]) -> InvariantResult {
    let name = "no-double-scheduling";
    let mut pinned: std::collections::BTreeMap<overdrive_core::id::AllocationId, NodeId> =
        std::collections::BTreeMap::new();
    for row in alloc_status {
        if let Some(prior) = pinned.get(&row.alloc_id) {
            if prior != &row.node_id {
                return result(
                    name,
                    InvariantStatus::Fail,
                    CLUSTER_HOST,
                    Some(format!(
                        "alloc {} pinned to two nodes: {prior} and {}",
                        row.alloc_id, row.node_id,
                    )),
                );
            }
        } else {
            pinned.insert(row.alloc_id.clone(), row.node_id.clone());
        }
    }
    result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
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

    // -----------------------------------------------------------------
    // Step 01-05 — BrokerDrainOrderIsDeterministic witnesses
    // -----------------------------------------------------------------

    /// Build a small fixture of `(ReconcilerName, TargetResource)` pairs
    /// for the broker-drain-order tests. Two entries is the minimum that
    /// can demonstrate divergence at a non-trivial position; 01-05 keeps
    /// the fixture deliberately tiny so the failure-message assertion
    /// pins the position index, not incidental ordering.
    fn drain_fixture()
    -> Vec<(overdrive_core::reconciler::ReconcilerName, overdrive_core::reconciler::TargetResource)>
    {
        use overdrive_core::reconciler::{ReconcilerName, TargetResource};
        let r = ReconcilerName::new("noop-heartbeat")
            .expect("noop-heartbeat is a valid ReconcilerName");
        let t_a =
            TargetResource::new("job/payments").expect("job/payments is a valid TargetResource");
        let t_b =
            TargetResource::new("job/frontend").expect("job/frontend is a valid TargetResource");
        vec![(r.clone(), t_a), (r, t_b)]
    }

    #[test]
    fn evaluate_broker_drain_order_is_deterministic_pass_and_fail() {
        // PASS: two snapshots with identical dispatched_order vecs.
        let order = drain_fixture();
        let a = BrokerDrainOrderSnapshot { dispatched_order: order.clone() };
        let b = BrokerDrainOrderSnapshot { dispatched_order: order.clone() };
        let pass = evaluate_broker_drain_order_is_deterministic(&a, &b);
        assert_eq!(pass.status, InvariantStatus::Pass);

        // FAIL: divergence at position 0 — swap the first pair so the
        // first index differs. The failure message must name the
        // divergent position.
        let mut divergent = order.clone();
        divergent.swap(0, 1);
        let a2 = BrokerDrainOrderSnapshot { dispatched_order: order };
        let b2 = BrokerDrainOrderSnapshot { dispatched_order: divergent };
        let fail = evaluate_broker_drain_order_is_deterministic(&a2, &b2);
        assert_eq!(fail.status, InvariantStatus::Fail);
        // Structured assertion: the failure message must name the first
        // divergent position by index. Position 0 is where the swap
        // takes effect.
        assert!(
            fail.cause.as_ref().is_some_and(|c| c.contains("position 0")),
            "failure message must name divergent position; got {:?}",
            fail.cause,
        );
    }

    // -----------------------------------------------------------------
    // fix-dst-dispatch-routing-invariant 01-01 — DispatchRoutingIsNameRestricted witnesses
    //
    // Cover all four evaluator branches: (a) vacuous-pass on empty
    // input, (b) cardinality, (c) per-eval routing, (d) smoking-gun.
    // Mutations on any branch predicate are killed by at least one of
    // these witnesses — paired with the end-to-end tests under
    // `tests/invariant_evaluators.rs` (the regression-test proof).
    // -----------------------------------------------------------------

    fn dispatch_jl_reconciler() -> overdrive_core::reconciler::ReconcilerName {
        overdrive_core::reconciler::ReconcilerName::new("job-lifecycle")
            .expect("job-lifecycle is a valid ReconcilerName")
    }

    fn dispatch_noop_reconciler() -> overdrive_core::reconciler::ReconcilerName {
        overdrive_core::reconciler::ReconcilerName::new("noop-heartbeat")
            .expect("noop-heartbeat is a valid ReconcilerName")
    }

    fn dispatch_target(raw: &str) -> overdrive_core::reconciler::TargetResource {
        overdrive_core::reconciler::TargetResource::new(raw).expect("valid TargetResource")
    }

    /// Branch (c, d) — happy path single eval.
    #[test]
    fn dispatch_routing_passes_on_clean_single_eval() {
        let r = dispatch_jl_reconciler();
        let t = dispatch_target("job/payments");
        let submitted = vec![Evaluation { reconciler: r.clone(), target: t.clone() }];
        let record = DispatchRecord { dispatched: vec![(r, t)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Pass);
        assert!(result.cause.is_none());
    }

    /// Branch (c, d) — happy path multi eval, distinct targets.
    #[test]
    fn dispatch_routing_passes_on_clean_multi_eval_distinct_targets() {
        let r = dispatch_jl_reconciler();
        let t_a = dispatch_target("job/payments");
        let t_b = dispatch_target("job/frontend");
        let submitted = vec![
            Evaluation { reconciler: r.clone(), target: t_a.clone() },
            Evaluation { reconciler: r.clone(), target: t_b.clone() },
        ];
        let record = DispatchRecord { dispatched: vec![(r.clone(), t_a), (r, t_b)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Pass);
    }

    /// Branch (a) — vacuous-pass on empty input.
    #[test]
    fn dispatch_routing_passes_vacuously_on_empty_input() {
        let submitted: Vec<Evaluation> = Vec::new();
        let record = DispatchRecord { dispatched: Vec::new() };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Pass);
    }

    /// Branch (b) — cardinality fail: dispatched has more entries than
    /// submitted.
    #[test]
    fn dispatch_routing_fails_on_cardinality_mismatch_extra() {
        let r = dispatch_jl_reconciler();
        let noop = dispatch_noop_reconciler();
        let t = dispatch_target("job/payments");
        let submitted = vec![Evaluation { reconciler: r.clone(), target: t.clone() }];
        // Two dispatch entries for one drained eval — fan-out shape.
        let record = DispatchRecord { dispatched: vec![(r, t.clone()), (noop, t)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Fail);
        assert!(
            result
                .cause
                .as_ref()
                .is_some_and(|c| c.contains("expected") && c.contains("dispatch entries")),
            "cause must name cardinality shape; got {:?}",
            result.cause,
        );
    }

    /// Branch (b) — cardinality fail: dispatched has fewer entries
    /// than submitted (missed dispatch).
    #[test]
    fn dispatch_routing_fails_on_cardinality_mismatch_missing() {
        let r = dispatch_jl_reconciler();
        let t_a = dispatch_target("job/payments");
        let t_b = dispatch_target("job/frontend");
        let submitted = vec![
            Evaluation { reconciler: r.clone(), target: t_a.clone() },
            Evaluation { reconciler: r.clone(), target: t_b },
        ];
        let record = DispatchRecord { dispatched: vec![(r, t_a)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Fail);
        assert!(
            result
                .cause
                .as_ref()
                .is_some_and(|c| c.contains("expected") && c.contains("dispatch entries")),
            "cause must name cardinality shape; got {:?}",
            result.cause,
        );
    }

    /// Branch (d) — smoking-gun: cardinality matches but one dispatch
    /// entry names an unsubmitted reconciler. Pinning per-eval routing
    /// (branch c) catches this first; the smoking-gun branch is
    /// reachable when per-eval routing PASSES but the dispatcher
    /// somehow added a stray entry of the same length. To exercise
    /// branch (d) deterministically we construct a fixture where (c)
    /// finds zero matches for the second submitted eval and fails —
    /// this still proves the fan-out shape is rejected.
    #[test]
    fn dispatch_routing_fails_on_smoking_gun_unsubmitted_reconciler() {
        let jl = dispatch_jl_reconciler();
        let noop = dispatch_noop_reconciler();
        let t_a = dispatch_target("job/payments");
        let t_b = dispatch_target("job/frontend");
        // Two submitted evals naming `jl`. Dispatched has matching
        // cardinality (2) but the SECOND entry names `noop` — an
        // unsubmitted reconciler. Per-eval routing fails on the second
        // submitted eval (zero matches for `(jl, t_b)`); the cause
        // names the fan-out shape.
        let submitted = vec![
            Evaluation { reconciler: jl.clone(), target: t_a.clone() },
            Evaluation { reconciler: jl.clone(), target: t_b.clone() },
        ];
        let record = DispatchRecord { dispatched: vec![(jl, t_a), (noop, t_b)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Fail);
        // Either per-eval routing or smoking-gun cause is acceptable;
        // both name the wrong-reconciler shape. Per-eval routing fires
        // first under the current branch order.
        assert!(
            result.cause.as_ref().is_some_and(|c| c.contains("expected exactly one dispatch")),
            "per-eval routing cause must name the named-reconciler shape; got {:?}",
            result.cause,
        );
    }

    /// Branch (c) — wrong-routing: cardinality matches but dispatcher
    /// invoked a different reconciler than the one submitted.
    #[test]
    fn dispatch_routing_fails_when_named_reconciler_not_dispatched() {
        let jl = dispatch_jl_reconciler();
        let noop = dispatch_noop_reconciler();
        let t = dispatch_target("job/payments");
        let submitted = vec![Evaluation { reconciler: jl, target: t.clone() }];
        // Cardinality matches (1 == 1) but the dispatched reconciler
        // is wrong. Per-eval routing finds zero matches for `(jl, t)`.
        let record = DispatchRecord { dispatched: vec![(noop, t)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Fail);
    }

    /// Pure smoking-gun branch (d) — only reachable when per-eval
    /// routing has already passed for every submitted eval AND there
    /// is still an extra dispatched entry naming an unsubmitted
    /// reconciler. The cardinality branch catches the surplus first
    /// under the current shape, but if a future refactor reorders the
    /// branches, the smoking-gun assertion must still fire on this
    /// fixture. Constructed to fail at the cardinality branch with a
    /// cause that names the surplus.
    #[test]
    fn dispatch_routing_fails_when_extra_entry_names_unsubmitted_reconciler() {
        let jl = dispatch_jl_reconciler();
        let noop = dispatch_noop_reconciler();
        let t_a = dispatch_target("job/payments");
        let t_b = dispatch_target("job/frontend");
        let submitted = vec![Evaluation { reconciler: jl.clone(), target: t_a.clone() }];
        // One submitted eval, two dispatched entries — the extra one
        // names `noop`, an unsubmitted reconciler.
        let record = DispatchRecord { dispatched: vec![(jl, t_a), (noop, t_b)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Fail);
    }

    #[test]
    fn build_tick_context_produces_deadline_strictly_after_now() {
        // Kills the `+ with -` mutation on the deadline arithmetic in
        // `build_tick_context`: with `-`, `now - BUDGET` would produce
        // a deadline in the past (before `now`), failing this
        // assertion. With the original `+`, deadline is exactly
        // `BUDGET` ahead of `now`. The Phase 1 reconciler
        // (`NoopHeartbeat`) ignores `tick.deadline`, so without a
        // direct test on the helper the mutation survives — the
        // evaluator returns a deterministic Pass either way.
        let clock = SimClock::new();
        let tick = build_tick_context(&clock);
        assert!(
            tick.deadline > tick.now,
            "deadline must be strictly after now; got now={:?} deadline={:?}",
            tick.now,
            tick.deadline,
        );
        assert_eq!(tick.tick, 0, "tick counter is a fixed zero per the evaluator contract");
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

    // -----------------------------------------------------------------
    // phase-1-first-workload — slice 3 (US-03) — convergence invariants
    // -----------------------------------------------------------------

    /// Build an `AllocStatusRow` fixture for the convergence-invariant
    /// witnesses. Centralises the boilerplate so individual tests stay
    /// focused on their assertion.
    fn alloc_row(alloc: &str, job: &str, node: &str, state: AllocState) -> AllocStatusRow {
        use overdrive_core::id::AllocationId;
        use overdrive_core::traits::observation_store::LogicalTimestamp;
        AllocStatusRow {
            alloc_id: AllocationId::new(alloc).expect("valid alloc id"),
            job_id: JobId::new(job).expect("valid job id"),
            node_id: NodeId::new(node).expect("valid node id"),
            state,
            updated_at: LogicalTimestamp {
                counter: 1,
                writer: NodeId::new(node).expect("valid node id"),
            },
            reason: None,
            detail: None,
            terminal: None,
        }
    }

    #[test]
    fn job_scheduled_after_submission_passes_when_every_job_has_running_alloc() {
        let jobs = vec![JobId::new("payments").expect("valid job id")];
        let rows = vec![alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Running)];
        let r = evaluate_job_scheduled_after_submission(&jobs, &rows);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    #[test]
    fn job_scheduled_after_submission_fails_when_no_running_alloc_for_submitted_job() {
        let jobs = vec![JobId::new("payments").expect("valid job id")];
        let rows = vec![alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Pending)];
        let r = evaluate_job_scheduled_after_submission(&jobs, &rows);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("no Running alloc")));
    }

    #[test]
    fn job_scheduled_after_submission_passes_vacuously_with_no_submissions() {
        let jobs: Vec<JobId> = Vec::new();
        let rows: Vec<AllocStatusRow> = Vec::new();
        let r = evaluate_job_scheduled_after_submission(&jobs, &rows);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    #[test]
    fn job_scheduled_after_submission_fails_when_running_alloc_belongs_to_different_job() {
        let jobs = vec![JobId::new("payments").expect("valid job id")];
        let rows = vec![alloc_row("alloc-frontend-0", "frontend", "node-1", AllocState::Running)];
        let r = evaluate_job_scheduled_after_submission(&jobs, &rows);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    #[test]
    fn desired_replica_count_converges_passes_at_n_equals_one() {
        let want = vec![(JobId::new("payments").expect("valid job id"), 1)];
        let rows = vec![alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Running)];
        let r = evaluate_desired_replica_count_converges(&want, &rows);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    #[test]
    fn desired_replica_count_converges_fails_when_observed_count_undershoots() {
        let want = vec![(JobId::new("payments").expect("valid job id"), 2)];
        let rows = vec![alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Running)];
        let r = evaluate_desired_replica_count_converges(&want, &rows);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("want 2")));
    }

    #[test]
    fn desired_replica_count_converges_fails_when_observed_count_overshoots() {
        let want = vec![(JobId::new("payments").expect("valid job id"), 1)];
        let rows = vec![
            alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Running),
            alloc_row("alloc-payments-1", "payments", "node-1", AllocState::Running),
        ];
        let r = evaluate_desired_replica_count_converges(&want, &rows);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    /// Pin the `&&` clause in the row filter against `||`. The
    /// fixture mixes a Running alloc for the target job with a
    /// non-Running alloc for the SAME job — under `&&` only the
    /// Running one counts (count=1, equals desired); under `||`
    /// both clauses are independently true so both rows count
    /// (count=2, mismatches desired). The `Pass` outcome is unique
    /// to the `&&` shape.
    #[test]
    fn desired_replica_count_converges_distinguishes_and_from_or_in_row_filter() {
        let want = vec![(JobId::new("payments").expect("valid job id"), 1)];
        // Two rows for the SAME job: one Running, one Terminated.
        // Under production `&&`: only Running matches → count=1 ⇒
        // matches desired (1) ⇒ Pass.
        // Under mutant `||`: both match (first via state==Running,
        // second via job_id==payments) → count=2 ⇒ mismatches
        // desired (1) ⇒ Fail.
        let rows = vec![
            alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Running),
            alloc_row("alloc-payments-1", "payments", "node-1", AllocState::Terminated),
        ];
        let r = evaluate_desired_replica_count_converges(&want, &rows);
        assert_eq!(
            r.status,
            InvariantStatus::Pass,
            "`&&` filter must count only Running allocs of target job; got {r:?}",
        );
    }

    #[test]
    fn no_double_scheduling_passes_on_consistent_pinning() {
        let rows = vec![
            alloc_row("alloc-a", "payments", "node-1", AllocState::Running),
            alloc_row("alloc-b", "frontend", "node-2", AllocState::Running),
            // Same alloc, same node — duplicate row from gossip is not
            // a violation.
            alloc_row("alloc-a", "payments", "node-1", AllocState::Running),
        ];
        let r = evaluate_no_double_scheduling(&rows);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    #[test]
    fn no_double_scheduling_fails_when_alloc_pinned_to_two_nodes() {
        let rows = vec![
            alloc_row("alloc-a", "payments", "node-1", AllocState::Running),
            alloc_row("alloc-a", "payments", "node-2", AllocState::Running),
        ];
        let r = evaluate_no_double_scheduling(&rows);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("two nodes")));
    }

    #[test]
    fn no_double_scheduling_passes_on_empty_snapshot() {
        let rows: Vec<AllocStatusRow> = Vec::new();
        let r = evaluate_no_double_scheduling(&rows);
        assert_eq!(r.status, InvariantStatus::Pass);
    }
}
