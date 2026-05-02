//! Slice 01 step 01-03 — journey TUI mockup renderer.
//!
//! S-AS-04 / S-AS-05 / S-AS-06 — pure rendering tests against a typed
//! `AllocStatusResponse`. Per `crates/overdrive-cli/CLAUDE.md` the
//! renderer is a pure string-builder; no subprocess, no HTTP.
//!
//! The renderer maps the snapshot envelope per ADR-0033 §4 amended
//! 2026-04-30 — cause-class `TransitionReason` payloads render via the
//! shared `human_readable()` shape, so operators see the same prose on
//! the snapshot and streaming surfaces.

#![allow(clippy::expect_used, clippy::expect_fun_call, clippy::unwrap_used)]

use overdrive_cli::render::alloc_snapshot;
use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, ResourcesBody, RestartBudget,
    TransitionRecord, TransitionSource,
};
use overdrive_core::TransitionReason;
use overdrive_core::traits::driver::DriverType;
use overdrive_core::transition_reason::ResourceEnvelope;

const SAMPLE_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn running_fixture() -> AllocStatusResponse {
    AllocStatusResponse {
        job_id: Some("payments-v2".to_string()),
        spec_digest: Some(SAMPLE_DIGEST.to_string()),
        replicas_desired: 1,
        replicas_running: 1,
        rows: vec![AllocStatusRowBody {
            alloc_id: "alloc-payments-v2-0".to_string(),
            job_id: "payments-v2".to_string(),
            node_id: "node-a".to_string(),
            state: AllocStateWire::Running,
            reason: None,
            resources: ResourcesBody { cpu_milli: 500, memory_bytes: 134_217_728 },
            started_at: Some("(c=2,w=node-a)".to_string()),
            exit_code: None,
            last_transition: Some(TransitionRecord {
                from: Some(AllocStateWire::Pending),
                to: AllocStateWire::Running,
                reason: TransitionReason::Started,
                source: TransitionSource::Driver(DriverType::Exec),
                at: "(c=2,w=node-a)".to_string(),
            }),
            error: Some("driver started (pid 12345)".to_string()),
        }],
        restart_budget: Some(RestartBudget { used: 0, max: 5, exhausted: false }),
    }
}

fn failed_fixture() -> AllocStatusResponse {
    AllocStatusResponse {
        job_id: Some("payments-v2".to_string()),
        spec_digest: Some(SAMPLE_DIGEST.to_string()),
        replicas_desired: 1,
        replicas_running: 0,
        rows: vec![AllocStatusRowBody {
            alloc_id: "alloc-payments-v2-0".to_string(),
            job_id: "payments-v2".to_string(),
            node_id: "node-a".to_string(),
            state: AllocStateWire::Failed,
            reason: None,
            resources: ResourcesBody { cpu_milli: 500, memory_bytes: 134_217_728 },
            started_at: None,
            exit_code: None,
            last_transition: Some(TransitionRecord {
                from: Some(AllocStateWire::Pending),
                to: AllocStateWire::Failed,
                reason: TransitionReason::ExecBinaryNotFound {
                    path: "/usr/local/bin/payments".to_string(),
                },
                source: TransitionSource::Driver(DriverType::Exec),
                at: "(c=4,w=node-a)".to_string(),
            }),
            error: Some("stat /usr/local/bin/payments: no such file or directory".to_string()),
        }],
        restart_budget: Some(RestartBudget { used: 5, max: 5, exhausted: true }),
    }
}

fn pending_no_capacity_fixture() -> AllocStatusResponse {
    AllocStatusResponse {
        job_id: Some("payments-v2".to_string()),
        spec_digest: Some(SAMPLE_DIGEST.to_string()),
        replicas_desired: 1,
        replicas_running: 0,
        rows: vec![AllocStatusRowBody {
            alloc_id: "alloc-payments-v2-0".to_string(),
            job_id: "payments-v2".to_string(),
            node_id: "node-a".to_string(),
            state: AllocStateWire::Pending,
            reason: Some(TransitionReason::NoCapacity {
                requested: ResourceEnvelope {
                    cpu_milli: 10000,
                    memory_bytes: 10 * 1024 * 1024 * 1024,
                },
                free: ResourceEnvelope {
                    cpu_milli: 4000,
                    memory_bytes: 3 * 1024 * 1024 * 1024 + 200 * 1024 * 1024,
                },
            }),
            resources: ResourcesBody { cpu_milli: 10000, memory_bytes: 10 * 1024 * 1024 * 1024 },
            started_at: None,
            exit_code: None,
            last_transition: None,
            error: Some("requested 10000mCPU/10 GiB / free 4000mCPU/3.2 GiB".to_string()),
        }],
        restart_budget: Some(RestartBudget { used: 0, max: 5, exhausted: false }),
    }
}

// ---------------------------------------------------------------------------
// S-AS-04 — Running TUI mockup match
// ---------------------------------------------------------------------------

#[test]
fn s_as_04_running_snapshot_renders_journey_tui_mockup() {
    let out = running_fixture();
    let rendered = alloc_snapshot(&out);

    assert!(rendered.contains("payments-v2"), "Running render must echo job_id; got:\n{rendered}");
    assert!(
        rendered.contains(SAMPLE_DIGEST),
        "Running render must echo spec_digest; got:\n{rendered}",
    );
    assert!(
        rendered.contains("Restart budget: 0 / 5 used"),
        "Running render must show restart budget line literally; got:\n{rendered}",
    );
    assert!(
        rendered.contains("Pending → Running"),
        "Running render must show last transition arrow; got:\n{rendered}",
    );
    assert!(
        rendered.contains("driver started"),
        "Running render must surface 'driver started' from TransitionReason::Started; \
         got:\n{rendered}",
    );
    assert!(
        rendered.contains("source: driver(exec)"),
        "Running render must show source: driver(exec) (DriverType::Exec); got:\n{rendered}",
    );
    assert!(
        !rendered.contains("(backoff exhausted)"),
        "Running render must NOT show '(backoff exhausted)' when used < max; \
         got:\n{rendered}",
    );
}

// ---------------------------------------------------------------------------
// S-AS-05 — Failed mockup with cause-class rendering + verbatim driver error
// ---------------------------------------------------------------------------

#[test]
fn s_as_05_failed_snapshot_renders_cause_class_and_verbatim_error() {
    let out = failed_fixture();
    let rendered = alloc_snapshot(&out);

    // Cause-class rendering — replaces "driver start failed" with cause-specific prose.
    assert!(
        rendered.contains("binary not found: /usr/local/bin/payments"),
        "Failed render must surface the cause-class TransitionReason::ExecBinaryNotFound \
         in human form; got:\n{rendered}",
    );
    // Verbatim driver error in the `error:` line — preserved as-is.
    assert!(
        rendered.contains("stat /usr/local/bin/payments: no such file or directory"),
        "Failed render must include the verbatim driver error string; got:\n{rendered}",
    );
    // Restart budget exhausted annotation.
    assert!(
        rendered.contains("Restart budget: 5 / 5 used (backoff exhausted)"),
        "Failed render must surface the (backoff exhausted) annotation; got:\n{rendered}",
    );
    assert!(
        rendered.contains("Pending → Failed"),
        "Failed render must surface the from→to arrow; got:\n{rendered}",
    );
    assert!(
        rendered.contains("source: driver(exec)"),
        "Failed render must show source: driver(exec); got:\n{rendered}",
    );
}

// ---------------------------------------------------------------------------
// S-AS-06 — Pending-no-capacity explicit reason
// ---------------------------------------------------------------------------

#[test]
fn s_as_06_pending_no_capacity_renders_explicit_reason_not_silent_zero() {
    let out = pending_no_capacity_fixture();
    let rendered = alloc_snapshot(&out);

    // Must NOT be the silent empty-state — Allocations: 0 would be a
    // dishonest empty state when the row IS present and Pending.
    assert!(
        !rendered.contains("Allocations: 0"),
        "Pending-no-capacity render must NOT use the legacy 'Allocations: 0' \
         silent empty state — the reason row carries the actionable diagnostic; \
         got:\n{rendered}",
    );

    // Must surface the no-capacity diagnostic.
    assert!(
        rendered.contains("no capacity"),
        "Pending-no-capacity render must include 'no capacity' from \
         TransitionReason::NoCapacity rendering; got:\n{rendered}",
    );

    // Verbatim error/detail line — requested vs free.
    assert!(
        rendered.contains("requested 10000mCPU/10 GiB / free 4000mCPU/3.2 GiB"),
        "Pending-no-capacity render must include the verbatim requested-vs-free \
         text on its own line; got:\n{rendered}",
    );
}
