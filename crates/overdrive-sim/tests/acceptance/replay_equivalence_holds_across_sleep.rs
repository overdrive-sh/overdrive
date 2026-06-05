//! Slice 02 / AC3 — replay-equivalence holds across the sleep, seeded and
//! reproducible.
//!
//! Scenario S-WP-02-04. K4 (O5). The `replay_equivalence_*` invariant
//! extended over the 3-await `ctx.call → ctx.sleep → ctx.call` shape:
//! replaying the journal across the sleep produces a bit-identical
//! trajectory, green on the CI critical path, reproducing bit-for-bit
//! from the printed seed. ADR-0064 §6.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! The sleep-extended replay invariant does not exist yet (DELIVER slice
//! 02). `#[should_panic(expected = "RED scaffold")]` keeps this RED-not-
//! BROKEN and compiling.

#[test]
#[should_panic(expected = "RED scaffold")]
fn replay_equivalence_holds_across_a_durable_sleep_seeded_and_reproducible() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-02-04 / replay_equivalence_* invariant is bit-identical across a ctx.sleep, green on CI critical path, seed-reproducible)"
    );
}
