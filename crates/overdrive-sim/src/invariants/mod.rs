//! DST invariant catalogue.
//!
//! The [`Invariant`] enum is the canonical name source for `--only <NAME>`
//! on `cargo xtask dst` and for every invariant entry in
//! `target/xtask/dst-summary.json`. `Display` emits kebab-case, lowercase;
//! [`FromStr`] accepts any ASCII-case spelling of a canonical name. A name
//! printed by the harness MUST round-trip losslessly through
//! `FromStr → Display` — the proptest in
//! `crates/overdrive-sim/tests/invariant_roundtrip.rs` enforces that.
//!
//! Phase 1 ships the catalogue definition and canonical-name machinery.
//! The invariant *evaluators* — the code that decides whether an
//! invariant holds in a given run — land in step 06-02. Every name in
//! this enum is already known to `cargo xtask dst`, so CI wiring and
//! artifact shape are stable even before the evaluators exist.

#![allow(clippy::missing_errors_doc)]

use std::fmt::{self, Display};
use std::str::FromStr;

pub mod evaluators;

/// Catalogue of invariants the DST harness evaluates.
///
/// Each variant name IS the canonical name printed in both green
/// progress lines and red failure output. `--only <NAME>` resolves
/// against this enum via [`FromStr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Invariant {
    /// At most one leader across the Raft cluster at any simulated tick.
    SingleLeader,
    /// No row in any `ObservationStore` carries intent-class data, and
    /// no key in any `IntentStore` carries observation-class data.
    IntentNeverCrossesIntoObservation,
    /// `IntentStore::export_snapshot` → `bootstrap_from` →
    /// `export_snapshot` is byte-identical.
    SnapshotRoundtripBitIdentical,
    /// LWW convergence across a `SimObservationStore` cluster under
    /// arbitrary seeded delivery orders reaches the same row set on
    /// every peer.
    SimObservationLwwConverges,
    /// An empty workflow journal replays bit-identically.
    ReplayEquivalentEmptyWorkflow,
    /// `SimEntropy` seeded with the same `u64` twice produces the same
    /// draw sequence — the twin-run identity property.
    EntropyDeterminismUnderReseed,
    /// SCAFFOLD: true — phase-1-control-plane-core DISTILL per ADR-0013.
    /// At least one reconciler is registered with the runtime after
    /// boot; the registry is never empty. The evaluator body panics
    /// until DELIVER wires the control-plane runtime into the harness.
    AtLeastOneReconcilerRegistered,
    /// SCAFFOLD: true — phase-1-control-plane-core DISTILL per ADR-0013.
    /// N (≥3) concurrent evaluations at the same `(ReconcilerName,
    /// TargetResource)` key collapse to exactly one dispatched
    /// invocation and `N - 1` cancellations. The evaluator body panics
    /// until DELIVER ships the broker.
    DuplicateEvaluationsCollapse,
    /// Two drain passes against identical submit sequences produce
    /// element-equal `dispatched_order` vecs at every position. Closes
    /// `docs/feature/fix-eval-broker-drain-determinism` RCA — the
    /// broker's drain order MUST be deterministic, not dependent on
    /// `HashSet` iteration order or other implicit state. Sibling to
    /// `DuplicateEvaluationsCollapse`: that invariant pins counters,
    /// this one pins ordering.
    BrokerDrainOrderIsDeterministic,
    /// SCAFFOLD: true — phase-1-control-plane-core DISTILL per ADR-0013.
    /// Twin invocation of a reconciler's `reconcile` with identical
    /// inputs produces bit-identical `Vec<Action>` outputs. The
    /// evaluator body panics until DELIVER wires the noop-heartbeat
    /// reconciler into the harness.
    ReconcilerIsPure,
    /// phase-1-control-plane-core / fix-eval-reconciler-discarded follow-up.
    /// For any drained `Evaluation { reconciler: R, target: T }`, exactly
    /// one reconciler — R — runs through the dispatch path against T per
    /// tick. The DST-tier peer of the unit/acceptance pin at
    /// `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs::eval_dispatch_runs_only_the_named_reconciler`
    /// (commit `e6f5e5e`). Closes the §8 storm-proofing dispatch-routing
    /// contract end-to-end. Sibling to `DuplicateEvaluationsCollapse`:
    /// that invariant pins broker-side entry collapse, this one pins
    /// dispatcher-side routing.
    DispatchRoutingIsNameRestricted,
    /// `IntentStore::put(k, v)` followed by `IntentStore::get(k)`
    /// returns `Some(v)` byte-for-byte — no framing, no prefix, no
    /// transformation. Closes ADR-0020 §Enforcement: the structural-
    /// regression guard against re-introducing inline row encoding
    /// in `LocalIntentStore`.
    IntentStoreReturnsCallerBytes,
    /// phase-1-first-workload (slice 3, US-03) — eventually invariant.
    /// For every submitted Job, an `AllocStatusRow{state: Running}`
    /// exists within budget N ticks. The harness drives the
    /// convergence loop forward N ticks and inspects the
    /// `ObservationStore` for at least one `Running` row per
    /// submitted job. Lives in
    /// `crates/overdrive-sim/src/invariants/evaluators.rs` per the
    /// existing single-file evaluator pattern.
    JobScheduledAfterSubmission,
    /// phase-1-first-workload (slice 3, US-03) — eventually invariant.
    /// `count(state == Running) == job.replicas` per submitted job.
    /// Vacuous-pass at N=1 (a 1-replica job has at most one Running
    /// row), but the evaluator still has to walk the rows and tally
    /// per job to catch the failure mode where a Running row leaks
    /// across jobs.
    DesiredReplicaCountConverges,
    /// phase-1-first-workload (slice 3, US-03) — always invariant.
    /// Each `alloc_id` agrees on a single `node_id` across the
    /// `alloc_status` snapshot. Two rows for the same `alloc_id`
    /// pinned to different nodes is a double-scheduling violation.
    NoDoubleScheduling,
}

impl Invariant {
    /// Every variant in the catalogue, in the order the harness runs
    /// them by default. Keep this list synchronised with the enum —
    /// `ALL` is the default catalogue the harness iterates when
    /// `--only <NAME>` is absent.
    pub const ALL: &'static [Self] = &[
        Self::SingleLeader,
        Self::IntentNeverCrossesIntoObservation,
        Self::SnapshotRoundtripBitIdentical,
        Self::SimObservationLwwConverges,
        Self::ReplayEquivalentEmptyWorkflow,
        Self::EntropyDeterminismUnderReseed,
        // SCAFFOLD: true — phase-1-control-plane-core DISTILL per ADR-0013.
        Self::AtLeastOneReconcilerRegistered,
        Self::DuplicateEvaluationsCollapse,
        Self::BrokerDrainOrderIsDeterministic,
        Self::ReconcilerIsPure,
        Self::DispatchRoutingIsNameRestricted,
        Self::IntentStoreReturnsCallerBytes,
        // SCAFFOLD: false — phase-1-first-workload slice 3 (US-03).
        Self::JobScheduledAfterSubmission,
        Self::DesiredReplicaCountConverges,
        Self::NoDoubleScheduling,
    ];

    /// The canonical kebab-case spelling of this invariant, as a static
    /// string. `Display` renders the same text; having a `&'static str`
    /// view lets callers embed the name in logs without allocating.
    #[must_use]
    pub const fn as_canonical(self) -> &'static str {
        match self {
            Self::SingleLeader => "single-leader",
            Self::IntentNeverCrossesIntoObservation => "intent-never-crosses-into-observation",
            Self::SnapshotRoundtripBitIdentical => "snapshot-roundtrip-bit-identical",
            Self::SimObservationLwwConverges => "sim-observation-lww-converges",
            Self::ReplayEquivalentEmptyWorkflow => "replay-equivalent-empty-workflow",
            Self::EntropyDeterminismUnderReseed => "entropy-determinism-under-reseed",
            // SCAFFOLD: true — phase-1-control-plane-core DISTILL per ADR-0013.
            Self::AtLeastOneReconcilerRegistered => "at-least-one-reconciler-registered",
            Self::DuplicateEvaluationsCollapse => "duplicate-evaluations-collapse",
            Self::BrokerDrainOrderIsDeterministic => "broker-drain-order-is-deterministic",
            Self::ReconcilerIsPure => "reconciler-is-pure",
            Self::DispatchRoutingIsNameRestricted => "dispatch-routing-is-name-restricted",
            Self::IntentStoreReturnsCallerBytes => "intent-store-returns-caller-bytes",
            // phase-1-first-workload slice 3 (US-03).
            Self::JobScheduledAfterSubmission => "job-scheduled-after-submission",
            Self::DesiredReplicaCountConverges => "desired-replica-count-converges",
            Self::NoDoubleScheduling => "no-double-scheduling",
        }
    }
}

impl Display for Invariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_canonical())
    }
}

impl FromStr for Invariant {
    type Err = InvariantParseError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        // Case-insensitive match against the canonical forms. Hyphens
        // are preserved, only alphabetic characters are folded.
        let lowered = raw.to_ascii_lowercase();
        for candidate in Self::ALL {
            if candidate.as_canonical() == lowered {
                return Ok(*candidate);
            }
        }
        Err(InvariantParseError { raw: raw.to_owned() })
    }
}

/// Error returned when `--only <NAME>` cannot be resolved to an
/// [`Invariant`] variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown invariant name: {raw:?}")]
pub struct InvariantParseError {
    /// The caller-provided string that did not match any variant.
    pub raw: String,
}
