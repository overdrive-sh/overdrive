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
    // single-node-dataplane-wiring step 01-03 (ADR-0061 § 1) — shared
    // `lo`-named `DataplaneConfig` helper for SimDataplane-override
    // fixtures (the `job_stop_*` acceptance tests boot `run_server`).
    // `#[path]`-included (each `tests/*.rs` is its own crate root) so
    // the same SSOT source backs both the acceptance and integration
    // binaries and the `lo`/`lo` shape cannot drift. Gated behind
    // `integration-tests` — its only consumers (`job_stop_*`) are.
    #[cfg(feature = "integration-tests")]
    #[path = "../common/dataplane_lo.rs"]
    pub mod dataplane_lo;

    // S-CP-09 — AllocStatusRow rkyv round-trip with the new
    // `reason: Option<TransitionReason>` and `detail: Option<String>`
    // fields per ADR-0032 §3 (Amendment 2026-04-30) and §4.
    mod alloc_status_row_archive_roundtrip;

    // S-AS-01 / S-AS-07 / S-AS-08 / S-AS-09 (Slice 01 step 01-03) —
    // `AllocStatusResponse` extension, handler hydration via observation
    // rows + `WorkloadLifecycleView`, 404 on missing job. KPI-03 satisfaction.
    mod alloc_status_snapshot;

    mod api_type_shapes;
    mod cluster_status_lists_both_reconcilers;

    // S-AS-02 (Slice 01 step 02) — `TransitionRecord.reason` is the
    // `TransitionReason` enum from `overdrive-core`. Compile-time
    // type-identity witness; the snapshot/streaming surfaces share the
    // SAME type so byte-equality is structural.
    mod error_mapping_exhaustive;
    mod eval_broker_collapse;
    #[cfg(feature = "integration-tests")]
    mod job_stop_idempotent;
    #[cfg(feature = "integration-tests")]
    mod job_stop_intent_key;
    #[cfg(feature = "integration-tests")]
    mod job_stop_unknown;
    mod workload_lifecycle_backoff;
    // `pending_no_capacity_renders_reason` was retired in slice 01 step
    // 01-03: the legacy `AllocStatusRowBody.reason: Option<String>`
    // surface is replaced by the typed `Option<TransitionReason>` per
    // the cause-class refactor (ADR-0032 §3 Amendment 2026-04-30).
    // S-AS-06 in `alloc_status_render` covers the new contract:
    // Pending-no-capacity renders an explicit reason row, never
    // `Allocations: 0`.
    // Tests for the deleted legacy `SubmitEvent` enum were removed
    // in step 01-03e3 alongside the enum deletion per single-cut
    // discipline (`CLAUDE.md` § `feedback_single_cut_greenfield_migrations.md`):
    //   * `submit_event_serialization` — proptest round-trips of the
    //     deleted `SubmitEvent` / `TerminalReason` enums.
    //   * `streaming_channel_closed` — exercised the legacy
    //     `build_stream` channel-closed arm via `TerminalReason::
    //     StreamInterrupted`; replaced by `service_submit_dispatch_wiring::
    //     s_shcp_wire_14_broadcast_closed_synthesises_stream_interrupted`
    //     in this same step.
    //   * `transition_reason_type_identity` — compile-time witness on
    //     `TransitionRecord.reason` (still pinned by `alloc_status_snapshot`).
    mod row_body_conversions;
    // `runtime_convergence_loop.rs` consumes the `*_for_test`
    // accessors on `ReconcilerRuntime` that are gated behind
    // `#[cfg(any(test, feature = "integration-tests"))]`. The
    // accessors are visible inside the crate's own `cfg(test)` build
    // but NOT inside the integration-test binary's view of the lib
    // crate (cfg(test) does not propagate to dependencies). Gate the
    // module to match.
    #[cfg(feature = "integration-tests")]
    mod runtime_convergence_loop;
    mod runtime_registers_noop_heartbeat;
    mod submit_job_idempotency;
    mod trust_triple_getters;

    // wire-exec-spec-end-to-end — operator-facing job spec carries
    // `[exec]` block end-to-end. Per ADR-0031.
    mod action_shim_restart_uses_spec_from_action;
    mod openapi_exec_block;
    mod submit_job_handler_rejects_empty_exec_command_with_400;

    // cli-submit-vs-deploy-and-alloc-status — Slice 02 step 02-01.
    // S-CP-04 broadcast property test + S-CP-05 classifier scenarios.
    mod lifecycle_broadcast;

    // cli-submit-vs-deploy-and-alloc-status — Slice 02 step 02-03.
    // Content-negotiated submit_workload + streaming_submit_loop with
    // select! cap timer + lagged-recovery fallback.
    // Scenarios: S-CP-01, S-CP-02, S-CP-03, S-CP-06, S-CP-07,
    // S-CP-08, S-CP-10 (#[ignore]'d per wave-decisions.md).
    mod streaming_submit;

    // issue-141-persist-backoff-inputs step 02-01 — runtime
    // construction-site verification: `run_convergence_tick`
    // populates `TickContext.now_unix` from the injected `Clock`
    // (`state.clock`), exactly once per tick.
    mod tick_context_now_unix_runtime;

    // GH #160 — `service_backends` ObservationStore table wires
    // through to `hydrate_desired` for `ServiceMapHydrator`.
    mod service_backends_hydrate_desired;

    // service-vip-allocator step 02-02 — TOML `[dataplane.vip_allocator]`
    // parser surface. S-VIP-15/16/17/18: section presence + delegation to
    // `VipRange::new` for the three type-level invariants + structured
    // `health.startup.refused` event on every refusal.
    mod vip_allocator_config_parsing;

    // service-vip-allocator step 02-03d — Service-arm submit_workload /
    // alloc_status code paths. Six S-VIP scenarios per ADR-0049
    // (amended 2026-05-15) + ADR-0050 + ADR-0051.
    mod service_vip_submit_acceptance;

    // service-vip-allocator step 03-02 — action-shim dispatch arm for
    // Action::ReleaseServiceVip. S-VIP-06 PARTIAL (dispatch layer only;
    // reconciler emission in 03-01; end-to-end S-VIP-06 + S-VIP-07 in
    // 03-03).
    mod release_service_vip_dispatch;

    // Regression: Service workload convergence must not panic via stale
    // `unreachable!()` in `read_job`. Gated behind `integration-tests`
    // for the same reason as `runtime_convergence_loop` — the
    // `run_convergence_tick` accessor is `#[cfg(any(test, feature =
    // "integration-tests"))]`.
    #[cfg(feature = "integration-tests")]
    mod service_workload_convergence_no_panic;

    // backend-discovery-bridge-service-reachability — Service-arm
    // convergence emission. Service workloads must produce a non-empty
    // alloc_status row stream with kind == Service. Closes the
    // coverage gap that let the read_job Service-arm defect slip past
    // the sibling no_panic test. See:
    // docs/feature/backend-discovery-bridge-service-reachability/deliver/rca-service-arm-convergence.md
    // Gated behind `integration-tests` for the same `run_convergence_tick`
    // visibility reason as `service_workload_convergence_no_panic`.
    #[cfg(feature = "integration-tests")]
    mod service_workload_emits_start_allocation;

    // backend-discovery-bridge-service-reachability — UI-05
    // architectural remediation (the cross-reconciler handoff RCA
    // surfaced during step 02-04 walking-skeleton investigation).
    // Two acceptance properties:
    //   1. The bridge emits `Action::EnqueueEvaluation` alongside
    //      every `WriteServiceBackendRow` so the
    //      `service-map-hydrator` ticks on the bridge-written row.
    //   2. The production boot registers BOTH the bridge AND the
    //      hydrator against the runtime (the hydrator was missing
    //      pre-UI-05; architecture.md § 4.7 / § 6 misclaimed it
    //      was `// existing` wiring).
    mod bridge_emits_enqueue_evaluation_for_hydrator;
    mod service_map_hydrator_registered_at_boot;

    // Regression: ADR-0028 ordering invariant — preflight must
    // execute before the workloads-slice bootstrap in `run_server`.
    // See `docs/feature/fix-preflight-ordering/bugfix-rca.md`.
    mod preflight_before_workloads_bootstrap;

    // service-health-check-probes — Tier 1 acceptance for the
    // `ServiceLifecycleReconciler` per ADR-0055 + wire shape
    // evolution per ADR-0056. RED scaffolds.
    //   * Slice 01 (US-01 / US-08): Stable / EarlyExit / StartupProbeFailed
    //   * Slice 04 (US-04 / K2): readiness → Backend.healthy
    //   * Slice 05 (US-05 / K3): liveness → RestartAllocation
    //   * Cross-cutting: reconcile-fn purity + View-no-derived-state
    //   * Wire shape: ServiceSubmitEvent::Stable / Failed serde roundtrip
    /// Service-health-check-probes step 01-03d — composition-root
    /// `ProbeRunner` Earned-Trust gate per ADR-0054 § 7.
    mod probe_runner_boot_gate;
    /// GAP-4 + GAP-5 corrective AT — production `ExecDriver` carries
    /// a wired `ProbeRunner` and its lifecycle hooks drive the
    /// supervisor on the runner. Closes the structural gap that pre-
    /// patch let the production composition root discard
    /// `Arc<ProbeRunner>` into an underscore-binding. See
    /// `.context/01-03-structural-gap-audit.md`.
    mod probe_runner_composition;
    // GAP-1 corrective patch — real ServiceLifecycle hydrate impls
    // (`hydrate_desired` / `hydrate_actual` join intent + observation
    // + LWW probe projection per the Phase 01 structural gap audit).
    // See `.context/01-03-structural-gap-audit.md`.
    mod service_lifecycle_hydrate;
    mod service_lifecycle_liveness;
    // GAP-7 closure — end-to-end witness that ProbeRunner → row →
    // hydrate → ServiceLifecycleReconciler emits Stable. See
    // `.context/01-03-structural-gap-audit.md` GAP-7.
    mod service_lifecycle_probe_to_stable;
    mod service_lifecycle_purity;
    mod service_lifecycle_readiness;
    // GAP-9 — runtime self-re-enqueue witness (Shape B). Uses the
    // `loaded_service_lifecycle_views_for_test` runtime accessor, which is
    // `#[cfg(any(test, feature = "integration-tests"))]`; cfg(test) does
    // not propagate to dependencies from an integration-test binary, so
    // gate the module behind the feature (mirrors `runtime_convergence_loop`).
    #[cfg(feature = "integration-tests")]
    mod service_lifecycle_runtime_reenqueue;
    mod service_lifecycle_stable;
    mod service_submit_event_taxonomy;
    mod service_submit_event_v2;

    // service-health-check-probes step 01-03e3 — handler dispatch
    // wiring for Service-kind submit. S-SHCP-WIRE-09 through
    // S-SHCP-WIRE-15 cover the production dispatch path from
    // handlers.rs:498 through build_service_stream end-to-end.
    mod service_submit_dispatch_wiring;

    // udp-service-support US-01 / S-01-F (ADR-0060 D1a) — RED scaffold.
    // IPv6 VIP rejected at the action-shim as an operator-visible Failed
    // row via ServiceFrontend::new (NOT a late opaque DataplaneError).
    mod service_frontend_ipv6_rejected;
}
