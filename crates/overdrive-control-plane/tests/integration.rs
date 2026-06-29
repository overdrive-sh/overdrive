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

    // built-in-ca (GH #28) — DISTILL RED scaffolds. `ca_equivalence` is the
    // central `Ca` trait-contract enforcement test driving BOTH `RcgenCa`
    // (host) and `SimCa` (sim) through the same call sequence (ADR-0063 D8;
    // development.md § "The DST equivalence test is the structural guard").
    // This crate is the only home — it dev-deps both host and sim adapters.
    // `ca_boot_and_audit` proves the consumer wiring: refuse-to-start
    // (Earned Trust), root reuse across restart, audit-row write, no silent
    // issuance, re-issue without restart.
    mod ca_boot_and_audit;
    mod ca_equivalence;

    /// transparent-mtls-enrollment step 04-04 (D-TME-12 § "Amended 2026-06-18
    /// (02-06 adopt-on-restart)", §1–§4) — Tier-3 acceptance for the
    /// `run_server` boot-recovery pass. Drives the PRODUCTION
    /// `veth_provisioner::adopt_on_restart_recovery` seam against a real
    /// surviving netns + spawned PID + cgroup scope + Running obs row and a
    /// real orphan netns: the fresh (post-restart) allocator ADOPTS the
    /// survivor's slot (a later `assign` cannot collide) and the orphan netns
    /// is GC'd. Root + CAP_NET_ADMIN; SKIP otherwise.
    mod adopt_on_restart;
    /// transparent-mtls-enrollment step 04-01 (Path A / ADR-0071, D-TME-12 /
    /// AC14) — Tier-3 acceptance for the C3 action-shim seam. Drives the
    /// PRODUCTION `action_shim::dispatch` with `mtls_worker: Some(...)` (the
    /// ACTIVE seam path) + a real `NetSlotAllocator`: provision-before-spawn +
    /// lands-in-`ovd-ns-<slot>` (`ip netns identify`) + terminal teardown
    /// (root-gated, SKIP otherwise), and provision-failure → `Failed` row
    /// carrying `WorkloadNetnsProvisionFailed` (deterministic, slot exhaustion,
    /// no root — mirrors `fail_closed_on_mtls_install`).
    mod alloc_netns_lifecycle;
    mod concurrent_submit_toctou;
    /// Action-shim `deregister_local_backend::dispatch` mutation kill
    /// per ADR-0053 § 3 — asserts the post-dispatch observable state
    /// on `SimDataplane::local_backend_for`.
    mod deregister_local_backend_dispatch;
    mod describe_round_trip;
    /// dial-by-name-responder step 02-01 (ADR-0072 REV-2, GH #243) — Tier-3
    /// acceptance for the `DnsResponder` socket loop + the `run_server`
    /// Earned-Trust boot gate (S-DBN-BIND-01/02/03). Drives the PRODUCTION
    /// `DnsResponder::{probe,serve}` (wildcard `0.0.0.0:53` IP_PKTINFO bind,
    /// per-gateway-addr fallback, `ipi_spec_dst` source-pin) and the boot
    /// refuse-on-unbindable-port / unreadable-store path. `getent` is the
    /// source-pin litmus (NEVER `dig` alone, DDN-5). Root + Lima; SKIP
    /// otherwise. No Tier-2 backstop (DDN-4) — irreducibly real-kernel.
    mod dns_responder_bind;
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
    /// single-node-dataplane-wiring step 01-04 (ADR-0061 § 8 / § 5) —
    /// Tier-3 regression for the serve-boot dataplane wiring. Drives the
    /// real `provision` + `EbpfDataplane::new_with_pin_dir` seam: (HAPPY)
    /// default veth config attaches TWO DISTINCT XDP programs to TWO
    /// distinct veth ifaces with NO `DataplaneBootError`, asserted via
    /// observable kernel state (`ip link show prog/xdp id`); (DIAGNOSTIC)
    /// both ifaces = one real iface surfaces the typed
    /// `DataplaneError::IfaceXdpSlotBusy` on a REAL `EBUSY`.
    mod serve_boot_provisions_veth;
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
    /// transparent-mtls-enrollment step 02-02 (Path A / ADR-0071) — Tier-3
    /// per-allocation netns + veth provision. Drives real `ip netns` /
    /// `ip -n <ns>` / `sysctl` / `ethtool` through
    /// `veth_provisioner::provision_workload_netns` + `teardown_workload_netns`:
    /// fresh-create (netns + pair, all three up-states, addresses, default
    /// route, ip_forward, rp_filter, tx-off) + idempotent re-converge +
    /// half-provisioned heal + zero-residue teardown.
    mod workload_netns_provision;
    /// workflow-primitive DISTILL (GH #39, J-PLAT-005) — `@real-io`
    /// journal-persistence scenario S-WP-01-04 (US-WP-2 AC1/AC2, K5/O6).
    /// Exercises a REAL `RedbJournalStore` (real redb file) per
    /// `.claude/rules/testing.md` § "Integration vs unit gating".
    /// `#[should_panic(expected = "RED scaffold")]` until DELIVER slice 01
    /// lands `RedbJournalStore` + the `JournalStore` port (ADR-0066).
    pub mod workflow_journal {
        mod journal_writes_to_redb;
    }
    /// workflow-primitive slice 01 — step 01-08 — end-to-end production
    /// composition proof (ADR-0064 §5). A fixture trigger reconciler emits
    /// `Action::StartWorkflow` → the real `WorkflowEngine` composed into
    /// `AppState` drives `ProvisionRecord` to terminal → the
    /// `WorkflowTerminal` observation row appears → the workflow-lifecycle
    /// reconciler converges to terminated; the `ctx.call` effect fired
    /// exactly once. Real redb (journal + obs + intent) + real engine.
    pub mod workflow_e2e {
        mod reconciler_emit_drives_workflow_to_terminal;
        // step 03-03 — a fixture trigger reconciler emits StartWorkflow for
        // an EmittingWorkflow whose ctx.emit_action flows through the
        // PRODUCTION emit-drain task into action_shim::dispatch, driving a
        // second workflow to a terminal observation row.
        mod workflow_emit_action_drives_through_production_composition;
    }
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

    /// canonical-workload-address-inbound-tproxy (GH #241) — S-WS keystone.
    /// RELOCATED here (R1) from the `overdrive-worker` test tree because the
    /// keystone drives in-process `run_server` (the production boot composition
    /// root + the composed mTLS worker), which lives in `overdrive-control-plane`
    /// — a worker-crate test physically cannot reach it (the reverse dep edge is
    /// a Cargo-rejected cycle). Drives the REAL `EbpfDataplane` (NO
    /// `dataplane_override`, so `compose_mtls` composes the mTLS worker) +
    /// `mtls_identity_override = Some(TestPki)`; deploys two mesh workloads and
    /// proves a client dialing the server's canonical `workload_addr:service_port`
    /// is captured by the PRODUCTION-installed (03-01) inbound nft-TPROXY rule,
    /// mTLS terminates, and the round-trip completes. Root + CAP_NET_ADMIN;
    /// SKIP otherwise. MERGE-BLOCKING on the pinned-6.18 Tier-3 matrix (ADR-0068).
    mod canonical_address_inbound_walking_skeleton;

    /// dial-by-name-responder step 02-02 (ADR-0072 REV-2, GH #243) — the
    /// WALKING-SKELETON vertical slice. Four Tier-3 scenarios sharing one
    /// real-`EbpfDataplane` + `mtls_identity_override` boot fixture (the
    /// keystone shape): a deployed workload resolves its peer's STABLE
    /// frontend `F ∈ 10.98.0.0/16` via `getaddrinfo`/`getent` (NOT `dig` —
    /// K2) from inside its production-provisioned netns, the connect to `F`
    /// is captured by the production egress nft-TPROXY rule, the re-keyed
    /// `MtlsResolve` translates `F` → the live backend, and the hop is
    /// mTLS'd (S-DBN-WS); `F` is byte-stable across a backend alloc cycle
    /// (S-DBN-WS-STABLE — the SQ1 elimination); the answered `F` is the addr
    /// `MtlsResolve` recognizes + translates (S-DBN-SINGLE-SRC); in-flight
    /// churn fails fast bounded by `TCP_USER_TIMEOUT`, no `sock_destroy`
    /// (S-DBN-CHURN). Drives ONLY production: NO test binds `:53`, installs a
    /// `resolv.conf`, allocates `F`, programs a map, or hand-installs the
    /// egress capture. Root + Lima; SKIP otherwise. MERGE-BLOCKING on the
    /// pinned-6.18 Tier-3 matrix (ADR-0068).
    mod dns_responder_walking_skeleton;

    /// dial-by-name-responder EMPTY-CANDIDATE HONESTY (roadmap 03-01;
    /// US-DBN-4 · K-DBN-2). Tier-3 `getent` observables that the production
    /// responder WITHHOLDS the answer (NXDOMAIN) for a name with no
    /// running-and-healthy backend (S-DBN-NXDOMAIN-01 query-before-healthy;
    /// S-DBN-NXDOMAIN-02 withheld-after-stop, F-not-released; S-DBN-NXDOMAIN-03
    /// unknown name), resolving to the stable frontend `F` once a backend is
    /// running-and-healthy. Drives ONLY production: NO test binds `:53`,
    /// installs a `resolv.conf`, allocates `F`, or programs a map. Root + Lima;
    /// SKIP otherwise. The NXDOMAIN-02 recovery-after-stop observable is
    /// `#[ignore]`'d to overdrive#249 (sticky operator-stop; same dependency as
    /// 02-02 S-DBN-WS-STABLE). MERGE-BLOCKING on the pinned-6.18 Tier-3 matrix
    /// (ADR-0068).
    mod dns_responder_nxdomain;

    /// dial-by-name-responder step 03-02 (ADR-0072 REV-2, GH #243; US-DBN-3 ·
    /// K-DBN-3) — the BIDIRECTIONAL PING-PONG demo, the operator-runnable proof.
    /// Two services dial each other by name: `a` resolves `b.svc.overdrive.local`
    /// and calls B, `b` resolves `a.svc.overdrive.local` and calls A; each call
    /// increments a counter + refreshes a date on a ~10s cadence; each hop is
    /// resolved through the in-agent responder, then intercepted + mTLS'd
    /// (S-DBN-PINGPONG). Drives ONLY production: two `overdrive deploy`s
    /// (`examples/dial-by-name-responder/{a,b}.toml`) + a staged tiny Rust
    /// ping-pong bin against `run_server` — NO test binds `:53`, installs a
    /// `resolv.conf`, allocates `F`, programs a map, or hand-installs the egress
    /// capture. Each egress hop uses the CORRECTED PLAINTEXT-egress model (the
    /// dialer speaks plaintext; the mTLS proof lives on the inter-agent
    /// leg-B ↔ leg-C wire — CLAUDE.md § "East-west mTLS tests" / the 02-02 RCA).
    /// `#[should_panic(expected = "RED scaffold")]` per
    /// `distill/red-classification.md` (S-DBN-PINGPONG): the operator-runnable
    /// bidirectional proof graduates to the E05 EDD expectation (honest
    /// `pending`, #227/#75), NOT an in-process `#[test]`. Root + Lima; SKIP
    /// otherwise. MERGE-BLOCKING on the pinned-6.18 Tier-3 matrix (ADR-0068).
    mod dns_responder_ping_pong;

    /// workload-identity-manager (GH #35) — DISTILL RED scaffolds for the
    /// Layer-3 walking skeleton and bounded audited restart re-issue.
    pub mod workload_identity_manager {
        mod lifecycle;
    }

    /// built-in-ca-operator-composition (folds GH #40 + GH #215) — DISTILL RED
    /// scaffolds for the Layer-3 (Lima) operator surfaces this feature
    /// completes: `overdrive serve` persistent-CA boot / restart-adopt /
    /// refuse-to-start (Slice ②, EDD D01/O04), and the rotate-correlation
    /// `IssueSvid` executor dispatch (Slice ①). Per
    /// docs/feature/built-in-ca-operator-composition/distill/test-scenarios.md
    /// (S-OC-06..10). `#[ignore]` until the production wiring lands per slice —
    /// the runtime surface is `#[cfg(feature = "integration-tests")]` and only
    /// reachable under Lima.
    ///
    /// The Slice ③ issued-certificates summary render (S-OC-11 / S-OC-12, EDD
    /// O05) lives in `overdrive-cli`
    /// (`tests/integration/alloc_status.rs`), NOT here: its driving port is
    /// the single live `overdrive_cli::render::alloc_status` renderer, and
    /// `overdrive-control-plane` cannot depend on `overdrive-cli` (that is the
    /// only illegal dependency direction). The original DISTILL placement was a
    /// defect, relocated single-cut in step 03-02.
    mod built_in_ca_operator_composition {
        /// Feature-delta § D1-AMEND — the `alloc_status` server projection
        /// selects the CURRENT issued cert by monotonic `IssuanceOrdinal`,
        /// not `issued_at` (closes step-0302 review findings 1 + 2). Drives
        /// the real `alloc_status` handler; replaces the deleted misplaced
        /// render-layer scaffold.
        mod alloc_status_issued_certificates;
        mod rotate_issue_svid_dispatch;
        mod serve_persistent_ca;
    }
}
