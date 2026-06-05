//! Slice 02 / AC2 — the post-sleep step fires only at/after the original
//! deadline, regardless of crash timing.
//!
//! Scenario S-WP-02-02. K3 (O4). A sequence suspended on `ctx.sleep` with
//! a recorded deadline is crashed at an arbitrary point in the sleep
//! window and resumed (`SimClock` advances logical time); the post-sleep
//! `ctx.call` fires only at/after the ORIGINAL deadline, never earlier,
//! and the terminal result is unchanged by the crash timing. ADR-0063 §2
//! (`SleepArmed { deadline_unix }`), ADR-0064 §3.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! `ctx.sleep` deadline-park + resume-recompute do not exist yet (DELIVER
//! slice 02). `#[should_panic(expected = "RED scaffold")]` keeps this
//! RED-not-BROKEN and compiling.

#[test]
#[should_panic(expected = "RED scaffold")]
fn post_sleep_step_fires_only_at_or_after_the_original_deadline_regardless_of_crash_timing() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-02-02 / post-sleep ctx.call fires only at/after the original recorded deadline regardless of when the crash occurred; terminal unchanged by crash timing)"
    );
}
