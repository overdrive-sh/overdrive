//! RED acceptance scaffolds pinning lifecycle gate ownership for VM Service
//! probe observations. These are regression extensions over reused
//! reconcilers, not authority to add VM-specific lifecycle state.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::missing_panics_doc)]

/// S-SVM-18 — Beacon/driver start commits Running before probe registration.
/// A closed guest startup port leaves Running intact and only the existing
/// startup-attempt/deadline path can produce `StartupProbeFailed`.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_startup_failure_leaves_running_owned_by_beacon_and_fails_only_startup() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-18 / Running versus startup ownership)");
}

/// S-SVM-19 — a VM startup Pass makes `ServiceLifecycle` emit Stable with the
/// existing witness/timing shape. It does not write readiness health or make
/// a restart decision.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_startup_pass_changes_only_service_stable() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-19 / startup owns Stable)");
}

/// S-SVM-20 — readiness 204 -> 503 -> 204 flips only
/// `Backend.healthy` true -> false -> true at the existing thresholds; the
/// allocation remains Running and Stable and no RestartAllocation is emitted.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_readiness_flaps_only_backend_eligibility_and_recovers() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-20 / readiness owns backend health)");
}

/// S-SVM-21A — the liveness threshold makes `ServiceLifecycle` emit only the
/// existing liveness `StopAllocation`; success before threshold resets the
/// counter.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_liveness_threshold_emits_only_the_existing_liveness_stop() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-21A / liveness stop owner)");
}

/// S-SVM-21B — after the liveness-stopped row is observed,
/// `WorkloadLifecycle` alone chooses restart versus final failure under the
/// unified budget.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn workload_lifecycle_alone_decides_restart_after_liveness_stop() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-21B / restart owner)");
}
