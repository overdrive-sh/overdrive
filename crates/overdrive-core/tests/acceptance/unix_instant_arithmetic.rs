//! Acceptance scenarios for issue #141 step 01-01 — `UnixInstant`
//! newtype arithmetic and clock-construction.
//!
//! Translates the canonical "Recommended call-site shape" skeleton from
//! `docs/research/control-plane/issue-139-followup-portable-deadline-representation-research.md`
//! § "Recommended call-site shape" directly into Rust `#[test]` bodies.
//!
//! Port-to-port at domain scope: the newtype's public signature IS its
//! driving port (per `~/.claude/skills/nw-tdd-methodology/SKILL.md` —
//! "Pure domain functions are their own driving ports").
//!
//! Step 01-01 covers ONLY the arithmetic + constructor surface. The
//! proptest roundtrip + rkyv roundtrip + `Display` / `FromStr` / serde
//! coverage land in step 01-02.
//!
//! The `from_clock` test uses `SimClock` from `overdrive-sim` rather than a
//! hand-rolled in-test stub: the production entry point `from_clock`
//! consumes the `Clock` trait, and the project's standard test fixture
//! for that trait IS `SimClock`. Using it here keeps the test fixture
//! aligned with how every other reconciler test in the workspace
//! exercises clock-shaped boundaries.

use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::traits::clock::Clock;
use overdrive_sim::adapters::clock::SimClock;

// -----------------------------------------------------------------------------
// Arithmetic — `UnixInstant + Duration` returns a forward-shifted
// `UnixInstant`.
// -----------------------------------------------------------------------------

#[test]
fn add_duration_shifts_unix_instant_forward() {
    // Given a `UnixInstant` 10 s past the epoch.
    let base = UnixInstant::from_unix_duration(Duration::from_secs(10));

    // When 5 s is added.
    let shifted = base + Duration::from_secs(5);

    // Then the result equals a `UnixInstant` 15 s past the epoch.
    assert_eq!(shifted, UnixInstant::from_unix_duration(Duration::from_secs(15)));
}

// -----------------------------------------------------------------------------
// Arithmetic — `UnixInstant - UnixInstant` returns the elapsed
// `Duration` between them.
// -----------------------------------------------------------------------------

#[test]
fn sub_returns_duration_between_unix_instants() {
    // Given two `UnixInstant`s 15 s and 10 s past the epoch.
    let later = UnixInstant::from_unix_duration(Duration::from_secs(15));
    let earlier = UnixInstant::from_unix_duration(Duration::from_secs(10));

    // When the earlier is subtracted from the later.
    let elapsed = later - earlier;

    // Then the result equals the difference as a `Duration`.
    assert_eq!(elapsed, Duration::from_secs(5));
}

// -----------------------------------------------------------------------------
// Arithmetic — saturating subtraction returns `Duration::ZERO` rather
// than panicking when the right-hand side is greater. Per research
// doc § "Recommended call-site shape" — `checked_sub.unwrap_or(ZERO)`.
// -----------------------------------------------------------------------------

#[test]
fn sub_saturates_to_zero_when_rhs_is_greater() {
    // Given a `UnixInstant` 5 s past the epoch and another 15 s past.
    let earlier = UnixInstant::from_unix_duration(Duration::from_secs(5));
    let later = UnixInstant::from_unix_duration(Duration::from_secs(15));

    // When the larger is subtracted from the smaller.
    let elapsed = earlier - later;

    // Then the result saturates to `Duration::ZERO` instead of
    // panicking on the underflow.
    assert_eq!(elapsed, Duration::ZERO);
}

// -----------------------------------------------------------------------------
// Construction — `from_clock` snapshots `Clock::unix_now()`. Using
// `SimClock` as the fixture: both sides of the assertion read the same
// underlying counter, so the assertion holds regardless of any
// wall-clock skew between calls.
// -----------------------------------------------------------------------------

#[test]
fn from_clock_snapshots_clock_unix_now() {
    // Given a `SimClock` advanced to a known logical time.
    let clock = SimClock::new();
    clock.tick(Duration::from_secs(7));

    // When a `UnixInstant` is snapshotted from the clock.
    let snapshot = UnixInstant::from_clock(&clock);

    // Then `as_unix_duration()` equals `clock.unix_now()` at the moment
    // of capture. (SimClock advances `now()` and `unix_now()` in
    // lockstep from the same elapsed-nanos counter, so two reads at the
    // same logical time produce the same value.)
    assert_eq!(snapshot.as_unix_duration(), clock.unix_now());
}

// -----------------------------------------------------------------------------
// Construction — `from_unix_duration` round-trips through
// `as_unix_duration` losslessly. This is the libSQL-hydrate entry point
// (research doc § "Recommended call-site shape": "used by tests and by
// libSQL hydrate paths reconstructing a persisted row").
// -----------------------------------------------------------------------------

#[test]
fn from_unix_duration_round_trips_through_as_unix_duration() {
    // Given a `Duration` representing some persisted nanosecond count.
    let persisted = Duration::from_nanos(1_234_567_890);

    // When a `UnixInstant` is reconstructed from it and then read back.
    let instant = UnixInstant::from_unix_duration(persisted);
    let read_back = instant.as_unix_duration();

    // Then the round-trip is lossless.
    assert_eq!(read_back, persisted);
}
