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

    // udp-service-support step 01-05 — S-04-A driving-adapter companion:
    // `overdrive deploy <udp-spec>` accepted via the direct
    // `commands::deploy::deploy` handler; the persisted
    // `WorkloadIntent::Service` intent carries `Proto::Udp` (C3 guard at
    // the spec → handler → intent boundary). Closes the deploy half of
    // S-04-A that step 01-03 (dataplane wire half) scoped out.
    mod deploy;
    mod deploy_udp_walking_skeleton;
    mod http_client;
    mod post_http_invalid_job_id;
    mod walking_skeleton;

    // Slice 02 step 02-04 — Tier 3 streaming submit:
    //   * S-WS-01 (happy path: real `/bin/sleep` → Succeeded → exit 0)
    // Legacy Service-kind streaming integration tests
    // (`streaming_submit_broken_binary`, `streaming_submit_converged_stopped`,
    // `streaming_submit_happy_path`) were removed in step 01-03e3
    // per single-cut greenfield discipline alongside the deleted
    // `SubmitEvent::Converged*` variants. The Service-kind dispatch
    // wiring is covered end-to-end by
    // `overdrive-control-plane::tests::acceptance::service_submit_dispatch_wiring`
    // (S-SHCP-WIRE-09 through WIRE-15).

    // workload-kind-discriminator slice 05 — Schedule kind submit /
    // alloc-status render surface + IntentStore persistence, with
    // KPI K5 byte-equal deferral URL across surfaces. Per ADR-0047
    // §1, §3 + slice 05 spec.
    mod deploy_schedule;

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

    // service-health-check-probes — Tier 3 integration test that
    // closes the K1 north-star contract:
    //   * Fixture A: coinflip-as-Service (RCA-A regression guard) →
    //     99/100 deterministic seeds emit `Failed { EarlyExit }`.
    //   * Fixture B: quick-bind Service → Stable with settled_in
    //     ∈ [500ms, 2000ms].
    //   * Fixture C: never-binds Service → Failed StartupProbeFailed.
    //   * Fixture D: snapshot/streaming terminal byte-equality.
    //   * Cross-fixture regression: NEVER "(took live)" for Service.
    // RED scaffold — production bodies land in slice 01 + slice 08.
    mod service_honest_stable;

    // service-health-check-probes step 01-03e3-fix — CLI submit-side
    // dispatch routing. Closes the gap 01-03e3 missed: a Service-kind
    // TOML through `deploy_streaming` must route to the new
    // `deploy_streaming_service` (the `ServiceSubmitEvent` consumer),
    // not fall through to the legacy `JobSpecInput` path.
    mod service_submit_streaming_cli_dispatch;

    // backend-instance-replacement slice 01 step 01-04 — the e2e
    // production-loop closer: `overdrive workload restart` driven as a
    // direct CLI handler-call against an in-process run_server through
    // the production POST /v1/jobs/:id/restart route.
    //   * S-BIR-CLI-RESTART-SUCCESS — declared workload (absent /stop) →
    //     RestartOutput with deterministic Restarted label.
    //   * S-BIR-CLI-RESTART-RESUMED — declared workload stopped via the
    //     production stop verb (present /stop) → RestartOutput with
    //     deterministic Resumed label.
    //   * S-BIR-CLI-RESTART-UNKNOWN — undeclared workload → typed 404 →
    //     non-zero exit code.
    mod workload_restart;
}
