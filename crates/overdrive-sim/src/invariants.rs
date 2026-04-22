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
