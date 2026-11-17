//! The build-time rules of the `overdrive_bench` attribute (ADR-0297, DDD-2/DDD-4):
//! each names the offending value or the missing declaration, so an author reads the
//! instruction in the compiler output.
//!
//! The `error:` message line of every fixture's expected output is the contract DDD-2
//! pins (the table of rejections); the `-->`, source and label lines are rustc's rendering
//! of the macro's spans, written here as the expected shape and regenerated once with
//! `TRYBUILD=overwrite` when the attribute lands (review that every message line is
//! unchanged). Substrate and `smoke(none)` rejections are constant evaluations, so rustc
//! prints them as `error[E0080]: evaluation panicked: <message>`.
//! Driving port: the attribute. Evidence lane: trybuild (compile-fail).
//! Placement: each is a trybuild run that builds nothing of the platform, so `host_pure`.
//! Traces: US-BENCH-01 AC (e), US-BENCH-02 AC (d), US-BENCH-04 AC (c), DDD-2, DDD-4.
//!
//! Pending state. While the attribute is a passthrough every fixture compiles, so trybuild reports
//! `Expected test case to fail to compile, but it succeeded` for each and ends with
//! `<n> of <n> tests failed`; each test pends on that summary with its own fixture count, which
//! excludes a setup failure (trybuild's bare `tests failed`) and a partial failure. trybuild's panic
//! carries no per-fixture text, so this RED is weaker than the other tests' (see `red-classification.md`).
//! DELIVER activation: remove the `should_panic` of a test in the step that lands the attribute,
//! regenerate the `.stderr` span lines once with `TRYBUILD=overwrite`, and verify that every pinned
//! `error:` line of the regenerated files is unchanged (`git diff` shows only span and label lines).

#![cfg(target_os = "linux")]
#![allow(clippy::doc_markdown, reason = "Contract Shape declarations keep their literal spelling")]

use overdrive_testing::overdrive_test;
use serial_test::serial;

/// An unregistered substrate identifier, and the guest substrate, are build errors naming the value.
/// CONTRACT_SHAPE: pure-function (source text in, diagnostic out).
#[test]
#[serial(trybuild)]
#[overdrive_test(host_pure)]
fn an_unregistered_or_guest_substrate_fails_the_build_naming_it() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/unknown_substrate.rs");
    cases.compile_fail("tests/compile_fail/guest_substrate.rs");
}

/// A substrate written as a string literal is a build error that says a substrate is a bare
/// identifier, even when the string names the registered substrate.
/// CONTRACT_SHAPE: pure-function (source text in, diagnostic out).
#[test]
#[serial(trybuild)]
#[overdrive_test(host_pure)]
fn a_substrate_written_as_a_string_literal_fails_the_build_asking_for_an_identifier() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/substrate_string_literal.rs");
}

/// Fewer than nine replicates per arm is a build error naming the minimum.
/// CONTRACT_SHAPE: pure-function. Boundary: 8 refused (9 is the accepted floor).
#[test]
#[serial(trybuild)]
#[overdrive_test(host_pure)]
fn fewer_than_nine_replicates_fails_the_build_naming_the_minimum() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/five_replicates.rs");
    cases.compile_fail("tests/compile_fail/eight_replicates.rs");
}

/// `smoke(none)` is a build error outside `spike-scratch/`; a missing `smoke(...)`
/// argument is a build error naming it; an unknown guest group in `smoke` reuses the
/// placement registry's diagnostic.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[serial(trybuild)]
#[overdrive_test(host_pure)]
fn the_smoke_argument_rules_fail_the_build_naming_the_value_or_the_missing_argument() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/smoke_none_outside_a_spike.rs");
    cases.compile_fail("tests/compile_fail/missing_smoke_argument.rs");
    cases.compile_fail("tests/compile_fail/unknown_guest_group_in_smoke.rs");
}

/// Unknown or repeated keys, duplicate arms, an arm outside the grammar and an
/// unknown metric unit are build errors naming the offending key or value.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[serial(trybuild)]
#[overdrive_test(host_pure)]
fn malformed_declarations_fail_the_build_naming_the_offending_key_or_value() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/unknown_key.rs");
    cases.compile_fail("tests/compile_fail/repeated_key.rs");
    cases.compile_fail("tests/compile_fail/duplicate_arm.rs");
    cases.compile_fail("tests/compile_fail/arm_outside_the_grammar.rs");
    cases.compile_fail("tests/compile_fail/unknown_metric_unit.rs");
}
