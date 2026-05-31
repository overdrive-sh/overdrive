//! Tier 1 acceptance — CLI Probes-section render (TUI + JSON) per
//! ADR-0033 enrichment / US-06 / K4. Slice 06.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`: tests call the render /
//! JSON-marshal functions directly (NOT subprocess).
//!
//! Render contract (US-06):
//! - Probes section renders IFF `kind == Service AND probes_present`;
//!   ABSENT for Job / Schedule (the kind-guard is load-bearing).
//! - Each row: role + probe_idx + mechanic summary + last status +
//!   last_observed_at.
//! - `(inferred)` suffix on synthesised default probes.
//! - `last=pending` for probes with no `ProbeResultRow` yet.
//! - `(<consecutive_failures>/<threshold>)` suffix when a
//!   liveness/readiness probe is currently failing.
//! - `NO_COLOR` honoured: zero ANSI escapes when set.
//!
//! Per-mechanic summary line shapes are pinned by `insta` snapshots
//! (golden-output diffs — the PBT-paradigm EXEMPT case; the
//! `ProbeRenderIsKindGuarded` proptest on the adjacent kind-guard slot
//! compensates). Inline-snapshot variant used so the golden lives in
//! the test source and is reviewable in the same diff.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    reason = "acceptance test module — doc backticks and const-fn promotion are not load-bearing for test fixtures"
)]

use overdrive_cli::render::{ProbeRenderRow, probes_section};
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::aggregate::probe_descriptor::ProbeMechanic;
use overdrive_core::observation::probe_result_row::{ProbeIdx, ProbeRole, ProbeStatus};

/// Build a `ProbeRenderRow` with a Pass status observed at `at_ms`.
fn pass_row(role: ProbeRole, idx: u32, mechanic: ProbeMechanic, at_ms: u64) -> ProbeRenderRow {
    ProbeRenderRow {
        role,
        probe_idx: ProbeIdx::new(idx),
        mechanic,
        status: Some(ProbeStatus::Pass),
        last_observed_at_unix_ms: Some(at_ms),
        inferred: false,
        consecutive_failures: 0,
        failure_threshold: None,
    }
}

/// S-SHCP-CLI-01 (US-06 / K4) — stable Service with startup,
/// readiness, liveness probes (all Pass) renders a "Probes:" section
/// with one row per probe naming role, probe_idx, mechanic summary,
/// last status, last observed timestamp.
#[test]
fn given_stable_service_with_three_probes_pass_when_render_then_probes_section_one_row_each() {
    let probes = vec![
        pass_row(
            ProbeRole::Startup,
            0,
            ProbeMechanic::Tcp { host: "127.0.0.1".to_string(), port: 8080 },
            1000,
        ),
        pass_row(
            ProbeRole::Readiness,
            0,
            ProbeMechanic::Http {
                path: "/healthz".to_string(),
                port: 8080,
                host: Some("127.0.0.1".to_string()),
            },
            2000,
        ),
        pass_row(
            ProbeRole::Liveness,
            0,
            ProbeMechanic::Exec { command: vec!["/usr/local/bin/check.sh".to_string()] },
            3000,
        ),
    ];

    let rendered = probes_section(WorkloadKind::Service, &probes, /*no_color=*/ true);

    assert!(rendered.contains("Probes:"), "expected Probes header; got:\n{rendered}");
    // One row per role.
    assert!(rendered.contains("startup"), "expected startup row; got:\n{rendered}");
    assert!(rendered.contains("readiness"), "expected readiness row; got:\n{rendered}");
    assert!(rendered.contains("liveness"), "expected liveness row; got:\n{rendered}");
    // Mechanic summaries per AC shape.
    assert!(rendered.contains("tcp 127.0.0.1:8080"), "expected TCP summary; got:\n{rendered}");
    assert!(
        rendered.contains("http GET http://127.0.0.1:8080/healthz"),
        "expected HTTP summary; got:\n{rendered}",
    );
    assert!(
        rendered.contains("exec /usr/local/bin/check.sh"),
        "expected Exec summary; got:\n{rendered}",
    );
    // Last status + observed timestamp present.
    assert!(rendered.contains("last=pass"), "expected last=pass; got:\n{rendered}");
    assert!(rendered.contains("3000"), "expected last_observed_at ms; got:\n{rendered}");

    // Golden-output pin per-mechanic line shapes (insta inline snapshot
    // — golden-file PBT-paradigm exempt case; compensated by the
    // `ProbeRenderIsKindGuarded` proptest below).
    insta::assert_snapshot!(rendered);
}

/// S-SHCP-CLI-02 (US-06 / K4 negative) — Job-kind alloc renders
/// WITHOUT a Probes section anywhere in output. Renderer-side kind
/// guard.
#[test]
fn given_job_kind_alloc_when_render_then_no_probes_section() {
    let probes = vec![pass_row(
        ProbeRole::Startup,
        0,
        ProbeMechanic::Tcp { host: "127.0.0.1".to_string(), port: 8080 },
        1000,
    )];
    let rendered = probes_section(WorkloadKind::Job, &probes, /*no_color=*/ true);
    assert!(
        !rendered.contains("Probes:"),
        "S-SHCP-CLI-02: Job-kind render must NOT contain Probes section; got:\n{rendered}",
    );
}

/// S-SHCP-CLI-03 (US-06 / K4 negative) — Schedule-kind alloc renders
/// WITHOUT a Probes section.
#[test]
fn given_schedule_kind_alloc_when_render_then_no_probes_section() {
    let probes = vec![pass_row(
        ProbeRole::Startup,
        0,
        ProbeMechanic::Tcp { host: "127.0.0.1".to_string(), port: 8080 },
        1000,
    )];
    let rendered = probes_section(WorkloadKind::Schedule, &probes, /*no_color=*/ true);
    assert!(
        !rendered.contains("Probes:"),
        "S-SHCP-CLI-03: Schedule-kind render must NOT contain Probes section; got:\n{rendered}",
    );
}

/// S-SHCP-CLI-04 (US-06 — failing probe with reason) — Probe Fail row
/// renders `last_fail_reason` (e.g. "HTTP 503") in the same row, and a
/// `(<consecutive_failures>/<threshold>)` ratio suffix when the probe
/// is currently failing under a threshold.
#[test]
fn given_probe_fail_row_when_render_then_last_fail_reason_in_output() {
    let probes = vec![ProbeRenderRow {
        role: ProbeRole::Liveness,
        probe_idx: ProbeIdx::new(0),
        mechanic: ProbeMechanic::Http {
            path: "/healthz".to_string(),
            port: 8080,
            host: Some("127.0.0.1".to_string()),
        },
        status: Some(ProbeStatus::Fail { last_fail_reason: "HTTP 503".to_string() }),
        last_observed_at_unix_ms: Some(4000),
        inferred: false,
        consecutive_failures: 2,
        failure_threshold: Some(3),
    }];
    let rendered = probes_section(WorkloadKind::Service, &probes, /*no_color=*/ true);
    assert!(rendered.contains("HTTP 503"), "expected last_fail_reason text; got:\n{rendered}");
    assert!(
        rendered.contains("(2/3)"),
        "expected (consecutive/threshold) ratio suffix; got:\n{rendered}",
    );
}

/// S-SHCP-CLI-05 (US-06 — pending state) — Service alloc with probes
/// declared but no ProbeResultRow yet written renders `last=pending`
/// (NOT blank).
#[test]
fn given_just_started_service_with_no_probe_result_yet_when_render_then_last_equals_pending() {
    let probes = vec![ProbeRenderRow {
        role: ProbeRole::Startup,
        probe_idx: ProbeIdx::new(0),
        mechanic: ProbeMechanic::Tcp { host: "127.0.0.1".to_string(), port: 8080 },
        status: None,
        last_observed_at_unix_ms: None,
        inferred: false,
        consecutive_failures: 0,
        failure_threshold: None,
    }];
    let rendered = probes_section(WorkloadKind::Service, &probes, /*no_color=*/ true);
    assert!(rendered.contains("last=pending"), "expected last=pending; got:\n{rendered}");
}

/// S-SHCP-CLI-06 (US-01 — inferred default render) — Stable Service
/// submitted WITHOUT explicit probes renders the inferred TCP startup
/// probe with `(inferred)` suffix.
#[test]
fn given_stable_service_with_inferred_default_probe_when_render_then_marked_inferred() {
    let probes = vec![ProbeRenderRow {
        role: ProbeRole::Startup,
        probe_idx: ProbeIdx::new(0),
        mechanic: ProbeMechanic::Tcp { host: "127.0.0.1".to_string(), port: 8080 },
        status: Some(ProbeStatus::Pass),
        last_observed_at_unix_ms: Some(1000),
        inferred: true,
        consecutive_failures: 0,
        failure_threshold: None,
    }];
    let rendered = probes_section(WorkloadKind::Service, &probes, /*no_color=*/ true);
    assert!(rendered.contains("(inferred)"), "expected (inferred) suffix; got:\n{rendered}");
}

/// NO_COLOR AC — with `no_color = true`, render output contains zero
/// ANSI escape sequences (the ESC byte 0x1b never appears).
#[test]
fn given_no_color_when_render_then_zero_ansi_escapes() {
    let probes = vec![ProbeRenderRow {
        role: ProbeRole::Liveness,
        probe_idx: ProbeIdx::new(0),
        mechanic: ProbeMechanic::Http {
            path: "/healthz".to_string(),
            port: 8080,
            host: Some("127.0.0.1".to_string()),
        },
        status: Some(ProbeStatus::Fail { last_fail_reason: "HTTP 503".to_string() }),
        last_observed_at_unix_ms: Some(4000),
        inferred: false,
        consecutive_failures: 2,
        failure_threshold: Some(3),
    }];
    let rendered = probes_section(WorkloadKind::Service, &probes, /*no_color=*/ true);
    assert!(
        !rendered.contains('\u{1b}'),
        "NO_COLOR: render must contain zero ANSI escape (0x1b); got:\n{rendered:?}",
    );
}

// ---------------------------------------------------------------------------
// JSON-mode acceptance — `probes` field skip-if-none per ADR-0033
// enrichment shape.
// ---------------------------------------------------------------------------

use overdrive_cli::commands::alloc::format_alloc_status_json;
use overdrive_core::id::AllocationId;
use overdrive_core::observation::probe_result_row::ProbeResultRow;
use std::str::FromStr as _;

fn fixture_result_row(role: ProbeRole, status: ProbeStatus) -> ProbeResultRow {
    ProbeResultRow {
        alloc_id: AllocationId::from_str("alloc-json-0").expect("valid alloc id"),
        probe_idx: ProbeIdx::new(0),
        role,
        status,
        last_observed_at_unix_ms: 1000,
        inferred: false,
    }
}

/// JSON-mode AC (Service) — `format_alloc_status_json` for a
/// Service-kind alloc includes a `probes` array carrying the
/// `ProbeResultRowJson` projection.
#[test]
fn given_service_kind_when_format_json_then_probes_array_present() {
    let rows = vec![fixture_result_row(ProbeRole::Startup, ProbeStatus::Pass)];
    let json = format_alloc_status_json(WorkloadKind::Service, &rows);
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let probes = value.get("probes").expect("Service JSON must carry `probes`");
    assert!(probes.is_array(), "`probes` must be an array; got: {probes}");
    assert_eq!(probes.as_array().expect("array").len(), 1, "one probe row; got: {probes}");
    assert_eq!(probes[0]["role"], "startup", "role projected; got: {probes}");
}

/// JSON-mode AC (Job) — `format_alloc_status_json` for a Job-kind
/// alloc OMITS the `probes` field entirely (serde skip-if-none), not
/// `null`.
#[test]
fn given_job_kind_when_format_json_then_probes_field_omitted() {
    let rows = vec![fixture_result_row(ProbeRole::Startup, ProbeStatus::Pass)];
    let json = format_alloc_status_json(WorkloadKind::Job, &rows);
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert!(
        value.get("probes").is_none(),
        "Job-kind JSON must OMIT `probes` (not null); got: {json}",
    );
}

// ---------------------------------------------------------------------------
// Property-based unit tests (Stage 0 — paradigm-from-day-zero).
// ---------------------------------------------------------------------------

use proptest::prelude::*;

/// Strategy over the FULL `WorkloadKind` enum universe (Service / Job /
/// Schedule). Exhaustive — every variant is generated.
fn arb_workload_kind() -> impl Strategy<Value = WorkloadKind> {
    prop_oneof![Just(WorkloadKind::Service), Just(WorkloadKind::Job), Just(WorkloadKind::Schedule),]
}

/// Strategy over the three concrete probe mechanics, with arbitrary
/// host/port/path/command content.
fn arb_mechanic() -> impl Strategy<Value = ProbeMechanic> {
    prop_oneof![
        ("[a-z0-9.]{1,12}", 1u16..=65535)
            .prop_map(|(host, port)| ProbeMechanic::Tcp { host, port }),
        ("/[a-z]{1,8}", 1u16..=65535, proptest::option::of("[a-z0-9.]{1,12}"))
            .prop_map(|(path, port, host)| ProbeMechanic::Http { path, port, host }),
        proptest::collection::vec("[a-z./]{1,10}", 1..=3)
            .prop_map(|command| ProbeMechanic::Exec { command }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// `ProbeRenderIsKindGuarded` — Universe = full `WorkloadKind` enum
    /// × `present: bool`. INVARIANT: the rendered output contains the
    /// literal substring `"Probes:"` IFF (kind == Service AND present);
    /// otherwise ZERO occurrences.
    ///
    /// State universe (port-exposed observable surface of the render
    /// function): `{ output.contains("Probes:") }`. The kind-guard is
    /// the single load-bearing render contract per US-06; every other
    /// slot is `unchanged()` by construction (the function returns a
    /// fresh String — there is no adjacent persistent state).
    #[test]
    fn probe_render_is_kind_guarded(
        kind in arb_workload_kind(),
        present in any::<bool>(),
        mechanic in arb_mechanic(),
    ) {
        let probes = if present {
            vec![ProbeRenderRow {
                role: ProbeRole::Startup,
                probe_idx: ProbeIdx::new(0),
                mechanic,
                status: Some(ProbeStatus::Pass),
                last_observed_at_unix_ms: Some(1000),
                inferred: false,
                consecutive_failures: 0,
                failure_threshold: None,
            }]
        } else {
            Vec::new()
        };

        let rendered = probes_section(kind, &probes, /*no_color=*/ true);

        let should_render = matches!(kind, WorkloadKind::Service) && present;
        let has_section = rendered.contains("Probes:");
        prop_assert_eq!(
            has_section, should_render,
            "kind-guard: kind={:?} present={} -> expected Probes section={}, got section={}; \
             rendered=\n{}",
            kind, present, should_render, has_section, rendered,
        );
    }

    /// NO_COLOR invariant — for every kind × present × mechanic, with
    /// `no_color = true` the render contains zero ANSI escape bytes
    /// (0x1b). Universe: `{ output.contains('\u{1b}') }` must be
    /// `false` unconditionally.
    #[test]
    fn no_color_render_has_zero_ansi_escapes(
        kind in arb_workload_kind(),
        mechanic in arb_mechanic(),
        failing in any::<bool>(),
    ) {
        let status = if failing {
            Some(ProbeStatus::Fail { last_fail_reason: "HTTP 503".to_string() })
        } else {
            Some(ProbeStatus::Pass)
        };
        let probes = vec![ProbeRenderRow {
            role: ProbeRole::Liveness,
            probe_idx: ProbeIdx::new(0),
            mechanic,
            status,
            last_observed_at_unix_ms: Some(4000),
            inferred: false,
            consecutive_failures: if failing { 2 } else { 0 },
            failure_threshold: Some(3),
        }];
        let rendered = probes_section(kind, &probes, /*no_color=*/ true);
        prop_assert!(
            !rendered.contains('\u{1b}'),
            "NO_COLOR: render must contain zero ANSI escape (0x1b); got: {:?}",
            rendered,
        );
    }
}
