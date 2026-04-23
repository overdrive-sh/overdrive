//! Trybuild harness for type-level separation invariants.
//!
//! Every file under `tests/compile_fail/*.rs` is compiled and its stderr
//! compared against the sibling `*.stderr` fixture. Regenerate with
//! `TRYBUILD=overwrite cargo test -p overdrive-core --test compile_fail`
//! after an intentional compiler-diagnostic change; the pinned trybuild
//! version in the workspace manifest exists to make that regeneration a
//! deliberate act.
//!
//! # What this asserts
//!
//! * `IntentStore` and `ObservationStore` are not type-substitutable.
//!   Passing an `&dyn IntentStore` to a function parameter typed
//!   `&dyn ObservationStore` (or vice versa) is a compile error, and
//!   the diagnostic names both trait paths — see
//!   `compile_fail/intent_vs_observation.rs` and the §4.4 scenario in
//!   `docs/feature/phase-1-foundation/distill/test-scenarios.md`.
//!
//! ADR-0001 row 24 and ADR-0005 authorise `trybuild` as a dev-dependency
//! for this purpose only.

#[test]
fn compile_fail_cases() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
