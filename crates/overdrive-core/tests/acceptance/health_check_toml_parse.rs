//! Tier 1 acceptance — `[[health_check.*]]` TOML parse rules per
//! ADR-0057 / ADR-0058. Step 01-02 GREEN landing for the TCP
//! variant + default inference + opt-out distinction.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    reason = "RED scaffolds for later slices; lints land when DELIVER replaces todo!() bodies + rewrites docs"
)]

use overdrive_core::aggregate::{
    ParseError, ProbeDescriptor, ProbeMechanic, ServiceSpec, WorkloadSpecInput,
};
use overdrive_core::observation::ProbeRole;
use proptest::prelude::*;

// ---------------------------------------------------------------------
// RED scaffolds — owned by later slices (02 / 03 / 07).
// ---------------------------------------------------------------------

/// S-SHCP-PARSE-01 (US-02 / K1 / ADR-0057 §1) — HTTP probe parse + defaults.
#[test]
#[should_panic(expected = "RED scaffold")]
fn health_check_startup_http_parses_with_defaults() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PARSE-01 / [[health_check.startup]] HTTP parses with defaults)"
    );
}

/// S-SHCP-PARSE-02 (US-02 / K5 / ADR-0057 §3) — HTTP missing path.
#[test]
#[should_panic(expected = "RED scaffold")]
fn health_check_http_missing_path_yields_named_parse_error() {
    panic!("Not yet implemented -- RED scaffold (S-SHCP-PARSE-02 / HTTP probe missing path)");
}

/// S-SHCP-PARSE-03 (US-03 / K1 / ADR-0057 §1) — Exec probe parse.
#[test]
#[should_panic(expected = "RED scaffold")]
fn health_check_startup_exec_parses_with_defaults() {
    panic!("Not yet implemented -- RED scaffold (S-SHCP-PARSE-03 / Exec parses with defaults)");
}

/// S-SHCP-PARSE-04 (US-03 / K5 / ADR-0057 §3) — Exec empty command.
#[test]
#[should_panic(expected = "RED scaffold")]
fn health_check_exec_empty_command_yields_named_parse_error() {
    panic!("Not yet implemented -- RED scaffold (S-SHCP-PARSE-04 / Exec empty command)");
}

/// S-SHCP-PARSE-05 (US-07 / K5) — probes under [job] rejected.
#[test]
#[should_panic(expected = "RED scaffold")]
fn probes_under_job_kind_yields_named_parse_error_with_guidance() {
    panic!("Not yet implemented -- RED scaffold (S-SHCP-PARSE-05 / probes under job rejected)");
}

/// S-SHCP-PARSE-06 (US-07 / K5) — probes under [schedule] rejected.
#[test]
#[should_panic(expected = "RED scaffold")]
fn probes_under_schedule_kind_yields_named_parse_error_with_guidance() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-PARSE-06 / probes under schedule rejected)"
    );
}

/// S-SHCP-PARSE-07 (US-07 regression guard) — probes under [service] parses.
#[test]
#[should_panic(expected = "RED scaffold")]
fn probes_under_service_kind_parses_successfully() {
    panic!("Not yet implemented -- RED scaffold (S-SHCP-PARSE-07 / probes under service parses)");
}

/// S-SHCP-PARSE-08 (US-02 AC) — https:// scheme rejected.
#[test]
#[should_panic(expected = "RED scaffold")]
fn health_check_http_https_scheme_yields_named_parse_error() {
    panic!("Not yet implemented -- RED scaffold (S-SHCP-PARSE-08 / https:// scheme rejected)");
}

// ---------------------------------------------------------------------
// Step 01-02 active scenarios — ADR-0057 TCP + ADR-0058 default-inference.
// PBT per the standing paradigm mandate.
// ---------------------------------------------------------------------

fn port_strategy() -> impl Strategy<Value = u16> {
    1u16..=65535u16
}

fn service_id_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::char::range('a', 'z'), 1..=16)
        .prop_map(|chars| chars.into_iter().collect())
}

fn unwrap_service(input: WorkloadSpecInput) -> ServiceSpec {
    match input {
        WorkloadSpecInput::Service(s) => s,
        other => panic!("expected Service kind, got {other:?}"),
    }
}

const SERVICE_PRELUDE: &str = r#"
[service]
id = "svc"
replicas = 1

[[listener]]
port = 8080
protocol = "tcp"

[exec]
command = "/usr/bin/server"
args = []

[resources]
cpu_milli = 100
memory_bytes = 134217728
"#;

fn parse_err(toml: &str) -> ParseError {
    WorkloadSpecInput::from_toml_str(toml).expect_err("expected parse to fail")
}

// S-SHCP-INFER-01 (US-01 WS / DDD-15 / ADR-0058) — default TCP inference.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn service_without_probes_with_listener_infers_default_tcp_startup_probe(
        id in service_id_strategy(),
        port in port_strategy(),
    ) {
        let toml = format!(
            r#"
[service]
id = "{id}"
replicas = 1

[[listener]]
port = {port}
protocol = "tcp"

[exec]
command = "/usr/bin/server"
args = []

[resources]
cpu_milli = 100
memory_bytes = 134217728
"#
        );
        let input = WorkloadSpecInput::from_toml_str(&toml)
            .expect("parse must succeed");
        let svc = unwrap_service(input);

        prop_assert_eq!(svc.startup_probes.len(), 1, "exactly one inferred startup probe");
        let p: &ProbeDescriptor = &svc.startup_probes[0];
        prop_assert_eq!(p.role, ProbeRole::Startup);
        match &p.mechanic {
            ProbeMechanic::Tcp { host, port: probe_port } => {
                prop_assert_eq!(host.as_str(), "0.0.0.0");
                prop_assert_eq!(*probe_port, port);
            }
            other => prop_assert!(false, "expected Tcp mechanic, got {:?}", other),
        }
        prop_assert!(p.inferred, "inferred default carries inferred: true");
        prop_assert_eq!(p.timeout_seconds, 5);
        prop_assert_eq!(p.interval_seconds, 2);
        prop_assert_eq!(p.max_attempts, 30);
    }
}

// S-SHCP-INFER-02 (DDD-16 / ADR-0058) — explicit empty opts out.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn service_with_empty_startup_array_opts_out_of_default_inference(
        id in service_id_strategy(),
        port in port_strategy(),
    ) {
        let toml = format!(
            r#"
[service]
id = "{id}"
replicas = 1

[[listener]]
port = {port}
protocol = "tcp"

[exec]
command = "/usr/bin/server"
args = []

[resources]
cpu_milli = 100
memory_bytes = 134217728

[health_check]
startup = []
"#
        );
        let input = WorkloadSpecInput::from_toml_str(&toml)
            .expect("parse must succeed");
        let svc = unwrap_service(input);
        prop_assert!(svc.startup_probes.is_empty(),
            "explicit empty array opts out of default inference; expected zero probes");
    }
}

#[test]
fn tcp_probe_missing_port_yields_named_parse_error() {
    let toml = format!(
        "{SERVICE_PRELUDE}\n\
         [[health_check.startup]]\n\
         type = \"tcp\"\n"
    );
    match parse_err(&toml) {
        ParseError::TcpProbeMissingPort { probe_idx } => assert_eq!(probe_idx, 0),
        other => panic!("expected TcpProbeMissingPort, got {other:?}"),
    }
}

#[test]
fn probe_timeout_zero_yields_named_parse_error() {
    let toml = format!(
        "{SERVICE_PRELUDE}\n\
         [[health_check.startup]]\n\
         type = \"tcp\"\n\
         port = 8080\n\
         timeout_seconds = 0\n"
    );
    match parse_err(&toml) {
        ParseError::ProbeTimeoutZero { probe_idx } => assert_eq!(probe_idx, 0),
        other => panic!("expected ProbeTimeoutZero, got {other:?}"),
    }
}

#[test]
fn probe_interval_zero_yields_named_parse_error() {
    let toml = format!(
        "{SERVICE_PRELUDE}\n\
         [[health_check.startup]]\n\
         type = \"tcp\"\n\
         port = 8080\n\
         interval_seconds = 0\n"
    );
    match parse_err(&toml) {
        ParseError::ProbeIntervalZero { probe_idx } => assert_eq!(probe_idx, 0),
        other => panic!("expected ProbeIntervalZero, got {other:?}"),
    }
}

#[test]
fn probe_max_attempts_zero_yields_named_parse_error() {
    let toml = format!(
        "{SERVICE_PRELUDE}\n\
         [[health_check.startup]]\n\
         type = \"tcp\"\n\
         port = 8080\n\
         max_attempts = 0\n"
    );
    match parse_err(&toml) {
        ParseError::ProbeMaxAttemptsZero { probe_idx } => assert_eq!(probe_idx, 0),
        other => panic!("expected ProbeMaxAttemptsZero, got {other:?}"),
    }
}

#[test]
fn tcp_type_field_is_case_insensitive() {
    for casing in &["tcp", "TCP", "Tcp", "tCp"] {
        let toml = format!(
            "{SERVICE_PRELUDE}\n\
             [[health_check.startup]]\n\
             type = \"{casing}\"\n\
             port = 9000\n"
        );
        let input = WorkloadSpecInput::from_toml_str(&toml)
            .unwrap_or_else(|e| panic!("parse for casing {casing:?} must succeed: {e}"));
        let svc = unwrap_service(input);
        assert_eq!(svc.startup_probes.len(), 1);
        match &svc.startup_probes[0].mechanic {
            ProbeMechanic::Tcp { port, .. } => assert_eq!(*port, 9000),
            other => panic!("expected Tcp mechanic for casing {casing:?}, got {other:?}"),
        }
        assert!(
            !svc.startup_probes[0].inferred,
            "operator-declared probe must NOT carry inferred=true"
        );
    }
}

#[test]
fn declared_probe_inherits_adr_0057_defaults_when_fields_omitted() {
    let toml = format!(
        "{SERVICE_PRELUDE}\n\
         [[health_check.startup]]\n\
         type = \"tcp\"\n\
         port = 8080\n"
    );
    let input = WorkloadSpecInput::from_toml_str(&toml).expect("parse succeeds");
    let svc = unwrap_service(input);
    let p = &svc.startup_probes[0];
    assert_eq!(p.timeout_seconds, 5);
    assert_eq!(p.interval_seconds, 2);
    assert_eq!(p.max_attempts, 30);
    assert!(!p.inferred);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn declared_probe_round_trips_operator_overrides(
        timeout in 1u32..=60u32,
        interval in 1u32..=60u32,
        max_attempts in 1u32..=120u32,
        port in port_strategy(),
    ) {
        let toml = format!(
            "{SERVICE_PRELUDE}\n\
             [[health_check.startup]]\n\
             type = \"tcp\"\n\
             port = {port}\n\
             timeout_seconds = {timeout}\n\
             interval_seconds = {interval}\n\
             max_attempts = {max_attempts}\n"
        );
        let input = WorkloadSpecInput::from_toml_str(&toml)
            .expect("parse must succeed");
        let svc = unwrap_service(input);
        prop_assert_eq!(svc.startup_probes.len(), 1);
        let p = &svc.startup_probes[0];
        prop_assert_eq!(p.timeout_seconds, timeout);
        prop_assert_eq!(p.interval_seconds, interval);
        prop_assert_eq!(p.max_attempts, max_attempts);
        match &p.mechanic {
            ProbeMechanic::Tcp { port: pp, .. } => prop_assert_eq!(*pp, port),
            other => prop_assert!(false, "expected Tcp, got {:?}", other),
        }
        prop_assert!(!p.inferred);
    }
}
