//! Trybuild harness for type-level compile-PASS invariants.
//!
//! Files under `tests/compile_pass/*.rs` are compiled and required
//! to succeed. Counterpart to `tests/compile_fail/*.rs` which asserts
//! diagnostic shapes.
//!
//! # What this asserts
//!
//! * `TcpProber` / `HttpProber` / `ExecProber` are object-safe
//!   (dyn-compatible) — required by the `ProbeRunner` per ADR-0054
//!   §3. See `compile_pass/prober_traits_are_dyn_compatible.rs`.

#[test]
fn compile_pass_cases() {
    let t = trybuild::TestCases::new();
    t.pass("tests/compile_pass/*.rs");
}
