//! A byte count with one binary-unit grammar and one canonical spelling
//! (GH #372, ADR-0339). `memory` in `[resources]` is a `ByteSize`; the volume
//! size refines it.
//!
//! RED scaffold (DISTILL): the shapes below are the ones
//! `docs/feature/workload-resources-human-readable/feature-delta.md`
//! § "Interface contracts" pins. The grammar refuses every input and `Display`
//! renders nothing until DELIVER implements it, so a test about the grammar
//! fails on its own accept or reject assertion rather than on a panic inside
//! the scaffold.

#![allow(
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    reason = "DISTILL RED scaffold: bodies are placeholders until DELIVER implements the grammar"
)]

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// SCAFFOLD: true
///
/// A byte count. Grammar `<digits><unit>`, unit case-insensitive `B`, `KiB`,
/// `MiB`, `GiB` or `TiB`. `Display` is the canonical form: the integer count
/// in the largest unit that divides the value exactly; zero is `0B`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct ByteSize(u64);

impl ByteSize {
    /// Total: every `u64` is a valid size, including 0.
    #[must_use]
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    /// The size in bytes.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.0
    }
}

/// A byte size that failed the grammar, one variant per distinct operator
/// remedy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ByteSizeParseError {
    /// Not `<digits><unit>` at all.
    #[error("byte size scaffold: malformed {input:?}")]
    Malformed { input: String },
    /// Digits with no unit.
    #[error("byte size scaffold: missing unit {input:?}")]
    MissingUnit { input: String },
    /// A decimal unit (`KB`, `MB`, `GB`, `TB`).
    #[error("byte size scaffold: decimal unit {input:?}")]
    DecimalUnit { input: String },
    /// A fractional count.
    #[error("byte size scaffold: fractional {input:?}")]
    Fractional { input: String },
    /// A valid input whose value exceeds `u64::MAX` bytes.
    #[error("byte size scaffold: overflow {input:?}")]
    Overflow { input: String },
}

impl FromStr for ByteSize {
    type Err = ByteSizeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // RED scaffold: refuses every input until DELIVER implements the grammar.
        Err(ByteSizeParseError::Malformed { input: s.to_owned() })
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // RED scaffold: renders nothing until DELIVER implements the canonical text.
        Ok(())
    }
}

impl Serialize for ByteSize {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}
