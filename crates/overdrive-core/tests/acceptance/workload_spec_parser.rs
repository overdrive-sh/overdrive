//! Acceptance scenarios for `workload-kind-discriminator` Slice 01 — the
//! `WorkloadSpec` tagged enum at the parser boundary.
//!
//! Driving port: `WorkloadSpecInput::deserialize(toml_bytes)` —
//! custom `Deserialize` impl per ADR-0047 §2 (NOT bare
//! `#[serde(untagged)]`; error messages must name the offending TOML
//! sections).
//!
//! Scenarios from
//! `docs/feature/workload-kind-discriminator/distill/test-scenarios.md`
//! §1 (S-01-01 .. S-01-09).

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use overdrive_core::aggregate::{ParseError, WorkloadKind, WorkloadSpecInput};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse `src` as a `WorkloadSpecInput`. Returns the kind discriminator
/// on success, the structured `ParseError` on failure.
fn parse(src: &str) -> Result<WorkloadKind, ParseError> {
    WorkloadSpecInput::from_toml_str(src).map(|input| input.kind())
}

/// Canonical valid `[service]` body — id, two listeners, exec, resources.
const SERVICE_TOML: &str = r#"
[service]
id = "payments"
replicas = 1

[[listener]]
port = 8080
protocol = "tcp"
vip = "10.0.0.1"

[[listener]]
port = 8081
protocol = "udp"

[exec]
command = "/opt/payments/bin/server"
args = ["--port", "8080"]

[resources]
cpu_milli = 500
memory_bytes = 134217728
"#;

/// Canonical valid `[job]` body.
const JOB_TOML: &str = r#"
[job]
id = "coinflip"

[exec]
command = "/bin/bash"
args = ["-c", "exit 0"]

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#;

/// Canonical valid `[job] + [schedule]` body.
const SCHEDULE_TOML: &str = r#"
[job]
id = "nightly-backup"

[schedule]
cron = "0 2 * * *"

[exec]
command = "/usr/local/bin/backup"
args = []

[resources]
cpu_milli = 200
memory_bytes = 134217728
"#;

// ---------------------------------------------------------------------------
// S-01-01 — Service spec is recognised by [service] section presence
// ---------------------------------------------------------------------------

#[test]
fn s_01_01_service_spec_recognised_by_section_presence() {
    let kind = parse(SERVICE_TOML).expect("canonical service spec must parse");
    assert_eq!(kind, WorkloadKind::Service, "[service]-only must yield Service kind");

    // Identifier surfaces through the parsed input.
    let parsed =
        WorkloadSpecInput::from_toml_str(SERVICE_TOML).expect("canonical service spec must parse");
    assert_eq!(parsed.id_as_str(), "payments");
}

// ---------------------------------------------------------------------------
// S-01-02 — Job spec is recognised by [job] section presence
// ---------------------------------------------------------------------------

#[test]
fn s_01_02_job_spec_recognised_by_section_presence() {
    let kind = parse(JOB_TOML).expect("canonical job spec must parse");
    assert_eq!(kind, WorkloadKind::Job, "[job]-only must yield Job kind");

    let parsed = WorkloadSpecInput::from_toml_str(JOB_TOML).expect("canonical job spec must parse");
    assert_eq!(parsed.id_as_str(), "coinflip");
}

// ---------------------------------------------------------------------------
// S-01-03 — Scheduled Job spec is recognised by [job] + [schedule] co-presence
// ---------------------------------------------------------------------------

#[test]
fn s_01_03_scheduled_job_recognised_by_co_presence() {
    let parsed = WorkloadSpecInput::from_toml_str(SCHEDULE_TOML)
        .expect("canonical schedule spec must parse");
    assert_eq!(parsed.kind(), WorkloadKind::Schedule);
    // Cron expression captured verbatim per AC.
    assert_eq!(parsed.cron_expr_str(), Some("0 2 * * *"));
}

// ---------------------------------------------------------------------------
// S-01-04 — Spec with both [service] and [job] is rejected with named guidance
// ---------------------------------------------------------------------------

#[test]
fn s_01_04_mixed_service_job_rejected_with_named_guidance() {
    let toml = r#"
[service]
id = "ambiguous"
replicas = 1

[[listener]]
port = 8080
protocol = "tcp"

[job]
id = "ambiguous"

[exec]
command = "/bin/true"
args = []

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#;
    let err = parse(toml).expect_err("mixed [service] + [job] must be rejected");
    let display = err.to_string();
    assert!(display.contains("[service]"), "error must name [service]; got: {display}");
    assert!(display.contains("[job]"), "error must name [job]; got: {display}");
    assert!(
        display.contains("exactly one"),
        "error must suggest 'exactly one of [service] or [job] is required'; got: {display}"
    );
}

// ---------------------------------------------------------------------------
// S-01-05 — Spec with [schedule] but no [job] is rejected
// ---------------------------------------------------------------------------

#[test]
fn s_01_05_schedule_without_job_rejected() {
    let toml = r#"
[schedule]
cron = "0 2 * * *"

[exec]
command = "/bin/true"
args = []

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#;
    let err = parse(toml).expect_err("[schedule] without [job] must be rejected");
    let display = err.to_string();
    assert!(display.contains("[schedule]"), "error must name [schedule]; got: {display}");
    assert!(
        display.contains("[schedule] is only valid alongside [job]"),
        "error must explain [schedule] requires [job]; got: {display}"
    );
}

// ---------------------------------------------------------------------------
// S-01-06 — Spec with [schedule] AND [service] is rejected
// ---------------------------------------------------------------------------

#[test]
fn s_01_06_schedule_with_service_rejected() {
    let toml = r#"
[service]
id = "payments"
replicas = 1

[[listener]]
port = 8080
protocol = "tcp"

[schedule]
cron = "0 2 * * *"

[exec]
command = "/bin/true"
args = []

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#;
    let err = parse(toml).expect_err("[schedule] + [service] must be rejected");
    let display = err.to_string();
    assert!(
        display.contains("[schedule] is only valid alongside [job]"),
        "error must explain [schedule] requires [job]; got: {display}"
    );
}

// ---------------------------------------------------------------------------
// S-01-07 — Spec missing [exec] is rejected
// ---------------------------------------------------------------------------

#[test]
fn s_01_07_missing_exec_rejected() {
    let toml = r#"
[job]
id = "coinflip"

[resources]
cpu_milli = 100
memory_bytes = 67108864
"#;
    let err = parse(toml).expect_err("missing [exec] must be rejected");
    let display = err.to_string();
    assert!(display.contains("[exec]"), "error must name [exec]; got: {display}");
}

// ---------------------------------------------------------------------------
// S-01-08 — Parser rejection latency is within K2 (95% within 50ms)
// ---------------------------------------------------------------------------

#[test]
fn s_01_08_rejection_latency_within_kpi_budget() {
    // Test fixture allowed to call Instant::now per
    // .claude/rules/testing.md § "RED scaffolds and intentionally-failing
    // commits" — fixtures and tests aren't on the dst-lint banned list
    // for `crate_class = "core"` only via tests/ files.
    use std::time::Instant;

    let invalid_specs: Vec<&str> = vec![
        // [service] + [job]
        r#"
[service]
id = "x"
replicas = 1
[[listener]]
port = 1
protocol = "tcp"
[job]
id = "x"
[exec]
command = "/bin/true"
args = []
[resources]
cpu_milli = 1
memory_bytes = 1
"#,
        // [schedule] alone
        r#"
[schedule]
cron = "0 2 * * *"
[exec]
command = "/bin/true"
args = []
[resources]
cpu_milli = 1
memory_bytes = 1
"#,
        // [schedule] + [service]
        r#"
[service]
id = "x"
replicas = 1
[[listener]]
port = 1
protocol = "tcp"
[schedule]
cron = "0 2 * * *"
[exec]
command = "/bin/true"
args = []
[resources]
cpu_milli = 1
memory_bytes = 1
"#,
        // missing [exec]
        r#"
[job]
id = "x"
[resources]
cpu_milli = 1
memory_bytes = 1
"#,
        // missing [resources]
        r#"
[job]
id = "x"
[exec]
command = "/bin/true"
args = []
"#,
    ];

    let mut durations_us: Vec<u128> = Vec::new();
    for src in &invalid_specs {
        // Run each input several times to amortise warmup noise.
        for _ in 0..20 {
            let t0 = Instant::now();
            let _ = WorkloadSpecInput::from_toml_str(src);
            durations_us.push(t0.elapsed().as_micros());
        }
    }
    durations_us.sort_unstable();
    // 95th-percentile index: floor(len * 0.95). Done in integer
    // arithmetic to avoid precision warnings on the f64 round-trip.
    let p95_idx = durations_us.len().saturating_mul(95) / 100;
    let p95_us = durations_us[p95_idx.min(durations_us.len() - 1)];
    // 50 ms = 50_000 us. Generous slack for CI runners.
    assert!(
        p95_us < 50_000,
        "K2 KPI: 95th-percentile rejection latency must be <50ms; observed {p95_us}us \
         across {} samples",
        durations_us.len()
    );
}

// ---------------------------------------------------------------------------
// S-01-09 — Mixed-kind rejection holds across arbitrary section orderings
// ---------------------------------------------------------------------------

mod s_01_09 {
    use super::*;
    use proptest::prelude::*;

    /// Three-section presence permutations: each section may or may not
    /// appear, with a deterministic ordering by section name.
    #[derive(Debug, Clone)]
    struct Permutation {
        has_service: bool,
        has_job: bool,
        has_schedule: bool,
        order: u8, // 0..6, picks one of 3! orderings
    }

    fn arb_perm() -> impl Strategy<Value = Permutation> {
        (any::<bool>(), any::<bool>(), any::<bool>(), 0u8..6).prop_map(
            |(has_service, has_job, has_schedule, order)| Permutation {
                has_service,
                has_job,
                has_schedule,
                order,
            },
        )
    }

    fn render(p: &Permutation) -> String {
        let service =
            "[service]\nid = \"x\"\nreplicas = 1\n\n[[listener]]\nport = 1\nprotocol = \"tcp\"\n";
        let job = "[job]\nid = \"x\"\n";
        let schedule = "[schedule]\ncron = \"0 2 * * *\"\n";
        let mut sections: Vec<&str> = Vec::new();
        if p.has_service {
            sections.push(service);
        }
        if p.has_job {
            sections.push(job);
        }
        if p.has_schedule {
            sections.push(schedule);
        }
        // Permute sections by `order` mod len.
        if !sections.is_empty() {
            let n = sections.len();
            let shift = (p.order as usize) % n;
            sections.rotate_left(shift);
        }
        let mut out = String::new();
        for s in &sections {
            out.push_str(s);
            out.push('\n');
        }
        out.push_str(
            "[exec]\ncommand = \"/bin/true\"\nargs = []\n\n[resources]\ncpu_milli = 1\nmemory_bytes = 1\n",
        );
        out
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1024))]
        /// Every two-of-three or three-of-three section permutation must
        /// be rejected with a named-guidance error. Singleton (exactly
        /// one of [service]/[job]) permutations are allowed and produce
        /// success; we check rejection only for the invalid shapes.
        #[test]
        fn mixed_kind_rejected_across_orderings(p in arb_perm()) {
            let kinds_present = u8::from(p.has_service)
                + u8::from(p.has_job)
                + u8::from(p.has_schedule && !p.has_job);
            let toml = render(&p);
            let parsed = WorkloadSpecInput::from_toml_str(&toml);
            // Valid shapes: [service] alone (no schedule), [job] alone
            // (no service), [job] + [schedule] (no service).
            let valid = matches!(
                (p.has_service, p.has_job, p.has_schedule),
                (true, false, false) | (false, true, _)
            );
            if valid {
                prop_assert!(
                    parsed.is_ok(),
                    "valid kind shape (S={} J={} Sched={}) must parse; got error: {:?}",
                    p.has_service,
                    p.has_job,
                    p.has_schedule,
                    parsed.err(),
                );
            } else {
                prop_assert!(
                    parsed.is_err(),
                    "invalid kind shape (S={} J={} Sched={}) must be rejected (kinds_present={})",
                    p.has_service,
                    p.has_job,
                    p.has_schedule,
                    kinds_present,
                );
            }
        }
    }
}
