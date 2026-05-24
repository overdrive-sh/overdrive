//! Compile-fail fixture harness for `overdrive-worker`.
//!
//! Per ADR-0054 § migration step 01-01 (this step) and `.claude/rules/testing.md`
//! § "Compile-fail testing (trybuild)": pins **compile-time invariants**
//! about the worker's public type surface — specifically, that the
//! post-step-01-05 `ExecDriver::new` arity is structurally required
//! (a constructor call missing the mandatory `fs: Arc<dyn CgroupFs>`
//! parameter must fail to compile, not surface only at runtime).
//!
//! # RED scaffold status (step 01-01)
//!
//! The fixture stub at `tests/compile_fail/exec_driver_missing_fs.rs`
//! is wired in place but **NOT yet active** — `ExecDriver::new` does
//! not gain the `fs` parameter until step 01-05. Activating the
//! fixture today would either (a) compile (the current 2-arity `new`
//! accepts the 2-argument call) and the assertion would be vacuous,
//! or (b) fail to compile for an unrelated reason (e.g. missing
//! `clock` argument), which would mask the real signal once 01-05
//! lands.
//!
//! The `t.compile_fail(...)` invocation is commented out below with
//! a marker; uncomment it as part of step 01-05's GREEN. See
//! `docs/feature/cgroup-fs-port/distill/test-scenarios.md` scenario
//! A1 for the deferred-activation contract.

#[test]
fn compile_fail_fixtures() {
    let t = trybuild::TestCases::new();
    // RED scaffold: re-enable at step 01-05 once `ExecDriver::new`
    // gains `fs: Arc<dyn CgroupFs>` as a mandatory parameter per
    // ADR-0054 § D5. Until then the fixture would either be vacuous
    // or fail-for-wrong-reason; both shapes mask the real assertion.
    // t.compile_fail("tests/compile_fail/exec_driver_missing_fs.rs");
    let _ = t; // silence unused-variable while the fixture is deferred
}
