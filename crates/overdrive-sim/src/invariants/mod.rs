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
