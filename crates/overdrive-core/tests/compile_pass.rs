//! Trybuild compile-pass harness for type-level structural assertions
//! that must continue to compile.
//!
//! Every file under `tests/compile_pass/*.rs` is compiled as a
//! standalone crate; the test passes when the file compiles cleanly
//! and fails when any compile error fires.
//!
//! # What this asserts
//!
//! * `Box<dyn Reconciler<State = ..., View = ...>>` is constructible
//!   for a concrete `(State, View)` pair. The associated-type-bearing
//!   trait is dyn-compatible after the ADR-0035 collapse removed the
//!   `async fn` (which was the dyn-incompatibility driver). See
//!   `compile_pass/reconciler_trait_is_dyn_compatible.rs`.
//!
//! Trybuild is pinned in `Cargo.toml`; regenerate `*.stderr` (none
//! today; compile-pass cases have no stderr) only by intentional
//! schema bump.

#[test]
fn compile_pass_cases() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile_pass/*.rs");
}
