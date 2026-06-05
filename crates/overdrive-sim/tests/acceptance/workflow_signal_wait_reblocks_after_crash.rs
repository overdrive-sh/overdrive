//! Slice 03 / US-WP-5 AC1 — a sequence blocked on a signal re-blocks on
//! the SAME signal after a crash; slice-03 AC1.
//!
//! Scenario S-WP-03-01. K1 (O1). Under DST a sequence blocked on
//! `ctx.wait_for_signal(key)` (signal NOT yet present in the
//! `ObservationStore`) is killed while blocked and restarted on the same
//! node; on resume it blocks on the SAME signal (neither lost nor
//! satisfied prematurely) and no duplicate downstream effect occurs.
//! ADR-0063 §2 (`SignalAwaited`), ADR-0064 §3.
//!
//! SINGLE-NODE SCOPE (D3 / #205): process-local kill/restart; in-process
//! single-node signal delivery via the `ObservationStore` (#207 defers
//! cross-node partition semantics).
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! `ctx.wait_for_signal`, the `SignalAwaited` variant, and the typed
//! `ObservationStore` signal surface do not exist yet (DELIVER slice 03).
//! `#[should_panic(expected = "RED scaffold")]` keeps this RED-not-BROKEN
//! and compiling.

#[test]
#[should_panic(expected = "RED scaffold")]
fn crash_while_blocked_on_signal_reblocks_on_the_same_signal_on_resume() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-03-01 / crash while blocked on ctx.wait_for_signal: on resume the workflow blocks on the SAME signal, no lost wait, no duplicate downstream effect)"
    );
}
