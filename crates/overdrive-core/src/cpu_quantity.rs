//! A CPU amount at millicore resolution with one grammar and one canonical
//! spelling (GH #372, ADR-0340). `cpu` in `[resources]` is a `CpuQuantity`.
//!
//! RED scaffold (DISTILL): the shapes below are the ones
//! `docs/feature/workload-resources-human-readable/feature-delta.md`
//! § "Interface contracts" pins. The grammar refuses every input and `Display`
//! renders nothing until DELIVER implements it.

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
/// A CPU amount in millicores. Grammar: `<digits>m` (integer millicores) or
/// decimal cores with at most three fractional digits. `Display` is the
/// canonical form: the whole-core integer when the value is an exact multiple
/// of 1000, otherwise `<n>m`.
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
pub struct CpuQuantity(u32);

impl CpuQuantity {
    /// Total: every `u32` is a valid amount, including 0.
    #[must_use]
    pub const fn from_millis(millis: u32) -> Self {
        Self(millis)
    }

    /// The amount in millicores.
    #[must_use]
    pub const fn millis(self) -> u32 {
        self.0
    }
}

/// A CPU quantity that failed the grammar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CpuQuantityParseError {
    /// Not a CPU quantity at all.
    #[error("cpu quantity scaffold: malformed {input:?}")]
    Malformed { input: String },
    /// Digits followed by an alphabetic suffix other than lowercase `m`.
    #[error("cpu quantity scaffold: unknown suffix {suffix:?} in {input:?}")]
    UnknownSuffix { input: String, suffix: String },
    /// More than three fractional digits.
    #[error("cpu quantity scaffold: too many decimals {input:?}")]
    TooManyDecimals { input: String },
    /// A value above `u32::MAX` millicores.
    #[error("cpu quantity scaffold: overflow {input:?}")]
    Overflow { input: String },
}

impl FromStr for CpuQuantity {
    type Err = CpuQuantityParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // RED scaffold: refuses every input until DELIVER implements the grammar.
        Err(CpuQuantityParseError::Malformed { input: s.to_owned() })
    }
}

impl fmt::Display for CpuQuantity {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // RED scaffold: renders nothing until DELIVER implements the canonical text.
        Ok(())
    }
}

impl Serialize for CpuQuantity {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for CpuQuantity {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}
