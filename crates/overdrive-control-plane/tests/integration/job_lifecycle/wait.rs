//! Wait helpers for Tier-3 integration tests that drive the
//! **spawned** convergence loop via `SimClock` (e.g.
//! `convergence_loop_spawned_in_production_boot`).
//!
//! Tests that drive `run_convergence_tick` directly
//! (`submit_to_running.rs`) do NOT need this — those run the tick
//! synchronously on the test task and observe state immediately
//! after each `.await`.
//!
//! The spawned-loop case is different: each logical `SimClock::tick`
//! wakes the parked `clock.sleep(cadence)` future inside the
//! convergence task, but the convergence task then runs a multi-
//! await drain (`hydrate_desired` + `hydrate_actual` + `persist_view`
//! + `action_shim::dispatch` + `yield_now`). With only one
//! `yield_now` between ticks the test task can resume before the
//! convergence task finishes its drain — under Lima FS contention
//! against `RedbViewStore` this is the difference between "passes
//! 10/10 in isolation" and "flakes under the 207-test suite".
//!
//! `advance_and_settle` advances logical time and then **really**
//! sleeps the test task on the wall clock. A real sleep parks the
//! test task with the runtime for a fixed duration, during which the
//! scheduler can run the spawned convergence task to completion (and
//! re-park it) regardless of how many `.await` boundaries the drain
//! crosses. This is preferable to spinning `yield_now` N times: yield
//! only re-enters the scheduler, and ready CPU-bound peer tasks
//! (axum accept loop, per-connection handlers) can keep returning to
//! the test before the spawned loop gets enough sequential
//! scheduling time.
//!
//! This helper is **only** for Tier-3 integration tests that already
//! do real network I/O and are wall-clock dependent. DST tests
//! (Tier 1) MUST NOT use it — they continue to drive `SimClock`
//! deterministically per `.claude/rules/testing.md` § "Sources of
//! Nondeterminism".

use std::time::Duration;

use overdrive_sim::adapters::clock::SimClock;

/// Wall-clock pause after each `SimClock::tick`. Sized empirically to
/// cover the widest `.await` surface the spawned convergence loop
/// traverses per tick (5+ awaits in `run_convergence_tick` plus 1 in
/// `action_shim::dispatch_single`) with safety margin under Lima FS
/// contention; the happy path completes well under this budget so
/// tests typically break out of the polling loop within the first
/// 1–2 iterations.
pub const SETTLE_AFTER_TICK: Duration = Duration::from_millis(20);

/// Advance `SimClock` by `cadence`, then `tokio::time::sleep` for
/// [`SETTLE_AFTER_TICK`] so the spawned convergence task gets real
/// scheduling time to wake, drain, run a full tick, and re-park.
///
/// Call inside a `for _ in 0..MAX_TICKS { … }` polling loop where the
/// test asserts on observable state (HTTP endpoint, `ObservationStore`
/// row, broker counter) between calls.
pub async fn advance_and_settle(clock: &SimClock, cadence: Duration) {
    clock.tick(cadence);
    tokio::time::sleep(SETTLE_AFTER_TICK).await;
}
