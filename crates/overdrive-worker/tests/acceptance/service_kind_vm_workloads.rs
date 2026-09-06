//! RED acceptance scaffolds for ADR-0090's existing `ProbeRunner` and
//! `VmDriver` boundaries. No fallback case for `Vm + None` is present because
//! the accepted design establishes `Some(workload_addr)` as a production
//! registration precondition.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::missing_panics_doc)]

/// S-SVM-10 — for a VM allocation, omitted HTTP host, wildcard HTTP host,
/// and wildcard TCP host each resolve once to the provisioned guest
/// `workload_addr`; the persisted descriptor remains unchanged.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_default_and_wildcard_network_probe_targets_resolve_to_workload_addr_once() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-10 / VM default target projection)");
}

/// S-SVM-11 — every non-wildcard explicit HTTP/TCP host is passed byte-for-
/// byte to the existing prober adapter for both VM and Exec allocations.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[should_panic(expected = "RED scaffold")]
fn explicit_network_probe_hosts_are_preserved_for_both_drivers() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-11 / explicit host preservation)");
}

/// S-SVM-12 — Exec/process omitted or wildcard HTTP/TCP targets retain their
/// existing loopback semantics; the VM change cannot move that baseline.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[should_panic(expected = "RED scaffold")]
fn exec_default_network_probe_targets_remain_loopback() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-12 / Exec loopback compatibility)");
}

/// S-SVM-13 — TCP reaches the projected guest address and records Pass on
/// connect success or Fail with the existing reason on a closed guest port;
/// neither result changes allocation Running state.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_tcp_probe_records_guest_connect_outcome_without_owning_running() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-13 / VM TCP result)");
}

/// S-SVM-14 — HTTP reaches the projected guest URL: 2xx (including 204) is
/// Pass, 3xx/4xx/5xx (including 302 and 503) is Fail with the numeric status,
/// and the response body is never consumed without bound.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_http_probe_preserves_status_policy_and_bounded_body_handling() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-14 / VM HTTP status contract)");
}

/// S-SVM-15 — `VmDriver` receives the one shared trusted runner as its exact
/// fifth constructor argument and delegates Running, Stable, and terminal to
/// the same existing hooks as `ExecDriver`; no new Driver method exists.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_driver_delegates_existing_probe_lifecycle_hooks_to_shared_runner() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-15 / VmDriver hook delegation)");
}

/// S-SVM-16 — registering the same allocation again after restart is
/// idempotent: the effective target is re-derived from the current full
/// `AllocationSpec`, while exactly one supervisor/task set remains live.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_restart_reregistration_is_idempotent_and_does_not_duplicate_probe_tasks() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-16 / restart re-registration)");
}
