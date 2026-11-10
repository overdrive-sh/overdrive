//! Feature `service-allocation-readiness-and-ending`, decision D7, obligation
//! S-27 (render half) — `render::workload_describe` for a Service whose
//! startup-stopped predecessor is followed by a `Running` successor.
//!
//! CONTRACT_SHAPE: pure-function (a `String` for a wire fixture; no state, no
//! I/O). Per `crates/overdrive-cli/CLAUDE.md` the render function is called
//! directly, never through a subprocess.

#![allow(clippy::expect_used)]
#![allow(
    clippy::doc_markdown,
    reason = "acceptance-test docs name bare API identifiers (StoppedBy, StartupProbe) in prose"
)]

use overdrive_cli::commands::workload::WorkloadDescribeOutput;
use overdrive_cli::render::workload_describe;
use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, ProbeResultRowJson, ProbeStatusJson,
    ResourcesBody,
};
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn row(
    alloc_id: &str,
    state: AllocStateWire,
    terminal: Option<TerminalCondition>,
) -> AllocStatusRowBody {
    AllocStatusRowBody {
        alloc_id: alloc_id.to_owned(),
        workload_id: "payments".to_owned(),
        node_id: "node-a".to_owned(),
        state,
        reason: None,
        resources: ResourcesBody { cpu_milli: 100, memory_bytes: 1024 },
        workload_addr: None,
        started_at: None,
        exit_code: None,
        last_transition: None,
        error: None,
        last_terminated: None,
        terminal,
        network_cleanup_pending: false,
        svid_issue_failures: 0,
        last_svid_issue_failure: None,
        restart_count: 0,
    }
}

fn startup_probe() -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: "127.0.0.1".to_owned(), port: 8080 },
        timeout_seconds: 1,
        interval_seconds: 1,
        max_attempts: 3,
        failure_threshold: None,
        success_threshold: None,
    }
}

fn describe_output(
    rows: Vec<AllocStatusRowBody>,
    probe_results: Vec<ProbeResultRowJson>,
) -> WorkloadDescribeOutput {
    WorkloadDescribeOutput {
        workload_id: "payments".to_owned(),
        spec_digest: DIGEST.to_owned(),
        allocations_total: rows.len(),
        empty_state_message: String::new(),
        snapshot: AllocStatusResponse {
            workload_id: Some("payments".to_owned()),
            spec_digest: Some(DIGEST.to_owned()),
            kind: Some(WorkloadKind::Service),
            replicas_desired: 1,
            replicas_running: 1,
            rows,
            probes: vec![startup_probe()],
            probe_results,
            ..Default::default()
        },
    }
}

/// S-27 — a startup-stopped predecessor renders `terminal: stopped (startup
/// probe failed)` beneath its own row, with its `Terminated` state and its
/// `Fail` startup probe line; the `Running` successor renders no such line.
#[test]
/// CONTRACT_SHAPE: pure-function.
fn describe_names_a_startup_stopped_predecessor_and_shows_the_running_successor() {
    let predecessor = row(
        "alloc-payments-0",
        AllocStateWire::Terminated,
        Some(TerminalCondition::Stopped { by: StoppedBy::StartupProbe }),
    );
    let successor = row("alloc-payments-1", AllocStateWire::Running, None);
    let fail = ProbeResultRowJson {
        alloc_id: "alloc-payments-0".to_owned(),
        probe_idx: 0,
        role: "startup".to_owned(),
        status: ProbeStatusJson::Fail { last_fail_reason: "connection refused".to_owned() },
        last_observed_at_unix_ms: 1_000,
        inferred: false,
    };

    let rendered = workload_describe(&describe_output(vec![predecessor, successor], vec![fail]));

    let pred_row = rendered
        .find("alloc-payments-0")
        .unwrap_or_else(|| panic!("the predecessor row; got:\n{rendered}"));
    let succ_row = rendered
        .find("alloc-payments-1")
        .unwrap_or_else(|| panic!("the successor row; got:\n{rendered}"));
    assert!(pred_row < succ_row, "predecessor first; got:\n{rendered}");
    let predecessor_block = &rendered[pred_row..succ_row];
    assert!(predecessor_block.contains("Terminated"), "its state; got:\n{rendered}");
    assert!(
        predecessor_block.contains("    terminal: stopped (startup probe failed)\n"),
        "the presence-guarded terminal line; got:\n{rendered}",
    );
    let successor_block = &rendered[succ_row..];
    assert!(successor_block.contains("Running"), "the successor is Running; got:\n{rendered}");
    assert!(
        rendered.contains("last=fail (connection refused)"),
        "the predecessor's Fail startup probe line; got:\n{rendered}",
    );
    assert_eq!(
        rendered.matches("terminal: stopped (startup probe failed)").count(),
        1,
        "only the startup-stopped row carries the line; got:\n{rendered}",
    );
}

/// S-27 unaffected control — a `Stable` row still renders `terminal: Stable`
/// and no stopped line; a Service with no terminal renders neither.
#[test]
fn describe_keeps_the_stable_terminal_line_and_adds_no_stopped_line_for_other_rows() {
    let stable = row(
        "alloc-payments-0",
        AllocStateWire::Running,
        Some(TerminalCondition::Stable {
            settled_in_ms: 700,
            witness: overdrive_core::transition_reason::ProbeWitness {
                probe_idx: 0,
                role: "startup".to_owned(),
                mechanic_summary: "tcp 127.0.0.1:8080".to_owned(),
                inferred: false,
            },
        }),
    );
    let rendered = workload_describe(&describe_output(vec![stable], vec![]));

    assert!(rendered.contains("    terminal: Stable\n"), "got:\n{rendered}");
    assert!(!rendered.contains("stopped (startup probe failed)"), "got:\n{rendered}");
}

/// S-27 / D7 — only `Stopped { StartupProbe }` earns the line: an operator stop and
/// a liveness stop render no `startup probe failed` text.
#[test]
fn describe_renders_no_startup_line_for_an_operator_or_liveness_stop() {
    for by in [StoppedBy::Operator, StoppedBy::LivenessProbe] {
        let stopped = row(
            "alloc-payments-0",
            AllocStateWire::Terminated,
            Some(TerminalCondition::Stopped { by }),
        );
        let rendered = workload_describe(&describe_output(vec![stopped], vec![]));
        assert!(
            !rendered.contains("startup probe failed"),
            "StoppedBy::{by:?} must not render the startup line; got:\n{rendered}",
        );
    }
}
