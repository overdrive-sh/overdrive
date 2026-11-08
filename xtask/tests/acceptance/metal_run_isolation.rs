//! Developer-tool acceptance tests for the existing Metal shell driving ports.
//!
//! State machine: unregistered -> waiting_for_lease -> preparing -> running
//! -> terminal -> stopped/completion. EOF/ABORT before START cannot run a body;
//! controller loss after START cannot cancel one. Collection is read-only.
//!
//! This is tooling integration, following `metal_qualification_tests` in
//! `xtask/src/main.rs`: Bash is the runner under test, not the Overdrive product
//! binary. SSH/systemctl substitutes exercise external transport/observation
//! partitions only; they never provide kernel, guest, or native evidence.
#![allow(clippy::expect_used, clippy::expect_fun_call)]
#![allow(
    clippy::doc_markdown,
    reason = "Contract Shape declarations retain their required literal spelling"
)]

#[path = "metal_run_isolation/admission.rs"]
mod admission;
#[path = "metal_run_isolation/collection.rs"]
mod collection;
#[path = "metal_run_isolation/controller.rs"]
mod controller;
#[path = "metal_run_isolation/finalization.rs"]
mod finalization;
#[path = "metal_run_isolation/identity.rs"]
mod identity;
#[path = "metal_run_isolation/placement.rs"]
mod placement;
#[path = "metal_run_isolation/qualification.rs"]
mod qualification;
#[path = "metal_run_isolation/support.rs"]
mod support;

#[cfg(target_arch = "x86_64")]
#[path = "metal_run_isolation/namespace_trace.rs"]
mod namespace_trace;

#[path = "metal_run_isolation/native_controller.rs"]
mod native_controller;
