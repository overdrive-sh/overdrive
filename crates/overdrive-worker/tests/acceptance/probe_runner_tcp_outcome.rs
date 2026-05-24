//! Tier 1 acceptance — `ProbeRunner` against `SimTcpProber`.
//!
//! Slice 01 (walking skeleton) — RED scaffold.
//!
//! Per `.claude/rules/testing.md`: default-lane Tier 1 tests use Sim
//! adapters; the production `TokioTcpProber` is exercised by Tier 3
//! integration tests at `tests/integration/probe_runner_real_tcp.rs`.
//!
//! Per `.claude/rules/testing.md` § "RED scaffolds and intentionally-
//! failing commits":
//! - Use `#[should_panic(expected = "RED scaffold")]` attribute.
//! - panic body names the scenario ID.

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

use std::sync::Arc;
use std::time::Duration;

use overdrive_core::traits::prober::{ProbeOutcome, TcpProber};
use overdrive_sim::adapters::probers::SimTcpProber;

/// S-SHCP-01-01 (US-01 / K1) — `ProbeRunner` returns `Pass` when the
/// `SimTcpProber`'s outcome queue yields `Pass`.
///
/// Universe (port-exposed observable surface at the worker boundary):
/// - `ProbeRunner` returns `Ok(ProbeOutcome::Pass)` from the TCP
///   prober.
///
/// Failure mode under audit at RED: production binding doesn't exist;
/// `SimTcpProber::probe` is a `todo!`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_sim_tcp_prober_with_pass_outcome_when_probe_then_returns_pass() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-01-01 / SimTcpProber returns Pass when queue yields Pass)"
    );
}

/// S-SHCP-01-02 (US-01 / K1) — `ProbeRunner` returns `Fail
/// { reason: "connection refused" }` when the `SimTcpProber`'s
/// outcome queue yields `Fail`.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_sim_tcp_prober_with_fail_outcome_when_probe_then_returns_fail_with_named_reason() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-01-02 / SimTcpProber returns Fail with named reason)"
    );
}

/// S-SHCP-01-03 (US-01 — Pillar 1 / 3 contract verification) — the
/// `SimTcpProber` implements the `TcpProber` trait surface declared
/// in `overdrive-core::traits::prober`. Structural verification that
/// the three port traits exist and are implemented as expected.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_sim_tcp_prober_when_used_as_dyn_tcp_prober_then_compiles_and_calls_through() {
    let _prober: Arc<dyn TcpProber> = Arc::new(SimTcpProber::new());
    let _ = (Duration::from_secs(5), ProbeOutcome::Pass);
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-01-03 / SimTcpProber implements TcpProber port)"
    );
}
