//! A byte count with one binary-unit grammar and one canonical spelling
//! (GH #372, ADR-0339). `memory` in `[resources]` is a `ByteSize`; the volume
//! size refines it.

use std::fmt;
use std::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const BYTES_PER_KIB: u64 = 1 << 10;
const BYTES_PER_MIB: u64 = 1 << 20;
const BYTES_PER_GIB: u64 = 1 << 30;
const BYTES_PER_TIB: u64 = 1 << 40;

/// Canonical units, largest first: `Display` picks the first that divides exactly.
const CANONICAL_UNITS: [(&str, u64); 5] = [
    ("TiB", BYTES_PER_TIB),
    ("GiB", BYTES_PER_GIB),
    ("MiB", BYTES_PER_MIB),
    ("KiB", BYTES_PER_KIB),
    ("B", 1),
];

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
    Hash,
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
    #[error(
        "invalid byte size {input:?}: expected a count and a unit, written like 128MiB \
         (units: B, KiB, MiB, GiB, TiB)"
    )]
    Malformed { input: String },
    /// Digits with no unit.
    #[error(
        "invalid byte size {input:?}: the count has no unit; write the unit after the count, \
         like 128MiB"
    )]
    MissingUnit { input: String },
    /// A decimal unit (`KB`, `MB`, `GB`, `TB`).
    #[error(
        "invalid byte size {input:?}: decimal units are not accepted; use the binary units \
         KiB, MiB, GiB or TiB"
    )]
    DecimalUnit { input: String },
    /// A fractional count.
    #[error(
        "invalid byte size {input:?}: the count must be a whole number; write it in the \
         smaller unit instead, like 1536MiB for 1.5GiB"
    )]
    Fractional { input: String },
    /// A valid input whose value exceeds `u64::MAX` bytes.
    #[error(
        "invalid byte size {input:?}: the value is too large; the limit is 18446744073709551615 bytes"
    )]
    Overflow { input: String },
}

/// The byte multiple of a binary unit spelling, case-insensitive.
fn binary_unit_multiple(unit: &str) -> Option<u64> {
    CANONICAL_UNITS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(unit))
        .map(|&(_, multiple)| multiple)
}

fn is_decimal_unit(unit: &str) -> bool {
    ["KB", "MB", "GB", "TB"].iter().any(|name| name.eq_ignore_ascii_case(unit))
}

impl FromStr for ByteSize {
    type Err = ByteSizeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = |build: fn(String) -> ByteSizeParseError| build(s.to_owned());
        let digits_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let (digits, unit) = s.split_at(digits_end);
        if digits.is_empty() {
            return Err(err(|input| ByteSizeParseError::Malformed { input }));
        }
        if unit.is_empty() {
            return Err(err(|input| ByteSizeParseError::MissingUnit { input }));
        }
        if unit.starts_with('.') {
            return Err(err(|input| ByteSizeParseError::Fractional { input }));
        }
        if is_decimal_unit(unit) {
            return Err(err(|input| ByteSizeParseError::DecimalUnit { input }));
        }
        let Some(multiple) = binary_unit_multiple(unit) else {
            return Err(err(|input| ByteSizeParseError::Malformed { input }));
        };
        digits
            .parse::<u64>()
            .ok()
            .and_then(|count| count.checked_mul(multiple))
            .map(Self)
            .ok_or_else(|| err(|input| ByteSizeParseError::Overflow { input }))
    }
}

impl fmt::Display for ByteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Zero divides every unit, so it renders in bytes rather than as `0TiB`.
        if self.0 == 0 {
            return f.write_str("0B");
        }
        let (name, multiple) = CANONICAL_UNITS
            .iter()
            .copied()
            .find(|&(_, multiple)| self.0.is_multiple_of(multiple))
            .unwrap_or(("B", 1));
        write!(f, "{}{name}", self.0 / multiple)
    }
}

impl Serialize for ByteSize {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

struct ByteSizeVisitor;

impl Visitor<'_> for ByteSizeVisitor {
    type Value = ByteSize;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a byte size string such as 128MiB (units: B, KiB, MiB, GiB, TiB)")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<ByteSize, E> {
        v.parse().map_err(E::custom)
    }
}

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_str(ByteSizeVisitor)
    }
}
