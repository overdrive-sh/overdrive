//! Tier 1 acceptance — CLI render of
//! `ServiceSubmitEvent::Failed { reason: EarlyExit { ... }, stderr_tail }`
//! per US-08 / K1.
//!
//! Slice 08 — RED scaffolds.
//!
//! Per US-08: the multi-line failure render closes the RCA-A
//! coinflip regression at the operator surface. NEVER emits
//! literal `"(took live)"` for Service-kind allocs.

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

/// S-SHCP-CLI-07 (US-08 / K1) — `Failed { EarlyExit { exit_code:
/// 1 }, stderr_tail: "ERROR ..." }` renders as multi-line block
/// naming `exit_code`, `elapsed`, `stderr_tail`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_failed_early_exit_when_render_then_multi_line_block_with_exit_code_elapsed_stderr() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-07 / Failed ( EarlyExit ) renders multi-line with exit_code + elapsed + stderr_tail)"
    );
}

/// S-SHCP-CLI-08 (US-08 — distinguish exit 0 case) — `Failed
/// { EarlyExit { exit_code: 0 } }` renders the Service-kind
/// guidance ("Service kind expects long-lived; use [job] for run-
/// to-completion.").
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_failed_early_exit_zero_when_render_then_includes_service_kind_guidance() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-08 / Failed ( EarlyExit ( exit_code: 0 ) ) renders Service-kind guidance)"
    );
}

/// S-SHCP-CLI-09 (US-01 + US-08 — load-bearing RCA-A regression
/// guard) — NEVER emit the literal `"(took live)"` string for a
/// Service-kind alloc, regardless of the alloc state. Pins the
/// RCA-A solution D contract per C9.
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_any_service_kind_alloc_when_render_then_never_emits_took_live_literal() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-09 / RCA-A regression guard: NEVER \"(took live)\" for Service kind)"
    );
}

/// S-SHCP-CLI-10 (US-01 — Stable render) — `Stable
/// { settled_in: 1.2s, witness: ProbeWitness { ... } }` renders
/// real Duration via `format_human_duration` (e.g. "1.2s"), NEVER
/// the literal `"live"` (per C9).
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_stable_when_render_then_settled_in_is_real_duration_never_live_literal() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-10 / Stable settled_in renders real Duration, never \"live\")"
    );
}

/// S-SHCP-CLI-11 (US-01 — witness render) — `Stable` witness line
/// names `probe_idx + role + mechanic_summary`
/// (e.g. `witness: startup probe #0 (tcp 0.0.0.0:8080)`).
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_stable_with_witness_when_render_then_witness_line_names_probe_idx_role_mechanic() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-11 / Stable witness line names probe_idx + role + mechanic)"
    );
}
