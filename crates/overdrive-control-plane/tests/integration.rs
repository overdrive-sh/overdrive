//! Integration-test entrypoint for `overdrive-control-plane`.
//!
//! Per `.claude/rules/testing.md` §"Integration vs unit gating":
//! integration tests — those that touch real infrastructure
//! (filesystem, network, subprocesses, real consensus / gossip) or
//! whose wall-clock exceeds the default unit-test budget — live under
//! `crates/{crate}/tests/integration/*.rs` and are wired into a single
//! Cargo integration-test binary by this entrypoint.
//!
//! Gated behind the `integration-tests` feature — see the feature
//! comment in `overdrive-control-plane/Cargo.toml`.

#![cfg(feature = "integration-tests")]
// `expect` is the standard idiom in test code — a panic with a message
// is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
#![allow(clippy::unwrap_used)]
// `submit_a_body` / `submit_b_body` style bindings naturally read as
// "submission A's body" vs "submission B's body" in scenarios that
// pin a property across two submits. The `similar_names` lint flags
// the shared prefix; renaming to `_payments_body` / `_frontend_body`
// would couple the variable name to a fixture detail, which is worse.
#![allow(clippy::similar_names)]

// The inline `mod integration { ... }` mirrors the `tests/acceptance.rs`
// pattern: an integration-test crate root resolves `mod foo;` against
// `tests/foo.rs`, not `tests/integration/foo.rs`. Wrapping the
// declarations in an inline module of the matching name shifts the
// lookup base so the per-scenario files under `tests/integration/`
// resolve naturally.
mod integration {
    //! Per ADR-0020 (drop `commit_index` from Phase 1) the
    //! `per_entry_commit_index` module was deleted in step 01-04 of
    //! `redesign-drop-commit-index` — the per-entry index assertion
    //! has no consumer on the post-ADR-0020 wire shape.

    // single-node-dataplane-wiring step 01-03 (ADR-0061 § 1) — shared
    // `lo`-named `DataplaneConfig` helper for SimDataplane-override
    // fixtures. `#[path]`-included (each `tests/*.rs` is its own crate
    // root) so the same SSOT source backs both the acceptance and
    // integration binaries and the `lo`/`lo` shape cannot drift.
    #[path = "../common/dataplane_lo.rs"]
    pub mod dataplane_lo;

    mod concurrent_submit_toctou;
    /// Action-shim `deregister_local_backend::dispatch` mutation kill
    /// per ADR-0053 § 3 — asserts the post-dispatch observable state
    /// on `SimDataplane::local_backend_for`.
    mod deregister_local_backend_dispatch;
    mod describe_round_trip;
    /// Slice 02c (step 02-05) of `workload-kind-discriminator` —
    /// `ExitObserver` stderr-tail capture per ADR-0033 Amendment
    /// 2026-05-10. Real `/bin/sh` workload writes 7 stderr lines;
    /// asserts the observer's terminal row carries the last 5.
    mod exit_observer_stderr_tail;
    mod idempotent_resubmit;
    /// Regression test for the boot-time `node_health` write per
    /// ADR-0025 § 3 step 5 (amended by ADR-0029). `start_local_node`
    /// in `run_server_with_obs_and_driver` writes the row. See
    /// `docs/feature/fix-orphaned-node-health-writer/deliver/rca.md`.
    mod node_health_writer_runs_at_boot;
    mod observation_empty_rows;
    /// `ReconcilerRuntime` ↔ `ViewStore` wiring (step 01-06 of
    /// `reconciler-memory-redb`). Probe-failure refusal + bulk-load at
    /// register + `WriteThroughOrdering` per ADR-0035 §5/§6.
    mod reconciler_runtime_view_store;
    /// `RedbViewStore` adapter (step 01-04 of `reconciler-memory-redb`).
    /// Real-fs round-trip + per-reconciler table isolation + Earned-Trust
    /// probe coverage per ADR-0035 § Earned Trust + §4.
    mod redb_view_store;
    /// Regression for the per-call `Box::leak` defect in
    /// `RedbViewStore::table_def` — see
    /// `docs/feature/refactor-reconciler-static-name/deliver/bugfix-rca.md`.
    /// Asserts `Reconciler::NAME` is a compile-time anchor and that
    /// `write_through_bytes` accepts `&'static str` directly.
    mod redb_view_store_no_leak;
    mod server_lifecycle;
    /// phase-2-xdp-service-map Slice 08 (US-08) — service-map
    /// hydrator dispatch RED scaffold per
    /// `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
    /// S-2.2-28. Body panics until DELIVER fills it.
    mod service_map_hydrator_dispatch;
    mod submit_round_trip;
    /// `TerminalCondition` propagation — step 02-02 of
    /// `reconciler-memory-redb`. Action shim threads `Action.terminal`
    /// onto BOTH `AllocStatusRow.terminal` AND `LifecycleEvent.terminal`
    /// in the same call frame; per ADR-0037 §4 drift between the two
    /// surfaces is structurally impossible.
    mod terminal_propagation;
    mod tls_bootstrap;
    /// single-node-dataplane-wiring step 01-02 — Tier-3 idempotent
    /// veth provision (ADR-0061 § 3.1). Drives real `ip(8)` through
    /// `veth_provisioner::provision`: creates-when-absent +
    /// adopts-pre-existing-without-recreating.
    mod veth_provision_idempotent;
    /// phase-1-first-workload — slice 3 (US-03) — walking skeletons.
    pub mod workload_lifecycle {
        // Shared cleanup helper — reaps real `/bin/sleep` workloads
        // spawned by the action shim so nextest does not flag the
        // tests as `LEAK`. Used by `crash_recovery` and
        // `submit_to_running`; `stop_to_terminated` cleans up via the
        // production stop path under test. `pub` so the slice-4
        // `cgroup_isolation::cluster_status_under_burst` test can
        // reuse the same `AllocCleanup` guard via `super::super::`.
        pub mod cleanup;
        mod convergence_loop_spawned_in_production_boot;
        mod crash_recovery;
        mod crash_recovery_obs_write_rejected;
        mod exit_observer;
        /// Step 01-01 of `fix-exit-observer-running-gate` — RED
        /// regression for the producer-ordering race between the
        /// action shim's `obs.write(Running)` and the worker exit
        /// observer's `ExitEvent` consumption. Today's
        /// `RetryOutcome::NoPriorRow` arm drops the event silently
        /// for sub-millisecond-exit workloads. See
        /// `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`.
        mod exit_observer_running_gate;
        mod stop_to_terminated;
        mod submit_to_running;
        /// Wait helpers for Tier-3 integration tests that drive the
        /// spawned convergence loop via `SimClock`. See module docs.
        pub mod wait;
    }
    /// phase-1-first-workload — slice 4 (US-03 final) —
    /// cgroup-isolation harness. Per ADR-0028 the control-plane
    /// boots through a 4-step pre-flight check + creates its own
    /// slice.
    mod cgroup_isolation {
        /// Step 01-01 of `fix-cgroup-subtree-control-delegation` —
        /// RED regression for the missing `subtree_control` write.
        /// See module-level docs for the GREEN transition (step
        /// 01-02). Holds AC2 + AC3 (sibling
        /// `alloc_start_does_not_emit_resource_limit_warning`).
        mod alloc_scope_has_writable_cpu_weight_and_memory_max;
        mod cluster_status_under_burst;
        mod idempotent_slice_creation;
        mod preflight_falls_back_to_parent_slice_on_empty_scope;
        mod preflight_missing_cpu;
        mod preflight_no_delegation;
        mod preflight_proc_filesystems_unreadable;
        mod preflight_proc_self_cgroup_malformed;
        mod preflight_reads_enclosing_slice;
        mod preflight_refuses_when_both_scope_and_parent_slice_lack_delegation;
        mod preflight_subtree_control_missing_is_not_delegation;
        mod preflight_subtree_control_unreadable;
        mod preflight_v1_host;
        mod server_enrols_in_slice;
        /// Step 01-02 of `fix-cgroup-subtree-control-delegation` —
        /// idempotency regression for both the control-plane and the
        /// new `create_workloads_slice_with_controllers` inits. A
        /// second boot of the supervisor must re-enable controllers
        /// as a kernel no-op rather than EBUSY.
        mod subtree_control_delegation_is_idempotent;
    }

    /// Runtime validator at the reconcile-output boundary — rejects
    /// `Vec<Action>` returns that target the same service-LB VIP from
    /// two or more write-Actions in one tick. Closes the inter-Action
    /// conflict gap Phase 16 D11 surfaced.
    mod reconcile_output_validator;

    /// `cargo openapi-{gen,check}` library + binary scenarios — relocated
    /// from xtask when the OpenAPI gate moved into overdrive-control-plane.
    /// Covers test-scenarios.md §3.3. See § "xtask is build / test / dev
    /// orchestration, NOT a runtime entry point" in
    /// `.claude/rules/development.md` for the layering rationale.
    mod openapi_gate;
    mod streaming_attempt_failed;
    /// service-vip-allocator step 03-03 — end-to-end VIP lifecycle:
    /// submit → allocate → action-shim release-dispatch → reuse on
    /// next submit. Owns S-VIP-06 (end-to-end) and S-VIP-07 (released-
    /// VIP reuse) per the DISTILL test-scenarios contract.
    mod vip_allocator_lifecycle;
    /// `backend-discovery-bridge-service-reachability` (joint #174 + #175)
    /// DISTILL — RED scaffolds per
    /// `docs/feature/backend-discovery-bridge-service-reachability/distill/test-scenarios.md`.
    /// Walking-skeleton (S-BDB-01, S-BDB-18, S-BDB-19) + boot-composition
    /// (S-BDB-11..S-BDB-17, S-BDB-20). All tests
    /// `#[should_panic(expected = "RED scaffold")]` until DELIVER Slices
    /// 1 and 2 land the bridge + production `EbpfDataplane` wiring.
    mod backend_discovery_bridge {
        mod boot_composition;
        /// Shared fixture for the walking-skeleton (S-BDB-01) — spawns
        /// a production server wired against a real `EbpfDataplane`
        /// + drives `submit_workload` through the real HTTPS client.
        /// Lives under `tests/` per architecture.md § 6.2 / Atlas Q1.
        mod test_server;
        mod walking_skeleton;
    }
}
