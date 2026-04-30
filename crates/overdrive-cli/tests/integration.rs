//! Integration-test entrypoint for `overdrive-cli`.
//!
//! Per `.claude/rules/testing.md` and `crates/overdrive-cli/CLAUDE.md`,
//! integration tests that spin up a real in-process control-plane
//! server (real TLS, real reqwest) live under `tests/integration/*.rs`
//! and are gated behind the `integration-tests` feature. The inline
//! `mod integration { ... }` block shifts the module lookup base into
//! the `integration/` subdirectory — a Cargo integration-test crate
//! root resolves `mod foo;` against `tests/foo.rs`, not
//! `tests/integration/foo.rs`, so the wrapping inline module is
//! load-bearing.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
#![allow(clippy::unwrap_used)]

mod integration {
    mod cluster_and_node_commands;
    mod cluster_init_removed;
    mod endpoint_from_config;
    mod exec_spec_walking_skeleton;
    mod http_client;
    mod job_submit;
    mod post_http_invalid_job_id;
    mod walking_skeleton;

    // Slice 02 step 02-04 — Tier 3 streaming submit:
    //   * S-WS-01 (happy path: real `/bin/sleep` → ConvergedRunning → exit 0)
    //   * S-WS-02 (REGRESSION TARGET KPI-02: real ENOENT → ConvergedFailed
    //     with byte-equal cause-class payload across streaming + snapshot)
    // Both #[cfg(target_os = "linux")] — production `ExecDriver`
    // requires real `tokio::process::Command::spawn`. macOS dev runs
    // via `cargo xtask lima run --` per `crates/overdrive-cli/CLAUDE.md`.
    mod streaming_submit_broken_binary;
    mod streaming_submit_happy_path;

    // Slice 03 step 03-02 — S-CLI-03 Tier 3 jq-pipeline-equivalent:
    // a pipe-redirected stdout (non-TTY) without --detach MUST
    // auto-select the JSON-ack lane and emit a single parseable JSON
    // object whose `spec_digest` is 64 lowercase-hex chars. CLAUDE.md
    // forbids `Command::spawn`, so this is the in-process equivalent
    // of the shell pipeline; see file rustdoc for the full mapping.
    mod submit_jq_pipeline;
}
