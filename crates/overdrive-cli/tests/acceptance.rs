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
    // Slice 01 step 01-03 — journey TUI mockup renderer for the
    // extended `AllocStatusResponse`. S-AS-04 / S-AS-05 / S-AS-06.
    mod alloc_status_render;

    mod insecure_rejected;
    mod render_alloc_status;
    mod render_cluster_and_node;
    mod render_job_stop;
    mod render_job_submit;

    // Slice 02 step 02-04 — S-CLI-04 (Failed-block render) and
    // S-CLI-05 (exit-code 2 across HTTP error variants).
    mod streaming_submit_cli_render;
    // fix-converged-stopped-cli-arm — render-fn tests for
    // `format_stopped_summary` (one per `StoppedBy` variant). Pairs
    // with the integration-test regression in
    // `tests/integration/streaming_submit_converged_stopped.rs`.
    mod streaming_submit_cli_render_stopped;
    mod streaming_submit_http_error_exit_2;

    // Slice 03 step 03-01 — S-CLI-01 `--detach` flag argv surface.
    mod submit_detach_flag;

    // Slice 03 step 03-02 — S-CLI-02 + S-CLI-06 IsTerminal auto-detach
    // truth table (the dispatch decision: JSON-ack lane vs NDJSON
    // streaming lane). The wire-level Accept-header pinning is covered
    // by the existing JSON-ack and streaming integration suites.
    mod submit_pipe_autodetect;
}
