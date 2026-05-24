//! Acceptance test entrypoint.
//!
//! Each scenario from `docs/feature/{feature-id}/distill/test-scenarios.md`
//! is translated to a Rust integration-test module under
//! `tests/acceptance/*.rs` per ADR-0005. This entrypoint wires those
//! modules into Cargo's single integration-test binary.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

mod acceptance {
    //! Phase-1-foundation + phase-1-control-plane-core acceptance
    //! scenarios.

    // Phase-1-foundation acceptance scenarios.
    mod content_hash_cert_serial;
    mod core_newtype_roundtrip;
    mod core_newtype_validation;
    mod extended_newtype_completeness;
    mod spiffe_region_validation;

    // Phase-1-control-plane-core acceptance scenarios.
    mod aggregate_constructors;
    mod aggregate_roundtrip;
    mod aggregate_validation;
    mod intent_key_canonical;
    mod observation_row_display;
    mod reconciler_trait_surface;

    // Bug-fix `fix-observation-lww-merge` — function-level mutation-killing
    // surface for `LogicalTimestamp::dominates`. Trait-level conformance
    // is exercised from each adapter's test suite via
    // `overdrive_core::testing::observation_store::run_lww_conformance`.
    mod logical_timestamp_dominates;

    // phase-1-first-workload — branch-coverage tests pinning the
    // `WorkloadLifecycle::reconcile` decision points (Stop/Run/Restart).
    mod any_reconciler_dispatch;
    mod first_fit_place_branches;
    mod workload_lifecycle_reconcile_branches;

    // wire-exec-spec-end-to-end — operator-facing job spec carries
    // explicit `[exec]` block (command + args) and the projection
    // flows end-to-end through Job::from_submit → Action::Start/Restart.
    // Per ADR-0031.
    mod exec_constructors;
    mod exec_reconciler_purity;
    mod exec_roundtrip;
    mod exec_validation;

    // issue-141-persist-backoff-inputs — `UnixInstant` newtype for
    // portable wall-clock deadlines. Step 01-01 covers arithmetic +
    // constructor surface; step 01-02 covers Display/FromStr/Serde
    // completeness + proptest roundtrips; step 02-01 wires it through
    // `TickContext.now_unix` + introduces the `backoff_for_attempt`
    // const fn; subsequent steps wire it through `WorkloadLifecycleView`.
    mod unix_instant_arithmetic;
    mod unix_instant_completeness;

    // Step 02-01 — `TickContext.now_unix` field surface +
    // `backoff_for_attempt` const fn. The runtime construction-site
    // verification lives in the control-plane acceptance suite (the
    // core crate cannot build an `AppState` without circular deps).
    mod tick_context_now_unix;

    // Step 02-02 — `WorkloadLifecycleView` persists inputs
    // (`last_failure_seen_at: UnixInstant` is the canonical input;
    // a precomputed `Instant` deadline would have been a derived
    // value); deadline recomputed each tick from
    // `seen_at + backoff_for_attempt(restart_count)`. Restart-survival
    // idempotence is structural rather than coincidental — see
    // `.claude/rules/development.md` § "Persist inputs, not derived state".
    mod workload_lifecycle_recompute_deadline;

    // reconciler-memory-redb step 01-02 — `TerminalCondition` enum +
    // `AllocStatusRow.terminal` field (ADR-0037 prerequisite for the
    // Phase 02 action-shim wiring). Property: every variant + None
    // survives the rkyv roundtrip at the row level.
    mod terminal_condition_roundtrip;

    // reconciler-memory-redb step 01-05 — collapsed `Reconciler` trait
    // surface (single sync `reconcile`, typed `View` with
    // `Serialize + DeserializeOwned + Default + Clone + Send + Sync`
    // bounds, no `migrate` / `hydrate` / `persist`). Per ADR-0035 §1
    // and ADR-0036.
    mod collapsed_reconciler_trait;

    // reconciler-memory-redb step 02-01 — `WorkloadLifecycle::reconcile`
    // stamps `TerminalCondition` on the lifecycle-concluding `Action`
    // variants (`StopAllocation`, `FinalizeFailed`). Per ADR-0037 §4.
    // Property test asserts the terminal-decision logic is a pure
    // function of `(view.restart_counts, view.last_failure_seen_at,
    // desired.desired_to_stop)` against the fixed WorkloadLifecycle-internal
    // ceiling.
    mod workload_lifecycle_terminal_decision;

    // phase-2-xdp-service-map step 08-02 (Slice 08; ASR-2.2-04) —
    // `ServiceMapHydrator::reconcile` decision tree. Pins the
    // four-arm dispatch logic (Pending / Completed / Failed-same /
    // Failed-different fingerprints) plus the per-service retry
    // memory invariants: increment-on-dispatch, reset-on-convergence,
    // GC-of-stale-services. Lives in `tests/` rather than `src/`
    // because dst-lint scans `src/**/*.rs` for banned APIs even
    // under `#[cfg(test)]`; `Instant::now()` for `TickContext.now`
    // is the legitimate test-fixture exception.
    mod service_map_hydrator_reconcile;

    // workload-kind-discriminator Slice 01 — `WorkloadSpec` tagged
    // enum at the parser boundary (§1) + migrated `examples/coinflip.toml`
    // (§7). Per ADR-0047 §1, §2.
    mod coinflip_migration;
    mod workload_spec_parser;

    // workload-kind-discriminator Slice 06 — Service `[[listener]]`
    // spec shape per ADR-0047 §1. S-08-01..S-08-06 (per-scenario
    // parser tests) + S-08-10 (round-trip property test).
    mod listener_parser;
    mod listener_roundtrip;

    // service-vip-allocator Slice 02 step 02-01 — parser-level
    // rejection of operator-supplied `vip` field on `[[listener]]`
    // blocks (S-VIP-13, S-VIP-14). Per ADR-0049 § 5 the `Listener`
    // struct has no `vip` field; the parser rejects with a typed
    // `ParseError::UnknownField` variant naming `vip`.
    mod listener_rejects_vip_field;

    // workload-kind-discriminator Slice 05 — parser-side cron
    // required-field scenario. S-05-04 in distill/test-scenarios.md §5.
    mod schedule_parser;

    // workload-kind-discriminator Slice 02a (step 02-03) — typed
    // `TerminalCondition::Completed { exit_code: i32 }` /
    // `Failed { exit_code: i32 }` variants per ADR-0037 Amendment
    // 2026-05-10. Property: rkyv + serde JSON roundtrip preserves
    // the new variants for every `i32` exit code (1024 cases including
    // boundary values + common Unix exit codes).
    mod transition_reason_roundtrip;

    // workload-kind-discriminator Slice 02b (step 02-04) — Job-vs-Service
    // branching in `WorkloadLifecycle::reconcile` for natural-exit terminals.
    // Job kind emits typed `TerminalCondition::Completed { exit_code }` /
    // `Failed { exit_code }`; Service kind preserves existing
    // restart-budget semantics. Per ADR-0037 Amendment 2026-05-10 +
    // ADR-0047 §1.
    mod workload_lifecycle_natural_exit;

    // service-vip-allocator step 03-01 — `WorkloadLifecycle::reconcile`
    // emits `Action::ReleaseServiceVip` exactly once when a Service-kind
    // workload's allocation reaches a terminal-state observation row.
    // Per-layer scope: reconciler emission only (action-shim dispatch is
    // step 03-02; end-to-end lifecycle is step 03-03). Per ADR-0049
    // (amended 2026-05-15) + persist-inputs discipline on
    // `released_for_terminal: BTreeSet<ContentHash>`.
    mod workload_lifecycle_release_service_vip;

    // backend-discovery-bridge-service-reachability — UI-06
    // WorkloadLifecycle → BackendDiscoveryBridge dual-emit (closes F1
    // gap per audit-reconciler-handoff-topology.md). The reconciler
    // appends one `Action::EnqueueEvaluation` routed at the bridge
    // alongside every `StartAllocation` / `RestartAllocation` /
    // `StopAllocation` / `FinalizeFailed`. Mirrors UI-05.
    mod workload_lifecycle_enqueues_bridge_on_alloc_transitions;

    // service-health-check-probes — Tier 1 acceptance for the
    // `[[health_check.*]]` TOML parser surface per ADR-0057 + ADR-
    // 0058 default-inference rule + the `ProbeResultRowEnvelope`
    // V1 roundtrip + discriminant pinning per ADR-0054 §5 QR1.
    // Slices 01 / 02 / 03 / 07. RED scaffolds.
    mod health_check_toml_parse;
    mod probe_descriptor_roundtrip;
    mod probe_result_row_envelope;
}
