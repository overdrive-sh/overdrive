//! Acceptance test entrypoint for `overdrive-sim`.
//!
//! Each scenario from `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! is translated to a Rust integration-test module under
//! `tests/acceptance/*.rs` per ADR-0005. This entrypoint wires those
//! modules into Cargo's single integration-test binary.

// `expect` / `expect_err` are the standard idiom in test code.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

mod acceptance {
    //! Phase-1-foundation acceptance scenarios.

    // US-04 — ObservationStore trait + SimObservationStore.
    mod sim_observation_gossip;
    mod sim_observation_gossip_mechanics;
    mod sim_observation_lww_converges;
    mod sim_observation_row_readers;
    mod sim_observation_single_peer;

    // Step 01-01 — LWW conformance harness invocation. RED scaffold;
    // GREEN counterpart is step 01-02 (lands the harness in
    // `overdrive-core::testing::observation_store`).
    mod lww_conformance;

    // fix-issuance-ordinal-toctou Step 01-01 — issuance-ordinal
    // allocation conformance against `SimObservationStore` (ADR-0063 D6
    // rev 8). Drives `next_issuance_ordinal` through the DST-equivalence
    // sequence: monotonic-and-unique under concurrency + independent of
    // the audit table.
    mod issuance_ordinal_conformance;

    // US-06 §6.1 — Sim adapters for every nondeterminism port.
    mod sim_adapters_deterministic;

    // fix-terminated-slot-accumulation Step 01-01 — RED scaffold:
    // SimDriver allocations map cardinality must return to zero after
    // start+stop cycles. Mirror of the host-side regression in
    // `crates/overdrive-worker/tests/integration/exec_driver/
    // live_map_bounded.rs`.
    mod sim_driver_live_map_bounded;

    // Step 04-05 — Reconciler-primitive DST invariants.
    mod reconciler_invariants_pass;

    // Step 02-02 — `ReconcilerIsPure` holds for `WorkloadLifecycle` (scenario 3.2).
    mod reconciler_is_pure_with_workload_lifecycle;

    // reconciler-memory-redb Step 01-03 — `SimViewStore` is a lossless
    // CBOR-byte cache for arbitrary `View` values (ADR-0035 §2 /
    // wave-decisions §D6 `ViewStoreRoundtripIsLossless`).
    mod sim_view_store;

    // built-in-ca (GH #28) — DISTILL RED scaffolds for `SimCa` DST
    // determinism (ADR-0063 D1/D7, KPI K5): fixture P-256 keys +
    // SeededEntropy serials -> bit-identical issuance from a seed. Layer 2,
    // example-only per Mandate 9 (DST determinism is same-seed-same-bytes).
    mod sim_ca_deterministic;
    // workflow-primitive DISTILL (GH #39, J-PLAT-005) — the DST-invariant
    // home for the durable-workflow scenarios per
    // `docs/feature/workflow-primitive/distill/test-scenarios.md`. All
    // `#[should_panic(expected = "RED scaffold")]`; the engine,
    // `SimJournalStore`, and the graduated `ReplayEquivalenceProvision
    // Record` / `WorkflowJournalWriteOrdering` / `WorkflowExactlyOnce
    // EffectOnResume` invariants (ADR-0064 §6) land in DELIVER slices
    // 01–03.  Slice 01:
    mod journal_records_inputs_not_derived; // S-WP-01-05
    mod replay_equivalence_provision_record_invariant; // S-WP-01-09 (K4)
    mod workflow_committed_step_survives_crash; // S-WP-01-07
    mod workflow_crash_resume_exactly_once; // S-WP-01-06 WALKING SKELETON
    mod workflow_journal_write_ordering; // S-WP-01-10
    // Slice 02 (ctx.sleep):
    mod replay_equivalence_holds_across_sleep; // S-WP-02-04
    mod workflow_sleep_crash_pre_sleep_step_not_repeated; // S-WP-02-01
    mod workflow_sleep_records_deadline_not_remaining; // S-WP-02-03
    mod workflow_sleep_resumes_to_original_deadline; // S-WP-02-02
    // Slice 03 (signals + emit):
    mod replay_equivalence_holds_across_signal_and_emit; // S-WP-03-05
    mod workflow_emit_action_at_least_once_on_failed_record; // emit_action at-least-once (live-path)
    mod workflow_emit_action_not_re_emitted_after_crash; // S-WP-03-04
    mod workflow_signal_already_seen_not_rewaited; // S-WP-03-02
    mod workflow_signal_wait_reblocks_after_crash; // S-WP-03-01

    // workload-identity-manager (GH #35) — DISTILL RED scaffolds for
    // `SimIdentityRead` equivalence and the running-set identity DST
    // invariant. DELIVER replaces the pending bodies once the sim double and
    // invariant are introduced.
    mod identity_read_equivalence;

    // transparent-mtls-host-socket (GH #26) step 02-02 — the structural guard
    // for the `MtlsEnforcement` contract. Drives `SimMtlsEnforcement` through the
    // 4-method contract sequence for BOTH directions and asserts the
    // Established-vs-fail-closed OUTCOME (ADR-0069 F3). The host-arm equivalence
    // evidence is the real-kernel Tier-3 `mtls_agent_handshake` /
    // `mtls_composed_walking_skeleton` (the host adapter's `enforce` is a kernel
    // I/O boundary, ungated-in-sim by design).
    mod mtls_enforcement_equivalence;
}
