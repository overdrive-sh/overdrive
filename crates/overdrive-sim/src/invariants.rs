//! DST invariant catalogue.
//!
//! SCAFFOLD: true — DISTILL placeholder per DWD-06. The `Invariant` enum
//! is the canonical name source for the `shared-artifacts-registry`
//! `invariant_name` artifact. Every invariant name printed by the
//! harness MUST round-trip through `Invariant::from_str` →
//! `Invariant::to_string`. Crafter fills in the evaluator bodies during
//! DELIVER.

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
    SingleLeader,
    IntentNeverCrossesIntoObservation,
    SnapshotRoundtripBitIdentical,
    SimObservationLwwConverges,
    ReplayEquivalentEmptyWorkflow,
    EntropyDeterminismUnderReseed,
}

impl Display for Invariant {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SCAFFOLD: true — crafter implements canonical kebab-case form.
        unimplemented!("Invariant Display — RED scaffold; DELIVER fills in")
    }
}

impl FromStr for Invariant {
    type Err = InvariantParseError;

    fn from_str(_raw: &str) -> Result<Self, Self::Err> {
        // SCAFFOLD: true — crafter implements case-insensitive, kebab-case parse.
        unimplemented!("Invariant FromStr — RED scaffold; DELIVER fills in")
    }
}

/// Error returned when `--only <NAME>` cannot be resolved to an
/// [`Invariant`] variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvariantParseError {
    pub raw: String,
}
