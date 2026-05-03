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
