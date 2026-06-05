//! Slice 03 / US-WP-5 AC4 — replay-equivalence holds across a signal wait
//! and an emit, seeded and reproducible; slice-03 AC4.
//!
//! Scenario S-WP-03-05. K4 (O5). The `replay_equivalence_*` invariant
//! extended over a `ctx.wait_for_signal → ctx.emit_action → terminal`
//! shape: replaying the journal across the signal wait + emit produces a
//! bit-identical trajectory, green on the CI critical path, reproducing
//! bit-for-bit from the printed seed. ADR-0064 §6.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! The signal+emit-extended replay invariant does not exist yet (DELIVER
//! slice 03). `#[should_panic(expected = "RED scaffold")]` keeps this
//! RED-not-BROKEN and compiling.

#[test]
#[should_panic(expected = "RED scaffold")]
fn replay_equivalence_holds_across_a_signal_wait_and_an_emit_seeded_and_reproducible() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-03-05 / replay_equivalence_* invariant is bit-identical across a ctx.wait_for_signal + ctx.emit_action, green on CI critical path, seed-reproducible)"
    );
}
