//! Trybuild harness for type-level compile-PASS invariants.
//!
//! Files under `tests/compile_pass/*.rs` are compiled and required
//! to succeed. Counterpart to `tests/compile_fail/*.rs` which asserts
//! diagnostic shapes.
//!
//! # What this asserts
//!
//! * The surviving prober traits are object-safe (dyn-compatible).

#[test]
fn compile_pass_cases() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile_pass/*.rs");
}
