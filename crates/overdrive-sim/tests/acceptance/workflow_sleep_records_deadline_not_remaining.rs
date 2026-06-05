//! Slice 02 / AC4 — the sleep journal entry records the deadline (an
//! input), never a "remaining" cache.
//!
//! Scenario S-WP-02-03. O3/O6. Per `development.md` "Persist inputs, not
//! derived state": the `SleepArmed` entry carries `deadline_unix` (an
//! input) and no persisted "remaining duration" field — resume recomputes
//! `recorded_deadline − clock.now()`. ADR-0063 §2.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! The `SleepArmed` journal variant does not exist yet (DELIVER slice
//! 02). `#[should_panic(expected = "RED scaffold")]` keeps this RED-not-
//! BROKEN and compiling.

#[test]
#[should_panic(expected = "RED scaffold")]
fn sleep_armed_journal_entry_records_deadline_input_not_a_remaining_duration_cache() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-02-03 / SleepArmed records deadline_unix as an input and no persisted remaining-duration cache field -- persist inputs, not derived state)"
    );
}
