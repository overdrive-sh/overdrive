//! Acceptance test entrypoint for `overdrive-store-local`.
//!
//! Each scenario from `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! is translated to a Rust integration-test module under
//! `tests/acceptance/*.rs` per ADR-0005. This entrypoint wires those
//! modules into Cargo's single integration-test binary.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

mod acceptance {
    //! Phase-1-foundation — US-03 `LocalIntentStore` acceptance scenarios.
    //!
    //! Per ADR-0020 (drop `commit_index` from Phase 1) the
    //! `commit_index_monotonic` and `per_entry_commit_index` modules
    //! were deleted in step 01-04 of `redesign-drop-commit-index` —
    //! the counter they asserted no longer exists.
    mod local_store_basic_ops;
    mod local_store_error_paths;
    mod phantom_writes;
    mod put_if_absent;
    mod snapshot_roundtrip;

    // Phase-1-control-plane-core — step 03-06 `LocalObservationStore`.
    mod local_observation_store;

    // service-vip-allocator Phase 5 (Phase-5 aggregate mutation gate,
    // May 2026) — `IntentStore::scan_prefix` contract on `LocalIntentStore`,
    // plus the `open()`-time recovery-walk filter that skips
    // workload suffix-keys (`/stop`, `/kind`) while pre-decoding
    // aggregate envelope keys. Kills the three surviving mutants on
    // `redb_backend.rs:149` (`||` → `&&`), `:397` (`Ok(vec![])`), and
    // `:409` (`delete !`) — all introduced by this feature's
    // PersistentServiceVipAllocator + WorkloadIntent migration work.
    mod scan_prefix_contract;
}
