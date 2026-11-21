//! A CPU amount at millicore resolution with one grammar and one canonical
//! spelling (GH #372, ADR-0340). `cpu` in `[resources]` is a `CpuQuantity`.

use std::fmt;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const MILLIS_PER_CORE: u64 = 1000;
const MAX_FRACTIONAL_DIGITS: usize = 3;

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
    Hash,
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
    #[error(
        "invalid cpu quantity {input:?}: expected millicores like 500m, or cores like 0.5 or 2"
    )]
    Malformed { input: String },
    /// Digits followed by an alphabetic suffix other than lowercase `m`.
    #[error(
        "invalid cpu quantity {input:?}: unknown suffix {suffix:?}; the only suffix is the \
         lowercase milli suffix m, as in 500m"
    )]
    UnknownSuffix { input: String, suffix: String },
    /// More than three fractional digits.
    #[error(
        "invalid cpu quantity {input:?}: at most three decimal places are allowed (millicore \
         resolution); write 0.001 at the smallest or use millicores like 1m"
    )]
    TooManyDecimals { input: String },
    /// A value above `u32::MAX` millicores.
    #[error(
        "invalid cpu quantity {input:?}: the value is too large; the limit is 4294967295 millicores"
    )]
    Overflow { input: String },
}

/// Millicores for `whole` cores plus `fraction` (already right-padded to millis).
fn core_form_millis(whole: &str, fraction: u64) -> Option<u32> {
    let whole = whole.parse::<u64>().ok()?;
    let millis = whole.checked_mul(MILLIS_PER_CORE)?.checked_add(fraction)?;
    u32::try_from(millis).ok()
}

/// The fractional digits right-padded to three places, as millicores.
fn fraction_millis(fraction_digits: &str) -> Option<u64> {
    let padded = format!("{fraction_digits:0<MAX_FRACTIONAL_DIGITS$}");
    padded.parse().ok()
}

impl FromStr for CpuQuantity {
    type Err = CpuQuantityParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let malformed = || CpuQuantityParseError::Malformed { input: s.to_owned() };
        let overflow = || CpuQuantityParseError::Overflow { input: s.to_owned() };
        let digits_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let (digits, tail) = s.split_at(digits_end);
        if digits.is_empty() {
            return Err(malformed());
        }
        if tail.is_empty() {
            return core_form_millis(digits, 0).map(Self).ok_or_else(overflow);
        }
        if let Some(after_point) = tail.strip_prefix('.') {
            let fraction_end =
                after_point.find(|c: char| !c.is_ascii_digit()).unwrap_or(after_point.len());
            let (fraction, rest) = after_point.split_at(fraction_end);
            if fraction.is_empty() || !rest.is_empty() {
                return Err(malformed());
            }
            if fraction.len() > MAX_FRACTIONAL_DIGITS {
                return Err(CpuQuantityParseError::TooManyDecimals { input: s.to_owned() });
            }
            return fraction_millis(fraction)
                .and_then(|millis| core_form_millis(digits, millis))
                .map(Self)
                .ok_or_else(overflow);
        }
        if !tail.bytes().all(|b| b.is_ascii_alphabetic()) {
            return Err(malformed());
        }
        if tail != "m" {
            return Err(CpuQuantityParseError::UnknownSuffix {
                input: s.to_owned(),
                suffix: tail.to_owned(),
            });
        }
        digits.parse::<u32>().map(Self).map_err(|_| overflow())
    }
}

impl fmt::Display for CpuQuantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let millis = u64::from(self.0);
        if millis.is_multiple_of(MILLIS_PER_CORE) {
            write!(f, "{}", millis / MILLIS_PER_CORE)
        } else {
            write!(f, "{millis}m")
        }
    }
}

impl Serialize for CpuQuantity {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

struct CpuQuantityVisitor;

impl Visitor<'_> for CpuQuantityVisitor {
    type Value = CpuQuantity;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a cpu quantity string such as 500m or 0.5")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<CpuQuantity, E> {
        v.parse().map_err(E::custom)
    }
}

impl<'de> Deserialize<'de> for CpuQuantity {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_str(CpuQuantityVisitor)
    }
}
