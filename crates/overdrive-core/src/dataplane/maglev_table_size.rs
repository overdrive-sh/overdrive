//! `MaglevTableSize` — STRICT newtype for the Maglev permutation table size.
//!
//! Constrained to Cilium's prime list per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 6.
//! Default `M = 16_381`; the `M ≥ 100·N` rule is enforced at backend-
//! set-update time (separate concern; not at construction).
//!
//! **RED scaffold** — every body panics until DELIVER fills it
//! (Slice 04 per the carpaccio plan). The DESIGN-locked code shape
//! lives in `architecture.md` § 6 *MaglevTableSize*; DELIVER
//! transcribes it.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use core::fmt;
use std::str::FromStr;

/// Allowed Maglev table sizes, locked to Cilium's prime list per
/// research § 5.2.
pub const ALLOWED_PRIMES: [u32; 10] = [
    251, 509, 1_021, 2_039, 4_093, 8_191, 16_381, 32_749, 65_521, 131_071,
];

/// Default Maglev `M` — smallest prime ≥ 16_384; matches Cilium /
/// Katran. Supports up to ~160 backends per the M ≥ 100·N rule.
pub const DEFAULT_M: u32 = 16_381;

/// Maglev permutation table size. Constrained to
/// [`ALLOWED_PRIMES`].
///
/// Constructed via [`MaglevTableSize::new`] (validating) or
/// [`MaglevTableSize::default`] (yields [`DEFAULT_M`]). Raw `u32`
/// for any persisted Maglev table-size field is a blocking
/// violation per `.claude/rules/development.md` § Newtype
/// completeness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaglevTableSize(u32);

/// Parse / validation failure for [`MaglevTableSize`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Provided value is not in [`ALLOWED_PRIMES`].
    #[error("MaglevTableSize {value} not in Cilium prime list {ALLOWED_PRIMES:?}")]
    NotInPrimeList { value: u32 },

    /// String could not be parsed to a `u32`.
    #[error("MaglevTableSize parse failed: {0}")]
    Malformed(String),
}

impl MaglevTableSize {
    /// Default Maglev `M` — wraps [`DEFAULT_M`].
    pub const DEFAULT: Self = Self(DEFAULT_M);

    /// Validating constructor — rejects every value not in
    /// [`ALLOWED_PRIMES`].
    pub fn new(_value: u32) -> Result<Self, ParseError> {
        // RED scaffold — DELIVER fills this body per Slice 04.
        // See test-scenarios.md S-2.2-12 (Maglev determinism property).
        todo!("RED scaffold: MaglevTableSize::new — see Slice 04 / S-2.2-12")
    }

    /// Inner `u32` value.
    pub fn get(self) -> u32 {
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

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        // RED scaffold — DELIVER fills this body per Slice 04.
        todo!("RED scaffold: MaglevTableSize::from_str — see Slice 04 / S-2.2-12")
    }
}

impl TryFrom<u32> for MaglevTableSize {
    type Error = ParseError;

    fn try_from(_v: u32) -> Result<Self, Self::Error> {
        todo!("RED scaffold: MaglevTableSize::try_from<u32>")
    }
}

impl From<MaglevTableSize> for u32 {
    fn from(v: MaglevTableSize) -> Self {
        v.get()
    }
}
