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
}
