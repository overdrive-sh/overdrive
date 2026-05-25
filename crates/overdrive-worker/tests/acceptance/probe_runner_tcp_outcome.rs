//! Tier 1 acceptance — `ProbeRunner` against `SimTcpProber`.
//!
//! Per `.claude/rules/testing.md`: default-lane Tier 1 tests use Sim
//! adapters; the production `TokioTcpProber` is exercised by Tier 3
//! integration tests at
//! `tests/integration/probe_runner/real_tcp_probe.rs`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{ProbeIdx, ProbeRole, ProbeStatus};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::{ProbeOutcome, TcpProber};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};
use overdrive_worker::probe_runner::ProbeRunner;

fn alloc_id(s: &str) -> AllocationId {
    AllocationId::new(s).expect("alloc id parses")
}

fn descriptor_tcp(host: &str, port: u16) -> ProbeDescriptor {
    ProbeDescriptor {
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: host.to_owned(), port },
        timeout_seconds: 5,
        interval_seconds: 2,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

fn node_id_for_obs_store() -> overdrive_core::id::NodeId {
    overdrive_core::id::NodeId::new("node-test").expect("node id parses")
}

/// S-SHCP-01-01 (US-01 / K1) — `ProbeRunner` returns Pass when the
/// `SimTcpProber`'s outcome queue yields `Pass`. The
/// `ProbeRunner::probe_once_and_record` body writes a
/// `ProbeResultRow { status: Pass }` to the injected
/// `ObservationStore`.
///
/// Universe (port-exposed observable surface at the worker
/// boundary):
/// - return value of `probe_once_and_record` (the
///   `ProbeResultRow.status`)
/// - state delta on `ObservationStore.list_probe_results_for_alloc`
///   (one new row at `(alloc_id, probe_idx)`).
#[tokio::test]
async fn given_sim_tcp_prober_with_pass_outcome_when_probe_then_returns_pass() {
    let tcp = Arc::new(SimTcpProber::new());
    tcp.enqueue_outcome(ProbeOutcome::Pass);
    let http: Arc<dyn overdrive_core::traits::prober::HttpProber> = Arc::new(SimHttpProber::new());
    let exec: Arc<dyn overdrive_core::traits::prober::ExecProber> = Arc::new(SimExecProber::new());
    let runner = ProbeRunner::new(tcp.clone(), http, exec);

    let clock = SimClock::default();
    let obs = SimObservationStore::single_peer(node_id_for_obs_store(), 0);

    let alloc = alloc_id("alloc-pass-1");
    let descriptor = descriptor_tcp("127.0.0.1", 8080);

    // BEFORE: zero rows for the alloc.
    let before = obs.list_probe_results_for_alloc(&alloc).await.expect("list probe results before");
    assert!(before.is_empty(), "no rows before first probe");

    // ACT: probe once + record.
    let returned_row = runner
        .probe_once_and_record(&alloc, ProbeIdx::new(0), &descriptor, &clock, &obs)
        .await
        .expect("probe_once_and_record succeeds for Pass outcome");

    // ASSERT: return value carries Pass status.
    assert_eq!(returned_row.status, ProbeStatus::Pass);
    assert_eq!(returned_row.alloc_id, alloc);
    assert_eq!(returned_row.probe_idx, ProbeIdx::new(0));
    assert_eq!(returned_row.role, ProbeRole::Startup);
    assert!(!returned_row.inferred, "operator-declared descriptor → row.inferred = false");

    // STATE-DELTA: ObservationStore now carries one row.
    let after = obs.list_probe_results_for_alloc(&alloc).await.expect("list probe results after");
    assert_eq!(after.len(), 1, "exactly one row written");
    assert_eq!(after[0], returned_row, "stored row matches returned row");
}

/// S-SHCP-01-02 (US-01 / K1) — `ProbeRunner` returns `Fail
/// { reason: "connection refused" }` when the `SimTcpProber`'s
/// outcome queue yields `Fail`. The `ProbeResultRow.status` carries
/// the failure reason verbatim.
#[tokio::test]
async fn given_sim_tcp_prober_with_fail_outcome_when_probe_then_returns_fail_with_named_reason() {
    let tcp = Arc::new(SimTcpProber::new());
    tcp.enqueue_outcome(ProbeOutcome::Fail { reason: "connection refused".to_owned() });
    let http: Arc<dyn overdrive_core::traits::prober::HttpProber> = Arc::new(SimHttpProber::new());
    let exec: Arc<dyn overdrive_core::traits::prober::ExecProber> = Arc::new(SimExecProber::new());
    let runner = ProbeRunner::new(tcp.clone(), http, exec);

    let clock = SimClock::default();
    let obs = SimObservationStore::single_peer(node_id_for_obs_store(), 0);

    let alloc = alloc_id("alloc-fail-1");
    let descriptor = descriptor_tcp("127.0.0.1", 8080);

    let returned_row = runner
        .probe_once_and_record(&alloc, ProbeIdx::new(0), &descriptor, &clock, &obs)
        .await
        .expect("probe_once_and_record succeeds for Fail outcome");

    assert_eq!(
        returned_row.status,
        ProbeStatus::Fail { last_fail_reason: "connection refused".to_owned() }
    );

    let after = obs.list_probe_results_for_alloc(&alloc).await.expect("list probe results after");
    assert_eq!(after.len(), 1, "exactly one row written");
    assert_eq!(
        after[0].status,
        ProbeStatus::Fail { last_fail_reason: "connection refused".to_owned() },
        "stored row carries the named fail reason"
    );
}

/// S-SHCP-01-03 (US-01 — Pillar 1 / 3 contract verification) — the
/// `SimTcpProber` implements the `TcpProber` trait surface declared
/// in `overdrive-core::traits::prober`. Structural verification:
/// `Arc<SimTcpProber>` coerces to `Arc<dyn TcpProber>` AND the
/// production `ProbeRunner` accepts this trait-object at the
/// constructor boundary.
#[tokio::test]
async fn given_sim_tcp_prober_when_used_as_dyn_tcp_prober_then_compiles_and_calls_through() {
    let prober: Arc<dyn TcpProber> = Arc::new(SimTcpProber::new());
    // Trait method is callable through the dyn boundary.
    let outcome = prober
        .probe("127.0.0.1", 8080, Duration::from_secs(1))
        .await
        .expect("sim prober inputs valid");
    assert!(matches!(outcome, ProbeOutcome::Pass));

    // ProbeRunner accepts the dyn trait at constructor (the witness
    // is compilation).
    let _runner =
        ProbeRunner::new(prober, Arc::new(SimHttpProber::new()), Arc::new(SimExecProber::new()));
}

/// Earned Trust gate — `ProbeRunner::probe` against a Sim adapter
/// returning Pass surfaces as `Ok(())`. The composition-root
/// invocation lands in 01-03d; this test pins the method's body
/// (sacrificial loopback bind + adapter call + Ok/Err mapping).
#[tokio::test]
async fn given_sim_tcp_prober_pass_when_earned_trust_then_returns_ok() {
    let tcp = Arc::new(SimTcpProber::new());
    // Default SimTcpProber returns Pass on empty queue per its
    // contract; no enqueue needed.
    let runner =
        ProbeRunner::new(tcp, Arc::new(SimHttpProber::new()), Arc::new(SimExecProber::new()));
    runner.probe().await.expect("Earned Trust gate passes against a Pass-shaped Sim adapter");
}

/// Earned Trust gate — `ProbeRunner::probe` against a Sim adapter
/// returning Fail surfaces as
/// `Err(ProbeRunnerError::EarnedTrustFailure)`. The composition
/// root then refuses startup with `health.startup.refused` (wired
/// in 01-03d).
#[tokio::test]
async fn given_sim_tcp_prober_fail_when_earned_trust_then_returns_typed_error() {
    let tcp = Arc::new(SimTcpProber::new());
    tcp.enqueue_outcome(ProbeOutcome::Fail { reason: "earned-trust-injected".to_owned() });
    let runner =
        ProbeRunner::new(tcp, Arc::new(SimHttpProber::new()), Arc::new(SimExecProber::new()));
    let err = runner
        .probe()
        .await
        .expect_err("Earned Trust gate fails against a Fail-shaped Sim adapter");
    let msg = err.to_string();
    assert!(
        msg.contains("Earned Trust"),
        "EarnedTrustFailure variant identifies itself by name; got: {msg:?}"
    );
    assert!(
        msg.contains("earned-trust-injected"),
        "underlying reason propagates into the typed error; got: {msg:?}"
    );
}
