//! Slice 01 / US-WP-2 AC2 — the journal write does not advance the cursor
//! when the fsync fails (durability write-ordering); slice-01 AC2.
//!
//! Scenario S-WP-01-10. The `WorkflowJournalWriteOrdering` DST invariant
//! (ADR-0064 §6, mirroring ADR-0035 `WriteThroughOrdering`): under a
//! `SimJournalStore` configured to fail the fsync on the next `append`,
//! the engine does NOT advance the journal cursor and does NOT suspend
//! with an unrecorded step acknowledged; on the next boot the journal
//! carries no phantom half-written entry. fsync-then-suspend is
//! load-bearing (ADR-0063 §4).
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! The `WorkflowJournalWriteOrdering` invariant, the engine cursor, and
//! `SimJournalStore`'s injectable fsync-failure do not exist yet (DELIVER
//! slice 01). `#[should_panic(expected = "RED scaffold")]` keeps this
//! RED-not-BROKEN and compiling without the unbuilt types.

#[test]
#[should_panic(expected = "RED scaffold")]
fn fsync_failure_on_append_does_not_advance_cursor_or_suspend_with_unrecorded_step() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-10 / WorkflowJournalWriteOrdering: an fsync failure on append leaves the cursor unadvanced, no suspend with an unrecorded step, no phantom half-written entry on next boot)"
    );
}
