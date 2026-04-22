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
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::intent_store::IntentStore;

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
    for prefix in OBSERVATION_KEY_PREFIXES {
        // `get` on a prefix key directly is not enough — we need to
        // check whether *any* key in intent starts with one of the
        // banned prefixes. `watch` + a short drain would work but is
        // racy for a one-shot probe. Instead, we probe for the exact
        // prefix; a production writer of observation-class data into
        // intent would almost certainly write the prefix verbatim
        // (the failure shape the test exercises) plus some alloc id
        // suffix. We scan by export_snapshot to see every key.
        match intent.export_snapshot().await {
            Ok(snap) => {
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
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("intent snapshot failed: {err}")),
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
/// bootstrap a second `LocalStore` from the frame, re-export, and
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

    // Bootstrap a fresh LocalStore from the frame.
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
    let second_store = match overdrive_store_local::LocalStore::open(&path) {
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
    let name = "sim-observation-lww-converges";

    // Drain the gossip window before snapshotting — a pre-drain call
    // would race any in-flight writes from earlier harness setup.
    cluster.advance(Duration::from_millis(500)).await;

    let report = check_lww_convergence(cluster);
    if report.is_converged() {
        result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
    } else {
        result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
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
                Some(format!("SimEntropy diverges at draw {i}: {x:#x} vs {y:#x}",)),
            );
        }
    }
    result(name, InvariantStatus::Pass, "host-0", None)
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
}
