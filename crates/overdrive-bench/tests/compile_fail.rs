//! The build-time rules of the `overdrive_bench` attribute (ADR-0297, DDD-2/DDD-4):
//! each names the offending value or the missing declaration, so an author reads the
//! instruction in the compiler output.
//!
//! The `error:` message line of every fixture's expected output is the contract DDD-2
//! pins (the table of rejections); the `-->`, source and label lines are rustc's rendering
//! of the macro's spans, written here as the expected shape and regenerated once with
//! `TRYBUILD=overwrite` (review that every message line is unchanged). Substrate and `smoke(none)` rejections are constant evaluations, so rustc
//! prints them as `error[E0080]: evaluation panicked: <message>`.
//! Driving port: the attribute. Evidence lane: trybuild (compile-fail).
//! Placement: each is a trybuild run that builds nothing of the platform, so `host_pure`.
//! Traces: US-BENCH-01 AC (e), US-BENCH-02 AC (d), US-BENCH-04 AC (c), DDD-2, DDD-4.

#![cfg(target_os = "linux")]
#![allow(clippy::doc_markdown, reason = "Contract Shape declarations keep their literal spelling")]

use overdrive_testing::overdrive_test;
use serial_test::serial;

/// An unregistered substrate identifier is a build error naming the value and the registered
/// substrates in registration order (`native, guest`).
/// CONTRACT_SHAPE: pure-function (source text in, diagnostic out).
#[overdrive_test(host_pure)]
#[test]
#[serial(trybuild)]
#[should_panic(expected = "1 of 1 tests failed")]
fn an_unregistered_substrate_fails_the_build_naming_it_and_the_registered_ones() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/unknown_substrate.rs");
}

/// A benchmark declaring the `guest` substrate with the `guest_monotonic` clock source and a
/// series metric builds: the registry holds `guest` (ADR-0325) and the guest clock vocabulary holds
/// `guest_monotonic` (DDD-1). The "guest substrates are not available" refusal is gone.
/// CONTRACT_SHAPE: pure-function (source text in, diagnostic out).
#[overdrive_test(host_pure)]
#[test]
#[serial(trybuild)]
#[should_panic(expected = "1 of 1 tests failed")]
fn a_guest_benchmark_declaration_builds() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/compile_fail/guest_substrate_accepted.rs");
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

// ---- The `clock(service = ..., guest = ...)` key (DDD-2 "Clock sources") ----

/// A service clock source outside its closed set is a build error naming the set.
/// CONTRACT_SHAPE: pure-function (source text in, diagnostic out).
#[test]
#[serial(trybuild)]
#[overdrive_test(host_pure)]
fn an_unknown_service_clock_source_fails_the_build_naming_the_set() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/unknown_service_clock_source.rs");
}

/// A guest clock source outside its closed set is a build error naming the set, which holds
/// `init_elapsed_ms` and `guest_monotonic` in that order (DDD-1).
/// CONTRACT_SHAPE: pure-function (source text in, diagnostic out).
#[overdrive_test(host_pure)]
#[test]
#[serial(trybuild)]
#[should_panic(expected = "1 of 1 tests failed")]
fn an_unknown_guest_clock_source_fails_the_build_naming_the_set() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/unknown_guest_clock_source.rs");
}

/// An unknown or repeated sub-key of `clock(...)`, a source written as a string literal and an
/// empty `clock()` are build errors naming the offending key or value.
/// CONTRACT_SHAPE: pure-function (source text in, diagnostic out).
#[test]
#[serial(trybuild)]
#[overdrive_test(host_pure)]
fn a_malformed_clock_declaration_fails_the_build_naming_the_offending_key_or_value() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/unknown_clock_sub_key.rs");
    cases.compile_fail("tests/compile_fail/repeated_clock_sub_key.rs");
    cases.compile_fail("tests/compile_fail/clock_source_string_literal.rs");
    cases.compile_fail("tests/compile_fail/empty_clock.rs");
}
