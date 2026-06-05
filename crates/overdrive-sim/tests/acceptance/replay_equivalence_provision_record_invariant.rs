//! Slice 01 / US-WP-4 AC1/AC2/AC3 — replay-equivalence is a named DST
//! invariant on the CI critical path, green from a seed; slice-01 AC3.
//!
//! Scenario S-WP-01-09. **K4 — the load-bearing KPI (O5).** The
//! `replay_equivalence_provision_record` `SimInvariant` (a named enum
//! variant graduating `ReplayEquivalentEmptyWorkflow`,
//! `invariants/mod.rs:136`, no inline string literal — house convention)
//! drives an uninterrupted run, a crash-injected run, and a resumed run,
//! asserting the resumed trajectory is byte-identical to the
//! uninterrupted one (`assert_replay_equivalent!`) AND
//! `assert_eventually!(is_terminal)` within the declared step budget
//! (bounded progress). ADR-0063 §6, ADR-0064 §3/§6.
//!
//! Cross-scenario consistency (journey steps 3↔4): the run prints a seed
//! and reproduces bit-for-bit on a second run — the `trust-the-sim`
//! discipline.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! The graduated `ReplayEquivalenceProvisionRecord` invariant variant +
//! its evaluator (replacing the placeholder
//! `evaluate_replay_equivalent_empty_workflow`) do not exist yet (DELIVER
//! slice 01). `#[should_panic(expected = "RED scaffold")]` keeps this
//! RED-not-BROKEN and compiling without the unbuilt invariant.

#[test]
#[should_panic(expected = "RED scaffold")]
fn replay_equivalence_provision_record_is_a_named_invariant_green_and_seed_reproducible() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-09 / replay_equivalence_provision_record named SimInvariant: replay-equivalent + assert_eventually!(is_terminal) bounded progress, green on CI critical path, seed-reproducible -- K4)"
    );
}
