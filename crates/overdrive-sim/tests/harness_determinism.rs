#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Library-level determinism tests for the DST harness.
//!
//! These tests exercise the in-process `Harness::run(seed)` surface
//! that `xtask dst` consumes. The twin-run acceptance test in
//! `xtask/tests/acceptance/dst_seeded_reproduction.rs` exercises the
//! subprocess boundary; these tests exercise the library boundary so
//! that a determinism regression is caught at the cheapest level.
//!
//! The tests also cover the `canary-bug` feature behaviour end-to-end
//! at the library surface — when the feature is compiled in and the
//! harness sees the canary trigger seed, the LWW convergence invariant
//! must fail. On non-trigger seeds (or without the feature), the
//! invariant must pass so that CI's normal `cargo xtask dst` run
//! remains green.

use overdrive_sim::{Harness, Invariant, InvariantStatus};

/// Two back-to-back runs with the same seed produce equal reports
/// (excluding wall-clock). This is the property the twin-run proptest
/// exercises across 16 seeds; this test pins it for one specific seed
/// so a regression shows up under a plain `cargo test` without
/// requiring proptest to happen to pick the broken case.
#[test]
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

/// The `canary-bug` feature is off by default. Under the default cargo
/// test, the LWW invariant must pass on the canary trigger seed — this
/// pins that the feature gate is respected and production builds are
/// safe.
#[cfg(not(feature = "canary-bug"))]
#[test]
fn default_build_passes_on_canary_trigger_seed() {
    let report = Harness::new().run(0xDEAD_BEEF).expect("harness must compose");
    let lww = report
        .invariants
        .iter()
        .find(|i| i.name == "sim-observation-lww-converges")
        .expect("lww invariant must be in catalogue");
    assert_eq!(
        lww.status,
        InvariantStatus::Pass,
        "without canary-bug, LWW invariant must pass on the trigger seed"
    );
    assert!(report.is_green(), "without canary-bug, seed=0xDEADBEEF must be green");
}

/// With the `canary-bug` feature enabled, the LWW invariant MUST fail
/// on the canary trigger seed. This is what the WS-3 acceptance test
/// subprocess-level asserts on; we mirror the assertion at the library
/// boundary for a cheap determinism check.
#[cfg(feature = "canary-bug")]
#[test]
fn canary_build_fails_on_canary_trigger_seed() {
    let report = Harness::new().run(0xDEAD_BEEF).expect("harness must compose");

    assert!(
        !report.is_green(),
        "canary build on trigger seed must fail; got {:?}",
        report.invariants
    );

    let lww = report
        .invariants
        .iter()
        .find(|i| i.name == "sim-observation-lww-converges")
        .expect("lww invariant must be in catalogue");
    assert_eq!(
        lww.status,
        InvariantStatus::Fail,
        "canary build must make LWW invariant fail on trigger seed; got {lww:?}"
    );
}

/// With the `canary-bug` feature enabled, the canary MUST NOT fire on
/// a non-trigger seed. This is essential for CI — enabling the feature
/// to run WS-3 must not silently break every other seed.
#[cfg(feature = "canary-bug")]
#[test]
fn canary_build_is_green_on_non_trigger_seed() {
    let report = Harness::new().run(42).expect("harness must compose");
    let lww = report
        .invariants
        .iter()
        .find(|i| i.name == "sim-observation-lww-converges")
        .expect("lww invariant must be in catalogue");
    assert_eq!(
        lww.status,
        InvariantStatus::Pass,
        "canary build on non-trigger seed must pass LWW invariant; got {lww:?}"
    );
}
