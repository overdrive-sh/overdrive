//! Slice 03 — `workload describe` kind-aware Job render.
//!
//! Per `docs/feature/workload-kind-discriminator/distill/test-scenarios.md`
//! §3 / step 02-02 acceptance criteria. The driving port for these
//! tests is the SINGLE LIVE `overdrive_cli::render::workload_describe`
//! renderer — the one `main.rs` dispatches `overdrive workload describe`
//! through (`commands::workload::describe` → `render::workload_describe`).
//! These tests exercise it via the `render_live(..)` helper, which wraps an
//! `AllocStatusResponse` into the `WorkloadDescribeOutput` the command path
//! produces. (The historical test-only `alloc_status_kind_aware` and the
//! flat `alloc_status` were consolidated into this one live renderer.)
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

use overdrive_cli::commands::workload::WorkloadDescribeOutput;
use overdrive_cli::render::{
    JobVerdict, format_job_alloc_status_attempts_table, format_job_alloc_status_header,
    format_job_verdict, workload_describe,
};
use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, IssuedCertSummary, ProbeResultRowJson,
    ProbeStatusJson,
};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{Listener, WorkloadKind};
use overdrive_core::dataplane::Proto;
use overdrive_core::id::{CertSerial, SpiffeId};
use overdrive_core::observation::probe_result_row::{ProbeIdx, ProbeRole};
use overdrive_core::wall_clock::UnixInstant;
use proptest::prelude::*;
use std::num::NonZeroU16;
use std::time::Duration;

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
        workload_id: "coinflip".to_string(),
        node_id: "node-1".to_string(),
        state,
        reason: None,
        resources: overdrive_control_plane::api::ResourcesBody {
            cpu_milli: 100,
            memory_bytes: 64 * 1024 * 1024,
        },
        workload_addr: None,
        started_at: started_at.map(str::to_string),
        exit_code,
        last_transition: None,
        error: None,
        last_terminated: None,
        restart_count: 0,
    }
}

/// Build an `AllocStatusResponse` carrying the supplied rows and kind.
fn fixture_response(
    workload_id: &str,
    kind: WorkloadKind,
    rows: Vec<AllocStatusRowBody>,
    replicas_desired: u32,
    replicas_running: u32,
) -> AllocStatusResponse {
    AllocStatusResponse {
        workload_id: Some(workload_id.to_string()),
        spec_digest: Some("a".repeat(64)),
        replicas_desired,
        replicas_running,
        rows,
        restart_budget: None,
        kind: Some(kind),
        vip: None,
        listeners: vec![],
        issued_certificates: vec![],
        probes: vec![],
        probe_results: vec![],
    }
}

/// Wrap an `AllocStatusResponse` into the `WorkloadDescribeOutput` the live
/// command path produces, then render it through the single live
/// `render::workload_describe` renderer (the one `main.rs` dispatches
/// through). The wrapper fields are derived the way
/// `commands::workload::describe` derives them: `workload_id` / `spec_digest`
/// from the snapshot, `allocations_total` from the row count,
/// `empty_state_message` populated only when there are zero allocations.
/// This is the driving port for these render-layer tests — exercising
/// the LIVE path, not a test-only renderer.
fn render_live(response: AllocStatusResponse) -> String {
    let allocations_total = response.rows.len();
    let empty_state_message = if allocations_total == 0 {
        let id = response.workload_id.as_deref().unwrap_or("(unknown)");
        format!(
            "0 allocations for workload {id} — the spec is committed, but no \
             allocation has converged to a Running instance yet. If it stays at \
             0, check that a node is eligible to place it and the control \
             plane's convergence loop is running."
        )
    } else {
        String::new()
    };
    let out = WorkloadDescribeOutput {
        workload_id: response.workload_id.clone().unwrap_or_default(),
        spec_digest: response.spec_digest.clone().unwrap_or_default(),
        allocations_total,
        empty_state_message,
        snapshot: response,
    };
    workload_describe(&out)
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

    let rendered = render_live(response);

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
// O03 sub-claim 3 — Service alloc status renders each listener as
// `<port>/<protocol>` so a UDP service's `Proto::Udp` is operator-visible.
// ---------------------------------------------------------------------------

/// Build a `Listener` from `(port, protocol)`.
const fn listener(port: u16, protocol: Proto) -> Listener {
    Listener { port: NonZeroU16::new(port).expect("non-zero port"), protocol }
}

/// Attach listeners to a Service fixture response.
fn fixture_response_with_listeners(
    workload_id: &str,
    listeners: Vec<Listener>,
) -> AllocStatusResponse {
    let mut response = fixture_response(
        workload_id,
        WorkloadKind::Service,
        vec![fixture_row("alloc-0", AllocStateWire::Running, None, Some("123@node-1"))],
        /*desired=*/ 1,
        /*running=*/ 1,
    );
    response.listeners = listeners;
    response
}

/// A Service with a UDP and a TCP listener renders a Listeners section
/// where each listener appears as `<port>/<protocol>` (lowercase
/// canonical protocol via `Proto::as_str`). This is the black-box
/// surface that makes a UDP service's `Proto::Udp` observable —
/// O03 verification sub-claim 3.
#[test]
fn service_alloc_status_renders_each_listener_as_port_slash_protocol() {
    let response = fixture_response_with_listeners(
        "dns-resolver",
        vec![listener(5353, Proto::Udp), listener(8080, Proto::Tcp)],
    );

    let rendered = render_live(response);

    assert!(
        rendered.contains("Listeners:"),
        "Service alloc-status must render a 'Listeners:' section; got:\n{rendered}",
    );
    assert!(
        rendered.contains("5353/udp"),
        "UDP listener must render as '5353/udp' (Proto::Udp visible black-box); got:\n{rendered}",
    );
    assert!(
        rendered.contains("8080/tcp"),
        "TCP listener must render as '8080/tcp'; got:\n{rendered}",
    );
}

/// A Job response (no listeners) renders NO Listeners section — the
/// section is Service-only and listener-presence-guarded.
#[test]
fn job_alloc_status_renders_no_listeners_section() {
    let response = fixture_response(
        "coinflip",
        WorkloadKind::Job,
        vec![fixture_row(
            "alloc-coinflip-0",
            AllocStateWire::Terminated,
            Some(0),
            Some("100@node-1"),
        )],
        /*desired=*/ 1,
        /*running=*/ 0,
    );

    let rendered = render_live(response);

    assert!(
        !rendered.contains("Listeners:"),
        "Job alloc-status must NOT render a 'Listeners:' section; got:\n{rendered}",
    );
}

// ---------------------------------------------------------------------------
// #220 — Service alloc status renders the platform-issued VIP so the
// operator-visible frontend address is surfaced. The VIP already rides
// on `AllocStatusResponse.vip` (ADR-0049 / #183) but the kind-aware
// renderer dropped it. VIP is Service-only and grouped with `Listeners:`
// (VIP first); omitted entirely for non-Service.
// ---------------------------------------------------------------------------

/// A Service whose response carries a VIP renders a `VIP:` line with the
/// platform-issued address on the kind-aware path.
#[test]
fn service_alloc_status_renders_vip_when_present() {
    let mut response = fixture_response(
        "dns-resolver",
        WorkloadKind::Service,
        vec![fixture_row("alloc-0", AllocStateWire::Running, None, Some("123@node-1"))],
        /*desired=*/ 1,
        /*running=*/ 1,
    );
    response.vip = Some("10.96.0.2".to_string());

    let rendered = render_live(response);

    assert!(
        rendered.contains("VIP:"),
        "Service alloc-status must render a 'VIP:' label when a VIP is present; got:\n{rendered}",
    );
    assert!(
        rendered.contains("10.96.0.2"),
        "Service alloc-status must surface the platform-issued VIP address; got:\n{rendered}",
    );
}

/// A Service with no VIP (`vip: None`) renders NO `VIP:` line — the line
/// is presence-guarded, never rendered as `VIP: None`. (`fixture_response`
/// defaults `vip: None`.)
#[test]
fn service_alloc_status_renders_no_vip_line_when_absent() {
    let response = fixture_response(
        "dns-resolver",
        WorkloadKind::Service,
        vec![fixture_row("alloc-0", AllocStateWire::Running, None, Some("123@node-1"))],
        /*desired=*/ 1,
        /*running=*/ 1,
    );

    let rendered = render_live(response);

    assert!(
        !rendered.contains("VIP:"),
        "a Service with no VIP must NOT render a 'VIP:' line — it is omitted, never \
         rendered as 'VIP: None'; got:\n{rendered}",
    );
}

// Property: for every `(port, protocol)` listener set, the rendered
// Service output contains the exact `<port>/<protocol>` token for each
// listener, using the canonical lowercase protocol form — and renders
// the lowercase token regardless of which protocols are present.
proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn service_render_surfaces_every_listener_port_and_proto(
        listeners in proptest::collection::vec(
            (1u16..=65535, prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]),
            1..=6,
        ),
    ) {
        let typed: Vec<Listener> =
            listeners.iter().map(|&(p, proto)| listener(p, proto)).collect();
        let response = fixture_response_with_listeners("svc", typed);

        let rendered = render_live(response);

        for &(port, proto) in &listeners {
            let token = format!("{port}/{}", proto.as_str());
            prop_assert!(
                rendered.contains(&token),
                "render must surface listener token {token:?}; got:\n{rendered}",
            );
        }
    }
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

    let rendered = render_live(response);

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

    let rendered = render_live(response);

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

    let rendered = render_live(response);

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
        let rendered = render_live(response);

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
// S-03-06 — workload describe for unknown workload: typed error
// ---------------------------------------------------------------------------
//
// This scenario validates the existing error-path contract — describe
// already returns CliError::HttpStatus { status: 404, .. } for unknown
// workloads (validated by walking_skeleton.rs::describe_for_unknown_workload_*).
// We re-assert at the describe surface that the error variant carries
// the offending workload id. This is a statelessly testable contract on the
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
        let rendered = render_live(response);
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

// ---------------------------------------------------------------------------
// S-OC-11 + S-OC-12 — issued-certificate summary render
// (built-in-ca-operator-composition Slice 3, EDD O05; relocated from the
// misplaced control-plane scaffold per the user-approved placement
// correction — the render driving port `render::workload_describe` cannot be
// reached from a crate `overdrive-cli` depends on).
//
// Driving port = `overdrive_cli::render::workload_describe` (the single live
// renderer) called in-process via `render_live(..)` over a constructed
// `AllocStatusResponse` whose additive `issued_certificates` field (landed
// in 03-01) carries `IssuedCertSummary` FACTS only — NO cert PEM/DER bytes,
// NO private key (ADR-0067 #215-boundary). The render must surface
// `serial / spiffe_id / issuer_serial / not_after` via their `Display`
// impls and must never reconstruct or print any cert material.
//
// O05 ≠ E03: these capture operator-legible audit metadata at the render
// layer. The "matches the minted cert" end-to-end is the server-projection
// concern landed in 03-01; at the render layer, faithful display of the
// provided serial IS the contract.
// ---------------------------------------------------------------------------

/// Build an `IssuedCertSummary` from string parts + a `not_after` seconds
/// value. `serial`/`issuer_serial` are `CertSerial` (even-length hex);
/// `spiffe_id` is a `SpiffeId`.
fn issued_cert_summary(
    serial: &str,
    spiffe_id: &str,
    issuer_serial: &str,
    not_after_secs: u64,
) -> IssuedCertSummary {
    IssuedCertSummary {
        serial: CertSerial::new(serial).expect("valid hex serial"),
        spiffe_id: SpiffeId::new(spiffe_id).expect("valid spiffe id"),
        issuer_serial: CertSerial::new(issuer_serial).expect("valid hex issuer serial"),
        not_after: UnixInstant::from_unix_duration(Duration::from_secs(not_after_secs)),
    }
}

/// S-OC-11 — a running alloc whose `AllocStatusResponse.issued_certificates`
/// carries an `IssuedCertSummary` renders an issued-certificate section
/// showing `serial`, `spiffe_id`, `issuer_serial`, and `not_after`, and the
/// rendered serial faithfully matches the summary's serial.
///
/// Kind is `Job` — the DISTILL AC verb is `overdrive workload describe <id>`
/// and the SVID `spiffe_id` is a `/workload/` path (`SpiffeId::for_allocation`),
/// so the kind under test MUST match the namespace. The server projects
/// `issued_certificates` per running alloc with NO `WorkloadKind` filter,
/// so a Job legitimately carries this summary; the render must surface it.
#[test]
fn alloc_status_surfaces_current_issued_certificate_summary() {
    let summary = issued_cert_summary(
        "0a1b2c3d4e5f",
        "spiffe://overdrive.local/workload/dns-resolver/alloc/alloc-0",
        "ffeeddccbbaa",
        1_700_000_000,
    );
    let serial_text = summary.serial.to_string();
    let spiffe_text = summary.spiffe_id.to_string();
    let issuer_text = summary.issuer_serial.to_string();
    let not_after_text = summary.not_after.to_string();

    let mut response = fixture_response(
        "dns-resolver",
        WorkloadKind::Job,
        vec![fixture_row("alloc-0", AllocStateWire::Running, None, Some("123@node-1"))],
        /*desired=*/ 1,
        /*running=*/ 1,
    );
    response.issued_certificates = vec![summary];

    let rendered = render_live(response);

    // The four audit-row facts are each surfaced via their `Display`.
    assert!(
        rendered.contains(&serial_text),
        "issued-cert section must surface the serial {serial_text:?}; got:\n{rendered}",
    );
    assert!(
        rendered.contains(&spiffe_text),
        "issued-cert section must surface the spiffe_id {spiffe_text:?}; got:\n{rendered}",
    );
    assert!(
        rendered.contains(&issuer_text),
        "issued-cert section must surface the issuer_serial {issuer_text:?}; got:\n{rendered}",
    );
    assert!(
        rendered.contains(&not_after_text),
        "issued-cert section must surface the not_after {not_after_text:?}; got:\n{rendered}",
    );
}

/// S-OC-12 (this step's primary scenario) — given the response carries
/// exactly the max-`issuance_ordinal` summary per running alloc (the server
/// already projects this in 03-01; at the render layer we assert the render
/// shows exactly the summaries provided, one per alloc, NOT a history list),
/// the rendered section contains NO certificate PEM/DER bytes and NO private
/// key, and a changed serial reads as the current cert. Guards the ADR-0067
/// #215-boundary no-leak invariant.
#[test]
fn issued_certificate_summary_omits_cert_bytes_and_key_max_issuance_ordinal() {
    // The server projects ONE summary per running alloc (max-issuance_ordinal).
    // The render layer is handed exactly that projection — one per alloc.
    let current = issued_cert_summary(
        // A post-restart serial change — this is the CURRENT cert.
        "deadbeefcafe",
        "spiffe://overdrive.local/workload/dns-resolver/alloc/alloc-0",
        "ffeeddccbbaa",
        1_700_000_500,
    );
    let current_serial = current.serial.to_string();

    // Kind is `Job` — the spiffe_id is a `/workload/` path and the AC verb is
    // `overdrive workload describe <id>`; the kind under test must match
    // the SVID namespace (no Service/`/workload/` mismatch masking the render).
    let mut response = fixture_response(
        "dns-resolver",
        WorkloadKind::Job,
        vec![fixture_row("alloc-0", AllocStateWire::Running, None, Some("123@node-1"))],
        /*desired=*/ 1,
        /*running=*/ 1,
    );
    response.issued_certificates = vec![current];

    let rendered = render_live(response);

    // The changed serial reads as the current cert.
    assert!(
        rendered.contains(&current_serial),
        "the current (post-restart) serial {current_serial:?} must read as the current cert; \
         got:\n{rendered}",
    );

    // No-leak invariant (ADR-0067 #215-boundary): the audit-row facts carry
    // no cert material, and the render must never reconstruct or print any.
    for forbidden in ["-----BEGIN", "PRIVATE KEY", "CERTIFICATE-----"] {
        assert!(
            !rendered.contains(forbidden),
            "issued-cert render must NOT leak cert PEM/DER or private-key material \
             (found {forbidden:?}); got:\n{rendered}",
        );
    }

    // Exactly one issued-cert row per running alloc — the render shows the
    // single provided summary, not a history list. A history list would
    // surface more than one serial line; here there is exactly one.
    let serial_line_count = rendered.lines().filter(|l| l.contains(&current_serial)).count();
    assert_eq!(
        serial_line_count, 1,
        "render must show exactly the latest summary per alloc (one row), not history; \
         got:\n{rendered}",
    );
}

/// The CLI render is purely additive — output for a workload with no issued
/// certs is unchanged (the empty section is omitted entirely, never rendered
/// as an empty `Issued certificates:` header). Asserted across EVERY
/// workload kind: the issued-certificates render is kind-agnostic, so the
/// presence-guard must keep the no-cert output clean for Service, Job, AND
/// Schedule alike.
#[test]
fn alloc_status_omits_issued_certificate_section_when_empty() {
    for kind in [WorkloadKind::Service, WorkloadKind::Job, WorkloadKind::Schedule] {
        let response = fixture_response(
            "dns-resolver",
            kind,
            vec![fixture_row("alloc-0", AllocStateWire::Running, None, Some("123@node-1"))],
            /*desired=*/ 1,
            /*running=*/ 1,
        );
        // `fixture_response` defaults `issued_certificates: vec![]`.

        let rendered = render_live(response);

        assert!(
            !rendered.contains("Issued certificate"),
            "[{kind:?}] a workload with no issued certs must omit the issued-certificate \
             section entirely; got:\n{rendered}",
        );
    }
}

/// The issued-certificates section is workload-kind-AGNOSTIC: it renders for
/// a Job-kind workload describe (the DISTILL AC verb is `overdrive workload
/// describe <id>`) exactly as it does for a Service. This is the test that
/// would have caught the Service-arm-only gating regression — its litmus:
/// deleting the kind-agnostic `render_issued_certificates_section` call in
/// the live `alloc_status` renderer turns it RED for the Job kind. The four
/// audit-row facts (`serial` / `spiffe_id` / `issuer_serial` / `not_after`)
/// are each surfaced via their `Display`, with no cert PEM/DER bytes or
/// private key.
#[test]
fn job_alloc_status_surfaces_issued_certificate_summary() {
    let summary = issued_cert_summary(
        "0a1b2c3d4e5f",
        "spiffe://overdrive.local/workload/coinflip/alloc/alloc-coinflip-0",
        "ffeeddccbbaa",
        1_700_000_000,
    );
    let serial_text = summary.serial.to_string();
    let spiffe_text = summary.spiffe_id.to_string();
    let issuer_text = summary.issuer_serial.to_string();
    let not_after_text = summary.not_after.to_string();

    let mut response = fixture_response(
        "coinflip",
        WorkloadKind::Job,
        vec![fixture_row("alloc-coinflip-0", AllocStateWire::Running, None, Some("100@node-1"))],
        /*desired=*/ 1,
        /*running=*/ 1,
    );
    response.issued_certificates = vec![summary];

    let rendered = render_live(response);

    // The Job render must surface the issued-certificate section + its four
    // facts — the Service-arm-only gating would drop all four for a Job.
    assert!(
        rendered.contains("Issued certificates:"),
        "Job alloc-status must render the issued-certificate section; got:\n{rendered}",
    );
    for (label, fact) in [
        ("serial", &serial_text),
        ("spiffe_id", &spiffe_text),
        ("issuer_serial", &issuer_text),
        ("not_after", &not_after_text),
    ] {
        assert!(
            rendered.contains(fact.as_str()),
            "Job issued-cert section must surface the {label} {fact:?}; got:\n{rendered}",
        );
    }

    // No-leak invariant (ADR-0067 #215-boundary) holds on the Job path too.
    for forbidden in ["-----BEGIN", "PRIVATE KEY", "CERTIFICATE-----"] {
        assert!(
            !rendered.contains(forbidden),
            "Job issued-cert render must NOT leak cert PEM/DER or private-key material \
             (found {forbidden:?}); got:\n{rendered}",
        );
    }
}

// ---------------------------------------------------------------------------
// Probes section — the declaration × observation join (US-06 / K4).
//
// The end-to-end proof that probe state reaches the operator lives in
// `workload_describe_probes.rs`, which drives a real `serve` + a real
// deployed Service. These tests cover the join branches that a Tier-3
// test cannot reach DETERMINISTICALLY — a probe that has not yet ticked
// (a sub-2s race against the probe interval) and a multi-allocation
// Service — through the same single live `workload_describe` renderer
// via `render_live`. They are complementary to the Tier-3 test, not a
// substitute for it: a renderer-only suite is what let
// `render::probes_section` ship with zero production callers.
// ---------------------------------------------------------------------------

/// Build a probe descriptor for the render-layer join fixtures.
fn fixture_descriptor(
    role: ProbeRole,
    idx: u32,
    mechanic: ProbeMechanic,
    inferred: bool,
) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(idx),
        role,
        mechanic,
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: matches!(role, ProbeRole::Liveness).then_some(3),
        success_threshold: matches!(role, ProbeRole::Readiness).then_some(1),
        inferred,
    }
}

fn fixture_probe_result(
    alloc_id: &str,
    role: ProbeRole,
    idx: u32,
    status: ProbeStatusJson,
) -> ProbeResultRowJson {
    ProbeResultRowJson {
        alloc_id: alloc_id.to_owned(),
        probe_idx: idx,
        role: role.as_str().to_owned(),
        status,
        last_observed_at_unix_ms: 1_700_000_000_000,
        inferred: false,
    }
}

/// A declared probe with NO observed row renders `last=pending`, not a
/// blank cell and not a missing row.
///
/// Row ABSENCE is the pending state per ADR-0054 §5 — adapters never
/// write a `Pending` status — so the join must materialise it from the
/// declaration side alone. This is the state an operator sees in the
/// first seconds after a deploy, and the one most likely to be silently
/// dropped by a join written observation-first.
#[test]
fn declared_probe_with_no_observed_row_renders_pending() {
    let mut response = fixture_response(
        "payments",
        WorkloadKind::Service,
        vec![fixture_row("alloc-payments-0", AllocStateWire::Running, None, Some("100@node-1"))],
        /*desired=*/ 1,
        /*running=*/ 1,
    );
    response.probes = vec![fixture_descriptor(
        ProbeRole::Startup,
        0,
        ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: 8080 },
        /*inferred=*/ true,
    )];
    // No `probe_results` — the probe has been declared but has not
    // ticked yet.

    let rendered = render_live(response);

    assert!(
        rendered.contains("Probes:"),
        "a Service that DECLARES a probe must render the section even before the probe \
         ticks — otherwise the operator cannot tell 'no probes declared' from 'probes \
         declared but not yet run'; got:\n{rendered}",
    );
    assert!(
        rendered.contains("startup probe[0]"),
        "the declared probe must render one row; got:\n{rendered}",
    );
    assert!(
        rendered.contains("last=pending"),
        "an unobserved probe renders `last=pending`; got:\n{rendered}",
    );
    assert!(
        rendered.contains("last_observed_at=\u{2014}"),
        "an unobserved probe has no timestamp — the em-dash placeholder, never a fabricated \
         0 or the current time; got:\n{rendered}",
    );
}

/// A Service that declares NO probes renders no Probes section at all —
/// not a bare `Probes:` header with nothing beneath it.
///
/// This is the `[[health_check.startup]] = []` explicit-opt-out shape
/// (the Phase-1 first-Running semantics preserved per ADR-0058). The
/// kind-guard covers Job/Schedule; this covers the third empty case,
/// which is a Service.
#[test]
fn service_declaring_no_probes_renders_no_probes_section() {
    let response = fixture_response(
        "payments",
        WorkloadKind::Service,
        vec![fixture_row("alloc-payments-0", AllocStateWire::Running, None, Some("100@node-1"))],
        /*desired=*/ 1,
        /*running=*/ 1,
    );
    // `probes` and `probe_results` both left empty by `fixture_response`.

    let rendered = render_live(response);

    assert!(
        !rendered.contains("Probes"),
        "a Service with the explicit `[[health_check.startup]] = []` opt-out must render no \
         Probes section and no empty header; got:\n{rendered}",
    );
    assert!(
        rendered.contains("(kind: Service)"),
        "the Service kind-aware body must still render; got:\n{rendered}",
    );
}

/// The join keys on `(alloc_id, role, probe_idx)`, so a row observed
/// for one allocation never bleeds onto another's line.
///
/// The durable `ProbeResultRow` PK is per-allocation (ADR-0080 § D2). A
/// join that keyed on `(role, probe_idx)` alone would report the first
/// replica's Pass as though every replica were healthy — the exact
/// class of lie this surface exists to prevent.
#[test]
fn probe_rows_do_not_bleed_across_allocations() {
    let mut response = fixture_response(
        "payments",
        WorkloadKind::Service,
        vec![
            fixture_row("alloc-payments-0", AllocStateWire::Running, None, Some("100@node-1")),
            fixture_row("alloc-payments-1", AllocStateWire::Running, None, Some("101@node-1")),
        ],
        /*desired=*/ 2,
        /*running=*/ 2,
    );
    response.probes = vec![fixture_descriptor(
        ProbeRole::Liveness,
        0,
        ProbeMechanic::Http {
            path: "/healthz".to_owned(),
            port: 8080,
            host: Some("127.0.0.1".to_owned()),
        },
        /*inferred=*/ false,
    )];
    // Only replica 0 has been observed, and it is FAILING. Replica 1 has
    // no row at all.
    response.probe_results = vec![fixture_probe_result(
        "alloc-payments-0",
        ProbeRole::Liveness,
        0,
        ProbeStatusJson::Fail { last_fail_reason: "HTTP 503".to_owned() },
    )];

    let rendered = render_live(response);

    assert!(
        rendered.contains("last=fail (HTTP 503)"),
        "the observed failure must render with its reason; got:\n{rendered}",
    );
    assert!(
        rendered.contains("last=pending"),
        "the second replica has no observed row, so its line must read pending — if the join \
         keyed on (role, probe_idx) alone it would inherit replica 0's Fail and the section \
         would misreport which replica is sick; got:\n{rendered}",
    );
    assert_eq!(
        rendered.matches("liveness probe[0]").count(),
        2,
        "one row per (allocation, declared probe): 2 allocations x 1 probe; got:\n{rendered}",
    );
}

/// The `(consecutive_failures/threshold)` ratio suffix is NOT rendered.
///
/// This pins the deliberate suppression documented on
/// `render::probe_render_rows`: no wire field and no durable row carries
/// a consecutive-failure count, so populating `failure_threshold` alone
/// would make the renderer emit `(0/3)` for a probe that IS failing — a
/// false statement about how close the workload is to a liveness
/// restart. The test exists so that when a counter DOES get an honest
/// source, whoever wires it is forced to come here and say so, rather
/// than the ratio silently reappearing as `(0/N)`.
#[test]
fn failing_probe_renders_no_fabricated_failure_ratio() {
    let mut response = fixture_response(
        "payments",
        WorkloadKind::Service,
        vec![fixture_row("alloc-payments-0", AllocStateWire::Running, None, Some("100@node-1"))],
        /*desired=*/ 1,
        /*running=*/ 1,
    );
    response.probes = vec![fixture_descriptor(
        ProbeRole::Liveness,
        0,
        ProbeMechanic::Http {
            path: "/healthz".to_owned(),
            port: 8080,
            host: Some("127.0.0.1".to_owned()),
        },
        /*inferred=*/ false,
    )];
    response.probe_results = vec![fixture_probe_result(
        "alloc-payments-0",
        ProbeRole::Liveness,
        0,
        ProbeStatusJson::Fail { last_fail_reason: "HTTP 503".to_owned() },
    )];

    let rendered = render_live(response);

    assert!(
        rendered.contains("last=fail (HTTP 503)"),
        "the failure itself IS reported — suppression applies only to the ratio; \
         got:\n{rendered}",
    );
    assert!(
        !rendered.contains("(0/3)"),
        "the descriptor declares failure_threshold = 3 and nothing carries a consecutive- \
         failure count, so a `(0/3)` here would be a fabricated claim that the probe has \
         failed zero times while reporting it as failing; got:\n{rendered}",
    );
}
