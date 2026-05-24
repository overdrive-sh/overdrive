//! Tier 1 acceptance — `HttpProber` against `SimHttpProber`.
//!
//! Slice 02 (US-02) — RED scaffold.
//!
//! Per US-02 AC + research § 6.1 Pitfall 5: HTTP 3xx responses are
//! treated as Fail; probe does NOT follow redirects. HTTP method =
//! GET only per Phase 1.

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

/// S-SHCP-02-01 (US-02 / K1) — `SimHttpProber` returns `Pass` for an
/// outcome-queue-supplied 200 OK.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_sim_http_prober_with_pass_outcome_when_probe_then_returns_pass() {
    panic!("Not yet implemented -- RED scaffold (S-SHCP-02-01 / SimHttpProber 200 OK → Pass)");
}

/// S-SHCP-02-02 (US-02 / K1) — 503 response yields
/// `Fail { reason: "HTTP 503" }`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_sim_http_prober_with_503_outcome_when_probe_then_returns_fail_http_503() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-02-02 / SimHttpProber 503 → Fail with reason \"HTTP 503\")"
    );
}

/// S-SHCP-02-03 (US-02 AC / research Pitfall 5) — HTTP 3xx redirect
/// is treated as Fail. The probe does NOT follow redirects.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_sim_http_prober_with_302_outcome_when_probe_then_returns_fail_no_redirect_follow() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-02-03 / SimHttpProber 302 → Fail; no redirect-follow)"
    );
}

/// S-SHCP-02-04 (US-02 / K1) — connection refused captures named
/// failure reason `"connection refused"`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_sim_http_prober_with_connection_refused_when_probe_then_returns_fail_named() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-02-04 / SimHttpProber connection refused → Fail with named reason)"
    );
}
