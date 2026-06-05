//! Slice 02 / AC1 — a waiting sequence survives a crash spanning the
//! sleep window without repeating the pre-sleep step.
//!
//! Scenario S-WP-02-01. K1 (O1). Under DST a `ctx.call → ctx.sleep →
//! ctx.call` sequence is killed DURING the sleep window and restarted on
//! the same node; the pre-sleep `ctx.call` executes exactly once on
//! resume (`SimTransport` call count == 1) and the sequence resumes the
//! remaining wait, not the whole sleep. ADR-0064 §3/§4 (`SleepArmed`
//! check-then-record).
//!
//! SINGLE-NODE SCOPE (D3 / #205): process-local kill/restart on one node.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! `ctx.sleep`, the `SleepArmed` journal variant, and the extended
//! 3-await consumer do not exist yet (DELIVER slice 02). `#[should_panic
//! (expected = "RED scaffold")]` keeps this RED-not-BROKEN and compiling.

#[test]
#[should_panic(expected = "RED scaffold")]
fn crash_during_sleep_window_does_not_repeat_the_pre_sleep_step() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-02-01 / crash during the ctx.sleep window: pre-sleep ctx.call executes once on resume, the sequence resumes the remaining wait not the whole sleep)"
    );
}
