//! Tier 3 integration — Service-submit honesty regression guard.
//!
//! Slice 01 (US-01 walking skeleton) + Slice 08 (US-08 EarlyExit
//! hardening) — RED scaffold.
//!
//! KPI K1 north-star: ≥99 of 100 deterministic seeds of submitting
//! a coinflip-shaped Service emit `Failed { EarlyExit |
//! StartupProbeFailed }`, never `Stable`, never bare `(took live)`.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`: this test starts a real
//! in-process control-plane server on an ephemeral port, calls
//! `commands::job::submit` directly (NOT subprocess), and parses
//! the typed `SubmitOutput` (NOT stdout). Per
//! `.claude/rules/testing.md` § "Running tests — Lima VM":
//! invocation goes through `cargo xtask lima run -- cargo nextest
//! run -p overdrive-cli --features integration-tests -E
//! 'test(service_honest_stable)'`.

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

/// S-SHCP-INT-CLI-01 (US-01 WS Fixture A — RCA-A regression guard /
/// K1) — Service with exec that exits 1 within 30ms (the coinflip-
/// reshaped-as-Service fixture). Across 100 deterministic seeds:
/// ≥99 emit `Failed { reason: EarlyExit { exit_code: 1 } }` AND
/// zero emit `Stable`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_coinflip_as_service_fixture_when_submit_100_seeds_then_99_emit_failed_early_exit() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-CLI-01 / K1 north star: coinflip-as-Service 99/100 emit Failed ( EarlyExit ))"
    );
}

/// S-SHCP-INT-CLI-02 (US-01 WS Fixture B — happy path / K1) —
/// Service whose listener binds within 600ms. Assert `Stable`
/// emitted with `settled_in` parseable as a Duration in
/// [500ms, 2000ms] window.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_quick_bind_service_fixture_when_submit_then_stable_settled_in_within_window() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-CLI-02 / quick-bind Service → Stable with settled_in ∈ [500ms, 2000ms])"
    );
}

/// S-SHCP-INT-CLI-03 (US-01 WS Fixture C — startup probe timeout
/// sad path / K1) — Service that never binds the listener. Assert
/// `Failed { reason: StartupProbeFailed { last_fail: "connection
/// refused", ... } }` after `startup_deadline` elapses.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_never_binds_service_fixture_when_submit_then_failed_startup_probe_after_deadline() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-CLI-03 / never-binds Service → Failed ( StartupProbeFailed ( last_fail: \"connection refused\" ) ) after deadline)"
    );
}

/// S-SHCP-INT-CLI-04 (US-01 WS Fixture D — byte-equality across
/// snapshot + streaming) — for the same Stable deciding tick,
/// `AllocStatusRow.terminal` and the captured `LifecycleEvent.
/// terminal` are byte-equal under rkyv archive serialisation.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_stable_decision_when_snapshot_and_streaming_terminal_then_byte_equal() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-CLI-04 / Stable terminal byte-equality across snapshot + streaming)"
    );
}

/// S-SHCP-INT-CLI-05 (US-08 / K1 regression guard) — the literal
/// `"(took live)"` string is NEVER present in any Service-kind
/// submit output across any of the three fixture seeds above.
/// Cross-fixture pinned regression for ADR-0037 + RCA-A solution D.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_any_service_kind_submit_when_streaming_output_then_never_contains_took_live() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-CLI-05 / cross-fixture regression: NEVER \"(took live)\" for Service kind)"
    );
}
