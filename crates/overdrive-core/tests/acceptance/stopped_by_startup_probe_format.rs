//! Feature `service-allocation-readiness-and-ending`, decision D7 item 1, obligation
//! S-25 (format half) — `StoppedBy::StartupProbe`, the fieldless initiator appended
//! after `LivenessProbe` (rkyv discriminant 6; serde and OpenAPI name
//! `startup_probe`).
//!
//! The serde and rkyv roundtrips cover every initiator, the new one included; the
//! text arm reads `stopped (startup probe)`; the platform-reclaim arm reads `false`.
//! The golden-bytes fixtures that pin the archived layout live in
//! `tests/schema_evolution/alloc_status_row.rs`; the OpenAPI listing is checked in
//! `overdrive-control-plane`'s acceptance suite.
//!
//! CONTRACT_SHAPE: pure-function (a value or a string for a value; no state, no I/O).

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(clippy::doc_markdown)]

use std::net::Ipv4Addr;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{
    StoppedBy, TerminalCondition, TransitionReason, is_platform_reclaimed,
};

/// Every initiator, in rkyv discriminant order, with its serde name.
const INITIATORS: [(StoppedBy, &str); 7] = [
    (StoppedBy::Operator, "operator"),
    (StoppedBy::Reconciler, "reconciler"),
    (StoppedBy::Process, "process"),
    (StoppedBy::SystemGc, "system_gc"),
    (StoppedBy::PlatformReclaimed, "platform_reclaimed"),
    (StoppedBy::LivenessProbe, "liveness_probe"),
    (StoppedBy::StartupProbe, "startup_probe"),
];

fn terminated_row(by: StoppedBy) -> AllocStatusRow {
    let node = NodeId::new("node-001").expect("node id");
    AllocStatusRow {
        alloc_id: AllocationId::new("alloc-web-0").expect("alloc id"),
        workload_id: WorkloadId::new("web").expect("workload id"),
        node_id: node.clone(),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter: 1, writer: node },
        reason: Some(TransitionReason::Stopped { by }),
        detail: None,
        terminal: Some(TerminalCondition::Stopped { by }),
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: Some(Ipv4Addr::new(10, 99, 0, 6)),
        last_terminated: None,
        restart_count: 0,
    }
}

/// S-25 — the serde form of every initiator is its snake-case name, the new one is
/// `startup_probe`, and each round-trips.
#[test]
fn every_initiator_round_trips_through_serde_under_its_snake_case_name() {
    for (by, name) in INITIATORS {
        let json = serde_json::to_string(&by).expect("serialise");
        assert_eq!(json, format!("\"{name}\""), "{by:?} serialises as {name}");
        let back: StoppedBy = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, by, "{by:?} round-trips");
    }
}

/// S-25 — the new initiator is fieldless and tail-appended: it archives to the same
/// width as the last pre-existing initiator and its discriminant byte follows it, so
/// every pre-existing archived row decodes unchanged.
#[test]
fn the_startup_probe_initiator_is_tail_appended_and_fieldless() {
    let archive = |by: StoppedBy| {
        rkyv::to_bytes::<rkyv::rancor::Error>(&by).expect("archive").as_ref().to_vec()
    };
    let liveness = archive(StoppedBy::LivenessProbe);
    let startup = archive(StoppedBy::StartupProbe);

    assert_eq!(liveness.len(), startup.len(), "fieldless: the same archived width");
    assert_eq!(startup[0], liveness[0] + 1, "rkyv discriminant 6 follows LivenessProbe's 5");
    assert_eq!(startup[0], 6, "StartupProbe is rkyv discriminant 6");
}

/// S-25 — the exhaustive-match arms: the human-readable text reads `stopped (startup
/// probe)` and a startup stop is not a platform reclamation (it consumes the restart
/// budget like a crash).
#[test]
fn the_startup_probe_initiator_has_its_text_and_is_not_a_platform_reclaim() {
    assert_eq!(
        TransitionReason::Stopped { by: StoppedBy::StartupProbe }.human_readable(),
        "stopped (startup probe)",
    );
    assert_eq!(
        TransitionReason::Stopped { by: StoppedBy::LivenessProbe }.human_readable(),
        "stopped (liveness probe)",
        "the neighbouring text is unchanged",
    );
    for (by, _) in INITIATORS {
        assert_eq!(
            is_platform_reclaimed(&terminated_row(by)),
            by == StoppedBy::PlatformReclaimed,
            "only PlatformReclaimed reads as a platform reclaim; {by:?}",
        );
    }
}
