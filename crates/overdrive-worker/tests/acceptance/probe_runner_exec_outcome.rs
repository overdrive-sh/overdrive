//! Tier 1 acceptance — `ExecProber` against `SimExecProber`.
//!
//! Slice 03 (US-03) — RED scaffold. NOTE: cgroup-membership assertion
//! lives in Tier 3 integration test
//! `tests/integration/probe_runner_exec_cgroup.rs` per ADR-0059 §2 +
//! ADR-0054 § "Sim adapter does NOT assert cgroup membership".

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

/// S-SHCP-03-01 (US-03 / K1) — exit-0 outcome yields Pass.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_sim_exec_prober_with_exit_zero_when_probe_then_returns_pass() {
    panic!("Not yet implemented -- RED scaffold (S-SHCP-03-01 / SimExecProber exit 0 → Pass)");
}

/// S-SHCP-03-02 (US-03 / K1) — non-zero exit yields
/// `Fail { reason: "exit <N>" }`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_sim_exec_prober_with_exit_one_when_probe_then_returns_fail_exit_named() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-03-02 / SimExecProber exit 1 → Fail with named reason)"
    );
}

/// S-SHCP-03-03 (US-03 / K1) — command-not-found yields
/// `Fail { reason: "exec: command not found" }`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_sim_exec_prober_with_command_not_found_when_probe_then_returns_fail_named() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-03-03 / SimExecProber command not found → Fail with named reason)"
    );
}

/// S-SHCP-03-04 (US-03 / K1) — timeout yields
/// `Fail { reason: "timeout after <N>s" }`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_sim_exec_prober_with_timeout_when_probe_then_returns_fail_timeout_named() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-03-04 / SimExecProber timeout → Fail with timeout reason)"
    );
}
