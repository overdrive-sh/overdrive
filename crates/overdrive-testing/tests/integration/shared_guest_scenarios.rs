//! US-PLACE-09 and the per-test guest of US-PLACE-01 through the driving port
//! a person uses: a fixture test of this binary run the way nextest runs it,
//! on a Metal run, with the guest kit's timing lines as the evidence of how
//! many guests booted.
//!
//! Each scenario names the fixture it drives (`shared_guest_fixtures`), the
//! fault it injects and the oracle it reads. Real guests, example-only.
//!
//! Placement on activation: every scenario here takes
//! `#[overdrive_test(native_launcher)]` (each reads the host half's timing
//! lines from outside any guest); the form is given when the scenario
//! activates and no placement-table entry is ever added.

use std::collections::BTreeSet;

use overdrive_testing::overdrive_test;

use super::scenario_support::{
    FAIL_ENV, OwnerSampler, owners_in_this_run, run_test, shared_generations, test_path, timings,
};

const SHARED: &str = "shared_guest_fixtures::shared_module";
const ISOLATED: &str = "shared_guest_fixtures::isolated_guest";

/// US-PLACE-09 s1: one guest for the whole module means one generation and one
/// owner process: two owners could each boot a "generation 1", so the owner
/// processes seen while the three tests ran are counted too. The three tests run
/// back to back, far inside the owner's 30 s idle period (design DDD-17).
/// CONTRACT_SHAPE: bounded-change (guests booted per run).
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn a_declared_module_boots_one_guest_and_every_test_reports_its_own_result() {
    let before = owners_in_this_run();
    let sampler = OwnerSampler::start();
    let mut generations = BTreeSet::new();
    for name in [
        "first_test_of_the_shared_module",
        "second_test_of_the_shared_module",
        "third_test_of_the_shared_module",
    ] {
        let run = run_test(&test_path(SHARED, name), &[], &[]);
        assert!(run.success, "{name} reports its own result and passes:\n{}", run.output);
        let attached = timings(&run.output).iter().filter(|t| t.shared()).count();
        assert_eq!(attached, 1, "{name} attaches once:\n{}", run.output);
        generations.extend(shared_generations(&run.output));
    }
    let owners: Vec<_> = sampler.stop().difference(&before).copied().collect();
    assert_eq!(owners.len(), 1, "one owner served the whole module: {owners:?}");
    assert_eq!(
        generations.len(),
        1,
        "one guest booted for the module, saw generations {generations:?}"
    );
}

/// US-PLACE-09 s2: a failing test is reported alone, its guest is discarded and
/// the tests after it still pass on a replacement. CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn one_failing_test_in_a_shared_module_does_not_hide_the_others() {
    let before = run_test(&test_path(SHARED, "first_test_of_the_shared_module"), &[], &[]);
    assert!(before.success, "{}", before.output);
    let failing = run_test(
        &test_path(SHARED, "test_of_the_shared_module_that_fails_when_told_to"),
        &[(FAIL_ENV, "1")],
        &[],
    );
    assert!(!failing.success, "the failing test is reported FAIL:\n{}", failing.output);
    assert!(
        failing.output.contains("the scenario asked this test to fail"),
        "the guest's failure output is visible:\n{}",
        failing.output
    );
    let after = run_test(&test_path(SHARED, "second_test_of_the_shared_module"), &[], &[]);
    assert!(after.success, "the next test reports its own result:\n{}", after.output);
    let (g_before, g_after) =
        (shared_generations(&before.output), shared_generations(&after.output));
    assert!(!g_before.is_empty(), "the first test attached a guest:\n{}", before.output);
    assert!(!g_after.is_empty(), "the next test attached a guest:\n{}", after.output);
    assert!(
        g_after.iter().min() > g_before.iter().max(),
        "the failed test's guest was discarded: {g_before:?} then {g_after:?}"
    );
}

/// US-PLACE-09 s3. CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn filtering_to_one_test_of_a_shared_module_runs_only_that_test() {
    let run = run_test(&test_path(SHARED, "second_test_of_the_shared_module"), &[], &[]);
    assert!(run.success, "{}", run.output);
    assert_eq!(
        timings(&run.output).len(),
        1,
        "only the filtered test attached a guest:\n{}",
        run.output
    );
}

/// US-PLACE-09 s4 and the approved override rule: a guest-placed test outside a
/// declared module, and an explicit form inside one, keep one guest each. The
/// explicit-form fixture (`shared_module::explicit_guest_test_in_the_shared_module`)
/// needs only the per-test guest form, so it is pending with this scenario.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[overdrive_test(native_launcher)]
fn guest_placed_tests_outside_a_shared_module_boot_one_guest_each() {
    for (module, name) in [
        (ISOLATED, "first_isolated_guest_test"),
        (ISOLATED, "second_isolated_guest_test"),
        (SHARED, "explicit_guest_test_in_the_shared_module"),
    ] {
        let run = run_test(&test_path(module, name), &[], &[]);
        assert!(run.success, "{name}:\n{}", run.output);
        let lines = timings(&run.output);
        assert_eq!(lines.len(), 1, "{name} boots exactly one guest:\n{}", run.output);
        assert!(!lines[0].shared(), "{name} keeps an isolated guest:\n{}", run.output);
    }
}

/// US-PLACE-01 AC f: the boot time of a per-test guest is retained in the run
/// output. CONTRACT_SHAPE: bounded-change.
#[test]
#[overdrive_test(native_launcher)]
fn the_boot_time_of_a_guest_per_test_is_recorded_in_the_run_output() {
    let run = run_test(&test_path(ISOLATED, "first_isolated_guest_test"), &[], &[]);
    assert!(run.success, "{}", run.output);
    let lines = timings(&run.output);
    assert_eq!(lines.len(), 1, "{}", run.output);
    let line = &lines[0];
    assert_eq!(line.field("group"), Some("cgroup-default"));
    assert!(
        line.field("test").is_some_and(|t| t.ends_with("first_isolated_guest_test")),
        "{line:?}"
    );
    for field in ["staging_ms", "boot_ms", "total_ms"] {
        assert!(
            line.field(field).is_some_and(|v| v.parse::<u64>().is_ok()),
            "{field} is a number of milliseconds: {line:?}"
        );
    }
}
