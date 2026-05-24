//! Tier 3 integration — `CgroupExecProber` against a real cgroup
//! scope inside Lima.
//!
//! Slice 03 (US-03) — RED scaffold.
//!
//! Per ADR-0059 §1 + ADR-0054 §3 ExecProber postcondition: spawned
//! probe process MUST be a member of the workload's cgroup scope per
//! its `/proc/<pid>/cgroup` readout — the load-bearing invariant
//! this test pins.
//!
//! Per `.claude/rules/testing.md` § "Running tests — Lima VM" the
//! cgroup-write path needs root via `cargo xtask lima run --` (the
//! wrapper defaults to running as root). Lima sudo is the canonical
//! shape per ADR-0034.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    reason = "DISTILL RED scaffold; per `.claude/rules/testing.md` § 'RED scaffolds' lints land when DELIVER replaces todo!() bodies + rewrites docs"
)]

/// S-SHCP-INT-03-01 (US-03 / K1) — exec probe spawns `/bin/true`
/// inside a real test cgroup scope; assert exit 0 → Pass.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_real_cgroup_scope_when_cgroup_exec_prober_runs_bin_true_then_returns_pass() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-03-01 / real cgroup scope + /bin/true → CgroupExecProber Pass)"
    );
}

/// S-SHCP-INT-03-02 (US-03 AC — cgroup membership) — spawned probe
/// process is a member of `alloc-<id>.scope`, NOT the worker's
/// scope. The structural defense against ADR-0059 §2 sim/prod
/// divergence: the test asserts the production-only invariant.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_real_cgroup_scope_when_cgroup_exec_prober_runs_then_pid_membership_matches_scope() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-03-02 / probe PID /proc/<pid>/cgroup names alloc scope, NOT worker scope)"
    );
}

/// S-SHCP-INT-03-03 (US-03 / K1) — `/bin/sleep 10` with
/// `timeout_seconds = 2` is SIGKILLed at 2s boundary; ProbeOutcome
/// Fail with `timeout after 2s`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_real_cgroup_scope_when_cgroup_exec_prober_times_out_then_sigkill_and_fail_named() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-03-03 / real cgroup + sleep timeout → SIGKILL + Fail \"timeout after 2s\")"
    );
}
