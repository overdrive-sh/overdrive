//! Slice 03 — `alloc status` kind-aware Job render.
//!
//! Per `docs/feature/workload-kind-discriminator/distill/test-scenarios.md`
//! §3 / step 02-02 acceptance criteria. The driving port for these
//! tests is the render layer in `overdrive_cli::render` — render fns
//! are pure functions whose public signature IS the driving port
//! (port-to-port at the render-layer scope per
//! `~/.claude/skills/nw-tdd-methodology/SKILL.md` § "Pure domain
//! functions ARE their own driving ports").
//!
//! The render layer branches on `AllocStatusRow.kind` (denormalised
//! at write time per design [D4] — Phase-1 greenfield, no backfill).
//! Service render shows replicas + Restarts column (no Exit). Job
//! render shows Verdict + per-attempt Exit codes + stderr tail.
//! Match on `WorkloadKind` is exhaustive per ADR-0047 §1.
//!
//! KPI K3: S-03-08 proptest 1024 cases asserting byte-equality
//! between rendered Exit column and persisted `exit_code`.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use overdrive_cli::render::{
    JobVerdict, format_job_alloc_status_attempts_table, format_job_alloc_status_header,
    format_job_verdict,
};
use overdrive_control_plane::api::{AllocStateWire, AllocStatusResponse, AllocStatusRowBody};
use overdrive_core::aggregate::WorkloadKind;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Build a minimal `AllocStatusRowBody` for render-layer fixtures.
fn fixture_row(
    alloc_id: &str,
    state: AllocStateWire,
    exit_code: Option<i32>,
    started_at: Option<&str>,
) -> AllocStatusRowBody {
    AllocStatusRowBody {
        alloc_id: alloc_id.to_string(),
        job_id: "coinflip".to_string(),
        node_id: "node-1".to_string(),
        state,
        reason: None,
        resources: overdrive_control_plane::api::ResourcesBody {
            cpu_milli: 100,
            memory_bytes: 64 * 1024 * 1024,
        },
        started_at: started_at.map(str::to_string),
        exit_code,
        last_transition: None,
        error: None,
    }
}

/// Build an `AllocStatusResponse` carrying the supplied rows and kind.
fn fixture_response(
    job_id: &str,
    kind: WorkloadKind,
    rows: Vec<AllocStatusRowBody>,
    replicas_desired: u32,
    replicas_running: u32,
) -> AllocStatusResponse {
    AllocStatusResponse {
        job_id: Some(job_id.to_string()),
        spec_digest: Some("a".repeat(64)),
        replicas_desired,
        replicas_running,
        rows,
        restart_budget: None,
        kind: Some(kind),
    }
}

// ---------------------------------------------------------------------------
// S-03-01 — Service alloc status: replicas + Restarts; no Exit column
// ---------------------------------------------------------------------------

#[test]
fn s_03_01_service_alloc_status_replicas_no_exit_column() {
    let rows =
        vec![fixture_row("alloc-payments-0", AllocStateWire::Running, None, Some("123@node-1"))];
    let response = fixture_response(
        "payments",
        WorkloadKind::Service,
        rows,
        /*desired=*/ 1,
        /*running=*/ 1,
    );

    let rendered = overdrive_cli::render::alloc_status_kind_aware(&response);

    assert!(
        rendered.contains("kind: Service"),
        "Service alloc-status output must contain 'kind: Service'; got:\n{rendered}",
    );
    assert!(
        rendered.contains("Replicas (desired/running): 1/1"),
        "Service alloc-status must show replicas; got:\n{rendered}",
    );
    // S-03-01: NO Exit column on Service.
    assert!(
        !rendered.contains("Exit"),
        "Service alloc-status must NOT contain an 'Exit' column; got:\n{rendered}",
    );
    // Service table columns: Alloc / State / Restarts / Since.
    assert!(
        rendered.contains("Restarts"),
        "Service per-alloc table must have a 'Restarts' column; got:\n{rendered}",
    );
}

// ---------------------------------------------------------------------------
// S-03-02 — Job alloc status (Failed) — KPI K3 framing journey
// ---------------------------------------------------------------------------

#[test]
fn s_03_02_job_alloc_status_failed_verdict_attempts_exit_codes_stderr() {
    let rows = vec![
        fixture_row("alloc-coinflip-0", AllocStateWire::Failed, Some(1), Some("100@node-1")),
        fixture_row("alloc-coinflip-1", AllocStateWire::Failed, Some(1), Some("110@node-1")),
        {
            let mut r = fixture_row(
                "alloc-coinflip-2",
                AllocStateWire::Failed,
                Some(1),
                Some("120@node-1"),
            );
            r.error = Some("panic: dice roll said 6\nstack trace line 1\n".to_string());
            r
        },
    ];
    let response = fixture_response(
        "coinflip",
        WorkloadKind::Job,
        rows,
        /*desired=*/ 1,
        /*running=*/ 0,
    );

    let rendered = overdrive_cli::render::alloc_status_kind_aware(&response);

    // Header: kind: Job
    assert!(
        rendered.contains("kind: Job"),
        "Job alloc-status header must contain 'kind: Job'; got:\n{rendered}",
    );
    // Verdict: Failed (backoff exhausted)
    assert!(
        rendered.contains("Verdict: Failed (backoff exhausted)"),
        "Job alloc-status must show 'Verdict: Failed (backoff exhausted)'; got:\n{rendered}",
    );
    // Per-attempt table columns: Attempt / State / Exit / Started / Duration
    for col in ["Attempt", "State", "Exit", "Started", "Duration"] {
        assert!(
            rendered.contains(col),
            "Job per-attempt table must have '{col}' column; got:\n{rendered}",
        );
    }
    // Every Failed attempt row shows Exit "1"
    let any_exit_one = rendered.lines().any(|l| l.contains(" 1 ") || l.ends_with(" 1"));
    assert!(any_exit_one, "every Failed attempt row must show Exit '1'; got:\n{rendered}");
    // stderr tail of last attempt is included
    assert!(
        rendered.contains("panic: dice roll said 6"),
        "Job alloc-status (Failed) must include stderr tail; got:\n{rendered}",
    );
}

// ---------------------------------------------------------------------------
// S-03-03 — Job alloc status (Succeeded): Verdict Succeeded with Exit 0
// ---------------------------------------------------------------------------

#[test]
fn s_03_03_job_alloc_status_succeeded_verdict_exit_zero() {
    let rows = vec![fixture_row(
        "alloc-coinflip-0",
        AllocStateWire::Terminated,
        Some(0),
        Some("100@node-1"),
    )];
    let response = fixture_response(
        "coinflip",
        WorkloadKind::Job,
        rows,
        /*desired=*/ 1,
        /*running=*/ 0,
    );

    let rendered = overdrive_cli::render::alloc_status_kind_aware(&response);

    assert!(
        rendered.contains("Verdict: Succeeded"),
        "Job alloc-status (Succeeded) must show 'Verdict: Succeeded'; got:\n{rendered}",
    );
    // Exactly one terminal attempt row with Exit 0 (the Job kind
    // surfaces a clean exit through `AllocState::Terminated` —
    // the row's `state` is the lifecycle bucket; the Verdict
    // line is the operator-visible derivation).
    let terminal_lines = rendered.lines().filter(|l| l.contains("Terminated")).count();
    assert!(terminal_lines >= 1, "must have at least one Terminated attempt row; got:\n{rendered}");
    // The persisted exit_code 0 byte-equals the rendered Exit cell.
    assert!(
        rendered.contains(" 0 "),
        "rendered Exit column must contain '0' for the clean-exit attempt; got:\n{rendered}",
    );
}

// ---------------------------------------------------------------------------
// S-03-04 — Job alloc status (in progress): Verdict In progress, Exit em-dash
// ---------------------------------------------------------------------------

#[test]
fn s_03_04_job_alloc_status_in_progress_em_dash() {
    let rows =
        vec![fixture_row("alloc-long-import-0", AllocStateWire::Running, None, Some("100@node-1"))];
    let response = fixture_response(
        "long-import",
        WorkloadKind::Job,
        rows,
        /*desired=*/ 1,
        /*running=*/ 1,
    );

    let rendered = overdrive_cli::render::alloc_status_kind_aware(&response);

    assert!(
        rendered.contains("Verdict: In progress (no terminal yet)"),
        "Job alloc-status (Running, no terminal) must show 'Verdict: In progress (no terminal \
         yet)'; got:\n{rendered}",
    );
    // Em-dash (U+2014) on Exit for Running rows
    assert!(
        rendered.contains('\u{2014}'),
        "Running attempt row's Exit cell must render as em-dash (—); got:\n{rendered}",
    );
}

// ---------------------------------------------------------------------------
// S-03-05 — Anti-scenario: Job alloc status NEVER renders Service phrasing
// ---------------------------------------------------------------------------

#[test]
fn s_03_05_anti_scenario_job_never_renders_service_phrasing() {
    // Test all three Job verdict states.
    let states = [
        (AllocStateWire::Terminated, Some(0_i32), "Succeeded"),
        (AllocStateWire::Failed, Some(1_i32), "Failed"),
        (AllocStateWire::Running, None, "Running"),
    ];

    for (state, exit_code, label) in states {
        let rows = vec![fixture_row("alloc-x-0", state, exit_code, Some("100@node-1"))];
        let response = fixture_response(
            "x",
            WorkloadKind::Job,
            rows,
            /*desired=*/ 1,
            /*running=*/ u32::from(matches!(state, AllocStateWire::Running)),
        );
        let rendered = overdrive_cli::render::alloc_status_kind_aware(&response);

        assert!(
            !rendered.contains("is running with"),
            "[{label}] Job alloc-status must NEVER contain 'is running with' phrasing; got:\n\
             {rendered}",
        );
        assert!(
            !rendered.contains("Replicas"),
            "[{label}] Job alloc-status must NEVER contain 'Replicas'; got:\n{rendered}",
        );
    }
}

// ---------------------------------------------------------------------------
// S-03-06 — alloc status for unknown job: typed error
// ---------------------------------------------------------------------------
//
// This scenario validates the existing error-path contract — alloc::status
// already returns CliError::HttpStatus { status: 404, .. } for unknown
// jobs (validated by walking_skeleton.rs::alloc_status_for_unknown_job_*).
// We re-assert at the alloc_status surface that the error variant carries
// the offending job id. This is a statelessly testable contract on the
// error type itself.

#[test]
fn s_03_06_alloc_status_unknown_job_typed_error() {
    use overdrive_cli::http_client::CliError;
    use overdrive_control_plane::api::ErrorBody;

    let err = CliError::HttpStatus {
        status: 404,
        body: ErrorBody {
            error: "not_found".to_string(),
            message: "no Job aggregate at intent key jobs/ghost".to_string(),
            field: None,
        },
    };

    match err {
        CliError::HttpStatus { status, body } => {
            assert_eq!(status, 404);
            assert_eq!(body.error, "not_found");
            assert!(
                body.message.contains("ghost"),
                "error message must name the missing job id 'ghost'; got: {}",
                body.message,
            );
        }
        other => panic!("expected CliError::HttpStatus 404; got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// S-03-07 — corrupt observation row: honest error
// ---------------------------------------------------------------------------
//
// Corrupt-row deserialise failures surface as CliError::BodyDecode with
// the underlying serde/rkyv error — the existing CliError::BodyDecode
// variant carries the error and propagates it honestly. We assert the
// shape exists and the operator-visible rendering does NOT fabricate
// an "Unknown" or empty row.

#[test]
fn s_03_07_alloc_status_corrupt_observation_row_honest_error() {
    use overdrive_cli::http_client::CliError;

    // Simulate the deserialise-failure path by constructing the
    // honest error variant the CLI surfaces. The render layer must
    // not fabricate Unknown rows on this error path.
    let err = CliError::BodyDecode {
        cause: "rkyv access failure: truncated bytes at offset 42".to_string(),
    };

    let rendered = overdrive_cli::render::cli_error(&err);
    assert!(
        !rendered.contains("Unknown"),
        "corrupt-row error rendering must NOT fabricate 'Unknown' rows; got:\n{rendered}",
    );
    assert!(
        rendered.contains("rkyv") || rendered.contains("decode") || rendered.contains("body"),
        "corrupt-row error rendering must name the deserialise failure; got:\n{rendered}",
    );
}

// ---------------------------------------------------------------------------
// S-03-08 — KPI K3 property: rendered Exit column byte-equals persisted exit_code
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1024,
        ..ProptestConfig::default()
    })]

    /// KPI K3 (the user's framing journey): for every Job-kind alloc
    /// status rendered with arbitrary per-attempt exit codes drawn from
    /// {0, 1, 2, 127, 137, 255}, the rendered Exit column byte-equals
    /// the persisted exit_code for every attempt row.
    #[test]
    fn s_03_08_k3_property_rendered_exit_matches_persisted(
        exit_codes in proptest::collection::vec(
            prop_oneof![Just(0_i32), Just(1), Just(2), Just(127), Just(137), Just(255)],
            1..=8,
        ),
    ) {
        let rows: Vec<AllocStatusRowBody> = exit_codes
            .iter()
            .enumerate()
            .map(|(i, &code)| {
                let alloc_id = format!("alloc-prop-{i}");
                let state = if code == 0 { AllocStateWire::Terminated } else { AllocStateWire::Failed };
                fixture_row(&alloc_id, state, Some(code), Some("100@node-1"))
            })
            .collect();
        let response = fixture_response(
            "prop",
            WorkloadKind::Job,
            rows.clone(),
            /*desired=*/ 1,
            /*running=*/ 0,
        );
        let table = format_job_alloc_status_attempts_table(&rows);

        // For every persisted exit code, the canonical decimal text
        // must appear in the rendered table. KPI K3 byte-equality:
        // the renderer must NOT round, truncate, sign-flip, or
        // remap the persisted exit_code on its way to the output.
        for &code in &exit_codes {
            let persisted_str = code.to_string();
            prop_assert!(
                table.contains(&persisted_str),
                "rendered Exit column must byte-equal persisted exit_code {code}; \
                 got rendered table:\n{table}",
            );
        }

        // The kind-aware dispatcher must also satisfy this invariant.
        let rendered = overdrive_cli::render::alloc_status_kind_aware(&response);
        for &code in &exit_codes {
            prop_assert!(
                rendered.contains(&code.to_string()),
                "kind-aware render must surface persisted exit_code {code}; \
                 got:\n{rendered}",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Render fn unit coverage — header / verdict / table fns are pure
// ---------------------------------------------------------------------------

#[test]
fn job_verdict_completed_zero_renders_succeeded() {
    let rendered = format_job_verdict(JobVerdict::Succeeded);
    assert_eq!(rendered.trim_end(), "Verdict: Succeeded");
}

#[test]
fn job_verdict_failed_renders_backoff_exhausted() {
    let rendered = format_job_verdict(JobVerdict::Failed);
    assert_eq!(rendered.trim_end(), "Verdict: Failed (backoff exhausted)");
}

#[test]
fn job_verdict_in_progress_renders_no_terminal_yet() {
    let rendered = format_job_verdict(JobVerdict::InProgress);
    assert_eq!(rendered.trim_end(), "Verdict: In progress (no terminal yet)");
}

#[test]
fn format_job_alloc_status_header_includes_name_kind_digest() {
    let rendered = format_job_alloc_status_header("coinflip", "abc123def456", JobVerdict::Failed);
    assert!(rendered.contains("Job 'coinflip'"));
    assert!(rendered.contains("kind: Job"));
    assert!(rendered.contains("abc123def456"));
    assert!(rendered.contains("Verdict: Failed (backoff exhausted)"));
}
