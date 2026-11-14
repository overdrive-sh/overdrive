//! The build-time diagnostics of the placement attribute and `shared_guest!`:
//! each names the offending value or the missing declaration, so an author
//! reads the instruction in the compiler output.
//!
//! The message line of every fixture's expected output is the pinned contract
//! (design DDD-1 and DDD-5, `spike/findings-const-diagnostic.md`). The span and
//! label lines are rustc's rendering of those messages.
//!
//! One test per fixture group, each pending the later of the slice that builds
//! its diagnostic and the slice that delivers the placement form the test takes:
//! an unknown group on the attribute (built by `guest-placement-on-the-test`, but
//! placed with the host-pure form, so it activates with `host-pure-placement-on-the-test`);
//! an unknown profile (the registry's per-group profiles); the `shared_guest!`
//! forms (the module declaration). Placement on activation: each is a trybuild run
//! that builds and runs nothing of the platform, so each takes
//! `#[overdrive_test(host_pure)]`.

#![cfg(target_os = "linux")]
#![allow(clippy::doc_markdown, reason = "Contract Shape declarations keep their literal spelling")]

use serial_test::serial;

/// An unknown group on the attribute is a build error naming the group.
/// CONTRACT_SHAPE: pure-function (source text in, diagnostic out).
#[test]
#[serial(trybuild)]
#[ignore = "pending DELIVER: host-pure-placement-on-the-test"]
fn an_unknown_guest_group_on_the_attribute_fails_the_build_naming_the_group() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/unknown_guest_group_on_the_attribute.rs");
}

/// An unknown profile of a known group is a build error naming both.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[serial(trybuild)]
#[ignore = "pending DELIVER: guest-artifacts-declared-per-group"]
fn an_unknown_guest_profile_on_the_attribute_fails_the_build_naming_profile_and_group() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/unknown_guest_profile_on_the_attribute.rs");
}

/// `shared_guest!`: an unknown group names the group, the removed `shared`
/// argument names the declaration, and the argument-free form in a module with
/// no declaration names the missing declaration.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[serial(trybuild)]
#[ignore = "pending DELIVER: module-tests-share-one-guest"]
fn the_shared_guest_forms_fail_the_build_naming_the_value_or_the_missing_declaration() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/unknown_guest_group_in_a_shared_guest_declaration.rs");
    cases.compile_fail("tests/compile_fail/a_shared_argument_names_the_declaration.rs");
    cases.compile_fail("tests/compile_fail/module_form_without_a_declaration.rs");
}
