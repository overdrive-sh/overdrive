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
    mod concurrent_submit_toctou;
    mod describe_round_trip;
    mod idempotent_resubmit;
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
    mod submit_round_trip;
    /// `TerminalCondition` propagation — step 02-02 of
    /// `reconciler-memory-redb`. Action shim threads `Action.terminal`
    /// onto BOTH `AllocStatusRow.terminal` AND `LifecycleEvent.terminal`
    /// in the same call frame; per ADR-0037 §4 drift between the two
    /// surfaces is structurally impossible.
    mod terminal_propagation;
    mod tls_bootstrap;
    /// phase-1-first-workload — slice 3 (US-03) — Linux-only walking
    /// skeletons. Each scenario file gates itself with
    /// `#[cfg(target_os = "linux")]` so the module declarations
    /// compile cleanly on macOS/Windows even when no test bodies
    /// exist there.
    pub mod job_lifecycle {
        // Shared cleanup helper — reaps real `/bin/sleep` workloads
        // spawned by the action shim so nextest does not flag the
        // tests as `LEAK`. Used by `crash_recovery` and
        // `submit_to_running`; `stop_to_terminated` cleans up via the
        // production stop path under test. `pub` so the slice-4
        // `cgroup_isolation::cluster_status_under_burst` test can
        // reuse the same `AllocCleanup` guard via `super::super::`.
        #[cfg(target_os = "linux")]
        pub mod cleanup;
        mod convergence_loop_spawned_in_production_boot;
        mod crash_recovery;
        mod crash_recovery_obs_write_rejected;
        mod exit_observer;
        mod stop_to_terminated;
        mod submit_to_running;
        /// Wait helpers for Tier-3 integration tests that drive the
        /// spawned convergence loop via `SimClock`. See module docs.
        pub mod wait;
    }
    /// phase-1-first-workload — slice 4 (US-03 final) — Linux-only
    /// cgroup-isolation harness. Per ADR-0028 the control-plane
    /// boots through a 4-step pre-flight check + creates its own
    /// slice. The scenario files gate themselves with
    /// `#[cfg(target_os = "linux")]` so the module declarations
    /// compile cleanly on macOS/Windows.
    mod cgroup_isolation {
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
    }
}
