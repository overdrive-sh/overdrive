//! Trybuild harness for compile-PASS invariants on `overdrive-control-plane`.
//!
//! Every file under `tests/compile_pass/*.rs` is compiled and asserted
//! to compile without error. Pairs with `tests/compile_fail.rs` which
//! asserts the inverse (specific shapes do NOT compile).
//!
//! # What this asserts
//!
//! * `ViewStore` is dyn-compatible — `Arc<dyn ViewStore>` is a valid
//!   shape (ADR-0035 §7). The trait sits at the runtime's
//!   constructor-required port-trait boundary; if a generic-by-method or
//!   `-> impl Trait` shape ever lands on the trait surface, it silently
//!   breaks dyn compatibility and the runtime constructor fails to
//!   instantiate. The fixture pins this property mechanically.

#[test]
fn compile_pass_cases() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile_pass/*.rs");
}
