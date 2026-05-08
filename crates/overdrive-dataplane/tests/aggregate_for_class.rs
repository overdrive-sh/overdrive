//! Unit-shape test for `overdrive_dataplane::maps::drop_counter_handle::aggregate_for_class`.
//!
//! Closes the 2 missed mutations the `QUALITY_GATE` wave's mutation run flagged:
//! - line 44: `replace aggregate_for_class -> u64 with 0`
//! - line 44: `replace aggregate_for_class -> u64 with 1`
//!
//! The function delegates to `overdrive_core::dataplane::aggregate_per_cpu`,
//! which is also re-exported. Mutating the wrapper body to a constant `0`
//! or `1` would make any test that asserts a non-{0, 1} sum fail. The
//! assertion below pins both: the sum exceeds 1 for non-trivial inputs,
//! and the sum is sensitive to per-CPU value differences.

#![allow(clippy::expect_used)]

use overdrive_core::dataplane::DropClass;
use overdrive_dataplane::maps::drop_counter_handle::aggregate_for_class;

#[test]
fn aggregate_for_class_sums_per_cpu_slots_above_unity() {
    // Construct a per-CPU slice with non-trivial non-zero values.
    let per_cpu: [u64; 4] = [10, 20, 30, 40];
    // Real implementation: 10 + 20 + 30 + 40 = 100.
    // `Ok(0)` / `Ok(1)` body mutations would fail this assertion.
    let total = aggregate_for_class(DropClass::MalformedHeader, &per_cpu);
    assert_eq!(total, 100, "real aggregate must sum slots; got {total}");
    // Defensive: the value MUST exceed both replacement bodies (0 and 1).
    assert!(total > 1, "aggregate must exceed 1 to discriminate `with 1` mutation");
}

#[test]
fn aggregate_for_class_handles_empty_slice() {
    // Empty slice returns 0 — trivially equal to the `with 0` mutation,
    // but every other test below pins the non-zero discriminator.
    let total = aggregate_for_class(DropClass::UnknownVip, &[]);
    assert_eq!(total, 0);
}

#[test]
fn aggregate_for_class_is_sensitive_to_input_changes() {
    // Two distinct inputs MUST produce distinct outputs. A constant
    // body (`Ok(0)` / `Ok(1)`) cannot distinguish two different inputs.
    let total_a = aggregate_for_class(DropClass::NoHealthyBackend, &[5_u64]);
    let total_b = aggregate_for_class(DropClass::NoHealthyBackend, &[7_u64]);
    assert_ne!(total_a, total_b, "real aggregate must reflect input change");
    assert_eq!(total_a, 5);
    assert_eq!(total_b, 7);
}
