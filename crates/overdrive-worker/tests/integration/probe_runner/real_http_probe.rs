//! Tier 3 integration — `HyperHttpProber` against a real in-process
//! tokio-spawned mock HTTP server inside Lima.
//!
//! Slice 02 (US-02) — RED scaffold.

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

/// S-SHCP-INT-02-01 (US-02 / K1) — real HTTP server returns 200 OK;
/// `HyperHttpProber` returns Pass.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_real_http_server_200_when_hyper_http_prober_probes_then_returns_pass() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-02-01 / real HTTP 200 → HyperHttpProber Pass)"
    );
}

/// S-SHCP-INT-02-02 (US-02 / K1) — real HTTP server returns 503;
/// `HyperHttpProber` returns `Fail { reason: "HTTP 503" }`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_real_http_server_503_when_hyper_http_prober_probes_then_returns_fail_named() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-02-02 / real HTTP 503 → HyperHttpProber Fail \"HTTP 503\")"
    );
}

/// S-SHCP-INT-02-03 (US-02 AC / research Pitfall 5) — real HTTP
/// server returns 302 redirect; `HyperHttpProber` returns Fail (NOT
/// Pass; NOT redirect-follow).
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_real_http_server_302_when_hyper_http_prober_probes_then_returns_fail_no_follow() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-02-03 / real HTTP 302 → HyperHttpProber Fail; no redirect-follow)"
    );
}
