//! `MaglevTableSize` — STRICT newtype for the Maglev permutation table size.
//!
//! Constrained to Cilium's prime list per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 6.
//! Default `M = 16_381`; the `M ≥ 100·N` rule is enforced at backend-
//! set-update time (separate concern; not at construction).
//!
//! # Wire form
//!
//! `Display` emits the decimal `u32` representation. `FromStr` parses
//! decimal `u32`, then validates against [`ALLOWED_PRIMES`] — every
//! value outside the list rejects with [`ParseError::NotInPrimeList`].
//! `serde` validates on `Deserialize` via the
//! `#[serde(try_from = "u32", into = "u32")]` attribute, so wire
//! payloads carrying non-primes are rejected at the deserialisation
//! boundary, not silently accepted.
//!
//! There is no case axis for a numeric identifier — the
//! case-insensitivity rule from `development.md` § Newtype completeness
//! applies only to human-typed string identifiers (matches the
//! `BackendId` / `ServiceId` precedent).

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use core::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Allowed Maglev table sizes, locked to Cilium's prime list per
/// research § 5.2. Each is the largest prime ≤ a power-of-2 boundary
/// (2^8, 2^9, …, 2^17) — the shape Cilium / Katran ship in production.
pub const ALLOWED_PRIMES: [u32; 10] =
    [251, 509, 1_021, 2_039, 4_093, 8_191, 16_381, 32_749, 65_521, 131_071];

/// Default Maglev `M` — smallest prime ≥ 16_384; matches Cilium /
/// Katran. Supports up to ~160 backends per the `M ≥ 100·N` rule.
pub const DEFAULT_M: u32 = 16_381;

/// Maglev permutation table size. Constrained to [`ALLOWED_PRIMES`].
///
/// Constructed via [`MaglevTableSize::new`] (validating) or
/// [`MaglevTableSize::default`] (yields [`MaglevTableSize::DEFAULT`]).
/// Raw `u32` for any persisted Maglev table-size field is a blocking
/// violation per `.claude/rules/development.md` § Newtype completeness.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(try_from = "u32", into = "u32")]
pub struct MaglevTableSize(u32);

/// Parse / validation failure for [`MaglevTableSize`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Provided value is not in [`ALLOWED_PRIMES`]. The variant carries
    /// the rejected `value` plus the canonical `allowed` list so
    /// operator-facing error messages can name both the bad input and
    /// the acceptable set without re-deriving it at the call site.
    #[error(
        "MaglevTableSize {value} is not in the Cilium prime list \
         (allowed: {allowed:?})"
    )]
    NotInPrimeList {
        /// Rejected value.
        value: u32,
        /// Canonical accepted set — `&'static` reference to
        /// [`ALLOWED_PRIMES`].
        allowed: &'static [u32],
    },

    /// String could not be parsed to a `u32` (numeric overflow,
    /// non-digit characters, empty input).
    #[error("MaglevTableSize parse failed: {0}")]
    Malformed(String),
}

impl MaglevTableSize {
    /// Default Maglev `M` — wraps [`DEFAULT_M`]. `Self(16_381)` is
    /// known-prime by construction (the value is in
    /// [`ALLOWED_PRIMES`]); the const-context constructor is
    /// load-bearing for downstream consumers (e.g.
    /// `crates/overdrive-bpf/src/maps/maglev_map.rs`) that need the
    /// table size in const context.
    pub const DEFAULT: Self = Self(DEFAULT_M);

    /// Validating constructor — rejects every value not in
    /// [`ALLOWED_PRIMES`] with a structured
    /// [`ParseError::NotInPrimeList`]. The `M ≥ 100·N` rule is enforced
    /// at backend-set-update time (separate concern; not at
    /// construction).
    pub fn new(value: u32) -> Result<Self, ParseError> {
        // Binary search is sound: ALLOWED_PRIMES is sorted ascending.
        ALLOWED_PRIMES
            .binary_search(&value)
            .map(|_| Self(value))
            .map_err(|_| ParseError::NotInPrimeList { value, allowed: &ALLOWED_PRIMES })
    }

    /// Inner `u32` value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl Default for MaglevTableSize {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for MaglevTableSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for MaglevTableSize {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u32>().map_err(|e| ParseError::Malformed(e.to_string())).and_then(Self::new)
    }
}

impl TryFrom<u32> for MaglevTableSize {
    type Error = ParseError;

    fn try_from(v: u32) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}

impl From<MaglevTableSize> for u32 {
    fn from(v: MaglevTableSize) -> Self {
        v.get()
    }
}
