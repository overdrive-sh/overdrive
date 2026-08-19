//! Acceptance tests for `overdrive_cli::render::workload_describe` — the
//! SINGLE LIVE workload-describe renderer.
//!
//! `main.rs` dispatches `overdrive workload describe` through
//! `commands::workload::describe(..)` (returning a `WorkloadDescribeOutput`) →
//! `render::workload_describe(&out)`. This is the only renderer an operator
//! sees; after the workload-kind-discriminator consolidation it carries
//! the kind-aware body (Service replicas table / Job Verdict + per-attempt
//! Exit + stderr tail / Schedule cron) plus the shared VIP / Listeners /
//! Issued-certificates sections and the empty-state onboarding signpost.
//! There is no second/duplicate renderer to test — these tests ARE the
//! authoritative operator-visible-output coverage.
//!
//! Rendering is a pure string-builder — no I/O, no server dependency —
//! so it belongs in the default acceptance lane rather than the
//! `integration-tests`-gated slow lane. This is also the load-bearing
//! place the empty-state diagnostic must appear on an empty
//! allocation-status read per DWD-05 §6.2 / §6.7.
//!
//! Acceptance coverage:
//!   (d) empty-state rendering diagnoses the unconverged workload
//!       (spec committed, no allocation converged to Running yet).
//!   (e) non-empty Service rendering shows the kind-aware header +
//!       `Replicas (desired/running)` + `Spec digest` (per ADR-0020 the
//!       `commit_index` field is dropped — the digest is the per-write
//!       witness).
//!   (j) Job kind-aware view (Verdict + per-attempt Exit + stderr tail).
//!   (g/g2/h/i) Listeners, VIP, Failed-cause, issued-certificates.

use overdrive_cli::commands::workload::WorkloadDescribeOutput;
use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, IssuedCertSummary, ResourcesBody,
};
use overdrive_core::aggregate::{Listener, WorkloadKind};
use overdrive_core::dataplane::Proto;
use overdrive_core::id::{CertSerial, SpiffeId};
use overdrive_core::wall_clock::UnixInstant;
use std::num::NonZeroU16;
use std::time::Duration;

const EMPTY_STATE_DIGEST: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
const NONEMPTY_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// An empty-state read: zero allocations. The snapshot carries the
/// server-populated `workload_id` / `spec_digest` / `kind` (Service) the
/// way the live command path does; the wrapper carries the empty-state
/// onboarding message gated on `allocations_total == 0`.
fn fixture_empty_state() -> WorkloadDescribeOutput {
    let snapshot = AllocStatusResponse {
        workload_id: Some("payments".to_string()),
        spec_digest: Some(EMPTY_STATE_DIGEST.to_string()),
        kind: Some(WorkloadKind::Service),
        ..Default::default()
    };
    WorkloadDescribeOutput {
        workload_id: "payments".to_string(),
        spec_digest: EMPTY_STATE_DIGEST.to_string(),
        allocations_total: 0,
        empty_state_message: "0 allocations for workload payments — the spec is committed, but \
             no allocation has converged to a Running instance yet. If it stays at 0, check that \
             a node is eligible to place it and the control plane's convergence loop is running."
            .to_string(),
        snapshot,
    }
}

/// A non-empty Service read: 3 running replicas. Snapshot fields are
/// populated as the live server path populates them.
fn fixture_with_allocations() -> WorkloadDescribeOutput {
    let rows: Vec<AllocStatusRowBody> = (0..3)
        .map(|i| {
            row_with_state(&format!("alloc-payments-{i}"), AllocStateWire::Running, None, None)
        })
        .collect();
    let snapshot = AllocStatusResponse {
        workload_id: Some("payments".to_string()),
        spec_digest: Some(NONEMPTY_DIGEST.to_string()),
        kind: Some(WorkloadKind::Service),
        replicas_desired: 3,
        replicas_running: 3,
        rows,
        ..Default::default()
    };
    WorkloadDescribeOutput {
        workload_id: "payments".to_string(),
        spec_digest: NONEMPTY_DIGEST.to_string(),
        allocations_total: 3,
        empty_state_message: String::new(),
        snapshot,
    }
}

// -------------------------------------------------------------------
// (d) empty-state rendering diagnoses the unconverged workload
// -------------------------------------------------------------------

#[test]
fn render_workload_describe_empty_state_diagnoses_unconverged_workload() {
    let out = fixture_empty_state();
    let rendered = overdrive_cli::render::workload_describe(&out);

    assert!(
        rendered.contains("0 allocations for workload"),
        "rendered workload-describe empty-state must diagnose the zero-allocation state as \
         '0 allocations for workload'; got:\n{rendered}",
    );
    assert!(
        rendered.contains("converged"),
        "rendered workload-describe empty-state must explain the workload has not yet \
         converged to a Running instance; got:\n{rendered}",
    );
    assert!(
        !rendered.contains("phase-1-first-workload"),
        "rendered workload-describe empty-state must NOT carry the stale \
         `phase-1-first-workload` forward-pointer (#219); got:\n{rendered}",
    );
    assert!(
        !rendered.contains("for job"),
        "rendered workload-describe empty-state must use workload-generic language, \
         not `for job` (#219); got:\n{rendered}",
    );
    assert!(
        rendered.contains("payments"),
        "rendered workload-describe must name the workload id; got:\n{rendered}",
    );
    assert!(
        rendered.contains(&out.spec_digest),
        "rendered workload-describe must carry the spec_digest; got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (e) non-empty rendering shows allocations_total + spec_digest
// -------------------------------------------------------------------

#[test]
fn render_workload_describe_with_allocations_shows_total_and_digest() {
    let out = fixture_with_allocations();
    let rendered = overdrive_cli::render::workload_describe(&out);

    assert!(
        rendered.contains("payments"),
        "rendered alloc-status must name the job id; got:\n{rendered}",
    );
    assert!(
        rendered.contains('3'),
        "rendered alloc-status must carry allocations_total value; got:\n{rendered}",
    );
    assert!(
        rendered.contains(&out.spec_digest),
        "rendered alloc-status must carry the spec_digest; got:\n{rendered}",
    );
    // On non-empty results we SHOULD NOT print the empty-state hint
    // (would confuse the operator).
    assert!(
        !rendered.contains("0 allocations for workload"),
        "rendered workload-describe with allocations must NOT print the empty-state hint; \
         got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (f) the empty-state hint is conditioned on BOTH (allocations_total
// == 0) AND (message non-empty) — crucially NOT on either alone. A
// mutation that flips `&&` → `||` would print the hint whenever
// allocations exist (false positive) or print an empty-line blank hint
// when the producer set no message (noise). This test pins both
// asymmetric branches of the `&&` gate.
// -------------------------------------------------------------------

#[test]
fn render_workload_describe_suppresses_hint_when_allocations_exist_even_with_message_populated() {
    // A defensive fixture where allocations_total > 0 AND an
    // empty_state_message happens to be populated (producer might
    // populate it unconditionally). The orig `&&` gate suppresses the
    // hint because `allocations_total == 0` is false; a mutation to
    // `||` would print it because the message is non-empty.
    let snapshot = AllocStatusResponse {
        workload_id: Some("payments".to_string()),
        spec_digest: Some("deadbeef".repeat(8)),
        kind: Some(WorkloadKind::Service),
        replicas_desired: 5,
        replicas_running: 5,
        rows: (0..5)
            .map(|i| {
                row_with_state(&format!("alloc-payments-{i}"), AllocStateWire::Running, None, None)
            })
            .collect(),
        ..Default::default()
    };
    let out = WorkloadDescribeOutput {
        workload_id: "payments".to_string(),
        spec_digest: "deadbeef".repeat(8),
        allocations_total: 5,
        empty_state_message: "0 allocations for workload payments — the spec is committed, but \
             no allocation has converged to a Running instance yet. If it stays at 0, check that \
             a node is eligible to place it and the control plane's convergence loop is running."
            .to_string(),
        snapshot,
    };
    let rendered = overdrive_cli::render::workload_describe(&out);

    assert!(
        !rendered.contains("0 allocations for workload"),
        "when allocations_total > 0 the empty-state hint MUST NOT appear, \
         even if the producer left an empty_state_message populated — the \
         `allocations_total == 0 && !msg.is_empty()` gate is asymmetric; \
         a mutation of `&&` → `||` would leak the hint. Got:\n{rendered}",
    );
    // The kind-aware Service body must still render (5 running replicas).
    assert!(
        rendered.contains("Replicas (desired/running): 5/5"),
        "the kind-aware Service body must render the replica count; got:\n{rendered}",
    );
}

#[test]
fn render_workload_describe_suppresses_hint_when_message_is_empty_even_with_zero_allocations() {
    // `allocations_total == 0 && msg.is_empty()` — the symmetric
    // asymmetric case. Orig: both checks gate → hint not printed.
    // Mutation `&&` → `||`: `0 == 0 || false` = true → writeln!(s,
    // "{}", "") emits a leading blank line BEFORE the kind-aware header.
    //
    // We pin the absence of that spurious leading blank line: under the
    // correct `&&` gate the first rendered line is the kind-aware
    // header, never an empty line.
    let snapshot = AllocStatusResponse {
        workload_id: Some("payments".to_string()),
        spec_digest: Some("cafebabe".repeat(8)),
        kind: Some(WorkloadKind::Service),
        ..Default::default()
    };
    let out = WorkloadDescribeOutput {
        workload_id: "payments".to_string(),
        spec_digest: "cafebabe".repeat(8),
        allocations_total: 0,
        empty_state_message: String::new(),
        snapshot,
    };
    let rendered = overdrive_cli::render::workload_describe(&out);

    // Under the correct `&&` gate the empty-state line is suppressed and
    // the first line is the kind-aware header. A `&&`→`||` mutation would
    // fire `writeln!(s, "{}", "")` and prepend a blank line.
    let first_line = rendered.lines().next().unwrap_or("");
    assert_eq!(
        first_line, "Service 'payments' (kind: Service)",
        "with both predicates false (msg empty) the empty-state writeln must NOT \
         fire — a `&&`→`||` mutation would prepend a blank line before the \
         kind-aware header. got:\n{rendered}",
    );
    assert!(
        !rendered.contains("0 allocations for workload"),
        "with both predicates false (msg empty), the hint must not appear; \
         got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (g) Listener protocol rendering on the LIVE path.
//
// `main.rs` dispatches `overdrive workload describe` through the single
// live `render::workload_describe(&WorkloadDescribeOutput)` renderer. The listener
// protocol (`<port>/<proto>`) MUST render here so an operator deploying a
// UDP Service sees `5353/udp`. Listeners are an INTENT property,
// independent of allocations/convergence, so they render even at zero
// allocations
// (the O03 capture is pre-convergence: `allocations_total == 0`).
// -------------------------------------------------------------------

/// Build a `Listener` from `(port, protocol)`.
const fn listener(port: u16, protocol: Proto) -> Listener {
    Listener { port: NonZeroU16::new(port).expect("non-zero port"), protocol }
}

/// A pre-convergence (zero-allocation) UDP+TCP Service renders each
/// listener as `<port>/<protocol>` under a `Listeners:` header — on the
/// `render::workload_describe` path that the live command actually calls.
#[test]
fn render_workload_describe_renders_listener_protocol_at_zero_allocations() {
    let snapshot = AllocStatusResponse {
        listeners: vec![listener(5353, Proto::Udp), listener(8080, Proto::Tcp)],
        ..Default::default()
    };

    let out = WorkloadDescribeOutput {
        workload_id: "dns-resolver".to_string(),
        spec_digest: "d7b885".to_string() + &"0".repeat(58),
        allocations_total: 0,
        empty_state_message: "0 allocations for workload dns-resolver — the spec is committed, \
             but no allocation has converged to a Running instance yet. If it stays at 0, check \
             that a node is eligible to place it and the control plane's convergence loop is \
             running."
            .to_string(),
        snapshot,
    };

    let rendered = overdrive_cli::render::workload_describe(&out);

    assert!(
        rendered.contains("Listeners:"),
        "live alloc_status render must include a 'Listeners:' header for a Service with \
         declared listeners (even pre-convergence at 0 allocations); got:\n{rendered}",
    );
    assert!(
        rendered.contains("5353/udp"),
        "live alloc_status render must surface the UDP listener as '5353/udp' so Proto::Udp \
         is operator-visible; got:\n{rendered}",
    );
    assert!(
        rendered.contains("8080/tcp"),
        "live alloc_status render must surface the TCP listener as '8080/tcp'; got:\n{rendered}",
    );
}

/// A Job-shape output (empty `listeners`) renders NO `Listeners:`
/// section — the section is listener-presence-guarded, not kind-guarded.
#[test]
fn render_workload_describe_renders_no_listeners_section_when_empty() {
    let out = WorkloadDescribeOutput {
        workload_id: "coinflip".to_string(),
        spec_digest: "f".repeat(64),
        allocations_total: 1,
        empty_state_message: String::new(),
        // default snapshot carries an empty `listeners` vec.
        snapshot: AllocStatusResponse::default(),
    };

    let rendered = overdrive_cli::render::workload_describe(&out);

    assert!(
        !rendered.contains("Listeners:"),
        "a workload with no declared listeners must NOT render a 'Listeners:' section; \
         got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (g2) Service VIP rendering on the LIVE path (#220).
//
// `AllocStatusResponse.vip` already carries the platform-issued Service
// VIP on the wire (ADR-0049 / #183) — populated for `WorkloadKind::Service`
// reads from the allocator memo, `None` for Job/Schedule. The live
// `render::workload_describe` path (the function `main.rs` actually calls)
// dropped it. An operator deploying a Service must see the VIP so the
// frontend address is visible; this is the operator-visibility half of
// #220 (NOT the alloc-status→describe-workload rename). VIP is a
// Service-only frontend property, grouped with `Listeners:` (VIP first),
// and omitted entirely (not rendered as `VIP: None`) for non-Service.
// -------------------------------------------------------------------

/// A Service whose `AllocStatusResponse` carries a VIP renders a `VIP:`
/// line with the platform-issued address on the live `render::workload_describe`
/// path so the operator sees the frontend address.
#[test]
fn render_workload_describe_renders_service_vip_when_present() {
    let snapshot = AllocStatusResponse {
        vip: Some("10.96.0.2".to_string()),
        listeners: vec![listener(5353, Proto::Udp)],
        ..Default::default()
    };

    let out = WorkloadDescribeOutput {
        workload_id: "dns-resolver".to_string(),
        spec_digest: "d7b885".to_string() + &"0".repeat(58),
        allocations_total: 1,
        empty_state_message: String::new(),
        snapshot,
    };

    let rendered = overdrive_cli::render::workload_describe(&out);

    assert!(
        rendered.contains("VIP:"),
        "live alloc_status render must include a 'VIP:' label for a Service with a \
         platform-issued VIP; got:\n{rendered}",
    );
    assert!(
        rendered.contains("10.96.0.2"),
        "live alloc_status render must surface the Service VIP address so the operator \
         sees the frontend; got:\n{rendered}",
    );
}

/// A workload with no VIP (`vip: None` — Job/Schedule) renders NO `VIP:`
/// line — the line is presence-guarded, never rendered as `VIP: None`.
#[test]
fn render_workload_describe_renders_no_vip_line_when_absent() {
    let out = WorkloadDescribeOutput {
        workload_id: "coinflip".to_string(),
        spec_digest: "f".repeat(64),
        allocations_total: 1,
        empty_state_message: String::new(),
        // default snapshot carries `vip: None`.
        snapshot: AllocStatusResponse::default(),
    };

    let rendered = overdrive_cli::render::workload_describe(&out);

    assert!(
        !rendered.contains("VIP:"),
        "a workload with no VIP (Job/Schedule) must NOT render a 'VIP:' line — \
         it is omitted, never rendered as 'VIP: None'; got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (h) Failed/terminal allocation surfaces state + error on the LIVE path.
//
// RCA finding S-A4 (root-cause-analysis-convergence-dataplane-gap.md):
// when a backend process fails to start (e.g. `bind(): Address already
// in use`), the allocation goes terminal/Failed but `overdrive alloc
// status` read as a healthy bare `Allocations: 1` with NO per-row state
// or error. An operator could not distinguish a healthy Running workload
// from one whose process died on startup. The live renderer
// (`render::workload_describe`, the function `main.rs` actually calls)
// MUST surface each allocation's state, and render a Failed allocation
// prominently with its captured failure detail.
// -------------------------------------------------------------------

/// Build a minimal `AllocStatusRowBody` for the given state, error, and
/// exit code. Other fields carry inert defaults — they are not the
/// subject of these assertions.
fn row_with_state(
    alloc_id: &str,
    state: AllocStateWire,
    error: Option<&str>,
    exit_code: Option<i32>,
) -> AllocStatusRowBody {
    AllocStatusRowBody {
        alloc_id: alloc_id.to_string(),
        workload_id: "dns-resolver".to_string(),
        node_id: "node-a".to_string(),
        state,
        reason: None,
        resources: ResourcesBody { cpu_milli: 100, memory_bytes: 1024 },
        started_at: None,
        exit_code,
        last_transition: None,
        error: error.map(str::to_owned),
        last_terminated: None,
        restart_count: 0,
    }
}

/// A Failed allocation whose backend crashed on `bind(): Address already
/// in use` must read as Failed WITH its captured error on the live path.
/// The bare `Allocations: 1` line is no longer the only signal.
#[test]
fn render_workload_describe_surfaces_failed_allocation_state_and_error() {
    let snapshot = AllocStatusResponse {
        rows: vec![row_with_state(
            "alloc-dns-resolver-0",
            AllocStateWire::Failed,
            Some("bind: Address already in use"),
            Some(1),
        )],
        ..Default::default()
    };

    let out = WorkloadDescribeOutput {
        workload_id: "dns-resolver".to_string(),
        spec_digest: "d7b885".to_string() + &"0".repeat(58),
        allocations_total: 1,
        empty_state_message: String::new(),
        snapshot,
    };

    let rendered = overdrive_cli::render::workload_describe(&out);

    assert!(
        rendered.contains("Failed"),
        "a Failed allocation must read as Failed on the live alloc_status path — \
         the bare 'Allocations: 1' line must not be the only signal; got:\n{rendered}",
    );
    assert!(
        rendered.contains("bind: Address already in use"),
        "the Failed allocation's captured error detail must be surfaced so the \
         operator sees the cause; got:\n{rendered}",
    );
    assert!(
        rendered.contains("alloc-dns-resolver-0"),
        "the failing allocation's id must be rendered so the operator can locate \
         it; got:\n{rendered}",
    );
}

/// A healthy Running allocation must NOT read as Failed — no false
/// failure signal on the live path.
#[test]
fn render_workload_describe_running_allocation_does_not_read_as_failed() {
    let snapshot = AllocStatusResponse {
        rows: vec![row_with_state("alloc-dns-resolver-0", AllocStateWire::Running, None, None)],
        ..Default::default()
    };

    let out = WorkloadDescribeOutput {
        workload_id: "dns-resolver".to_string(),
        spec_digest: "d7b885".to_string() + &"0".repeat(58),
        allocations_total: 1,
        empty_state_message: String::new(),
        snapshot,
    };

    let rendered = overdrive_cli::render::workload_describe(&out);

    assert!(
        rendered.contains("Running"),
        "a healthy Running allocation must surface its Running state; got:\n{rendered}",
    );
    assert!(
        !rendered.contains("Failed"),
        "a healthy Running allocation must NOT read as Failed — no false failure \
         signal; got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (i) Issued-certificate section on the LIVE path (built-in-ca #215,
// EDD O05 / S-OC-11 + S-OC-12, ADR-0067 #215-boundary).
//
// `main.rs` dispatches `overdrive workload describe` through the single
// live `render::workload_describe(&WorkloadDescribeOutput)` renderer. The 03-02
// issued-certificates section was originally wired only into the (now
// retired) test-only `alloc_status_kind_aware`, so the operator saw
// nothing until this consolidation. The section MUST render on the live
// path: it reads `out.snapshot.issued_certificates`
// (the `&WorkloadDescribeOutput` shape — fields live under `out.snapshot.*`),
// surfacing the four audit-row FACTS (serial / spiffe_id / issuer_serial
// / not_after) via `Display` and NEVER any cert PEM/DER bytes or private
// key (the audit row carries facts only). See `overdrive-cli/CLAUDE.md`
// § "Workload-describe rendering — `render::workload_describe` is the LIVE path".
// -------------------------------------------------------------------

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

/// A running alloc whose `AllocStatusResponse.issued_certificates` carries
/// an `IssuedCertSummary` renders the issued-certificate section on the
/// LIVE `render::workload_describe` path — surfacing the four audit-row facts
/// (serial / `spiffe_id` / `issuer_serial` / `not_after`) via `Display`, and
/// NEVER leaking cert PEM/DER bytes or private-key material (the S-OC-11 +
/// S-OC-12 contract on the path `main.rs` actually calls).
///
/// Kind is realistic: a running Job alloc with a `/workload/` SPIFFE id. The
/// server projects `issued_certificates` per running alloc with no
/// `WorkloadKind` filter, so a Job legitimately carries this summary.
#[test]
fn render_workload_describe_surfaces_issued_certificate_summary_on_live_path() {
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

    let snapshot = AllocStatusResponse {
        rows: vec![row_with_state("alloc-0", AllocStateWire::Running, None, None)],
        issued_certificates: vec![summary],
        ..Default::default()
    };

    let out = WorkloadDescribeOutput {
        workload_id: "dns-resolver".to_string(),
        spec_digest: "d7b885".to_string() + &"0".repeat(58),
        allocations_total: 1,
        empty_state_message: String::new(),
        snapshot,
    };

    let rendered = overdrive_cli::render::workload_describe(&out);

    // The four audit-row facts are each surfaced via their `Display` on the
    // LIVE path (these FAIL before the production wiring — the live
    // `alloc_status` does not render the section yet).
    assert!(
        rendered.contains(&serial_text),
        "live alloc_status render must surface the issued-cert serial {serial_text:?}; \
         got:\n{rendered}",
    );
    assert!(
        rendered.contains(&spiffe_text),
        "live alloc_status render must surface the issued-cert spiffe_id {spiffe_text:?}; \
         got:\n{rendered}",
    );
    assert!(
        rendered.contains(&issuer_text),
        "live alloc_status render must surface the issued-cert issuer_serial {issuer_text:?}; \
         got:\n{rendered}",
    );
    assert!(
        rendered.contains(&not_after_text),
        "live alloc_status render must surface the issued-cert not_after {not_after_text:?}; \
         got:\n{rendered}",
    );

    // No-leak invariant (ADR-0067 #215-boundary): the audit-row facts carry
    // no cert material, and the live render must never reconstruct or print
    // any cert PEM/DER bytes or private key.
    for forbidden in ["-----BEGIN", "PRIVATE KEY", "CERTIFICATE-----"] {
        assert!(
            !rendered.contains(forbidden),
            "live alloc_status render must NOT leak cert PEM/DER or private-key material \
             (found {forbidden:?}); got:\n{rendered}",
        );
    }
}

/// A workload with no issued certs renders NO `Issued certificates:`
/// header on the LIVE path — the section is presence-guarded and purely
/// additive, so the output is byte-identical to before the section
/// existed.
#[test]
fn render_workload_describe_omits_issued_certificate_section_when_empty_on_live_path() {
    let out = WorkloadDescribeOutput {
        workload_id: "coinflip".to_string(),
        spec_digest: "f".repeat(64),
        allocations_total: 1,
        empty_state_message: String::new(),
        // default snapshot carries an empty `issued_certificates` vec.
        snapshot: AllocStatusResponse::default(),
    };

    let rendered = overdrive_cli::render::workload_describe(&out);

    assert!(
        !rendered.contains("Issued certificates:"),
        "a workload with no issued certs must NOT render an 'Issued certificates:' \
         section on the live path; got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (j) Kind-aware body on the LIVE path — Job verdict/attempts/stderr and
// the Service replica table. This is the operator-visible change the
// workload-kind-discriminator feature designed (step 02-02) but never
// wired into the command; these tests prove it now renders on the path
// `main.rs` actually calls (`render::workload_describe`), not a test-only
// renderer. Per design [D4] / ADR-0047 §4 / distill §3 (S-03-01..04).
// -------------------------------------------------------------------

/// Build a `Job`-kind snapshot carrying the supplied attempt rows.
fn job_snapshot(workload: &str, rows: Vec<AllocStatusRowBody>) -> AllocStatusResponse {
    AllocStatusResponse {
        workload_id: Some(workload.to_string()),
        spec_digest: Some("a".repeat(64)),
        kind: Some(WorkloadKind::Job),
        replicas_desired: 1,
        replicas_running: 0,
        rows,
        ..Default::default()
    }
}

/// Wrap a snapshot into the `WorkloadDescribeOutput` the command path
/// produces (deriving `allocations_total` from the row count, the
/// empty-state message only when there are zero allocations).
fn wrap_live(snapshot: AllocStatusResponse) -> WorkloadDescribeOutput {
    let allocations_total = snapshot.rows.len();
    let workload_id = snapshot.workload_id.clone().unwrap_or_default();
    let empty_state_message = if allocations_total == 0 {
        format!(
            "0 allocations for workload {workload_id} — the spec is committed, but no \
             allocation has converged to a Running instance yet. If it stays at 0, check that \
             a node is eligible to place it and the control plane's convergence loop is running."
        )
    } else {
        String::new()
    };
    WorkloadDescribeOutput {
        spec_digest: snapshot.spec_digest.clone().unwrap_or_default(),
        workload_id,
        allocations_total,
        empty_state_message,
        snapshot,
    }
}

/// A Failed Job renders the kind-aware Job view on the LIVE path:
/// `kind: Job`, `Verdict: Failed (backoff exhausted)`, the per-attempt
/// table columns (`Attempt / State / Exit / Started / Duration`), every
/// Failed attempt's Exit code, the last attempt's NAMED cause in the
/// ratified `TransitionReason::human_readable()` vocabulary AND its
/// verbatim driver detail — and NEVER the Service `is running with` /
/// `Replicas` phrasing (S-03-05 anti-scenario).
///
/// The named-cause obligation is the Job-arm half of the operator
/// contract the Service arm has carried since RCA S-A4: a row that
/// carries a structured `reason` must render that reason, not only the
/// free-form driver text it happens to sit beside. Step 03-06 / DWD-24.
///
/// The header obligation is the honesty half. `AllocStatusRowBody.error`
/// is verbatim driver / OS detail — for a VM allocation it is a boot
/// diagnostic or a guest console tail, never process stderr — and
/// `STDERR_TAIL_LINES` is `ExecDriver`'s own retention constant. Labelling
/// this field `stderr (last N lines):` asserted two things that are not
/// true of it, so `workload describe` must not.
#[test]
fn render_workload_describe_renders_job_kind_aware_view_on_live_path() {
    let rows = vec![row_with_state("alloc-coinflip-0", AllocStateWire::Failed, None, Some(1)), {
        let mut r = row_with_state("alloc-coinflip-1", AllocStateWire::Failed, None, Some(1));
        r.reason = Some(overdrive_core::TransitionReason::ExecBinaryNotFound {
            path: "/usr/local/bin/coinflip".to_owned(),
        });
        r.error = Some("panic: dice roll said 6\nstack trace line 1\n".to_string());
        r
    }];
    let rendered =
        overdrive_cli::render::workload_describe(&wrap_live(job_snapshot("coinflip", rows)));

    assert!(rendered.contains("kind: Job"), "Job header must read 'kind: Job'; got:\n{rendered}");
    assert!(
        rendered.contains("Verdict: Failed (backoff exhausted)"),
        "Failed Job must show the backoff-exhausted verdict on the live path; got:\n{rendered}",
    );
    for col in ["Attempt", "State", "Exit", "Started", "Duration"] {
        assert!(
            rendered.contains(col),
            "Job per-attempt table must carry the '{col}' column; got:\n{rendered}",
        );
    }
    assert!(
        rendered.contains("panic: dice roll said 6"),
        "Failed Job must surface the last attempt's verbatim driver detail; got:\n{rendered}",
    );
    let named = overdrive_core::TransitionReason::ExecBinaryNotFound {
        path: "/usr/local/bin/coinflip".to_owned(),
    }
    .human_readable();
    assert!(
        !"panic: dice roll said 6\nstack trace line 1\n".contains(&named),
        "test integrity: the verbatim detail must not already contain the named cause {named:?}, \
         or the assertion below proves nothing",
    );
    assert!(
        rendered.contains(&named),
        "Failed Job must name the last attempt's structured cause in the same vocabulary the \
         Service arm uses ({named:?}); got:\n{rendered}",
    );
    assert!(
        !rendered.contains("stderr (last"),
        "verbatim driver detail must NOT be labelled as a stderr tail with ExecDriver's line \
         budget — the field is neither; got:\n{rendered}",
    );
    // S-03-05 anti-scenario: a Job must never render Service phrasing.
    assert!(
        !rendered.contains("is running with"),
        "Job render must NEVER contain 'is running with'; got:\n{rendered}",
    );
    assert!(
        !rendered.contains("Replicas"),
        "Job render must NEVER contain 'Replicas'; got:\n{rendered}",
    );
}

/// A Service renders the kind-aware Service view on the LIVE path:
/// `kind: Service`, `Replicas (desired/running): N/M`, the per-alloc
/// table (`Alloc / State / Restarts / Since`) and NO `Exit` column nor
/// `Verdict:` line (those are Job-only). Per S-03-01.
#[test]
fn render_workload_describe_renders_service_kind_aware_view_on_live_path() {
    let snapshot = AllocStatusResponse {
        workload_id: Some("payments".to_string()),
        spec_digest: Some("a".repeat(64)),
        kind: Some(WorkloadKind::Service),
        replicas_desired: 2,
        replicas_running: 1,
        rows: vec![row_with_state("alloc-payments-0", AllocStateWire::Running, None, None)],
        ..Default::default()
    };
    let rendered = overdrive_cli::render::workload_describe(&wrap_live(snapshot));

    assert!(
        rendered.contains("kind: Service"),
        "Service header must read 'kind: Service'; got:\n{rendered}",
    );
    assert!(
        rendered.contains("Replicas (desired/running): 2/1"),
        "Service must show the desired/running replica count; got:\n{rendered}",
    );
    assert!(
        rendered.contains("Restarts"),
        "Service per-alloc table must carry the 'Restarts' column; got:\n{rendered}",
    );
    assert!(
        !rendered.contains("Exit"),
        "Service render must NOT carry an 'Exit' column (Job-only); got:\n{rendered}",
    );
    assert!(
        !rendered.contains("Verdict:"),
        "Service render must NOT carry a 'Verdict:' line (Job-only); got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// T-E (ADR-0078 § D6) — the crash-observability operator surface.
//
// Two obligations, both on the LIVE renderer:
//   1. a row carrying `restart_count: 2` + a populated `last_terminated`
//      renders the `Restarts` cell `2` AND the `last terminated:` block;
//   2. a row with `last_terminated: None` renders byte-identically to
//      today — the additive/presence-guard proof.
// -------------------------------------------------------------------

/// A `LastTerminatedBody` describing a SIGKILL crash the allocation
/// survived — the shape `handlers.rs` projects from a real
/// `AllocStatusRow.last_terminated`.
fn crash_snapshot() -> overdrive_control_plane::api::LastTerminatedBody {
    overdrive_control_plane::api::LastTerminatedBody {
        state: AllocStateWire::Failed,
        reason: Some(overdrive_core::TransitionReason::WorkloadCrashedImmediately {
            exit_code: Some(137),
            signal: Some(9),
            stderr_tail: Some("Segmentation fault".to_owned()),
        }),
        detail: Some("killed by SIGKILL".to_owned()),
        terminal: None,
        stderr_tail: Some("Segmentation fault".to_owned()),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        terminated_at: "(c=2,w=local)".to_owned(),
    }
}

/// Wrap a single row into a `WorkloadDescribeOutput` of the given kind.
fn describe_output_for(kind: WorkloadKind, row: AllocStatusRowBody) -> WorkloadDescribeOutput {
    let snapshot = AllocStatusResponse {
        workload_id: Some("recovery".to_string()),
        spec_digest: Some(NONEMPTY_DIGEST.to_string()),
        kind: Some(kind),
        replicas_desired: 1,
        replicas_running: 1,
        rows: vec![row],
        ..Default::default()
    };
    WorkloadDescribeOutput {
        workload_id: "recovery".to_string(),
        spec_digest: NONEMPTY_DIGEST.to_string(),
        allocations_total: 1,
        empty_state_message: String::new(),
        snapshot,
    }
}

/// T-E (1) — Service arm. The `Restarts` column carries the real count
/// (it rendered a hard-coded `"0"` from Phase 1 until ADR-0078), and the
/// presence-guarded `last terminated:` block names the crash beneath the
/// table row.
///
/// Kills the mutant that reverts `row.restart_count` to the `"0"`
/// literal, and the one that drops the detail block entirely.
#[test]
fn render_workload_describe_service_renders_restart_count_and_last_terminated() {
    let mut row = row_with_state("alloc-recovery-0", AllocStateWire::Running, None, None);
    row.restart_count = 2;
    row.last_terminated = Some(crash_snapshot());

    let rendered =
        overdrive_cli::render::workload_describe(&describe_output_for(WorkloadKind::Service, row));

    assert!(
        rendered.contains("Restarts"),
        "the Service arm must keep its Restarts column; got:\n{rendered}",
    );
    let alloc_line = rendered
        .lines()
        .find(|l| l.starts_with("alloc-recovery-0"))
        .unwrap_or_else(|| panic!("the alloc table row must be present; got:\n{rendered}"));
    assert!(
        alloc_line.contains('2'),
        "the Restarts cell must carry the real count (2), not a hard-coded 0; got: {alloc_line:?}",
    );
    assert!(
        rendered.contains("last terminated: Failed at (c=2,w=local)"),
        "the crash must be named with its state and LWW coordinate; got:\n{rendered}",
    );
    assert!(
        rendered.contains("last terminated detail: killed by SIGKILL"),
        "the verbatim driver text must surface; got:\n{rendered}",
    );
    assert!(
        rendered.contains("last terminated stderr"),
        "the workload's dying words must surface; got:\n{rendered}",
    );
    assert!(
        rendered.contains("Segmentation fault"),
        "and the stderr lines themselves; got:\n{rendered}",
    );
    assert!(
        !rendered.contains("    restarts: "),
        "the Service arm already has a Restarts COLUMN — the detail block must not \
         duplicate it; got:\n{rendered}",
    );
}

/// T-E (2) — the additive/presence-guard proof. A row that has never
/// been terminal renders byte-identically to a run with the fields
/// absent, so healthy output is unchanged.
///
/// Asserted as a byte-equality between two renders that differ ONLY in
/// `last_terminated`, which is stronger than a `!contains` probe: it
/// catches a stray blank line or trailing space as well as a stray block.
#[test]
fn render_workload_describe_healthy_row_is_byte_identical_without_last_terminated() {
    let baseline = row_with_state("alloc-recovery-0", AllocStateWire::Running, None, None);
    assert_eq!(baseline.restart_count, 0, "the baseline fixture is a never-restarted alloc");
    assert_eq!(baseline.last_terminated, None, "and it has survived no terminal");

    let rendered_service = overdrive_cli::render::workload_describe(&describe_output_for(
        WorkloadKind::Service,
        baseline.clone(),
    ));
    let rendered_job =
        overdrive_cli::render::workload_describe(&describe_output_for(WorkloadKind::Job, baseline));

    for (arm, rendered) in [("Service", &rendered_service), ("Job", &rendered_job)] {
        assert!(
            !rendered.contains("last terminated"),
            "{arm} arm: a never-terminal row must emit no last-terminated block; got:\n{rendered}",
        );
        assert!(
            !rendered.contains("    restarts: "),
            "{arm} arm: a never-restarted row must emit no restarts detail line; got:\n{rendered}",
        );
    }
}

/// T-E (3) — Job arm. The per-attempt column set
/// (`Attempt / State / Exit / Started / Duration`) is UNCHANGED — it is
/// pinned by the KPI-K3 byte-equality assertions — so the restart count
/// surfaces on the detail line instead of as a sixth column.
#[test]
fn render_workload_describe_job_renders_restarts_in_the_detail_block_not_a_column() {
    let mut row = row_with_state("alloc-recovery-0", AllocStateWire::Running, None, None);
    row.restart_count = 2;
    row.last_terminated = Some(crash_snapshot());

    let rendered =
        overdrive_cli::render::workload_describe(&describe_output_for(WorkloadKind::Job, row));

    let header = rendered
        .lines()
        .find(|l| l.starts_with("Attempt"))
        .unwrap_or_else(|| panic!("the Job per-attempt header must be present; got:\n{rendered}"));
    assert!(
        !header.contains("Restarts"),
        "the Job per-attempt column set is pinned — no Restarts column; got: {header:?}",
    );
    assert!(
        rendered.contains("    restarts: 2"),
        "the Job arm surfaces the count on the detail line; got:\n{rendered}",
    );
    assert!(
        rendered.contains("last terminated: Failed at (c=2,w=local)"),
        "and the crash is named; got:\n{rendered}",
    );
}
