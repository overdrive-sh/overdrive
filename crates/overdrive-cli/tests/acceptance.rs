//! Acceptance-test entrypoint for `overdrive-cli`.
//!
//! Per ADR-0005 and `.claude/rules/testing.md`, acceptance tests live
//! under `tests/acceptance/*.rs` and are wired into a single Cargo
//! integration-test binary via this entrypoint. The inline
//! `mod acceptance { ... }` block shifts the module lookup base into
//! the `acceptance/` subdirectory — a Cargo integration-test crate
//! root resolves `mod foo;` against `tests/foo.rs`, not
//! `tests/acceptance/foo.rs`, so the wrapping inline module is
//! load-bearing.
//!
//! Acceptance tests in this crate stay in the default unit lane —
//! they exercise only in-process clap argv parsing and pure function
//! calls (no subprocess, no TCP, per `crates/overdrive-cli/CLAUDE.md`).

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
#![allow(clippy::unwrap_used)]

mod acceptance {
    mod insecure_rejected;
    mod render_alloc_status;
    mod render_cluster_and_node;
    mod render_workload_stop;
    mod render_workload_submit;

    // Legacy `streaming_submit_cli_render` (Failed-block render
    // against deleted `TerminalReason`) and
    // `streaming_submit_http_error_exit_2` were deleted in step
    // 01-03e3 alongside the legacy `format_failed_block` removal
    // per single-cut greenfield discipline. `format_stopped_summary`
    // render coverage stays in `streaming_submit_cli_render_stopped`
    // (the function itself is kind-aware and survives the migration).
    mod streaming_submit_cli_render_stopped;

    // Slice 03 step 03-01 — S-CLI-01 `--detach` flag argv surface.
    mod submit_detach_flag;

    // Slice 03 step 03-02 — S-CLI-02 + S-CLI-06 IsTerminal auto-detach
    // truth table (the dispatch decision: JSON-ack lane vs NDJSON
    // streaming lane). The wire-level Accept-header pinning is covered
    // by the existing JSON-ack and streaming integration suites.
    mod submit_pipe_autodetect;

    // workload-kind-discriminator slice 02 — Job-kind render fns
    // (`format_job_succeeded_summary`, `format_job_failed_summary`,
    // `format_job_attempt_failed`, `format_job_submit_echo`) per
    // ADR-0047 §3 [D2] / [D7]. The structural fix that closes the
    // bug under audit lands here.
    mod job_kind_render;

    // Pure render helpers — format_human_duration, derive_job_verdict,
    // and the live `alloc_status` spec-digest branches.
    mod render_pure_fns;

    // service-health-check-probes — Tier 1 acceptance for the CLI
    // render surface per US-06 / US-07 / US-08. RED scaffolds.
    //   * Slice 06 (US-06 / K4): Probes section in alloc-status render
    //   * Slice 07 (US-07 / K5): ProbesNotAllowedOnKind CLI surface
    //   * Slice 08 (US-08 / K1): EarlyExit multi-line render +
    //     RCA-A "(took live)" regression guard
    mod probes_kind_rejection_cli;
    mod probes_section_render;
    // Step 03-02 / Slice 08 — EarlyExit multi-line render
    // (S-SHCP-CLI-07/08) + the cross-cutting RCA-A
    // `ServiceKindRenderNeverContainsTookLive` regression guard
    // (S-SHCP-CLI-09..11). Re-created against the current
    // `format_service_failed_block` surface (the 01-03e3 deletion
    // removed the legacy `format_failed_block` variant; this file
    // targets the typed `ServiceFailureReason` renderer).
    mod service_early_exit_render;
}
