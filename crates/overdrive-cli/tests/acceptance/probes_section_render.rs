//! Tier 1 acceptance — `alloc status` Probes section render per
//! US-06 / K4.
//!
//! Slice 06 — RED scaffolds.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`: tests call render
//! functions directly; no subprocess. Snapshot tests use the
//! existing `insta` library per US-06 Technical Notes.

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

/// S-SHCP-CLI-01 (US-06 / K4) — stable Service with startup,
/// readiness, liveness probes (all Pass) renders a "Probes:" section
/// with one row per probe naming role, probe_idx, mechanic
/// summary, last status, last observed timestamp.
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_stable_service_with_three_probes_pass_when_render_then_probes_section_one_row_each() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-01 / Stable Service with 3 probes renders Probes section)"
    );
}

/// S-SHCP-CLI-02 (US-06 / K4 negative) — Job-kind alloc renders
/// WITHOUT a Probes section anywhere in output. Renderer-side
/// kind guard.
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_job_kind_alloc_when_render_then_no_probes_section() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-02 / Job-kind alloc renders no Probes section)"
    );
}

/// S-SHCP-CLI-03 (US-06 / K4 negative) — Schedule-kind alloc
/// renders WITHOUT a Probes section.
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_schedule_kind_alloc_when_render_then_no_probes_section() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-03 / Schedule-kind alloc renders no Probes section)"
    );
}

/// S-SHCP-CLI-04 (US-06 — failing probe with reason) — Probe Fail
/// row renders `last_fail_reason` (e.g. "HTTP 503") in the same
/// row.
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_probe_fail_row_when_render_then_last_fail_reason_in_output() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-04 / probe Fail row renders last_fail_reason)"
    );
}

/// S-SHCP-CLI-05 (US-06 — pending state) — Service alloc with
/// probes declared but no ProbeResultRow yet written renders
/// `last=pending` (NOT blank).
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_just_started_service_with_no_probe_result_yet_when_render_then_last_equals_pending() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-05 / probe with no row → last=pending)"
    );
}

/// S-SHCP-CLI-06 (US-01 — inferred default render) — Stable Service
/// submitted WITHOUT explicit probes renders the inferred TCP
/// startup probe with `(inferred)` suffix.
#[test]
#[should_panic(expected = "RED scaffold")]
fn given_stable_service_with_inferred_default_probe_when_render_then_marked_inferred() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-CLI-06 / inferred default probe renders \"(inferred)\")"
    );
}
