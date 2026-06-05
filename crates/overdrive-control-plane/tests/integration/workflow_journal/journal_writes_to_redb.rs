//! Slice 01 / US-WP-2 AC1 (O6) / AC2 (ordering) — `@real-io`: a completed
//! step is recorded in the **real redb** journal before the run suspends,
//! and no libSQL journal table exists.
//!
//! Scenario S-WP-01-04. **K5 (O6).** This is the one journal-persistence
//! scenario that exercises a REAL redb file (the `RedbJournalStore`
//! sharing the reconciler redb file + `Arc<Database>`), per
//! `.claude/rules/testing.md` § "Integration vs unit gating": real
//! filesystem I/O (opening a real redb file) MUST be gated behind the
//! `integration-tests` feature and live under `tests/integration/`. The
//! recorded `CallResult` entry is present in the redb journal when read
//! back through the journal handle (the bytes written are the bytes read
//! — `journal_checkpoint` consistency, journey steps 2↔3), and a
//! grep/dep-graph check confirms no libSQL journal table. ADR-0063 §1/§3.
//!
//! Per Mandate 11, this layer-3 sad path / persistence scenario is
//! example-based (one representative real-redb roundtrip), NOT PBT-
//! generated.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! `RedbJournalStore`, the `JournalStore` port, and the `JournalEntry`
//! codec do not exist yet (DELIVER slice 01). Per the project RED-
//! scaffold convention the body is a `panic!` naming the scenario, gated
//! by `#[should_panic(expected = "RED scaffold")]`, so the integration
//! test COMPILES and PASSES at the bar without importing the unbuilt
//! types or touching a real redb file before the production code exists.

#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn call_result_is_present_in_the_real_redb_journal_and_no_libsql_table_exists() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-04 / a recorded CallResult is read back from a REAL RedbJournalStore -- bytes written == bytes read -- and no libSQL journal table exists (K5))"
    );
}
