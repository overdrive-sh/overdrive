//! Slice 01 / US-WP-3 AC2 — a committed step survives the crash (not
//! lost) on resume; slice-01 AC2.
//!
//! Scenario S-WP-01-07. K2 (O2, single-node). The recorded step's result
//! is read back from the redb journal on resume (committed step NOT
//! lost), and the resumed run continues from the first unrecorded await,
//! not from the top. ADR-0064 §3 (replay buffer; check-then-record).
//!
//! Cross-scenario consistency (journey steps 2↔3): the bytes read here
//! are the bytes S-WP-01-04 wrote to the real redb journal.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! Engine replay + `SimJournalStore` do not exist yet (DELIVER slice 01).
//! `#[should_panic(expected = "RED scaffold")]` keeps this RED-not-BROKEN
//! and compiling without the unbuilt types.

#[test]
#[should_panic(expected = "RED scaffold")]
fn committed_step_is_read_back_from_journal_and_run_resumes_from_first_unrecorded_await() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-07 / a committed ctx.call step is read back from the redb journal on resume -- not lost -- and the run continues from the first unrecorded await)"
    );
}
