//! Acceptance tests for `overdrive_cli::render::alloc_status` — step 05-05.
//!
//! Rendering is a pure string-builder — no I/O, no server dependency —
//! so it belongs in the default acceptance lane rather than the
//! `integration-tests`-gated slow lane. This is also the load-bearing
//! place the `phase-1-first-workload` reference must appear on an empty
//! allocation-status read per DWD-05 §6.2 / §6.7.
//!
//! Acceptance coverage:
//!   (d) empty-state rendering contains the `phase-1-first-workload`
//!       reference (walking-skeleton gate for the onboarding signpost).
//!   (e) non-empty rendering shows the `spec_digest` (per ADR-0020 the
//!       `commit_index` field is dropped — the digest is the per-write
//!       witness).

use overdrive_cli::commands::alloc::AllocStatusOutput;
use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, IssuedCertSummary, ResourcesBody,
};
use overdrive_core::aggregate::Listener;
use overdrive_core::dataplane::Proto;
use overdrive_core::id::{CertSerial, SpiffeId};
use overdrive_core::wall_clock::UnixInstant;
use std::num::NonZeroU16;
use std::time::Duration;

fn fixture_empty_state() -> AllocStatusOutput {
    AllocStatusOutput {
        workload_id: "payments".to_string(),
        spec_digest: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".to_string(),
        allocations_total: 0,
        empty_state_message: "0 allocations for job payments — the scheduler + driver land in \
             phase-1-first-workload"
            .to_string(),
        snapshot: AllocStatusResponse::default(),
    }
}

fn fixture_with_allocations() -> AllocStatusOutput {
    AllocStatusOutput {
        workload_id: "payments".to_string(),
        spec_digest: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        allocations_total: 3,
        empty_state_message: String::new(),
        snapshot: AllocStatusResponse::default(),
    }
}

// -------------------------------------------------------------------
// (d) empty-state rendering contains phase-1-first-workload
// -------------------------------------------------------------------

#[test]
fn render_alloc_status_empty_state_contains_phase_1_first_workload() {
    let out = fixture_empty_state();
    let rendered = overdrive_cli::render::alloc_status(&out);

    assert!(
        rendered.contains("phase-1-first-workload"),
        "rendered alloc-status empty-state must reference phase-1-first-workload; \
         got:\n{rendered}",
    );
    assert!(
        rendered.contains("payments"),
        "rendered alloc-status must name the job id; got:\n{rendered}",
    );
    assert!(
        rendered.contains(&out.spec_digest),
        "rendered alloc-status must carry the spec_digest; got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (e) non-empty rendering shows allocations_total + spec_digest
// -------------------------------------------------------------------

#[test]
fn render_alloc_status_with_allocations_shows_total_and_digest() {
    let out = fixture_with_allocations();
    let rendered = overdrive_cli::render::alloc_status(&out);

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
        !rendered.contains("phase-1-first-workload"),
        "rendered alloc-status with allocations must NOT print the empty-state hint; got:\n{rendered}",
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
fn render_alloc_status_suppresses_hint_when_allocations_exist_even_with_message_populated() {
    // A defensive fixture where allocations_total > 0 AND an
    // empty_state_message happens to be populated (producer might
    // populate it unconditionally). The orig `&&` gate suppresses the
    // hint because `allocations_total == 0` is false; a mutation to
    // `||` would print it because the message is non-empty.
    let out = AllocStatusOutput {
        workload_id: "payments".to_string(),
        spec_digest: "deadbeef".repeat(8),
        allocations_total: 5,
        empty_state_message: "0 allocations for job payments — the scheduler + driver land in \
             phase-1-first-workload"
            .to_string(),
        snapshot: AllocStatusResponse::default(),
    };
    let rendered = overdrive_cli::render::alloc_status(&out);

    assert!(
        !rendered.contains("phase-1-first-workload"),
        "when allocations_total > 0 the empty-state hint MUST NOT appear, \
         even if the producer left an empty_state_message populated — the \
         `allocations_total == 0 && !msg.is_empty()` gate is asymmetric; \
         a mutation of `&&` → `||` would leak the hint. Got:\n{rendered}",
    );
    assert!(
        rendered.contains("Allocations:   5"),
        "the Allocations field must be rendered; got:\n{rendered}",
    );
}

#[test]
fn render_alloc_status_suppresses_hint_when_message_is_empty_even_with_zero_allocations() {
    // `allocations_total == 0 && msg.is_empty()` — the symmetric
    // asymmetric case. Orig: both checks gate → hint not printed.
    // Mutation `&&` → `||`: `0 == 0 || false` = true → writeln!(s,
    // "{}", "") emits a blank line (the leading `\n`).
    //
    // We pin the absence of a spurious trailing blank line that the
    // mutation would leave behind.
    let out = AllocStatusOutput {
        workload_id: "payments".to_string(),
        spec_digest: "cafebabe".repeat(8),
        allocations_total: 0,
        empty_state_message: String::new(),
        snapshot: AllocStatusResponse::default(),
    };
    let rendered = overdrive_cli::render::alloc_status(&out);

    // Under original: last non-empty line is `Allocations:   0\n`;
    // under mutation (`||`) a blank line would follow.
    let lines: Vec<&str> = rendered.split('\n').collect();
    // `split('\n')` on a string ending in `\n` produces a trailing
    // empty element. That's expected and fine. A mutation that fires
    // an empty `writeln!` adds an ADDITIONAL trailing empty line.
    let trailing_empty_count = lines.iter().rev().take_while(|l| l.is_empty()).count();
    assert_eq!(
        trailing_empty_count, 1,
        "render_alloc_status must end in exactly one `\\n` — a mutation of \
         the `&&` gate to `||` would fire writeln! on an empty message and \
         append a spurious blank line. lines = {lines:?}",
    );
    assert!(
        !rendered.contains("phase-1-first-workload"),
        "with both predicates false (msg empty), the hint must not appear; \
         got:\n{rendered}",
    );
}

// -------------------------------------------------------------------
// (g) Listener protocol rendering on the LIVE path.
//
// `main.rs:158` dispatches `overdrive alloc status` through
// `render::alloc_status(&AllocStatusOutput)` — NOT through
// `alloc_status_kind_aware`. The listener protocol (`<port>/<proto>`)
// MUST render here so an operator deploying a UDP Service sees
// `5353/udp`. Listeners are an INTENT property, independent of
// allocations/convergence, so they render even at zero allocations
// (the O03 capture is pre-convergence: `allocations_total == 0`).
// -------------------------------------------------------------------

/// Build a `Listener` from `(port, protocol)`.
const fn listener(port: u16, protocol: Proto) -> Listener {
    Listener { port: NonZeroU16::new(port).expect("non-zero port"), protocol }
}

/// A pre-convergence (zero-allocation) UDP+TCP Service renders each
/// listener as `<port>/<protocol>` under a `Listeners:` header — on the
/// `render::alloc_status` path that the live command actually calls.
#[test]
fn render_alloc_status_renders_listener_protocol_at_zero_allocations() {
    let snapshot = AllocStatusResponse {
        listeners: vec![listener(5353, Proto::Udp), listener(8080, Proto::Tcp)],
        ..Default::default()
    };

    let out = AllocStatusOutput {
        workload_id: "dns-resolver".to_string(),
        spec_digest: "d7b885".to_string() + &"0".repeat(58),
        allocations_total: 0,
        empty_state_message: "0 allocations for job dns-resolver — the scheduler + driver land \
             in phase-1-first-workload"
            .to_string(),
        snapshot,
    };

    let rendered = overdrive_cli::render::alloc_status(&out);

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
fn render_alloc_status_renders_no_listeners_section_when_empty() {
    let out = AllocStatusOutput {
        workload_id: "coinflip".to_string(),
        spec_digest: "f".repeat(64),
        allocations_total: 1,
        empty_state_message: String::new(),
        // default snapshot carries an empty `listeners` vec.
        snapshot: AllocStatusResponse::default(),
    };

    let rendered = overdrive_cli::render::alloc_status(&out);

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
// `render::alloc_status` path (the function `main.rs:158` actually calls)
// dropped it. An operator deploying a Service must see the VIP so the
// frontend address is visible; this is the operator-visibility half of
// #220 (NOT the alloc-status→describe-workload rename). VIP is a
// Service-only frontend property, grouped with `Listeners:` (VIP first),
// and omitted entirely (not rendered as `VIP: None`) for non-Service.
// -------------------------------------------------------------------

/// A Service whose `AllocStatusResponse` carries a VIP renders a `VIP:`
/// line with the platform-issued address on the live `render::alloc_status`
/// path so the operator sees the frontend address.
#[test]
fn render_alloc_status_renders_service_vip_when_present() {
    let snapshot = AllocStatusResponse {
        vip: Some("10.96.0.2".to_string()),
        listeners: vec![listener(5353, Proto::Udp)],
        ..Default::default()
    };

    let out = AllocStatusOutput {
        workload_id: "dns-resolver".to_string(),
        spec_digest: "d7b885".to_string() + &"0".repeat(58),
        allocations_total: 1,
        empty_state_message: String::new(),
        snapshot,
    };

    let rendered = overdrive_cli::render::alloc_status(&out);

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
fn render_alloc_status_renders_no_vip_line_when_absent() {
    let out = AllocStatusOutput {
        workload_id: "coinflip".to_string(),
        spec_digest: "f".repeat(64),
        allocations_total: 1,
        empty_state_message: String::new(),
        // default snapshot carries `vip: None`.
        snapshot: AllocStatusResponse::default(),
    };

    let rendered = overdrive_cli::render::alloc_status(&out);

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
// (`render::alloc_status`, the function `main.rs:158` actually calls)
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
    }
}

/// A Failed allocation whose backend crashed on `bind(): Address already
/// in use` must read as Failed WITH its captured error on the live path.
/// The bare `Allocations: 1` line is no longer the only signal.
#[test]
fn render_alloc_status_surfaces_failed_allocation_state_and_error() {
    let snapshot = AllocStatusResponse {
        rows: vec![row_with_state(
            "alloc-dns-resolver-0",
            AllocStateWire::Failed,
            Some("bind: Address already in use"),
            Some(1),
        )],
        ..Default::default()
    };

    let out = AllocStatusOutput {
        workload_id: "dns-resolver".to_string(),
        spec_digest: "d7b885".to_string() + &"0".repeat(58),
        allocations_total: 1,
        empty_state_message: String::new(),
        snapshot,
    };

    let rendered = overdrive_cli::render::alloc_status(&out);

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
fn render_alloc_status_running_allocation_does_not_read_as_failed() {
    let snapshot = AllocStatusResponse {
        rows: vec![row_with_state("alloc-dns-resolver-0", AllocStateWire::Running, None, None)],
        ..Default::default()
    };

    let out = AllocStatusOutput {
        workload_id: "dns-resolver".to_string(),
        spec_digest: "d7b885".to_string() + &"0".repeat(58),
        allocations_total: 1,
        empty_state_message: String::new(),
        snapshot,
    };

    let rendered = overdrive_cli::render::alloc_status(&out);

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
// `main.rs:158` dispatches `overdrive alloc status` through
// `render::alloc_status(&AllocStatusOutput)` — NOT through
// `alloc_status_kind_aware`. The 03-02 issued-certificates section was
// wired only into `alloc_status_kind_aware` (a test-only renderer with
// zero `src/` callers), so the operator saw nothing. The section MUST
// render on the live path: it reads `out.snapshot.issued_certificates`
// (the `&AllocStatusOutput` shape — fields live under `out.snapshot.*`),
// surfacing the four audit-row FACTS (serial / spiffe_id / issuer_serial
// / not_after) via `Display` and NEVER any cert PEM/DER bytes or private
// key (the audit row carries facts only). See `overdrive-cli/CLAUDE.md`
// § "Alloc-status rendering — `render::alloc_status` is the LIVE path".
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
/// LIVE `render::alloc_status` path — surfacing the four audit-row facts
/// (serial / `spiffe_id` / `issuer_serial` / `not_after`) via `Display`, and
/// NEVER leaking cert PEM/DER bytes or private-key material (the S-OC-11 +
/// S-OC-12 contract on the path `main.rs:158` actually calls).
///
/// Kind is realistic: a running Job alloc with a `/job/` SPIFFE id. The
/// server projects `issued_certificates` per running alloc with no
/// `WorkloadKind` filter, so a Job legitimately carries this summary.
#[test]
fn render_alloc_status_surfaces_issued_certificate_summary_on_live_path() {
    let summary = issued_cert_summary(
        "0a1b2c3d4e5f",
        "spiffe://overdrive.local/job/dns-resolver/alloc/alloc-0",
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

    let out = AllocStatusOutput {
        workload_id: "dns-resolver".to_string(),
        spec_digest: "d7b885".to_string() + &"0".repeat(58),
        allocations_total: 1,
        empty_state_message: String::new(),
        snapshot,
    };

    let rendered = overdrive_cli::render::alloc_status(&out);

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
fn render_alloc_status_omits_issued_certificate_section_when_empty_on_live_path() {
    let out = AllocStatusOutput {
        workload_id: "coinflip".to_string(),
        spec_digest: "f".repeat(64),
        allocations_total: 1,
        empty_state_message: String::new(),
        // default snapshot carries an empty `issued_certificates` vec.
        snapshot: AllocStatusResponse::default(),
    };

    let rendered = overdrive_cli::render::alloc_status(&out);

    assert!(
        !rendered.contains("Issued certificates:"),
        "a workload with no issued certs must NOT render an 'Issued certificates:' \
         section on the live path; got:\n{rendered}",
    );
}
