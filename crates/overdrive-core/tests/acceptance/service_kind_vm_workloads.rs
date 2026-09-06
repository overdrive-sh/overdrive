//! RED acceptance scaffolds for the pure Service VM ingress contracts in
//! ADR-0091. These tests intentionally name no unimplemented type: the
//! accepted public surface lands in DELIVER, then each body is replaced with
//! its real assertion and the `should_panic` marker is removed.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::missing_panics_doc)]

/// S-SVM-02 — a `[service]` plus `[vm]` document carrying only HTTP/TCP
/// probes parses to the existing VM driver arm, with every probe descriptor
/// preserved as declared.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "RED scaffold")]
fn service_vm_http_tcp_spec_parses_to_vm_driver_without_rewriting_probe_intent() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-02 / ServiceSpecV3 driver-union parse)");
}

/// S-SVM-03 — parser admission checks Startup, then Readiness, then Liveness,
/// and within a role selects the lowest vector position when rejecting the
/// first VM Exec probe.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "RED scaffold")]
fn parser_rejects_first_vm_exec_probe_in_role_then_position_order() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-03 / parser VM Exec rejection order)");
}

/// S-SVM-04 — parser rejection uses the exact selected
/// `[[health_check.<role>]]` section, `entry [N]`, and the shared diagnostic
/// ending in the GH #280 guidance; no Service aggregate is produced.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "RED scaffold")]
fn parser_vm_exec_rejection_is_role_and_entry_localized_before_aggregate_creation() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-04 / parser localization and diagnostic)");
}

/// S-SVM-05 — direct wire clients receive the same first-offender decision
/// from `ServiceV2::from_submit`, localized to `<role>_probes` and `[N]`,
/// before any intent can be persisted.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "RED scaffold")]
fn authoritative_admission_rejects_first_vm_exec_probe_before_intent_exists() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-05 / authoritative VM Exec rejection)");
}

/// S-SVM-06 — the parser and authoritative admission diagnostics share the
/// exact text: `exec probes are not supported for VM Service workloads; use
/// HTTP or TCP; optional VM Exec probes are tracked by GH #280`.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "RED scaffold")]
fn both_vm_exec_rejection_layers_share_the_exact_gh_280_diagnostic() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-06 / rejection diagnostic parity)");
}

/// S-SVM-07 — frozen V1 and V2 ServiceSpec bytes still decode to Latest with
/// an Exec driver, while V3 has its own appended golden fixture and exact
/// discriminant set `[0, 1, 2]`; neither older fixture changes.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "RED scaffold")]
fn service_spec_v1_v2_compatibility_and_v3_golden_bytes_are_preserved() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SVM-07 / ServiceSpec envelope V3 compatibility)"
    );
}

/// S-SVM-08 — admitted VM Service intent projects through allocation and
/// describe using the existing VM variants, preserving every VM field; an
/// Exec Service continues to round-trip through the existing Exec variants.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "RED scaffold")]
fn service_driver_roundtrip_preserves_both_existing_union_arms() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-08 / both-arm driver roundtrip)");
}

/// S-SVM-09 — Exec-backed Services may still declare Exec probes. The new
/// cross-field exclusion is exactly `(Service, VM, Exec probe)`, not a global
/// probe-mechanic rejection.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "RED scaffold")]
fn exec_service_exec_probe_compatibility_is_unchanged() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-09 / Exec Service compatibility)");
}
