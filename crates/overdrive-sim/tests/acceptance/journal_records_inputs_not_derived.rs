//! Slice 01 / US-WP-2 AC3 — the journal records step inputs/results,
//! not a derived deadline cache.
//!
//! Scenario S-WP-01-05. O6. Per `development.md` "Persist inputs, not
//! derived state": the recorded `JournalEntry` carries the step's
//! inputs/result digest and no derived-deadline / "remaining" field.
//! ADR-0063 §2 (`CallResult { step, correlation, response_digest }`).
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! `SimJournalStore`, the `JournalEntry` enum, and a `ProvisionRecord`
//! consumer do not exist yet (DELIVER slice 01). The `#[should_panic
//! (expected = "RED scaffold")]` body keeps this RED-not-BROKEN and
//! compiling without importing those unbuilt types.

#[test]
#[should_panic(expected = "RED scaffold")]
fn provision_record_journal_entry_records_inputs_not_a_derived_cache() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-05 / the CallResult journal entry carries the step input/result digest and no derived-deadline cache field)"
    );
}
