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
    mod streaming_submit_broken_binary;
    // fix-converged-stopped-cli-arm — regression: ConvergedStopped
    // must terminate the streaming consumer with exit code 0; current
    // code falls through to the `_ =>` catch-all and returns
    // Err(BodyDecode).
    mod streaming_submit_converged_stopped;
    mod streaming_submit_happy_path;

    // workload-kind-discriminator slice 05 — Schedule kind submit /
    // alloc-status render surface + IntentStore persistence, with
    // KPI K5 byte-equal deferral URL across surfaces. Per ADR-0047
    // §1, §3 + slice 05 spec.
    mod job_submit_schedule;

    // Slice 03 step 03-02 — S-CLI-03 Tier 3 jq-pipeline-equivalent:
    // a pipe-redirected stdout (non-TTY) without --detach MUST
    // auto-select the JSON-ack lane and emit a single parseable JSON
    // object whose `spec_digest` is 64 lowercase-hex chars. CLAUDE.md
    // forbids `Command::spawn`, so this is the in-process equivalent
    // of the shell pipeline; see file rustdoc for the full mapping.
    mod submit_jq_pipeline;

    // workload-kind-discriminator slice 02 — Job-kind streaming
    // submit acceptance tests + S-02-09 K1 honesty (Lima-gated).
    // The load-bearing assertion is S-02-05 anti-scenario: no
    // Job-kind submit produces "is running with" or "(took live)".
    mod coinflip_honesty_100_trials;
    mod job_kind_streaming;

    // workload-kind-discriminator slice 03 — kind-aware alloc-status
    // Job render. KPI K3 byte-equality between rendered Exit column
    // and persisted exit_code (S-03-08 proptest 1024 cases). Per
    // step 02-02 acceptance criteria + ADR-0047 §1 / §4.
    mod alloc_status;

    // cgroup-fs-port step 01-06 — E2 walking-skeleton: composition root
    // probes RealCgroupFs BEFORE worker subsystem startup; on probe
    // failure emits `health.startup.refused` event and returns
    // `CliError::ProbeRefused`. Per ADR-0054 § Composition root wiring.
    mod serve_probe_refusal;
}
