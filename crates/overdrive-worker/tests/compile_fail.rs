//! Compile-fail fixture harness for `overdrive-worker`.
//!
//! Per ADR-0054 § migration steps 01-01 / 01-05 and
//! `.claude/rules/testing.md` § "Compile-fail testing (trybuild)":
//! pins **compile-time invariants** about the worker's public type
//! surface — specifically, that `ExecDriver::new`'s post-01-05 arity
//! is structurally required (a constructor call missing the mandatory
//! `fs: Arc<dyn CgroupFs>` parameter must fail to compile, not surface
//! only at runtime) AND that `ExecDriver` does NOT implement `Default`
//! (per `.claude/rules/development.md` § "Port-trait dependencies":
//! every dependency at the call site must be explicit — defaulting
//! to a production binding silently inherits real I/O into tests that
//! forgot to override).
//!
//! Both fixtures went GREEN at step 01-05 alongside the
//! `ExecDriver::new` arity change.

#[test]
fn compile_fail_fixtures() {
    let t = trybuild::TestCases::new();
    // A1 — `ExecDriver::new(cgroup_root, clock)` (2-arg) MUST fail to
    // compile against the post-01-05 3-arg signature. The `.stderr`
    // companion pins the diagnostic shape so future arity drift
    // (e.g. an accidentally-defaulted `fs` parameter) is caught at
    // PR time.
    t.compile_fail("tests/compile_fail/exec_driver_missing_fs.rs");
    // A2 — `ExecDriver::default()` MUST fail to compile. ExecDriver
    // carries no `Default` impl per `.claude/rules/development.md`
    // § "Port-trait dependencies"; tests that forget to inject `fs`
    // / `clock` fail at compile time rather than silently inheriting
    // wall-clock or real-cgroupfs behaviour.
    t.compile_fail("tests/compile_fail/exec_driver_no_default.rs");
}
