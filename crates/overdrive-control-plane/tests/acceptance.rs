//! Acceptance-test entrypoint for `overdrive-control-plane`.
//!
//! Per ADR-0005 and `.claude/rules/testing.md`, acceptance tests live
//! under `tests/acceptance/*.rs` and are wired into a single
//! Cargo integration-test binary via this entrypoint. The inline
//! `mod acceptance { ... }` block shifts the module lookup base into
//! the `acceptance/` subdirectory — an integration-test crate root
//! resolves `mod foo;` against `tests/foo.rs`, not
//! `tests/acceptance/foo.rs`, so the wrapping inline module is
//! load-bearing.
//!
//! Acceptance tests in this crate stay in the default unit lane —
//! they exercise only in-process serde round-trips and utoipa schema
//! emission, no real infrastructure.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
#![allow(clippy::unwrap_used)]

mod acceptance {
    mod api_type_shapes;
    mod cluster_status_lists_both_reconcilers;
    mod default_lane_no_cgroup_dependency;
    mod error_mapping_exhaustive;
    mod eval_broker_collapse;
    mod job_lifecycle_backoff;
    mod job_stop_idempotent;
    mod job_stop_intent_key;
    mod job_stop_unknown;
    mod pending_no_capacity_renders_reason;
    mod row_body_conversions;
    mod runtime_convergence_loop;
    mod runtime_registers_noop_heartbeat;
    mod submit_job_idempotency;
    mod trust_triple_getters;

    // wire-exec-spec-end-to-end — operator-facing job spec carries
    // `[exec]` block end-to-end. Per ADR-0031.
    mod action_shim_restart_uses_spec_from_action;
    mod openapi_exec_block;
    mod submit_job_handler_rejects_empty_exec_command_with_400;
}
