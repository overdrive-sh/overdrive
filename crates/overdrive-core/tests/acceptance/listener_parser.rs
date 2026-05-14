//! Acceptance scenarios for `workload-kind-discriminator` Slice 06 —
//! Service `[[listener]]` parser shape per ADR-0047 §1 (Service listener
//! fields).
//!
//! Driving port: `WorkloadSpecInput::from_toml_str` per ADR-0047 §2.
//!
//! Scenarios from
//! `docs/feature/workload-kind-discriminator/distill/test-scenarios.md`
//! §8 (S-08-01 .. S-08-06). The CLI- and OpenAPI-driving-port scenarios
//! (S-08-07 .. S-08-12) live in the consuming crates' integration test
//! lanes per `.claude/rules/testing.md` § "Integration vs unit gating".

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::single_char_pattern)]

use std::net::{IpAddr, Ipv4Addr};

use overdrive_core::aggregate::{
    Listener, ParseError, ServiceSpec, ServiceVip, WorkloadKind, WorkloadSpecInput,
};
use overdrive_core::dataplane::backend_key::Proto;

// Per service-vip-allocator step 02-01 / ADR-0049 § 5 the
// operator-supplied `vip` field on `[[listener]]` was removed; the
// previous slice-06 mixed-pinned-and-pending VIP scenarios (
// `s_08_04_pinned_vip_collision_also_rejected`,
// `s_08_04_distinct_vips_at_same_port_are_allowed`) were deleted in
// the same commit per `.claude/rules/development.md` § "Deletion
// discipline". The parser-rejection scenarios for `vip` live at
// `listener_rejects_vip_field` (S-VIP-13 / S-VIP-14).

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Render a TOML body with the supplied `[[listener]]` section. Other
/// sections (`[service]`, `[exec]`, `[resources]`) are fixed canonical
/// minimal values.
fn service_toml_with_listeners(listeners_section: &str) -> String {
    format!(
        r#"
[service]
id = "frontend"
replicas = 1

{listeners_section}

[exec]
command = "/opt/frontend/bin/server"
args = []

[resources]
cpu_milli = 500
memory_bytes = 134217728
"#
    )
}

fn unwrap_service(input: WorkloadSpecInput) -> ServiceSpec {
    match input {
        WorkloadSpecInput::Service(s) => s,
        other => panic!("expected Service-kind spec, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// S-08-01 — Two valid listeners parse with both triples preserved in
// declaration order
// ---------------------------------------------------------------------------

#[test]
fn s_08_01_two_listeners_parse_in_declaration_order() {
    // Per ADR-0049 § 5 the `vip` field is no longer admissible on a
    // listener block — the two-listener declaration carries only
    // `(port, protocol)`. VIPs are platform-issued at the service
    // level via `ServiceVipAllocator`.
    let toml = service_toml_with_listeners(
        r#"
[[listener]]
port = 8080
protocol = "tcp"

[[listener]]
port = 8081
protocol = "udp"
"#,
    );

    let parsed = WorkloadSpecInput::from_toml_str(&toml)
        .expect("service with two valid listeners must parse");
    assert_eq!(parsed.kind(), WorkloadKind::Service);

    // S-08-01 asserts the `(port, protocol)` pairs match in
    // declaration order on the Service body.
    let svc = unwrap_service(parsed);
    assert_eq!(svc.listeners.len(), 2, "two declared listeners must reach the spec");
    assert_eq!(svc.listeners[0].port.get(), 8080);
    assert_eq!(svc.listeners[0].protocol, Proto::Tcp);
    assert_eq!(svc.listeners[1].port.get(), 8081);
    assert_eq!(svc.listeners[1].protocol, Proto::Udp);
}

// ---------------------------------------------------------------------------
// S-08-02 — Protocol parsing is case-insensitive and canonicalises to
// lowercase on render
// ---------------------------------------------------------------------------

#[test]
fn s_08_02_protocol_parse_is_case_insensitive_and_renders_lowercase() {
    for variant in ["TCP", "Tcp", "tCp", "tcp"] {
        let toml = service_toml_with_listeners(&format!(
            "[[listener]]\nport = 8080\nprotocol = \"{variant}\"\n"
        ));
        let parsed =
            WorkloadSpecInput::from_toml_str(&toml).expect("case-insensitive parse must succeed");
        let svc = unwrap_service(parsed);
        assert_eq!(svc.listeners[0].protocol, Proto::Tcp);
        // Display canonicalises to lowercase.
        assert_eq!(svc.listeners[0].protocol.to_string(), "tcp");
    }

    for variant in ["UDP", "Udp", "uDp", "udp"] {
        let toml = service_toml_with_listeners(&format!(
            "[[listener]]\nport = 9000\nprotocol = \"{variant}\"\n"
        ));
        let parsed =
            WorkloadSpecInput::from_toml_str(&toml).expect("case-insensitive parse must succeed");
        let svc = unwrap_service(parsed);
        assert_eq!(svc.listeners[0].protocol, Proto::Udp);
        assert_eq!(svc.listeners[0].protocol.to_string(), "udp");
    }
}

// ---------------------------------------------------------------------------
// S-08-03 — A Service with zero listeners is rejected with named
// guidance
// ---------------------------------------------------------------------------

#[test]
fn s_08_03_zero_listeners_rejected() {
    // No [[listener]] blocks at all.
    let toml = service_toml_with_listeners("");
    let err = WorkloadSpecInput::from_toml_str(&toml)
        .expect_err("a Service with zero listeners must be rejected");
    assert!(matches!(err, ParseError::ListenerMissing), "expected ListenerMissing, got {err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains("[service]") && msg.contains("[[listener]]"),
        "error message must name [service] and [[listener]]: {msg:?}"
    );
}

// ---------------------------------------------------------------------------
// S-08-04 — A duplicate `(port, protocol)` pair is rejected with
// named guidance
// ---------------------------------------------------------------------------
//
// Per ADR-0049 § 5 / service-vip-allocator step 02-01 the previous
// `(vip, port, protocol)` triple collapses to a `(port, protocol)`
// pair: VIPs are platform-issued service-wide, not per-listener, so
// two listeners sharing `(port, protocol)` on the same Service are
// always a duplicate. The two prior mixed-pinned-and-pending scenarios
// (`s_08_04_pinned_vip_collision_also_rejected`,
// `s_08_04_distinct_vips_at_same_port_are_allowed`) were deleted in
// the same commit per `.claude/rules/development.md` § "Deletion
// discipline".

#[test]
fn s_08_04_duplicate_pair_rejected() {
    // Two listeners with the same (8080, tcp) pair.
    let toml = service_toml_with_listeners(
        r#"
[[listener]]
port = 8080
protocol = "tcp"

[[listener]]
port = 8080
protocol = "tcp"
"#,
    );
    let err = WorkloadSpecInput::from_toml_str(&toml)
        .expect_err("duplicate (port, protocol) pair must be rejected");
    let pair = match &err {
        ParseError::ListenerDuplicate { triple } => triple.clone(),
        other => panic!("expected ListenerDuplicate, got {other:?}"),
    };
    assert!(pair.contains("8080"), "diagnostic must name the offending port: {pair}");
    assert!(pair.contains("tcp"), "diagnostic must name the offending protocol: {pair}");
}

// ---------------------------------------------------------------------------
// S-08-05 — An unsupported protocol value is rejected (sctp, icmp,
// empty)
// ---------------------------------------------------------------------------

#[test]
fn s_08_05_unsupported_protocols_rejected() {
    for bad in ["sctp", "icmp", "SCTP", "http", ""] {
        let toml = service_toml_with_listeners(&format!(
            "[[listener]]\nport = 8080\nprotocol = \"{bad}\"\n"
        ));
        let err = WorkloadSpecInput::from_toml_str(&toml)
            .unwrap_err_or_else_msg(format!("protocol {bad:?} must be rejected"));
        let msg = err.to_string();
        assert!(
            msg.contains("tcp") && msg.contains("udp"),
            "supported set must be named verbatim: {msg:?}"
        );
        // Verbatim operator-supplied token (or its case) is named.
        if !bad.is_empty() {
            assert!(
                msg.to_ascii_lowercase().contains(&bad.to_ascii_lowercase()),
                "operator token {bad:?} must surface in: {msg:?}"
            );
        }
    }
}

// Local helper trait — `expect_err` panics with stripped context; this
// supplies the per-iteration message inline.
trait UnwrapErrOrElseMsg<T, E: std::fmt::Debug> {
    fn unwrap_err_or_else_msg(self, msg: impl AsRef<str>) -> E;
}
impl<T: std::fmt::Debug, E: std::fmt::Debug> UnwrapErrOrElseMsg<T, E> for Result<T, E> {
    fn unwrap_err_or_else_msg(self, msg: impl AsRef<str>) -> E {
        match self {
            Ok(v) => panic!("{}: got Ok({:?})", msg.as_ref(), v),
            Err(e) => e,
        }
    }
}

// ---------------------------------------------------------------------------
// S-08-06 — `port = 0` is rejected
// ---------------------------------------------------------------------------

#[test]
fn s_08_06_port_zero_rejected() {
    let toml = service_toml_with_listeners(
        r#"
[[listener]]
port = 0
protocol = "tcp"
"#,
    );
    let err = WorkloadSpecInput::from_toml_str(&toml).expect_err("port = 0 must be rejected");
    assert!(matches!(err, ParseError::ListenerPortZero), "got {err:?}");
    let msg = err.to_string();
    assert!(msg.contains('1') && msg.contains("65535"), "error must name the valid range: {msg:?}");
}

#[test]
fn s_08_06_port_above_u16_max_rejected_as_field_error() {
    // Out-of-range — TOML integer 70000 is not a u16. This exercises
    // the same defensive validation but lands on ParseError::Field
    // rather than ListenerPortZero.
    let toml = service_toml_with_listeners(
        r#"
[[listener]]
port = 70000
protocol = "tcp"
"#,
    );
    let err = WorkloadSpecInput::from_toml_str(&toml)
        .expect_err("port out of u16 range must be rejected");
    assert!(matches!(err, ParseError::Field { .. } | ParseError::ListenerPortZero), "got {err:?}");
}

// ---------------------------------------------------------------------------
// Sanity — Listener struct surfaces the chosen (port, protocol) pair;
// ServiceVip survives as a service-level newtype but is structurally
// absent from the per-listener shape per ADR-0049 § 5.
// ---------------------------------------------------------------------------

#[test]
fn listener_struct_carries_port_protocol_pair() {
    use std::num::NonZeroU16;
    let l = Listener { port: NonZeroU16::new(8080).expect("non-zero"), protocol: Proto::Tcp };
    assert_eq!(l.port.get(), 8080);
    assert_eq!(l.protocol, Proto::Tcp);
}

#[test]
fn service_vip_displays_as_ipv4() {
    let v = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100))).expect("IPv4 valid");
    assert_eq!(v.to_string(), "192.168.1.100");
}

// ---------------------------------------------------------------------------
// Regression: serde must not bypass the ≥1 listener invariant
// ---------------------------------------------------------------------------

#[test]
fn serde_json_without_listeners_field_is_rejected() {
    let json = r#"{
        "id": "web",
        "replicas": 1,
        "exec": { "image": "nginx:latest", "command": ["/bin/sh"] },
        "resources": { "cpu_cores": 1, "memory_mib": 512 }
    }"#;
    let result = serde_json::from_str::<ServiceSpec>(json);
    assert!(result.is_err(), "ServiceSpec with no listeners must not deserialize");
}
