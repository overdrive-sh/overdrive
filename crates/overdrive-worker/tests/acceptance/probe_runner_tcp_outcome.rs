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
use overdrive_core::traits::clock::Clock;
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
    let clock = Arc::new(SimClock::default());
    let obs = Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0));
    let runner = ProbeRunner::new(
        tcp.clone(),
        http,
        exec,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
    );

    let alloc = alloc_id("alloc-pass-1");
    let descriptor = descriptor_tcp("127.0.0.1", 8080);

    // BEFORE: zero rows for the alloc.
    let before = obs.list_probe_results_for_alloc(&alloc).await.expect("list probe results before");
    assert!(before.is_empty(), "no rows before first probe");

    // ACT: probe once + record.
    let returned_row = runner
        .probe_once_and_record(&alloc, ProbeIdx::new(0), &descriptor, clock.as_ref(), obs.as_ref())
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
    let clock = Arc::new(SimClock::default());
    let obs = Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0));
    let runner = ProbeRunner::new(
        tcp.clone(),
        http,
        exec,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
    );

    let alloc = alloc_id("alloc-fail-1");
    let descriptor = descriptor_tcp("127.0.0.1", 8080);

    let returned_row = runner
        .probe_once_and_record(&alloc, ProbeIdx::new(0), &descriptor, clock.as_ref(), obs.as_ref())
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
    let _runner = ProbeRunner::new(
        prober,
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        Arc::new(SimClock::default()) as Arc<dyn Clock>,
        Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0))
            as Arc<dyn ObservationStore>,
    );
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
    let runner = ProbeRunner::new(
        tcp,
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        Arc::new(SimClock::default()) as Arc<dyn Clock>,
        Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0))
            as Arc<dyn ObservationStore>,
    );
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
    let runner = ProbeRunner::new(
        tcp,
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        Arc::new(SimClock::default()) as Arc<dyn Clock>,
        Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0))
            as Arc<dyn ObservationStore>,
    );
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

/// Regression — TCP probe host `"0.0.0.0"` must be translated to
/// `"127.0.0.1"` before reaching the `TcpProber` adapter, matching
/// the HTTP branch's `http_probe_host` translation and the
/// `TcpProber` trait contract (prober.rs:77-80: "caller is
/// responsible for translating to the workload's reachable address").
#[tokio::test]
async fn tcp_probe_translates_wildcard_host_to_loopback() {
    let tcp = Arc::new(SimTcpProber::new());
    tcp.enqueue_outcome(ProbeOutcome::Pass);
    let clock = Arc::new(SimClock::default());
    let obs = Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0));
    let runner = ProbeRunner::new(
        tcp.clone(),
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
    );

    let alloc = alloc_id("alloc-wildcard-host");
    // Descriptor carries the bind-side wildcard — the default for
    // inferred TCP startup probes.
    let descriptor = descriptor_tcp("0.0.0.0", 8080);

    runner
        .probe_once_and_record(&alloc, ProbeIdx::new(0), &descriptor, clock.as_ref(), obs.as_ref())
        .await
        .expect("probe succeeds");

    assert_eq!(
        tcp.last_probed_host(),
        "127.0.0.1",
        "probe_tick must translate 0.0.0.0 to 127.0.0.1 before calling TcpProber"
    );
}

/// Mutation-kill (01-03c follow-on): `ProbeRunner::stop_alloc` must
/// remove the supervisor entry AND cooperatively cancel any tasks
/// derived from its `CancellationToken`. Replacing the body with
/// `()` (mutant `stop_alloc` body deleted) leaves the supervisor map
/// populated AND leaves the child token uncancelled — both
/// observable through `active_alloc_count` and through the token's
/// `is_cancelled` surface.
///
/// Also pins the `active_alloc_count` exact-count contract across
/// a 0→1→2→1→0 sequence, killing mutants that replace the body with
/// the constant `0` or `1` (both of which fail at distinct points in
/// the sequence).
#[tokio::test]
async fn register_and_stop_alloc_lifecycle_drives_active_count_and_cancels_children() {
    let tcp = Arc::new(SimTcpProber::new());
    let runner = ProbeRunner::new(
        tcp,
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        Arc::new(SimClock::default()) as Arc<dyn Clock>,
        Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0))
            as Arc<dyn ObservationStore>,
    );

    // Universe slot: active_alloc_count starts at 0.
    // Mutant `active_alloc_count -> 1` fails here.
    assert_eq!(runner.active_alloc_count(), 0, "no allocs registered yet");

    let alloc_a = alloc_id("alloc-lifecycle-a");
    let alloc_b = alloc_id("alloc-lifecycle-b");

    // Register first alloc. Capture the child token so we can
    // observe the cooperative-shutdown propagation later.
    let token_a = runner.register_alloc(&alloc_a);
    assert!(!token_a.is_cancelled(), "freshly-registered alloc's token is live");
    // Mutant `active_alloc_count -> 0` fails here.
    assert_eq!(runner.active_alloc_count(), 1, "one supervisor live after first register");

    // Register a second, distinct alloc.
    let token_b = runner.register_alloc(&alloc_b);
    assert!(!token_b.is_cancelled(), "second alloc's token is live");
    // Mutants `active_alloc_count -> 0` AND `active_alloc_count -> 1`
    // both fail here — the count must be exactly 2.
    assert_eq!(runner.active_alloc_count(), 2, "two distinct supervisors live");

    // Re-registering an existing alloc is idempotent (per the
    // production docstring). Count must not advance.
    let token_a_again = runner.register_alloc(&alloc_a);
    assert!(!token_a_again.is_cancelled(), "re-registered token is the same live token");
    assert_eq!(runner.active_alloc_count(), 2, "re-register does not double-count");

    // Stop the first alloc. Mutant `stop_alloc` body deleted (`()`)
    // would leave the supervisor in the map AND leave token_a
    // uncancelled — both assertions below fail under that mutant.
    runner.stop_alloc(&alloc_a);
    assert_eq!(runner.active_alloc_count(), 1, "stop_alloc removed alloc-a from the map");
    assert!(
        token_a.is_cancelled(),
        "stop_alloc cooperatively cancels the alloc's root token (and every child clone)"
    );
    assert!(!token_b.is_cancelled(), "stop_alloc must not affect a sibling alloc's token");

    // Stop is idempotent — calling on an absent alloc is a no-op.
    runner.stop_alloc(&alloc_a);
    assert_eq!(runner.active_alloc_count(), 1, "idempotent stop on already-stopped alloc");

    // After stop_alloc, re-registering the same id must yield a
    // FRESH (live) token — the prior entry must have been removed.
    // The mutant `stop_alloc -> ()` leaves the old entry in place,
    // so `register_alloc` would return the OLD (already-cancelled)
    // token, failing the assertion below.
    let token_a_fresh = runner.register_alloc(&alloc_a);
    assert!(
        !token_a_fresh.is_cancelled(),
        "re-registering after stop_alloc returns a fresh, uncancelled token"
    );
    assert_eq!(runner.active_alloc_count(), 2, "re-register after stop reinstates the supervisor");

    // Tear down both. Mutant `active_alloc_count -> 1` fails at the
    // final assertion (count must reach exactly 0).
    runner.stop_alloc(&alloc_a);
    runner.stop_alloc(&alloc_b);
    assert!(token_b.is_cancelled(), "second alloc's token cancelled by its stop_alloc");
    assert_eq!(runner.active_alloc_count(), 0, "all supervisors removed");
}

/// Mutation-kill (01-03c follow-on): `unix_ms_from_clock` reads the
/// injected `Clock::unix_now()` and converts to `u64` milliseconds.
/// Mutants that replace the body with the constant `0` or `1` corrupt
/// the `ProbeResultRow.last_observed_at_unix_ms` field that operators
/// and downstream reconcilers consume.
///
/// Universe slot: returned `row.last_observed_at_unix_ms` MUST equal
/// the clock's `unix_now().as_millis() as u64`. Pinning against the
/// clock-derived value (not just `> 0`) kills BOTH constant-body
/// mutants — the real value at runtime is the current UNIX epoch in
/// ms (≈ 1.78×10^12), which is neither 0 nor 1.
#[tokio::test]
async fn observed_at_unix_ms_pins_to_injected_clock_value() {
    let tcp = Arc::new(SimTcpProber::new());
    tcp.enqueue_outcome(ProbeOutcome::Pass);

    // Clone the SimClock so the test and the SUT share the same
    // logical-time counter. SimClock returns `unix_epoch + elapsed()`;
    // elapsed only advances via `tick`. Neither side calls `tick`
    // during the probe, so the value is stable across the read in
    // the test and the read inside `probe_once_and_record`.
    let clock = Arc::new(SimClock::default());
    let expected_ms = u64::try_from(clock.unix_now().as_millis())
        .expect("unix_now in ms fits in u64 for the next 580M years");
    // Defence in depth: assert the test fixture itself produces a
    // value that distinguishes the {0,1} mutants from the real one.
    // If this ever fires, the SimClock semantics changed and the
    // test needs to advance the clock explicitly.
    assert!(
        expected_ms > 1,
        "test precondition: SimClock unix_now must exceed the 0/1 mutant constants; got {expected_ms}"
    );

    let obs = Arc::new(SimObservationStore::single_peer(node_id_for_obs_store(), 0));
    let runner = ProbeRunner::new(
        tcp,
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
    );
    let alloc = alloc_id("alloc-clock-pin");
    let descriptor = descriptor_tcp("127.0.0.1", 8080);

    let row = runner
        .probe_once_and_record(&alloc, ProbeIdx::new(0), &descriptor, clock.as_ref(), obs.as_ref())
        .await
        .expect("probe succeeds");

    // Returned row carries the clock-derived timestamp byte-equal.
    // Mutant `unix_ms_from_clock -> 0` fails (row.last_observed = 0
    // ≠ expected_ms).
    // Mutant `unix_ms_from_clock -> 1` fails (row.last_observed = 1
    // ≠ expected_ms).
    assert_eq!(
        row.last_observed_at_unix_ms, expected_ms,
        "row.last_observed_at_unix_ms must equal clock.unix_now().as_millis() (byte-equal, not just > 0)"
    );

    // STATE-DELTA: the stored row carries the same clock-pinned
    // value (no silent transformation between in-memory return and
    // observation-store write).
    let stored =
        obs.list_probe_results_for_alloc(&alloc).await.expect("list probe results after probe");
    assert_eq!(stored.len(), 1, "exactly one probe-result row written");
    assert_eq!(
        stored[0].last_observed_at_unix_ms, expected_ms,
        "stored row's last_observed_at_unix_ms is the clock-derived value, unchanged"
    );
}
