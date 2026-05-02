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
    // S-CP-09 — AllocStatusRow rkyv round-trip with the new
    // `reason: Option<TransitionReason>` and `detail: Option<String>`
    // fields per ADR-0032 §3 (Amendment 2026-04-30) and §4.
    mod alloc_status_row_archive_roundtrip;

    // S-AS-01 / S-AS-07 / S-AS-08 / S-AS-09 (Slice 01 step 01-03) —
    // `AllocStatusResponse` extension, handler hydration via observation
    // rows + `JobLifecycleView`, 404 on missing job. KPI-03 satisfaction.
    mod alloc_status_snapshot;

    mod api_type_shapes;
    mod cluster_status_lists_both_reconcilers;

    // S-AS-02 (Slice 01 step 02) — `TransitionRecord.reason` is the
    // `TransitionReason` enum from `overdrive-core`. Compile-time
    // type-identity witness; the snapshot/streaming surfaces share the
    // SAME type so byte-equality is structural.
    mod error_mapping_exhaustive;
    mod eval_broker_collapse;
    mod job_lifecycle_backoff;
    mod job_stop_idempotent;
    mod job_stop_intent_key;
    mod job_stop_unknown;
    // `pending_no_capacity_renders_reason` was retired in slice 01 step
    // 01-03: the legacy `AllocStatusRowBody.reason: Option<String>`
    // surface is replaced by the typed `Option<TransitionReason>` per
    // the cause-class refactor (ADR-0032 §3 Amendment 2026-04-30).
    // S-AS-06 in `alloc_status_render` covers the new contract:
    // Pending-no-capacity renders an explicit reason row, never
    // `Allocations: 0`.
    mod row_body_conversions;
    mod runtime_convergence_loop;
    mod runtime_registers_noop_heartbeat;
    mod submit_job_idempotency;
    mod transition_reason_type_identity;
    mod trust_triple_getters;

    // wire-exec-spec-end-to-end — operator-facing job spec carries
    // `[exec]` block end-to-end. Per ADR-0031.
    mod action_shim_restart_uses_spec_from_action;
    mod openapi_exec_block;
    mod submit_job_handler_rejects_empty_exec_command_with_400;

    // cli-submit-vs-deploy-and-alloc-status — Slice 02 step 02-01.
    // S-CP-04 broadcast property test + S-CP-05 classifier scenarios.
    mod lifecycle_broadcast;

    // cli-submit-vs-deploy-and-alloc-status — Slice 02 step 02-02.
    // SubmitEvent wire enum serde round-trip + literal wire-shape
    // regression assertions per ADR-0032 §3 Amendment 2026-04-30.
    mod submit_event_serialization;

    // cli-submit-vs-deploy-and-alloc-status — Slice 02 step 02-03.
    // Content-negotiated submit_job + streaming_submit_loop with
    // select! cap timer + lagged-recovery fallback.
    // Scenarios: S-CP-01, S-CP-02, S-CP-03, S-CP-06, S-CP-07,
    // S-CP-08, S-CP-10 (#[ignore]'d per wave-decisions.md).
    mod streaming_submit;
}
