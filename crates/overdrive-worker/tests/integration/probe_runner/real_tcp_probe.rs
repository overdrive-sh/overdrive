//! Tier 3 integration — `TokioTcpProber` against a real loopback
//! `TcpListener` inside Lima.
//!
//! Slice 01 (US-01 WS) — RED scaffold.
//!
//! Per `.claude/rules/testing.md` § "Integration vs unit gating":
//! real-network test belongs in the `integration-tests` slow lane.
//! Per `.claude/rules/testing.md` § "Running tests — Lima VM":
//! invocation goes through `cargo xtask lima run -- cargo nextest
//! run -p overdrive-worker --features integration-tests -E
//! 'test(real_tcp_probe)'`.

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
    reason = "DISTILL RED scaffold; per `.claude/rules/testing.md` § 'RED scaffolds' lints land when DELIVER replaces todo!() bodies + rewrites docs"
)]

/// S-SHCP-INT-01-01 (US-01 WS / K1) — happy path: bind a real
/// loopback listener on `127.0.0.1:0`, probe it, assert Pass.
///
/// Per ADR-0054 §7 (Earned Trust gate uses sacrificial listener on
/// `127.0.0.1:0` — kernel-assigned port; no race).
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_real_loopback_listener_when_tokio_tcp_prober_probes_then_returns_pass() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-01-01 / real loopback listener → TokioTcpProber Pass)"
    );
}

/// S-SHCP-INT-01-02 (US-01 WS / K1 sad path) — connection refused
/// against an unbound port surfaces as
/// `Fail { reason: "connection refused" }`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_unbound_port_when_tokio_tcp_prober_probes_then_returns_fail_connection_refused() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-01-02 / unbound port → TokioTcpProber Fail \"connection refused\")"
    );
}

/// S-SHCP-INT-01-03 (US-01 WS / K1 sad path) — timeout against a
/// black-hole address surfaces as
/// `Fail { reason: "timeout after <duration>" }`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_blackhole_address_when_tokio_tcp_prober_probes_then_returns_fail_timeout() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-01-03 / black-hole address → TokioTcpProber Fail timeout)"
    );
}
