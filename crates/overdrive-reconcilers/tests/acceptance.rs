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
}
