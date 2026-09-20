//! Shared guest-network direct-handler/public-API recovery conformance.
//!
//! The production server handler is started directly, like the repository's
//! other server-composition tests. Workload/admission behavior is driven only
//! through HTTPS. No product CLI, subprocess SUT, trycmd fixture, or internal
//! application call authors workload state.

#![cfg(feature = "integration-tests")]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    reason = "conformance test assertions use exact CONTRACT_SHAPE markers and diagnostic expectations"
)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::guest_network::{
    ServeShutdownRequest, SharedGuestNetworkComponent, SharedGuestNetworkFailStopCause,
};
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_system_conformance::{
    CleanupObservation, DirectHandlerHarness, TraceEvent, TraceHistory, wait_until,
};
use serde_json::{Value, json};

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

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER step for retained shared-owner supervisor and direct-handler replacement"]
#[allow(
    clippy::too_many_lines,
    reason = "one conformance body preserves the complete fail-stop, drain, fresh-handler, event, API, and cleanup narrative"
)]
async fn shared_owner_fail_stop_shuts_down_before_a_fresh_handler_reopens_admission() {
    let trace = TraceHistory::install_global();
    let cleanup = CleanupObservation::capture(&[
        Path::new("/run/overdrive/vm"),
        Path::new("/sys/fs/cgroup/overdrive.slice/workloads.slice"),
    ]);
    let harness = DirectHandlerHarness::new();

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

    // Fault only the accepted owner-port outcome. The retained production
    // supervisor authors recovery transitions, retries, and the terminal
    // request; the fixture does not inject a resulting gate transition.
    harness.record("shared_owner_fault", "audit refusal armed after healthy API admission");
    failing_owner.script_audit_failure(true);
    let clock = Arc::clone(first.clock());
    let request_task = tokio::spawn(async move {
        let request = first.shutdown_requested().await;
        (first, request)
    });
    tokio::task::yield_now().await;
    // Advance the one injected monotonic clock across the one-second audit,
    // 250 ms attempts, and five-second deadline.
    for _ in 0..24 {
        clock.tick(Duration::from_millis(250));
        tokio::task::yield_now().await;
    }
    let (first, request) = tokio::time::timeout(Duration::from_secs(16), request_task)
        .await
        .expect("retained supervisor requests fail-stop inside the accepted outer bound")
        .expect("request waiter joins");
    let ServeShutdownRequest::SharedGuestNetwork(fail_stop) = request;
    assert_eq!(fail_stop.component, SharedGuestNetworkComponent::Bridge);
    assert_eq!(fail_stop.cause, SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded);
    assert!(fail_stop.attempts > 0);
    assert!(fail_stop.elapsed >= Duration::from_secs(5));
    let failure_events = trace.snapshot();
    assert_eq!(
        events_named(&failure_events, "guest_network.shared_owner_unhealthy").len(),
        1,
        "one unhealthy record owns the episode"
    );
    let retry_events = events_named(&failure_events, "guest_network.shared_owner_retry");
    assert_eq!(
        retry_events.len(),
        usize::try_from(fail_stop.attempts).expect("attempt count fits usize")
    );
    for retry in retry_events {
        assert_eq!(retry.fields.get("component").map(String::as_str), Some("Bridge"));
    }
    for (index, retry) in
        events_named(&failure_events, "guest_network.shared_owner_retry").into_iter().enumerate()
    {
        let attempt = u32::try_from(index + 1).expect("bounded retry index");
        assert_eq!(retry.fields.get("attempt"), Some(&attempt.to_string()));
        assert_eq!(
            retry.fields.get("elapsed"),
            Some(&format!("{:?}", Duration::from_millis(u64::from(attempt) * 250))),
            "retry cadence is the injected 250 ms schedule"
        );
    }
    let fail_stop_events = events_named(&failure_events, "guest_network.shared_owner_fail_stop");
    assert!(fail_stop_events.iter().any(|event| {
        event.fields.get("cleanup").map(String::as_str) == Some("abandoned_to_shutdown")
            && [
                "taps",
                "links",
                "pins",
                "maps",
                "managed_set_elements",
                "intercept_set_elements",
                "handles",
            ]
            .iter()
            .all(|field| event.fields.contains_key(*field))
    }));
    harness.record(
        "fail_stop_observed",
        &format!(
            "component={:?} attempts={} elapsed_ms={}",
            fail_stop.component,
            fail_stop.attempts,
            fail_stop.elapsed.as_millis()
        ),
    );

    let old_api = first.api().clone();
    first.shutdown(Duration::from_secs(10)).await;
    assert!(old_api.get("/v1/cluster/info").await.is_err());
    assert!(events_named(&trace.snapshot(), "guest_network.shared_owner_fail_stop").iter().any(
        |event| { event.fields.get("cleanup").map(String::as_str) == Some("drained_before_exit") }
    ));

    let replacement_owner = Arc::new(SimSharedGuestNetworkOwner::default());
    let replacement = harness.start(replacement_owner).await;
    replacement
        .api()
        .submit_workload(
            "shared-network-after-recovery",
            &service_request("shared-network-after-recovery"),
        )
        .await;
    replacement.wait_running("shared-network-after-recovery", Duration::from_secs(30)).await;
    replacement.api().stop_workload("shared-network-after-recovery").await;
    replacement.shutdown(Duration::from_secs(10)).await;

    let replacement_events = trace.snapshot();
    let boot_recovered =
        events_named(&replacement_events, "guest_network.shared_owner_boot_recovered");
    assert_eq!(boot_recovered.len(), 1);
    assert_eq!(boot_recovered[0].fields.get("complement_empty").map(String::as_str), Some("true"));
    for field in [
        "taps",
        "links",
        "pins",
        "maps",
        "managed_set_elements",
        "intercept_set_elements",
        "handles",
    ] {
        assert!(boot_recovered[0].fields.contains_key(field), "boot recovery records {field}");
    }
    assert_eq!(
        events_named(&replacement_events, "guest_network.shared_owner_unhealthy").len(),
        1,
        "replacement appends recovery evidence without overwriting the original failure"
    );

    wait_until(Duration::from_secs(10), "handler cleanup complement", || cleanup.is_restored());
    harness.record("cleanup_complete", "public and typed host complements restored");
    let history = harness.diagnostic_history();
    assert!(history.iter().any(|entry| entry.contains("phase=shared_owner_fault")));
    assert!(history.iter().any(|entry| entry.contains("phase=fail_stop_observed")));
    assert!(history.iter().any(|entry| entry.contains("phase=cleanup_complete")));
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER step for retained shared-owner supervisor forced-timeout diagnostics"]
async fn forced_shutdown_timeout_records_abandoned_at_exit_separately_from_graceful_drain() {
    let trace = TraceHistory::install_global();
    let harness = DirectHandlerHarness::new();
    let owner = Arc::new(SimSharedGuestNetworkOwner::default());
    let mut handler = harness.start(Arc::clone(&owner)).await;
    owner.script_audit_failure(true);
    let clock = Arc::clone(handler.clock());
    let request = tokio::spawn(async move {
        let request = handler.shutdown_requested().await;
        (handler, request)
    });
    tokio::task::yield_now().await;
    for _ in 0..24 {
        clock.tick(Duration::from_millis(250));
        tokio::task::yield_now().await;
    }
    let (handler, request) = tokio::time::timeout(Duration::from_secs(16), request)
        .await
        .expect("bounded fail-stop request")
        .expect("request waiter joins");
    assert!(matches!(request, ServeShutdownRequest::SharedGuestNetwork(_)));
    handler
        .shutdown_result(Duration::ZERO)
        .await
        .expect_err("zero outer bound forces the abandoned-at-exit branch");

    let events = trace.snapshot();
    assert!(events_named(&events, "guest_network.shared_owner_fail_stop").iter().any(|event| {
        event.fields.get("cleanup").map(String::as_str) == Some("abandoned_at_exit")
    }));
    assert!(!events_named(&events, "guest_network.shared_owner_fail_stop").iter().any(|event| {
        event.fields.get("cleanup").map(String::as_str) == Some("drained_before_exit")
    }));
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "pending DELIVER step for shared-owner quiesce failure cgroup-kill/fail-stop"]
async fn unconfirmed_tap_quiescence_stops_the_affected_vm_and_requests_fail_stop() {
    let _trace = TraceHistory::install_global();
    let harness = DirectHandlerHarness::new();
    let owner = Arc::new(SimSharedGuestNetworkOwner::default());
    let mut handler = harness.start(Arc::clone(&owner)).await;
    handler
        .api()
        .submit_workload(
            "shared-network-quiesce-refused",
            &service_request("shared-network-quiesce-refused"),
        )
        .await;
    handler.wait_running("shared-network-quiesce-refused", Duration::from_secs(30)).await;

    owner.script_audit_failure(true);
    owner.script_quiesce_failure(true);
    let clock = Arc::clone(handler.clock());
    let request = tokio::spawn(async move {
        let request = handler.shutdown_requested().await;
        (handler, request)
    });
    tokio::task::yield_now().await;
    for _ in 0..24 {
        clock.tick(Duration::from_millis(250));
        tokio::task::yield_now().await;
    }
    let (handler, request) = tokio::time::timeout(Duration::from_secs(16), request)
        .await
        .expect("quiesce refusal reaches bounded fail-stop")
        .expect("request waiter joins");
    let ServeShutdownRequest::SharedGuestNetwork(fail_stop) = request;
    assert_eq!(fail_stop.cause, SharedGuestNetworkFailStopCause::RecoveryDeadlineExceeded);
    assert!(
        owner
            .calls()
            .contains(&overdrive_control_plane::guest_network::GuestNetworkOperation::TapSetDown),
        "the production supervisor attempts the accepted TAP-quiesce owner operation"
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while handler.api().one_replica_running("shared-network-quiesce-refused").await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("existing per-VM stop/cgroup-kill path removes the affected Running owner");
    handler.shutdown_result(Duration::from_secs(10)).await.expect("failed owner drains at exit");
}
