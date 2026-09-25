//! Shared guest-network direct-handler/public-API recovery conformance
//! (S-ND295-33).
//!
//! The production server handler is started directly, like the repository's
//! other server-composition tests, with its required protection and DNS ports
//! (D-295-R16). Workload/admission behavior is driven only through HTTPS. No
//! product CLI, subprocess SUT, trycmd fixture, or internal application call
//! authors workload state. Faults enter only through the accepted
//! `SimSharedGuestNetworkOwner` port scripting; the production supervisor
//! authors every recovery transition, quiescence, kill, and request.
//!
//! Admission, recovery progress, and the typed request are observed as
//! `test-scenarios.md` § *In-process observation* states: the harness steps the
//! injected clock and polls each observation once per step. The cadence the
//! oracles assert is ADR-0124's accepted contract (`accepted_cadence`).

#![cfg(feature = "integration-tests")]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    reason = "conformance test assertions use exact CONTRACT_SHAPE markers and diagnostic expectations"
)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::guest_network::GuestNetworkOperation;
use overdrive_core::guest_network::{
    SharedGuestNetworkComponent, SharedGuestNetworkFailStopCause,
};
use overdrive_sim::adapters::guest_network::{SimQuiesceOutcome, SimSharedGuestNetworkOwner};
use overdrive_system_conformance::accepted_cadence::{
    ATTEMPT_PERIOD, AUDIT_PERIOD, RECOVERY_ATTEMPTS, RECOVERY_DEADLINE,
};
use overdrive_system_conformance::{
    Admission, CleanupObservation, DirectHandlerHarness, DirectHandlerInstance, TraceEvent,
    TraceHistory, Trajectory, wait_until,
};
use serde_json::{Value, json};

/// Host directories a VM workload handler could leave residue in.
const HOST_RESIDUE_DIRECTORIES: [&str; 2] =
    ["/run/overdrive/vm", "/sys/fs/cgroup/overdrive.slice/workloads.slice"];

/// The owned-object counts the supervisor records when it abandons cleanup to
/// shutdown at the recovery deadline (FD 10030-10032).
const ABANDONED_COUNT_FIELDS: [&str; 7] =
    ["taps", "links", "pins", "maps", "managed_set_elements", "intercept_set_elements", "handles"];

fn service_request(id: &str) -> Value {
    json!({
        "spec": {
            "kind": "service",
            "id": id,
            "replicas": 1,
            "resources": { "cpu_milli": 100, "memory_bytes": 67_108_864_u64 },
            "vm": {
                "command": "/bin/true",
                "args": [],
                "kernel": "/conformance/kernel",
                "rootfs": "/conformance/rootfs.ext4"
            },
            "listeners": [{ "port": 18951, "protocol": "tcp" }],
            "startup_probes": [],
            "readiness_probes": [],
            "liveness_probes": []
        }
    })
}

fn events_named<'a>(events: &'a [TraceEvent], name: &str) -> Vec<&'a TraceEvent> {
    events
        .iter()
        .filter(|event| {
            event.name == name || event.fields.get("name").map(String::as_str) == Some(name)
        })
        .collect()
}

fn field<'a>(event: &'a TraceEvent, name: &str) -> Option<&'a str> {
    event.fields.get(name).map(String::as_str)
}

fn host_residue_baseline() -> CleanupObservation {
    let directories: Vec<&Path> = HOST_RESIDUE_DIRECTORIES.iter().map(Path::new).collect();
    CleanupObservation::capture(&directories)
}

/// Then (shared by both fail-stop causes): the failed handler shuts down and
/// refuses its API; a fresh handler over the same roots completed its startup
/// checks before its API became reachable, and only then admits a VM workload
/// over HTTPS.
async fn a_fresh_handler_admits_work_only_after_its_boot(
    harness: &DirectHandlerHarness,
    trace: &TraceHistory,
    failed: DirectHandlerInstance,
    workload: &str,
) {
    let failed_api = failed.api().clone();
    failed.shutdown(Duration::from_secs(10)).await;
    assert!(
        failed_api.get("/v1/cluster/info").await.is_err(),
        "the failed handler's API admits nothing after its shutdown"
    );
    harness.record("failed_handler_shut_down", "the failed handler's API refuses connections");

    let (replacement, boot) =
        harness.start_fresh(Arc::new(SimSharedGuestNetworkOwner::default()), trace).await;
    assert_eq!(
        boot.observation.admission,
        Admission::Open,
        "the fresh handler's admission opens at the end of its boot: {boot:?}"
    );
    assert!(
        !boot.observation.boot_closed
            && boot.observation.recovery.is_none()
            && boot.observation.request.is_none(),
        "the fresh handler starts healthy, with no inherited recovery or request: {boot:?}"
    );
    for phase in ["vm_reclamation", "stale_sweep"] {
        let at = |transition: &str| {
            boot.boot_phases.iter().position(|(p, t)| p == phase && t == transition)
        };
        assert!(
            matches!((at("started"), at("completed")), (Some(started), Some(completed)) if started < completed),
            "boot phase {phase} completed before the API became reachable: {boot:?}"
        );
    }
    let position = |operation: GuestNetworkOperation| {
        boot.owner_calls.iter().position(|call| *call == operation)
    };
    let probe = position(GuestNetworkOperation::StartupProbe)
        .unwrap_or_else(|| panic!("the fresh handler ran its startup probe: {boot:?}"));
    let sweep = position(GuestNetworkOperation::CleanupComplement)
        .unwrap_or_else(|| panic!("the fresh handler ran its stale sweep: {boot:?}"));
    assert!(probe < sweep, "the startup probe precedes the stale sweep: {boot:?}");
    assert!(
        boot.owner_calls[..sweep].iter().all(|call| {
            !matches!(call, GuestNetworkOperation::TapCreate | GuestNetworkOperation::TapSetUp)
        }),
        "no guest attachment is made before the stale sweep: {boot:?}"
    );

    replacement.api().submit_workload(workload, &service_request(workload)).await;
    replacement.wait_running(workload, Duration::from_secs(30)).await;
    replacement.api().stop_workload(workload).await;
    replacement.shutdown(Duration::from_secs(10)).await;
    harness.record("fresh_handler_admitted", &format!("workload={workload}"));
}

/// Then: exactly one typed request — further steps receive no second request
/// and admission stays FailStop.
async fn no_second_request_follows(handler: &mut DirectHandlerInstance) {
    let after = handler.step_until(AUDIT_PERIOD, |_| false).await;
    assert!(
        after.fail_stops().is_empty()
            && after.observations.iter().all(|observation| {
                observation.admission == Admission::FailStop && observation.recovery.is_none()
            }),
        "one fail-stop request per episode, and admission stays FailStop: {after:?}"
    );
}

fn assert_detected_then_closed_until_request(trajectory: &Trajectory) -> (Duration, Duration) {
    let detected = trajectory
        .first(|observation| observation.admission != Admission::Open)
        .unwrap_or_else(|| panic!("admission closes once the loss is detected: {trajectory:?}"))
        .at;
    assert!(
        detected <= AUDIT_PERIOD,
        "the loss is detected within one {AUDIT_PERIOD:?} audit period: {trajectory:?}"
    );
    let requested =
        trajectory.first(|observation| observation.request.is_some()).unwrap_or_else(|| {
            panic!("the handler returns one typed fail-stop request: {trajectory:?}")
        });
    assert!(
        requested.at.saturating_sub(detected) <= RECOVERY_DEADLINE,
        "the request arrives within the {RECOVERY_DEADLINE:?} recovery window after detection: {trajectory:?}"
    );
    assert_eq!(
        requested.admission,
        Admission::FailStop,
        "admission is FailStop when the request is received: {trajectory:?}"
    );
    assert!(
        !trajectory.reopened_after_closing(),
        "admission never reopens once the loss is detected: {trajectory:?}"
    );
    assert_eq!(trajectory.fail_stops().len(), 1, "exactly one request: {trajectory:?}");
    (detected, requested.at)
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-33 — A failed handler shuts down before a fresh handler admits work
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER step 10-03 (S-ND295-33)"]
#[allow(
    clippy::too_many_lines,
    reason = "one conformance body preserves the complete fail-stop, drain, fresh-handler, event, API, and cleanup narrative"
)]
async fn shared_owner_fail_stop_shuts_down_before_a_fresh_handler_reopens_admission() {
    let trace = TraceHistory::install_global();
    let cleanup = host_residue_baseline();
    let harness = DirectHandlerHarness::new();

    // Given one production handler with required ports admitting a VM workload
    // over HTTPS.
    let failing_owner = Arc::new(SimSharedGuestNetworkOwner::default());
    let mut first = harness.start(Arc::clone(&failing_owner)).await;
    first
        .api()
        .submit_workload(
            "shared-network-before-loss",
            &service_request("shared-network-before-loss"),
        )
        .await;
    first.wait_running("shared-network-before-loss", Duration::from_secs(30)).await;
    assert_eq!(first.observe().admission, Admission::Open, "the healthy handler admits");

    // When the shared bridge is lost and every repair through its owner is
    // refused, so ownership cannot be restored in the bounded window.
    harness.record(
        "shared_owner_fault",
        "standing Bridge audit failure and converge refusal armed after healthy API admission",
    );
    failing_owner.script_component_audit_failure(SharedGuestNetworkComponent::Bridge, true);
    failing_owner.script_converge_failure(true);
    let trajectory = first
        .step_until(AUDIT_PERIOD + RECOVERY_DEADLINE, |observation| observation.request.is_some())
        .await;

    // Then the handler returns one typed fail-stop request naming the deadline.
    let (detected, requested) = assert_detected_then_closed_until_request(&trajectory);
    assert!(
        trajectory
            .observations
            .iter()
            .filter(|observation| observation.at >= detected && observation.at < requested)
            .all(|observation| observation.admission == Admission::Closed
                && observation.recovery.is_some()),
        "admission stays closed and recovery is in progress until the request: {trajectory:?}"
    );
    let fail_stop = trajectory.fail_stops()[0].clone();
    assert_eq!(fail_stop.component, SharedGuestNetworkComponent::Bridge);
    assert_eq!(fail_stop.cause, SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded);
    assert!(
        (1..=RECOVERY_ATTEMPTS).contains(&fail_stop.attempts)
            && fail_stop.elapsed <= RECOVERY_DEADLINE
            && (fail_stop.attempts == RECOVERY_ATTEMPTS || fail_stop.elapsed >= RECOVERY_DEADLINE),
        "the request is sent at {RECOVERY_ATTEMPTS} attempts of {ATTEMPT_PERIOD:?} or at the \
         {RECOVERY_DEADLINE:?} deadline, not before: {fail_stop:?}"
    );
    let failure_events = trace.snapshot();
    let unhealthy = events_named(&failure_events, "guest_network.shared_owner_unhealthy");
    assert_eq!(unhealthy.len(), 1, "one unhealthy record owns the episode");
    assert_eq!(field(unhealthy[0], "component"), Some("Bridge"));
    assert_eq!(field(unhealthy[0], "cause"), Some("audit_mismatch"));
    let retries = events_named(&failure_events, "guest_network.shared_owner_retry");
    assert_eq!(
        retries.len(),
        usize::try_from(fail_stop.attempts).expect("attempt count fits usize"),
        "one retry record per completed attempt"
    );
    for (index, retry) in retries.iter().enumerate() {
        assert_eq!(field(retry, "component"), Some("Bridge"));
        assert_eq!(field(retry, "attempt"), Some((index + 1).to_string().as_str()));
    }
    let fail_stop_events = events_named(&failure_events, "guest_network.shared_owner_fail_stop");
    assert_eq!(fail_stop_events.len(), 1, "the supervisor records one fail-stop");
    assert_eq!(field(fail_stop_events[0], "cleanup"), Some("abandoned_to_shutdown"));
    for count in ABANDONED_COUNT_FIELDS {
        assert!(
            fail_stop_events[0].fields.contains_key(count),
            "the fail-stop record counts the owned {count} it abandons to shutdown"
        );
    }
    no_second_request_follows(&mut first).await;
    harness.record(
        "fail_stop_observed",
        &format!(
            "component={:?} attempts={} elapsed_ms={}",
            fail_stop.component,
            fail_stop.attempts,
            fail_stop.elapsed.as_millis()
        ),
    );

    // And the failed handler shuts down before a fresh handler over the same
    // roots admits work only after its boot.
    a_fresh_handler_admits_work_only_after_its_boot(
        &harness,
        &trace,
        first,
        "shared-network-after-recovery",
    )
    .await;
    assert_eq!(
        events_named(&trace.snapshot(), "guest_network.shared_owner_unhealthy").len(),
        1,
        "the fresh handler appends evidence without overwriting the original failure"
    );

    wait_until(Duration::from_secs(10), "handler cleanup complement", || cleanup.is_restored());
    harness.record("cleanup_complete", "public and typed host complements restored");
    let history = harness.diagnostic_history();
    assert!(history.iter().any(|entry| entry.contains("phase=shared_owner_fault")));
    assert!(history.iter().any(|entry| entry.contains("phase=fail_stop_observed")));
    assert!(history.iter().any(|entry| entry.contains("phase=fresh_handler_admitted")));
    assert!(history.iter().any(|entry| entry.contains("phase=cleanup_complete")));
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-33 — A failed handler shuts down before a fresh handler admits work
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER step 10-03 (S-ND295-33)"]
async fn undetermined_tap_quiescence_requests_one_typed_fail_stop_before_a_fresh_handler_reopens() {
    let trace = TraceHistory::install_global();
    let cleanup = host_residue_baseline();
    let harness = DirectHandlerHarness::new();

    // Given one production handler with required ports admitting a VM workload
    // over HTTPS.
    let failing_owner = Arc::new(SimSharedGuestNetworkOwner::default());
    let mut first = harness.start(Arc::clone(&failing_owner)).await;
    first
        .api()
        .submit_workload(
            "shared-network-before-undetermined",
            &service_request("shared-network-before-undetermined"),
        )
        .await;
    first.wait_running("shared-network-before-undetermined", Duration::from_secs(30)).await;
    assert_eq!(first.observe().admission, Admission::Open, "the healthy handler admits");

    // When a kernel-path loss is detected and the set of TAPs that failed to
    // go down cannot be determined.
    harness.record(
        "shared_owner_fault",
        "standing Bridge audit failure and failing TAP quiescence armed after healthy API admission",
    );
    failing_owner.script_component_audit_failure(SharedGuestNetworkComponent::Bridge, true);
    failing_owner.script_quiesce_outcome(SimQuiesceOutcome::Fail);
    let trajectory = first
        .step_until(AUDIT_PERIOD + RECOVERY_DEADLINE, |observation| observation.request.is_some())
        .await;

    // Then the handler returns one typed fail-stop request naming the
    // undetermined quiescence, before any recovery attempt.
    assert_detected_then_closed_until_request(&trajectory);
    let fail_stop = trajectory.fail_stops()[0].clone();
    assert_eq!(fail_stop.component, SharedGuestNetworkComponent::Bridge);
    assert_eq!(fail_stop.cause, SharedGuestNetworkFailStopCause::TapQuiescenceUndetermined);
    assert_eq!(
        fail_stop.attempts, 0,
        "quiescence precedes the first recovery attempt and ends recovery: {fail_stop:?}"
    );
    let failure_events = trace.snapshot();
    let unhealthy = events_named(&failure_events, "guest_network.shared_owner_unhealthy");
    assert_eq!(unhealthy.len(), 1, "one unhealthy record owns the episode");
    assert_eq!(field(unhealthy[0], "component"), Some("Bridge"));
    assert_eq!(field(unhealthy[0], "cause"), Some("audit_mismatch"));
    assert!(
        events_named(&failure_events, "guest_network.shared_owner_retry").is_empty(),
        "no recovery attempt follows an undetermined quiescence"
    );
    let fail_stop_events = events_named(&failure_events, "guest_network.shared_owner_fail_stop");
    assert_eq!(fail_stop_events.len(), 1, "the supervisor records one fail-stop");
    assert!(
        fail_stop_events[0].fields.contains_key("vm_kill"),
        "the fail-stop record carries the workloads-slice kill outcome"
    );
    no_second_request_follows(&mut first).await;
    harness.record(
        "fail_stop_observed",
        &format!("component={:?} cause={:?}", fail_stop.component, fail_stop.cause),
    );

    // And the failed handler shuts down before a fresh handler over the same
    // roots admits work only after its boot.
    a_fresh_handler_admits_work_only_after_its_boot(
        &harness,
        &trace,
        first,
        "shared-network-after-undetermined",
    )
    .await;

    wait_until(Duration::from_secs(10), "handler cleanup complement", || cleanup.is_restored());
    harness.record("cleanup_complete", "public and typed host complements restored");
    let history = harness.diagnostic_history();
    assert!(history.iter().any(|entry| entry.contains("phase=shared_owner_fault")));
    assert!(history.iter().any(|entry| entry.contains("phase=fail_stop_observed")));
    assert!(history.iter().any(|entry| entry.contains("phase=fresh_handler_admitted")));
    assert!(history.iter().any(|entry| entry.contains("phase=cleanup_complete")));
}
