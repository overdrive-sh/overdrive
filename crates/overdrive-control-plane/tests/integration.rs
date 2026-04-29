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
    mod libsql_isolation;
    mod observation_empty_rows;
    mod server_lifecycle;
    mod submit_round_trip;
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
        mod stop_to_terminated;
        mod submit_to_running;
    }
    /// phase-1-first-workload — slice 4 (US-03 final) — Linux-only
    /// cgroup-isolation harness. Per ADR-0028 the control-plane
    /// boots through a 4-step pre-flight check + creates its own
    /// slice. The scenario files gate themselves with
    /// `#[cfg(target_os = "linux")]` so the module declarations
    /// compile cleanly on macOS/Windows.
    mod cgroup_isolation {
        mod allow_no_cgroups_flag;
        mod cluster_status_under_burst;
        mod idempotent_slice_creation;
        mod preflight_missing_cpu;
        mod preflight_no_delegation;
        mod preflight_proc_filesystems_unreadable;
        mod preflight_proc_self_cgroup_malformed;
        mod preflight_reads_enclosing_slice;
        mod preflight_v1_host;
        mod server_enrols_in_slice;
    }
}
