//! Acceptance test entrypoint for `overdrive-reconcilers`.
//!
//! Wires the per-scenario modules under `tests/acceptance/*.rs` into Cargo's
//! single integration-test binary (ADR-0005 layout).

// `expect` / `expect_err` are the standard idiom in test code — a panic with a
// message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

mod acceptance {
    //! Step 02-02 (S2 of ADR-0086) — crate-extraction gate.
    mod crate_extraction_import_rewrite_compiles;
    // service-kind-vm-workloads (GH #257) — proves that VM-originated probe
    // rows retain the existing ServiceLifecycle/WorkloadLifecycle ownership
    // split. RED scaffolds only; the reconcilers are reused unchanged.
    mod service_kind_vm_workloads;

    // vm-recreation-allocation-id-reuse corrective re-DISTILL (GH #284 /
    // ADR-0105/0108/0109) — driver-neutral WorkloadLifecycle acceptance
    // properties for predecessor→fresh-successor RestartAllocation,
    // accepted-row current selection, candidate-keyed retry memory, durable
    // issued-ID reservations, terminal handoff, and checked exhaustion.
    // Complete bodies carry reasoned markers owned by the future re-DELIVER
    // roadmap; the rejected VM/Exec action split is not retained.
    mod vm_recreation_allocation_identity;
}
