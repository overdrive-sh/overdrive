//! Wall-clock instants — portable, persistable, DST-replayable.
//!
//! [`UnixInstant`] wraps a `Duration` since `UNIX_EPOCH`. It is the
//! correct type for any deadline that must survive process restart or
//! be persisted to libSQL — `std::time::Instant` cannot, because it is
//! "an opaque type that can only be compared to one another" with no
//! method to extract seconds from.
//!
//! Production code constructs values via [`UnixInstant::from_clock`],
//! which snapshots [`Clock::unix_now`]. Tests and libSQL hydrate paths
//! reconstruct values from a stored `Duration` via
//! [`UnixInstant::from_unix_duration`]. Arithmetic mirrors the
//! `Instant`/`Duration` algebra: `UnixInstant + Duration -> UnixInstant`
//! shifts forward; `UnixInstant - UnixInstant -> Duration` returns the
//! elapsed span (saturating to [`Duration::ZERO`] on a negative diff
//! instead of panicking, matching the research doc § "Recommended
//! call-site shape").
//!
//! See `docs/research/control-plane/issue-139-followup-portable-deadline-representation-research.md`
//! for the full design rationale, including why this type is preferred
//! over `chrono::DateTime`, `jiff::Timestamp`, or persisting an HLC
//! pair.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use crate::traits::clock::Clock;

/// Wall-clock instant expressed as duration since `UNIX_EPOCH`.
///
/// Distinct from [`Duration`] (a span) and from [`std::time::Instant`]
/// (process-local, monotonic, opaque). Persistable via the rkyv
/// derives; portable across process restart; advanceable under DST via
/// [`Clock::unix_now`] (`SimClock` advances `now` and `unix_now` in
/// lockstep from the same elapsed-nanos counter).
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
pub struct UnixInstant(Duration);

impl UnixInstant {
    /// Snapshot the wall-clock from the injected [`Clock`]. The only
    /// production entry point — call sites that need a `UnixInstant`
    /// in `reconcile` read it from `tick.now_unix` (set by the
    /// reconciler runtime via this constructor at evaluation start).
    #[must_use]
    pub fn from_clock<C: Clock + ?Sized>(clock: &C) -> Self {
        Self(clock.unix_now())
    }

    /// Construct from an explicit [`Duration`] since `UNIX_EPOCH`. Used
    /// by tests and by libSQL hydrate paths reconstructing a persisted
    /// row (where the row column stores nanoseconds via
    /// [`UnixInstant::as_unix_duration`] + [`Duration::as_nanos`]).
    #[must_use]
    pub const fn from_unix_duration(d: Duration) -> Self {
        Self(d)
    }

    /// The wrapped [`Duration`] since `UNIX_EPOCH`. Used by libSQL
    /// write paths (extract nanos for the `INTEGER` column) and by
    /// tests asserting on the raw value.
    #[must_use]
    pub const fn as_unix_duration(self) -> Duration {
        self.0
    }
}

impl std::ops::Add<Duration> for UnixInstant {
    type Output = Self;

    fn add(self, d: Duration) -> Self {
        Self(self.0 + d)
    }
}

// -----------------------------------------------------------------------------
// Newtype completeness — `Display`, `FromStr`, `Serialize`,
// `Deserialize`, plus the structured `ParseError` variant set per
// `.claude/rules/development.md` § "Newtype completeness". Mandatory
// proptest call sites in `tests/acceptance/unix_instant_completeness.rs`.
// -----------------------------------------------------------------------------

/// Errors returned by [`UnixInstant::from_str`] for inputs that do not
/// match the canonical decimal form `<seconds>.<nanos>`.
///
/// The variant set is the discrete failure-mode taxonomy required by
/// the development rule "Distinct failure modes get distinct error
/// variants" — every reject branch in `FromStr` produces one of these
/// three variants and no other.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The input was the empty string.
    #[error("UnixInstant cannot be parsed from an empty string")]
    Empty,

    /// The input was non-empty but did not match the
    /// `<digits>(.<digits>)?` shape: a non-digit character outside the
    /// optional decimal point, more than one decimal point, leading
    /// or trailing whitespace, a sign character, an empty fractional
    /// component after the dot, or a fractional component longer than
    /// 9 digits (which would make the canonical Display form
    /// non-injective).
    #[error("UnixInstant input is not a well-formed `<seconds>.<nanos>` decimal")]
    MalformedDecimal,

    /// The seconds component overflowed `u64` (a digit-only string
    /// longer than ~20 digits, or any 20-digit value above
    /// `u64::MAX`). Reserved for the integer-component overflow path;
    /// in practice an input long enough to trip this is also long
    /// enough to look implausible, but the variant exists so the
    /// failure-mode taxonomy is total and a caller `match`-ing on it
    /// can phrase a diagnostic that names the integer part rather
    /// than the fractional part.
    #[error("UnixInstant seconds component overflowed `u64`")]
    SecsOverflow,

    /// The fractional component, after zero-padding to 9 digits,
    /// overflowed `u32`. Reserved for arithmetic-overflow paths even
    /// though, in practice, a 9-digit fractional cannot exceed
    /// `999_999_999 < u32::MAX`; the variant exists to keep the
    /// failure-mode taxonomy total.
    #[error("UnixInstant nanos component overflowed `u32`")]
    NanosOverflow,
}

impl fmt::Display for UnixInstant {
    /// Canonical form: `<seconds>.<nanos>` with **exactly 9 nanos
    /// digits, zero-padded** (e.g. `1700000000.000000123`,
    /// `1700000000.000000000`, `0.000000000`). The 9-digit width is
    /// load-bearing — it makes Display injective over every valid
    /// `Duration` value, which is what makes
    /// `from_str(&u.to_string()) == Ok(u)` total.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:09}", self.0.as_secs(), self.0.subsec_nanos())
    }
}

impl FromStr for UnixInstant {
    type Err = ParseError;

    /// Parse a `UnixInstant` from any of the decimal forms emitted by
    /// peer systems:
    ///
    /// * `1700000000` (no fractional component)
    /// * `1700000000.0` (1-digit fractional)
    /// * `1700000000.000000123` (full 9-digit fractional)
    ///
    /// All forms normalise to the canonical 9-digit form on parse, so
    /// `Display` re-emits a representation that re-parses to the same
    /// value.
    ///
    /// Rejects: empty input (`Empty`); non-digit characters anywhere
    /// outside the single decimal point; leading or trailing
    /// whitespace; a sign character; an empty fractional after `.`;
    /// more than 9 fractional digits; arithmetic overflow on the
    /// seconds or nanos component.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ParseError::Empty);
        }

        // Reject any input containing a character outside `[0-9.]`.
        // This covers signs (`-`, `+`), whitespace, scientific
        // notation, hex prefixes, and every other non-decimal shape
        // a peer system might emit.
        if !s.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return Err(ParseError::MalformedDecimal);
        }

        // Split into seconds + optional fractional. `split_once` on
        // `.` returns at most one split; a second `.` ends up inside
        // the right half and trips the all-digits check on `frac`.
        let (secs_str, frac_str) = match s.split_once('.') {
            Some((secs, frac)) => (secs, Some(frac)),
            None => (s, None),
        };

        // Empty seconds (e.g. ".5") is malformed — the integer part
        // must be present.
        if secs_str.is_empty() {
            return Err(ParseError::MalformedDecimal);
        }

        let secs: u64 = secs_str.parse().map_err(|_| ParseError::SecsOverflow)?;

        let nanos: u32 = match frac_str {
            None => 0,
            Some(frac) => {
                // Empty fractional ("1.") — malformed.
                if frac.is_empty() {
                    return Err(ParseError::MalformedDecimal);
                }
                // Re-check the digits-only constraint on the
                // fractional alone — catches the `1.2.3` shape where
                // `frac == "2.3"` slipped past the top-level
                // all-ascii-digit-or-dot check.
                if !frac.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ParseError::MalformedDecimal);
                }
                // > 9 fractional digits would let two distinct values
                // share one canonical Display form — reject.
                if frac.len() > 9 {
                    return Err(ParseError::MalformedDecimal);
                }
                // Right-pad to 9 digits, then parse. `frac.len() <=
                // 9` and digits-only, so the resulting `u32` is at
                // most `999_999_999` < `u32::MAX`; the
                // `NanosOverflow` arm is unreachable under that
                // guarantee but kept for taxonomic totality.
                let mut padded = String::with_capacity(9);
                padded.push_str(frac);
                for _ in frac.len()..9 {
                    padded.push('0');
                }
                padded.parse().map_err(|_| ParseError::NanosOverflow)?
            }
        };

        Ok(Self(Duration::new(secs, nanos)))
    }
}

// `serde::Serialize` / `Deserialize` delegate to `Display` /
// `FromStr` exactly so the JSON form matches the human-typed form (a
// quoted decimal string, not an object or a bare number). Mixing the
// two wire forms would silently break content-hash determinism for
// any record carrying a `UnixInstant` field.

impl serde::Serialize for UnixInstant {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for UnixInstant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = UnixInstant;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a `<seconds>.<nanos>` decimal string (e.g. `1700000000.000000123`)")
            }

            fn visit_str<E>(self, v: &str) -> Result<UnixInstant, E>
            where
                E: serde::de::Error,
            {
                UnixInstant::from_str(v).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

impl std::ops::Sub<Self> for UnixInstant {
    type Output = Duration;

    /// Returns the elapsed span between two wall-clock instants.
    /// Saturates to [`Duration::ZERO`] when `other > self` rather than
    /// panicking on underflow — the read site at
    /// `tick.now_unix - view.last_failure_seen_at` must not panic
    /// when the seen-at timestamp is in the future relative to the
    /// current tick (a possibility under DST clock skew or under
    /// adversarial gossip).
    fn sub(self, other: Self) -> Duration {
        self.0.checked_sub(other.0).unwrap_or(Duration::ZERO)
    }
}
