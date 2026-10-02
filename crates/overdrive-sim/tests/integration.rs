//! Integration-test entrypoint for `overdrive-sim`.
//!
//! Per `.claude/rules/testing.md` § "Integration vs unit gating":
//! integration tests live under `tests/integration/<scenario>.rs`
//! and are wired through this single entrypoint. Whole binary
//! gated behind `integration-tests` feature; per-scenario modules
//! inherit the gate without repeating the cfg attribute.
//!
//! Submodules MUST be declared inside an inline `mod integration { … }`
//! block — Cargo treats each `tests/*.rs` file as a crate root, so a
//! bare `mod foo;` resolves to `tests/foo.rs`, not
//! `tests/integration/foo.rs`. The inline wrapper shifts the lookup
//! base into the subdirectory.
//!
//! Phase 2.2 first integration scenario:
//! - `maglev_churn` — DST proptest of ASR-2.2-02 (≤ 1 % Maglev
//!   incidental disruption) and S-2.2-12 (Maglev determinism). RED
//!   scaffold; DELIVER fills the body per Slice 04.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![expect(
    clippy::doc_markdown,
    reason = "integration scenarios retain repository-required Contract Shape metadata"
)]

mod integration {
    #[allow(dead_code)]
    pub mod serve_ports {
        use std::collections::BTreeMap;
        use std::net::Ipv4Addr;
        use std::sync::Arc;

        use overdrive_control_plane::guest_network::GuestAddressPool;
        use overdrive_core::guest_network::{GuestNetworkExecGate, GuestNetworkExecWiring};
        use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
        use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
        use overdrive_sim::adapters::clock::SimClock;
        use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
        use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
        use overdrive_sim::adapters::{SimIdentityRead, SimMtlsIntercept, SimMtlsResolve};
        use overdrive_worker::mtls_intercept_port::MtlsIntercept;
        use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;

        pub fn worker() -> Arc<MtlsInterceptWorker> {
            let enforcement: Arc<dyn MtlsEnforcement> = Arc::new(SimMtlsEnforcement::new(
                Arc::new(SimIdentityRead::new(BTreeMap::new(), None)),
                MtlsLimits::default(),
            ));
            let resolve: Arc<dyn MtlsResolve> =
                Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
            let intercept: Arc<dyn MtlsIntercept> = Arc::new(SimMtlsIntercept::new());
            Arc::new(MtlsInterceptWorker::new(
                enforcement,
                resolve,
                Arc::new(SimClock::new()),
                intercept,
            ))
        }

        pub fn owner() -> Arc<SimSharedGuestNetworkOwner> {
            Arc::new(SimSharedGuestNetworkOwner::default())
        }

        pub fn exec_gate() -> Arc<GuestNetworkExecGate> {
            GuestNetworkExecWiring::new(Arc::new(SimClock::new())).gate()
        }

        pub fn pool() -> Arc<GuestAddressPool> {
            Arc::new(GuestAddressPool::new(
                "100.95.0.0/16".parse().expect("static guest prefix"),
                "ovd-gbr0".to_owned(),
                Ipv4Addr::new(100, 95, 0, 1),
                Ipv4Addr::new(100, 95, 0, 1),
            ))
        }
    }
    /// phase-2-xdp-service-map Slice 04 (US-04) — Maglev determinism
    /// + ≤ 1 % incidental disruption proptests per
    /// `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
    /// S-2.2-12, S-2.2-13. RED scaffolds; DELIVER fills the bodies.
    mod maglev_churn;
    mod service_backend_projection;

    /// `cargo dst` subprocess scenarios — relocated from xtask when the
    /// DST harness binary moved into overdrive-sim. See § "xtask is
    /// build / test / dev orchestration, NOT a runtime entry point" in
    /// `.claude/rules/development.md` for the layering rationale.
    mod dst_clean_clone_green;
    mod dst_harness_smoke;
    mod dst_seeded_reproduction;

    /// fix-exit-observer-running-gate step 01-05 (Solution 4) — DST
    /// invariant defending the post-condition that every `ExitEvent`
    /// consumed by the worker exit_observer produces at least one of
    /// (a) obs row write with state ∈ {Failed, Terminated}, (b)
    /// degraded `LifecycleEvent` carrying
    /// `TransitionReason::DriverInternalError`, or (c) structured
    /// `tracing::error!` naming the alloc_id. See
    /// `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
    /// (Solution 4) and the evaluator at
    /// `crates/overdrive-sim/src/invariants/
    /// exit_event_observable_outcome.rs`.
    mod exit_event_observable_outcome;

    /// workload-gc-absent-stale-allocs step 01-03 — DST integration
    /// scenarios for the absent-intent workload GC arm + resubmit
    /// race. See architecture.md § 7 of the feature dir, the
    /// evaluator at
    /// `crates/overdrive-sim/src/invariants/workload_gc_absent_intent.rs`,
    /// and GitHub issue #148 AC §1.3.
    mod workload_gc_absent_intent;

    /// netns-density-295 S-ND295-05D (recovery proof §3.2, moved here) —
    /// seeded node-wide held-attachment admission through the real
    /// reconciler, scheduler, and action shim. Filling one node to the
    /// 16,384 cap runs past the default-lane budget, so it lives in this
    /// binary with a nextest timeout override for this test alone.
    mod netns_density_node_admission;

    /// netns-density-295 S-ND295-57 — seeded reclaim of leftover guest
    /// networks across restart predecessors, stopped and deleted workloads,
    /// and at the cap (D-295-R11). The at-cap body fills one node to the
    /// fixed 16,384 cap through the production action shim for each of its
    /// seeds; its GREEN run time is unmeasured until DELIVER 07-03, so it is
    /// kept out of the default lane with a widened budget (the module doc).
    /// Both bodies share one seam fixture, so the file lives in this binary.
    mod netns_density_reclaim;

    /// built-in-ca (GH #28, ADR-0063 D9 review P2) — guards the `SimCa`
    /// fixture leaf cert↔key matched-pair invariant against silent
    /// desync (parses the fixture key/cert via `rcgen` / `x509-parser`,
    /// real-crypto byte parsing). Ports the host adapter's cert↔key
    /// correspondence proof to the sim fixture consts.
    mod sim_ca_fixture_cert_key_match;
}
