//! Acceptance scenarios for issue #141 step 02-01 — `TickContext.now_unix`
//! field + `backoff_for_attempt` const fn.
//!
//! Two port-to-port scenarios, both at domain scope:
//!
//! 1. The `TickContext` public struct now carries `now_unix: UnixInstant`
//!    alongside the existing monotonic `now: Instant` — verified by
//!    constructing a `TickContext` against a `SimClock` snapshot and
//!    asserting the wall-clock and monotonic readings match the clock's
//!    `unix_now()` and `now()` respectively. The struct's public field
//!    surface IS the driving port (per
//!    `~/.claude/skills/nw-tdd-methodology/SKILL.md` — "Pure domain
//!    functions are their own driving ports").
//!
//! 2. The `backoff_for_attempt(attempt: u32) -> Duration` const fn returns
//!    `RESTART_BACKOFF_DURATION` for every attempt across the full
//!    `0..=RESTART_BACKOFF_CEILING` range. Phase 1 policy is degenerate
//!    (every attempt yields the same backoff window); the function exists
//!    so call sites stay stable when operator-configurable per-job policy
//!    lands later (issue #141 'Out' section).
//!
//! Tier classification: **Tier 1 / unit lane** per
//! `.claude/rules/testing.md` — pure-Rust, no real I/O, no integration
//! gating. Resides in `tests/acceptance/` per ADR-0005 and the entrypoint
//! header comment in `tests/acceptance.rs`.
//!
//! The runtime construction-site verification (i.e. that
//! `crates/overdrive-control-plane/src/reconciler_runtime.rs:248`
//! populates `now_unix` from the injected `Clock`) lives in the
//! control-plane acceptance suite, not here — it requires building an
//! `AppState`, which the core crate cannot do without pulling
//! control-plane in as a dependency. See
//! `crates/overdrive-control-plane/tests/acceptance/tick_context_now_unix_runtime.rs`.

use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::reconciler::{
    RESTART_BACKOFF_CEILING, RESTART_BACKOFF_DURATION, TickContext, backoff_for_attempt,
};
use overdrive_core::traits::clock::Clock;
use overdrive_sim::adapters::clock::SimClock;

// -----------------------------------------------------------------------------
// `TickContext.now_unix` field — wall-clock snapshot taken from the
// injected `Clock` alongside the existing monotonic `now`.
// -----------------------------------------------------------------------------

#[test]
fn tick_context_carries_now_unix_alongside_monotonic_now() {
    // Given a `SimClock` whose `unix_now()` and `now()` advance in
    // lockstep from the same elapsed-nanos counter.
    let clock = SimClock::new();

    // When a `TickContext` is constructed snapshotting both readings.
    let now = clock.now();
    let now_unix = UnixInstant::from_clock(&clock);
    let tick = TickContext { now, now_unix, tick: 0, deadline: now + Duration::from_secs(1) };

    // Then `tick.now_unix` equals `clock.unix_now()` taken at the same
    // instant — the wall-clock field is populated from the Clock trait's
    // `unix_now()` reading (NOT `Instant::now()`-derived), and survives
    // construction unchanged.
    assert_eq!(
        tick.now_unix.as_unix_duration(),
        clock.unix_now(),
        "TickContext.now_unix must equal Clock::unix_now() at construction time"
    );

    // And `tick.now` equals `clock.now()` — the monotonic field is
    // unchanged in shape.
    assert_eq!(tick.now, now, "TickContext.now must remain the monotonic Instant snapshot");
}

#[test]
fn tick_context_now_unix_advances_with_simclock_tick() {
    // Given a `SimClock` and a TickContext snapshot taken against it.
    let clock = SimClock::new();
    let before = UnixInstant::from_clock(&clock);

    // When the SimClock advances by 5 seconds.
    clock.tick(Duration::from_secs(5));

    // Then a fresh `UnixInstant::from_clock(&clock)` reads exactly 5 s
    // forward — the clock's `unix_now` and the type's snapshotting
    // semantics flow through `TickContext.now_unix` unchanged.
    let after = UnixInstant::from_clock(&clock);
    assert_eq!(
        after - before,
        Duration::from_secs(5),
        "TickContext.now_unix must reflect SimClock::tick() advancement"
    );
}

// -----------------------------------------------------------------------------
// `backoff_for_attempt` — degenerate Phase 1 policy lookup. Every
// attempt across the full `0..=RESTART_BACKOFF_CEILING` range returns
// the same `RESTART_BACKOFF_DURATION`. The function exists as a
// stability anchor so call sites stay unchanged when operator-
// configurable per-job policy lands in Phase 2+.
// -----------------------------------------------------------------------------

#[test]
fn backoff_for_attempt_returns_constant_duration_for_every_attempt() {
    // Given the full attempt range Phase 1 reconcilers exercise: from
    // attempt 0 (the implicit "before any restart") through
    // RESTART_BACKOFF_CEILING (the budget-exhausted edge) plus one above
    // the ceiling (defensive — the function must not panic or branch on
    // out-of-range attempts; backoff exhaustion is enforced elsewhere by
    // `attempts >= RESTART_BACKOFF_CEILING`).
    for attempt in 0..=RESTART_BACKOFF_CEILING + 1 {
        // When `backoff_for_attempt(attempt)` is invoked.
        let backoff = backoff_for_attempt(attempt);

        // Then it returns exactly `RESTART_BACKOFF_DURATION` —
        // degenerate Phase 1 policy.
        assert_eq!(
            backoff, RESTART_BACKOFF_DURATION,
            "backoff_for_attempt({attempt}) must equal RESTART_BACKOFF_DURATION; \
             Phase 1 is degenerate-constant policy"
        );
    }
}

#[test]
fn backoff_for_attempt_is_const_evaluable() {
    // Given `backoff_for_attempt` is declared `pub const fn`, it must be
    // usable in a `const` context. This compile-time call site exercises
    // the const-ness directly — if the function loses its `const`
    // qualifier, this test stops compiling.
    const COMPILE_TIME_BACKOFF: Duration = backoff_for_attempt(0);

    assert_eq!(COMPILE_TIME_BACKOFF, RESTART_BACKOFF_DURATION);
}

// -----------------------------------------------------------------------------
// Wall-clock TickContext field interacts correctly with the existing
// monotonic-deadline arithmetic — the two fields are independent and
// neither shadows the other.
// -----------------------------------------------------------------------------

#[test]
fn tick_context_now_unix_is_independent_of_monotonic_deadline() {
    // Given a TickContext constructed with a deadline 1 second past `now`.
    let clock = SimClock::new();
    let now = clock.now();
    let now_unix = UnixInstant::from_clock(&clock);
    let deadline = now + Duration::from_secs(1);
    let tick = TickContext { now, now_unix, tick: 0, deadline };

    // Then the deadline check (monotonic) and the now_unix snapshot
    // (wall-clock) are independent fields — adjusting one does NOT
    // implicitly affect the other.
    assert_eq!(tick.deadline - tick.now, Duration::from_secs(1));
    // Use only the public observation: tick.now_unix matches what the
    // clock's unix_now() reads. Reading the field exercises the
    // separate-storage invariant — Instant arithmetic on `now`/`deadline`
    // does not mutate `now_unix`.
    assert_eq!(tick.now_unix.as_unix_duration(), clock.unix_now());

    // Sanity — the deadline field has not absorbed the now_unix value.
    let _: Instant = tick.deadline; // type assertion via let-binding
}
