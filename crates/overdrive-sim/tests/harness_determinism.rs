#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Library-level determinism tests for the DST harness.
//!
//! These tests exercise the in-process `Harness::run(seed)` surface
//! that `xtask dst` consumes. The twin-run acceptance test in
//! `xtask/tests/acceptance/dst_seeded_reproduction.rs` exercises the
//! subprocess boundary; these tests exercise the library boundary so
//! that a determinism regression is caught at the cheapest level.

use overdrive_sim::{Harness, Invariant};

/// Two back-to-back runs with the same seed produce equal reports
/// (excluding wall-clock). This is the property the twin-run proptest
/// exercises across 16 seeds; this test pins it for one specific seed
/// so a regression shows up under a plain `cargo test` without
/// requiring proptest to happen to pick the broken case.
///
/// Downstream fallout: `Harness::new().run(...)` walks the full
/// invariant catalogue including the `HydratorEventuallyConverges`
/// RED scaffold (DISTILL wave 5e9ca73). `#[should_panic]` per
/// `.claude/rules/testing.md` § "Downstream fallout on pre-existing
/// tests" until step 08-NN lands the impl.
#[test]
#[should_panic(expected = "RED scaffold")]
fn harness_run_is_deterministic_under_fixed_seed() {
    let a = Harness::new().run(42).expect("harness must compose");
    let b = Harness::new().run(42).expect("harness must compose");

    assert_eq!(a.seed, b.seed, "seed echoes back unchanged");
    assert_eq!(
        a.invariants.len(),
        b.invariants.len(),
        "same seed must produce same invariant count"
    );

    for (x, y) in a.invariants.iter().zip(b.invariants.iter()) {
        assert_eq!(x.name, y.name, "invariant names must match in order");
        assert_eq!(x.status, y.status, "invariant status must match");
        assert_eq!(x.tick, y.tick, "per-invariant tick must match");
        assert_eq!(x.host, y.host, "per-invariant host must match");
        assert_eq!(x.cause, y.cause, "per-invariant cause must match");
    }

    assert_eq!(a.failures, b.failures, "failures vector must match byte-for-byte");
}

/// The `--only` narrowing composes with the determinism claim — same
/// seed, same invariant, same result across two runs.
#[test]
fn harness_run_is_deterministic_under_only_narrowing() {
    let a = Harness::new()
        .only(Invariant::SimObservationLwwConverges)
        .run(7)
        .expect("harness must compose");
    let b = Harness::new()
        .only(Invariant::SimObservationLwwConverges)
        .run(7)
        .expect("harness must compose");

    assert_eq!(a.invariants, b.invariants);
    assert_eq!(a.failures, b.failures);
}
