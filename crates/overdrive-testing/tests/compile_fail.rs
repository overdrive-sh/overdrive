//! The build-time diagnostics of the placement attribute and `shared_guest!`:
//! each names the offending value or the missing declaration, so an author
//! reads the instruction in the compiler output.
//!
//! The message line of every fixture's expected output is the pinned contract
//! (design DDD-1 and DDD-5, `spike/findings-const-diagnostic.md`). The span and
//! label lines are rustc's rendering of those messages.

#![cfg(target_os = "linux")]
#![allow(clippy::doc_markdown, reason = "Contract Shape declarations keep their literal spelling")]

/// CONTRACT_SHAPE: pure-function (source text in, diagnostic out).
#[test]
#[ignore = "pending DELIVER: guest-placement-on-the-test"]
fn placement_diagnostics_name_the_offending_value_and_the_missing_declaration() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/compile_fail/*.rs");
}
