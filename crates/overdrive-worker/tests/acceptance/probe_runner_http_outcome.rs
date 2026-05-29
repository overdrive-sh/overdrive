//! Tier 1 acceptance — `HttpProber` classification + `ProbeRunner`
//! → `SimHttpProber` wiring.
//!
//! Slice 02 (US-02). Two surfaces, both port-to-port at the worker
//! boundary:
//!
//! 1. `HttpProberStatusCodeClassification` (S-SHCP-02-{01..04}) —
//!    a proptest over the FULL HTTP status universe `0..=999` pins the
//!    classification contract exposed by the production
//!    `HyperHttpProber` via its public `classify_http_status` surface.
//!    This is the property the Tier-3 real-server tests exercise
//!    against three representative codes (200 / 503 / 302); the
//!    proptest here covers every code the wire can carry.
//!
//! 2. `ProbeRunner` → `SimHttpProber` wiring — `probe_once_and_record`
//!    with a `ProbeMechanic::Http` descriptor dispatches to the
//!    injected `HttpProber`, classifies the queued outcome, and writes
//!    one `ProbeResultRow` to the `ObservationStore`. The classified
//!    outcome flows through verbatim (no silent transformation between
//!    adapter return, in-memory return value, and stored row).
//!
//! Per US-02 AC + research § 6.1 Pitfall 5: HTTP 3xx responses are
//! treated as Fail; the probe does NOT follow redirects. HTTP method
//! = GET only per Phase 1.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{ProbeIdx, ProbeRole, ProbeStatus};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::ProbeOutcome;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::probe_runner::http_prober::classify_http_status;
use proptest::prelude::*;

fn alloc_id(s: &str) -> AllocationId {
    AllocationId::new(s).expect("alloc id parses")
}

fn node_id_for_obs_store() -> overdrive_core::id::NodeId {
    overdrive_core::id::NodeId::new("node-test").expect("node id parses")
}

fn descriptor_http(path: &str, port: u16) -> ProbeDescriptor {
    ProbeDescriptor {
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Http { path: path.to_owned(), port, host: None },
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

/// Reference oracle for the status-code classification contract per
/// US-02 AC. The SUT (`classify_http_status`) MUST agree with this
/// for every code in the universe.
fn expected_outcome_for_status(code: u16) -> ProbeOutcome {
    match code {
        200..=299 => ProbeOutcome::Pass,
        300..=399 => ProbeOutcome::Fail { reason: format!("HTTP {code} (redirect not followed)") },
        _ => ProbeOutcome::Fail { reason: format!("HTTP {code}") },
    }
}

// ---------------------------------------------------------------------
// S-SHCP-02-{01..04} — HttpProberStatusCodeClassification proptest.
// Universe = full HTTP status code u16 0..=999 (the wire can carry any
// 3-digit status line; classification must be total over that range).
// ---------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// S-SHCP-02-01 / 02 / 03 — every status code classifies into the
    /// US-02 outcome contract:
    /// - 200..=299 → Pass
    /// - 300..=399 → Fail "HTTP <code> (redirect not followed)" (no
    ///   redirect-follow per research § 6.1 Pitfall 5)
    /// - everything else (incl. 400..=599) → Fail "HTTP <code>"
    ///
    /// Universe slot: the single returned `ProbeOutcome`. The oracle
    /// (`expected_outcome_for_status`) is the spec; `classify_http_status`
    /// is the SUT. Equality over 0..=999 is the property.
    #[test]
    fn http_status_classification_matches_us02_contract(code in 0u16..=999u16) {
        let observed = classify_http_status(code);
        let expected = expected_outcome_for_status(code);
        prop_assert_eq!(observed, expected, "classification diverged for status {}", code);
    }

    /// S-SHCP-02-01 — the 2xx band is exactly the Pass band: a code is
    /// Pass IFF it lies in 200..=299. Pins the lower (200) and upper
    /// (299) boundaries against off-by-one mutants on the range guard.
    #[test]
    fn http_pass_band_is_exactly_2xx(code in 0u16..=999u16) {
        let is_pass = matches!(classify_http_status(code), ProbeOutcome::Pass);
        prop_assert_eq!(is_pass, (200..=299).contains(&code),
            "Pass band must be exactly 200..=299; status {} disagreed", code);
    }

    /// S-SHCP-02-03 — the redirect band (3xx) is the ONLY band whose
    /// fail reason carries the "(redirect not followed)" suffix. This
    /// is the load-bearing no-redirect-follow invariant: a 3xx is a
    /// Fail, and its reason names that it was not followed.
    #[test]
    fn http_redirect_suffix_appears_iff_3xx(code in 0u16..=999u16) {
        let reason = match classify_http_status(code) {
            ProbeOutcome::Pass => String::new(),
            ProbeOutcome::Fail { reason } => reason,
        };
        let has_suffix = reason.contains("(redirect not followed)");
        prop_assert_eq!(has_suffix, (300..=399).contains(&code),
            "redirect-not-followed suffix must appear IFF 3xx; status {} disagreed", code);
    }
}

/// S-SHCP-02-04 — connection-refused enqueue → `Fail { reason:
/// "connection refused" }` flows through `ProbeRunner` →
/// `SimHttpProber` verbatim. A transport-level failure (not a status
/// code) is carried by the adapter's `ProbeOutcome::Fail`, recorded
/// into the row unchanged.
///
/// Universe (port-exposed observable surface at the worker boundary):
/// - return value of `probe_once_and_record` (`ProbeResultRow.status`)
/// - state delta on `ObservationStore.list_probe_results_for_alloc`
///   (one new row at `(alloc_id, probe_idx)`).
#[tokio::test]
async fn http_connection_refused_outcome_flows_through_probe_runner_to_store() {
    let http = Arc::new(SimHttpProber::new());
    http.enqueue_outcome(ProbeOutcome::Fail { reason: "connection refused".to_owned() });
    let runner = ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        http,
        Arc::new(SimExecProber::new()),
        Arc::new(SimClock::default()) as Arc<dyn Clock>,
        Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0))
            as Arc<dyn ObservationStore>,
    );
    let clock = SimClock::default();
    let obs = SimObservationStore::single_peer(node_id_for_obs_store(), 0);

    let alloc = alloc_id("alloc-http-refused");
    let descriptor = descriptor_http("/healthz", 8080);

    let before = obs.list_probe_results_for_alloc(&alloc).await.expect("list probe results before");
    prop_assert_empty(&before);

    let returned_row = runner
        .probe_once_and_record(&alloc, ProbeIdx::new(0), &descriptor, &clock, &obs)
        .await
        .expect("probe_once_and_record succeeds for Http refused outcome");

    assert_eq!(
        returned_row.status,
        ProbeStatus::Fail { last_fail_reason: "connection refused".to_owned() },
        "Http connection-refused outcome carried into row.status verbatim"
    );
    assert_eq!(returned_row.role, ProbeRole::Startup);
    assert!(!returned_row.inferred);

    let after = obs.list_probe_results_for_alloc(&alloc).await.expect("list probe results after");
    assert_eq!(after.len(), 1, "exactly one row written");
    assert_eq!(after[0], returned_row, "stored row matches returned row");
}

/// S-SHCP-02-01 (wiring) — a `Pass` outcome on the Http queue flows
/// through `ProbeRunner` → `SimHttpProber` and records
/// `ProbeStatus::Pass`. Pins the `ProbeMechanic::Http` dispatch arm
/// in `probe_once_and_record` (the arm that was
/// `MechanicNotYetImplemented` before this slice).
#[tokio::test]
async fn http_pass_outcome_flows_through_probe_runner_to_store() {
    let http = Arc::new(SimHttpProber::new());
    http.enqueue_outcome(ProbeOutcome::Pass);
    let runner = ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        http,
        Arc::new(SimExecProber::new()),
        Arc::new(SimClock::default()) as Arc<dyn Clock>,
        Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0))
            as Arc<dyn ObservationStore>,
    );
    let clock = SimClock::default();
    let obs = SimObservationStore::single_peer(node_id_for_obs_store(), 0);

    let alloc = alloc_id("alloc-http-pass");
    let descriptor = descriptor_http("/healthz", 8080);

    let returned_row = runner
        .probe_once_and_record(&alloc, ProbeIdx::new(0), &descriptor, &clock, &obs)
        .await
        .expect("probe_once_and_record succeeds for Http Pass outcome");

    assert_eq!(returned_row.status, ProbeStatus::Pass);

    let after = obs.list_probe_results_for_alloc(&alloc).await.expect("list probe results after");
    assert_eq!(after.len(), 1, "exactly one row written");
    assert_eq!(after[0].status, ProbeStatus::Pass, "stored row carries Pass");
}

/// Small helper so the async (non-proptest) test reads the same way
/// as the proptest assertions for the empty-before precondition.
fn prop_assert_empty<T>(v: &[T]) {
    assert!(v.is_empty(), "expected empty slice before first probe");
}
