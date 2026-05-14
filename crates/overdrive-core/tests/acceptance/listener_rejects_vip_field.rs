//! Acceptance scenarios for `service-vip-allocator` step 02-01 —
//! parser-level rejection of operator-supplied `vip` field on a
//! `[[listener]]` block.
//!
//! Per ADR-0049 § 5 (Amendments — parser-level vs admission-level
//! rejection) the `Listener` struct has no `vip` field; VIPs are
//! platform-issued via `ServiceVipAllocator` keyed by `spec_digest`.
//! Operator-supplied VIPs are STRUCTURALLY UNREPRESENTABLE — the
//! parser rejects any TOML carrying a `vip` field on a listener block
//! with a typed [`ParseError::UnknownField`] variant that names the
//! offending field and guides the operator to remove it.
//!
//! Driving port: `WorkloadSpecInput::from_toml_str` per ADR-0047 §2.
//! The S-VIP-14 "no state mutation" property is asserted indirectly:
//! the parser rejects the spec before any downstream layer reaches the
//! allocator. The allocator stays untouched because the spec never
//! produces a parsed `WorkloadSpec` value to thread through to the
//! handler.
//!
//! Scenarios:
//!
//! * S-VIP-13 — Parser rejects `vip` field with named guidance.
//! * S-VIP-14 — Parser rejection causes no state mutation (asserted
//!   via the structural absence of a parsed Listener / Service spec).

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use overdrive_core::aggregate::{ParseError, WorkloadSpecInput};

/// Canonical TOML used in both scenarios — a Service spec carrying a
/// single `[[listener]]` block with the disallowed `vip` field.
const SERVICE_TOML_WITH_LISTENER_VIP: &str = r#"
[service]
id = "frontend"
replicas = 1

[[listener]]
port = 8080
protocol = "tcp"
vip = "10.96.42.17"

[exec]
command = "/opt/frontend/bin/server"
args = []

[resources]
cpu_milli = 500
memory_bytes = 134217728
"#;

// ---------------------------------------------------------------------------
// S-VIP-13 — Parser rejects `vip` field with named guidance
// ---------------------------------------------------------------------------

#[test]
fn s_vip_13_parser_rejects_vip_field() {
    let err = WorkloadSpecInput::from_toml_str(SERVICE_TOML_WITH_LISTENER_VIP)
        .expect_err("listener carrying `vip` must be rejected at parse time");

    // Typed-variant assertion — the parser must surface the offending
    // field through a structured `ParseError::UnknownField` variant,
    // not a stringy fallback. Operators downstream can branch on the
    // variant for audit / re-render purposes.
    match &err {
        ParseError::UnknownField { section, field } => {
            assert_eq!(*section, "[[listener]]", "section must name the offending block");
            assert_eq!(field, "vip", "field must name the offending key verbatim");
        }
        other => panic!("expected ParseError::UnknownField, got {other:?}"),
    }

    // Operator-facing `Display` form names the offending field AND
    // tells the operator what to do — per ADR-0049 § 5 the guidance
    // is "remove the field; VIPs are platform-issued".
    let msg = err.to_string();
    assert!(msg.contains("vip"), "error message must name `vip`: {msg:?}");
    assert!(
        msg.contains("remove") || msg.contains("platform-issued"),
        "error message must guide the operator: {msg:?}",
    );
}

// ---------------------------------------------------------------------------
// S-VIP-14 — Parser rejection causes no state mutation
// ---------------------------------------------------------------------------
//
// Per the step description / ADR-0049 § 5, the parser fires BEFORE the
// admission handler reaches the allocator. The structural defense
// against state mutation is that no `WorkloadSpecInput` value is
// produced — every downstream consumer (handler, allocator, alloc
// status) is unreachable. We assert the load-bearing property
// directly: `from_toml_str` returns `Err`, with NO partially-
// constructed `Service` / `Job` / `Schedule` value escaping the
// parser.

#[test]
fn s_vip_14_parser_rejection_no_state_mutation() {
    let result = WorkloadSpecInput::from_toml_str(SERVICE_TOML_WITH_LISTENER_VIP);
    assert!(
        result.is_err(),
        "parse MUST fail — operator-supplied VIP is structurally unrepresentable",
    );

    // The structural property: no `WorkloadSpecInput` value is
    // produced; downstream layers (submit_workload handler,
    // ServiceVipAllocator) are unreachable for this spec. Asserting
    // the typed `Err` shape is the observable boundary the parser
    // owns.
    match result {
        Err(ParseError::UnknownField { field, .. }) => {
            assert_eq!(
                field, "vip",
                "rejection must be attributable to the `vip` field (not a different field)",
            );
        }
        Err(other) => {
            panic!("expected UnknownField rejection (no parsed spec escapes), got {other:?}")
        }
        Ok(spec) => {
            panic!("operator-supplied VIP MUST be rejected at parse time; got Ok({spec:?})")
        }
    }
}
