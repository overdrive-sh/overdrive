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

// S-SHCP-PARSE-01 (US-02 / K1 / ADR-0057 §1) — HTTP probe parse +
// defaults. Proptest over arbitrary valid (path, port) tuples: a
// `type = "http"` startup probe parses into
// `ProbeMechanic::Http { path, port, host: None }` with the ADR-0057
// §2 defaults (timeout 5, interval 2, max_attempts 30).
// Universe (port-exposed observable surface of the parsed descriptor):
// mechanic (path / port / host), timeout_seconds, interval_seconds,
// max_attempts, inferred. Every slot is asserted; none silently drifts.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn health_check_startup_http_parses_with_defaults(
        id in service_id_strategy(),
        port in port_strategy(),
        path in http_path_strategy(),
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

[[health_check.startup]]
type = "http"
path = "{path}"
port = {port}
"#
        );
        let input = WorkloadSpecInput::from_toml_str(&toml)
            .expect("HTTP probe parse must succeed");
        let svc = unwrap_service(input);
        prop_assert_eq!(svc.startup_probes.len(), 1, "exactly one declared http probe");
        let p: &ProbeDescriptor = &svc.startup_probes[0];
        prop_assert_eq!(p.role, ProbeRole::Startup);
        match &p.mechanic {
            ProbeMechanic::Http { path: parsed_path, port: parsed_port, host } => {
                prop_assert_eq!(parsed_path.as_str(), path.as_str());
                prop_assert_eq!(*parsed_port, port);
                prop_assert_eq!(host.as_ref(), None::<&String>,
                    "host omitted in TOML → None (defaulted at probe time, not parse time)");
            }
            other => prop_assert!(false, "expected Http mechanic, got {:?}", other),
        }
        prop_assert_eq!(p.timeout_seconds, 5);
        prop_assert_eq!(p.interval_seconds, 2);
        prop_assert_eq!(p.max_attempts, 30);
        prop_assert!(!p.inferred, "operator-declared probe carries inferred=false");
    }
}

// S-SHCP-PARSE-02 (US-02 / K5 / ADR-0057 §3) — HTTP probe with `port`
// present and `path` absent yields
// `ParseError::HttpProbeMissingPath { probe_idx }`. Proptest over
// arbitrary ports — the missing-path verdict is invariant in the port
// value, and the reported probe_idx is the observed position (0 for the
// single-probe TOML).
proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn health_check_http_missing_path_yields_named_parse_error(
        port in port_strategy(),
    ) {
        let toml = format!(
            "{SERVICE_PRELUDE}\n\
             [[health_check.startup]]\n\
             type = \"http\"\n\
             port = {port}\n"
        );
        match parse_err(&toml) {
            ParseError::HttpProbeMissingPath { probe_idx } => {
                prop_assert_eq!(probe_idx, 0, "single-probe TOML reports probe_idx 0");
            }
            other => prop_assert!(false, "expected HttpProbeMissingPath, got {:?}", other),
        }
    }
}

// S-SHCP-PARSE-03 (US-03 / K1 / ADR-0057 §1) — Exec probe parse +
// defaults. Proptest over arbitrary non-empty `command` arrays and
// optional `args` arrays: a `type = "exec"` startup probe parses into
// `ProbeMechanic::Exec { command }` where `command` is the
// concatenation `[command_binary, command_extra.., args..]` (binary at
// index 0, the rest as argv tail per the ExecProber trait contract),
// with the ADR-0057 §2 defaults (timeout 5, interval 2, max_attempts
// 30) and `inferred: false`.
//
// Universe (port-exposed observable surface of the parsed descriptor):
// mechanic.command, timeout_seconds, interval_seconds, max_attempts,
// inferred. Every slot is asserted; none silently drifts.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn health_check_startup_exec_parses_with_defaults(
        id in service_id_strategy(),
        port in port_strategy(),
        command in exec_command_strategy(),
        args in exec_args_strategy(),
    ) {
        let command_toml = toml_string_array(&command);
        let args_toml = toml_string_array(&args);
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

[[health_check.startup]]
type = "exec"
command = {command_toml}
args = {args_toml}
"#
        );
        let input = WorkloadSpecInput::from_toml_str(&toml)
            .expect("Exec probe parse must succeed");
        let svc = unwrap_service(input);
        prop_assert_eq!(svc.startup_probes.len(), 1, "exactly one declared exec probe");
        let p: &ProbeDescriptor = &svc.startup_probes[0];
        prop_assert_eq!(p.role, ProbeRole::Startup);
        // The argv the ExecProber receives is `command` followed by
        // `args` — binary at index 0, every other token an argv tail.
        let mut expected_argv = command;
        expected_argv.extend(args.iter().cloned());
        match &p.mechanic {
            ProbeMechanic::Exec { command: parsed } => {
                prop_assert_eq!(parsed, &expected_argv,
                    "parsed argv is `command` ++ `args`");
            }
            other => prop_assert!(false, "expected Exec mechanic, got {:?}", other),
        }
        prop_assert_eq!(p.timeout_seconds, 5);
        prop_assert_eq!(p.interval_seconds, 2);
        prop_assert_eq!(p.max_attempts, 30);
        prop_assert!(!p.inferred, "operator-declared probe carries inferred=false");
    }
}

// S-SHCP-PARSE-04 (US-03 / K5 / ADR-0057 §3) — Exec probe with an
// EMPTY `command` array yields
// `ParseError::ExecProbeMissingCommand { probe_idx }`. Proptest over
// arbitrary optional `args` — the missing-command verdict is invariant
// in the presence/absence of args (an empty `command` is the trigger),
// and the reported probe_idx is the observed position (0 for the
// single-probe TOML).
proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn health_check_exec_empty_command_yields_named_parse_error(
        args in exec_args_strategy(),
    ) {
        let args_toml = toml_string_array(&args);
        let toml = format!(
            "{SERVICE_PRELUDE}\n\
             [[health_check.startup]]\n\
             type = \"exec\"\n\
             command = []\n\
             args = {args_toml}\n"
        );
        match parse_err(&toml) {
            ParseError::ExecProbeMissingCommand { probe_idx } => {
                prop_assert_eq!(probe_idx, 0, "single-probe TOML reports probe_idx 0");
            }
            other => prop_assert!(false, "expected ExecProbeMissingCommand, got {:?}", other),
        }
    }
}

/// Arbitrary `[[health_check.<role>]]` TOML fragment naming any of the
/// three roles with a minimal valid TCP body. Used by the kind-
/// rejection proptests so the rejection is shown invariant across
/// every probe role (startup / readiness / liveness), not just one.
fn health_check_role_strategy() -> impl Strategy<Value = String> {
    (prop::sample::select(vec!["startup", "readiness", "liveness"]), port_strategy()).prop_map(
        |(role, port)| format!("[[health_check.{role}]]\ntype = \"tcp\"\nport = {port}\n"),
    )
}

// S-SHCP-PARSE-05 (US-07 / K5) — a `[[health_check.*]]` array of ANY
// role declared on a `[job]` workload is rejected with
// `ParseError::ProbesNotAllowedOnKind { kind: "job", guidance: <job
// guidance> }`. Universe = (probe role ∈ {startup, readiness,
// liveness}) × (arbitrary port). The observable surface is the parser's
// returned `ParseError` variant + its `kind` + `guidance` fields; both
// are asserted, neither drifts.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn probes_under_job_kind_yields_named_parse_error_with_guidance(
        health_check in health_check_role_strategy(),
    ) {
        let toml = format!(
            "[job]\nid = \"j\"\n\n\
             [exec]\ncommand = \"/usr/bin/server\"\nargs = []\n\n\
             [resources]\ncpu_milli = 100\nmemory_bytes = 134217728\n\n\
             {health_check}"
        );
        match parse_err(&toml) {
            ParseError::ProbesNotAllowedOnKind { kind, guidance } => {
                prop_assert_eq!(kind, "job");
                prop_assert_eq!(
                    guidance,
                    overdrive_core::aggregate::JOB_PROBES_GUIDANCE
                );
            }
            other => prop_assert!(false, "expected ProbesNotAllowedOnKind(job), got {:?}", other),
        }
    }
}

// S-SHCP-PARSE-06 (US-07 / K5) — same as PARSE-05 but for a
// `[job]+[schedule]` workload → `ProbesNotAllowedOnKind { kind:
// "schedule", guidance: <schedule guidance> }`. Universe = (probe role)
// × (arbitrary port).
proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn probes_under_schedule_kind_yields_named_parse_error_with_guidance(
        health_check in health_check_role_strategy(),
    ) {
        let toml = format!(
            "[job]\nid = \"j\"\n\n\
             [schedule]\ncron = \"* * * * *\"\n\n\
             [exec]\ncommand = \"/usr/bin/server\"\nargs = []\n\n\
             [resources]\ncpu_milli = 100\nmemory_bytes = 134217728\n\n\
             {health_check}"
        );
        match parse_err(&toml) {
            ParseError::ProbesNotAllowedOnKind { kind, guidance } => {
                prop_assert_eq!(kind, "schedule");
                prop_assert_eq!(
                    guidance,
                    overdrive_core::aggregate::SCHEDULE_PROBES_GUIDANCE
                );
            }
            other => prop_assert!(
                false,
                "expected ProbesNotAllowedOnKind(schedule), got {:?}",
                other
            ),
        }
    }
}

// S-SHCP-PARSE-07 (US-07 regression guard) — a Service-kind workload
// with a `[[health_check.readiness]]` section parses successfully and
// the readiness probe survives into `ServiceSpec.readiness_probes` with
// the ADR-0057 §2 / ADR-0055 §6 `success_threshold` default of 1.
// Universe = (arbitrary readiness port). Observable surface: the parsed
// `ServiceSpec`'s readiness_probes vec (len, role, success_threshold).
proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn probes_under_service_kind_parses_successfully(
        readiness_port in port_strategy(),
    ) {
        let toml = format!(
            "{SERVICE_PRELUDE}\n\
             [[health_check.readiness]]\n\
             type = \"tcp\"\n\
             port = {readiness_port}\n"
        );
        let spec = unwrap_service(
            WorkloadSpecInput::from_toml_str(&toml).expect("service+readiness parses"),
        );
        prop_assert_eq!(spec.readiness_probes.len(), 1, "one readiness probe survives");
        let probe = &spec.readiness_probes[0];
        prop_assert_eq!(probe.role, ProbeRole::Readiness);
        prop_assert_eq!(
            probe.success_threshold,
            Some(1),
            "ADR-0057 §2 / ADR-0055 §6 success_threshold default is 1"
        );
        prop_assert_eq!(probe.inferred, false, "operator-declared probe is not inferred");
    }
}

// S-SHCP-PARSE-08 (US-02 AC / ADR-0057 C6) — any `https://` URL in an
// HTTP probe is rejected at parse time with
// `ParseError::HttpsNotSupported { probe_idx }`. Phase 1 is plain HTTP
// only. Proptest over arbitrary host/path tails after the `https://`
// scheme prefix — the rejection is invariant in the URL body; the
// scheme is the trigger.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn health_check_http_https_scheme_yields_named_parse_error(
        port in port_strategy(),
        tail in "[a-z0-9./]{1,24}",
    ) {
        // `https://` can appear either as the `path` value (the
        // operator pasting a full URL) — the parser rejects the
        // scheme wherever it appears in the probe's path.
        let toml = format!(
            "{SERVICE_PRELUDE}\n\
             [[health_check.startup]]\n\
             type = \"http\"\n\
             path = \"https://{tail}\"\n\
             port = {port}\n"
        );
        match parse_err(&toml) {
            ParseError::HttpsNotSupported { probe_idx } => {
                prop_assert_eq!(probe_idx, 0, "single-probe TOML reports probe_idx 0");
            }
            other => prop_assert!(false, "expected HttpsNotSupported, got {:?}", other),
        }
    }
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

/// Arbitrary absolute HTTP probe path — always leads with `/`, never
/// contains a scheme. Used by the S-SHCP-PARSE-01 happy-path proptest.
fn http_path_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::char::range('a', 'z'), 1..=12)
        .prop_map(|chars| format!("/{}", chars.into_iter().collect::<String>()))
}

/// Arbitrary non-empty exec `command` array — each token is a short
/// lowercase identifier with a leading `/` on the first (the binary
/// path). Used by the S-SHCP-PARSE-03 happy-path proptest.
fn exec_command_strategy() -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec(exec_token_strategy(), 1..=4).prop_map(|mut tokens| {
        // First token is the binary path — lead with `/`.
        tokens[0] = format!("/usr/bin/{}", tokens[0]);
        tokens
    })
}

/// Arbitrary (possibly empty) exec `args` array of short tokens.
fn exec_args_strategy() -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec(exec_token_strategy(), 0..=4)
}

/// A single exec token — a short lowercase identifier with no TOML
/// metacharacters (so `toml_string_array` produces valid TOML without
/// escaping concerns).
fn exec_token_strategy() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::char::range('a', 'z'), 1..=8)
        .prop_map(|chars| chars.into_iter().collect())
}

/// Render a slice of tokens as a TOML inline array of strings:
/// `["a", "b"]`. Tokens are constrained to `[a-z/]` by the strategies
/// above so no escaping is required.
fn toml_string_array(tokens: &[String]) -> String {
    let body = tokens.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(", ");
    format!("[{body}]")
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
